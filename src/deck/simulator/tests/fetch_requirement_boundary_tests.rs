//! Fetch metadata cannot omit costs, search filters, or linked resolution text.

use super::card_report_tests::{execute_turn, report_executed_turn};
use super::card_runtime_tests::renamed;
use super::card_test_support::{assert_library_rng, main_phase_state, named_row};
use super::cast_pass::play_land;
use super::game::{CardRef, GameState, ManaPool, new_perm_with};
use super::game_effects::activation::{pick_best_activation, resolve_activation};
use super::game_effects::spend_leftover;
use super::model::{BasicLandType, CardIdx, ManaColor, SearchDestination, SearchSpec, SimDeck};
use super::oracle_ast::{
    AbilityRestriction, ActivationCost, ActivationTarget, CostObject, OracleAbility, OracleEffect,
};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row};
use super::support::AbilitySupportStatus;
use crate::db::CardRow;

/// Build synthetic Oracle text without relying on a known fetch name.
fn fetch_source(cost: &str, selector: &str, destination: &str, tail: &str) -> CardRow {
    row(
        "Boundary fetch",
        "",
        "Land",
        &format!(
            "{cost}: Search your library for {selector}, put it {destination}, then shuffle.{tail}"
        ),
    )
}

/// Reject native metadata without treating the card's name as an admission rule.
fn assert_no_native_fetch(source: &CardRow) {
    let oracle = parse_oracle_card(source);
    assert!(oracle.land.fetch.is_none(), "{}", source.oracle_text);
    assert!(!oracle.land.fetch_enters_tapped);
    let card = parse_sim_card(source);
    assert!(!card.is_fetch_land, "{}", source.oracle_text);
    assert_eq!(card.fetch_life_cost, 0);
    assert!(card.fetch_target_types.is_empty());
    assert!(!card.fetch_basic_only && !card.fetch_enters_tapped);
}

/// A blocked activation is not a legally committed search that failed to find a card.
fn assert_blocked_activation(cards: &SimDeck, game: &mut GameState, pool: &mut ManaPool) {
    let before_zones = (
        game.hand.clone(),
        game.library.clone(),
        game.graveyard.clone(),
        game.exile.clone(),
    );
    let before_board = game
        .battlefield
        .iter()
        .map(|permanent| {
            (
                permanent.uid,
                permanent.card,
                permanent.tapped,
                permanent.counters,
            )
        })
        .collect::<Vec<_>>();
    let before_resources = (game.life, game.life_paid, game.next_uid, game.seen);
    let before_pool = pool.clone();
    let before_rng = game.library_rng.clone();
    assert!(pick_best_activation(cards, game, pool, 1).is_none());
    spend_leftover(cards, game, pool, 1);
    assert_eq!(
        (&game.hand, &game.library, &game.graveyard, &game.exile),
        (
            &before_zones.0,
            &before_zones.1,
            &before_zones.2,
            &before_zones.3
        )
    );
    assert_eq!(
        game.battlefield
            .iter()
            .map(|permanent| (
                permanent.uid,
                permanent.card,
                permanent.tapped,
                permanent.counters
            ))
            .collect::<Vec<_>>(),
        before_board
    );
    assert_eq!(
        (game.life, game.life_paid, game.next_uid, game.seen),
        before_resources
    );
    assert_eq!(*pool, before_pool);
    assert_library_rng(&game.library_rng, &before_rng);
}

/// Unsupported requirements cannot consume payment resources or execute a linked reward.
fn assert_unsupported_fetch_source(source: CardRow) {
    assert_no_native_fetch(&source);
    let cards = deck(&[
        source,
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Marker", "{99}", "Artifact", ""),
    ]);
    assert!(
        cards.cards[0]
            .support
            .iter()
            .all(|record| matches!(record.status, AbilitySupportStatus::Unsupported { .. }))
    );
    let mut game = main_phase_state(vec![0, 2], vec![1, 2]);
    assert!(play_land(&cards, &mut game, 1));
    let mut pool = ManaPool {
        colorless: 9,
        ..ManaPool::default()
    };
    assert_blocked_activation(&cards, &mut game, &mut pool);
    assert_eq!((game.life_gained, game.spells_cast_this_turn), (0, 0));
}

