//! Permanent-branch evidence uses synthetic sources and actual production reports.
//!
//! `counters.power_equipment`: static power, paid host identity, and hidden power
//! complement `permanent_core_report_tests::assert_counter_consumer_reports`.
//! `permanents.crew_station`: sick crew and Vehicle attack readiness complement
//! the existing vehicle and station callbacks. They do not count station again.
//! `permanents.loyalty`: zero stored loyalty, minus-cost funding, and a zero-cost
//! loyalty symbol complement the existing ultimate-affordability callback.
//! `permanents.saga`: linked chapter resolution complements scheduled lore and
//! read-ahead coverage. It does not certify arbitrary lore placement.
//! `permanents.blink_subset`, `cast.legend_rule`, and `hand.cleanup_policy` each
//! receive positive, nearby negative, complete-action, and executed-report checks.
//! These helpers supply evidence, not binding or whole-card coverage counts.
//! Packet bindings retain the existing callback limits.

/// Cleanup order and supported live hand-size sources.
#[path = "permanent_cleanup_boundary_report_tests.rs"]
mod cleanup;
/// Blink-shaped events, loyalty payments, compound chapters, and legendary copies.
#[path = "permanent_event_boundary_report_tests.rs"]
mod events;
/// Shared-power consumers and physical crew readiness.
#[path = "permanent_power_boundary_report_tests.rs"]
mod power;

/// Bind supported static bonuses to combat, crew, and station reports.
pub(super) fn assert_static_power_consumers_report() {
    power::assert_static_power_consumers_report();
}

/// Bind actual equip payment and stable physical hosts to all power consumers.
pub(super) fn assert_equipment_host_report() {
    power::assert_equipment_host_report();
}

/// Bind face-down and unknown-power limits to executed power-consumer reports.
pub(super) fn assert_hidden_power_consumers_report() {
    power::assert_hidden_power_consumers_report();
}

/// Bind sick crew payment and new-Vehicle attack rejection to actual reports.
pub(super) fn assert_sick_crew_report() {
    power::assert_sick_crew_report();
}

/// Bind the one-shot deferred entry subset and ordinary search controls.
pub(super) fn assert_blink_subset_report() {
    events::assert_blink_subset_report();
}

/// Bind zero stored loyalty and unaffordable minus payments without claiming an ultimate.
pub(super) fn assert_loyalty_rejection_report() {
    events::assert_loyalty_rejection_report();
}

/// Bind complete compound Saga resolution to actual loss and survivor denominators.
pub(super) fn assert_compound_saga_report() {
    events::assert_compound_saga_report();
}

/// Bind same-name legendary cleanup and its distinct-name and nonlegendary controls.
pub(super) fn assert_legend_rule_report() {
    events::assert_legend_rule_report();
}

/// Bind highest-value cleanup, live limit removal, and graveyard-only access reports.
pub(super) fn assert_cleanup_policy_report() {
    cleanup::assert_cleanup_policy_report();
}

/// Check static bonuses without repeating existing counter packets.
#[test]
fn static_power_consumers_report() {
    assert_static_power_consumers_report();
}

/// Check paid Equipment host identity and its resource controls.
#[test]
fn equipment_host_report() {
    assert_equipment_host_report();
}

/// Check hidden characteristics and the unknown-power fallback.
#[test]
fn hidden_power_consumers_report() {
    assert_hidden_power_consumers_report();
}

/// Check sick crew bodies and Vehicle entry-turn attack rejection.
#[test]
fn sick_crew_report() {
    assert_sick_crew_report();
}

/// Check deferred entry rewards without claiming a real exile-and-return event.
#[test]
fn blink_subset_report() {
    assert_blink_subset_report();
}

/// Check loyalty payment rejection and the supported zero loyalty symbol.
#[test]
fn loyalty_rejection_report() {
    assert_loyalty_rejection_report();
}

/// Check compound chapter effects before final-chapter cleanup or player loss.
#[test]
fn compound_saga_report() {
    assert_compound_saga_report();
}

/// Check the declared earlier-copy legend policy and nearby unaffected controls.
#[test]
fn legend_rule_report() {
    assert_legend_rule_report();
}

/// Check cleanup zone changes without granting draw or library-only rewards.
#[test]
fn cleanup_policy_report() {
    assert_cleanup_policy_report();
}
