//! Private helpers for building report deltas.

use super::*;

/// Include report fields that do not have a dedicated comparison helper yet.
pub(super) fn add_unlisted_report_deltas(
    diff: &mut ReportDiff,
    old: &serde_json::Value,
    new: &serde_json::Value,
) -> Result<(), String> {
    let old = old
        .as_object()
        .ok_or_else(|| "serialized baseline report is not an object".to_string())?;
    let new = new
        .as_object()
        .ok_or_else(|| "serialized current report is not an object".to_string())?;
    let keys: std::collections::BTreeSet<&str> = old
        .keys()
        .chain(new.keys())
        .map(String::as_str)
        .filter(|key| !matches!(*key, "findings" | "card_castability" | "deck_shape"))
        .collect();
    for key in keys {
        let old_value = old.get(key).unwrap_or(&serde_json::Value::Null);
        let new_value = new.get(key).unwrap_or(&serde_json::Value::Null);
        add_json_deltas(diff, key, &keyed_rows(old_value)?, &keyed_rows(new_value)?);
    }
    Ok(())
}

/// Compare JSON values recursively and add changed scalar paths once.
pub(super) fn add_json_deltas(
    diff: &mut ReportDiff,
    path: &str,
    old: &serde_json::Value,
    new: &serde_json::Value,
) {
    if old == new {
        return;
    }
    match (old, new) {
        (serde_json::Value::Object(old), serde_json::Value::Object(new)) => {
            let keys: std::collections::BTreeSet<&str> =
                old.keys().chain(new.keys()).map(String::as_str).collect();
            for key in keys {
                add_json_deltas(
                    diff,
                    &if path.ends_with(".commander_potential") && key.starts_with('[') {
                        format!("{path}.{}", key.trim_matches(['[', ']']))
                    } else if key.starts_with('[') {
                        format!("{path}{key}")
                    } else {
                        format!("{path}.{key}")
                    },
                    old.get(key).unwrap_or(&serde_json::Value::Null),
                    new.get(key).unwrap_or(&serde_json::Value::Null),
                );
            }
        }
        (serde_json::Value::Array(old), serde_json::Value::Array(new)) => {
            for index in 0..old.len().max(new.len()) {
                add_json_deltas(
                    diff,
                    &format!("{path}[{index}]"),
                    old.get(index).unwrap_or(&serde_json::Value::Null),
                    new.get(index).unwrap_or(&serde_json::Value::Null),
                );
            }
        }
        _ if !diff.metrics.iter().any(|delta| delta.path == path) => {
            diff.metrics.push(Delta {
                path: path.to_string(),
                old: json_metric_value(old),
                new: json_metric_value(new),
            });
        }
        _ => {}
    }
}

/// Convert a JSON scalar into the report-diff value type.
pub(super) fn json_metric_value(value: &serde_json::Value) -> MetricValue {
    match value {
        serde_json::Value::Null => MetricValue::Null,
        serde_json::Value::Number(number) => number.as_f64().map_or_else(
            || MetricValue::Text(number.to_string()),
            MetricValue::Number,
        ),
        serde_json::Value::String(text) => MetricValue::Text(text.clone()),
        serde_json::Value::Bool(value) => MetricValue::Text(value.to_string()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            MetricValue::Text(value.to_string())
        }
    }
}

pub(super) fn add_percent_metric(
    diff: &mut ReportDiff,
    path: &str,
    old: Option<super::super::report_schema::Percent>,
    new: Option<super::super::report_schema::Percent>,
) {
    add_optional_percent_metric(diff, path, old, new);
}

pub(super) fn add_optional_percent_metric(
    diff: &mut ReportDiff,
    path: &str,
    old: Option<super::super::report_schema::Percent>,
    new: Option<super::super::report_schema::Percent>,
) {
    let old_value = old.map_or(MetricValue::Null, |value| {
        MetricValue::Number(value.value())
    });
    let new_value = new.map_or(MetricValue::Null, |value| {
        MetricValue::Number(value.value())
    });
    if old_value != new_value {
        diff.metrics.push(Delta {
            path: path.to_string(),
            old: old_value,
            new: new_value,
        });
    }
}

/// Compare numeric per-turn values at the union of turns in both reports.
pub(super) fn add_turn_number_values(
    diff: &mut ReportDiff,
    path: &str,
    old: &super::super::report_schema::TurnValues,
    new: &super::super::report_schema::TurnValues,
) {
    let turns: std::collections::BTreeSet<u32> = old.keys().chain(new.keys()).copied().collect();
    for turn in turns {
        add_optional_number_metric(
            diff,
            &format!("{path}.{turn}"),
            old.get(&turn).copied(),
            new.get(&turn).copied(),
        );
    }
}

