//! Combo and uniform-sample access output, plus baseline diffing for
//! `deck simulate --baseline`: the print helpers and the delta view.

use crate::output::Output;

mod diff_helpers;
use diff_helpers::*;

/// Human output for `--combo` pair access.
pub fn print_combo_access(out: &Output, rows: &[super::report_schema::PairAccessReport]) {
    let s = out.styles();
    println!();
    out.print_wrapped(&s.header("Requested combo piece access"));
    out.print_wrapped("  Rates count games where both pieces were seen at least once. The pieces need not have been available together. This does not show combo execution.");
    if rows.is_empty() {
        out.print_wrapped("  No pair-access rows to show.");
    }
    for row in rows {
        let target = if row.target_turn == 0 {
            "no applicable target turn".to_string()
        } else {
            format!("by turn {}", row.target_turn)
        };
        out.print_wrapped(&format!(
            "  {}  {}  {}",
            s.card_name(&row.pair),
            s.dim(&target),
            access_rate(row.percent_of_games, &row.access),
        ));
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

/// Spellbook effect labels included in the win-path filter, not verified wins.
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

/// Show piece access for listed win features without claiming combo execution.
pub fn print_win_paths(out: &Output, report: &super::report_schema::WinPathsReport) {
    let s = out.styles();
    println!();
    out.print_wrapped(&s.header("Combo access for Spellbook win features"));
    if report.paths.is_empty() {
        out.print_wrapped("  No win-feature combo rows to show from the local Spellbook data.");
        return;
    }
    out.print_wrapped("  Spellbook supplies the effect labels. These rates measure piece access, not a legal combo or a win.");
    for row in &report.paths {
        let feature = row
            .produces
            .iter()
            .find(|p| is_win_path(std::slice::from_ref(p)))
            .map(String::as_str)
            .unwrap_or("");
        out.print_wrapped(&format!(
            "  {}  {}  {}",
            s.card_name(&row.combo),
            s.dim(&format!(
                "{feature} · piece access by turn {}",
                row.target_turn
            )),
            combo_rate(row)
        ));
    }
}

/// Human view of the store-backed combo assembly: complete combos and
/// one-card-away near misses, each capped at `limit` rows.
pub fn print_store_combos(out: &Output, report: &super::report_schema::CombosReport) {
    let s = out.styles();
    println!();
    out.print_wrapped(&format!(
        "{} {}",
        s.header("Combo piece access (Spellbook)"),
        s.dim(&format!(
            "{} variants considered",
            report.variants_considered
        ))
    ));
    out.print_wrapped("  Rates count games where each piece reached a required zone at least once. Visits need not overlap. This does not test legal combo execution.");
    if report.complete.is_empty() {
        out.print_wrapped("  No complete combo rows to show.");
    }
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
        out.print_wrapped(&format!(
            "  {}  {}  {}{}{}",
            s.card_name(&row.combo),
            s.dim(&format!("by turn {}", row.target_turn)),
            combo_rate(row),
            s.dim(&tags),
            s.dim(&bracket)
        ));
    }
    let misses = &report.near_misses;
    if !misses.is_empty() {
        out.print_wrapped(&s.header("Unmet combo requirements"));
        out.print_wrapped("  Listed pieces are absent from the deck or need commander assignment. Adding one piece may not meet every requirement.");
        for row in misses {
            out.print_wrapped(&format!(
                "  {}  {}  {}{}",
                s.card_name(&row.missing.join(" + ")),
                s.dim(&format!("required for {}", row.combo)),
                s.dim(&format!("target turn {}", row.target_turn)),
                row.bracket_tag
                    .as_deref()
                    .map(|b| s.dim(&format!(" [{b}]")))
                    .unwrap_or_default()
            ));
        }
    }
}

/// Display the uniform-sample access baseline without treating it as a ceiling.
pub fn print_hypgeo(out: &Output, payload: &super::hypgeo::HypgeoReport) {
    let s = out.styles();
    println!();
    out.print_wrapped(&s.header("Spell and land access baseline"));
    out.print_wrapped("  Uniform sample without replacement: at least one spell copy and enough lands. This is not a cast-rate estimate.");
    if payload.cards.is_empty() {
        out.print_wrapped("  No spell targets to show within this turn limit.");
    }
    for row in payload.cards.iter().take(5) {
        out.print_wrapped(&format!(
            "    {}  {}  access {:.2}%",
            s.card_name(&row.name),
            s.dim(&format!("by turn {}", row.target_turn)),
            row.percent_modeled_access.value()
        ));
    }
    out.print_wrapped(&s.note(&payload.note));
}

/// Separate event-free observations from unavailable combo windows.
fn combo_rate(row: &super::report_schema::ComboAccessReport) -> String {
    access_rate(row.percent_of_games, &row.access)
}

/// Explain unavailable access windows without exposing enum names to readers.
fn access_rate(
    percent: Option<super::report_schema::Percent>,
    access: &super::report_schema::RateEstimate,
) -> String {
    use super::report_schema::ObservationStatus;
    percent.map_or_else(
        || match access.status {
            ObservationStatus::Observed => "unavailable (incomplete report data)".to_string(),
            ObservationStatus::OutsideHorizon => {
                "unavailable (target turn is beyond the turn limit)".to_string()
            }
            ObservationStatus::NotApplicable => "not applicable to this deck".to_string(),
            ObservationStatus::NoObservations => {
                "unavailable (no games observed this access window)".to_string()
            }
        },
        |percent| {
            format!(
                "{:.2}% of {} observed games",
                percent.value(),
                access.observed_games,
            )
        },
    )
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
    /// "resolved", "new", or "worsened" for an existing diagnosis.
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

/// Diff two report payloads, including scalar metrics, findings, and deck shape.
/// Identical inputs yield an empty diff.
pub fn diff_reports(
    baseline: &super::report_schema::SimReport,
    current: &super::report_schema::SimReport,
) -> Result<ReportDiff, String> {
    check_compatibility(baseline, current)?;
    let mut diff = ReportDiff::default();
    let old_findings = finding_map(baseline)?;
    let new_findings = finding_map(current)?;
    add_text_metric(
        &mut diff,
        "simulation.format",
        &baseline.format,
        &current.format,
    );
    add_text_metric(
        &mut diff,
        "simulation.seed",
        &baseline.seed.to_string(),
        &current.seed.to_string(),
    );
    add_text_metric(
        &mut diff,
        "simulation.runs",
        &baseline.runs.to_string(),
        &current.runs.to_string(),
    );
    add_text_metric(
        &mut diff,
        "simulation.turns",
        &baseline.turns.to_string(),
        &current.turns.to_string(),
    );

    add_turn_number_values(
        &mut diff,
        "creatures_by_turn",
        &baseline.creatures_by_turn,
        &current.creatures_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "repeatable_sources_by_turn",
        &baseline.repeatable_sources_by_turn,
        &current.repeatable_sources_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "mana.avg_available_by_turn",
        &baseline.mana.avg_available_by_turn,
        &current.mana.avg_available_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "mana.avg_cast_spent_by_turn",
        &baseline.mana.avg_cast_spent_by_turn,
        &current.mana.avg_cast_spent_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "mana.avg_unused_by_turn",
        &baseline.mana.avg_unused_by_turn,
        &current.mana.avg_unused_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "velocity.avg_cards_seen",
        &baseline.velocity.avg_cards_seen,
        &current.velocity.avg_cards_seen,
    );
    add_turn_percent_values(
        &mut diff,
        "velocity.library_awareness_by_turn",
        &baseline.velocity.library_awareness_by_turn,
        &current.velocity.library_awareness_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "velocity.self_milled_by_turn",
        &baseline.velocity.self_milled_by_turn,
        &current.velocity.self_milled_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "velocity.opponent_milled_by_turn",
        &baseline.velocity.opponent_milled_by_turn,
        &current.velocity.opponent_milled_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "velocity.library_remaining_by_turn",
        &baseline.velocity.library_remaining_by_turn,
        &current.velocity.library_remaining_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "combat.attack_power_avg_by_turn",
        &baseline.combat.attack_power_avg_by_turn,
        &current.combat.attack_power_avg_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "combat.player_damage_avg_by_turn",
        &baseline.combat.player_damage_avg_by_turn,
        &current.combat.player_damage_avg_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "combat.attackers_by_turn",
        &baseline.combat.attackers_by_turn,
        &current.combat.attackers_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "combat.evasive_by_turn",
        &baseline.combat.evasive_by_turn,
        &current.combat.evasive_by_turn,
    );
    for current_row in &current.combat.commander_potential {
        if let Some(baseline_row) = baseline
            .combat
            .commander_potential
            .iter()
            .find(|row| row.slot == current_row.slot)
        {
            add_turn_number_values(
                &mut diff,
                &format!(
                    "combat.commander_potential.{}.damage_by_turn",
                    current_row.slot
                ),
                &baseline_row.damage_by_turn,
                &current_row.damage_by_turn,
            );
            add_turn_number_values(
                &mut diff,
                &format!(
                    "combat.commander_potential.{}.cumulative_damage_by_turn",
                    current_row.slot
                ),
                &baseline_row.cumulative_damage_by_turn,
                &current_row.cumulative_damage_by_turn,
            );
            let prefix = format!("combat.commander_potential.{}", current_row.slot);
            add_optional_percent_metric(
                &mut diff,
                &format!("{prefix}.percent_games_reaching_21"),
                baseline_row.percent_games_reaching_21,
                current_row.percent_games_reaching_21,
            );
            add_number_metric(
                &mut diff,
                &format!("{prefix}.games_reaching_21"),
                f64::from(baseline_row.games_reaching_21),
                f64::from(current_row.games_reaching_21),
            );
            add_number_metric(
                &mut diff,
                &format!("{prefix}.observed_games"),
                f64::from(baseline_row.observed_games),
                f64::from(current_row.observed_games),
            );
            add_optional_number_metric(
                &mut diff,
                &format!("{prefix}.median_turn_to_21"),
                baseline_row.median_turn_to_21.map(f64::from),
                current_row.median_turn_to_21.map(f64::from),
            );
        }
    }
    add_turn_number_values(
        &mut diff,
        "win_conditions.opponent_life_loss_by_turn",
        &baseline.win_conditions.opponent_life_loss_by_turn,
        &current.win_conditions.opponent_life_loss_by_turn,
    );
    add_turn_percent_values(
        &mut diff,
        "win_conditions.percent_games_at_or_above_table_life_by_turn",
        &baseline
            .win_conditions
            .percent_games_at_or_above_table_life_by_turn,
        &current
            .win_conditions
            .percent_games_at_or_above_table_life_by_turn,
    );
    add_turn_percent_values(
        &mut diff,
        "interaction.percent_games_with_ready_interaction_by_turn",
        &baseline
            .interaction
            .percent_games_with_ready_interaction_by_turn,
        &current
            .interaction
            .percent_games_with_ready_interaction_by_turn,
    );
    add_optional_number_metric(
        &mut diff,
        "interaction.mana_held_avg",
        baseline.interaction.mana_held_avg,
        current.interaction.mana_held_avg,
    );
    add_optional_number_metric(
        &mut diff,
        "win_conditions.p50_lethal_turn",
        baseline.win_conditions.p50_lethal_turn.map(f64::from),
        current.win_conditions.p50_lethal_turn.map(f64::from),
    );
    add_percent_metric(
        &mut diff,
        "win_conditions.percent_games_with_extra_turn",
        baseline.win_conditions.percent_games_with_extra_turn,
        current.win_conditions.percent_games_with_extra_turn,
    );
    add_percent_metric(
        &mut diff,
        "win_conditions.percent_games_reaching_counter_win_threshold",
        baseline
            .win_conditions
            .percent_games_reaching_counter_win_threshold,
        current
            .win_conditions
            .percent_games_reaching_counter_win_threshold,
    );
    add_percent_metric(
        &mut diff,
        "win_conditions.percent_games_with_affordable_ultimate",
        baseline
            .win_conditions
            .percent_games_with_affordable_ultimate,
        current
            .win_conditions
            .percent_games_with_affordable_ultimate,
    );
    add_percent_metric(
        &mut diff,
        "win_conditions.percent_games_with_suspected_infinite_mana",
        baseline
            .win_conditions
            .percent_games_with_suspected_infinite_mana,
        current
            .win_conditions
            .percent_games_with_suspected_infinite_mana,
    );
    add_turn_number_values(
        &mut diff,
        "graveyard.avg_size_by_turn",
        &baseline.graveyard.avg_size_by_turn,
        &current.graveyard.avg_size_by_turn,
    );
    add_optional_number_metric(
        &mut diff,
        "graveyard.replay_casts_avg",
        baseline.graveyard.replay_casts_avg,
        current.graveyard.replay_casts_avg,
    );
    add_turn_number_values(
        &mut diff,
        "milestones.free_cast_permanents_entered_avg_by_turn",
        &baseline.milestones.free_cast_permanents_entered_avg_by_turn,
        &current.milestones.free_cast_permanents_entered_avg_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "milestones.dredge_uses_avg_by_turn",
        &baseline.milestones.dredge_uses_avg_by_turn,
        &current.milestones.dredge_uses_avg_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "milestones.graveyard_casts_avg_by_turn",
        &baseline.milestones.graveyard_casts_avg_by_turn,
        &current.milestones.graveyard_casts_avg_by_turn,
    );
    add_turn_number_values(
        &mut diff,
        "milestones.life_funded_draws_avg_by_turn",
        &baseline.milestones.life_funded_draws_avg_by_turn,
        &current.milestones.life_funded_draws_avg_by_turn,
    );
    add_turn_percent_values(
        &mut diff,
        "milestones.percent_games_with_positive_mana_loop_by_turn",
        &baseline
            .milestones
            .percent_games_with_positive_mana_loop_by_turn,
        &current
            .milestones
            .percent_games_with_positive_mana_loop_by_turn,
    );
    add_card_castability(
        &mut diff,
        &baseline.card_castability,
        &current.card_castability,
    )?;

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
    add_optional_number_metric(
        &mut diff,
        "commander.avg_first_cast_turn",
        baseline
            .commander
            .as_ref()
            .and_then(|c| c.avg_first_cast_turn),
        current
            .commander
            .as_ref()
            .and_then(|c| c.avg_first_cast_turn),
    );
    add_optional_percent_metric(
        &mut diff,
        "land_drops.percent_games_with_two_or_fewer_lands_by_turn_4",
        baseline
            .land_drops
            .percent_games_with_two_or_fewer_lands_by_turn_4,
        current
            .land_drops
            .percent_games_with_two_or_fewer_lands_by_turn_4,
    );
    add_optional_percent_metric(
        &mut diff,
        "land_drops.percent_games_with_six_or_more_lands_by_turn_4",
        baseline
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4,
        current
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4,
    );
    add_optional_percent_metric(
        &mut diff,
        "land_drops.expected_percent_with_six_or_more_lands_by_turn_4",
        baseline
            .land_drops
            .expected_percent_with_six_or_more_lands_by_turn_4,
        current
            .land_drops
            .expected_percent_with_six_or_more_lands_by_turn_4,
    );
    add_optional_percent_metric(
        &mut diff,
        "mana.percent_games_with_three_or_more_unused_mana_by_turn_6",
        baseline
            .mana
            .percent_games_with_three_or_more_unused_mana_by_turn_6,
        current
            .mana
            .percent_games_with_three_or_more_unused_mana_by_turn_6,
    );
    add_optional_percent_metric(
        &mut diff,
        "draw.percent_games_with_no_draw_source_by_turn_6",
        baseline.draw.percent_games_with_no_draw_source_by_turn_6,
        current.draw.percent_games_with_no_draw_source_by_turn_6,
    );
    add_optional_percent_metric(
        &mut diff,
        "role_access.removal_spell_percent_seen_by_turn_5",
        baseline.role_access.removal_spell_percent_seen_by_turn_5,
        current.role_access.removal_spell_percent_seen_by_turn_5,
    );
    add_optional_percent_metric(
        &mut diff,
        "role_access.draw_source_percent_seen_by_turn_6",
        baseline.role_access.draw_source_percent_seen_by_turn_6,
        current.role_access.draw_source_percent_seen_by_turn_6,
    );
    add_optional_percent_metric(
        &mut diff,
        "role_access.creature_role_percent_seen_by_turn_3",
        baseline.role_access.creature_role_percent_seen_by_turn_3,
        current.role_access.creature_role_percent_seen_by_turn_3,
    );
    add_optional_percent_metric(
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
    add_shape(
        &mut diff,
        "average_nonland_mana_value",
        old.average_nonland_mana_value,
        new.average_nonland_mana_value,
    );
    add_shape(
        &mut diff,
        "sideboard_cards",
        old.sideboard_cards,
        new.sideboard_cards,
    );
    add_shape(
        &mut diff,
        "maybeboard_cards",
        old.maybeboard_cards,
        new.maybeboard_cards,
    );
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
    add_shape(&mut diff, "locks", old.locks, new.locks);
    add_shape(&mut diff, "boosters", old.boosters, new.boosters);
    let curve_keys: std::collections::BTreeSet<&str> = old
        .curve
        .keys()
        .chain(new.curve.keys())
        .map(String::as_str)
        .collect();
    for key in curve_keys {
        add_optional_shape(
            &mut diff,
            &format!("curve.{key}"),
            old.curve.get(key).copied(),
            new.curve.get(key).copied(),
        );
    }

    let comparable_findings =
        baseline.outcomes.finished_games > 0 && current.outcomes.finished_games > 0;
    for (identity, finding) in new_findings.iter().filter(|_| comparable_findings) {
        match old_findings.get(identity) {
            None => diff.findings.push(FindingDelta {
                change: "new",
                identity: identity.to_string(),
                kind: finding.kind.clone(),
                explanation: finding.explanation.clone(),
            }),
            Some(old_finding) => {
                if finding_worsened(old_finding, finding) {
                    diff.findings.push(FindingDelta {
                        change: "worsened",
                        identity: identity.to_string(),
                        kind: finding.kind.clone(),
                        explanation: finding.explanation.clone(),
                    });
                }
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
    for (identity, finding) in old_findings.iter().filter(|_| comparable_findings) {
        if !new_findings.contains_key(identity) {
            diff.findings.push(FindingDelta {
                change: "resolved",
                identity: identity.to_string(),
                kind: finding.kind.clone(),
                explanation: finding.explanation.clone(),
            });
        }
    }
    let old_value = serde_json::to_value(baseline).map_err(|error| error.to_string())?;
    let new_value = serde_json::to_value(current).map_err(|error| error.to_string())?;
    add_unlisted_report_deltas(&mut diff, &old_value, &new_value)?;
    Ok(diff)
}

/// Render the diff for humans: shape changes, metric deltas, findings.
pub fn print_diff(out: &Output, diff: &ReportDiff) {
    let s = out.styles();
    if diff.is_empty() {
        out.print_wrapped(&s.success("No report changes from baseline."));
        return;
    }
    out.print_wrapped(&s.header("Changes from baseline"));
    if !diff.shape.is_empty() {
        out.print_wrapped(&s.header("Deck counts and roles"));
        for d in &diff.shape {
            out.print_wrapped(&format!("    {}: {} → {}", d.name, d.old, d.new));
        }
    }
    if !diff.metrics.is_empty() {
        out.print_wrapped(&s.header("Metrics (JSON paths and units)"));
        out.print_wrapped("  Percent metrics use a 0 to 100 scale. null means unavailable; check the full report for the reason.");
        for d in &diff.metrics {
            out.print_wrapped(&format!("    {}: {} → {}", d.path, d.old, d.new));
        }
    }
    if !diff.findings.is_empty() {
        out.print_wrapped(&s.header("Findings"));
        for p in &diff.findings {
            match p.change {
                "new" | "worsened" => out.print_wrapped(&format!(
                    "    {} {} {}: {}",
                    s.glyph("+", crate::output::GlyphKind::Bad),
                    p.change,
                    p.kind,
                    p.explanation
                )),
                _ => out.print_wrapped(&format!(
                    "    {} {} {}: {}",
                    s.glyph("-", crate::output::GlyphKind::Good),
                    p.change,
                    p.kind,
                    p.explanation
                )),
            }
        }
    }
}

impl ReportDiff {
    /// True when nothing changed between the two reports.
    pub fn is_empty(&self) -> bool {
        self.shape.is_empty() && self.metrics.is_empty() && self.findings.is_empty()
    }

    /// True when the run introduced a diagnosis or worsened an existing one.
    pub fn has_regressions(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| matches!(finding.change, "new" | "worsened"))
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

/// Exit status for a baseline comparison: new and worsening findings fail.
pub fn baseline_exit_code(diff: &ReportDiff) -> i32 {
    if diff.has_regressions() {
        crate::cli::codes::ERROR
    } else {
        crate::cli::codes::OK
    }
}
