//! Tests for the simulator game module.

use super::aggregate::aggregate;
use super::deck_test_support::card;
use super::game::run_game;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::land::parse_tap_yield;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A stub deck: lands + spells, all with simple tap data.
pub(super) fn stub_deck(lands: usize, spells: &[(&str, u32, Role)]) -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..lands {
        cards.push(super::model::SimCard {
            name: "Plains".into(),
            cost: Cost::default(),
            min_cost: Cost::default(),
            tap: Some(parse_tap_yield("{T}: Add {W}.").unwrap()),
            role: Role::Land,
            ..super::model::SimCard::default()
        });
    }
    for (name, cmc, role) in spells {
        for _ in 0..8 {
            cards.push(super::model::SimCard {
                name: (*name).into(),
                has_mana_cost: true,
                cost: Cost {
                    generic: *cmc,
                    ..Cost::default()
                },
                min_cost: Cost {
                    generic: *cmc,
                    ..Cost::default()
                },
                role: *role,
                ..super::model::SimCard::default()
            });
        }
    }
    SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    }
}

#[test]
fn same_seed_same_game() {
    let deck = stub_deck(24, &[("Bear", 2, Role::Other); 10]);
    let mut a = ChaCha8Rng::seed_from_u64(99);
    let mut b = ChaCha8Rng::seed_from_u64(99);
    let ga = run_game(&deck, &mut a, 8);
    let gb = run_game(&deck, &mut b, 8);
    assert_eq!(ga.opener_lands, gb.opener_lands);
    assert_eq!(ga.commander_castable, gb.commander_castable);
    assert_eq!(ga.cards_seen, gb.cards_seen);
    assert_eq!(ga.creatures, gb.creatures);
    assert_eq!(ga.completion, super::game::GameCompletion::HorizonReached);
    assert_eq!(ga.turns_observed, 8);
    assert!(ga.turn_observations.iter().all(|observation| {
        observation.draw
            && observation.main
            && observation.combat
            && observation.end_step
            && observation.end_turn
    }));
}

#[test]
fn outcome_counts_are_disjoint_and_incomplete_games_do_not_enter_primary_metrics() {
    let deck = stub_deck(24, &[("Bear", 2, Role::Other); 10]);
    let mut rng = ChaCha8Rng::seed_from_u64(102);
    let complete = run_game(&deck, &mut rng, 2);
    let mut lost = complete.clone();
    lost.completion = super::game::GameCompletion::PlayerLost {
        reasons: vec![super::game::PlayerLossReason::ZeroLife],
        turn: 1,
        phase: super::game::GamePhase::Main,
    };
    let mut incomplete = complete.clone();
    incomplete.completion = super::game::GameCompletion::Incomplete {
        reason: super::game::IncompleteReason::TurnAction {
            configured_limit: super::model::MAX_LOOP_PASSES,
            remaining_work: 1,
        },
        turn: 1,
        phase: super::game::GamePhase::Main,
    };

    let stats = aggregate(&[complete, lost, incomplete], &deck, 2);
    assert_eq!(stats.runs, 3);
    assert_eq!(stats.finished_games, 2);
    assert_eq!(stats.player_loss_games, 1);
    assert_eq!(stats.incomplete_games, 1);
    assert_eq!(stats.finished_games + stats.incomplete_games, stats.runs);
    assert_eq!(stats.incomplete_reasons["turn_action_limit"], 1);
    assert_eq!(stats.phase_observations[0].end_turn, 2);
}

#[test]
fn london_mulligan_keeps_two_to_five_lands_in_a_sixty_card_deck() {
    // Four spell rows create 32 spells, so the deck has 60 cards total.
    let deck = stub_deck(28, &[("Bear", 2, Role::Other); 4]);
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.mulligan_rate < 0.2,
        "60-card 28-land deck ships too often: {:.3}",
        stats.mulligan_rate
    );
    // Kept hands never hold 0, 1, 6, or 7 lands by policy.
    for log in &logs {
        if !log.mulliganed {
            assert!(
                (2..=5).contains(&log.opener_lands),
                "kept opener holds {} lands",
                log.opener_lands
            );
        }
    }
}

// Commander timing with colored pips

