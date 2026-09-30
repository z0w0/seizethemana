//! Tests for rendering and diffing simulator reports.
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
    let stats = super::aggregate::aggregate(&[], &deck, 10);
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

#[test]
fn metric_deltas_and_shape_changes_report_public_percentages() {
    let mut baseline = base_report();
    baseline.commander = Some(CommanderReport {
        name: "Boss".to_string(),
        mana_value: 4.0,
        percent_castable_by_turn: Default::default(),
        avg_first_cast_turn: 4.0,
        p50_cast_turn: 4,
        p95_cast_turn: 6,
        percent_castable_by_curve: Some(Percent::from_percent(77.0)),
    });
    baseline.color_mana_shortage.blue = Percent::from_percent(31.0);
    baseline.deck_shape.lands = 44;

    let mut current = baseline.clone();
    current
        .commander
        .as_mut()
        .expect("commander")
        .percent_castable_by_curve = Some(Percent::from_percent(82.0));
    current.color_mana_shortage.blue = Percent::from_percent(20.0);
    current.deck_shape.lands = 42;

    let diff = diff_reports(&baseline, &current).expect("typed report schema");
    assert_eq!(diff.metrics.len(), 2);
    assert_eq!(diff.metrics[0].path, "commander.percent_castable_by_curve");
    assert_eq!(diff.shape.len(), 1);
    assert_eq!(diff.shape[0].name, "lands");
    assert_eq!(diff.shape[0].old, "44");
    assert_eq!(diff.shape[0].new, "42");
    assert!(diff.findings.is_empty());
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