/// Keep the rejected placement verbatim while retaining its separate, nonexecutable reward.
fn assert_unsupported_search_placement(source: CardRow) {
    let oracle = parse_oracle_card(&source);
    let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
        panic!("the complete search remains an activated ability");
    };
    let [OracleEffect::Unsupported(search), OracleEffect::GainLife(2)] = ability.effects.as_slice()
    else {
        panic!("the exact rejected search and linked reward remain separate syntax nodes");
    };
    let (_, resolution) = source
        .oracle_text
        .split_once(": ")
        .expect("activation separator");
    assert_eq!(
        search,
        resolution
            .strip_suffix(" You gain 2 life.")
            .expect("linked reward")
    );
    assert!(oracle.abilities[0].to_runtime().is_empty());
    assert_unsupported_fetch_source(source);
}

/// Required mana cannot become a free native fetch, but the complete typed activation can pay it.
#[test]
fn paid_fetch_requires_its_mana_before_source_sacrifice() {
    let source = fetch_source(
        "{2}, {T}, Pay 1 life, Sacrifice this land",
        "a basic land card",
        "onto the battlefield tapped",
        "",
    );
    for source in renamed(&source) {
        assert_no_native_fetch(&source);
        let cards = deck(&[
            source,
            row("First marker", "{99}", "Artifact", ""),
            row("Second marker", "{99}", "Sorcery", ""),
            row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
            row("Nonbasic forest", "", "Land — Forest", ""),
        ]);
        let mut game = main_phase_state(vec![0], vec![1, 2, 3, 4]);
        assert!(play_land(&cards, &mut game, 1));
        let source_uid = game.battlefield[0].uid;
        let mut pool = ManaPool {
            colorless: 1,
            ..ManaPool::default()
        };
        assert_blocked_activation(&cards, &mut game, &mut pool);
        pool.colorless += 2;
        let mut expected_library = vec![CardIdx(1), CardIdx(2), CardIdx(4)];
        let mut expected_rng = game.library_rng.clone();
        super::rng::shuffle(&mut expected_library, &mut expected_rng);
        let activation = pick_best_activation(&cards, &game, &pool, 1)
            .expect("the shared activation can pay the complete cost");
        assert_eq!(activation.ability_cost.generic, 2);
        resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
        assert_eq!(pool.colorless, 1);
        assert_eq!(pool.total(), 1);
        assert_eq!((game.life, game.life_paid), (19, 1));
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.battlefield.len(), 1);
        assert_ne!(game.battlefield[0].uid, source_uid);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(3)));
        assert!(game.battlefield[0].tapped);
        assert_eq!(game.library, expected_library);
        assert_library_rng(&game.library_rng, &expected_rng);
    }
}

/// Native costs cannot retain just one piece of a larger typed payment.
#[test]
fn native_fetch_rejects_incomplete_source_cost_bundles() {
    let source = fetch_source(
        "{T}, Sacrifice this land",
        "a basic land card",
        "onto the battlefield tapped",
        "",
    );
    let sacrifice = ActivationCost::Sacrifice {
        count: 1,
        object: CostObject::Source,
        exclude_source: false,
    };
    for costs in [
        vec![
            ActivationCost::Tap(ActivationTarget::Source),
            ActivationCost::Sacrifice {
                count: 2,
                object: CostObject::Source,
                exclude_source: false,
            },
        ],
        vec![
            ActivationCost::Tap(ActivationTarget::Source),
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Source,
                exclude_source: true,
            },
        ],
        vec![
            ActivationCost::Tap(ActivationTarget::Source),
            sacrifice.clone(),
            sacrifice.clone(),
        ],
        vec![
            ActivationCost::Tap(ActivationTarget::Source),
            sacrifice.clone(),
            ActivationCost::PayLife(1),
            ActivationCost::PayLife(2),
        ],
        vec![
            ActivationCost::Tap(ActivationTarget::Source),
            ActivationCost::Tap(ActivationTarget::Source),
            sacrifice.clone(),
        ],
        vec![sacrifice],
    ] {
        let mut oracle = parse_oracle_card(&source);
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_mut_slice() else {
            panic!("one complete fetch activation");
        };
        ability.costs = costs;
        let land =
            super::oracle_parser::land::parse_land_data(&source.oracle_text, &oracle.abilities);
        assert!(land.fetch.is_none());
        assert!(!land.fetch_enters_tapped);
    }
}

