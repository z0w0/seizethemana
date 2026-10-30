//! Saga duration follows parsed chapters without guessing an unsupported chapter count.

use super::card_runtime_tests::attempt_index;
use super::card_test_support::{assert_library_rng, main_phase_state};
use super::game::{
    CardRef, GameCompletion, GamePhase, IncompleteReason, ManaPool, PlayerLossReason, TokenKind,
    new_perm_with,
};
use super::game_commander::CommanderProfile;
use super::game_run::{TurnCensus, census::finish_log, run_turn};
use super::library_core_report_tests::executed_report;
use super::model::{CardIdx, SimEffect, SimTrigger};
use super::permanent_core_report_tests::{execute_turns, report_log};
use super::scenario_test_support::{deck, row, state};

/// One- and two-chapter structures retain their written lengths.
#[test]
fn saga_chapter_count_has_no_three_chapter_minimum() {
    for (text, expected) in [
        ("I — Gain 1 life.", 1),
        ("I — Gain 1 life.\nII — Gain 1 life.", 2),
        ("No supported chapter wording.", 0),
    ] {
        let cards = deck(&[row("Synthetic Saga", "{0}", "Enchantment — Saga", text)]);
        assert_eq!(cards[CardIdx(0)].chapter_count(), expected);
    }
}

/// An entry chapter ends the same physical Saga before the action returns or cleanup begins.
pub(super) fn assert_one_chapter_saga_lifecycle() {
    let cards = deck(&[
        row(
            "Synthetic one-chapter Saga",
            "{0}",
            "Enchantment — Saga",
            "I — Gain 1 life.",
        ),
        row("Unrelated permanent", "{99}", "Artifact", ""),
        row("Safe next draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = main_phase_state(vec![0], vec![2]);
    game.battlefield
        .push(new_perm_with(40, &cards, CardIdx(1), 0, false));
    game.next_uid = 40;
    let random = game.library_rng.clone();
    let mut pool = ManaPool::default();
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        Some(0)
    );
    assert_eq!(game.next_uid, 41);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].uid, 40);
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.graveyard_generations.get(&CardIdx(0)), Some(&1));
    assert_eq!(game.graveyard_seen.get(&CardIdx(0)), Some(&1));
    assert_eq!(game.battlefield_seen.get(&CardIdx(0)), Some(&1));
    assert_eq!((game.life, game.life_gained), (21, 1));
    assert!(game.hand.is_empty());
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(pool.total(), 0);
    assert_library_rng(&game.library_rng, &random);

    let mut game = state(vec![0], vec![2]);
    game.battlefield
        .push(new_perm_with(40, &cards, CardIdx(1), 0, false));
    game.next_uid = 40;
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].uid, 40);
    assert_eq!(log.graveyard_size, [1]);
    assert_eq!(log.card_first_battlefield.get(&0), Some(&1));
    assert_eq!(log.card_first_graveyard.get(&0), Some(&1));
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(log.life_gained, 1);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 1.0);
    assert_eq!(json["draw"]["avg_life_gained"], 1.0);
}

/// Run the callable turn-one Saga action and report regression.
#[test]
fn one_chapter_saga_sacrifices_on_its_entry_turn() {
    assert_one_chapter_saga_lifecycle();
}