/// Compare per-turn percentages at the union of turns in both reports.
pub(super) fn add_turn_percent_values(
    diff: &mut ReportDiff,
    path: &str,
    old: &std::collections::BTreeMap<u32, super::super::report_schema::Percent>,
    new: &std::collections::BTreeMap<u32, super::super::report_schema::Percent>,
) {
    let turns: std::collections::BTreeSet<u32> = old.keys().chain(new.keys()).copied().collect();
    for turn in turns {
        add_optional_percent_metric(
            diff,
            &format!("{path}.{turn}"),
            old.get(&turn).copied(),
            new.get(&turn).copied(),
        );
    }
}

/// Compare castability fields by stable card name.
pub(super) fn add_card_castability(
    diff: &mut ReportDiff,
    old: &[super::super::report_schema::CardCastability],
    new: &[super::super::report_schema::CardCastability],
) -> Result<(), String> {
    let old = card_castability_map(old)?;
    let new = card_castability_map(new)?;
    let names: std::collections::BTreeSet<&str> = old.keys().chain(new.keys()).copied().collect();
    for name in names {
        let before = old.get(name);
        let after = new.get(name);
        for (field, old_value, new_value) in [
            (
                "mana_value",
                before.map(|row| row.mana_value),
                after.map(|row| row.mana_value),
            ),
            (
                "target_turn",
                before.map(|row| f64::from(row.target_turn)),
                after.map(|row| f64::from(row.target_turn)),
            ),
            (
                "percent_castable_by_target",
                before.and_then(|row| {
                    row.percent_castable_by_target
                        .map(|percent| percent.value())
                }),
                after.and_then(|row| {
                    row.percent_castable_by_target
                        .map(|percent| percent.value())
                }),
            ),
            (
                "avg_first_castable_turn",
                before.and_then(|row| row.avg_first_castable_turn),
                after.and_then(|row| row.avg_first_castable_turn),
            ),
            (
                "payable_games",
                before.map(|row| f64::from(row.payable_games)),
                after.map(|row| f64::from(row.payable_games)),
            ),
        ] {
            add_optional_number_metric(
                diff,
                &format!("card_castability[{name}].{field}"),
                old_value,
                new_value,
            );
        }
    }
    Ok(())
}

/// Build a unique card-name lookup for report comparison.
pub(super) fn card_castability_map(
    rows: &[super::super::report_schema::CardCastability],
) -> Result<std::collections::BTreeMap<&str, &super::super::report_schema::CardCastability>, String>
{
    let mut by_name = std::collections::BTreeMap::new();
    for row in rows {
        if by_name.insert(row.name.as_str(), row).is_some() {
            return Err(format!(
                "report has duplicate castability row for {:?}",
                row.name
            ));
        }
    }
    Ok(by_name)
}

/// Add a numeric delta when one side is missing or the values changed.
pub(super) fn add_optional_number_metric(
    diff: &mut ReportDiff,
    path: &str,
    old: Option<f64>,
    new: Option<f64>,
) {
    let old_value = old.map_or(MetricValue::Null, MetricValue::Number);
    let new_value = new.map_or(MetricValue::Null, MetricValue::Number);
    if old_value != new_value {
        diff.metrics.push(Delta {
            path: path.to_string(),
            old: old_value,
            new: new_value,
        });
    }
}

pub(super) fn add_number_metric(diff: &mut ReportDiff, path: &str, old: f64, new: f64) {
    if old != new {
        diff.metrics.push(Delta {
            path: path.to_string(),
            old: MetricValue::Number(old),
            new: MetricValue::Number(new),
        });
    }
}

pub(super) fn add_text_metric(diff: &mut ReportDiff, path: &str, old: &str, new: &str) {
    if old != new {
        diff.metrics.push(Delta {
            path: path.to_string(),
            old: MetricValue::Text(old.to_string()),
            new: MetricValue::Text(new.to_string()),
        });
    }
}

pub(super) fn evidence_display(
    evidence: &[super::super::report_schema::FindingEvidence],
) -> String {
    let mut rows: Vec<_> = evidence
        .iter()
        .map(|row| {
            let percent = row.percent_of_games.map_or_else(
                || "not game-based".to_string(),
                |value| format!("{}%", value.value()),
            );
            format!("{}: {} ({percent})", row.subject, row.explanation)
        })
        .collect();
    rows.sort();
    rows.join("; ")
}