/// Unknown costs, conditions, selectors, and destinations block the whole real activation.
#[test]
fn unsupported_fetch_requirements_stay_inert_on_the_battlefield() {
    for (cost, selector, destination, tail) in [
        (
            "{T}, Discard a card, Sacrifice this land",
            "a basic land card",
            "onto the battlefield tapped",
            "",
        ),
        (
            "{T}, Remove a quest counter from this land, Sacrifice this land",
            "a basic land card",
            "onto the battlefield tapped",
            "",
        ),
        (
            "{T}, Sacrifice this land, Sacrifice another land",
            "a basic land card",
            "onto the battlefield tapped",
            "",
        ),
        (
            "{T}, Sacrifice this land, Sacrifice this land",
            "a basic land card",
            "onto the battlefield tapped",
            "",
        ),
        (
            "{T}, Sacrifice another land",
            "a basic land card",
            "onto the battlefield tapped",
            "",
        ),
        (
            "{T}, Sacrifice two lands",
            "a basic land card",
            "onto the battlefield tapped",
            "",
        ),
        (
            "{T}, Sacrifice this land",
            "a basic land card",
            "onto the battlefield tapped",
            " Activate only if you control a creature.",
        ),
        (
            "{T}, Sacrifice this land",
            "a nonbasic land card",
            "onto the battlefield tapped",
            "",
        ),
    ] {
        assert_unsupported_fetch_source(fetch_source(cost, selector, destination, tail));
    }
    for destination in [
        "onto the battlefield under an opponent's control",
        "onto the battlefield tapped under an opponent's control",
        "onto the battlefield under an opponent's control tapped",
        "onto the battlefield under target player's control",
        "onto the battlefield under that player's control",
        "onto the battlefield under your teammate's control",
        "onto the battlefield under the control of an opponent",
        "onto the battlefield under your control until end of turn",
        "onto the battlefield tapped and attacking",
        "onto the battlefield under your control tapped and attacking",
        "onto the battlefield with a +1/+1 counter on it",
        "face down onto the battlefield",
        "on top of an opponent's library",
        "on top of your library face up",
        "into target player's hand",
        "into an opponent's graveyard",
    ] {
        assert_unsupported_search_placement(fetch_source(
            "{2}, {T}, Pay 1 life, Sacrifice this land",
            "a Forest card",
            destination,
            " You gain 2 life.",
        ));
    }
    for movement in [
        "have an opponent put it onto the battlefield",
        "have target player put it onto the battlefield",
        "exile it face down",
        "exile that card with a time counter on it",
    ] {
        let mut source = fetch_source(
            "{2}, {T}, Pay 1 life, Sacrifice this land",
            "a Forest card",
            "onto the battlefield",
            " You gain 2 life.",
        );
        source.oracle_text = source.oracle_text.replace(
            "put it onto the battlefield",
            &format!("reveal it, {movement}"),
        );
        assert_unsupported_search_placement(source);
    }
}

/// Extra source zones cannot grant native fetch metadata or consume activation costs.
#[test]
fn unsupported_source_zones_cannot_become_library_only_fetches() {
    for source_zone in [
        "your library and an opponent's library",
        "your library and an opponent’s library",
        "your library and/or graveyard",
        "your library or your graveyard",
        "your library except the top card",
    ] {
        for verb in ["Search", "You may search"] {
            let mut source = fetch_source(
                "{T}, Pay 1 life, Sacrifice this land",
                "a Forest card",
                "onto the battlefield tapped",
                "",
            );
            source.oracle_text = source.oracle_text.replace(
                "Search your library for",
                &format!("{verb} {source_zone} for"),
            );
            let oracle = parse_oracle_card(&source);
            let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
                panic!("the source keeps its complete activation");
            };
            let [OracleEffect::Unsupported(search)] = ability.effects.as_slice() else {
                panic!("the complete source scope stays in one unsupported node");
            };
            assert_eq!(
                search,
                source
                    .oracle_text
                    .split_once(": ")
                    .expect("activation separator")
                    .1
            );
            assert!(oracle.abilities[0].to_runtime().is_empty());
            assert_unsupported_fetch_source(source);
        }
    }
    for source in [
        "Look at the top five cards of your library and an opponent's library",
        "Look at the top five cards of your library and/or graveyard",
        "Look at the top five cards of your library except the first card",
        "Look at the top six cards of your library",
    ] {
        assert_unsupported_search_placement(row(
            "Unsupported bounded search source",
            "",
            "Land",
            &format!(
                "{{2}}, {{T}}, Pay 1 life, Sacrifice this land: {source}. You may put a basic land card from among them onto the battlefield. Put the rest on the bottom of your library. You gain 2 life."
            ),
        ));
    }
}