/// Linked entry effects can restore life before the completed Saga is sacrificed.
#[test]
fn entry_final_chapter_finishes_its_linked_life_effects_before_cleanup() {
    let cards = deck(&[
        row(
            "Synthetic linked entry Saga",
            "{0}",
            "Enchantment — Saga",
            "I — Each player loses 1 life. Gain 3 life.",
        ),
        row("Safe next draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![1]);
    game.life = 1;
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!((game.life, game.life_gained), (3, 3));
    assert!(game.battlefield.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(log.card_first_graveyard.get(&0), Some(&1));
    assert_eq!(json["outcomes"]["player_loss_games"], 0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 1.0);
}

/// Final sacrifice uses the dying source's trigger and prevents its later end-step reward.
#[test]
fn entry_final_chapter_sacrifice_fires_death_but_not_end_step_rewards() {
    let cards = deck(&[
        row(
            "Synthetic creature Saga",
            "{0}",
            "Enchantment Creature — Saga",
            "I — Gain 1 life.\nWhen this creature dies, gain 2 life.\nAt the beginning of your end step, gain 100 life.",
        ),
        row("Safe next draw", "{99}", "Sorcery", ""),
    ]);
    assert!(
        cards[CardIdx(0)]
            .unlocked_abilities(0)
            .any(|ability| ability.trigger == SimTrigger::Dies)
    );
    assert!(
        cards[CardIdx(0)]
            .unlocked_abilities(0)
            .any(|ability| ability.trigger == SimTrigger::EndStep)
    );
    let (game, log, json) = executed_report(&cards, state(vec![0], vec![1]), 1, vec![]);
    assert_eq!((game.life, game.life_gained), (23, 3));
    assert!(game.battlefield.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(1)]);
    assert_eq!(log.creatures, [0]);
    assert_eq!(log.graveyard_size, [1]);
    assert_eq!(log.card_first_graveyard.get(&0), Some(&1));
    assert_eq!(json["draw"]["avg_life_gained"], 3.0);
}

/// A linked final chapter finishes before loss checks and the two-chapter Saga then leaves.
#[test]
fn two_chapter_saga_finishes_and_sacrifices_after_its_last_chapter() {
    let cards = deck(&[
        row(
            "Synthetic two-chapter Saga",
            "{0}",
            "Enchantment — Saga",
            "I — Exile target permanent.\nII — Each player loses 1 life. Gain 3 life.",
        ),
        row("Safe next draw", "{99}", "Sorcery", ""),
    ]);
    assert_eq!(cards[CardIdx(0)].chapter_count(), 2);
    let mut game = state(vec![0], vec![1]);
    game.life = 1;
    let (game, log, json) = executed_report(&cards, game, 2, vec![]);
    assert!(game.completion.is_none());
    assert!(game.battlefield.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.life, 3);
    assert_eq!(game.life_gained, 3);
    assert_eq!(log.graveyard_size, [0, 1]);
    assert_eq!(log.card_first_graveyard.get(&0), Some(&2));
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
}

/// Ordinary later chapters use the same sacrifice and dying-source event path.
#[test]
fn later_final_chapter_sacrifice_keeps_death_and_access_observations() {
    let cards = deck(&[
        row(
            "Synthetic two-chapter creature Saga",
            "{0}",
            "Enchantment Creature — Saga",
            "I — Gain 1 life.\nII — Gain 1 life.\nWhen this creature dies, gain 2 life.",
        ),
        row("Safe next draw", "{99}", "Sorcery", ""),
    ]);
    let (game, log, json) = executed_report(&cards, state(vec![0], vec![1]), 2, vec![]);
    assert_eq!((game.life, game.life_gained), (24, 4));
    assert!(game.battlefield.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.graveyard_generations.get(&CardIdx(0)), Some(&1));
    assert_eq!(log.creatures, [1, 0]);
    assert_eq!(log.graveyard_size, [0, 1]);
    assert_eq!(log.card_first_graveyard.get(&0), Some(&2));
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 0.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["2"], 1.0);
    assert_eq!(json["draw"]["avg_life_gained"], 4.0);
}

/// A final chapter with no legal target grants no linked reward but still completes its Saga.
pub(super) fn assert_saga_missing_target_rejection() {
    for turns in [1, 2] {
        let text = if turns == 1 {
            "I — Return target card from your graveyard to your hand. Gain 7 life."
        } else {
            "I — Gain 1 life.\nII — Return target card from your graveyard to your hand. Gain 7 life."
        };
        let cards = deck(&[
            row("Synthetic targeted Saga", "{0}", "Enchantment — Saga", text),
            row("Safe next draw", "{99}", "Sorcery", ""),
            row("Existing graveyard target", "{99}", "Artifact", ""),
        ]);
        assert!(matches!(
            cards[CardIdx(0)].saga.chapters[turns - 1].as_slice(),
            [
                SimEffect::ReturnFromGraveyard {
                    to_hand: true,
                    count: 1
                },
                SimEffect::GainLife(7),
            ]
        ));
        for has_target in [true, false] {
            let mut game = state(vec![0], vec![1]);
            if has_target {
                game.graveyard.push(CardIdx(2));
            }
            let (game, log, json) = executed_report(&cards, game, turns, vec![]);
            let gained = u32::from(turns == 2) + 7 * u32::from(has_target);
            assert_eq!((game.life, game.life_gained), (20 + gained as i32, gained));
            assert_eq!(game.hand.contains(&CardIdx(2)), has_target);
            assert!(game.battlefield.is_empty());
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(log.life_gained, gained);
            assert_eq!(log.card_first_graveyard.get(&0), Some(&(turns as u32)));
            assert_eq!(json["draw"]["avg_life_gained"], f64::from(gained));
            assert_eq!(
                json["graveyard"]["avg_size_by_turn"][turns.to_string()],
                1.0
            );
        }
    }
}

/// Run supported targeted chapters with and without a legal target.
#[test]
fn final_chapters_without_targets_do_not_gain_free_linked_life() {
    assert_saga_missing_target_rejection();
}

/// Filtered graveyard-return wording stays inert even when a matching card exists.
#[test]
fn unsupported_filtered_saga_return_stays_inert() {
    let cards = deck(&[
        row(
            "Unsupported filtered Saga",
            "{0}",
            "Enchantment — Saga",
            "I — Return target creature card from your graveyard to your hand. Gain 7 life.",
        ),
        row("Safe next draw", "{99}", "Sorcery", ""),
        row("Existing creature card", "{99}", "Creature", ""),
    ]);
    assert!(cards[CardIdx(0)].saga.chapters[0].is_empty());
    let mut game = state(vec![0], vec![1]);
    game.graveyard.push(CardIdx(2));
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!((game.life, game.life_gained), (20, 0));
    assert!(game.hand.is_empty());
    assert!(game.battlefield.is_empty());
    assert_eq!(game.graveyard, [CardIdx(2), CardIdx(0)]);
    assert_eq!(log.graveyard_size, [2]);
    assert_eq!(json["draw"]["avg_life_gained"], 0.0);
}

/// Simultaneous later chapters cannot target a source sacrificed within their own batch.
pub(super) fn assert_simultaneous_saga_chapter_targets() {
    let cards = deck(&[
        row(
            "First Saga",
            "{0}",
            "Enchantment — Saga",
            "I — Gain 1 life.\nII — Gain 1 life.",
        ),
        row(
            "Second Saga",
            "{0}",
            "Enchantment — Saga",
            "I — Gain 1 life.\nII — Return target card from your graveyard to your hand.",
        ),
        row("Safe next draw", "{99}", "Sorcery", ""),
    ]);
    assert!(matches!(
        cards[CardIdx(1)].saga.chapters[1].as_slice(),
        [SimEffect::ReturnFromGraveyard {
            to_hand: true,
            count: 1
        },]
    ));
    let mut game = state(vec![0, 1], vec![2, 2]);
    game.next_uid = 100;
    let mut census = TurnCensus::new(2, cards.cards.len());
    let commander = CommanderProfile::new(&cards);
    let mut sources = Vec::new();
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut sources,
        1,
        false,
    );
    assert_eq!(
        game.battlefield
            .iter()
            .map(|permanent| (permanent.uid, permanent.card, permanent.saga_step))
            .collect::<Vec<_>>(),
        [
            (101, CardRef::Deck(CardIdx(0)), 1),
            (102, CardRef::Deck(CardIdx(1)), 1),
        ]
    );
    assert!(game.graveyard.is_empty());
    assert_eq!((game.life, game.life_gained), (22, 2));
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut sources,
        2,
        false,
    );
    assert!(game.completion.is_none());
    assert!(game.battlefield.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0), CardIdx(1)]);
    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(game.next_uid, 102);
    assert_eq!((game.life, game.life_gained), (23, 3));
    for card in [CardIdx(0), CardIdx(1)] {
        assert_eq!(game.graveyard_generations.get(&card), Some(&1));
        assert_eq!(game.graveyard_seen.get(&card), Some(&2));
    }
    let log = finish_log(&cards, game, census, 0, false, 2);
    assert_eq!(log.graveyard_size, [0, 2]);
    assert_eq!(log.mana_spent, [0.0, 0.0]);
    assert_eq!(log.life_gained, 3);
    assert_eq!(log.card_first_graveyard.get(&0), Some(&2));
    assert_eq!(log.card_first_graveyard.get(&1), Some(&2));
    let json =
        serde_json::to_value(report_log(&cards, &log, 2)).expect("Saga batch report serializes");
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 0.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["2"], 2.0);
    assert_eq!(json["draw"]["avg_life_gained"], 3.0);
    assert_saga_batch_loss_boundary();
}

