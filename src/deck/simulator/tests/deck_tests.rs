// Tests for the simulator deck module.

/// A minimal card row for tests.
use super::deck::build_sim_deck;
use super::model::*;
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
fn deck_build_splits_commander_and_sideboard() {
    let cards = cards_map(vec![
        card(
            "IGS",
            "{W}{U}{B}{R}{G}",
            "Legendary Artifact — Spacecraft",
            "Station (It's an artifact creature at 12+.)\n12+ | Flying\nWhenever IGS attacks, draw a card for each multicolored permanent you control.",
        ),
        card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"),
        card("Bolt", "{R}", "Instant", "Deal 3 damage."),
    ]);
    let mut deck = deck_text("COMMANDER", &[("IGS", 1)]);
    deck.section_entries_mut("DECK")
        .push(crate::deck::grammar::DeckEntry {
            quantity: 30,
            name: "Plains".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    deck.section_entries_mut("DECK")
        .push(crate::deck::grammar::DeckEntry {
            quantity: 2,
            name: "Bolt".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    deck.section_entries_mut("SIDEBOARD")
        .push(crate::deck::grammar::DeckEntry {
            quantity: 1,
            name: "Bolt".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    let sim = build_sim_deck(&deck, &cards, None);
    assert_eq!(sim.format, Format::Commander);
    assert_eq!(sim.commanders.len(), 1);
    assert_eq!(sim.commanders[0].name, "IGS");
    // Sideboard excluded: 30 + 2, not 30 + 3.
    assert_eq!(sim.cards.len(), 32);
    // An attack-gated draw gets no synthetic engine tier: it fires through
    // the combat path once the spacecraft animates.
    assert!(
        !sim.commanders[0]
            .station_tiers
            .iter()
            .any(|t| t.at == 0 && t.abilities.iter().any(|a| a.trigger == Trigger::OnUpkeep))
    );
    // A real upkeep draw does get the synthetic engine.
    let upkeep_cmd = card(
        "Oracle",
        "{3}{U}",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
    );
    let cards2 = cards_map(vec![
        upkeep_cmd,
        card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"),
    ]);
    let mut deck2 = deck_text("COMMANDER", &[("Oracle", 1)]);
    deck2
        .section_entries_mut("DECK")
        .push(crate::deck::grammar::DeckEntry {
            quantity: 30,
            name: "Plains".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    let sim2 = build_sim_deck(&deck2, &cards2, None);
    assert!(
        sim2.commanders[0]
            .station_tiers
            .iter()
            .any(|t| t.at == 0 && t.abilities.iter().any(|a| a.trigger == Trigger::OnUpkeep))
    );
}

#[test]
fn unknown_cards_count_but_stay_passive() {
    let cards: HashMap<String, CardRow> = HashMap::new();
    let deck = deck_text("DECK", &[("Mystery Card", 3)]);
    let sim = build_sim_deck(&deck, &cards, None);
    assert_eq!(sim.cards.len(), 3);
    assert_eq!(sim.cards[0].role, Role::Other);
    assert_eq!(sim.cards[0].cost.total(), 0);
}

#[test]
fn format_override_merges_commander_into_library() {
    let cards = cards_map(vec![
        card("Boss", "{4}", "Legendary Creature — Boss", ""),
        card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"),
    ]);
    let mut deck = deck_text("COMMANDER", &[("Boss", 1)]);
    deck.section_entries_mut("DECK")
        .push(crate::deck::grammar::DeckEntry {
            quantity: 30,
            name: "Plains".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    let mut sim = build_sim_deck(&deck, &cards, None);
    assert!(super::deck::apply_format_override(&mut sim, "modern"));
    assert_eq!(sim.format, Format::Constructed);
    assert_eq!(sim.commanders.len(), 0);
    assert_eq!(sim.cards.len(), 31);
    assert!(!super::deck::apply_format_override(&mut sim, "commander"));
}

// Aggregation + findings

#[test]
fn jegantha_five_color_rock_counts_once() {
    let jegantha = card(
        "Jegantha, the Wellspring",
        "{5}",
        "Legendary Creature — Elemental Elk",
        "Companion — No card in your starting deck has more than one of the same mana symbol in its mana cost.\n{T}: Add {W}{U}{B}{R}{G}. This mana can't be spent to pay generic mana costs.",
    );
    let sim = parse_sim_card(&jegantha);
    let tap = sim.tap.expect("Jegantha taps for mana");
    // One tap = five simultaneous pips, one source.
    assert_eq!(tap.total(), 5);
    for p in tap.fixed {
        assert_eq!(p, 1);
    }
    assert_eq!(sim.role, Role::Dork);
}
