//! Search boundaries preserve result counts, front-face types, and known colors.

use super::card_runtime_tests::{attempt, renamed};
use super::card_test_support::{assert_library_rng, main_phase_state, named_row};
use super::fetch_requirement_boundary_tests::{
    assert_bounded_search_report, assert_bounded_search_state, bounded_search_scenario,
    bounded_search_source,
};
use super::game::{ManaPool, new_perm_with};
use super::game_effects::spend_leftover;
use super::model::{CardIdx, ManaColor, SearchCardType, SearchDestination, SearchSpec, SimEffect};
use super::oracle_ast::{OracleAbility, OracleEffect};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row};
use super::support::AbilitySupportStatus;
use crate::db::CardRow;

/// Compare every search restriction with an independently written expected rule.
fn assert_search_spec(actual: &SearchSpec, expected: &SearchSpec) {
    assert_eq!(actual.card_type, expected.card_type);
    assert_eq!(actual.color, expected.color);
    assert_eq!(actual.colorless, expected.colorless);
    assert_eq!(actual.mana_value, expected.mana_value);
    assert_eq!(actual.max_mana_value, expected.max_mana_value);
    assert_eq!(actual.min_mana_value, expected.min_mana_value);
    assert_eq!(actual.destination, expected.destination);
    assert_eq!(actual.optional, expected.optional);
    assert_eq!(actual.top_count, expected.top_count);
    assert_eq!(actual.shuffle, expected.shuffle);
    assert_eq!(actual.non_human, expected.non_human);
    assert_eq!(actual.land_types, expected.land_types);
    assert_eq!(actual.basic_land_only, expected.basic_land_only);
}

/// Require the same complete search at the syntax and executable compiler boundaries.
fn assert_compiled_search(source: &CardRow, expected: SearchSpec) {
    let oracle = parse_oracle_card(source);
    let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
        panic!("the search source has one spell ability");
    };
    let [OracleEffect::Search(actual)] = spell.effects.as_slice() else {
        panic!("the supported selector parses as one search");
    };
    assert_search_spec(actual, &expected);
    let lowered = parse_sim_card(source);
    let [SimEffect::Search(actual)] = lowered.spell_data.effects.as_slice() else {
        panic!("the supported selector lowers as one search");
    };
    assert_search_spec(actual, &expected);
}

/// A supported union and its exact single-type controls retain different selectors.
#[test]
fn search_parser_keeps_exact_instant_sorcery_and_union_types() {
    for source in renamed(&named_row("Mystical Tutor")) {
        assert_compiled_search(
            &source,
            SearchSpec {
                card_type: Some(SearchCardType::InstantSorcery),
                destination: SearchDestination::LibraryTop,
                shuffle: true,
                ..SearchSpec::default()
            },
        );
        for (selector, card_type) in [
            ("an instant", SearchCardType::Instant),
            ("a sorcery", SearchCardType::Sorcery),
        ] {
            let mut changed = source.clone();
            changed.oracle_text = format!(
                "Search your library for {selector} card, put it into your hand, then shuffle."
            );
            assert_compiled_search(
                &changed,
                SearchSpec {
                    card_type: Some(card_type),
                    shuffle: true,
                    ..SearchSpec::default()
                },
            );
        }
    }
}

/// Cardinality checks must not consume numbers that belong to a mana-value relation.
#[test]
fn single_search_counts_preserve_numeric_mana_value_relations() {
    for (count, optional) in [
        ("a", false),
        ("one", false),
        ("1", false),
        ("up to one", true),
        ("up to 1", true),
    ] {
        for (relation, exact, maximum, minimum) in [
            ("0", Some(0), None, None),
            ("equal to 2", Some(2), None, None),
            ("exactly 3", Some(3), None, None),
            ("3 or less", None, Some(3), None),
            ("2 or more", None, None, Some(2)),
            ("3 or greater", None, None, Some(3)),
        ] {
            let source = row(
                "Numeric search control",
                "{U}",
                "Instant",
                &format!(
                    "Search your library for {count} instant card with mana value {relation}, put it into your hand, then shuffle."
                ),
            );
            assert_compiled_search(
                &source,
                SearchSpec {
                    card_type: Some(SearchCardType::Instant),
                    mana_value: exact,
                    max_mana_value: maximum,
                    min_mana_value: minimum,
                    optional,
                    shuffle: true,
                    ..SearchSpec::default()
                },
            );
        }
    }
}

