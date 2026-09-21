use super::deck_test_support::*;
use crate::db::CardRow;
use crate::deck::grammar::{Deck, DeckEntry};
use std::collections::HashMap;

/// A combat-only fixture: vanilla bears, no drain text anywhere.
/// Player damage must be cumulative, so the deck reaches lethal when
/// the running total of attack power crosses 20.
#[test]
fn combat_only_deck_reaches_lethal() {
    let mut cards: HashMap<String, CardRow> = HashMap::new();
    cards.insert(
        "Forest".to_string(),
        real_card("Forest", "", "Basic Land — Forest", "", "({T}: Add {G}.)"),
    );
    // Bears: cheap, reasonable power, no abilities.
    cards.insert(
        "Grizzly Bears".to_string(),
        real_card("Grizzly Bears", "{1}{G}", "Creature — Bear", "", ""),
    );
    let bears = cards.get_mut("Grizzly Bears").unwrap();
    bears.power = Some("2".to_string());
    bears.toughness = Some("2".to_string());

    let deck = Deck {
        sections: vec![(
            "DECK".to_string(),
            vec![entry(24, "Forest"), entry(36, "Grizzly Bears")],
        )],
    };
    let stats = sim(&deck, &cards, 400, 10);
    let p50 = stats
        .p50_lethal_turn
        .expect("combat-only deck reaches lethal once damage accumulates");
    assert!(
        (2..=9).contains(&p50),
        "p50 lethal t{p50}: cumulative combat damage never registered"
    );
}

/// Life paid as an additional cast cost is not damage dealt; it must
/// not push the lethal census across the line.
#[test]
fn self_paid_life_does_not_inflate_lethal() {
    let mut cards: HashMap<String, CardRow> = HashMap::new();
    cards.insert(
        "Swamp".to_string(),
        real_card("Swamp", "", "Basic Land — Swamp", "", ""),
    );
    // A creature with a life-payment additional cost, no other text.
    cards.insert(
        "Blood Tithe".to_string(),
        real_card(
            "Blood Tithe",
            "{1}{B}",
            "Creature — Horror",
            "",
            "As an additional cost to cast this spell, pay 2 life.",
        ),
    );
    let beast = cards.get_mut("Blood Tithe").unwrap();
    beast.power = Some("1".to_string());
    beast.toughness = Some("1".to_string());

    let deck = Deck {
        sections: vec![(
            "DECK".to_string(),
            vec![entry(24, "Swamp"), entry(36, "Blood Tithe")],
        )],
    };
    let stats = sim(&deck, &cards, 400, 10);
    // 1-power bears are slow; life payments alone must never read as lethal.
    for (t, p) in stats.lethal_damage_by_turn.iter().enumerate() {
        assert!(
            *p <= 0.5,
            "t{t}: lethal pct {p} — self-paid life leaked into the census"
        );
    }
}

fn entry(quantity: i64, name: &str) -> DeckEntry {
    DeckEntry {
        quantity,
        name: name.to_string(),
        set_code: None,
        collector_number: None,
        foil: false,
    }
}
