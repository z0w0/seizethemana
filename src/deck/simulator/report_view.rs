//! Combo and hypgeo human output, plus baseline diffing for
//! `deck simulate --baseline`: the print helpers and the delta view,
//! split from `report` to keep files small.

use crate::output::Output;

/// Human output for `--combo` pair access.
pub fn print_combo_access(out: &Output, rows: &[super::report_schema::PairAccessReport]) {
    let s = out.styles();
    println!();
    println!("{}", s.header("Combo assembly (both pieces in hand)"));
    for row in rows {
        println!(
            "  {}  {}  {:.0}% of games",
            s.card_name(&row.pair),
            s.dim(&format!("by t{}", row.target_turn)),
            row.percent_of_games.value()
        );
    }
}

/// Typed store-backed combo report, capped per direction.
pub fn combos_report(
    assembly: &super::combos::Assembly,
    limit: usize,
) -> super::report_schema::CombosReport {
    super::report_schema::CombosReport {
        source: "commanderspellbook".to_string(),
        variants_considered: assembly.variants_considered,
        complete: assembly
            .complete
            .iter()
            .take(limit)
            .map(super::report_schema::ComboAccessReport::from)
            .collect(),
        near_misses: assembly
            .near_misses
            .iter()
            .take(limit)
            .map(super::report_schema::ComboAccessReport::from)
            .collect(),
    }
}

/// Human output for store-backed combos: complete combos with assembly
/// rates, then near-misses (one card away). Combo rows never create
/// problems: they are opportunities, not violations.
/// Spellbook features that win the game, for the win-path filter.
const WIN_FEATURES: [&str; 6] = [
    "Win the game",
    "Infinite damage",
    "Infinite turns",
    "Infinite mana",
    "Infinite card draw",
    "Infinite storm",
];

/// True when the combo produces a win feature.
fn is_win_path(produces: &[String]) -> bool {
    produces
        .iter()
        .any(|p| WIN_FEATURES.iter().any(|f| p.contains(f)))
}

/// Win-path report: complete combos that produce a win feature.
pub fn win_paths_report(
    assembly: &super::combos::Assembly,
    limit: usize,
) -> super::report_schema::WinPathsReport {
    let paths: Vec<super::report_schema::ComboAccessReport> = assembly
        .complete
        .iter()
        .filter(|r| is_win_path(&r.produces))
        .take(limit)
        .map(super::report_schema::ComboAccessReport::from)
        .collect();
    super::report_schema::WinPathsReport {
        count: paths.len(),
        paths,
    }
}

/// Human win-path block: compact, only when win paths exist.
pub fn print_win_paths(out: &Output, report: &super::report_schema::WinPathsReport) {
    let s = out.styles();
    if report.paths.is_empty() {
        return;
    }
    println!();
    println!("{}", s.header("Win paths"));
    for row in &report.paths {
        let feature = row
            .produces
            .iter()
            .find(|p| is_win_path(std::slice::from_ref(p)))
            .map(String::as_str)
            .unwrap_or("");
        println!(
            "  {}  {}  {:.0}% of games",
            s.card_name(&row.combo),
            s.dim(&format!("{feature} by t{}", row.target_turn)),
            row.percent_of_games.value()
        );
    }
}

/// Human view of the store-backed combo assembly: complete combos and
/// one-card-away near misses, each capped at `limit` rows.
pub fn print_store_combos(out: &Output, report: &super::report_schema::CombosReport) {
    let s = out.styles();
    println!();
    println!(
        "{} {}",
        s.header("Combo assembly (Spellbook)"),
        s.dim(&format!(
            "{} variants in the deck",
            report.variants_considered
        ))
    );
    for row in &report.complete {
        let tags = row
            .produces
            .first()
            .map(|p| format!(" → {p}"))
            .unwrap_or_default();
        let bracket = row
            .bracket_tag
            .as_deref()
            .map(|b| format!(" [{b}]"))
            .unwrap_or_default();
        println!(
            "  {}  {}  {:.0}% of games{}{}",
            s.card_name(&row.combo),
            s.dim(&format!("by t{}", row.target_turn)),
            row.percent_of_games.value(),
            s.dim(&tags),
            s.dim(&bracket)
        );
    }
    let misses = &report.near_misses;
    if !misses.is_empty() {
        println!("{}", s.header("One card away"));
        for row in misses {
            println!(
                "  {}  {}  {}{}",
                s.card_name(row.missing.first().map(String::as_str).unwrap_or("?")),
                s.dim(&format!("completes {}", row.combo)),
                s.dim(&format!("by t{}", row.target_turn)),
                row.bracket_tag
                    .as_deref()
                    .map(|b| s.dim(&format!(" [{b}]")))
                    .unwrap_or_default()
            );
        }
    }
}