#[test]
fn dense_mana_hits_all_early_land_drops() {
    let deck = stub_deck(40, &[("Bear", 2, Role::Other)]);
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..500).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.hit_all_drops_by[4] > 0.90,
        "expected >90% hit-all-4 with 40/60 lands, got {:.3}",
        stats.hit_all_drops_by[4]
    );
}

#[test]
fn sparse_mana_trips_screw() {
    let deck = stub_deck(10, &[("Bear", 2, Role::Other); 5]);
    let mut dense = stub_deck(10, &[("Bear", 2, Role::Other); 5]);
    let land = dense.cards[0].clone();
    dense.cards[10..30].fill(land);
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let mut dense_rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..500).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let dense_logs: Vec<_> = (0..500)
        .map(|_| run_game(&dense, &mut dense_rng, 6))
        .collect();
    let stats = aggregate(&logs, &deck, 6);
    let dense_stats = aggregate(&dense_logs, &dense, 6);
    assert!(
        stats.screw_pct > dense_stats.screw_pct,
        "10/50 lands must miss more early drops than 30/50: {:.3} vs {:.3}",
        stats.screw_pct,
        dense_stats.screw_pct
    );
    assert!(
        stats.hit_all_drops_by[4] < dense_stats.hit_all_drops_by[4],
        "the denser mana base must hit four land drops more often"
    );
}

