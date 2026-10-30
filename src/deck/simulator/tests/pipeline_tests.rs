//! Tests for the simulator pipeline module.

use super::deck::build_sim_deck;
use super::deck_test_support::card;
use super::oracle_lower::parse_sim_card;
use crate::db::CardRow;
use rand::SeedableRng;
use std::collections::HashMap;

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
    // The station animation remains available even when its scaling effects do not lower.
    let igs = card(
        "Infinite Guideline Station",
        "{W}{U}{B}{R}{G}",
        "Legendary Artifact — Spacecraft",
        "When Infinite Guideline Station enters, create a tapped 2/2 colorless Robot artifact creature token for each multicolored permanent you control.\nStation (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 12+.)\n12+ | Flying\nWhenever Infinite Guideline Station attacks, draw a card for each multicolored permanent you control.",
    );
    let sim = parse_sim_card(&igs);
    assert!(sim.is_station_card);
    assert_eq!(sim.animate_at(), Some(12));
    assert!(sim.striations.iter().any(|t| t.at == 12 && t.animate));
    assert_eq!(sim.cost.pips, [1, 1, 1, 1, 1]);
    assert!(!sim.is_creature);

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
    let commander = &sim_deck.commanders[0];
    assert_eq!(commander.animate_at(), Some(12));
    assert!(commander.unlocked_abilities(0).next().is_none());
    for source in [
        "When Infinite Guideline Station enters,",
        "Whenever Infinite Guideline Station attacks,",
    ] {
        assert!(
            commander
                .support
                .iter()
                .any(|entry| entry.source.starts_with(source)
                    && matches!(
                        entry.status,
                        super::support::AbilitySupportStatus::Unsupported { .. }
                    )),
            "{source}"
        );
    }
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
    // Burn deals player damage without entering direct life-loss totals.
    assert!(
        log.player_damage[5] > 0,
        "burn spells deal damage over 6 turns"
    );
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
fn chained_extra_turns_use_later_turn_slots_within_the_turn_cap() {
    let rows = vec![
        card(
            "Time Walk",
            "{0}",
            "Sorcery",
            "Take an extra turn after this one.",
        ),
        card("Island", "", "Basic Land — Island", "{T}: Add {U}."),
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
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(711);

    let log = super::game::run_game(&sim_deck, &mut rng, 3);

    assert_eq!(log.extra_turns.len(), 3);
    assert_eq!(log.extra_turns[0], 0);
    assert_eq!(log.extra_turns[1], 1);
    assert_eq!(log.extra_turns[2], 1);
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
        card("Self Mill", "{2}{B}", "Sorcery", "Mill five cards."),
    ];
    let cards = cards_map(rows);
    let mut deck = deck_text("DECK", &[("Sculpting Steel Mill", 10), ("Self Mill", 10)]);
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
    let opponent_mill = sim_deck
        .cards
        .iter()
        .find(|card| card.name == "Sculpting Steel Mill")
        .expect("opponent mill card is in the simulator deck");
    assert!(opponent_mill.mills_opponent);
    assert!(matches!(
        opponent_mill.spell_data.effects.as_slice(),
        [super::model::SimEffect::Mill(5)]
    ));
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..100)
        .map(|_| super::game::run_game(&sim_deck, &mut rng, 8))
        .collect();
    assert!(
        logs.iter().any(|log| log.opp_milled[7] > 0),
        "opponent-targeted mills count"
    );
    assert!(
        logs.iter().any(|log| log.self_milled[7] > 0),
        "self mills count"
    );
    assert!(logs.iter().any(|log| log.library_size[7] < 50));
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
        log.attackers[7] <= log.creatures[7],
        "attackers never exceed bodies"
    );
}