/// Filters and destinations absent from OracleFetch stay available only through complete typed effects.
#[test]
fn native_fetch_does_not_broaden_supported_typed_search_shapes() {
    for (selector, destination) in [
        ("a basic land card", "into your hand"),
        ("a basic land card", "on top of your library"),
        ("a land card with mana value 0", "onto the battlefield"),
        ("a green land card", "onto the battlefield"),
        ("up to one basic land card", "onto the battlefield"),
    ] {
        let source = fetch_source("{T}, Sacrifice this land", selector, destination, "");
        assert_no_native_fetch(&source);
    }
    let mut source = fetch_source(
        "{T}, Sacrifice this land",
        "a basic land card",
        "onto the battlefield",
        "",
    );
    source.oracle_text = source.oracle_text.replace(", then shuffle", "");
    assert_no_native_fetch(&source);
}

/// Every modeled search requirement outside native metadata prevents native admission.
#[test]
fn native_fetch_rejects_unrepresented_search_filters_and_activation_limits() {
    let source = fetch_source(
        "{T}, Sacrifice this land",
        "a basic land card",
        "onto the battlefield tapped",
        "",
    );
    let oracle = parse_oracle_card(&source);
    let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
        panic!("one complete fetch activation");
    };
    let [OracleEffect::Search(base)] = ability.effects.as_slice() else {
        panic!("one complete fetch search");
    };
    for spec in [
        SearchSpec {
            color: Some(ManaColor::Green),
            ..*base
        },
        SearchSpec {
            colorless: true,
            ..*base
        },
        SearchSpec {
            mana_value: Some(0),
            ..*base
        },
        SearchSpec {
            max_mana_value: Some(1),
            ..*base
        },
        SearchSpec {
            min_mana_value: Some(1),
            ..*base
        },
        SearchSpec {
            optional: true,
            ..*base
        },
        SearchSpec {
            top_count: Some(4),
            ..*base
        },
        SearchSpec {
            shuffle: false,
            ..*base
        },
        SearchSpec {
            non_human: true,
            ..*base
        },
    ] {
        let mut changed = ability.clone();
        changed.effects = vec![OracleEffect::Search(spec)];
        let land = super::oracle_parser::land::parse_land_data(
            &source.oracle_text,
            &[OracleAbility::Activated(changed)],
        );
        assert!(land.fetch.is_none());
    }
    for restriction in [
        AbilityRestriction::OncePerTurn,
        AbilityRestriction::OncePerGame,
        AbilityRestriction::PowerUp,
        AbilityRestriction::SorcerySpeed,
        AbilityRestriction::Condition("you control a creature".into()),
    ] {
        let mut changed = ability.clone();
        changed.restrictions.push(restriction);
        let land = super::oracle_parser::land::parse_land_data(
            &source.oracle_text,
            &[OracleAbility::Activated(changed)],
        );
        assert!(land.fetch.is_none());
    }
}