/// Invalid counts, mismatched result nouns, and mixed selectors cannot become a broad search.
#[test]
fn search_parser_rejects_non_single_results_and_unsupported_type_unions() {
    for selector in [
        "zero basic land cards",
        "0 basic land cards",
        "a basic land cards",
        "one basic land cards",
        "basic land cards",
        "two basic land card",
        "up to zero basic land cards",
        "up to one basic land cards",
        "any number of basic land cards",
        "a basic land card and a basic land card",
        "one basic land card and one basic land card",
        "an instant or creature card",
        "an instant and sorcery card",
        "an artifact and enchantment card",
    ] {
        let source = row(
            "Unsupported search control",
            "{G}",
            "Sorcery",
            &format!(
                "Search your library for {selector}, put them onto the battlefield tapped, then shuffle. You gain 2 life."
            ),
        );
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
            panic!("the source has one spell ability: {selector}");
        };
        assert!(
            matches!(spell.effects.first(), Some(OracleEffect::Unsupported(_))),
            "{selector}"
        );
        assert!(super::oracle_lower::lower_effect_sequence(&spell.effects).is_none());
        let lowered = parse_sim_card(&source);
        assert!(lowered.spell_data.effects.is_empty(), "{selector}");
        assert_eq!(lowered.spell_data.life_gain_on_cast, 0);
        assert!(
            lowered
                .support
                .iter()
                .all(|ability| matches!(ability.status, AbilitySupportStatus::Unsupported { .. }))
        );
    }
}

/// Complete split searches lower both destinations, while insufficient typed capacity stays inert.
#[test]
fn exact_typed_two_destination_search_is_not_downgraded_to_one_result() {
    for name in ["Cultivate", "Kodama's Reach"] {
        let source = named_row(name);
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
            panic!("the pinned split search is one spell");
        };
        let [OracleEffect::MultiSearch(plan)] = spell.effects.as_slice() else {
            panic!("the complete multi-search grammar retains its typed plan");
        };
        assert_eq!((plan.min_cards, plan.max_cards), (0, 2));
        assert_eq!(plan.destinations.len(), 2);
        let lowered = parse_sim_card(&source);
        let [SimEffect::MultiSearch(actual)] = lowered.spell_data.effects.as_slice() else {
            panic!("both destinations lower as one complete plan");
        };
        assert_eq!((actual.min_cards, actual.max_cards), (0, 2));
        assert_eq!(actual.destinations.len(), 2);
        for (actual, expected) in actual.destinations.iter().zip(&plan.destinations) {
            assert_eq!(actual.destination, expected.destination);
            assert_eq!(actual.max_cards, expected.max_cards);
        }
        let mut invalid = plan.clone();
        invalid.destinations.pop();
        let invalid = OracleEffect::MultiSearch(invalid);
        assert!(invalid.to_runtime().is_none());
        assert!(
            super::oracle_lower::lower_effect_sequence(&[invalid, OracleEffect::GainLife(2)])
                .is_none()
        );
        super::card_boundary_search_tests::assert_split_search_actions(&source);
    }
}

/// Paid exact-type searches reject the other spell type and every permanent, even above a hit.
#[test]
fn paid_searches_match_only_the_named_front_face_spell_type() {
    for original in renamed(&named_row("Mystical Tutor")) {
        for (selector, target_type, other_type) in [
            ("an instant", "Instant", "Sorcery"),
            ("a sorcery", "Sorcery", "Instant"),
        ] {
            let mut source = original.clone();
            source.oracle_text = format!(
                "Search your library for {selector} card, put it into your hand, then shuffle."
            );
            let cards = deck(&[
                source,
                row("Matching spell", "{99}", target_type, ""),
                row("Other spell", "{99}", other_type, ""),
                row("Permanent", "{99}", "Artifact", ""),
                row(
                    "Other spell // matching back",
                    "{99} // {99}",
                    &format!("{other_type} // {target_type}"),
                    "",
                ),
                row(
                    "Matching spell // other back",
                    "{99} // {99}",
                    &format!("{target_type} // {other_type}"),
                    "",
                ),
            ]);
            for (library, found) in [
                (vec![1, 2, 3, 4], Some(1)),
                (vec![2, 3, 4], None),
                (vec![5, 2, 3, 4], Some(5)),
            ] {
                let mut game = main_phase_state(vec![0], library);
                let mut expected = game.library.clone();
                if let Some(found) = found {
                    expected.retain(|index| *index != CardIdx(found));
                }
                let mut expected_rng = game.library_rng.clone();
                super::rng::shuffle(&mut expected, &mut expected_rng);
                let mut pool = ManaPool {
                    fixed: [0, 1, 0, 0, 0],
                    ..ManaPool::default()
                };
                assert_eq!(attempt(&cards, &mut game, &mut pool), Some(1));
                assert_eq!(pool.total(), 0);
                assert_eq!(
                    game.hand,
                    found.map(CardIdx).into_iter().collect::<Vec<_>>()
                );
                assert_eq!(game.library, expected);
                assert_eq!(game.graveyard, [CardIdx(0)]);
                assert!(game.battlefield.is_empty() && game.exile.is_empty());
                assert_eq!(game.seen, u32::from(found.is_some()));
                assert_eq!(game.spells_cast_this_turn, 1);
                assert_library_rng(&game.library_rng, &expected_rng);
            }
        }
    }
}

