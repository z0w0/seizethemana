//! Synthetic upkeep conditions use one initial event state and a later recheck.

use crate::deck::simulator::card_test_support::assert_library_rng;
use crate::deck::simulator::game::{CardRef, GameCompletion, graveyard_generation};
use crate::deck::simulator::library_core_report_tests::put_permanents;
use crate::deck::simulator::model::{CardIdx, SimAbilityCondition, SimEffect, SimTrigger};
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use std::collections::HashSet;

/// Mill cannot create a false-at-start trigger, and a captured condition must remain true.
pub(super) fn assert_upkeep_condition_capture_report() {
    for case in [
        "initial_false",
        "initial_true",
        "resolution_false",
        "registered_order",
    ] {
        let returns = case == "resolution_false";
        let initial_true = case != "initial_false";
        let registered_order = case == "registered_order";
        let cards = deck(&[
            row(
                "First upkeep land",
                "",
                "Land",
                if returns {
                    "{T}: Add {C}.\nAt the beginning of your upkeep, return target creature card from your graveyard to the battlefield."
                } else {
                    "{T}: Add {C}.\nAt the beginning of your upkeep, mill a card."
                },
            ),
            row(
                "Conditional upkeep land",
                "",
                "Land",
                "{T}: Add {C}.\nAt the beginning of your upkeep, if there are four or more permanent cards in your graveyard, draw a card. This ability triggers only once each turn.",
            ),
            row("Graveyard creature", "{99}", "Creature", ""),
            row("First graveyard artifact", "{99}", "Artifact", ""),
            row("Second graveyard artifact", "{99}", "Artifact", ""),
            row("Fourth graveyard artifact", "{99}", "Artifact", ""),
            row("Top permanent", "{99}", "Artifact", ""),
            row("Next physical card", "{99}", "Sorcery", ""),
            row("Library bottom", "{99}", "Sorcery", ""),
        ]);
        let conditional = cards.cards[1]
            .unlocked_abilities(0)
            .find(|ability| ability.trigger == SimTrigger::Upkeep)
            .expect("the printed upkeep condition lowers");
        assert_eq!(conditional.condition, Some(SimAbilityCondition::Descend(4)));
        assert!(conditional.once_per_turn);
        assert!(matches!(
            conditional.effect_sequence(),
            [SimEffect::Draw(1)]
        ));
        let conditional_id = conditional.id;
        let mut game = state(vec![], vec![8, 7, 6]);
        game.graveyard = if initial_true {
            vec![CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(5)]
        } else {
            vec![CardIdx(2), CardIdx(3), CardIdx(4)]
        };
        if !initial_true {
            game.exile.push(CardIdx(5));
        }
        put_permanents(&cards, &mut game, &[0, 1]);
        let rng = game.library_rng.clone();
        let sources = if registered_order {
            vec![(2, 1), (1, 0)]
        } else {
            vec![]
        };
        let (game, log) = execute_turns(&cards, game, 1, sources);
        assert_eq!(log.completion, GameCompletion::HorizonReached, "{case}");
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(game.battlefield[1].uid, 2);
        assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
        let drawn = initial_true && !returns;
        assert_eq!(
            game.hand,
            if drawn {
                vec![CardIdx(if registered_order { 6 } else { 7 })]
            } else {
                vec![]
            },
            "{case}"
        );
        assert_eq!(
            game.library,
            if returns {
                vec![CardIdx(8), CardIdx(7), CardIdx(6)]
            } else if drawn {
                vec![CardIdx(8)]
            } else {
                vec![CardIdx(8), CardIdx(7)]
            },
            "{case}"
        );
        let expected_graveyard = if returns {
            vec![CardIdx(3), CardIdx(4), CardIdx(5)]
        } else if initial_true {
            vec![
                CardIdx(2),
                CardIdx(3),
                CardIdx(4),
                CardIdx(5),
                CardIdx(if registered_order { 7 } else { 6 }),
            ]
        } else {
            vec![CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(6)]
        };
        assert_eq!(game.graveyard, expected_graveyard, "{case}");
        assert_eq!(
            game.triggered_this_turn,
            if initial_true {
                HashSet::from([(2, conditional_id)])
            } else {
                HashSet::new()
            },
            "{case}"
        );
        assert!(game.activated_this_turn.is_empty());
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.life_gained, 0);
        assert!(!game.failed_draw);
        assert_eq!(game.milled_self, u32::from(!returns));
        if returns {
            assert_eq!(game.battlefield.len(), 3);
            assert_eq!(game.battlefield[2].card, CardRef::Deck(CardIdx(2)));
            assert_eq!(game.battlefield[2].uid, 3);
            assert_eq!(game.graveyard_generations.get(&CardIdx(2)), None);
        } else {
            assert_eq!(game.battlefield.len(), 2);
        }
        assert_library_rng(&game.library_rng, &rng);
        let report = report_log(&cards, &log, 1);
        assert_eq!(log.mana_spent, [0.0]);
        assert_eq!(log.land_drops, [0]);
        assert_eq!(log.cards_seen, [u32::from(!returns) + u32::from(drawn)]);
        assert_eq!(
            report.velocity.avg_cards_seen[&1],
            f64::from(u32::from(!returns) + u32::from(drawn))
        );
        assert_eq!(
            report.velocity.self_milled_by_turn[&1],
            f64::from(u32::from(!returns))
        );
        assert_eq!(
            report.graveyard.avg_size_by_turn[&1],
            expected_graveyard.len() as f64
        );
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], 0.0);
        assert_eq!(report.repeatable_sources_by_turn[&1], 2.0);
        assert_eq!(report.phase_observations.end_turn_by_turn[&1], 1.0);
        assert_eq!(report.outcomes.finished_games, 1);
        assert_eq!(report.outcomes.player_loss_games, 0);
        assert_eq!(report.outcomes.incomplete_games, 0);
    }
}

