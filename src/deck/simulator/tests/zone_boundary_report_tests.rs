//! Action and report controls for library, graveyard, and first-access boundaries.
//! Synthetic rows test engine behavior, not the complete rules of a named card.

/// Terminal access survives without creating unobserved turns or zone histories.
#[path = "zone_boundary_access_tests.rs"]
mod access;
/// Graveyard actions keep unsupported instructions and replay resources separate.
#[path = "zone_boundary_graveyard_tests.rs"]
mod graveyard;
/// Library actions keep selection, exposure, and life costs separate.
#[path = "zone_boundary_library_tests.rs"]
mod library;

/// Competing dredgers use the largest fully payable replacement on each draw.
pub(super) fn assert_largest_payable_dredge_report() {
    library::assert_largest_payable_dredge_report();
}

/// Unsupported board counts do not grant a reward after a paid spell action.
pub(super) fn assert_unsupported_scaled_draw_report() {
    library::assert_unsupported_scaled_draw_report();
}

/// Reveals use visible life and known deck risk, including the mandatory lethal reveal.
pub(super) fn assert_life_reveal_risk_report() {
    library::assert_life_reveal_risk_report();
}

/// Library-only listeners do not fire for discards, sacrifices, or opponent mill credit.
pub(super) fn assert_mill_origin_report() {
    library::assert_mill_origin_report();
}

/// Exposed-card choices keep scry order, surveil moves, and report measures separate.
pub(super) fn assert_scry_surveil_report() {
    library::assert_scry_surveil_report();
}

/// Unrepresented return requirements keep their complete linked reward inert.
pub(super) fn assert_filtered_return_safety_report() {
    graveyard::assert_filtered_return_safety_report();
}

/// Several legal graveyard targets preserve the existing newest-eligible policy.
pub(super) fn assert_first_eligible_return_report() {
    graveyard::assert_first_eligible_return_report();
}

/// Own Escape pays distinct physical costs and retains its normal spell destination.
pub(super) fn assert_own_escape_resources_report() {
    graveyard::assert_own_escape_resources_report();
}

/// Exact replay budgets distinguish finished actions from incomplete further work.
pub(super) fn assert_replay_cap_report() {
    graveyard::assert_replay_cap_report();
}

/// Terminal first visits survive aggregation without invented end-turn observations.
pub(super) fn assert_terminal_first_visits_report() {
    access::assert_terminal_first_visits_report();
}

/// Run each callable boundary group as an ordinary test too.
#[test]
fn largest_payable_dredge_action_and_report() {
    assert_largest_payable_dredge_report();
}

/// Keep unsupported scaling safety callable without changing the runtime-scale packet.
#[test]
fn unsupported_scaled_draw_action_and_report() {
    assert_unsupported_scaled_draw_report();
}

/// Keep mandatory and optional reveal controls together.
#[test]
fn life_reveal_risk_action_and_report() {
    assert_life_reveal_risk_report();
}

/// Keep player-zone moves separate from opponent mill counters.
#[test]
fn mill_origin_action_and_report() {
    assert_mill_origin_report();
}

/// Keep paid selection, unpaid controls, and report conservation in one callable group.
#[test]
fn scry_surveil_action_and_report() {
    assert_scry_surveil_report();
}

/// Keep unsupported return requirements separate from target selection.
#[test]
fn filtered_return_safety_action_and_report() {
    assert_filtered_return_safety_report();
}

/// Preserve the physical order of several legal return targets.
#[test]
fn first_eligible_return_action_and_report() {
    assert_first_eligible_return_report();
}

/// Keep own Escape distinct from granted Escape and flashback.
#[test]
fn own_escape_resources_action_and_report() {
    assert_own_escape_resources_report();
}

/// Preserve replay-cap outcomes and their sampling denominators.
#[test]
fn replay_cap_action_and_report() {
    assert_replay_cap_report();
}

/// Preserve terminal access without filling missing phases.
#[test]
fn terminal_first_visits_action_and_report() {
    assert_terminal_first_visits_report();
}
