//! Turn-boundary report evidence for the validated branch inventory.
//!
//! These synthetic controls do not add pinned-card or whole-card certificates.
//! Existing source packets retain their separate scope and compiler limits.

/// Physical event filters, captured listeners, and independent usage ledgers.
#[path = "turn_event_report_tests.rs"]
mod events;
/// Trigger work limits, linked losses, and retained pre-loss observations.
#[path = "turn_loss_report_tests.rs"]
mod losses;
/// Phase boundaries, scheduled extra slots, opponent rounds, and partner potential.
#[path = "turn_schedule_report_tests.rs"]
mod schedule;
/// Upkeep event-time capture and resolution-time condition checks.
#[path = "turn_upkeep_report_tests.rs"]
mod upkeep;

/// Close the generic nontoken and captured-entry-listener report stages.
pub(super) fn assert_entry_filter_capture_report() {
    events::assert_entry_filter_capture_report();
}

/// Close simultaneous-death and last-known token/card report stages.
pub(super) fn assert_simultaneous_death_report() {
    events::assert_simultaneous_death_report();
}

/// Close supported condition and independent source usage-ledger report stages.
pub(super) fn assert_condition_usage_report() {
    events::assert_condition_usage_report();
}

/// Close exact-finish and unresolved trigger-settlement report stages.
pub(super) fn assert_settlement_cap_report() {
    losses::assert_settlement_cap_report();
}

/// Close first-main, end-step, and same-turn Monarch report stages.
pub(super) fn assert_phase_boundary_report() {
    schedule::assert_phase_boundary_report();
}

/// Close linked failed-draw, cascade-child, and pre-loss access report stages.
pub(super) fn assert_loss_boundary_report() {
    losses::assert_loss_boundary_report();
}

/// Close configured extra-slot actions and delayed opponent-round report stages.
pub(super) fn assert_extra_slot_report() {
    schedule::assert_extra_slot_report();
}

/// Close separate partner potential rates and observed-game denominator stages.
pub(super) fn assert_partner_potential_report() {
    schedule::assert_partner_potential_report();
}

/// Close opponent-round damage and life-loss timing after the player census.
pub(super) fn assert_opponent_round_damage_report() {
    schedule::assert_opponent_round_damage_report();
}

/// Require upkeep conditions before any captured source changes the graveyard.
pub(super) fn assert_upkeep_condition_capture_report() {
    upkeep::assert_upkeep_condition_capture_report();
}

/// Keep a captured upkeep target tied to its original graveyard visit.
pub(super) fn assert_upkeep_target_generation_report() {
    upkeep::assert_upkeep_target_generation_report();
}

/// Execute the generic entry-filter and listener-capture packet.
#[test]
fn entry_filter_capture_report() {
    assert_entry_filter_capture_report();
}
/// Execute the simultaneous-death packet without live listener assumptions.
#[test]
fn simultaneous_death_report() {
    assert_simultaneous_death_report();
}
/// Execute supported conditions and separate usage ledgers.
#[test]
fn condition_usage_report() {
    assert_condition_usage_report();
}
/// Execute both sides of the trigger-settlement cap.
#[test]
fn settlement_cap_report() {
    assert_settlement_cap_report();
}
/// Execute the first-main, end-step, and Monarch boundaries.
#[test]
fn phase_boundary_report() {
    assert_phase_boundary_report();
}
/// Execute linked losses and retain their reached observations.
#[test]
fn loss_boundary_report() {
    assert_loss_boundary_report();
}
/// Execute extra slots through the production game driver.
#[test]
fn extra_slot_report() {
    assert_extra_slot_report();
}
/// Execute separate partner potential reports.
#[test]
fn partner_potential_report() {
    assert_partner_potential_report();
}
/// Execute post-census opponent-round damage reports.
#[test]
fn opponent_round_damage_report() {
    assert_opponent_round_damage_report();
}

/// Execute event-time and resolution-time upkeep condition controls.
#[test]
fn upkeep_condition_capture_report() {
    assert_upkeep_condition_capture_report();
}

/// Execute a real return, discard, and stale-target upkeep sequence.
#[test]
fn upkeep_target_generation_report() {
    assert_upkeep_target_generation_report();
}
