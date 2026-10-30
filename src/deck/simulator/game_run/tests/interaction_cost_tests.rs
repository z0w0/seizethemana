//! Interaction capacity includes live reductions and mandatory Behold mana.

use super::*;
use crate::deck::simulator::game::{ManaPool, new_perm_with};
use crate::deck::simulator::game_mana::ManaLot;
use crate::deck::simulator::model::SpendRestriction;
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// A live typed reducer changes capacity without spending a source or mana.
#[test]
fn interaction_capacity_uses_live_reducers() {
    let cards = deck(&[
        row("Answer", "{2}{U}", "Instant", "Counter target spell."),
        row(
            "Reducer",
            "{2}",
            "Artifact",
            "Instant and sorcery spells you cast cost {1} less to cast.",
        ),
    ]);
    for (present, face_down, ready) in [
        (false, false, false),
        (true, false, true),
        (true, true, false),
    ] {
        let mut game = state(vec![0], Vec::new());
        if present {
            let mut source = new_perm_with(1, &cards, CardIdx(1), 0, false);
            source.face_down = face_down;
            game.battlefield.push(source);
        }
        let pool = ManaPool {
            fixed: [0, 1, 0, 0, 0],
            colorless: 1,
            ..Default::default()
        };
        let mut census = TurnCensus::new(1, cards.cards.len());
        record_interaction_readiness(&cards, &game, &pool, &mut census, 1);
        assert_eq!(census.interaction_ready, [ready]);
        assert_eq!(census.interaction_mana_held, [0.0]);
        assert_eq!(pool.total(), 2);
        assert!(game.battlefield.iter().all(|source| !source.tapped));
    }
}

/// A pending answer cannot reveal itself, but another matching hand card qualifies.
#[test]
fn interaction_capacity_includes_behold_mana_and_source_exclusion() {
    let cards = deck(&[
        row(
            "Elf answer",
            "{U}",
            "Kindred Instant — Elf",
            "As an additional cost to cast this spell, behold an Elf or pay {1}.\nCounter target spell.",
        ),
        row("Elf witness", "{9}", "Creature — Elf", ""),
    ]);
    for (witness, generic, ready) in [(false, 0, false), (false, 1, true), (true, 0, true)] {
        let game = state(if witness { vec![0, 1] } else { vec![0] }, Vec::new());
        let pool = ManaPool {
            fixed: [0, 1, 0, 0, 0],
            colorless: generic,
            ..Default::default()
        };
        let mut census = TurnCensus::new(1, cards.cards.len());
        record_interaction_readiness(&cards, &game, &pool, &mut census, 1);
        assert_eq!(census.interaction_ready, [ready]);
        assert_eq!(census.interaction_mana_held, [0.0]);
        assert_eq!(pool.total(), 1 + generic);
        assert_eq!(game.hand.len(), if witness { 2 } else { 1 });
        assert!(game.graveyard.is_empty());
    }
}

/// Readiness skips unpayable answers and keeps hand-order ties and class limits.
#[test]
fn interaction_capacity_keeps_the_selected_payment_and_its_classes() {
    let cards = deck(&[
        row(
            "Unpayable answer",
            "{B}",
            "Instant",
            "Counter target spell.",
        ),
        row(
            "Legendary answer",
            "{U}",
            "Legendary Instant",
            "Counter target spell.",
        ),
        row("Ordinary answer", "{U}", "Instant", "Counter target spell."),
        row(
            "Costly answer",
            "{U}{U}",
            "Instant",
            "Counter target spell.",
        ),
    ]);
    for (hand, held) in [(vec![0, 1, 2, 3], 2.0), (vec![0, 2, 1, 3], 0.0)] {
        let game = state(hand, Vec::new());
        let pool = ManaPool {
            fixed: [0, 1, 0, 0, 0],
            lots: vec![ManaLot {
                amount: 2,
                types: 2,
                restriction: Some(SpendRestriction::Legendary),
                cannot_pay_generic: false,
                one_color: false,
            }],
            ..Default::default()
        };
        let mut census = TurnCensus::new(1, cards.cards.len());
        record_interaction_readiness(&cards, &game, &pool, &mut census, 1);
        assert_eq!(census.interaction_ready, [true]);
        assert_eq!(census.interaction_mana_held, [held]);
        assert_eq!(pool.fixed, [0, 1, 0, 0, 0]);
        assert_eq!(pool.lots[0].amount, 2);
        assert_eq!(game.hand.len(), 4);
        assert!(game.graveyard.is_empty());
    }
}
