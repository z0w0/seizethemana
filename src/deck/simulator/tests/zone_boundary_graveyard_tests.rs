//! Exact graveyard selection, unsupported rewards, and bounded own replay actions.

use crate::db::CardRow;
use crate::deck::simulator::card_test_support::{
    assert_library_rng, main_phase_state, named_row, pinned_revision,
};
use crate::deck::simulator::game::{
    CardRef, GameCompletion, GameState, IncompleteReason, ManaPool,
};
use crate::deck::simulator::game_effects::{CardZone, move_to_graveyard};
use crate::deck::simulator::library_core_report_tests::{
    assert_report_clause, cast_action, executed_report, put_permanents,
};
use crate::deck::simulator::model::{CardIdx, MAX_LOOP_PASSES, SimDeck};
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use serde_json::Value;

/// Preserve physical graveyard order and real generation records in controlled setup.
fn bury(cards: &SimDeck, game: &mut GameState, indexes: &[u32]) {
    for &index in indexes {
        move_to_graveyard(cards, game, CardIdx(index), 1, CardZone::Hand);
    }
}

/// Require the exact synthetic unsupported source and a concrete explanation.
fn assert_unsupported(report: &Value, source: &CardRow) {
    let card = report["card_support"]
        .as_array()
        .expect("card support")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("source report");
    assert_eq!(
        card["source_hash"],
        crate::deck::simulator::support::card_source_hash(source)
    );
    let ability = &card["abilities"][0];
    assert_eq!(ability["source"], source.oracle_text);
    assert_eq!(ability["status"], "unsupported");
    assert!(
        ability["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert!(ability["rule_id"].is_null());
}

/// Complete spell and activation attempts cannot split unsupported return requirements.
pub(super) fn assert_filtered_return_safety_report() {
    for instruction in [
        "Return target artifact card from your graveyard to your hand",
        "Return target creature card with mana value 3 or less from your graveyard to the battlefield",
        "Return two creature cards from your graveyard to the battlefield",
        "Return up to one target creature card from your graveyard to the battlefield",
        "Return target creature card from your graveyard to the battlefield tapped",
    ] {
        for activation in [false, true] {
            let source = row(
                "Synthetic unsupported return",
                if activation { "{99}" } else { "{1}" },
                if activation { "Creature" } else { "Sorcery" },
                &if activation {
                    format!("Sacrifice a creature: {instruction}. Gain 3 life.")
                } else {
                    format!("{instruction}. Gain 3 life.")
                },
            );
            let cards = deck(&[
                source.clone(),
                row("First return creature", "{99}", "Creature", ""),
                row("Second return creature", "{99}", "Creature", ""),
                row("Return artifact", "{99}", "Artifact", ""),
                row("Return retained library", "{99}", "Sorcery", ""),
                row("Return funding", "", "Basic Land - Wastes", "{T}: Add {C}."),
            ]);
            let mut game = state(if activation { vec![] } else { vec![0] }, vec![4]);
            bury(&cards, &mut game, &[1, 2, 3]);
            put_permanents(&cards, &mut game, &[5]);
            if activation {
                put_permanents(&cards, &mut game, &[0]);
            }
            let rng = game.library_rng.clone();
            let (game, log, report) = executed_report(&cards, game, 1, vec![]);
            assert_eq!(
                game.graveyard,
                if activation {
                    vec![CardIdx(1), CardIdx(2), CardIdx(3)]
                } else {
                    vec![CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(0)]
                }
            );
            assert_eq!(game.library, [CardIdx(4)]);
            assert!(game.hand.is_empty());
            assert!(game.exile.is_empty());
            assert_eq!(game.life, 20);
            assert_eq!(game.life_gained, 0);
            for index in 1..=3 {
                assert_eq!(game.graveyard_generations[&CardIdx(index)], 1);
            }
            assert_eq!(log.mana_spent, [if activation { 0.0 } else { 1.0 }]);
            assert_eq!(report["draw"]["avg_life_gained"], 0.0);
            assert_eq!(
                report["graveyard"]["avg_size_by_turn"]["1"],
                if activation { 3.0 } else { 4.0 }
            );
            assert_unsupported(&report, &source);
            assert_library_rng(&game.library_rng, &rng);
        }
    }
    assert_pinned_filtered_return();
    assert_supported_return_neighbor();
}

/// A pinned filtered return pays its printed cost without inventing an eligible reward.
fn assert_pinned_filtered_return() {
    let source = named_row("Sevinne's Reclamation");
    pinned_revision(&source);
    let mut rows = vec![
        source.clone(),
        row("Filtered permanent", "{1}", "Artifact", ""),
        row("Filtered retained", "{99}", "Sorcery", ""),
    ];
    rows.extend((0..3).map(|index| {
        row(
            &format!("Filtered funding {index}"),
            "",
            "Basic Land - Plains",
            "{T}: Add {W}.",
        )
    }));
    let cards = deck(&rows);
    let mut game = state(vec![0], vec![2]);
    bury(&cards, &mut game, &[1]);
    put_permanents(&cards, &mut game, &[3, 4, 5]);
    let rng = game.library_rng.clone();
    let (game, log, report) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert!(game.hand.is_empty());
    assert!(
        !game
            .battlefield
            .iter()
            .any(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
    );
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(report["graveyard"]["replay_casts_avg"], 0.0);
    assert_report_clause(
        &report,
        &source,
        "Return target permanent card with mana value 3 or less",
        "unsupported",
        None,
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Removing the unrepresented filter allows the paid targeted return and linked gain.
fn assert_supported_return_neighbor() {
    let cards = deck(&[
        row(
            "Synthetic supported return",
            "{1}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield. Gain 3 life.",
        ),
        row("Supported returned creature", "{99}", "Creature", ""),
        row(
            "Supported return funding",
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        ),
    ]);
    let mut game = state(vec![0], vec![]);
    bury(&cards, &mut game, &[1]);
    put_permanents(&cards, &mut game, &[2]);
    let (game, log, report) = executed_report(&cards, game, 1, vec![]);
    assert!(
        game.battlefield
            .iter()
            .any(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
    );
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.life, 23);
    assert_eq!(log.mana_spent, [1.0]);
    assert_eq!(report["draw"]["avg_life_gained"], 3.0);
}

/// A paid pinned Witness selects the newest eligible graveyard entry among several candidates.
pub(super) fn assert_first_eligible_return_report() {
    let source = named_row("Eternal Witness");
    pinned_revision(&source);
    let mut rows = vec![
        source.clone(),
        row("First eligible artifact", "{99}", "Artifact", ""),
        row("First eligible creature", "{99}", "Creature", ""),
        row("First eligible spell", "{99}", "Sorcery", ""),
        row("Excluded exile", "{99}", "Artifact", ""),
    ];
    rows.extend((0..3).map(|index| {
        row(
            &format!("Selection green {index}"),
            "",
            "Basic Land - Forest",
            "{T}: Add {G}.",
        )
    }));
    let cards = deck(&rows);
    for order in [[1, 2, 3], [3, 1, 2], [2, 3, 1]] {
        for funded in [false, true] {
            let mut game = state(vec![0], vec![]);
            bury(&cards, &mut game, &order);
            game.exile.push(CardIdx(4));
            put_permanents(&cards, &mut game, if funded { &[5, 6, 7] } else { &[5, 6] });
            let rng = game.library_rng.clone();
            let (game, log, report) = executed_report(&cards, game, 1, vec![]);
            assert_eq!(game.hand, [CardIdx(if funded { order[2] } else { 0 })]);
            assert_eq!(
                game.graveyard,
                order[..order.len() - usize::from(funded)]
                    .iter()
                    .copied()
                    .map(CardIdx)
                    .collect::<Vec<_>>()
            );
            assert_eq!(game.exile, [CardIdx(4)]);
            for &index in &order {
                assert_eq!(
                    log.card_first_seen.get(&(index as usize)),
                    (funded && index == order[2]).then_some(&1)
                );
                assert_eq!(game.graveyard_generations[&CardIdx(index)], 1);
            }
            assert_eq!(log.mana_spent, [if funded { 3.0 } else { 0.0 }]);
            assert_eq!(
                report["graveyard"]["avg_size_by_turn"]["1"],
                if funded { 2.0 } else { 3.0 }
            );
            assert_eq!(report["graveyard"]["replay_casts_avg"], 0.0);
            assert_eq!(game.life, 20);
            assert_report_clause(
                &report,
                &source,
                "return target card from your graveyard to your hand",
                "approximated",
                Some("pilot_card_selection"),
            );
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// Own Escape consumes its discard and three other cards, then returns the spell to the graveyard.
pub(super) fn assert_own_escape_resources_report() {
    let cards = deck(&[
        row(
            "Synthetic own Escape",
            "{9}",
            "Sorcery",
            "As an additional cost to cast this spell, discard a card. Gain 1 life.\nEscape—{R}, Exile three other cards from your graveyard.",
        ),
        row("Escape first fodder", "{99}", "Artifact", ""),
        row("Escape second fodder", "{99}", "Artifact", ""),
        row("Escape third fodder", "{99}", "Artifact", ""),
        row("Escape retained graveyard", "{99}", "Artifact", ""),
        row("Escape discard", "{99}", "Sorcery", ""),
        row("Escape library", "{99}", "Sorcery", ""),
        row(
            "Escape funding",
            "",
            "Basic Land - Mountain",
            "{T}: Add {R}.",
        ),
        row(
            "Escape wrong funding",
            "",
            "Basic Land - Island",
            "{T}: Add {U}.",
        ),
    ]);
    assert!(cards.cards[0].spell_data.own_escape.is_some());
    assert_eq!(cards.cards[0].spell_data.additional_cost_discards, 1);
    for (fodder, discard, red) in [
        (3, true, true),
        (2, true, true),
        (3, false, true),
        (3, true, false),
    ] {
        let mut game = state(if discard { vec![5] } else { vec![] }, vec![6]);
        bury(
            &cards,
            &mut game,
            &(1..=fodder).chain([0]).collect::<Vec<_>>(),
        );
        put_permanents(&cards, &mut game, &[if red { 7 } else { 8 }]);
        let rng = game.library_rng.clone();
        let (game, log, report) = executed_report(&cards, game, 1, vec![]);
        let paid = fodder == 3 && discard && red;
        assert_eq!(
            game.exile,
            if paid {
                vec![CardIdx(1), CardIdx(2), CardIdx(3)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.graveyard,
            if paid {
                vec![CardIdx(5), CardIdx(0)]
            } else {
                (1..=fodder).chain([0]).map(CardIdx).collect()
            }
        );
        assert_eq!(
            game.hand,
            if discard && !paid {
                vec![CardIdx(5)]
            } else {
                vec![]
            }
        );
        assert_eq!(game.library, [CardIdx(6)]);
        assert_eq!(
            game.graveyard_generations[&CardIdx(0)],
            if paid { 2 } else { 1 }
        );
        assert_eq!(game.life, if paid { 21 } else { 20 });
        assert_eq!(game.life_paid, 0);
        assert_eq!(log.mana_spent, [if paid { 1.0 } else { 0.0 }]);
        assert_eq!(log.replay_casts, u32::from(paid));
        assert_eq!(
            report["graveyard"]["replay_casts_avg"],
            f64::from(u32::from(paid))
        );
        assert_eq!(
            report["milestones"]["graveyard_casts_avg_by_turn"]["1"],
            f64::from(u32::from(paid))
        );
        assert_library_rng(&game.library_rng, &rng);
    }
    assert_spent_escape_fodder_cannot_replay(&cards);
}

/// Remaining red mana cannot replace the three consumed cards or the additional discard.
fn assert_spent_escape_fodder_cannot_replay(cards: &SimDeck) {
    let mut game = main_phase_state(vec![5], vec![6]);
    bury(cards, &mut game, &[1, 2, 3, 0]);
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 2, 0],
        ..Default::default()
    };
    assert_eq!(cast_action(cards, &mut game, &mut pool), 1.0);
    assert_eq!(pool.fixed[3], 1);
    assert_eq!(game.replay_casts, 1);
    assert_eq!(game.graveyard, [CardIdx(5), CardIdx(0)]);
    assert_eq!(cast_action(cards, &mut game, &mut pool), 0.0);
    assert_eq!(pool.fixed[3], 1);
    assert_eq!(game.replay_casts, 1);
}

/// Exact replay limits leave no imaginary end turns or completed-game means after a cap.
pub(super) fn assert_replay_cap_report() {
    for escape in [false, true] {
        for extra in [0, 1] {
            let count = MAX_LOOP_PASSES + extra;
            let mut rows: Vec<_> = (0..count).map(|index| row(&format!("Synthetic replay {index}"), "{99}", if escape { "Creature" } else { "Sorcery" }, if escape { "When this creature enters, gain 1 life.\nEscape—{1}, Exile three other cards from your graveyard." } else { "Gain 1 life.\nFlashback {1}" })).collect();
            let fodder_start = rows.len() as u32;
            if escape {
                rows.extend(
                    (0..count * 3)
                        .map(|index| row(&format!("Cap fodder {index}"), "{99}", "Artifact", "")),
                );
            }
            let funding_start = rows.len() as u32;
            rows.extend((0..count).map(|index| {
                row(
                    &format!("Cap funding {index}"),
                    "",
                    "Basic Land - Wastes",
                    "{T}: Add {C}.",
                )
            }));
            let future = rows.len() as u32;
            rows.push(row("Cap untouched library", "{99}", "Sorcery", ""));
            let cards = deck(&rows);
            let mut game = state(vec![], vec![future as usize]);
            bury(
                &cards,
                &mut game,
                &(fodder_start..funding_start)
                    .chain(0..count)
                    .collect::<Vec<_>>(),
            );
            put_permanents(
                &cards,
                &mut game,
                &(funding_start..future).collect::<Vec<_>>(),
            );
            let rng = game.library_rng.clone();
            let (game, log) = execute_turns(&cards, game, 1, vec![]);
            let report = report_log(&cards, &log, 1);
            assert_eq!(game.replay_casts, MAX_LOOP_PASSES);
            assert_eq!(game.life_gained, MAX_LOOP_PASSES);
            assert_eq!(game.life, 20 + MAX_LOOP_PASSES as i32);
            assert_eq!(game.library, [CardIdx(future)]);
            let mut expected_graveyard = if escape {
                (fodder_start + MAX_LOOP_PASSES * 3..funding_start)
                    .map(CardIdx)
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            expected_graveyard.extend((MAX_LOOP_PASSES..count).map(CardIdx));
            assert_eq!(game.graveyard, expected_graveyard);
            assert_eq!(
                game.exile,
                if escape {
                    (fodder_start..fodder_start + MAX_LOOP_PASSES * 3)
                        .map(CardIdx)
                        .collect::<Vec<_>>()
                } else {
                    (0..MAX_LOOP_PASSES).map(CardIdx).collect::<Vec<_>>()
                }
            );
            assert_eq!(report.outcomes.finished_games, u32::from(extra == 0));
            assert_eq!(report.outcomes.incomplete_games, extra);
            if extra == 0 {
                assert!(game.completion.is_none());
                assert_eq!(log.turns_observed, 1);
                assert_eq!(log.mana_spent, [f64::from(MAX_LOOP_PASSES)]);
                assert_eq!(
                    report.graveyard.replay_casts_avg,
                    Some(f64::from(MAX_LOOP_PASSES))
                );
            } else {
                assert!(matches!(
                    game.completion,
                    Some(GameCompletion::Incomplete {
                        reason: IncompleteReason::GraveyardCastPass {
                            configured_limit: MAX_LOOP_PASSES,
                            remaining_work: 1
                        },
                        ..
                    })
                ));
                assert_eq!(log.turns_observed, 0);
                assert!(report.graveyard.replay_casts_avg.is_none());
                assert!(report.milestones.graveyard_casts_avg_by_turn.is_empty());
                assert!(report.mana.avg_cast_spent_by_turn.is_empty());
                assert_eq!(
                    report.outcomes.incomplete_reasons["graveyard_cast_pass_limit"],
                    1
                );
            }
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}