/// Invalid counts and required name restrictions cannot grant movement, shuffles, or linked rewards.
#[test]
fn paid_non_single_search_does_not_move_cards_shuffle_or_gain_life() {
    for count in ["zero", "two", "three", "up to two"] {
        let restriction = if count == "zero" {
            ""
        } else {
            " with different names"
        };
        let source = row(
            "Unsupported land search",
            "{1}{G}",
            "Sorcery",
            &format!(
                "Search your library for {count} basic land cards{restriction}, put them onto the battlefield tapped, then shuffle. You gain 2 life."
            ),
        );
        assert_paid_inert_search(&source);
    }
}

/// Pinned flattened colors cannot prove that Etali's front face is green.
#[test]
fn pinned_multiface_search_colors_are_unknown_without_changing_flattened_colors() {
    let source = named_row("Etali, Primal Conqueror // Etali, Primal Sickness");
    assert!(source.type_line.contains(" // "));
    let card = parse_sim_card(&source);
    assert_eq!(card.colors, [false, false, false, true, true]);
    assert_eq!(card.search_colors, None);
    assert_eq!(card.card_types, ["legendary", "creature"]);
    let green = parse_sim_card(&named_row("Llanowar Elves"));
    assert_eq!(
        green.search_colors,
        Some([false, false, false, false, true])
    );
}

/// Check one paid creature tutor against a pinned multiface card and an optional real green hit.
fn assert_paid_green_search(source: &CardRow, etali: &CardRow, has_green: bool, colored: bool) {
    let cards = deck(&[
        source.clone(),
        named_row("Llanowar Elves"),
        etali.clone(),
        row("Library marker", "{99}", "Artifact", ""),
    ]);
    let mut game = main_phase_state(vec![0], if has_green { vec![1, 3, 2] } else { vec![3, 2] });
    let found = if colored {
        has_green.then_some(CardIdx(1))
    } else {
        Some(CardIdx(2))
    };
    let mut expected = game.library.clone();
    expected.retain(|index| Some(*index) != found);
    let mut expected_rng = game.library_rng.clone();
    super::rng::shuffle(&mut expected, &mut expected_rng);
    let mut pool = ManaPool {
        fixed: [1, 0, 0, 0, 1],
        ..ManaPool::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(2));
    assert_eq!(pool.total(), 0);
    assert_eq!(game.hand, found.into_iter().collect::<Vec<_>>());
    assert_eq!(game.library, expected);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert!(game.battlefield.is_empty() && game.exile.is_empty());
    assert_eq!(game.seen, u32::from(found.is_some()));
    assert_library_rng(&game.library_rng, &expected_rng);
}

/// Green searches reject unknown front colors; removing only the color filter restores eligibility.
#[test]
fn paid_green_search_rejects_pinned_etali_but_uncolored_search_keeps_its_front_type() {
    for original in renamed(&named_row("Eladamri's Call")) {
        for etali in renamed(&named_row(
            "Etali, Primal Conqueror // Etali, Primal Sickness",
        )) {
            for has_green in [false, true] {
                assert_paid_green_search(&original, &etali, has_green, false);
                let mut colored = original.clone();
                colored.oracle_text = original
                    .oracle_text
                    .replace("a creature card", "a green creature card");
                assert_ne!(colored.oracle_text, original.oracle_text);
                assert_compiled_search(
                    &colored,
                    SearchSpec {
                        card_type: Some(SearchCardType::Creature),
                        color: Some(ManaColor::Green),
                        shuffle: true,
                        ..SearchSpec::default()
                    },
                );
                assert_paid_green_search(&colored, &etali, has_green, true);
            }
        }
    }
}