/// Reject comparisons whose rules or external assumptions changed.
pub(super) fn check_compatibility(
    old: &super::super::report_schema::SimReport,
    new: &super::super::report_schema::SimReport,
) -> Result<(), String> {
    let a = &old.provenance;
    let b = &new.provenance;
    for (compatible, label) in [
        (a.schema_version == b.schema_version, "report schema"),
        (a.model_version == b.model_version, "rules model"),
        (a.policy_version == b.policy_version, "action policy"),
        (a.rng_version == b.rng_version, "random-number contract"),
        (a.engine_version == b.engine_version, "engine version"),
        (old.format == new.format, "format"),
        (old.turns == new.turns, "observation horizon"),
        (
            same_opponent_settings(old.opponent_model.as_ref(), new.opponent_model.as_ref()),
            "opponent scenario",
        ),
    ] {
        if !compatible {
            return Err(format!(
                "incompatible baseline: {label} changed; regenerate the baseline"
            ));
        }
    }
    if a.deck_fingerprint == b.deck_fingerprint
        && a.card_data_fingerprint != b.card_data_fingerprint
    {
        return Err(
            "incompatible baseline: card data changed for the same deck; regenerate the baseline"
                .to_string(),
        );
    }
    Ok(())
}

/// Actual activity may change with the deck; only declared settings define compatibility.
fn same_opponent_settings(
    old: Option<&super::super::report_schema::OpponentScenario>,
    new: Option<&super::super::report_schema::OpponentScenario>,
) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => {
            old.model_version == new.model_version
                && old.identity == new.identity
                && old.seats == new.seats
                && old.parameters == new.parameters
        }
        (None, None) => true,
        _ => false,
    }
}

/// Convert row arrays to stable keyed objects before comparing scalar fields.
fn keyed_rows(value: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    match value {
        Value::Object(object) => object
            .iter()
            .map(|(key, value)| Ok((key.clone(), keyed_rows(value)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object),
        Value::Array(rows) if rows.iter().all(Value::is_object) => {
            let mut keyed = serde_json::Map::new();
            for row in rows {
                let Some(identity) = row_identity(row) else {
                    return Err("report row has no stable identity".to_string());
                };
                if keyed
                    .insert(format!("[{identity}]"), keyed_rows(row)?)
                    .is_some()
                {
                    return Err(format!("report has duplicate row identity {identity:?}"));
                }
            }
            Ok(Value::Object(keyed))
        }
        Value::Array(rows) => {
            let mut rows = rows.clone();
            if rows.iter().all(Value::is_string) {
                rows.sort_by_key(Value::to_string);
            }
            Ok(Value::Array(rows))
        }
        value => Ok(value.clone()),
    }
}

/// Prefer public identifiers and include color in per-card pip-block identities.
fn row_identity(row: &serde_json::Value) -> Option<String> {
    if let Some(slot) = row.get("slot").and_then(serde_json::Value::as_u64) {
        return Some(slot.to_string());
    }
    if let (Some(phase), Some(turn), Some(reason)) = (
        row.get("phase").and_then(serde_json::Value::as_str),
        row.get("turn").and_then(serde_json::Value::as_u64),
        row.get("reason"),
    ) {
        return Some(format!("{phase}:{turn}:{reason}"));
    }
    for field in ["id", "identity", "pair", "name", "subject"] {
        if let Some(name) = row.get(field).and_then(serde_json::Value::as_str) {
            return Some(match row.get("color").and_then(serde_json::Value::as_str) {
                Some(color) => format!("{name}:{color}"),
                None => name.to_string(),
            });
        }
    }
    None
}

pub(super) fn add_shape<T: ToString + PartialEq>(
    diff: &mut ReportDiff,
    name: &str,
    old: T,
    new: T,
) {
    if old != new {
        diff.shape.push(ShapeDelta {
            name: name.to_string(),
            old: old.to_string(),
            new: new.to_string(),
        });
    }
}

/// Add a shape delta when a keyed value appears, disappears, or changes.
pub(super) fn add_optional_shape<T: ToString + PartialEq>(
    diff: &mut ReportDiff,
    name: &str,
    old: Option<T>,
    new: Option<T>,
) {
    if old != new {
        diff.shape.push(ShapeDelta {
            name: name.to_string(),
            old: old.map_or_else(|| "not present".to_string(), |value| value.to_string()),
            new: new.map_or_else(|| "not present".to_string(), |value| value.to_string()),
        });
    }
}

pub(super) fn finding_map(
    report: &super::super::report_schema::SimReport,
) -> Result<std::collections::BTreeMap<&str, &super::super::report_schema::Finding>, String> {
    let mut findings = std::collections::BTreeMap::new();
    for finding in &report.findings {
        if findings
            .insert(finding.identity.as_str(), finding)
            .is_some()
        {
            return Err(format!(
                "report has duplicate finding identity {:?}",
                finding.identity
            ));
        }
    }
    Ok(findings)
}

/// A higher severity or observed failure share worsens the same diagnosis.
pub(super) fn finding_worsened(
    old: &super::super::report_schema::Finding,
    new: &super::super::report_schema::Finding,
) -> bool {
    let severity = |value: &str| match value {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    };
    severity(&new.severity) > severity(&old.severity)
        || matches!((old.percent_of_games, new.percent_of_games), (Some(old), Some(new)) if new > old)
}