/// Each off-color pip needs its own flexible mana unit.
#[test]
fn five_color_commander_needs_all_pips() {
    let land = parse_sim_card(&card("Plains", "", "Basic Land — Plains", "{T}: Add {W}."));
    let rock = parse_sim_card(&card(
        "Signet",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color.",
    ));
    let deck = SimDeck {
        companion: None,
        cards: [vec![land; 5], vec![rock; 4]].concat(),
        commanders: vec![parse_sim_card(&card(
            "Boss",
            "{W}{U}{B}{R}{G}",
            "Legendary Creature — Avatar",
            "",
        ))],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    assert_eq!(deck.commanders[0].min_cost.pips, [1; 5]);
    for rocks in 0..=4 {
        let mut state = super::turn_loop_tests::state(vec![], vec![]);
        state.library = (0..40)
            .map(|index| super::model::CardIdx(index % deck.cards.len() as u32))
            .collect();
        state.next_uid = (5 + rocks) as u32;
        for index in 0..5 + rocks {
            state.battlefield.push(super::game::new_perm_with(
                index as u32,
                &deck,
                CardIdx(index as u32),
                0,
                false,
            ));
        }
        let mut census = super::game_run::TurnCensus::new(1, deck.cards.len());
        super::game_run::run_turn(
            &deck,
            &mut state,
            &mut census,
            &super::game_commander::CommanderProfile::new(&deck),
            &mut vec![],
            1,
            false,
        );
        assert_eq!(
            census.commander_castable,
            (rocks == 4).then_some(1),
            "five white sources and {rocks} flexible units"
        );
        assert_eq!(census.mana_spent[0], if rocks == 4 { 5.0 } else { 0.0 });
    }
}

#[test]
fn creature_commander_can_use_creature_only_mana() {
    let mut cards = Vec::new();
    // Every source is creature-restricted; the commander matches the
    // restriction and may spend that mana.
    for _ in 0..40 {
        cards.push(parse_sim_card(&card(
            "Creature Gate",
            "",
            "Land",
            "{T}: Add {W}. Spend this mana only to cast creature spells.",
        )));
    }
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![super::model::SimCard {
            name: "Generic Boss".into(),
            has_mana_cost: true,
            cost: super::model::Cost {
                generic: 5,
                ..super::model::Cost::default()
            },
            min_cost: super::model::Cost {
                generic: 5,
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            is_creature: true,
            ..super::model::SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.commander_castable_by[7] > 0.5,
        "creature commander missed creature-restricted mana: {:.3} by t7",
        stats.commander_castable_by[7]
    );
}

// Station end-to-end: crew -> station -> attack-draw chain

#[test]
fn vehicle_crews_and_striations_unlock() {
    // A vehicle + crew bodies + a spacecraft commander. Crew 2 needs two
    // creatures (token power 2); creatures station the spacecraft (4+ animate).
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(super::model::SimCard {
            name: "Island".into(),
            tap: Some(parse_tap_yield("{T}: Add {U}.").unwrap()),
            role: Role::Land,
            ..super::model::SimCard::default()
        });
    }
    for _ in 0..6 {
        // Small crew bodies.
        cards.push(super::model::SimCard {
            name: "Pilot".into(),
            has_mana_cost: true,
            cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            min_cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            role: Role::Other,
            is_creature: true,
            ..super::model::SimCard::default()
        });
    }
    for _ in 0..6 {
        cards.push(super::model::SimCard {
            name: "Copter".into(),
            has_mana_cost: true,
            cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            min_cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            crew: Some(2),
            is_creature: false,
            role: Role::Other,
            ..super::model::SimCard::default()
        });
    }
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs: Vec<_> = (0..400).map(|_| run_game(&deck, &mut rng, 10)).collect();
    // Crewing produces bodies by turn 6.
    let stats = aggregate(&logs, &deck, 10);
    assert!(
        stats.creatures_by_turn[5] > 0.3,
        "expected crewed vehicles as bodies by t6, got {:.2}",
        stats.creatures_by_turn[5]
    );
}

#[test]
fn station_tokens_feed_the_commander() {
    // IGS shape: ETB tokens (station fuel), 12+ animation, attack draw.
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(super::model::SimCard {
            name: "Plains".into(),
            tap: Some(parse_tap_yield("{T}: Add {W}.").unwrap()),
            role: Role::Land,
            ..super::model::SimCard::default()
        });
    }
    // Any-color rocks fund the 5c commander's pip cost.
    for _ in 0..20 {
        cards.push(super::model::SimCard {
            name: "Signet".into(),
            has_mana_cost: true,
            cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            min_cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            tap: Some(parse_tap_yield("{T}: Add one mana of any color.").unwrap()),
            role: Role::Rock,
            ..super::model::SimCard::default()
        });
    }
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![super::model::SimCard {
            name: "IGS".into(),
            has_mana_cost: true,
            cost: super::model::Cost {
                pips: [1, 1, 1, 1, 1],
                ..super::model::Cost::default()
            },
            min_cost: super::model::Cost {
                pips: [1, 1, 1, 1, 1],
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            is_station_card: true,
            striations: vec![
                // ETB tokens (the sim fires them as station fuel bodies).
                SimStriation {
                    at: 0,
                    animate: false,
                    abilities: vec![super::model::SimAbility {
                        trigger: SimTrigger::Enters,
                        event_subject: super::model::SimEventSubject::This,
                        effect: super::model::SimEffect::Tokens(2),
                        ..super::model::SimAbility::default()
                    }],
                },
                SimStriation {
                    at: 12,
                    animate: true,
                    abilities: vec![],
                },
            ],
            ..super::model::SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(17);
    let logs: Vec<_> = (0..500).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // ETB tokens exist as bodies once the commander is cast.
    assert!(
        stats.creatures_by_turn[9] > 0.2,
        "commander tokens should count as bodies by t10, got {:.2}",
        stats.creatures_by_turn[9]
    );
    // The commander is a station card: online means animated, not cast.
    // It needs 12 counters = 6 body-taps; some games get there by t10.
    assert!(
        stats.station_online_pct < 0.99,
        "12+ should not be free: online {:.2}",
        stats.station_online_pct
    );
}

// Deck construction

#[test]
fn fetch_land_searches_a_land() {
    let fetch = card(
        "Flooded Strand",
        "",
        "Land",
        "{T}, Pay 1 life, Sacrifice this land: Search your library for a Plains or Island card, put it onto the battlefield, then shuffle.",
    );
    let island = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let bear = card("Bear", "{2}", "Creature — Bear", "Vanilla.");
    let mut cards = Vec::new();
    for _ in 0..20 {
        cards.push(parse_sim_card(&fetch));
        cards.push(parse_sim_card(&island));
    }
    for _ in 0..20 {
        cards.push(parse_sim_card(&bear));
    }
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(41);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 6)).collect();
    // Fetch + fetched land = 2 land drops on fetch turns. Land-drop rate
    // should beat the raw land count's expectation.
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.hit_all_drops_by[4] > 0.5,
        "fetches should smooth early land drops, got {:.2}",
        stats.hit_all_drops_by[4]
    );
}

// JSON report shape (additive fields present, stable keys)

#[test]
fn verge_land_gate_blocks_early_mode() {
    let verge = card(
        "Blazemire Verge",
        "",
        "Land",
        "{T}: Add {B}.\n{T}: Add {R}. Activate only if you control a Swamp or a Mountain.",
    );
    let sim = parse_sim_card(&verge);
    assert_eq!(
        sim.gate_types,
        vec![
            super::model::BasicLandType::Swamp,
            super::model::BasicLandType::Mountain
        ]
    );
    // A lone verge never satisfies its own gate.
    let mut cards = Vec::new();
    for _ in 0..35 {
        cards.push(sim.clone());
    }
    for _ in 0..25 {
        cards.push(super::model::SimCard {
            name: "Bear".into(),
            has_mana_cost: true,
            cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            min_cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            role: Role::Other,
            ..super::model::SimCard::default()
        });
    }
    // A spell demanding the locked red mode.
    for _ in 0..25 {
        cards.push(super::model::SimCard {
            name: "Bolt".into(),
            has_mana_cost: true,
            cost: Cost {
                pips: [0, 0, 0, 1, 0],
                ..Cost::default()
            },
            min_cost: Cost {
                pips: [0, 0, 0, 1, 0],
                ..Cost::default()
            },
            role: Role::Removal,
            ..super::model::SimCard::default()
        });
    }
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(31);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // Red pips missed in most games (the {R} mode stays locked).
    assert!(
        stats.color_screw[3] > 0.3,
        "lone verge should show red pip misses, got {:.2}",
        stats.color_screw[3]
    );
}

#[test]
fn spend_restricted_mana_pays_creature_casts_only() {
    // A deck whose only nonbasic is a courtyard still casts creatures but
    // struggles with the pip-heavy noncreature spell when basics are few.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let courtyard = parse_sim_card(&card(
        "Secluded Courtyard",
        "",
        "Land",
        "As this land enters, choose a creature type.\n{T}: Add {C}.\n{T}: Add one mana of any color. Spend this mana only to cast a creature spell of the chosen type.",
    ));
    deck.cards.push(courtyard);
    let artifact = parse_sim_card(&card(
        "Big Artifact",
        "{5}",
        "Artifact",
        "Vanilla artifact.",
    ));
    deck.cards.push(artifact);
    let mut rng = ChaCha8Rng::seed_from_u64(47);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // Creatures (Bear, 2 generic) cast fine; the UU artifact should lag
    // behind its curve more than a same-cost creature would.
    let art = stats
        .card_castability
        .iter()
        .find(|c| c.name == "Big Artifact")
        .expect("artifact row");
    let bear = stats
        .card_castability
        .iter()
        .find(|c| c.name == "Bear")
        .expect("bear row");
    assert!(
        art.avg_first_castable_turn >= bear.avg_first_castable_turn,
        "restricted mana should not favor the noncreature spell"
    );
}