/// Native rejection does not discard a supported linked effect from the shared activation route.
#[test]
fn linked_supported_fetch_effects_keep_the_complete_runtime_sequence() {
    let source = fetch_source(
        "{T}, Pay 1 life, Sacrifice this land",
        "a Forest card",
        "onto the battlefield tapped",
        " You gain 2 life.",
    );
    assert_no_native_fetch(&source);
    let cards = deck(&[
        source,
        row("Type miss", "", "Basic Land — Island", ""),
        row("Forest", "", "Basic Land — Forest", ""),
        row("Marker", "{99}", "Artifact", ""),
    ]);
    let mut game = main_phase_state(vec![0], vec![1, 2, 3]);
    assert!(play_land(&cards, &mut game, 1));
    let mut expected_library = vec![CardIdx(1), CardIdx(3)];
    let mut expected_rng = game.library_rng.clone();
    super::rng::shuffle(&mut expected_library, &mut expected_rng);
    let mut pool = ManaPool::default();
    spend_leftover(&cards, &mut game, &mut pool, 1);
    assert_eq!((game.life, game.life_paid, game.life_gained), (21, 1, 2));
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(2)));
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.library, expected_library);
    assert_eq!(pool.total(), 0);
    assert_library_rng(&game.library_rng, &expected_rng);
}

/// The pinned linked untap clause stays visible and blocks partial search execution at either land count.
#[test]
fn pinned_fabled_passage_cannot_bypass_atomic_activation_lowering() {
    for source in renamed(&named_row("Fabled Passage")) {
        assert!(
            source
                .oracle_text
                .contains("Then if you control four or more lands, untap that land.")
        );
        assert_no_native_fetch(&source);
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
            panic!("Fabled Passage has one linked activation");
        };
        let [OracleEffect::Unsupported(resolution)] = ability.effects.as_slice() else {
            panic!("the complete conditional resolution stays in one unsupported node");
        };
        assert_eq!(
            resolution,
            source
                .oracle_text
                .split_once(": ")
                .expect("printed activation separator")
                .1
        );
        assert!(matches!(
            ability.costs.as_slice(),
            [
                ActivationCost::Tap(ActivationTarget::Source),
                ActivationCost::Sacrifice {
                    count: 1,
                    object: CostObject::Source,
                    exclude_source: false
                }
            ]
        ));
        assert!(ability.restrictions.is_empty());
        assert!(!ability.is_mana_ability && !ability.mana_per_spell_cast);
        assert!(oracle.abilities[0].to_runtime().is_empty());
        let [printed] = oracle.ability_sources.as_slice() else {
            panic!("the linked resolution retains one original source statement");
        };
        assert_eq!(printed.text, source.oracle_text);
        let span = printed
            .span
            .expect("the printed activation retains its source span");
        assert_eq!((span.start, span.end), (0, source.oracle_text.len()));
        let cards = deck(&[
            source.clone(),
            row("Unfunded spell", "{G}", "Sorcery", "You gain 2 life."),
            row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
            row("First marker", "{99}", "Artifact", ""),
            row("Second marker", "{99}", "Sorcery", ""),
            row("Other land", "", "Land", ""),
        ]);
        assert!(cards.cards[0].unlocked_abilities(0).next().is_none());
        for other_lands in [0, 3] {
            let mut game = main_phase_state(vec![0, 1], vec![2, 3, 4]);
            for uid in 0..other_lands {
                game.battlefield
                    .push(new_perm_with(uid, &cards, CardIdx(5), 0, false));
            }
            game.next_uid = 20;
            let expected_rng = game.library_rng.clone();
            let census = execute_turn(&cards, &mut game);
            assert_eq!(game.hand, [CardIdx(1)]);
            assert!(game.graveyard.is_empty() && game.exile.is_empty());
            assert_eq!(game.battlefield.len(), other_lands as usize + 1);
            let source_permanent = game
                .battlefield
                .last()
                .expect("the physical fetch stays in play");
            assert_eq!(source_permanent.card, CardRef::Deck(CardIdx(0)));
            assert_eq!(source_permanent.uid, 21);
            assert_eq!(source_permanent.entered_turn, 1);
            assert_eq!(game.next_uid, 21);
            assert!(!source_permanent.tapped);
            assert_eq!((game.life, game.life_paid), (20, 0));
            assert_eq!(
                (game.life_gained, game.seen, game.spells_cast_this_turn),
                (0, 0, 0)
            );
            assert_eq!(game.library, [CardIdx(2), CardIdx(3), CardIdx(4)]);
            assert_library_rng(&game.library_rng, &expected_rng);
            assert_blocked_activation(&cards, &mut game, &mut ManaPool::default());
            let (log, report) = report_executed_turn(&cards, game, census);
            assert_eq!(log.land_drops, [1]);
            assert_eq!(log.mana_spent, [0.0]);
            assert_eq!(log.life_paid, 0);
            assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 0.0);
            assert_eq!(report["draw"]["avg_life_paid"], 0.0);
            let record = report["card_support"]
                .as_array()
                .expect("compiler records")
                .iter()
                .find(|card| card["name"] == source.name)
                .expect("fetch compiler record");
            assert_eq!(record["abilities"][0]["source"], source.oracle_text);
            assert_eq!(record["abilities"][0]["status"], "unsupported");
            assert!(
                !record["abilities"][0]["reason"]
                    .as_str()
                    .expect("linked requirement gap")
                    .is_empty()
            );
            assert!(report["assumptions"].as_array().expect("model limits").iter().any(|limit|
                limit.as_str().is_some_and(|text| text.contains("Unsupported required costs, conditions, targets, and effects can leave an ability inert"))
            ));
        }
    }
}