/// A higher-cost returned card is discarded into a new graveyard visit, not a replacement target.
pub(super) fn assert_upkeep_target_generation_report() {
    for discard in [true, false] {
        let cards = deck(&[
            row(
                "Hand return land",
                "",
                "Land",
                "{T}: Add {C}.\nAt the beginning of your upkeep, return target card from your graveyard to your hand.",
            ),
            row(
                "Middle upkeep land",
                "",
                "Land",
                if discard {
                    "{T}: Add {C}.\nAt the beginning of your upkeep, draw a card, then discard a card."
                } else {
                    "{T}: Add {C}.\nAt the beginning of your upkeep, gain 1 life."
                },
            ),
            row(
                "Battlefield return land",
                "",
                "Land",
                "{T}: Add {C}.\nAt the beginning of your upkeep, return target creature card from your graveyard to the battlefield. This ability triggers only once each turn.",
            ),
            row("Physical target", "{99}", "Creature", ""),
            row("Physical draw", "{98}", "Sorcery", ""),
            row("Library bottom", "{99}", "Sorcery", ""),
        ]);
        assert!(cards.cards[3].min_cost.total() > cards.cards[4].min_cost.total());
        let final_ability = cards.cards[2]
            .unlocked_abilities(0)
            .find(|ability| ability.trigger == SimTrigger::Upkeep)
            .expect("the targeted upkeep ability lowers");
        assert!(matches!(
            final_ability.effect_sequence(),
            [SimEffect::ReturnFromGraveyard {
                to_hand: false,
                count: 1
            }]
        ));
        let final_id = final_ability.id;
        let mut game = state(vec![], vec![5, 4]);
        game.graveyard.push(CardIdx(3));
        put_permanents(&cards, &mut game, &[0, 1, 2]);
        let rng = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.battlefield.len(), 3);
        for (position, permanent) in game.battlefield.iter().enumerate() {
            assert_eq!(permanent.uid, position as u32 + 1);
            assert_eq!(permanent.card, CardRef::Deck(CardIdx(position as u32)));
        }
        assert_eq!(game.next_uid, 3);
        assert_eq!(game.hand, [CardIdx(if discard { 4 } else { 3 })]);
        assert_eq!(
            game.graveyard,
            if discard { vec![CardIdx(3)] } else { vec![] }
        );
        assert_eq!(
            game.library,
            if discard {
                vec![CardIdx(5)]
            } else {
                vec![CardIdx(5), CardIdx(4)]
            }
        );
        assert_eq!(graveyard_generation(&game, CardIdx(3)), u32::from(discard));
        assert_eq!(game.triggered_this_turn, HashSet::from([(3, final_id)]));
        assert!(!game.battlefield_seen.contains_key(&CardIdx(3)));
        assert_eq!(game.card_first_seen.get(&3), Some(&1));
        assert_eq!(game.life, if discard { 20 } else { 21 });
        assert_library_rng(&game.library_rng, &rng);
        let report = report_log(&cards, &log, 1);
        assert_eq!(log.completion, GameCompletion::HorizonReached);
        assert_eq!(
            report.velocity.avg_cards_seen[&1],
            f64::from(u32::from(discard))
        );
        assert_eq!(
            report.graveyard.avg_size_by_turn[&1],
            f64::from(u32::from(discard))
        );
        assert_eq!(
            report.draw.avg_life_gained,
            Some(if discard { 0.0 } else { 1.0 })
        );
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], 0.0);
        assert_eq!(report.outcomes.finished_games, 1);
        assert_eq!(report.outcomes.player_loss_games, 0);
        assert_eq!(report.outcomes.incomplete_games, 0);
    }
}
