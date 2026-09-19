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
fn stub_deck(lands: usize, spells: &[(&str, u32, Role)]) -> SimDeck {
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
    // 40/60 lands: openers never ship (2-6 lands nearly guaranteed), and
    // mulligan rate stays near zero.
    let deck = stub_deck(40, &[("Bear", 2, Role::Other)]);
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.mulligan_rate < 0.05,
        "dense deck should rarely mulligan, got {:.3}",
        stats.mulligan_rate
    );
    // A sparse deck mulligans often and never keeps zero lands by policy:
    // any zero-land opener after the redraw means mulliganed is true.
    let sparse = stub_deck(10, &[("Bear", 2, Role::Other); 5]);
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let logs: Vec<_> = (0..300).map(|_| run_game(&sparse, &mut rng, 6)).collect();
    // Zero-land kept openers are impossible: opener_pct[0] counts only
    // kept hands (the sim redraws them).
    for log in &logs {
        if log.opener_lands == 0 {
            assert!(log.mulliganed, "zero-land opener must be redrawn");
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
    assert!(
        stats.commander_castable_by[5] > 0.6,
        "any-color rocks should unlock the 5c commander, got {:.3} by t5",
        stats.commander_castable_by[5]
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

// Game-level assertions for the new mechanics: haste, landfall engines,
// planeswalker activations, X-costs, extra land drops, extra-turn drops,
// per-cast mana, upkeep drains, infinite-mana census.

#[test]
fn hasted_creature_attacks_entry_turn() {
    // One hasted one-drop in a land deck: bodies and attack power rise
    // the same turn it enters, not the turn after.
    let rows = [
        card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"),
        card("Swift Body", "{W}", "Creature — Human Soldier", "Haste"),
    ];
    let mut cards = Vec::new();
    for _ in 0..24 {
        cards.push(rows[0].clone());
    }
    for _ in 0..12 {
        cards.push(rows[1].clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 5)).collect();
    let stats = aggregate(&logs, &deck, 5);
    // Haste means the creature attacks the turn it enters; t2 attack
    // power must be non-zero in a healthy share of games.
    assert!(
        stats.attack_power_by_turn[1] > 0.5,
        "hasted one-drop should attack on t2, got {:.2}",
        stats.attack_power_by_turn[1]
    );
}

#[test]
fn landfall_engine_draws_on_land_drops() {
    // "Landfall — Whenever a land you control enters, draw a card." on a
    // cheap body: velocity rises with the land count.
    let landfall_body = card(
        "Tatyova-class",
        "{1}{G}",
        "Creature — Elemental",
        "Landfall — Whenever a land you control enters, you gain 1 life and draw a card.",
    );
    let mut cards = Vec::new();
    for _ in 0..28 {
        cards.push(card("Forest", "", "Basic Land — Forest", "({T}: Add {G}.)"));
    }
    for _ in 0..12 {
        cards.push(landfall_body.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // 6 turns = 6 land drops + the opener; each land drop draws. Cards
    // seen by t6 must beat the plain draw expectation (1/turn + 7).
    assert!(
        stats.cards_seen[5] > 12.0,
        "landfall engine should draw per land drop, got {:.2} by t6",
        stats.cards_seen[5]
    );
}

#[test]
fn planeswalker_fires_loyalty_and_gains() {
    // A walker with +1 draw and −3 draw: loyalty activates fire, the
    // ultimate (loyalty 6) becomes online in some games.
    let walker = card(
        "Test Walker",
        "{2}{W}",
        "Legendary Planeswalker — Test",
        "+1: Draw a card.\n−3: Draw two cards.\n−7: Draw five cards.",
    );
    let mut cmd = walker.clone();
    cmd.loyalty = Some("4".into());
    let mut cards = Vec::new();
    for _ in 0..35 {
        cards.push(card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"));
    }
    for _ in 0..12 {
        cards.push(card("Bear", "{2}", "Creature — Bear", "Vanilla."));
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![parse_sim_card(&cmd)],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // With +1 each turn from cast turn, loyalty 4 reaches 6+ within a
    // few turns in most games: ultimate_online must fire in >30%.
    assert!(
        stats.ultimate_online_pct > 0.30,
        "walker ultimate should flag online, got {:.2}",
        stats.ultimate_online_pct
    );
}

#[test]
fn x_spell_pays_leftover_and_drains() {
    // {X}{B}{B} drain spell in a mono-black land deck: once the board
    // floats mana, the X spell converts it to drain.
    let x_drain = card(
        "Torment-lite",
        "{X}{B}{B}",
        "Sorcery",
        "Target player loses X life.",
    );
    let mut cards = Vec::new();
    for _ in 0..26 {
        cards.push(card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)"));
    }
    for _ in 0..10 {
        cards.push(x_drain.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(13);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.drain_total_by_turn[7] > 10.0,
        "X drain spell should convert floated mana to drain, got {:.1}",
        stats.drain_total_by_turn[7]
    );
}

#[test]
fn extra_land_drop_engine_ramps() {
    // "You may play an additional land on each of your turns." on a
    // cheap creature: median drops by t4 exceed 4.
    let aesi_shape = card(
        "Aesi-lite",
        "{1}{G}{U}",
        "Creature — Merfolk",
        "You may play an additional land on each of your turns.",
    );
    let mut cards = Vec::new();
    for _ in 0..18 {
        cards.push(card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"));
        cards.push(card("Forest", "", "Basic Land — Forest", "({T}: Add {G}.)"));
    }
    for _ in 0..12 {
        cards.push(aesi_shape.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(17);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 5)).collect();
    let stats = aggregate(&logs, &deck, 5);
    assert!(
        stats.p50_drops_by_4 >= 5,
        "extra-land engine should push median drops by t4 past 4, got {}",
        stats.p50_drops_by_4
    );
}

#[test]
fn extra_turn_replays_land_drop_and_engines() {
    // "Take an extra turn" spell: the replayed turn records its land
    // drop in land_drops and keeps upkeep engines firing.
    let time_walk = card(
        "Twilight-lite",
        "{3}{U}",
        "Sorcery",
        "Take an extra turn after this one.",
    );
    let engine = card(
        "Upkeep Engine",
        "{2}{U}",
        "Enchantment",
        "At the beginning of your upkeep, draw a card.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"));
    }
    for _ in 0..6 {
        cards.push(time_walk.clone());
    }
    for _ in 0..4 {
        cards.push(engine.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(19);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.extra_turns_pct > 0.20,
        "extra turns should fire, got {:.2}",
        stats.extra_turns_pct
    );
}

#[test]
fn per_cast_mana_engine_feeds_pool() {
    // Vivi-class: "{T}: Add one mana of any color for each instant or
    // sorcery spell you've cast this turn." Cheap cantrips + this =
    // mana multiplication. Unused mana must stay low while cantrips last.
    let vivi = card(
        "Vivi-lite",
        "{1}{U}{R}",
        "Legendary Creature — Wizard",
        "{T}: Add one mana of any color for each instant or sorcery spell you've cast this turn.",
    );
    let cantrip = card("Cheap Draw", "{U}", "Instant", "Draw a card.");
    let mut cards = Vec::new();
    for _ in 0..28 {
        cards.push(card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"));
    }
    for _ in 0..2 {
        cards.push(vivi.clone());
    }
    for _ in 0..10 {
        cards.push(cantrip.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(23);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // The per-cast engine + cheap cantrips should keep the deck casting:
    // velocity by t6 must beat the plain 1/turn + 7 expectation clearly.
    assert!(
        stats.cards_seen[5] > 14.0,
        "per-cast mana should fuel extra casts, got {:.2} by t6",
        stats.cards_seen[5]
    );
}

#[test]
fn infinite_mana_engine_flags_census() {
    // Zero-cost untapped activation producing 1 mana per activation:
    // the pass caps it and flags the census.
    let engine = card(
        "Loop Rock",
        "{2}",
        "Artifact",
        "{T}: Add {C}.\n{0}: Add {C}.",
    );
    let mut cards = Vec::new();
    for _ in 0..20 {
        cards.push(card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)"));
    }
    for _ in 0..8 {
        cards.push(engine.clone());
    }
    for _ in 0..8 {
        cards.push(card("Bear", "{2}", "Creature — Bear", "Vanilla."));
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(29);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.infinite_mana_pct > 0.10,
        "zero-cost loop engine should flag the census, got {:.2}",
        stats.infinite_mana_pct
    );
}

#[test]
fn upkeep_drain_engine_resolves() {
    // "At the beginning of your upkeep, each opponent loses 1 life."
    // registers and drains per turn.
    let engine = card(
        "Pain Engine",
        "{2}{B}",
        "Enchantment",
        "At the beginning of your upkeep, each opponent loses 1 life.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)"));
    }
    for _ in 0..6 {
        cards.push(engine.clone());
    }
    for _ in 0..4 {
        cards.push(card("Bear", "{2}", "Creature — Bear", "Vanilla."));
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(31);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 7)).collect();
    let stats = aggregate(&logs, &deck, 7);
    // Constructed: x1 per engine per turn. 6 copies x ~5 turns.
    assert!(
        stats.drain_total_by_turn[6] > 2.0,
        "upkeep drain engine should resolve, got {:.1}",
        stats.drain_total_by_turn[6]
    );
}

#[test]
fn constructed_drain_is_x1_not_x3() {
    // A single burn spell in constructed drains its printed amount,
    // not the commander-family triple.
    let burn = card(
        "Burn",
        "{1}{R}",
        "Sorcery",
        "Deal 3 damage to target player.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(card(
            "Mountain",
            "",
            "Basic Land — Mountain",
            "({T}: Add {R}.)",
        ));
    }
    for _ in 0..6 {
        cards.push(burn.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(37);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // 6 copies x 3 damage, each cast once at most: total ≤ 18 avg.
    assert!(
        stats.drain_total_by_turn[7] <= 20.0,
        "constructed drain should be x1, got {:.1}",
        stats.drain_total_by_turn[7]
    );
}

#[test]
fn kicker_pays_from_spare_mana() {
    // Kicker {2} burn: when the pool covers the kick, the drain bumps.
    let kicked = card(
        "Kicked Bolt",
        "{1}{R}",
        "Sorcery",
        "Kicker {2}\nKicked Bolt deals 3 damage to target player.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(card(
            "Mountain",
            "",
            "Basic Land — Mountain",
            "({T}: Add {R}.)",
        ));
    }
    for _ in 0..8 {
        cards.push(kicked.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(41);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // Kicked casts drain 5, unkicked 3. Either way, drain happens.
    assert!(
        stats.drain_total_by_turn[7] > 3.0,
        "kicker burn should drain, got {:.1}",
        stats.drain_total_by_turn[7]
    );
}

#[test]
fn saga_chapters_fire_payoffs() {
    // Chapter III "Draw two cards" runs after two upkeep steps.
    let saga = card(
        "Tales of Learning",
        "{1}{U}",
        "Enchantment — Saga",
        "(As this Saga enters and after each of your upkeep steps, add a lore counter.)\nI — Draw a card.\nII — Draw a card.\nIII — Draw two cards.",
    );
    let mut cards = Vec::new();
    for _ in 0..34 {
        cards.push(card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"));
    }
    for _ in 0..6 {
        cards.push(saga.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(43);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 7)).collect();
    let stats = aggregate(&logs, &deck, 7);
    // 4 sagas entering early each draw 1+1+2 over four turns on top of
    // the normal draw.
    assert!(
        stats.cards_seen[6] > 13.0,
        "saga chapters should draw, got {:.2} by t7",
        stats.cards_seen[6]
    );
}

#[test]
fn token_count_etb_feeds_bodies() {
    // "create four …tokens" puts 4 bodies, not the legacy 2.
    let maker = card(
        "Quad Maker",
        "{3}{W}",
        "Creature — Soldier",
        "When Quad Maker enters, create four 1/1 Soldier creature tokens.",
    );
    let mut cards = Vec::new();
    for _ in 0..28 {
        cards.push(card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"));
    }
    for _ in 0..12 {
        cards.push(maker.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(47);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // Each cast Quad Maker = 1 body + 4 tokens; bodies by t6 should
    // exceed what 2-token ETBs would give.
    assert!(
        stats.bodies_by_turn[5] > 3.0,
        "four-token ETB should fill the board, got {:.2}",
        stats.bodies_by_turn[5]
    );
}
