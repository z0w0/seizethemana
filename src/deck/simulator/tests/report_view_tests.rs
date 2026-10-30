//! Tests for rendering and diffing simulator reports.

/// Real-terminal layout checks share this module's report fixtures.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[path = "simulator_output_tests.rs"]
mod simulator_output_tests;

use super::report_schema::{CommanderReport, Finding, Percent, SimReport};
use super::report_view::diff_reports;

fn base_report() -> SimReport {
    let deck = super::model::SimDeck {
        companion: None,
        cards: vec![],
        commanders: vec![],
        format: super::model::Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut stats = super::aggregate::aggregate(&[], &deck, 10);
    stats.runs = 1;
    stats.finished_games = 1;
    super::report::json_report(
        &stats,
        &deck,
        "test",
        42,
        &[],
        Default::default(),
        &Default::default(),
    )
}

fn finding(kind: &str, identity: &str, explanation: &str) -> Finding {
    Finding {
        kind: kind.to_string(),
        identity: identity.to_string(),
        severity: "high".to_string(),
        percent_of_games: Some(Percent::from_percent(31.0)),
        color: None,
        explanation: explanation.to_string(),
        suggestion: "adjust the deck".to_string(),
        evidence: vec![],
    }
}

#[test]
fn identical_reports_diff_to_empty() {
    let base = base_report();
    let diff = diff_reports(&base, &base).expect("typed report schema");
    assert!(diff.is_empty());
}

/// A missing finished subset cannot resolve a prior runtime diagnosis.
#[test]
fn incomplete_comparison_does_not_claim_findings_are_resolved() {
    let mut baseline = base_report();
    baseline.findings.push(finding(
        "limited_draw_access",
        "limited_draw_access",
        "No draw source reached hand.",
    ));
    let mut current = baseline.clone();
    current.outcomes.finished_games = 0;
    current.outcomes.incomplete_games = 1;
    current.findings.clear();
    let diff =
        diff_reports(&baseline, &current).expect("current reports can compare incomplete outcomes");
    assert!(diff.findings.is_empty());
    assert!(!diff.has_regressions());
    assert!(
        diff.metrics
            .iter()
            .any(|row| row.path == "outcomes.finished_games")
    );
}

#[test]
fn metric_deltas_and_shape_changes_report_public_percentages() {
    let mut baseline = base_report();
    baseline.commander = Some(CommanderReport {
        name: "Boss".to_string(),
        mana_value: 4.0,
        percent_castable_by_turn: Default::default(),
        avg_first_cast_turn: Some(4.0),
        p50_cast_turn: Some(4),
        p95_cast_turn: Some(6),
        payable_games: 8,
        percent_castable_by_curve: Some(Percent::from_percent(77.0)),
    });
    baseline.color_mana_shortage.blue = Some(Percent::from_percent(31.0));
    baseline.deck_shape.lands = 44;

    let mut current = baseline.clone();
    current
        .commander
        .as_mut()
        .expect("commander")
        .percent_castable_by_curve = Some(Percent::from_percent(82.0));
    current.color_mana_shortage.blue = Some(Percent::from_percent(20.0));
    current.deck_shape.lands = 42;

    let diff = diff_reports(&baseline, &current).expect("typed report schema");
    let metric_paths: Vec<&str> = diff
        .metrics
        .iter()
        .map(|metric| metric.path.as_str())
        .collect();
    assert!(metric_paths.contains(&"commander.percent_castable_by_curve"));
    assert!(metric_paths.contains(&"color_mana_shortage.blue"));
    assert_eq!(diff.shape.len(), 1);
    assert_eq!(diff.shape[0].name, "lands");
    assert_eq!(diff.shape[0].old, "44");
    assert_eq!(diff.shape[0].new, "42");
    assert!(diff.findings.is_empty());
}

#[test]
fn diff_reports_existing_turn_curves_and_card_castability() {
    let mut baseline = base_report();
    baseline.mana.avg_unused_by_turn.insert(5, 2.0);
    baseline.combat.attack_power_avg_by_turn.insert(5, 3.0);
    baseline
        .card_castability
        .push(super::report_schema::CardCastability {
            name: "Example Spell".to_string(),
            mana_value: 3.0,
            target_turn: 3,
            percent_castable_by_target: Some(Percent::from_percent(40.0)),
            avg_first_castable_turn: Some(4.0),
            payable_games: 8,
        });
    let mut current = baseline.clone();
    current.mana.avg_unused_by_turn.insert(5, 1.0);
    current.combat.attack_power_avg_by_turn.insert(5, 5.0);
    current.card_castability[0].percent_castable_by_target = Some(Percent::from_percent(60.0));
    current.card_castability[0].mana_value = 4.0;
    current.card_castability[0].target_turn = 4;

    let diff = diff_reports(&baseline, &current).expect("typed report schema");
    let paths: Vec<&str> = diff
        .metrics
        .iter()
        .map(|metric| metric.path.as_str())
        .collect();
    assert!(paths.contains(&"mana.avg_unused_by_turn.5"));
    assert!(paths.contains(&"combat.attack_power_avg_by_turn.5"));
    assert!(paths.contains(&"card_castability[Example Spell].percent_castable_by_target"));
    assert!(paths.contains(&"card_castability[Example Spell].mana_value"));
    assert!(paths.contains(&"card_castability[Example Spell].target_turn"));
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.starts_with("card_castability"))
            .count(),
        3,
        "card rows use the name-based comparison only"
    );
}