/// Standard typed fetches keep their full costs, tap state, exact targets, and explicit shuffle.
#[test]
fn standard_native_fetch_controls_preserve_matching_and_nonmatching_lands() {
    for (selector, target_type, forced_tapped, life_cost, destination) in [
        (
            "a Mountain or Plains card",
            "Land — Plains Mountain",
            false,
            1,
            "onto the battlefield",
        ),
        (
            "a basic land card",
            "Basic Land",
            true,
            0,
            "onto the battlefield tapped",
        ),
        (
            "a Mountain or Plains card",
            "Land — Plains Mountain",
            false,
            1,
            "onto the battlefield under your control",
        ),
        (
            "a basic land card",
            "Basic Land",
            true,
            0,
            "onto the battlefield tapped under your control",
        ),
        (
            "a basic land card",
            "Basic Land",
            true,
            0,
            "onto the battlefield under your control tapped",
        ),
    ] {
        let source = fetch_source(
            if life_cost == 0 {
                "{T}, Sacrifice this land"
            } else {
                "{T}, Pay 1 life, Sacrifice this land"
            },
            selector,
            destination,
            "",
        );
        for source in renamed(&source) {
            let oracle = parse_oracle_card(&source);
            let fetch = oracle
                .land
                .fetch
                .as_ref()
                .expect("complete native fetch subset");
            assert_eq!(fetch.life_cost, life_cost);
            assert_eq!(fetch.basic_only, forced_tapped);
            assert_eq!(
                fetch.target_types,
                if forced_tapped {
                    vec![]
                } else {
                    vec![BasicLandType::Plains, BasicLandType::Mountain]
                }
            );
            assert_eq!(oracle.land.fetch_enters_tapped, forced_tapped);
            let cards = deck(&[
                source,
                row("Type miss", "", "Land — Island", ""),
                row("Marker", "{99}", "Artifact", ""),
                row("Matching target", "", target_type, ""),
                row("Second marker", "{99}", "Sorcery", ""),
            ]);
            assert!(cards.cards[0].is_fetch_land);
            assert!(matches!(
                cards.cards[0].support[0].status,
                AbilitySupportStatus::Approximated {
                    rule_id: "bounded_search_selection"
                }
            ));
            let mut game = main_phase_state(vec![0], vec![1, 2, 3, 4]);
            assert!(play_land(&cards, &mut game, 1));
            let source_uid = game.battlefield[0].uid;
            let mut expected_library = vec![CardIdx(1), CardIdx(2), CardIdx(4)];
            let mut expected_rng = game.library_rng.clone();
            super::rng::shuffle(&mut expected_library, &mut expected_rng);
            let mut pool = ManaPool {
                colorless: 2,
                ..ManaPool::default()
            };
            spend_leftover(&cards, &mut game, &mut pool, 1);
            assert_eq!(pool.colorless, 2);
            assert_eq!(
                (game.life, game.life_paid),
                (20 - life_cost as i32, life_cost)
            );
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(game.battlefield.len(), 1);
            assert_ne!(game.battlefield[0].uid, source_uid);
            assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(3)));
            assert_eq!(game.battlefield[0].tapped, forced_tapped);
            assert_eq!(game.library, expected_library);
            assert_library_rng(&game.library_rng, &expected_rng);
        }
    }
}