/// Exact-probability ceilings (`--hypgeo`): print the top gaps between the
/// Monte Carlo castability and the hypergeometric ceiling, so a mana-base
/// problem separates from a draw problem.
pub fn print_hypgeo(out: &Output, payload: &super::hypgeo::HypgeoReport) {
    let s = out.styles();
    println!();
    println!(
        "{}",
        s.header("Cast-on-curve ceilings (exact hypergeometric)")
    );
    for row in payload.cards.iter().take(5) {
        println!(
            "    {}  {}  ceiling {:.0}%",
            s.card_name(&row.name),
            s.dim(&format!("by t{}", row.target_turn)),
            row.percent_castable_ceiling.value()
        );
    }
    println!(
        "{}",
        s.note("ceiling = exact upper bound on the real cast rate; sim castability is draw-agnostic and sits above it")
    );
}

/// One metric delta: typed path, old value, new value.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Delta {
    /// Dot path into the report.
    pub path: String,
    /// Prior value.
    pub old: MetricValue,
    /// New value.
    pub new: MetricValue,
}

/// A scalar value that can appear in a report delta.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum MetricValue {
    /// Numeric report value.
    Number(f64),
    /// Text report value.
    Text(String),
    /// Null report value.
    Null,
}

impl std::fmt::Display for MetricValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::Text(value) => formatter.write_str(value),
            Self::Null => formatter.write_str("null"),
        }
    }
}

/// Finding kinds that appeared or disappeared between the two runs.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FindingDelta {
    /// "resolved" (in baseline, gone now) or "new" (absent before).
    pub change: &'static str,
    /// Stable finding identity, including color when applicable.
    pub identity: String,
    /// The finding's kind and explanation.
    pub kind: String,
    pub explanation: String,
}

/// The full diff between two reports.
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct ReportDiff {
    pub metrics: Vec<Delta>,
    pub findings: Vec<FindingDelta>,
    /// Deck-shape count changes (name, old, new).
    pub shape: Vec<ShapeDelta>,
}

/// One deck-shape count change between the baseline and current reports.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ShapeDelta {
    /// Deck-shape key (e.g. "lands", "artifact_mana_sources").
    pub name: String,
    /// Prior value.
    pub old: String,
    /// New value.
    pub new: String,
}

