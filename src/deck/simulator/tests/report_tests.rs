//! Tests for the simulator report module.

use super::aggregate::aggregate;
use super::model::*;

/// Check the complete set of fields in a report object.
fn assert_object_keys(value: &serde_json::Value, expected: &[&str]) {
    let actual: std::collections::BTreeSet<_> = value
        .as_object()
        .expect("report object")
        .keys()
        .map(String::as_str)
        .collect();
    let expected: std::collections::BTreeSet<_> = expected.iter().copied().collect();
    assert_eq!(actual, expected);
}

#[test]
fn deck_without_commander_has_no_commander_potential_rows() {
    let deck = SimDeck {
        companion: None,
        cards: vec![],
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let stats = aggregate(&[], &deck, 1);
    let report = super::report::json_report(
        &stats,
        &deck,
        "empty",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );

    assert!(report.combat.commander_potential.is_empty());
    assert!(report.commander.is_none());
}

/// Current reports keep exact field sets and spacecraft commander metrics.
#[test]
fn json_report_preserves_current_shape_and_station_metrics() {
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
    typed.combo_access = Some(vec![super::report_schema::PairAccessReport::from_access(
        &super::findings::ComboAccess {
            pair: "A + B".to_string(),
            target_turn: 4,
            game_share: 0.77,
            event_games: 77,
            observed_games: 100,
        },
        100,
        10,
    )]);
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
    typed.hypgeo = Some(super::hypgeo::modeled_access(&deck, 10));
    let report = serde_json::to_value(&typed).expect("serialize report");
    let parsed: super::report_schema::SimReport =
        serde_json::from_value(report.clone()).expect("deserialize complete report");
    assert_eq!(parsed, typed);
    let obj = report.as_object().unwrap();
    assert!(obj.contains_key("station"));
    assert!(obj.contains_key("creatures_by_turn"));
    assert!(obj.contains_key("repeatable_sources_by_turn"));
    assert!(obj.contains_key("assumptions"));
    assert_eq!(obj["outcomes"]["requested_games"], serde_json::json!(0));
    assert_eq!(obj["outcomes"]["finished_games"], serde_json::json!(0));
    assert!(obj["milestones"].is_object());
    assert!(obj["milestones"]["dredge_uses_avg_by_turn"].is_object());
    assert!(obj["milestones"]["percent_games_with_positive_mana_loop_by_turn"].is_object());
    // Station metrics present for a spacecraft commander.
    assert!(obj["station"].is_object());
    assert_eq!(
        obj["combat"]["commander_potential"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        obj["combat"]["commander_potential"][0]["slot"],
        serde_json::json!(0)
    );
    assert!(
        obj["combat"]["commander_potential"][0]["note"]
            .as_str()
            .unwrap()
            .contains("Potential only")
    );
    assert!(obj["combat"]["commander_potential"][0]["percent_games_reaching_21"].is_null());
    assert_eq!(
        obj["combat"]["commander_potential"][0]["observed_games"],
        serde_json::json!(0)
    );
    assert!(obj["station"]["online_by_t6"].is_null());
    assert!(obj["draw"]["avg_life_paid"].is_null());
    assert!(obj["draw"]["avg_life_funded_draws"].is_null());
    assert!(obj["land_drops"]["percent_games_with_six_or_more_lands_by_turn_4"].is_null());
    assert!(obj["land_drops"]["expected_percent_with_six_or_more_lands_by_turn_4"].is_null());
    assert!(obj["velocity"]["opponent_milled_by_turn"].is_object());
    assert!(obj["color_mana_shortage"]["white"].is_null());
    assert_object_keys(
        &obj["color_mana_shortage"],
        &["white", "blue", "black", "red", "green"],
    );
    assert_object_keys(
        &obj["land_drops"],
        &[
            "percent_games_hitting_all_land_drops_by_turn",
            "percent_games_with_two_or_fewer_lands_by_turn_4",
            "percent_games_with_six_or_more_lands_by_turn_4",
            "expected_percent_with_six_or_more_lands_by_turn_4",
            "p50_drops_by_4",
            "p95_drops_by_4",
        ],
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
fn event_free_observations_have_a_wilson_interval_not_an_unavailable_status() {
    use super::report_schema::{ObservationStatus, RateEstimate};
    let observed = RateEstimate::observed(0, 100);
    assert_eq!(observed.status, ObservationStatus::Observed);
    assert_eq!(observed.observed_games, 100);
    assert_eq!(observed.event_games, Some(0));
    assert_eq!(observed.percent.expect("observed rate").value(), 0.0);
    let interval = observed.interval.expect("binomial interval");
    assert_eq!(interval.method, "wilson_score");
    assert_eq!(interval.confidence_level, 0.95);
    assert_eq!(interval.lower.value(), 0.0);
    assert_eq!(interval.upper.value(), 3.7);
    let empty = RateEstimate::observed(0, 0);
    assert_eq!(empty.status, ObservationStatus::NoObservations);
    assert!(empty.event_games.is_none());
    assert!(empty.percent.is_none());
    assert!(empty.interval.is_none());
}

#[test]
fn pair_report_separates_missing_pieces_unobserved_turns_and_event_free_games() {
    use super::report_schema::{ObservationStatus, PairAccessReport};
    let row = |target_turn| super::findings::ComboAccess {
        pair: "A + B".to_string(),
        target_turn,
        game_share: 0.0,
        event_games: 0,
        observed_games: 100,
    };
    let missing = PairAccessReport::from_access(&row(0), 100, 4);
    assert_eq!(missing.access.status, ObservationStatus::NotApplicable);
    assert!(missing.percent_of_games.is_none());
    let outside = PairAccessReport::from_access(&row(8), 100, 4);
    assert_eq!(outside.access.status, ObservationStatus::OutsideHorizon);
    assert!(outside.percent_of_games.is_none());
    let observed = PairAccessReport::from_access(&row(4), 100, 4);
    assert_eq!(observed.access.status, ObservationStatus::Observed);
    assert_eq!(observed.access.event_games, Some(0));
    assert_eq!(observed.access.observed_games, 100);
    assert_eq!(
        observed.percent_of_games.expect("observed zero").value(),
        0.0
    );
}

#[test]
fn short_horizon_metrics_are_null_and_report_the_unobserved_window() {
    let deck = SimDeck {
        companion: None,
        cards: vec![],
        commanders: vec![SimCard {
            name: "Commander".to_string(),
            cost: Cost {
                generic: 5,
                ..Cost::default()
            },
            ..SimCard::default()
        }],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let stats = aggregate(&[], &deck, 2);
    let report = super::report::json_report(
        &stats,
        &deck,
        "short",
        7,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert!(
        report
            .land_drops
            .percent_games_with_two_or_fewer_lands_by_turn_4
            .is_none()
    );
    assert!(
        report
            .role_access
            .removal_spell_percent_seen_by_turn_5
            .is_none()
    );
    assert!(
        report
            .role_access
            .draw_source_percent_seen_by_turn_6
            .is_none()
    );
    assert!(
        report
            .role_access
            .creature_role_percent_seen_by_turn_3
            .is_none()
    );
    assert!(
        report
            .role_access
            .win_condition_role_percent_seen_by_turn_8
            .is_none()
    );
    assert!(report.combat.attack_power_p90_by_t8.is_none());
    let commander = report.commander.expect("commander metadata");
    assert!(commander.percent_castable_by_curve.is_none());
    assert!(commander.avg_first_cast_turn.is_none());
    assert!(commander.p50_cast_turn.is_none());
    assert_eq!(commander.payable_games, 0);
    assert_eq!(
        report.sampling["role_access.draw_source_percent_seen_by_turn_6"].status,
        super::report_schema::ObservationStatus::OutsideHorizon
    );
    assert!(report.summary.contains("outside horizon"));
    assert!(
        report.opponent_model.is_none(),
        "actual settings must come from the runner"
    );
}

#[test]
fn report_records_actual_opponent_overrides_and_order_independent_deck_contents() {
    let mut deck = SimDeck {
        companion: None,
        cards: vec![
            SimCard {
                name: "A".to_string(),
                ..SimCard::default()
            },
            SimCard {
                name: "B".to_string(),
                ..SimCard::default()
            },
        ],
        commanders: vec![],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let report_for = |deck: &SimDeck| {
        super::report::json_report(
            &aggregate(&[], deck, 4),
            deck,
            "test",
            7,
            &[],
            Default::default(),
            &Default::default(),
        )
    };
    let mut first = report_for(&deck);
    deck.cards.reverse();
    let second = report_for(&deck);
    assert_eq!(
        first.provenance.deck_fingerprint,
        second.provenance.deck_fingerprint
    );
    assert_ne!(
        first.provenance.execution_order_fingerprint,
        second.provenance.execution_order_fingerprint
    );
    assert!(first.provenance.card_data_fingerprint.is_none());
    assert!(
        first
            .card_support
            .iter()
            .all(|card| card.note.contains("Card-source data is missing"))
    );
    for (index, card) in deck.cards.iter_mut().enumerate() {
        card.source_hash = format!("{index:064x}");
    }
    let complete = report_for(&deck);
    assert!(complete.provenance.card_data_fingerprint.is_some());
    deck.cards.push(deck.cards[0].clone());
    let duplicated = report_for(&deck);
    assert_eq!(
        complete.provenance.card_data_fingerprint,
        duplicated.provenance.card_data_fingerprint
    );
    assert_ne!(
        complete.provenance.deck_fingerprint,
        duplicated.provenance.deck_fingerprint
    );
    let config = super::opponents::OpponentConfig {
        active: false,
        land_target: 4,
        ..Default::default()
    };
    super::report::set_opponent_model(
        &mut first,
        deck.format,
        config.clone(),
        &aggregate(&[], &deck, 4),
    );
    assert_eq!(
        first
            .opponent_model
            .expect("actual run configuration")
            .parameters,
        config
    );
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
    assert!(joined.contains("declared opponent scenario"));
    assert!(joined.contains("commander"));
    assert!(joined.contains("previous cast from that zone adds {2}"));
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
            has_mana_cost: true,
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
            has_mana_cost: true,
            is_permanent: true,
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
            has_mana_cost: true,
            is_creature: true,
            is_permanent: true,
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
            has_mana_cost: true,
            is_permanent: true,
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
