use super::*;
use crate::deck::simulator::model::{LifeLossScope, SimEffect};
use crate::deck::simulator::turn_loop_tests::{deck, row, state};
use crate::deck::simulator::{
    aggregate, cast_pass, findings, game, game_run, opponents, report, report_schema,
};

/// An opponent-triggered ability finishes all linked effects before checking zero life.
#[test]
fn opponent_ability_uses_one_complete_resolution_boundary() {
    for gain in [0, 3] {
        let mut cards = deck(&[row(
            "Opponent listener",
            "{1}",
            "Artifact",
            "Whenever an opponent draws a card, gain 1 life.",
        )]);
        let ability = &mut cards.cards[0].striations[0].abilities[0];
        ability.effects = vec![
            SimEffect::Damage {
                amount: 1,
                scope: LifeLossScope::EachPlayer,
            },
            SimEffect::GainLife(gain),
        ];
        let mut st = state(vec![], vec![]);
        st.life = 1;
        st.current_turn = 1;
        st.current_phase = game::GamePhase::OpponentRound;
        st.opponents.config.active = true;
        st.battlefield
            .push(game::new_perm_with(1, &cards, CardIdx(0), 0, false));
        opponents::run_opponent_round(&cards, &mut st, 1);
        assert_eq!(st.life, gain as i32);
        assert_eq!(st.life_gained, gain);
        assert_eq!(st.damage_dealt_this_turn, 1);
        assert_eq!(st.completion.is_some(), gain == 0);
        if gain == 0 {
            assert!(matches!(
                st.completion,
                Some(game::GameCompletion::PlayerLost {
                    turn: 1,
                    phase: game::GamePhase::OpponentRound,
                    ..
                })
            ));
            assert_eq!(st.opponents.casts, 0);
        }
    }
}

/// A Saga's linked chapter effects finish before loss or final-chapter cleanup.
#[test]
fn saga_chapter_uses_one_complete_resolution_boundary() {
    for gain in [0, 3] {
        let mut cards = deck(&[row(
            "Saga",
            "{1}",
            "Enchantment — Saga",
            "I — Gain 1 life.\nII — Gain 1 life.\nIII — Gain 1 life.",
        )]);
        assert_eq!(cards.cards[0].chapter_count(), 3);
        cards.cards[0].saga.chapters[2] = vec![
            SimEffect::Damage {
                amount: 1,
                scope: LifeLossScope::EachPlayer,
            },
            SimEffect::GainLife(gain),
        ];
        let mut st = state(vec![], vec![]);
        st.life = 1;
        st.current_turn = 2;
        st.current_phase = game::GamePhase::Main;
        let mut saga = game::new_perm_with(1, &cards, CardIdx(0), 1, false);
        saga.saga_step = 2;
        st.battlefield.push(saga);
        game_run::run_sagas(&cards, &mut st, 2);
        assert_eq!(st.life, gain as i32);
        assert_eq!(st.life_gained, gain);
        assert_eq!(st.completion.is_some(), gain == 0);
        if gain > 0 {
            assert_eq!(st.graveyard, [CardIdx(0)]);
            assert!(st.battlefield.is_empty());
        } else {
            assert!(matches!(
                st.completion,
                Some(game::GameCompletion::PlayerLost {
                    turn: 2,
                    phase: game::GamePhase::Main,
                    ..
                })
            ));
            assert_eq!(st.battlefield.len(), 1);
            assert!(st.graveyard.is_empty());
        }
    }
}

/// Terminal main-phase and opponent-round draws reconcile every card-access view.
#[test]
fn terminal_draws_reconcile_pair_role_and_creature_access() {
    for opponent_round in [false, true] {
        let source_text = if opponent_round {
            "Whenever an opponent draws a card, draw three cards."
        } else {
            "Whenever you cast a spell, draw three cards."
        };
        let cards = deck(&[
            row("Removal", "{1}", "Instant", "Destroy target creature."),
            row("Creature", "{1}", "Creature", ""),
            row("Draw source", "{100}", "Artifact", source_text),
            row("Cast event", "{0}", "Artifact", ""),
        ]);
        assert_eq!(cards.cards[0].role, Role::Removal);
        assert!(cards.cards[1].is_creature);
        let mut st = state(if opponent_round { vec![] } else { vec![3] }, vec![1, 0]);
        st.current_turn = 1;
        st.opponents.config.active = true;
        st.battlefield
            .push(game::new_perm_with(1, &cards, CardIdx(2), 0, false));
        st.current_phase = if opponent_round {
            game::GamePhase::OpponentRound
        } else {
            game::GamePhase::Main
        };
        if opponent_round {
            opponents::run_opponent_round(&cards, &mut st, 1);
        } else {
            assert_eq!(
                cast_pass::try_cast_from_hand(
                    &cards,
                    &mut st,
                    &mut game::ManaPool::default(),
                    1,
                    CardIdx(3),
                    &mut vec![],
                    &mut vec![],
                    &mut [false; 5]
                ),
                Some(0)
            );
        }
        assert!(matches!(
            st.completion,
            Some(game::GameCompletion::PlayerLost { .. })
        ));
        assert!(st.failed_draw);
        assert!(st.library.is_empty());
        assert_eq!(st.card_first_seen[&0], 1);
        assert_eq!(st.card_first_seen[&1], 1);
        let log = finish_log(
            &cards,
            st,
            TurnCensus::new(6, cards.cards.len()),
            0,
            false,
            6,
        );
        assert_eq!(log.first_seen[&Role::Removal], 1);
        assert_eq!(log.first_creature, Some(1));
        let pairs = findings::piece_pair_access(
            std::slice::from_ref(&log),
            &cards,
            &[("Removal".into(), "Creature".into())],
            6,
        );
        let pair = report_schema::PairAccessReport::from_access(&pairs[0], 1, 6);
        assert_eq!(
            (pair.access.event_games, pair.access.observed_games),
            (Some(1), 1)
        );
        let stats = aggregate::aggregate(&[log], &cards, 6);
        let payload = report::json_report(
            &stats,
            &cards,
            "terminal access",
            1,
            &[],
            Default::default(),
            &Default::default(),
        );
        assert_eq!(payload.outcomes.player_loss_games, 1);
        assert_eq!(
            payload
                .role_access
                .removal_spell_percent_seen_by_turn_5
                .expect("finished loss denominator")
                .value(),
            100.0
        );
        assert_eq!(
            payload
                .role_access
                .creature_role_percent_seen_by_turn_3
                .expect("finished loss denominator")
                .value(),
            100.0
        );
        assert_eq!(
            payload.sampling["role_access.removal_spell_percent_seen_by_turn_5"].event_games,
            Some(1)
        );
        assert_eq!(
            payload.sampling["role_access.creature_role_percent_seen_by_turn_3"].event_games,
            Some(1)
        );
        assert!(payload.velocity.avg_cards_seen.is_empty());
    }
}