/// Diff two report payloads: scalar metrics at known paths, findings,
/// and deck-shape counts. Identical inputs yield an empty diff.
pub fn diff_reports(
    baseline: &super::report_schema::SimReport,
    current: &super::report_schema::SimReport,
) -> Result<ReportDiff, String> {
    let mut diff = ReportDiff::default();
    let old_findings = finding_map(baseline)?;
    let new_findings = finding_map(current)?;

    add_optional_percent_metric(
        &mut diff,
        "commander.percent_castable_by_curve",
        baseline
            .commander
            .as_ref()
            .and_then(|c| c.percent_castable_by_curve),
        current
            .commander
            .as_ref()
            .and_then(|c| c.percent_castable_by_curve),
    );
    add_number_metric(
        &mut diff,
        "commander.avg_first_cast_turn",
        baseline
            .commander
            .as_ref()
            .map_or(0.0, |c| c.avg_first_cast_turn),
        current
            .commander
            .as_ref()
            .map_or(0.0, |c| c.avg_first_cast_turn),
    );
    add_percent_metric(
        &mut diff,
        "land_drops.percent_games_with_two_or_fewer_lands_by_turn_4",
        baseline
            .land_drops
            .percent_games_with_two_or_fewer_lands_by_turn_4,
        current
            .land_drops
            .percent_games_with_two_or_fewer_lands_by_turn_4,
    );
    add_percent_metric(
        &mut diff,
        "land_drops.percent_games_with_six_or_more_lands_by_turn_4",
        baseline
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4,
        current
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4,
    );
    add_percent_metric(
        &mut diff,
        "land_drops.expected_percent_with_six_or_more_lands_by_turn_4",
        baseline
            .land_drops
            .expected_percent_with_six_or_more_lands_by_turn_4,
        current
            .land_drops
            .expected_percent_with_six_or_more_lands_by_turn_4,
    );
    add_percent_metric(
        &mut diff,
        "mana.percent_games_with_three_or_more_unused_mana_by_turn_6",
        baseline
            .mana
            .percent_games_with_three_or_more_unused_mana_by_turn_6,
        current
            .mana
            .percent_games_with_three_or_more_unused_mana_by_turn_6,
    );
    add_percent_metric(
        &mut diff,
        "draw.percent_games_with_no_draw_source_by_turn_6",
        baseline.draw.percent_games_with_no_draw_source_by_turn_6,
        current.draw.percent_games_with_no_draw_source_by_turn_6,
    );
    add_percent_metric(
        &mut diff,
        "role_access.removal_spell_percent_seen_by_turn_5",
        baseline.role_access.removal_spell_percent_seen_by_turn_5,
        current.role_access.removal_spell_percent_seen_by_turn_5,
    );
    add_percent_metric(
        &mut diff,
        "role_access.draw_source_percent_seen_by_turn_6",
        baseline.role_access.draw_source_percent_seen_by_turn_6,
        current.role_access.draw_source_percent_seen_by_turn_6,
    );
    add_percent_metric(
        &mut diff,
        "role_access.creature_role_percent_seen_by_turn_3",
        baseline.role_access.creature_role_percent_seen_by_turn_3,
        current.role_access.creature_role_percent_seen_by_turn_3,
    );
    add_percent_metric(
        &mut diff,
        "role_access.win_condition_role_percent_seen_by_turn_8",
        baseline
            .role_access
            .win_condition_role_percent_seen_by_turn_8,
        current
            .role_access
            .win_condition_role_percent_seen_by_turn_8,
    );
    add_percent_metric(
        &mut diff,
        "color_mana_shortage.white",
        baseline.color_mana_shortage.white,
        current.color_mana_shortage.white,
    );
    add_percent_metric(
        &mut diff,
        "color_mana_shortage.blue",
        baseline.color_mana_shortage.blue,
        current.color_mana_shortage.blue,
    );
    add_percent_metric(
        &mut diff,
        "color_mana_shortage.black",
        baseline.color_mana_shortage.black,
        current.color_mana_shortage.black,
    );
    add_percent_metric(
        &mut diff,
        "color_mana_shortage.red",
        baseline.color_mana_shortage.red,
        current.color_mana_shortage.red,
    );
    add_percent_metric(
        &mut diff,
        "color_mana_shortage.green",
        baseline.color_mana_shortage.green,
        current.color_mana_shortage.green,
    );

    let old = &baseline.deck_shape;
    let new = &current.deck_shape;
    add_shape(&mut diff, "total_cards", old.total_cards, new.total_cards);
    add_shape(&mut diff, "lands", old.lands, new.lands);
    add_shape(
        &mut diff,
        "artifact_mana_sources",
        old.artifact_mana_sources,
        new.artifact_mana_sources,
    );
    add_shape(
        &mut diff,
        "creature_mana_sources",
        old.creature_mana_sources,
        new.creature_mana_sources,
    );
    add_shape(&mut diff, "ramp_spells", old.ramp_spells, new.ramp_spells);
    add_shape(
        &mut diff,
        "draw_sources",
        old.draw_sources,
        new.draw_sources,
    );
    add_shape(
        &mut diff,
        "removal_spells",
        old.removal_spells,
        new.removal_spells,
    );
    add_shape(
        &mut diff,
        "targeted_removal_spells",
        old.targeted_removal_spells,
        new.targeted_removal_spells,
    );
    add_shape(
        &mut diff,
        "mass_removal_spells",
        old.mass_removal_spells,
        new.mass_removal_spells,
    );
    add_shape(
        &mut diff,
        "win_conditions",
        old.win_conditions,
        new.win_conditions,
    );

    for (identity, finding) in &new_findings {
        match old_findings.get(identity) {
            None => diff.findings.push(FindingDelta {
                change: "new",
                identity: identity.to_string(),
                kind: finding.kind.clone(),
                explanation: finding.explanation.clone(),
            }),
            Some(old_finding) => {
                add_text_metric(
                    &mut diff,
                    &format!("findings[{identity}].severity"),
                    &old_finding.severity,
                    &finding.severity,
                );
                add_optional_percent_metric(
                    &mut diff,
                    &format!("findings[{identity}].percent_of_games"),
                    old_finding.percent_of_games,
                    finding.percent_of_games,
                );
                add_text_metric(
                    &mut diff,
                    &format!("findings[{identity}].explanation"),
                    &old_finding.explanation,
                    &finding.explanation,
                );
                add_text_metric(
                    &mut diff,
                    &format!("findings[{identity}].suggestion"),
                    &old_finding.suggestion,
                    &finding.suggestion,
                );
                if old_finding.evidence != finding.evidence {
                    add_text_metric(
                        &mut diff,
                        &format!("findings[{identity}].evidence"),
                        &evidence_display(&old_finding.evidence),
                        &evidence_display(&finding.evidence),
                    );
                }
            }
        }
    }
    for (identity, finding) in &old_findings {
        if !new_findings.contains_key(identity) {
            diff.findings.push(FindingDelta {
                change: "resolved",
                identity: identity.to_string(),
                kind: finding.kind.clone(),
                explanation: finding.explanation.clone(),
            });
        }
    }
    Ok(diff)
}