#[test]
fn commander_potential_diffs_by_slot_and_threshold_result() {
    let row = || super::report_schema::CommanderCombatPotential {
        slot: 0,
        name: "Boss".to_string(),
        damage_by_turn: [(1, 5.0)].into(),
        cumulative_damage_by_turn: [(1, 5.0)].into(),
        percent_games_reaching_21: Some(Percent::from_percent(20.0)),
        observed_games: 10,
        games_reaching_21: 2,
        median_turn_to_21: Some(6),
        note: "Potential only.".to_string(),
    };
    let mut baseline = base_report();
    baseline.combat.commander_potential.push(row());
    let mut current = baseline.clone();
    let current_row = &mut current.combat.commander_potential[0];
    current_row.damage_by_turn.insert(1, 7.0);
    current_row.cumulative_damage_by_turn.insert(1, 7.0);
    current_row.percent_games_reaching_21 = Some(Percent::from_percent(40.0));
    current_row.observed_games = 11;
    current_row.games_reaching_21 = 4;
    current_row.median_turn_to_21 = Some(5);

    let diff = diff_reports(&baseline, &current).expect("same report schema");
    let paths: Vec<&str> = diff
        .metrics
        .iter()
        .map(|metric| metric.path.as_str())
        .collect();
    assert!(paths.contains(&"combat.commander_potential.0.damage_by_turn.1"));
    assert!(paths.contains(&"combat.commander_potential.0.percent_games_reaching_21"));
    assert!(paths.contains(&"combat.commander_potential.0.games_reaching_21"));
    assert!(paths.contains(&"combat.commander_potential.0.observed_games"));
    assert!(paths.contains(&"combat.commander_potential.0.median_turn_to_21"));
}

#[test]
fn removing_a_commander_reports_missing_timing_not_zero() {
    let mut baseline = base_report();
    baseline.commander = Some(CommanderReport {
        name: "Boss".to_string(),
        mana_value: 4.0,
        percent_castable_by_turn: Default::default(),
        avg_first_cast_turn: Some(4.0),
        p50_cast_turn: Some(4),
        p95_cast_turn: Some(6),
        payable_games: 8,
        percent_castable_by_curve: Some(Percent::from_percent(77.0)),
    });
    let mut current = baseline.clone();
    current.commander = None;

    let diff = diff_reports(&baseline, &current).expect("typed report schema");
    let delta = diff
        .metrics
        .iter()
        .find(|row| row.path == "commander.avg_first_cast_turn")
        .expect("commander timing delta");
    assert_eq!(delta.old, super::report_view::MetricValue::Number(4.0));
    assert_eq!(delta.new, super::report_view::MetricValue::Null);
}

#[test]
fn report_diff_includes_opening_hand_and_commander_identity() {
    let mut baseline = base_report();
    baseline.opening_hand.percent_1_land = Some(Percent::from_percent(20.0));
    baseline.commander = Some(CommanderReport {
        name: "Old Commander".to_string(),
        mana_value: 4.0,
        percent_castable_by_turn: Default::default(),
        avg_first_cast_turn: Some(4.0),
        p50_cast_turn: Some(4),
        p95_cast_turn: Some(6),
        payable_games: 8,
        percent_castable_by_curve: Some(Percent::from_percent(77.0)),
    });
    let mut current = baseline.clone();
    current.opening_hand.percent_1_land = Some(Percent::from_percent(30.0));
    current.commander.as_mut().expect("commander").name = "New Commander".to_string();

    let diff = diff_reports(&baseline, &current).expect("typed report schema");
    let paths: Vec<&str> = diff.metrics.iter().map(|row| row.path.as_str()).collect();
    assert!(paths.contains(&"opening_hand.percent_1_land"));
    assert!(paths.contains(&"commander.name"));
}