/// A later chapter in the same batch cannot rescue a loss from an earlier complete chapter.
fn assert_saga_batch_loss_boundary() {
    let cards = deck(&[
        row(
            "First lethal Saga",
            "{0}",
            "Enchantment — Saga",
            "I — Gain 1 life.\nII — Each player loses 4 life. Gain 1 life.",
        ),
        row(
            "Second life Saga",
            "{0}",
            "Enchantment — Saga",
            "I — Gain 1 life.\nII — Gain 100 life.",
        ),
        row("Safe next draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0, 1], vec![2]);
    game.life = 1;
    let (game, log) = execute_turns(&cards, game, 2, vec![]);
    assert_eq!(
        game.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::ZeroLife],
            turn: 2,
            phase: GamePhase::Main,
        })
    );
    assert_eq!((game.life, game.life_gained), (0, 3));
    assert_eq!(
        game.battlefield
            .iter()
            .map(|source| (source.uid, source.card, source.saga_step))
            .collect::<Vec<_>>(),
        [
            (1, CardRef::Deck(CardIdx(0)), 2),
            (2, CardRef::Deck(CardIdx(1)), 2),
        ]
    );
    assert!(game.graveyard.is_empty());
    assert!(log.card_first_graveyard.is_empty());
    assert_eq!(log.life_gained, 3);
    let report = report_log(&cards, &log, 2);
    assert_eq!(report.outcomes.finished_games, 1);
    assert_eq!(report.outcomes.player_loss_games, 1);
    assert_eq!(report.outcomes.incomplete_games, 0);
}