/// Stored single-face colors, including color indicators and devoid, outrank mana-cost guesses.
#[test]
fn color_search_uses_known_stored_colors_and_rejects_unknown_colorless_faces() {
    let mut indicator = row("Green indicator control", "{R}", "Creature", "");
    indicator.colors = "[\"G\"]".into();
    let devoid = row("Devoid control", "{G}", "Creature", "Devoid");
    let unknown = row("Unknown // back", "{G} // {R}", "Creature // Creature", "");
    let cards = deck(&[indicator, devoid, unknown]);
    assert_eq!(
        cards[CardIdx(0)].search_colors,
        Some([false, false, false, false, true])
    );
    assert_eq!(cards[CardIdx(1)].search_colors, Some([false; 5]));
    assert_eq!(cards[CardIdx(2)].search_colors, None);
    for (spec, expected) in [
        (
            SearchSpec {
                card_type: Some(SearchCardType::Creature),
                color: Some(ManaColor::Green),
                ..SearchSpec::default()
            },
            CardIdx(0),
        ),
        (
            SearchSpec {
                card_type: Some(SearchCardType::Creature),
                colorless: true,
                ..SearchSpec::default()
            },
            CardIdx(1),
        ),
    ] {
        let mut game = main_phase_state(vec![], vec![0, 1, 2]);
        super::game_effects::apply_effect_at(
            &cards,
            &SimEffect::Search(spec),
            &mut game,
            1,
            false,
            None,
        );
        assert_eq!(game.hand, [expected]);
        assert!(game.library.contains(&CardIdx(2)));
    }
}

/// A paid creature's entry search uses the same conservative front-color rule as a spell.
#[test]
fn paid_entry_green_search_rejects_unknown_multiface_colors() {
    for mut source in renamed(&named_row("Gravebreaker Lamia")) {
        source.oracle_text = "When this creature enters, search your library for a green creature card, put it into your hand, then shuffle.".into();
        let cards = deck(&[
            source,
            named_row("Llanowar Elves"),
            named_row("Etali, Primal Conqueror // Etali, Primal Sickness"),
            row("Library marker", "{99}", "Artifact", ""),
        ]);
        for has_green in [false, true] {
            let mut game =
                main_phase_state(vec![0], if has_green { vec![1, 3, 2] } else { vec![3, 2] });
            let mut expected = if has_green {
                vec![CardIdx(3), CardIdx(2)]
            } else {
                game.library.clone()
            };
            let mut expected_rng = game.library_rng.clone();
            super::rng::shuffle(&mut expected, &mut expected_rng);
            let mut pool = ManaPool {
                fixed: [0, 0, 1, 0, 0],
                colorless: 4,
                ..ManaPool::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), Some(5));
            assert_eq!(pool.total(), 0);
            assert_eq!(game.hand, if has_green { vec![CardIdx(1)] } else { vec![] });
            assert_eq!(game.library, expected);
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
            assert!(game.graveyard.is_empty() && game.exile.is_empty());
            assert_eq!(game.seen, u32::from(has_green));
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_library_rng(&game.library_rng, &expected_rng);
        }
    }
}

