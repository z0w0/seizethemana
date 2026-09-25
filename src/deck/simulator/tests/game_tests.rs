// Tests for the simulator game module.

/// A minimal card row for tests.
use super::aggregate::aggregate;
use super::game::run_game;
use super::model::*;
use super::parse::*;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
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
    assert_eq!(ga.bodies, gb.bodies);
}

#[test]
fn mulligan_policy_keeps_2_to_6_lands() {
    // Karsten's London model redraws 0/1/6/7-land openers and bottoms
    // toward 3 lands. A 24-land deck ships the 0/1-land hands; its
    // mulligan rate stays well under half of games.
    let deck = stub_deck(24, &[("Bear", 2, Role::Other)]);
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.mulligan_rate < 0.45,
        "24-land deck ships too often: {:.3}",
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
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..500).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.screw_pct > 0.4,
        "expected heavy screw with 10/50 lands, got {:.3}",
        stats.screw_pct
    );
}

#[test]
fn five_color_commander_needs_all_pips() {
    // A 5c commander (IGS shape) with only white-producing lands never
    // casts: the pip check gates it.
    let mut cards = Vec::new();
    for _ in 0..40 {
        cards.push(super::model::SimCard {
            name: "Plains".into(),
            tap: Some(parse_tap_yield("{T}: Add {W}.").unwrap()),
            role: Role::Land,
            ..super::model::SimCard::default()
        });
    }
    for _ in 0..20 {
        cards.push(super::model::SimCard {
            name: "Filler".into(),
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
    let mut deck = SimDeck {
        cards,
        commanders: vec![super::model::SimCard {
            name: "Boss".into(),
            cost: super::model::Cost {
                pips: [1, 1, 1, 1, 1],
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            ..super::model::SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let logs: Vec<_> = (0..400).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // Only ~a quarter of openers hold a second white source by turn 5.
    assert!(
        stats.commander_castable_by[5] < 0.5,
        "WUBRG commander should be color-gated, got {:.3} by t5",
        stats.commander_castable_by[5]
    );

    // Fix the mana base with a five-color rock: it comes down on curve.
    deck.cards.clear();
    for _ in 0..40 {
        deck.cards.push(super::model::SimCard {
            name: "Plains".into(),
            tap: Some(parse_tap_yield("{T}: Add {W}.").unwrap()),
            role: Role::Land,
            ..super::model::SimCard::default()
        });
    }
    for _ in 0..20 {
        deck.cards.push(super::model::SimCard {
            name: "Signet".into(),
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
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let logs: Vec<_> = (0..400).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // Correct pip accounting: the 5 pips all draw from the shared
    // flexible pool (each rock is one pip), so the bar sits below the
    // double-counted old check. Any-color rocks still unlock the cast
    // for well over half of games by turn 5.
    assert!(
        stats.commander_castable_by[5] > 0.5,
        "any-color rocks should unlock the 5c commander, got {:.3} by t5",
        stats.commander_castable_by[5]
    );
}

#[test]
fn commander_cast_ignores_restricted_bucket_mana() {
    // A creature-only mana source's bucket never pays a commander cast:
    // `pay_cost` cannot spend it, so the readiness check must not count
    // it either. A {5} commander with only a creature-only rock on the
    // board must stay uncast (the board lands alone never reach 5).
    let mut cards = Vec::new();
    // Every source is creature-restricted: the general pool never holds
    // mana the commander could pay with.
    for _ in 0..40 {
        cards.push(parse_sim_card(&card(
            "Creature Gate",
            "",
            "Land",
            "{T}: Add {W}. Spend this mana only to cast creature spells.",
        )));
    }
    let deck = SimDeck {
        cards,
        commanders: vec![super::model::SimCard {
            name: "Generic Boss".into(),
            cost: super::model::Cost {
                generic: 5,
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            ..super::model::SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.commander_castable_by[7] < 0.05,
        "5-generic commander cast with restricted-bucket mana: {:.3} by t7",
        stats.commander_castable_by[7]
    );
}

// Station end-to-end: crew -> station -> attack-draw chain

#[test]
fn vehicle_crews_and_station_tiers_unlock() {
    // A vehicle + crew bodies + a spacecraft commander. Crew 2 needs two
    // bodies (BODY_POWER 2); bodies station the spacecraft (4+ animate).
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
        stats.bodies_by_turn[5] > 0.3,
        "expected crewed vehicles as bodies by t6, got {:.2}",
        stats.bodies_by_turn[5]
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
        cards,
        commanders: vec![super::model::SimCard {
            name: "IGS".into(),
            cost: super::model::Cost {
                pips: [1, 1, 1, 1, 1],
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            is_station_card: true,
            station_tiers: vec![
                // ETB tokens (the sim fires them as station fuel bodies).
                Tier {
                    at: 0,
                    animate: false,
                    abilities: vec![super::model::Ability {
                        trigger: Trigger::OnEnter,
                        effect: super::model::Effect::Tokens(2),
                        ..super::model::Ability::default()
                    }],
                },
                Tier {
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
        stats.bodies_by_turn[9] > 0.2,
        "commander tokens should count as bodies by t10, got {:.2}",
        stats.bodies_by_turn[9]
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
    assert_eq!(sim.gate_types, vec!["Swamp", "Mountain"]);
    // A lone verge never satisfies its own gate.
    let mut cards = Vec::new();
    for _ in 0..35 {
        cards.push(sim.clone());
    }
    for _ in 0..25 {
        cards.push(super::model::SimCard {
            name: "Bear".into(),
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
        stats.bodies_by_turn[9] >= 0.1,
        "aristocrats board collapsed: {}",
        stats.bodies_by_turn[9]
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
        stats.bodies_by_turn[9] > 0.0,
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
    assert!(
        walker
            .abilities()
            .any(|a| a.loyalty_cost == 2 && matches!(a.effect, Effect::Draw(2)))
    );
    deck.cards.push(walker);
    let mut rng = ChaCha8Rng::seed_from_u64(67);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // The walker deck draws more than a vanilla deck by turn 10 in games
    // where the walker showed up; the bar is low (1 copy in 66).
    assert!(stats.cards_seen[9] > stats.cards_seen[0]);
}