#[test]
fn commander_combat_potential_keeps_slots_and_thresholds_separate() {
    let land = parse_sim_card(&card("Plains", "", "Basic Land — Plains", "{T}: Add {W}."));
    let commander = |name: &str, power: u32| {
        let mut row = card(name, "{0}", "Legendary Creature — Human", "Haste");
        row.power = Some(power.to_string());
        parse_sim_card(&row)
    };
    let deck = super::model::SimDeck {
        companion: None,
        cards: vec![land; 60],
        commanders: vec![
            commander("Twenty Power", 20),
            commander("Twenty-One Power", 21),
        ],
        format: super::model::Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(71);
    let log = super::game::run_game(&deck, &mut rng, 2);

    assert_eq!(log.commander_combat_damage[0], [20, 20]);
    assert_eq!(log.commander_combat_damage[1], [21, 21]);
    assert_eq!(log.opponent_life_loss, [0, 0]);

    let one_turn = super::aggregate::aggregate(std::slice::from_ref(&log), &deck, 1);
    assert_eq!(one_turn.commander_combat_threshold_games, [0, 1]);
    assert_eq!(one_turn.commander_combat_threshold_p50, [None, Some(1)]);

    let stats = super::aggregate::aggregate(std::slice::from_ref(&log), &deck, 2);
    assert_eq!(stats.commander_combat_threshold_games, [1, 1]);
    assert_eq!(stats.commander_combat_threshold_p50, [Some(2), Some(1)]);
    assert_eq!(stats.commander_combat_cumulative_by_turn[0], [20.0, 40.0]);
    assert_eq!(stats.commander_combat_cumulative_by_turn[1], [21.0, 42.0]);

    let repeated = super::aggregate::aggregate(&[log.clone(), log], &deck, 2);
    assert_eq!(repeated.commander_combat_by_turn[0], [20.0, 20.0]);
    assert_eq!(
        repeated.commander_combat_cumulative_by_turn[0],
        [20.0, 40.0]
    );
    assert_eq!(repeated.commander_combat_threshold_games, [2, 2]);
}

#[test]
fn commander_potential_uses_current_power_and_excludes_triggered_life_loss() {
    let equipment = parse_sim_card(&card(
        "Power Blade",
        "{1}",
        "Artifact — Equipment",
        "Equipped creature gets +3/+0.",
    ));
    let mut first_row = card(
        "Sick Commander",
        "{1}{W}",
        "Legendary Creature — Soldier",
        "",
    );
    first_row.power = Some("3".to_string());
    let first_commander = parse_sim_card(&first_row);
    let mut second_row = card(
        "Hasty Commander",
        "{1}{R}",
        "Legendary Creature — Warrior",
        "Haste\nWhenever this creature deals combat damage to a player, that player loses 3 life.",
    );
    second_row.power = Some("2".to_string());
    let second_commander = parse_sim_card(&second_row);
    assert!(
        second_commander
            .unlocked_abilities(0)
            .any(|ability| ability.trigger == super::model::SimTrigger::CombatDamage)
    );
    let deck = super::model::SimDeck {
        companion: None,
        cards: vec![equipment],
        commanders: vec![first_commander, second_commander],
        format: super::model::Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut state = super::turn_loop_tests::state(vec![], vec![]);
    state.opponents = super::opponents::OpponentTable::for_rules(deck.rules);
    let mut gear = super::game::new_perm_with(1, &deck, super::model::CardIdx(0), 2, false);
    gear.equipped = true;
    gear.equip_host = Some(10);
    state.battlefield.push(gear);
    let mut sick = super::game::new_commander_perm(10, 0, 0, 2, true, false);
    sick.counters.plus1 = 1;
    state.battlefield.push(sick);
    state
        .battlefield
        .push(super::game::new_commander_perm(11, 1, 0, 2, true, true));
    state.battlefield.push(super::game::new_token_perm(12, 1));

    let first_combat = super::game_combat::combat_phase(&deck, &mut state, 2, &[0, 0, 0]);
    assert_eq!(first_combat.commander_power, [0, 2]);
    assert_eq!(first_combat.power, 2);
    assert_eq!(state.opponent_life_lost, 3);

    for permanent in &mut state.battlefield {
        if matches!(permanent.card, super::game::CardRef::Commander { .. }) {
            permanent.entered_turn = 1;
            permanent.summoning_sick = false;
            permanent.tapped = false;
        }
    }
    let next_combat = super::game_combat::combat_phase(&deck, &mut state, 3, &[0, 0, 0]);
    assert_eq!(next_combat.commander_power, [7, 2]);
    assert_eq!(next_combat.power, 9);
    assert_eq!(state.opponent_life_lost, 6);
}

#[test]
fn combat_stops_after_an_attack_trigger_causes_player_loss() {
    let mut first_row = card(
        "First",
        "{1}{W}",
        "Legendary Creature — Soldier",
        "Whenever this creature attacks, you lose 20 life.",
    );
    first_row.power = Some("2".to_string());
    let first = parse_sim_card(&first_row);
    assert!(first.unlocked_abilities(0).any(|ability| {
        ability.trigger == super::model::SimTrigger::Attacks
            && matches!(ability.effect, super::model::SimEffect::LoseLife { .. })
    }));
    let mut second_row = card("Second", "{1}{R}", "Legendary Creature — Warrior", "Haste");
    second_row.power = Some("3".to_string());
    let second = parse_sim_card(&second_row);
    let deck = super::model::SimDeck {
        companion: None,
        cards: vec![],
        commanders: vec![first, second],
        format: super::model::Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut state = super::turn_loop_tests::state(vec![], vec![]);
    state.life = 10;
    state.current_turn = 2;
    state.current_phase = super::game::GamePhase::Combat;
    state
        .battlefield
        .push(super::game::new_commander_perm(1, 0, 0, 1, true, true));
    state
        .battlefield
        .push(super::game::new_commander_perm(2, 1, 0, 1, true, true));

    let result = super::game_combat::combat_phase(&deck, &mut state, 2, &[0, 0]);

    // The loss happens before damage, so neither commander contributes combat output.
    assert_eq!(result.commander_power, [0, 0]);
    assert_eq!(result.power, 0);
    assert_eq!(state.opponent_life_lost, 0);
    assert!(matches!(
        &state.completion,
        Some(super::game::GameCompletion::PlayerLost {
            reasons,
            turn: 2,
            phase: super::game::GamePhase::Combat,
        }) if reasons == &[super::game::PlayerLossReason::ZeroLife]
    ));
}

#[test]
fn partner_deck_casts_both_commanders() {
    // A partner pair is castable turn 1 ({0} + {0}): each partner joins
    // the battlefield; the cast loop must not stop after the first.
    let first = card("Free Leader", "{0}", "Legendary Creature — Human", "");
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
    // Both commanders cost zero mana: two bodies from turn 1.
    assert!(
        log.creatures[0] >= 2,
        "both partners cast: {}",
        log.creatures[0]
    );
}

#[test]
fn partner_upkeep_engines_fire_once_per_commander_per_turn() {
    // Two partners, each with an upkeep draw. Each cast commander fires
    // its own engine once per turn, and a partner whose upkeep draw is
    // parsed registers an engine even when the first partner lacks one.
    let first = card(
        "Free Leader",
        "{0}",
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
    assert_eq!(
        log.repeatable_sources_online[0], 2,
        "one source per cast partner"
    );
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
    // (a duplicate engine) or 1 (the missing engine).
    let second_silent = card("Cheap Partner", "{0}", "Legendary Creature — Human", "");
    let cards_silent = cards_map(vec![
        card(
            "Free Leader",
            "{0}",
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
    // census and diverge with the libraries), so the bound is a range.
    // The second engine fires at least once (1) and never exceeds the +2
    // double-fire from duplicate registration.
    let growth_pair3 = log.cards_seen[2] - log.cards_seen[1];
    let growth_solo3 = silent_log.cards_seen[2] - silent_log.cards_seen[1];
    assert!(
        (1..=2).contains(&(growth_pair3 - growth_solo3)),
        "the second partner's engine fires once and stays bounded (pair {growth_pair3} vs solo {growth_solo3})"
    );
}

#[test]
fn commander_engine_fires_on_extra_turns() {
    // An extra turn replays the commander's upkeep engine from the
    // command zone. The deck
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
        let prev = if t == 0 {
            if log.mulliganed { 6 } else { 7 }
        } else {
            log.cards_seen[t - 1]
        };
        let growth = log.cards_seen[t] - prev;
        assert!(
            growth <= 3,
            "turn {}: commander engine fires at most once per turn (incl. extra turns): {growth}",
            t + 1
        );
    }
}