/// Full-turn reports count only a known-color hit and still record a failed search's real cost.
#[test]
fn green_search_full_turn_reports_only_known_front_color_hits() {
    for mut source in renamed(&named_row("Eladamri's Call")) {
        source.oracle_text = source
            .oracle_text
            .replace("a creature card", "a green creature card");
        let cards = deck(&[
            source.clone(),
            named_row("Llanowar Elves"),
            named_row("Etali, Primal Conqueror // Etali, Primal Sickness"),
            row("Library marker", "{99}", "Artifact", ""),
            row("Funding Plains", "", "Basic Land — Plains", "{T}: Add {W}."),
            row("Funding Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        ]);
        for has_green in [false, true] {
            let mut game =
                main_phase_state(vec![0], if has_green { vec![1, 3, 2] } else { vec![3, 2] });
            for index in [4, 5] {
                game.battlefield
                    .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
            }
            game.next_uid = 6;
            let mut expected = vec![CardIdx(3), CardIdx(2)];
            let mut expected_rng = game.library_rng.clone();
            super::rng::shuffle(&mut expected, &mut expected_rng);
            let mut census = super::game_run::TurnCensus::new(1, cards.cards.len());
            super::game_run::run_turn(
                &cards,
                &mut game,
                &mut census,
                &super::game_commander::CommanderProfile::new(&cards),
                &mut Vec::new(),
                1,
                false,
            );
            assert_eq!(game.hand, if has_green { vec![CardIdx(1)] } else { vec![] });
            assert_eq!(game.library, expected);
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(game.battlefield.len(), 2);
            assert_library_rng(&game.library_rng, &expected_rng);
            let log = super::game_run::census::finish_log(&cards, game, census, 0, false, 1);
            assert_eq!(log.mana_spent, [2.0]);
            assert_eq!(log.mana_remaining, [0.0]);
            assert_eq!(log.cards_seen, [u32::from(has_green)]);
            assert_eq!(log.library_size, [2]);
            let stats = super::aggregate::aggregate(std::slice::from_ref(&log), &cards, 1);
            let report = super::report::json_report(
                &stats,
                &cards,
                "Front-face search boundary",
                0,
                &[],
                Default::default(),
                &Default::default(),
            );
            let json = serde_json::to_value(report).expect("the production report serializes");
            assert_eq!(json["outcomes"]["finished_games"], 1);
            assert_eq!(json["outcomes"]["incomplete_games"], 0);
            assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 2.0);
            assert_eq!(
                json["velocity"]["avg_cards_seen"]["1"],
                f64::from(u32::from(has_green))
            );
            assert_eq!(json["velocity"]["library_remaining_by_turn"]["1"], 2.0);
            let evidence = json["card_support"]
                .as_array()
                .expect("the report has compiler evidence")
                .iter()
                .find(|card| card["name"] == source.name)
                .expect("the search source is reported");
            assert_eq!(
                evidence["source_hash"],
                super::support::card_source_hash(&source)
            );
            assert_eq!(evidence["abilities"].as_array().map(Vec::len), Some(1));
            assert_eq!(evidence["abilities"][0]["identity"], "ability:0");
            assert_eq!(evidence["abilities"][0]["source"], source.oracle_text);
            assert_eq!(evidence["abilities"][0]["span_start"], 0);
            assert_eq!(
                evidence["abilities"][0]["span_end"],
                source.oracle_text.len()
            );
            assert_eq!(evidence["abilities"][0]["status"], "compiler_only");
            assert!(evidence["abilities"][0]["rule_id"].is_null());
            assert_eq!(
                evidence["abilities"][0]["reason"],
                "Ordered effects lowered, but their ordinary runtime resolution path is not verified."
            );
        }
    }
}

/// Farseek's commas describe one OR-list, not the end of its selector.
#[test]
fn pinned_farseek_preserves_the_whole_land_type_list_through_paid_searches() {
    for source in renamed(&named_row("Farseek")) {
        assert_compiled_search(
            &source,
            SearchSpec {
                card_type: Some(SearchCardType::Land),
                destination: SearchDestination::BattlefieldTapped,
                shuffle: true,
                land_types: [true, true, true, true, false],
                ..SearchSpec::default()
            },
        );
        for subtype in ["Plains", "Island", "Swamp", "Mountain"] {
            for basic in [false, true] {
                let target_type = if basic {
                    format!("Basic Land — {subtype}")
                } else {
                    format!("Land — {subtype} Forest")
                };
                let cards = deck(&[
                    source.clone(),
                    row("Allowed land", "", &target_type, ""),
                    row("Excluded Forest", "", "Basic Land — Forest", ""),
                    row("Nonland", "{99}", "Artifact", ""),
                    row(
                        "Spell // Plains",
                        "{99} // ",
                        "Sorcery // Land — Plains",
                        "",
                    ),
                ]);
                for has_match in [false, true] {
                    let mut game = main_phase_state(
                        vec![0],
                        if has_match {
                            vec![1, 2, 3, 4]
                        } else {
                            vec![2, 3, 4]
                        },
                    );
                    let mut expected = vec![CardIdx(2), CardIdx(3), CardIdx(4)];
                    let mut expected_rng = game.library_rng.clone();
                    super::rng::shuffle(&mut expected, &mut expected_rng);
                    let mut pool = ManaPool {
                        fixed: [0, 0, 0, 0, 1],
                        colorless: 1,
                        ..ManaPool::default()
                    };
                    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(2));
                    assert_eq!(pool.total(), 0);
                    assert_eq!(game.library, expected);
                    assert_eq!(game.battlefield.len(), usize::from(has_match));
                    if has_match {
                        assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(1)));
                        assert!(game.battlefield[0].tapped);
                    }
                    assert!(game.hand.is_empty() && game.exile.is_empty());
                    assert_eq!(game.graveyard, [CardIdx(0)]);
                    assert_eq!(game.seen, u32::from(has_match));
                    assert_eq!(game.spells_cast_this_turn, 1);
                    assert_library_rng(&game.library_rng, &expected_rng);
                }
            }
        }
    }
}