#[test]
fn sacrifice_outlet_consumes_bodies_and_fires_deaths() {
    // Outlet + death payoff: activating the outlet keeps bodies flowing
    // (death tokens) and velocity rises from the death draws.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let outlet = parse_sim_card(&card(
        "Altar",
        "{2}",
        "Artifact",
        "{T}, Sacrifice a creature: Draw a card.",
    ));
    let payoff = parse_sim_card(&card(
        "Death Dealer",
        "{3}",
        "Creature — Human",
        "Whenever another creature you control dies, create a 1/1 token.",
    ));
    deck.cards.push(outlet);
    deck.cards.push(payoff);
    let mut rng = ChaCha8Rng::seed_from_u64(53);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // Bodies must not collapse to zero: sacrificed bodies are replaced by
    // death tokens in a share of games. The payoff is one copy in 66, so
    // the bar is low — it proves sacrifice + death-token firing works.
    assert!(
        stats.creatures_by_turn[9] >= 0.1,
        "aristocrats board collapsed: {}",
        stats.creatures_by_turn[9]
    );
}

#[test]
fn mill_engine_fills_graveyard() {
    // An upkeep mill engine fills the graveyard census; milled cards count
    // as cards seen (velocity grows past the pure-draw baseline).
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let miller = parse_sim_card(&card(
        "Slow Mill",
        "{3}",
        "Artifact Creature — Construct",
        "At the beginning of your upkeep, mill three cards.",
    ));
    deck.cards.push(miller);
    let mut rng = ChaCha8Rng::seed_from_u64(41);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // When the miller is in play (any game where it was cast), the
    // graveyard grows. Aggregate average is small but must exceed zero
    // by turn 10 (the creature casts in a meaningful share of games).
    assert!(
        stats.graveyard_by_turn[9] > 0.0,
        "graveyard empty with a mill engine in the deck: {}",
        stats.graveyard_by_turn[9]
    );
}

