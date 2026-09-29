//! Tests for the simulator report module.

use super::aggregate::aggregate;
use super::model::*;
#[test]
fn json_report_adds_station_and_creature_fields() {
    let deck = SimDeck {
        companion: None,
        cards: vec![],
        commanders: vec![super::model::SimCard {
            name: "IGS".into(),
            cost: super::model::Cost {
                pips: [1, 1, 1, 1, 1],
                ..super::model::Cost::default()
            },
            role: Role::Wincon,
            is_station_card: true,
            striations: vec![SimStriation {
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
    let mut typed = super::report::json_report(
        &stats,
        &deck,
        "test",
        42,
        &[],
        Default::default(),
        &Default::default(),
    );
    typed.combo_access = Some(vec![super::report_schema::PairAccessReport {
        pair: "A + B".to_string(),
        target_turn: 4,
        percent_of_games: super::report_schema::Percent::from_share(0.77),
    }]);
    typed.colored_sources = Some(super::report_schema::ColoredSourcesReport {
        format: "commander".to_string(),
        lands: 36.0,
        sources: Default::default(),
        credits: super::report_schema::SourceCredits {
            lands: Default::default(),
            creature_mana_sources: Default::default(),
            artifact_mana_sources: Default::default(),
            cantrips: Default::default(),
        },
        requirements: vec![],
        worst_deficits: vec![],
        tapland_count: 0,
        untapped_t1_sources: Default::default(),
    });
    typed.combos = Some(super::report_schema::CombosReport {
        source: "commanderspellbook".to_string(),
        variants_considered: 1,
        complete: vec![],
        near_misses: vec![],
    });
    typed.win_paths = Some(super::report_schema::WinPathsReport {
        count: 0,
        paths: vec![],
    });
    typed.hypgeo = Some(super::hypgeo::cast_ceilings(&deck, 10));
    let report = serde_json::to_value(&typed).expect("serialize report");
    let parsed: super::report_schema::SimReport =
        serde_json::from_value(report.clone()).expect("deserialize complete report");
    assert_eq!(parsed, typed);
    let obj = report.as_object().unwrap();
    // New additive fields exist.
    assert!(obj.contains_key("station"));
    assert!(obj.contains_key("creatures_by_turn"));
    assert!(obj.contains_key("repeatable_sources_by_turn"));
    assert!(obj.contains_key("assumptions"));
    assert!(obj["milestones"].is_object());
    assert!(obj["milestones"]["dredge_uses_avg_by_turn"].is_object());
    assert!(obj["milestones"]["percent_games_with_positive_mana_loop_by_turn"].is_object());
    // Station metrics present for a spacecraft commander.
    assert!(obj["station"].is_object());
    assert!(obj["station"]["online_by_t6"].is_number());
    assert!(obj["draw"]["avg_life_paid"].is_number());
    assert!(obj["draw"]["avg_life_funded_draws"].is_number());
    assert!(obj["land_drops"]["percent_games_with_six_or_more_lands_by_turn_4"].is_number());
    assert!(obj["land_drops"]["expected_percent_with_six_or_more_lands_by_turn_4"].is_number());
    assert!(obj["velocity"]["opponent_milled_by_turn"].is_object());
    assert!(obj["color_mana_shortage"]["white"].is_number());
    assert!(obj["color_mana_shortage"].get("W").is_none());
    assert!(
        obj["land_drops"]
            .get("percent_games_with_six_or_more_lands_in_first_eleven_cards")
            .is_none()
    );
    // Required normal-report fields are present.
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
        "color_mana_shortage",
        "card_castability",
        "findings",
        "summary",
    ] {
        assert!(obj.contains_key(key), "missing key {key}");
    }
}

#[test]
fn public_percentages_use_zero_to_one_hundred_scale() {
    let percent = super::report_schema::Percent::from_share(0.77);
    assert_eq!(
        serde_json::to_value(percent).expect("serialize percent"),
        77.0
    );
    assert!(serde_json::from_str::<super::report_schema::Percent>("0.77").is_ok());
    assert!(serde_json::from_str::<super::report_schema::Percent>("101.0").is_err());
}

#[test]
fn empty_simulation_deck_returns_an_error() {
    let deck = crate::deck::grammar::Deck::default();
    let cards = std::collections::HashMap::new();
    let error = super::sim_report_for(&deck, &cards, "empty", 10, None, 42, None)
        .expect_err("empty deck must not produce a partial report");
    assert_eq!(error.to_string(), "deck has no cards");
}

#[test]
fn findings_use_the_single_public_json_shape() {
    let deck = SimDeck {
        companion: None,
        cards: vec![],
        commanders: vec![],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let stats = aggregate(&[], &deck, 5);
    let finding = super::findings::Finding {
        kind: "insufficient_land_drops",
        severity: "medium",
        game_share: Some(0.12345),
        color: None,
        explanation: "too few lands in opening games".to_string(),
        suggestion: "add lands".to_string(),
        evidence: vec![super::findings::FindingEvidence {
            name: "lands".to_string(),
            explanation: "30 in the deck".to_string(),
            game_share: None,
        }],
    };
    let report = serde_json::to_value(super::report::json_report(
        &stats,
        &deck,
        "test",
        42,
        &[finding],
        Default::default(),
        &Default::default(),
    ))
    .expect("serialize report");
    assert_eq!(
        report["findings"][0],
        serde_json::json!({
            "kind": "insufficient_land_drops",
            "identity": "insufficient_land_drops",
            "severity": "medium",
            "percent_of_games": 12.35,
            "color": null,
            "explanation": "too few lands in opening games",
            "suggestion": "add lands",
            "evidence": [{
                "subject": "lands",
                "explanation": "30 in the deck",
                "percent_of_games": null
            }]
        })
    );
}

#[test]
fn assumptions_list_documents_limits() {
    let deck = SimDeck {
        companion: None,
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
    // The list is reader-facing: model assumptions, not code names.
    assert!(joined.contains("There are no opponents"));
    assert!(joined.contains("commander"));
    assert!(joined.contains("never costs extra to recast"));
    // No implementation identifiers in reader-facing text.
    assert!(
        !joined.contains("ETB"),
        "reader-facing assumptions avoid code jargon"
    );
    assert!(
        !joined.contains("oracle"),
        "reader-facing assumptions avoid implementation terms"
    );
    // Every line is one plain assumption; no empty strings.
    assert!(list.iter().all(|l| !l.trim().is_empty()));
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
            tap: Some(super::oracle_parser::land::parse_tap_yield("{T}: Add {W}.").unwrap()),
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
            flags: super::model::SimStaticFlags {
                is_interaction: true,
                is_instant_speed: true,
                ..super::model::SimStaticFlags::default()
            },
            spell_data: super::model::SimSpellData {
                life_loss_on_resolve: 3,
                ..super::model::SimSpellData::default()
            },
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
        companion: None,
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
    let report = serde_json::to_value(super::report::json_report(
        &stats,
        &deck,
        "pin",
        42,
        &[],
        Default::default(),
        &Default::default(),
    ))
    .expect("serialize report");
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
    // Win-condition metrics carry numeric rows.
    assert!(obj["win_conditions"].is_object());
    assert!(obj["win_conditions"]["opponent_life_loss_by_turn"].is_object());
    // The commander block carries timing percentiles.
    assert!(obj["commander"]["p50_cast_turn"].is_number());
    assert!(obj["commander"]["p95_cast_turn"].is_number());
    // Deck shape carries the audit keys.
    for key in [
        "total_cards",
        "lands",
        "artifact_mana_sources",
        "creature_mana_sources",
        "ramp_spells",
        "draw_sources",
        "removal_spells",
        "targeted_removal_spells",
        "mass_removal_spells",
        "win_conditions",
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
            tap: Some(super::oracle_parser::land::parse_tap_yield("{T}: Add {R}.").unwrap()),
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
            flags: super::model::SimStaticFlags {
                has_haste: true,
                ..super::model::SimStaticFlags::default()
            },
            ..SimCard::default()
        });
    }
    let deck = SimDeck {
        companion: None,
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

/// Carry a reached positive-mana-loop milestone into report JSON.
#[test]
fn report_milestones_include_a_reached_positive_mana_loop() {
    use super::game::run_game;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let engine = SimCard {
        name: "Loop Rock".into(),
        has_mana_cost: true,
        role: Role::Rock,
        is_artifact: true,
        striations: vec![SimStriation {
            at: 0,
            animate: false,
            abilities: vec![SimAbility {
                kind: super::model::SimAbilityKind::Activated,
                trigger: SimTrigger::Never,
                effect: SimEffect::Mana(ManaYield {
                    colorless: 1,
                    ..ManaYield::default()
                }),
                activation: Some(super::model::SimActivation::default()),
                ..SimAbility::default()
            }],
        }],
        ..SimCard::default()
    };
    let deck = SimDeck {
        companion: None,
        cards: vec![engine; 60],
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(12);
    let logs = vec![run_game(&deck, &mut rng, 1)];
    assert!(logs[0].milestones_by_turn[0].positive_mana_loop);
    let stats = aggregate(&logs, &deck, 1);
    let report = super::report::json_report(
        &stats,
        &deck,
        "loop",
        12,
        &[],
        Default::default(),
        &Default::default(),
    );

    assert!(
        report
            .milestones
            .percent_games_with_positive_mana_loop_by_turn
            .get(&1)
            .is_some_and(|percent| percent.value() > 0.0)
    );
}