/// A subtype does not replace the required Basic supertype.
#[test]
fn basic_forest_search_rejects_nonbasic_forests_and_text_changes_restore_them() {
    for original in renamed(&named_row("Farseek")) {
        for basic_only in [false, true] {
            let mut source = original.clone();
            source.oracle_text = format!(
                "Search your library for a {}Forest card, put it onto the battlefield tapped, then shuffle.",
                if basic_only { "basic " } else { "" }
            );
            assert_compiled_search(
                &source,
                SearchSpec {
                    card_type: Some(if basic_only {
                        SearchCardType::BasicLand
                    } else {
                        SearchCardType::Land
                    }),
                    destination: SearchDestination::BattlefieldTapped,
                    shuffle: true,
                    land_types: [false, false, false, false, true],
                    basic_land_only: basic_only,
                    ..SearchSpec::default()
                },
            );
            let cards = deck(&[
                source,
                row("Basic Forest", "", "Basic Land — Forest", ""),
                row("Nonbasic Forest", "", "Land — Plains Forest", ""),
                row("Basic Plains", "", "Basic Land — Plains", ""),
            ]);
            for has_basic in [false, true] {
                let found = if basic_only {
                    has_basic.then_some(CardIdx(1))
                } else {
                    Some(CardIdx(2))
                };
                let mut game =
                    main_phase_state(vec![0], if has_basic { vec![1, 2, 3] } else { vec![2, 3] });
                let mut expected = game.library.clone();
                expected.retain(|index| Some(*index) != found);
                let mut expected_rng = game.library_rng.clone();
                super::rng::shuffle(&mut expected, &mut expected_rng);
                let mut pool = ManaPool {
                    fixed: [0, 0, 0, 0, 1],
                    colorless: 1,
                    ..ManaPool::default()
                };
                assert_eq!(attempt(&cards, &mut game, &mut pool), Some(2));
                assert_eq!(pool.total(), 0);
                assert_eq!(game.library, expected);
                assert_eq!(game.battlefield.len(), usize::from(found.is_some()));
                if let Some(found) = found {
                    assert_eq!(game.battlefield[0].card.deck_idx(), Some(found));
                    assert!(game.battlefield[0].tapped);
                }
                assert!(game.hand.is_empty() && game.exile.is_empty());
                assert_eq!(game.graveyard, [CardIdx(0)]);
                assert_eq!(game.seen, u32::from(found.is_some()));
                assert_library_rng(&game.library_rng, &expected_rng);
            }
        }
    }
}

/// Unsupported search requirements block every movement and linked reward after printed payment.
fn assert_paid_inert_search(source: &CardRow) {
    let oracle = parse_oracle_card(source);
    let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
        panic!("the search source is one spell");
    };
    assert!(matches!(
        spell.effects.first(),
        Some(OracleEffect::Unsupported(_))
    ));
    assert!(super::oracle_lower::lower_effect_sequence(&spell.effects).is_none());
    let card = parse_sim_card(source);
    assert!(card.spell_data.effects.is_empty());
    assert_eq!(card.spell_data.life_gain_on_cast, 0);
    assert!(
        card.support
            .iter()
            .all(|ability| matches!(ability.status, AbilitySupportStatus::Unsupported { .. }))
    );
    let cards = deck(&[
        source.clone(),
        row("Basic Forest", "", "Basic Land — Forest", ""),
        row("Nonbasic Forest Plains", "", "Land — Plains Forest", ""),
        row("Two-mana artifact", "{2}", "Artifact", ""),
        row("Three-mana artifact", "{3}", "Artifact", ""),
        row("Instant", "{2}", "Instant", ""),
        row("Graveyard marker", "{99}", "Artifact", ""),
        row("Exile marker", "{99}", "Artifact", ""),
    ]);
    let mut game = main_phase_state(vec![0], vec![1, 2, 3, 4, 5]);
    game.graveyard = vec![CardIdx(6)];
    game.exile = vec![CardIdx(7)];
    let expected_rng = game.library_rng.clone();
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 0, 1],
        colorless: 1,
        ..ManaPool::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(2));
    assert_eq!(pool.total(), 0);
    assert_eq!(
        game.library,
        [CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(5)]
    );
    assert_eq!(game.graveyard, [CardIdx(6), CardIdx(0)]);
    assert_eq!(game.exile, [CardIdx(7)]);
    assert!(game.hand.is_empty() && game.battlefield.is_empty());
    assert_eq!(game.seen, 0);
    assert_eq!(game.awareness_cards, 0);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_gained, 0);
    assert_eq!(game.extra_land_drops_this_turn, 0);
    assert_eq!(game.next_uid, 0);
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_library_rng(&game.library_rng, &expected_rng);
}

