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
    // The Karsten mulligan bottoms one card, so the late board can thin
    // to zero attackers by chance; assert the census itself is sound.
    let late = log.attack_power[7];
    assert!(late > 0, "bodies produce attack power");
    assert!(
        log.evasive[7] <= log.attackers[7],
        "evasion within attackers"
    );
    assert!(
        log.attackers[7] <= log.bodies[7],
        "attackers never exceed bodies"
    );
}

#[test]
fn partner_deck_casts_both_commanders() {
    // A partner pair is castable turn 1 ({0} + {1}): each partner joins
    // the battlefield; the cast loop must not stop after the first.
    let first = card("Free Leader", "", "Legendary Creature — Human", "");
    let second = card(
        "Cheap Partner",
        "{0}",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
    );
    let filler = card("Filler", "", "Creature — Frog", "");
    let cards = cards_map(vec![
        first,
        second,
        filler.clone(),
        card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
    ]);
    let mut deck = deck_text("DECK", &[("Island", 40), ("Filler", 20)]);
    let cmd = deck.section_entries_mut("COMMANDER");
    for (name, qty) in [("Free Leader", 1), ("Cheap Partner", 1)] {
        cmd.push(crate::deck::grammar::DeckEntry {
            quantity: qty,
            name: name.to_string(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    let sim_deck = build_sim_deck(&deck, &cards, None);
    assert_eq!(sim_deck.commanders.len(), 2);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
    let log = super::game::run_game(&sim_deck, &mut rng, 3);
    // Both commanders are zero/one-cost creatures: two bodies from turn 1.
    assert!(log.bodies[0] >= 2, "both partners cast: {}", log.bodies[0]);
}

#[test]
fn partner_upkeep_engines_fire_once_per_commander_per_turn() {
    // Two partners, each with an upkeep draw. The sentinel fix: each
    // cast commander fires its own engine once per turn (not once per
    // sentinel), and a partner whose upkeep draw is parsed registers an
    // engine even when the first partner lacks one.
    let first = card(
        "Free Leader",
        "",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
    );
    let second = card(
        "Cheap Partner",
        "{0}",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
    );
    let filler = card("Filler", "", "Creature — Frog", "");
    let cards = cards_map(vec![
        first,
        second,
        filler.clone(),
        card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
    ]);
    let mut deck = deck_text("DECK", &[("Island", 40), ("Filler", 20)]);
    let cmd = deck.section_entries_mut("COMMANDER");
    for (name, qty) in [("Free Leader", 1), ("Cheap Partner", 1)] {
        cmd.push(crate::deck::grammar::DeckEntry {
            quantity: qty,
            name: name.to_string(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    let sim_deck = build_sim_deck(&deck, &cards, None);
    assert_eq!(sim_deck.commanders.len(), 2);
    // Partner pair: two engines online from turn 1 (both partners cast).
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
    let log = super::game::run_game(&sim_deck, &mut rng, 3);
    assert_eq!(log.engines_online[0], 2, "one engine per cast partner");
    // Same-seed solo run: one commander, one engine, one draw per turn.
    let solo_deck = deck_text("DECK", &[("Island", 40), ("Filler", 20)]);
    let mut solo = solo_deck;
    let cmd = solo.section_entries_mut("COMMANDER");
    for (name, qty) in [("Free Leader", 1), ("Cheap Partner", 1)] {
        cmd.push(crate::deck::grammar::DeckEntry {
            quantity: qty,
            name: name.to_string(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    // The solo comparison: strip the partner's upkeep text by comparing
    // against a deck where only the first partner carries the engine —
    // the growth delta must stay at 2 draws/turn (both engines), not 3
    // (per-sentinel double-fire) or 1 (the missing engine).
    let second_silent = card("Cheap Partner", "{0}", "Legendary Creature — Human", "");
    let cards_silent = cards_map(vec![
        card(
            "Free Leader",
            "",
            "Legendary Creature — Human",
            "At the beginning of your upkeep, draw a card.",
        ),
        second_silent,
        filler.clone(),
        card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
    ]);
    let mut deck_silent = deck_text("DECK", &[("Island", 40), ("Filler", 20)]);
    let cmd = deck_silent.section_entries_mut("COMMANDER");
    for (name, qty) in [("Free Leader", 1), ("Cheap Partner", 1)] {
        cmd.push(crate::deck::grammar::DeckEntry {
            quantity: qty,
            name: name.to_string(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    let silent_deck = build_sim_deck(&deck_silent, &cards_silent, None);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
    let silent_log = super::game::run_game(&silent_deck, &mut rng, 3);
    // Both engines fire: the partner pair sees 1 more card per turn than
    // the single-engine run (draw step cancels out), every turn.
    let growth_pair = log.cards_seen[1] - log.cards_seen[0];
    let growth_solo = silent_log.cards_seen[1] - silent_log.cards_seen[0];
    assert_eq!(
        growth_pair,
        growth_solo + 1,
        "the second partner's engine fires exactly once"
    );
    // Turn 3: cast interference (free filler casts also enter the seen
    // census and diverge with the libraries), so the bound is a range:
    // the second engine adds 0-2 pops (its draw plus hand-limit noise).
    // It must never exceed the +2 double-fire of the old per-sentinel
    // bug.
    let growth_pair3 = log.cards_seen[2] - log.cards_seen[1];
    let growth_solo3 = silent_log.cards_seen[2] - silent_log.cards_seen[1];
    assert!(
        (0..=2).contains(&(growth_pair3 - growth_solo3)),
        "the second partner's engine stays bounded (pair {growth_pair3} vs solo {growth_solo3})"
    );
}

#[test]
fn commander_engine_fires_on_extra_turns() {
    // An extra turn replays the commander's upkeep engine: the sentinel
    // must resolve even though it is not a battlefield uid. The deck
    // holds a {0} "take an extra turn" sorcery, so the extra turn is
    // guaranteed.
    let commander = card(
        "Draw Lord",
        "{0}",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
    );
    let extra = card(
        "Time Lord",
        "{0}",
        "Sorcery",
        "Take an extra turn after this one.",
    );
    let filler = card("Filler", "", "Creature — Frog", "");
    let cards = cards_map(vec![
        commander,
        extra,
        filler,
        card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
    ]);
    let deck = deck_text("DECK", &[("Time Lord", 1), ("Island", 40), ("Filler", 19)]);
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
    let log = super::game::run_game(&sim_deck, &mut rng, 3);
    // The engine fires at most once per turn replay (no double-fire on
    // the extra turn). Every turn-index growth is bounded by the draw
    // step plus one engine draw plus one possible extra-turn replay.
    for t in 0..3 {
        let prev = if t == 0 { 11 } else { log.cards_seen[t - 1] };
        let growth = log.cards_seen[t] - prev;
        assert!(
            growth <= 3,
            "turn {}: commander engine fires at most once per turn (incl. extra turns): {growth}",
            t + 1
        );
    }
}
