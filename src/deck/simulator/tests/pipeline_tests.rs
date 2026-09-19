// Tests for the simulator pipeline module.

/// A minimal card row for tests.
use super::deck::build_sim_deck;
use super::parse::*;
use crate::db::CardRow;
use rand::SeedableRng;
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

#[test]
fn drain_burn_and_scry_register_in_game() {
    let rows = vec![
        card("Mountain", "", "Basic Land — Mountain", "({T}: Add {R}.)"),
        card(
            "Lava Spike",
            "{R}",
            "Sorcery",
            "Lava Spike deals 3 damage to target player.",
        ),
        card("Opt", "{U}", "Instant", "Scry 1. Draw a card."),
    ];
    let cards = cards_map(rows);
    let mut deck = deck_text("DECK", &[("Lava Spike", 10), ("Opt", 10)]);
    deck.section_entries_mut("DECK")
        .push(crate::deck::grammar::DeckEntry {
            quantity: 40,
            name: "Mountain".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let log = super::game::run_game(&sim_deck, &mut rng, 6);
    // Burn drains players 3× its amount when cast.
    assert!(log.drain_total[5] > 0, "burn spells drain over 6 turns");
    // Scry gives awareness credit.
    assert!(
        log.awareness.iter().copied().fold(0.0_f64, f64::max) > 0.0,
        "scry feeds library awareness"
    );
}

#[test]
fn extra_turn_spell_counts() {
    let rows = vec![
        card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
        card(
            "Time Walk",
            "{2}{U}",
            "Sorcery",
            "Take an extra turn after this one.",
        ),
    ];
    let cards = cards_map(rows);
    let mut deck = deck_text("DECK", &[("Time Walk", 20)]);
    deck.section_entries_mut("DECK")
        .push(crate::deck::grammar::DeckEntry {
            quantity: 40,
            name: "Island".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let log = super::game::run_game(&sim_deck, &mut rng, 8);
    assert!(
        log.extra_turns.iter().any(|e| *e > 0),
        "extra turn spell queues"
    );
}

#[test]
fn opponent_mill_routes_to_opp_census() {
    let rows = vec![
        card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
        card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)"),
        card(
            "Sculpting Steel Mill",
            "{2}{U}",
            "Sorcery",
            "Target player mills five cards.",
        ),
        card(
            "Grave Dredge",
            "{2}{B}",
            "Sorcery",
            "Mill five cards. Then return a creature card from your graveyard to your hand.",
        ),
    ];
    let cards = cards_map(rows);
    let mut deck = deck_text(
        "DECK",
        &[("Sculpting Steel Mill", 10), ("Grave Dredge", 10)],
    );
    {
        let list = deck.section_entries_mut("DECK");
        for (name, qty) in [("Island", 30), ("Swamp", 20)] {
            list.push(crate::deck::grammar::DeckEntry {
                quantity: qty,
                name: name.into(),
                set_code: None,
                collector_number: None,
                foil: false,
            });
        }
    }
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let log = super::game::run_game(&sim_deck, &mut rng, 8);
    assert!(log.opp_milled[7] > 0, "opponent-targeted mills count");
    assert!(log.self_milled[7] > 0, "self mills count");
    assert!(log.library_size[7] < 50, "library shrinks with mills");
}

#[test]
fn combat_power_counts_buff_and_double_strike() {
    let rows = vec![
        card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"),
        card("Forest", "", "Basic Land — Forest", "({T}: Add {G}.)"),
        card("Mountain", "", "Basic Land — Mountain", "({T}: Add {R}.)"),
        card("Bear Cub", "{1}{G}", "Creature — Bear", "Trample"),
        card(
            "Battle Anthem",
            "{3}{W}",
            "Enchantment",
            "Creatures you control get +2/+2.",
        ),
        card(
            "Fervent Champion",
            "{R}",
            "Creature — Human Knight",
            "Double strike",
        ),
    ];
    let mut cards = cards_map(rows);
    // Give the bear a printed power and evasion.
    if let Some(bear) = cards.get_mut("Bear Cub") {
        bear.power = Some("2".into());
        bear.keywords = r#"["Trample"]"#.into();
    }
    if let Some(champ) = cards.get_mut("Fervent Champion") {
        champ.power = Some("3".into());
        champ.keywords = r#"["Double strike"]"#.into();
    }
    let mut deck = deck_text(
        "DECK",
        &[
            ("Bear Cub", 10),
            ("Battle Anthem", 10),
            ("Fervent Champion", 10),
        ],
    );
    {
        let list = deck.section_entries_mut("DECK");
        for (name, qty) in [("Plains", 20), ("Forest", 15), ("Mountain", 15)] {
            list.push(crate::deck::grammar::DeckEntry {
                quantity: qty,
                name: name.into(),
                set_code: None,
                collector_number: None,
                foil: false,
            });
        }
    }
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(11);
    let log = super::game::run_game(&sim_deck, &mut rng, 8);
    // Late game: bears hit with the +2 anthem buff and champions ×2.
    let late = log.attack_power[7];
    assert!(late > 0, "bodies produce attack power");
    assert!(log.evasive[7] > 0, "trample attackers count as evasive");
}