/// A land-type intersection cannot become the runtime's broader OR mask.
#[test]
fn land_search_rejects_subtype_conjunctions_but_keeps_explicit_alternatives() {
    for original in renamed(&named_row("Farseek")) {
        for selector in [
            "a Plains Forest card",
            "a Plains and Forest card",
            "a Plains, Forest card",
            "a Plains Forest or Island card",
            "a Plains, Island, and Forest card",
        ] {
            let mut source = original.clone();
            source.oracle_text = format!(
                "Search your library for {selector}, put it onto the battlefield tapped, then shuffle. You gain 2 life."
            );
            assert_paid_inert_search(&source);
        }
        for selector in ["a Plains or Forest card", "a Plains, or Forest card"] {
            let mut source = original.clone();
            source.oracle_text = format!(
                "Search your library for {selector}, put it onto the battlefield tapped, then shuffle."
            );
            assert_compiled_search(
                &source,
                SearchSpec {
                    card_type: Some(SearchCardType::Land),
                    destination: SearchDestination::BattlefieldTapped,
                    shuffle: true,
                    land_types: [true, false, false, false, true],
                    ..SearchSpec::default()
                },
            );
        }
    }
}

/// Searching, revealing, and shuffling do not supply an unwritten hand destination.
#[test]
fn paid_search_without_a_movement_destination_keeps_all_library_cards_in_place() {
    for original in renamed(&named_row("Farseek")) {
        for text in [
            "Search your library for a basic land card.",
            "Search your library for a basic land card, then shuffle.",
            "Search your library for a basic land card, reveal it, then shuffle.",
            "Search your library for a basic land card. You gain 2 life.",
            "Search your library for a basic land card, then shuffle. You gain 2 life.",
            "Search your library for a green creature card with mana value exactly 3, then shuffle.",
        ] {
            let mut source = original.clone();
            source.oracle_text = text.into();
            assert_paid_inert_search(&source);
        }
    }
}

/// An optional activation also needs a written destination before it can lower.
#[test]
fn optional_search_without_a_destination_cannot_lower_to_a_hand_search() {
    let source = row(
        "Optional search control",
        "{1}",
        "Artifact",
        "{G}: You may search your library for a green creature card with mana value exactly 3, then shuffle.",
    );
    let oracle = parse_oracle_card(&source);
    let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
        panic!("the optional search retains its activation");
    };
    assert!(matches!(
        ability.effects.as_slice(),
        [OracleEffect::Unsupported(_)]
    ));
    assert!(ability.to_runtime_all().is_empty());
    let lowered = parse_sim_card(&source);
    assert!(lowered.unlocked_abilities(0).next().is_none());
    assert!(matches!(
        lowered.support[0].status,
        AbilitySupportStatus::Unsupported { .. }
    ));
}

/// Each unsupported numeric clause remains required, including clauses after a list comma.
#[test]
fn paid_search_rejects_repeated_and_compound_mana_value_relations() {
    for original in renamed(&named_row("Farseek")) {
        for relation in [
            "2 and mana value 3",
            "2 or mana value 3",
            "2 or 3",
            "2 and 3",
            "2 or less and 1 or more",
            "2 or less and mana value 1 or more",
            "2 with mana value 3",
            "equal to 2 and equal to 3",
            "equal to 2 or equal to 3",
            "2 or less or greater",
            "2, and mana value 3",
            "2, then 3 or less",
            "2 or less, with mana value 1 or more",
        ] {
            let mut source = original.clone();
            source.oracle_text = format!(
                "Search your library for an artifact card with mana value {relation}, put it into your hand, then shuffle. You gain 2 life."
            );
            assert_paid_inert_search(&source);
        }
    }
}