#[test]
fn mana_report_requires_current_measurement_fields() {
    let mut value = serde_json::to_value(base_report().mana).expect("serialize mana report");
    let object = value.as_object_mut().expect("mana report is an object");
    object.remove("avg_available_by_turn");
    object.remove("avg_cast_spent_by_turn");
    object.remove("avg_unused_by_turn");

    assert!(serde_json::from_value::<super::report_schema::ManaMeasures>(value).is_err());
}

#[test]
fn findings_diff_by_stable_identity() {
    let mut baseline = base_report();
    baseline.findings = vec![
        finding(
            "insufficient_color_mana",
            "insufficient_color_mana:U",
            "blue pips missed",
        ),
        finding(
            "low_castability",
            "low_castability:",
            "three cards cast slowly",
        ),
    ];
    let mut current = base_report();
    current.findings = vec![
        finding(
            "insufficient_color_mana",
            "insufficient_color_mana:U",
            "blue pips missed",
        ),
        finding(
            "excess_lands_seen",
            "excess_lands_seen:",
            "too many lands seen",
        ),
    ];
    let diff = diff_reports(&baseline, &current).expect("typed report schema");
    let changes: Vec<(&str, &str)> = diff
        .findings
        .iter()
        .map(|item| (item.change, item.kind.as_str()))
        .collect();
    assert!(changes.contains(&("resolved", "low_castability")));
    assert!(changes.contains(&("new", "excess_lands_seen")));

    current.findings.push(current.findings[0].clone());
    assert!(
        diff_reports(&baseline, &current)
            .expect_err("duplicate identities fail")
            .contains("duplicate finding identity")
    );
}

#[test]
fn changed_finding_fields_are_reported() {
    let identity = "insufficient_color_mana:U";
    let mut baseline = base_report();
    let mut old_finding = finding("insufficient_color_mana", identity, "blue pips missed");
    old_finding.evidence = vec![super::report_schema::FindingEvidence {
        subject: "Island count".to_string(),
        explanation: "2 sources".to_string(),
        percent_of_games: None,
    }];
    baseline.findings.push(old_finding);

    let mut current = baseline.clone();
    let new_finding = &mut current.findings[0];
    new_finding.severity = "medium".to_string();
    new_finding.percent_of_games = Some(Percent::from_percent(24.0));
    new_finding.suggestion = "add a blue source".to_string();
    new_finding.evidence[0].explanation = "3 sources".to_string();

    let diff = diff_reports(&baseline, &current).expect("typed reports");
    let paths: Vec<&str> = diff
        .metrics
        .iter()
        .map(|metric| metric.path.as_str())
        .collect();
    assert!(paths.contains(&"findings[insufficient_color_mana:U].severity"));
    assert!(paths.contains(&"findings[insufficient_color_mana:U].percent_of_games"));
    assert!(paths.contains(&"findings[insufficient_color_mana:U].suggestion"));
    assert!(paths.contains(&"findings[insufficient_color_mana:U].evidence"));
}

