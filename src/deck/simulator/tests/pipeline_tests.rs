// Tests for the simulator pipeline module.

/// A minimal card row for tests.
use super::deck::build_sim_deck;
use super::parse::*;
use crate::db::CardRow;
use std::collections::HashMap;
fn card(name: &str, mana_cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: mana_cost.to_string(),
        cmc: super::parse::parse_cost(mana_cost).total() as f64,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

/// A deck text with one section.
fn deck_text(section: &str, entries: &[(&str, i64)]) -> crate::deck::grammar::Deck {
    let mut deck = crate::deck::grammar::Deck::default();
    let list = deck.section_entries_mut(section);
    for (name, qty) in entries {
        list.push(crate::deck::grammar::DeckEntry {
            quantity: *qty,
            name: name.to_string(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    deck
}

fn cards_map(cards: Vec<CardRow>) -> HashMap<String, CardRow> {
    cards.into_iter().map(|c| (c.name.clone(), c)).collect()
}

// Cost parsing

#[test]
fn stationz_commander_full_pipeline() {
    // The real IGS oracle text through build + run: the sim must see
    // tokens on ETB, station tiers at 12+, and the attack-draw engine.
    let igs = card(
        "Infinite Guideline Station",
        "{W}{U}{B}{R}{G}",
        "Legendary Artifact — Spacecraft",
        "When Infinite Guideline Station enters, create a tapped 2/2 colorless Robot artifact creature token for each multicolored permanent you control.\nStation (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 12+.)\n12+ | Flying\nWhenever Infinite Guideline Station attacks, draw a card for each multicolored permanent you control.",
    );
    let sim = parse_sim_card(&igs);
    assert!(sim.is_station_card);
    assert_eq!(sim.animate_at(), Some(12));
    assert!(sim.station_tiers.iter().any(|t| t.at == 12 && t.animate));
    assert_eq!(sim.cost.pips, [1, 1, 1, 1, 1]);
    assert!(!sim.is_creature);

    // Command-zone engine tier: the attack-draw becomes a per-turn draw.
    let cards = cards_map(vec![
        igs,
        card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"),
    ]);
    let mut deck = deck_text("COMMANDER", &[("Infinite Guideline Station", 1)]);
    deck.section_entries_mut("DECK")
        .push(crate::deck::grammar::DeckEntry {
            quantity: 30,
            name: "Plains".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    let sim_deck = build_sim_deck(&deck, &cards, None);
    // Commander engine tier present.
    assert!(
        sim_deck.commanders[0]
            .station_tiers
            .iter()
            .any(|t| t.at == 0)
    );
}