/// Run the simultaneous chapter target-binding action and report regression.
#[test]
fn simultaneous_saga_chapters_bind_targets_before_any_resolve() {
    assert_simultaneous_saga_chapter_targets();
}

/// An incomplete final chapter stops source effects and sacrifice on entry and later turns.
pub(super) fn assert_saga_chapter_incomplete_stop() {
    for turns in [1, 2] {
        let text = if turns == 1 {
            "I — Create nine 1/1 Elf creature tokens. Put a charge counter on this artifact."
        } else {
            "I — Gain 1 life.\nII — Create nine 1/1 Elf creature tokens. Put a charge counter on this artifact."
        };
        let cards = deck(&[
            row(
                "Incomplete Saga",
                "{0}",
                "Artifact Enchantment — Saga",
                text,
            ),
            row("Safe next draw", "{99}", "Sorcery", ""),
        ]);
        assert!(matches!(
            cards[CardIdx(0)].saga.chapters[turns - 1].as_slice(),
            [SimEffect::Tokens(9), SimEffect::Counters(1),]
        ));
        let mut game = state(vec![0], vec![1]);
        game.next_uid = 100;
        let (game, log) = execute_turns(&cards, game, turns, vec![]);
        let completion = GameCompletion::Incomplete {
            reason: IncompleteReason::EffectWork {
                configured_limit: 8,
                remaining_work: 1,
            },
            turn: turns as u32,
            phase: GamePhase::Main,
        };
        assert_eq!(game.completion.as_ref(), Some(&completion));
        assert_eq!(log.completion, completion);
        assert_eq!(game.battlefield.len(), 9);
        let source = &game.battlefield[0];
        assert_eq!(
            (
                source.uid,
                source.card,
                source.saga_step,
                source.counters.charge
            ),
            (101, CardRef::Deck(CardIdx(0)), turns as u32, 0)
        );
        assert!(
            game.battlefield[1..]
                .iter()
                .all(|token| token.card == CardRef::Token(TokenKind::Creature))
        );
        assert_eq!(game.next_uid, 109);
        assert!(game.graveyard.is_empty());
        assert!(!log.card_first_graveyard.contains_key(&0));
        let report = report_log(&cards, &log, turns as u32);
        assert_eq!(report.outcomes.requested_games, 1);
        assert_eq!(report.outcomes.finished_games, 0);
        assert_eq!(report.outcomes.player_loss_games, 0);
        assert_eq!(report.outcomes.incomplete_games, 1);
        assert_eq!(report.outcomes.incomplete_observations.len(), 1);
        let observation = &report.outcomes.incomplete_observations[0];
        assert_eq!(
            observation.reason,
            IncompleteReason::EffectWork {
                configured_limit: 8,
                remaining_work: 1
            }
        );
        assert_eq!(
            (observation.turn, observation.phase),
            (turns as u32, GamePhase::Main)
        );
        assert!(!report.creatures_by_turn.contains_key(&(turns as u32)));
        assert!(
            !report
                .graveyard
                .avg_size_by_turn
                .contains_key(&(turns as u32))
        );
        assert_eq!(report.draw.avg_life_gained, None);
    }
}

