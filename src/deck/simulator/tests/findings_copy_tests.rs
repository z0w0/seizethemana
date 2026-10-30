//! Finding descriptions must not turn phase observations or card access into wins.

use super::*;

/// Unspent mana belongs to one main-phase sample, not the end of every turn.
#[test]
fn unused_mana_explanation_names_its_phase_and_exclusions() {
    let stats = SimStats {
        unused_mana: vec![3.0; 6],
        ..SimStats::default()
    };
    let mut findings = Vec::new();
    draw_unused_problems(&stats, 6, &mut findings);
    let finding = findings
        .iter()
        .find(|row| row.kind == "unused_mana")
        .expect("unused mana finding");
    assert_eq!(finding.severity, "high");
    assert_eq!(finding.game_share, None);
    assert!(
        finding
            .explanation
            .contains("after turn 6 main actions, 3.0 pool mana")
    );
    assert!(
        finding
            .explanation
            .contains("finished games that completed that phase")
    );
    assert!(
        finding
            .explanation
            .contains("untapped sources or unspent Treasures")
    );
    assert!(!finding.explanation.contains("end turns"));
}

/// Earlier hand access does not show a card is still in hand or can win.
#[test]
fn win_condition_explanation_reports_access_not_a_win() {
    let stats = SimStats {
        wincon_count: 3,
        wincon_access_8: 0.25,
        ..SimStats::default()
    };
    let mut findings = Vec::new();
    access_problems(&stats, true, 8, &mut findings);
    let finding = findings
        .iter()
        .find(|row| row.kind == "low_win_condition_access")
        .expect("win-condition access finding");
    assert_eq!(finding.severity, "high");
    assert_eq!(finding.game_share, Some(0.75));
    assert!(finding.explanation.contains("25.00% of finished games"));
    assert!(
        finding
            .explanation
            .contains("reach hand by turn 8 (3 classified copies)")
    );
    assert!(
        finding
            .explanation
            .contains("card access, not an available win")
    );
}