/// A legal committed miss pays life, sacrifices its source, and shuffles; it is not a pilot decline.
#[test]
fn committed_failed_fetch_still_pays_costs_and_shuffles() {
    let source = fetch_source(
        "{T}, Pay 1 life, Sacrifice this land",
        "a Mountain or Plains card",
        "onto the battlefield",
        "",
    );
    let cards = deck(&[
        source,
        row("Island", "", "Basic Land — Island", ""),
        row("Wastes", "", "Basic Land", ""),
        row("Marker", "{99}", "Artifact", ""),
    ]);
    let mut game = main_phase_state(vec![0], vec![1, 2, 3]);
    assert!(play_land(&cards, &mut game, 1));
    let mut expected_library = game.library.clone();
    let mut expected_rng = game.library_rng.clone();
    super::rng::shuffle(&mut expected_library, &mut expected_rng);
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &game, &pool, 1)
        .expect("the activation is legal without knowing a matching library card");
    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
    assert!(game.battlefield.is_empty() && game.hand.is_empty() && game.exile.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!((game.life, game.life_paid), (19, 1));
    assert_eq!(pool.total(), 0);
    assert_eq!(game.library, expected_library);
    assert_library_rng(&game.library_rng, &expected_rng);
}

/// Both destination tap states reach the real turn loop without granting extra cast mana.
#[test]
fn supported_fetch_destination_controls_full_turn_cast_and_report() {
    for forced_tapped in [false, true] {
        let source = fetch_source(
            "{T}, Pay 1 life, Sacrifice this land",
            "a Forest card",
            if forced_tapped {
                "onto the battlefield tapped"
            } else {
                "onto the battlefield"
            },
            "",
        );
        let cards = deck(&[
            source,
            row("Funded spell", "{G}", "Sorcery", "You gain 2 life."),
            row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
            row("Type miss", "", "Land — Island", ""),
            row("Marker", "{99}", "Artifact", ""),
        ]);
        let mut game = main_phase_state(vec![0, 1], vec![2, 3, 4]);
        let mut expected_library = vec![CardIdx(3), CardIdx(4)];
        let mut expected_rng = game.library_rng.clone();
        super::rng::shuffle(&mut expected_library, &mut expected_rng);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.hand,
            if forced_tapped {
                vec![CardIdx(1)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.graveyard,
            if forced_tapped {
                vec![CardIdx(0)]
            } else {
                vec![CardIdx(0), CardIdx(1)]
            }
        );
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(2)));
        assert!(game.battlefield[0].tapped);
        assert_eq!(game.life, if forced_tapped { 19 } else { 21 });
        assert_eq!(game.life_paid, 1);
        assert_eq!(game.library, expected_library);
        assert_library_rng(&game.library_rng, &expected_rng);
        let (log, report) = report_executed_turn(&cards, game, census);
        let spent = f64::from(u32::from(!forced_tapped));
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.mana_spent, [spent]);
        assert_eq!(log.life_paid, 1);
        assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], spent);
        assert_eq!(report["draw"]["avg_life_paid"], 1.0);
    }
}

/// Native destinations cannot silently map hand, top, exile, or graveyard placement to the battlefield.
#[test]
fn typed_fetch_destinations_remain_explicit_before_native_admission() {
    for destination in [
        SearchDestination::Hand,
        SearchDestination::LibraryTop,
        SearchDestination::Exile,
        SearchDestination::Graveyard,
    ] {
        let source = fetch_source(
            "{T}, Sacrifice this land",
            "a basic land card",
            "onto the battlefield",
            "",
        );
        let mut oracle = parse_oracle_card(&source);
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_mut_slice() else {
            panic!("one fetch activation");
        };
        let [OracleEffect::Search(spec)] = ability.effects.as_mut_slice() else {
            panic!("one complete search");
        };
        spec.destination = destination;
        assert!(
            super::oracle_parser::land::parse_land_data(&source.oracle_text, &oracle.abilities)
                .fetch
                .is_none()
        );
    }
}