/// Run the incomplete chapter stop action and report regression.
#[test]
fn incomplete_saga_chapters_stop_before_later_source_effects() {
    assert_saga_chapter_incomplete_stop();
}

/// Loss checks follow the complete final chapter and prevent a free death-trigger rescue.
#[test]
fn lethal_entry_and_later_chapters_finish_linked_effects_before_stopping() {
    for turns in [1, 2] {
        let text = if turns == 1 {
            "I — Each player loses 2 life. Gain 1 life.\nWhen this creature dies, gain 100 life."
        } else {
            "I — Gain 1 life.\nII — Each player loses 3 life. Gain 1 life.\nWhen this creature dies, gain 100 life."
        };
        let cards = deck(&[
            row(
                "Synthetic lethal Saga",
                "{0}",
                "Enchantment Creature — Saga",
                text,
            ),
            row("Safe next draw", "{99}", "Sorcery", ""),
        ]);
        assert!(
            cards[CardIdx(0)]
                .unlocked_abilities(0)
                .any(|ability| ability.trigger == SimTrigger::Dies)
        );
        let mut game = state(vec![0], vec![1]);
        game.life = 1;
        let (game, log) = execute_turns(&cards, game, turns, vec![]);
        assert_eq!(game.life, 0);
        assert_eq!(game.life_gained, turns as u32);
        assert_eq!(game.life_paid, 0);
        assert_eq!(
            game.completion,
            Some(GameCompletion::PlayerLost {
                reasons: vec![PlayerLossReason::ZeroLife],
                turn: turns as u32,
                phase: GamePhase::Main,
            })
        );
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert!(game.graveyard.is_empty());
        assert!(!log.card_first_graveyard.contains_key(&0));
        assert_eq!(log.life_gained, turns as u32);
        let json = serde_json::to_value(report_log(&cards, &log, turns as u32))
            .expect("Saga loss report serializes");
        assert_eq!(json["outcomes"]["player_loss_games"], 1);
        assert_eq!(json["outcomes"]["finished_games"], 1);
        assert_eq!(json["outcomes"]["incomplete_games"], 0);
    }
}

/// Unknown chapter structure stays inert instead of receiving a guessed sacrifice turn.
#[test]
fn saga_without_parsed_chapters_does_not_gain_a_cleanup_rule() {
    let cards = deck(&[
        row(
            "Unknown Saga structure",
            "{0}",
            "Enchantment — Saga",
            "No supported chapter wording.",
        ),
        row("Safe next draw", "{99}", "Sorcery", ""),
    ]);
    let game = state(vec![0], vec![1]);
    let (game, log, json) = executed_report(&cards, game, 2, vec![]);
    assert_eq!(cards[CardIdx(0)].chapter_count(), 0);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
    assert!(game.graveyard.is_empty());
    assert_eq!((game.life, game.life_gained), (20, 0));
    assert_eq!(log.graveyard_size, [0, 0]);
    assert!(!log.card_first_graveyard.contains_key(&0));
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 0.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["2"], 0.0);
}
