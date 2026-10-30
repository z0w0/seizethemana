//! Report evidence for cast, interaction, keyword, and land boundaries.
//! Synthetic controls certify only the stated engine policy, not printed cards.

use crate::db::CardRow;
use crate::deck::simulator::card_test_support::{
    assert_library_rng, main_phase_state, named_row, pinned_revision,
};
use crate::deck::simulator::game::{
    CardRef, GameCompletion, GameLog, GamePhase, GameState, IncompleteReason, PlayerLossReason,
    TokenKind, card_of,
};
use crate::deck::simulator::library_core_report_tests::put_permanents;
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::report_schema::SimReport;
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// Synthetic companion access, readiness, and land movement controls.
#[path = "cast_boundary_access_report_tests.rs"]
mod access;
/// Cascade child resolution and synthetic free-X controls.
#[path = "cast_boundary_cascade_report_tests.rs"]
mod cascade;
/// Synthetic split, plot, hidden-face, and unsupported-route controls.
#[path = "cast_boundary_faces_report_tests.rs"]
mod faces;
/// Synthetic keyword-body, quantity, and counter controls.
#[path = "cast_boundary_keywords_report_tests.rs"]
mod keywords;

/// Check the remaining free-X and child-stop stages of `cast.cascade`.
pub(super) fn assert_cascade_boundaries_report() {
    cascade::assert_cascade_boundaries_report();
}

/// Check first-bench access, actual payment, and the limits of `cast.companion`.
pub(super) fn assert_companion_access_report() {
    assert_unpinned_keyword_scope(&["Companion"]);
    access::assert_companion_access_report();
}

/// Check split, plot, morph, face-up, Warp, and manifest policies.
pub(super) fn assert_face_choices_report() {
    assert_unpinned_keyword_scope(&["Plot", "Morph", "Megamorph", "Disguise", "Warp"]);
    faces::assert_face_choices_report();
}

/// Check readiness negatives without claiming that an answer resolved.
pub(super) fn assert_interaction_readiness_report() {
    access::assert_interaction_readiness_report();
}

/// Check the generic bodies and chosen targets of the supported keyword paths.
pub(super) fn assert_keyword_limits_report() {
    assert_unpinned_keyword_scope(&["Mobilize", "Offspring", "Explore", "Connive"]);
    keywords::assert_keyword_limits_report();
}

/// Check unknown and combined land-entry conditions through actions and reports.
pub(super) fn assert_land_entry_safety_report() {
    access::assert_land_entry_safety_report();
}

/// Check a library-only move under a live pinned landfall observer.
pub(super) fn assert_landfall_library_move_report() {
    access::assert_landfall_library_move_report();
}

/// Require an actual finished turn and its one-game report denominator.
fn assert_finished_turn(report: &SimReport, log: &GameLog) {
    assert_eq!(report.outcomes.requested_games, 1);
    assert_eq!(report.outcomes.finished_games, 1);
    assert_eq!(report.outcomes.player_loss_games, 0);
    assert_eq!(report.outcomes.incomplete_games, 0);
    assert_eq!(log.completion, GameCompletion::HorizonReached);
    assert_eq!(report.phase_observations.end_turn_by_turn[&1], 1.0);
    assert_eq!(report.phase_observations.main_by_turn[&1], 1.0);
    assert_eq!(report.draw.avg_life_paid, Some(f64::from(log.life_paid)));
    assert!(report.win_conditions.p50_lethal_turn.is_none());
}

/// Require the report to state the scoped policy used by a synthetic control.
fn assert_assumption(report: &SimReport, text: &str) {
    assert!(
        report.assumptions.iter().any(|line| line.contains(text)),
        "{text}"
    );
}

/// Keep missing pinned sources explicit rather than promoting synthetic cards to certificates.
fn assert_unpinned_keyword_scope(keywords: &[&str]) {
    for source in crate::deck::simulator::card_test_support::source_cards() {
        let stored = source["metadata"]["keywords"]
            .as_array()
            .expect("pinned source keywords are an array");
        for keyword in keywords {
            assert!(
                stored.iter().all(|value| value.as_str() != Some(*keyword)),
                "{} now has a pinned {keyword} source; review its printed branch separately",
                source["name"]
            );
        }
    }
}

/// Keep the exact printed body data in a synthetic creature control.
fn body(name: &str, cost: &str, text: &str, power: u32) -> CardRow {
    let mut source = row(name, cost, "Creature", text);
    source.power = Some(power.to_string());
    source.toughness = Some("20".into());
    source
}

/// Synthetic cascade boundaries reach the actual public report.
#[test]
fn cascade_boundaries_execute_and_report() {
    assert_cascade_boundaries_report();
}

/// Synthetic first-bench companion access pays once and reaches the report.
#[test]
fn companion_access_executes_and_reports() {
    assert_companion_access_report();
}

/// Synthetic face routes keep their declared payment and timing limits.
#[test]
fn face_choices_execute_and_report() {
    assert_face_choices_report();
}

/// Readiness negatives and spare-mana counts use actual turns.
#[test]
fn interaction_readiness_executes_and_reports() {
    assert_interaction_readiness_report();
}

/// Synthetic keyword rewards retain their stated approximation limits.
#[test]
fn keyword_limits_execute_and_report() {
    assert_keyword_limits_report();
}

/// Synthetic unknown land-entry wording never gives free untapped mana.
#[test]
fn land_entry_safety_executes_and_reports() {
    assert_land_entry_safety_report();
}

/// A pinned landfall observer ignores a library move that does not enter play.
#[test]
fn landfall_library_move_executes_and_reports() {
    assert_landfall_library_move_report();
}