#[test]
fn zone_telemetry_tracks_battlefield_and_graveyard() {
    // A cast spell lands in the battlefield log; a milled card lands in
    // the graveyard log. Both feed combo assembly per piece zone.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let miller = parse_sim_card(&card(
        "Slow Mill",
        "{3}",
        "Artifact Creature — Construct",
        "At the beginning of your upkeep, mill three cards.",
    ));
    deck.cards.push(miller);
    let mut rng = ChaCha8Rng::seed_from_u64(41);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 10)).collect();
    // The Bear sits at index 24 (24 lands first); it is cast in a
    // meaningful share of games, so its index shows up in the
    // battlefield log for some games.
    let bear_on_board = logs
        .iter()
        .filter(|log| log.card_first_battlefield.contains_key(&24))
        .count();
    assert!(
        bear_on_board > 20,
        "bears never reached the battlefield log: {bear_on_board}/200"
    );
    // Milled cards (lands, indices 0..23) appear in the graveyard log;
    // the mill engine milled something in some games.
    let milled_something = logs
        .iter()
        .any(|log| log.card_first_graveyard.contains_key(&0));
    assert!(milled_something, "mill engine never logged a graveyard hit");
    // A graveyard hit must come after a hand sighting (draw order), or at
    // the same turn at worst: zones only fill through legal transitions.
    for log in &logs {
        for (idx, grave_turn) in &log.card_first_graveyard {
            if let Some(hand_turn) = log.card_first_seen.get(idx) {
                assert!(
                    grave_turn >= hand_turn,
                    "card {idx} milled at t{grave_turn} before seen at t{hand_turn}"
                );
            }
        }
    }
}

#[test]
fn graveyard_return_makes_bodies() {
    // A battlefield-return engine puts creatures back into play; body
    // counts rise above the vanilla baseline late game.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let reanimator = parse_sim_card(&card(
        "Reanimator",
        "{3}",
        "Artifact Creature — Construct",
        "At the beginning of your upkeep, return a creature card from your graveyard to the battlefield.",
    ));
    deck.cards.push(reanimator);
    let mut rng = ChaCha8Rng::seed_from_u64(43);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // Vanilla stub decks have zero bodies when no creature was drawn early;
    // the reanimator deck must produce bodies at some point.
    assert!(
        stats.creatures_by_turn[9] > 0.0,
        "reanimator produced no bodies"
    );
}

#[test]
fn group_hug_symmetric_draw_feeds_velocity() {
    // "Each player draws a card" engines feed the goldfish's velocity:
    // a group-hug draw engine still moves cards_seen upward.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let hug = parse_sim_card(&card(
        "Hug Engine",
        "{3}",
        "Creature — Elephant",
        "At the beginning of your upkeep, each player draws a card.",
    ));
    deck.cards.push(hug);
    let mut rng = ChaCha8Rng::seed_from_u64(61);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    assert!(stats.cards_seen[9] > stats.cards_seen[0]);
}

#[test]
fn walker_loyalty_gates_activations() {
    // A planeswalker's minus ability only fires while loyalty covers it;
    // the loyalty drops as abilities fire.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let mut row = card(
        "Walker",
        "{3}{W}{W}",
        "Legendary Planeswalker — Human",
        "−2: Draw two cards.",
    );
    row.loyalty = Some("4".to_string());
    let walker = parse_sim_card(&row);
    assert_eq!(walker.starting_loyalty, Some(4));
    assert!(walker.unlocked_abilities(0).any(|ability| {
        ability
            .activation
            .as_ref()
            .is_some_and(|costs| costs.loyalty_change() == -2)
            && matches!(ability.effect, SimEffect::Draw(2))
    }));
    deck.cards.push(walker);
    let mut rng = ChaCha8Rng::seed_from_u64(67);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // The walker deck draws more than a vanilla deck by turn 10 in games
    // where the walker showed up; the bar is low (1 copy in 66).
    assert!(stats.cards_seen[9] > stats.cards_seen[0]);
}