#[test]
fn incomplete_baseline_is_rejected_by_typed_deserialization() {
    let error = serde_json::from_str::<SimReport>(r#"{"findings":[]}"#)
        .expect_err("partial report is invalid");
    assert!(error.to_string().contains("missing field"));
}

#[test]
fn ranked_rows_compare_by_card_and_color_not_array_position() {
    use super::report_schema::ColorPipBlock;
    let mut baseline = base_report();
    baseline.color_pip_blocks = vec![
        ColorPipBlock {
            name: "Same card".to_string(),
            color: "U".to_string(),
            percent_of_games: Percent::from_percent(40.0),
        },
        ColorPipBlock {
            name: "Same card".to_string(),
            color: "W".to_string(),
            percent_of_games: Percent::from_percent(40.0),
        },
    ];
    let mut current = baseline.clone();
    current.color_pip_blocks.reverse();
    assert!(
        diff_reports(&baseline, &current)
            .expect("reordered stable rows")
            .is_empty()
    );
    current.color_pip_blocks[0].percent_of_games = Percent::from_percent(50.0);
    let diff = diff_reports(&baseline, &current).expect("changed stable row");
    assert_eq!(diff.metrics.len(), 1);
    assert_eq!(
        diff.metrics[0].path,
        "color_pip_blocks[Same card:W].percent_of_games"
    );
}

#[test]
fn incompatible_versions_and_actual_opponent_settings_reject_comparison() {
    let baseline = base_report();
    let mut current = baseline.clone();
    current.provenance.rng_version = "another-contract".to_string();
    assert!(
        diff_reports(&baseline, &current)
            .expect_err("different RNG contracts")
            .contains("incompatible baseline")
    );
    current = baseline.clone();
    let stats = super::aggregate::SimStats {
        runs: current.runs,
        turns: current.turns,
        ..Default::default()
    };
    super::report::set_opponent_model(
        &mut current,
        super::model::Format::Commander,
        Default::default(),
        &stats,
    );
    assert!(
        diff_reports(&baseline, &current)
            .expect_err("different external assumptions")
            .contains("opponent scenario")
    );
    current = baseline.clone();
    current.turns = 4;
    assert!(
        diff_reports(&baseline, &current)
            .expect_err("different observation windows")
            .contains("observation horizon")
    );
}

#[test]
fn observed_opponent_activity_changes_are_deltas_not_configuration_changes() {
    let mut baseline = base_report();
    baseline.runs = 2;
    let mut stats = super::aggregate::SimStats {
        runs: 2,
        finished_games: 2,
        opponent_casts: 3,
        opponent_payments: 1,
        ..Default::default()
    };
    super::report::set_opponent_model(
        &mut baseline,
        super::model::Format::Commander,
        Default::default(),
        &stats,
    );
    let mut current = baseline.clone();
    stats.opponent_casts = 4;
    stats.opponent_payments = 2;
    super::report::set_opponent_model(
        &mut current,
        super::model::Format::Commander,
        Default::default(),
        &stats,
    );
    let activity = &current
        .opponent_model
        .as_ref()
        .expect("actual settings")
        .observed_activity;
    assert_eq!(activity.observed_games, 2);
    assert_eq!(activity.totals.casts, 4);
    assert_eq!(
        activity.per_game.as_ref().expect("observed games").casts,
        2.0
    );
    let diff = diff_reports(&baseline, &current).expect("the declared scenario did not change");
    assert!(
        diff.metrics
            .iter()
            .any(|delta| delta.path == "opponent_model.observed_activity.totals.casts")
    );
}

#[test]
fn worse_existing_findings_are_not_labeled_new() {
    let mut baseline = base_report();
    baseline
        .findings
        .push(finding("draw_access", "draw_access", "draw access is low"));
    let mut current = baseline.clone();
    current.findings[0].percent_of_games = Some(Percent::from_percent(40.0));
    let diff = diff_reports(&baseline, &current).expect("same diagnosis");
    assert_eq!(diff.findings.len(), 1);
    assert_eq!(diff.findings[0].change, "worsened");
    assert_eq!(
        super::report_view::baseline_exit_code(&diff),
        crate::cli::codes::ERROR
    );
}

#[test]
fn duplicate_ranked_row_identities_are_rejected() {
    use super::report_schema::ColorPipBlock;
    let mut report = base_report();
    let row = ColorPipBlock {
        name: "A".to_string(),
        color: "U".to_string(),
        percent_of_games: Percent::from_percent(40.0),
    };
    report.color_pip_blocks = vec![row.clone(), row];
    assert!(
        diff_reports(&report, &report)
            .expect_err("ambiguous identities")
            .contains("duplicate row identity")
    );
}

#[test]
fn malformed_finding_fields_are_rejected() {
    let mut report = base_report();
    report.findings.push(finding(
        "insufficient_color_mana",
        "insufficient_color_mana:U",
        "blue pips missed",
    ));
    let mut json = serde_json::to_value(report).expect("serialize typed report");
    json["findings"][0]
        .as_object_mut()
        .expect("finding object")
        .remove("color");
    let error = serde_json::from_value::<SimReport>(json)
        .expect_err("finding color key is required, even when nullable");
    assert!(error.to_string().contains("missing field `color`"));
}

#[test]
fn normal_and_baseline_exit_codes_follow_the_finding_delta() {
    assert_eq!(
        super::report_view::normal_exit_code(&[]),
        crate::cli::codes::OK
    );
    assert_eq!(
        super::report_view::normal_exit_code(&[super::findings::Finding {
            kind: "excess_lands_seen",
            severity: "high",
            game_share: Some(0.25),
            color: None,
            explanation: "too many lands".to_string(),
            suggestion: "trim lands".to_string(),
            evidence: vec![],
        }]),
        crate::cli::codes::ERROR
    );

    let resolved_only = super::report_view::ReportDiff {
        findings: vec![super::report_view::FindingDelta {
            change: "resolved",
            identity: "low_castability".to_string(),
            kind: "low_castability".to_string(),
            explanation: "slow cards removed".to_string(),
        }],
        ..Default::default()
    };
    assert_eq!(
        super::report_view::baseline_exit_code(&resolved_only),
        crate::cli::codes::OK
    );

    let new_finding = super::report_view::ReportDiff {
        findings: vec![super::report_view::FindingDelta {
            change: "new",
            identity: "excess_lands_seen".to_string(),
            kind: "excess_lands_seen".to_string(),
            explanation: "too many lands".to_string(),
        }],
        ..Default::default()
    };
    assert_eq!(
        super::report_view::baseline_exit_code(&new_finding),
        crate::cli::codes::ERROR
    );
}
