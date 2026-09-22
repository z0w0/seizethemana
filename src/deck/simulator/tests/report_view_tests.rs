use super::report_view::diff_reports;

fn base_report() -> serde_json::Value {
    serde_json::json!({
        "deck_shape": {"total_cards": 100, "lands": 44, "removal": 4, "wincons": 3},
        "commander": {"on_curve_pct": 0.77, "avg_first_cast_turn": 4.0},
        "role_access": {"removal_pct_seen_by_5": 0.5, "draw_pct_seen_by_6": 0.94},
        "color_screw": {"W": 0.0, "U": 0.31, "B": 0.0, "R": 0.0, "G": 0.22},
        "problems": [
            {"kind": "color_screw", "severity": "high", "color": "U", "detail": "U pips missed 31%"},
            {"kind": "dead_cards", "severity": "high", "detail": "3 cards slow"}
        ],
    })
}

#[test]
fn identical_reports_diff_to_empty() {
    let base = base_report();
    let diff = diff_reports(&base, &base);
    assert!(diff.is_empty());
}

#[test]
fn metric_deltas_and_shape_changes_report() {
    let mut current = base_report();
    current["commander"]["on_curve_pct"] = serde_json::json!(0.82);
    current["color_screw"]["U"] = serde_json::json!(0.20);
    current["deck_shape"]["lands"] = serde_json::json!(42);
    let diff = diff_reports(&base_report(), &current);
    assert_eq!(diff.metrics.len(), 2);
    assert_eq!(diff.metrics[0].path, "commander.on_curve_pct");
    assert_eq!(diff.shape.len(), 1);
    assert_eq!(diff.shape[0].name, "lands");
    assert_eq!(diff.shape[0].old, "44");
    assert_eq!(diff.shape[0].new, "42");
    assert!(diff.problems.is_empty());
}

#[test]
fn new_and_resolved_problems_report() {
    let mut current = base_report();
    // Resolved: dead_cards gone. New: mana_flood appears.
    current["problems"] = serde_json::json!([
        {"kind": "color_screw", "severity": "high", "color": "U", "detail": "U pips missed 31%"},
        {"kind": "mana_flood", "severity": "medium", "detail": "22% flooded"}
    ]);
    let diff = diff_reports(&base_report(), &current);
    let kinds: Vec<(&str, &str)> = diff
        .problems
        .iter()
        .map(|p| (p.change, p.kind.as_str()))
        .collect();
    assert!(kinds_contains(&kinds, &("resolved", "dead_cards")));
}

fn kinds_contains(list: &[(&str, &str)], want: &(&str, &str)) -> bool {
    list.iter().any(|p| p == want)
}
