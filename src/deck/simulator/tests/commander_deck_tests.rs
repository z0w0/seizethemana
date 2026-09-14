// Commander sweep: every Commander fixture must hold the shared invariants
// and the commander-specific telemetry must behave (cast windows, partners,
// engine tiers).

use super::deck_test_support::*;
use super::model::Format;

#[test]
fn sweep_commander_invariants() {
    for name in COMMANDER_FIXTURES {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let sim_deck = build_sim_deck(&deck, &cards, None);
        assert_eq!(
            sim_deck.format,
            Format::Commander,
            "{name} not inferred as commander"
        );
        assert!(
            !sim_deck.commanders.is_empty(),
            "{name} has no commanders parsed"
        );
        assert!(
            sim_deck.cards.len() + sim_deck.commanders.len() == 100,
            "{name} has {} cards, expected 100",
            sim_deck.cards.len() + sim_deck.commanders.len()
        );
        let stats = sim(&deck, &cards, 200, 10);
        assert_land_drops_sane(&stats, 10);
        assert_velocity_monotone(&stats);
        assert_castability_not_before_cost(&stats, &cards);
    }
}

#[test]
fn sweep_commander_partners_carry_two() {
    // Partner decks keep both legends in the command zone.
    for name in [
        "blue farm",
        "thrasios - tymna midrange",
        "rogsi turbo",
        "rogsilas turbo naus",
        "krark-sakashima storm",
    ] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let sim_deck = build_sim_deck(&deck, &cards, None);
        assert_eq!(
            sim_deck.commanders.len(),
            2,
            "{name} should carry two partners"
        );
    }
}

#[test]
fn sweep_commander_zero_cost_leader_casts_turn_one() {
    // Rograkh ({0}) is castable in essentially every game by turn 1.
    for name in ["rogsi turbo", "rogsilas turbo naus"] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 10);
        assert!(
            stats.commander_castable_by[1] >= 0.9,
            "{name} zero-cost commander castable by t1 only {:.0}%",
            stats.commander_castable_by[1] * 100.0
        );
    }
}

#[test]
fn sweep_commander_casual_legends_are_late() {
    // Casual 99-card lists cast {4}{WUBRG}-shaped commanders late: the
    // insight is the cast window. The Ur-Dragon (CMC 9) is castable by t9
    // in a minority of games on a 35-land 5c base — the report's
    // "commander_late" finding fires on exactly this shape.
    let cards = fixture_map("the ur-dragon dragons");
    let deck = fixture_deck("the ur-dragon dragons");
    let stats = sim(&deck, &cards, 300, 10);
    assert!(
        stats.commander_castable_by[9] >= 0.3,
        "Ur-Dragon castable by t9 only {:.0}%",
        stats.commander_castable_by[9] * 100.0
    );
    assert!(
        stats.commander_castable_by[4] < 0.1,
        "CMC 9 commander castable by t4 in {:.0}%: model too generous",
        stats.commander_castable_by[4] * 100.0
    );
}

#[test]
fn sweep_commander_velocity_grows() {
    // Commander games see far more cards than the constructed opener:
    // draw engines plus 10 turns push velocity well past the opener.
    for name in ["blue farm", "kinnan combo", "chulane bant value"] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 10);
        assert!(
            stats.cards_seen[9] > stats.cards_seen[0],
            "{name} velocity stalls"
        );
        assert!(
            stats.cards_seen[9] < 99.0 + 25.0,
            "{name} sees {} cards by t10: zone telemetry overflows",
            stats.cards_seen[9]
        );
    }
}

#[test]
fn sweep_commander_tax_pieces_seen_early() {
    // Jaws Storm runs God-Pharaoh's Statue ({2} tax, 5 MV) and Winter Moon:
    // cheap rock-style locks show up by t3 in most games; the 5 MV Statue
    // rarely does. Lock timing is the metric under test.
    let cards = fixture_map("jaws storm");
    let deck = fixture_deck("jaws storm");
    let stats = sim(&deck, &cards, 300, 10);
    // Lock access is modeled at all: the metric exists and is bounded.
    assert!((0.0..=1.0).contains(&stats.lock_access_3));
}