fn add_percent_metric(
    diff: &mut ReportDiff,
    path: &str,
    old: super::report_schema::Percent,
    new: super::report_schema::Percent,
) {
    add_number_metric(diff, path, old.value(), new.value());
}

fn add_optional_percent_metric(
    diff: &mut ReportDiff,
    path: &str,
    old: Option<super::report_schema::Percent>,
    new: Option<super::report_schema::Percent>,
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

fn add_number_metric(diff: &mut ReportDiff, path: &str, old: f64, new: f64) {
    if old != new {
        diff.metrics.push(Delta {
            path: path.to_string(),
            old: MetricValue::Number(old),
            new: MetricValue::Number(new),
        });
    }
}

fn add_text_metric(diff: &mut ReportDiff, path: &str, old: &str, new: &str) {
    if old != new {
        diff.metrics.push(Delta {
            path: path.to_string(),
            old: MetricValue::Text(old.to_string()),
            new: MetricValue::Text(new.to_string()),
        });
    }
}

fn evidence_display(evidence: &[super::report_schema::FindingEvidence]) -> String {
    evidence
        .iter()
        .map(|row| {
            let percent = row.percent_of_games.map_or_else(
                || "not game-based".to_string(),
                |value| format!("{}%", value.value()),
            );
            format!("{}: {} ({percent})", row.subject, row.explanation)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn add_shape<T: ToString + PartialEq>(diff: &mut ReportDiff, name: &str, old: T, new: T) {
    if old != new {
        diff.shape.push(ShapeDelta {
            name: name.to_string(),
            old: old.to_string(),
            new: new.to_string(),
        });
    }
}

fn finding_map(
    report: &super::report_schema::SimReport,
) -> Result<std::collections::BTreeMap<&str, &super::report_schema::Finding>, String> {
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

/// Render the diff for humans: shape changes, metric deltas, findings.
pub fn print_diff(out: &Output, diff: &ReportDiff) {
    let s = out.styles();
    if diff.is_empty() {
        println!("{}", s.success("no changes vs baseline"));
        return;
    }
    println!("{}", s.header("Deltas vs baseline"));
    if !diff.shape.is_empty() {
        println!("{}", s.header("Shape"));
        for d in &diff.shape {
            println!("    {}: {} → {}", d.name, d.old, d.new);
        }
    }
    if !diff.metrics.is_empty() {
        println!("{}", s.header("Metrics"));
        for d in &diff.metrics {
            println!("    {}: {} → {}", d.path, d.old, d.new);
        }
    }
    if !diff.findings.is_empty() {
        println!("{}", s.header("Findings"));
        for p in &diff.findings {
            match p.change {
                // Bare signs: "+" = new problem (red), "-" = resolved
                // (green). error()/success() would print "error: +".
                "new" => println!(
                    "    {} {}: {}",
                    s.glyph("+", crate::output::GlyphKind::Bad),
                    p.kind,
                    p.explanation
                ),
                _ => println!(
                    "    {} {}: {}",
                    s.glyph("-", crate::output::GlyphKind::Good),
                    p.kind,
                    p.explanation
                ),
            }
        }
    }
}

impl ReportDiff {
    /// True when nothing changed between the two reports.
    pub fn is_empty(&self) -> bool {
        self.shape.is_empty() && self.metrics.is_empty() && self.findings.is_empty()
    }

    /// True when the run introduced a finding absent from its baseline.
    pub fn has_new_findings(&self) -> bool {
        self.findings.iter().any(|finding| finding.change == "new")
    }
}

/// Exit status for a normal report: any finding signals a problem.
pub fn normal_exit_code(findings: &[super::findings::Finding]) -> i32 {
    if findings.is_empty() {
        crate::cli::codes::OK
    } else {
        crate::cli::codes::ERROR
    }
}

/// Exit status for a baseline comparison: only new findings fail.
pub fn baseline_exit_code(diff: &ReportDiff) -> i32 {
    if diff.has_new_findings() {
        crate::cli::codes::ERROR
    } else {
        crate::cli::codes::OK
    }
}