/// Build a bounded selection separately from variants with mandatory unsupported remainder rules.
pub(super) fn bounded_search_source(remainder: &str) -> CardRow {
    let reward = if remainder.is_empty() {
        ""
    } else {
        " You gain 2 life."
    };
    row(
        "Bounded selection source",
        "{1}{G}",
        "Artifact",
        &format!(
            "{{1}}, {{T}}: Look at the top three cards of your library. You may put an artifact card from among them into your hand.{remainder}{reward}"
        ),
    )
}

/// Seed real payment sources and a six-card library with a target below the top-card bound.
pub(super) fn bounded_search_scenario(source: &CardRow, matching: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row("Below-bound artifact", "{99}", "Artifact", ""),
        row("Green payment", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Colorless payment", "", "Land", "{T}: Add {C}{C}."),
        row("Lower marker one", "{99}", "Sorcery", ""),
        row("Lower marker two", "{99}", "Sorcery", ""),
        row("Top-bound artifact", "{99}", "Artifact", ""),
        row("Top marker one", "{99}", "Sorcery", ""),
        row("Top marker two", "{99}", "Sorcery", ""),
        row("No-match marker", "{99}", "Sorcery", ""),
    ]);
    let mut game = main_phase_state(vec![0], vec![1, 4, 5, if matching { 6 } else { 9 }, 7, 8]);
    for (uid, index) in [(10, 2), (11, 3)] {
        game.battlefield
            .push(new_perm_with(uid, &cards, CardIdx(index), 0, true));
    }
    game.next_uid = 20;
    (cards, game)
}

/// Distinguish exposed cards from a found target without moving any unchosen library cards.
pub(super) fn assert_bounded_search_state(game: &GameState, supported: bool, matching: bool) {
    let mut expected = vec![
        CardIdx(1),
        CardIdx(4),
        CardIdx(5),
        CardIdx(if matching { 6 } else { 9 }),
        CardIdx(7),
        CardIdx(8),
    ];
    if supported && matching {
        expected.retain(|index| *index != CardIdx(6));
    }
    assert_eq!(game.library, expected);
    assert_eq!(
        game.hand,
        if supported && matching {
            vec![CardIdx(6)]
        } else {
            vec![]
        }
    );
    assert!(game.graveyard.is_empty() && game.exile.is_empty());
    assert_eq!(game.battlefield.len(), 3);
    assert_eq!(game.battlefield[2].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(game.battlefield[2].uid, 21);
    assert_eq!(game.battlefield[2].tapped, supported);
    assert_eq!((game.life, game.life_paid, game.life_gained), (20, 0, 0));
    assert_eq!(
        (game.seen, game.awareness_cards),
        if supported { (3, 3) } else { (0, 0) }
    );
    assert_eq!((game.spells_cast_this_turn, game.next_uid), (1, 21));
}

/// Run the same bounded requirement through a full paid turn and the public report pipeline.
pub(super) fn assert_bounded_search_report(source: &CardRow, supported: bool, matching: bool) {
    let (cards, mut game) = bounded_search_scenario(source, matching);
    let expected_rng = game.library_rng.clone();
    let census = execute_turn(&cards, &mut game);
    assert_bounded_search_state(&game, supported, matching);
    assert_library_rng(&game.library_rng, &expected_rng);
    let (log, report) = report_executed_turn(&cards, game, census);
    let seen = if supported { 3 } else { 0 };
    assert_eq!(log.cards_seen, [seen]);
    assert_eq!(log.mana_spent, [2.0]);
    assert_eq!(log.mana_remaining, [f64::from(u32::from(!supported))]);
    assert_eq!(report["velocity"]["avg_cards_seen"]["1"], f64::from(seen));
    assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 2.0);
    let records = report["card_support"].as_array().expect("compiler records");
    let source_record = records
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("bounded source record");
    let record = &source_record["abilities"][0];
    assert_eq!(record["source"], source.oracle_text);
    assert_eq!(
        record["status"],
        if supported {
            "approximated"
        } else {
            "unsupported"
        }
    );
    if supported {
        assert_eq!(record["rule_id"], "bounded_search_selection");
    }
}