/// Unknown search sources stay visible and cannot execute after printed spell payment.
#[test]
fn paid_search_rejects_extra_source_zones_and_unknown_source_modifiers() {
    for source in [
        "your library and an opponent's library",
        "your library and an opponent’s library",
        "your library and/or graveyard",
        "your library and your graveyard",
        "your library or your graveyard",
        "your library and cards you own in exile",
        "your library and your hand",
        "your library except the top card",
        "your library's top five cards",
    ] {
        let text = format!(
            "Search {source} for a Plains, Island, Swamp, or Mountain card, put it onto the battlefield tapped, then shuffle. You gain 2 life."
        );
        let source = row("Unsupported search source", "{1}{G}", "Sorcery", &text);
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
            panic!("the full source remains one spell");
        };
        let [OracleEffect::Unsupported(search), OracleEffect::GainLife(2)] =
            spell.effects.as_slice()
        else {
            panic!("the exact unsupported search and linked reward stay visible");
        };
        assert_eq!(format!("{search} You gain 2 life."), source.oracle_text);
        let [printed] = oracle.ability_sources.as_slice() else {
            panic!("the original source statement remains intact");
        };
        assert_eq!(printed.text, source.oracle_text);
        assert_paid_inert_search(&source);
    }
}

/// Destination references do not add a source, while a required unmodeled remainder blocks the search.
#[test]
fn supported_library_source_keeps_subtype_lists_and_top_card_selectors() {
    let source = row(
        "Library-top source control",
        "{G}",
        "Sorcery",
        "Search your library for a Plains, Island, Swamp, or Mountain card, then shuffle and put that card on top of your library.",
    );
    assert_compiled_search(
        &source,
        SearchSpec {
            card_type: Some(SearchCardType::Land),
            destination: SearchDestination::LibraryTop,
            shuffle: true,
            land_types: [true, true, true, true, false],
            ..SearchSpec::default()
        },
    );
    for verb in ["Look at", "Reveal"] {
        let source = row(
            "Bounded library source control",
            "",
            "Land",
            &format!(
                "{{T}}: {verb} the top five cards of your library. You may put a non-Human creature card from among them onto the battlefield. Put the rest on the bottom of your library."
            ),
        );
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
            panic!("the bounded selector belongs to one activation");
        };
        let [OracleEffect::Unsupported(resolution)] = ability.effects.as_slice() else {
            panic!("the full bounded source stays in one unsupported node");
        };
        let (_, expected) = source
            .oracle_text
            .split_once(": ")
            .expect("activation separator");
        assert_eq!(resolution, expected);
        let card = parse_sim_card(&source);
        assert!(card.unlocked_abilities(0).next().is_none());
        assert!(matches!(
            card.support[0].status,
            AbilitySupportStatus::Unsupported { .. }
        ));
    }
}

/// Paid casts and reports separate bare selection, rejected bottom rules, and legal no-match searches.
#[test]
fn bounded_search_remainder_rules_block_the_complete_paid_ability_and_report() {
    for remainder in [
        "",
        " Put the rest on the bottom of your library.",
        " Put the rest on the bottom of your library in a random order.",
    ] {
        let source = bounded_search_source(remainder);
        let supported = remainder.is_empty();
        let oracle = parse_oracle_card(&source);
        let text = &source.oracle_text;
        let (_, resolution) = text.split_once(": ").expect("activation separator");
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
            panic!("the complete bounded resolution belongs to one activation");
        };
        assert_eq!(oracle.ability_sources[0].text, source.oracle_text);
        if supported {
            let [OracleEffect::Search(spec)] = ability.effects.as_slice() else {
                panic!("bare selection keeps its exact top-card bound");
            };
            assert_search_spec(
                spec,
                &SearchSpec {
                    card_type: Some(SearchCardType::Artifact),
                    destination: SearchDestination::Hand,
                    optional: true,
                    top_count: Some(3),
                    ..SearchSpec::default()
                },
            );
        } else {
            let [OracleEffect::Unsupported(search), OracleEffect::GainLife(2)] =
                ability.effects.as_slice()
            else {
                panic!("the full required remainder and linked reward remain explicit");
            };
            assert_eq!(format!("{search} You gain 2 life."), resolution);
        }
        let lowered = parse_sim_card(&source);
        assert_eq!(lowered.unlocked_abilities(0).next().is_some(), supported);
        for matching in [false, true] {
            let (cards, mut game) = bounded_search_scenario(&source, matching);
            let expected_rng = game.library_rng.clone();
            let mut pool = ManaPool {
                fixed: [0, 0, 0, 0, 1],
                colorless: 2,
                ..ManaPool::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), Some(2));
            assert_eq!(pool.total(), 1);
            spend_leftover(&cards, &mut game, &mut pool, 1);
            assert_eq!(pool.total(), u32::from(!supported));
            assert_bounded_search_state(&game, supported, matching);
            assert_library_rng(&game.library_rng, &expected_rng);
            assert_bounded_search_report(&source, supported, matching);
        }
    }
}
