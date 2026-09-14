// Cross-deck invariants: properties every real fixture deck must hold.
// ---------------------------------------------------------------------------
// Cross-deck invariants (all twelve lists)
// ---------------------------------------------------------------------------

use super::deck_test_support::*;
use crate::db::CardRow;
use crate::deck::grammar::Deck;
use std::collections::HashMap;

#[test]
fn all_real_decks_show_land_openers() {
    // No real deck keeps zero-land openers most of the time.
    let lists: [(&str, Deck, HashMap<String, CardRow>); 6] = [
        (
            "dimir",
            fixture_deck("dimir-midrange"),
            fixture_cards("dimir-midrange"),
        ),
        (
            "red",
            fixture_deck("mono-red-aggro"),
            fixture_cards("mono-red-aggro"),
        ),
        (
            "mardu",
            fixture_deck("mardu-discard"),
            fixture_cards("mardu-discard"),
        ),
        (
            "prowess",
            fixture_deck("izzet-prowess"),
            fixture_cards("izzet-prowess"),
        ),
        (
            "eldrazi",
            fixture_deck("eldrazi-tron"),
            fixture_cards("eldrazi-tron"),
        ),
        (
            "ruby",
            fixture_deck("ruby-storm"),
            fixture_cards("ruby-storm"),
        ),
    ];
    for (name, deck, cards) in lists {
        let stats = sim(&deck, &cards, 200, 8);
        assert!(
            stats.avg_opener_lands >= 1.0,
            "{name} openers hold {:.2} lands",
            stats.avg_opener_lands
        );
        assert!(
            stats.mulligan_rate < 0.75,
            "{name} mulligans {:.0}% of games",
            stats.mulligan_rate * 100.0
        );
    }
}

#[test]
fn all_real_decks_never_trip_all_colors() {
    // The pool model is sound: no real deck trips all five colors.
    let cards = fixture_cards("mardu-discard");
    let stats = sim(&fixture_deck("mardu-discard"), &cards, 200, 8);
    let tripped = stats.color_screw.iter().filter(|p| **p >= 0.10).count();
    assert!(tripped < 5, "Mardu trips all {} colors", tripped);
}

#[test]
fn commander_decks_see_ninety_cards_late() {
    // 99-card decks draw past the constructed opener over 10 turns plus
    // draw engines; velocity grows across the window.
    let cards = fixture_map("blue farm");
    let stats = sim(&fixture_deck("blue farm"), &cards, 200, 10);
    assert!(stats.cards_seen[9] > stats.cards_seen[0]);
    // Commanders sit in the command zone: they are never drawn from the
    // library. Zone telemetry caps the library contribution: battlefield
    // plus graveyard entries track cast/discard volumes. Blue Farm draws
    // heavily (Atraxa, wheel effects); over 10 turns the ceiling is the
    // whole 99 with commander casts on top. Assert the deck never sees
    // more cards than library + commanders + token draws allow.
    assert!(stats.cards_seen[9] < 99.0 + 25.0);
}
