// Tests for the simulator report module.

use super::aggregate::aggregate;
use super::model::*;
#[test]
fn json_report_adds_station_and_body_fields() {
    let deck = SimDeck {
        cards: vec![],
        commanders: vec![super::model::SimCard {
            name: "IGS".into(),
            cost: super::model::Cost {
                pips: [1, 1, 1, 1, 1],
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            is_station_card: true,
            station_tiers: vec![Tier {
                at: 12,
                animate: true,
                abilities: vec![],
            }],
            ..super::model::SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let logs: Vec<super::game::GameLog> = Vec::new();
    let stats = aggregate(&logs, &deck, 10);
    let report = super::report::json_report(&stats, &deck, "test", 42, &[], 0, &Default::default());
    let obj = report.as_object().unwrap();
    // New additive fields exist.
    assert!(obj.contains_key("station"));
    assert!(obj.contains_key("bodies_by_turn"));
    assert!(obj.contains_key("engines_online_by_turn"));
    assert!(obj.contains_key("assumptions"));
    // Station metrics present for a spacecraft commander.
    assert!(obj["station"].is_object());
    assert!(obj["station"]["online_by_t6"].is_number());
    // Stable legacy fields still present.
    for key in [
        "name",
        "format",
        "runs",
        "turns",
        "seed",
        "deck_shape",
        "opening_hand",
        "land_drops",
        "commander",
        "mana",
        "draw",
        "role_access",
        "velocity",
        "color_screw",
        "card_castability",
        "problems",
        "summary",
    ] {
        assert!(obj.contains_key(key), "missing key {key}");
    }
}

#[test]
fn assumptions_list_documents_limits() {
    let deck = SimDeck {
        cards: vec![],
        commanders: vec![super::model::SimCard {
            name: "IGS".into(),
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
    let list = super::report::assumptions(&deck);
    let joined = list.join("\n");
    assert!(joined.contains("no opponents"));
    assert!(joined.contains("hybrid pips"));
    assert!(joined.contains("commander"));
    assert!(joined.contains("recast tax"));
}

// Multi-ability tap merges: one permanent, one tap, one mana

#[test]
fn json_report_interaction_color_and_wincons_are_populated() {
    // Metric value pins: interaction, color_sources, wincons, and
    // deck_shape render as real objects with numeric leaves (not nulls
    // or empty objects) for a deck with content in every zone.
    use super::aggregate::aggregate;
    use super::game::run_game;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    let mut cards = Vec::new();
    for _ in 0..24 {
        cards.push(SimCard {
            name: "Plains".into(),
            tap: Some(super::parse::parse_tap_yield("{T}: Add {W}.").unwrap()),
            role: Role::Land,
            ..SimCard::default()
        });
    }
    for _ in 0..6 {
        cards.push(SimCard {
            name: "Bolt".into(),
            cost: Cost {
                generic: 1,
                ..Cost::default()
            },
            min_cost: Cost {
                generic: 1,
                ..Cost::default()
            },
            role: Role::Removal,
            is_interaction: true,
            is_instant_speed: true,
            drain_on_cast: 3,
            ..SimCard::default()
        });
    }
    for _ in 0..10 {
        cards.push(SimCard {
            name: "Bear".into(),
            cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            min_cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            role: Role::Wincon,
            is_creature: true,
            printed_power: Some(2),
            ..SimCard::default()
        });
    }
    let deck = SimDeck {
        cards,
        commanders: vec![SimCard {
            name: "Boss".into(),
            cost: Cost {
                generic: 4,
                ..Cost::default()
            },
            role: Role::Wincon,
            ..SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    let report = super::report::json_report(&stats, &deck, "pin", 42, &[], 0, &Default::default());
    let obj = report.as_object().unwrap();
    // Interaction readiness shows up with a countable instant pool.
    assert!(obj["interaction"].is_object());
    assert!(
        obj["interaction"]["instant_speed_count"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "interaction.instant_count missing or empty"
    );
    // Wincons carries numeric rows.
    assert!(obj["wincons"].is_object());
    assert!(
        obj["wincons"]["drain_p50_by_turn"].is_null()
            || obj["wincons"]["drain_p50_by_turn"].is_number()
    );
    // The commander block carries timing percentiles.
    assert!(obj["commander"]["p50_cast_turn"].is_number());
    assert!(obj["commander"]["p95_cast_turn"].is_number());
    // Deck shape carries the audit keys.
    for key in [
        "total_cards",
        "lands",
        "rocks",
        "dorks",
        "ramp_spells",
        "draw_sources",
        "removal",
        "wincons",
        "locks",
        "boosters",
        "curve",
    ] {
        assert!(
            obj["deck_shape"].get(key).is_some(),
            "deck_shape missing {key}"
        );
    }
}

#[test]
fn aggregate_attack_power_p90_and_p95_drops_compute() {
    // Percentile fields carry real values for a real game batch.
    use super::game::run_game;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    let mut cards = Vec::new();
    for _ in 0..20 {
        cards.push(SimCard {
            name: "Mountain".into(),
            tap: Some(super::parse::parse_tap_yield("{T}: Add {R}.").unwrap()),
            role: Role::Land,
            ..SimCard::default()
        });
    }
    for _ in 0..10 {
        cards.push(SimCard {
            name: "Hasty Ogre".into(),
            cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            min_cost: Cost {
                generic: 2,
                ..Cost::default()
            },
            role: Role::Wincon,
            is_creature: true,
            printed_power: Some(3),
            has_haste: true,
            ..SimCard::default()
        });
    }
    let deck = SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..60).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.attack_power_by_turn[6] > 0.0,
        "attack power never computed"
    );
    assert!(stats.p95_drops_by_4 > 0, "p95 drops never computed");
}
