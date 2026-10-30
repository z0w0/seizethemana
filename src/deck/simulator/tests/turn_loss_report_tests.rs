//! Synthetic settlement and loss reports preserve executed work and missing phases.

use crate::deck::simulator::card_test_support::assert_library_rng;
use crate::deck::simulator::game::{
    CardRef, GameCompletion, GamePhase, IncompleteReason, PlayerLossReason, record_card_seen,
};
use crate::deck::simulator::library_core_report_tests::put_permanents;
use crate::deck::simulator::model::{CardIdx, SimEffect, SimTrigger};
use crate::deck::simulator::oracle_ast::{OracleAbility, OracleEffect, OracleTriggerEvent};
use crate::deck::simulator::oracle_parser::parse_oracle_card;
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::report_schema::ObservationStatus;
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// A paid event resolves exactly its queued listeners, including both sides of the cap.
pub(super) fn assert_settlement_cap_report() {
    for listeners in [1, 1023, 1024, 1025] {
        let mut rows = vec![
            row(
                "Paid Ring event",
                "{1}",
                "Sorcery",
                "Gain 0 life. The Ring tempts you.",
            ),
            row("Funding land", "", "Basic Land", "{T}: Add {C}."),
        ];
        rows.extend((0..listeners).map(|_| {
            row(
                "Finite listener",
                "{99}",
                "Artifact",
                "Whenever the Ring tempts you, gain 1 life.",
            )
        }));
        let spell = parse_oracle_card(&rows[0]);
        assert!(matches!(&spell.abilities[0], OracleAbility::Spell(ability)
            if matches!(ability.effects.as_slice(), [OracleEffect::GainLife(0), OracleEffect::RingTempts])));
        let listener = parse_oracle_card(&rows[2]);
        assert!(
            matches!(&listener.abilities[0], OracleAbility::Triggered(ability)
            if ability.event == OracleTriggerEvent::RingTempts
                && matches!(ability.effects.as_slice(), [OracleEffect::GainLife(1)]))
        );
        let cards = deck(&rows);
        assert!(matches!(
            cards.cards[0].spell_data.effects.as_slice(),
            [SimEffect::GainLife(0), SimEffect::RingTempts]
        ));
        assert!(
            cards.cards[2]
                .unlocked_abilities(0)
                .any(|ability| ability.trigger == SimTrigger::RingTempts
                    && matches!(ability.effect_sequence(), [SimEffect::GainLife(1)]))
        );
        let mut game = state(vec![0], vec![]);
        put_permanents(&cards, &mut game, &(1..listeners + 2).collect::<Vec<_>>());
        let rng = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        let incomplete = listeners > 1024;
        let resolved = listeners.min(1024);
        assert_eq!(game.life_gained, resolved);
        assert_eq!(game.life, 20 + resolved as i32);
        assert_eq!(game.ring_tempts, 1);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert!(game.hand.is_empty() && game.library.is_empty() && game.exile.is_empty());
        assert_eq!(game.battlefield.len(), listeners as usize + 1);
        assert_eq!(game.resolution.trigger_limit_reached, incomplete);
        assert_library_rng(&game.library_rng, &rng);
        if incomplete {
            assert_eq!(
                game.completion,
                Some(GameCompletion::Incomplete {
                    reason: IncompleteReason::TriggerSettlement {
                        configured_limit: 1024,
                        remaining_work: 1
                    },
                    turn: 1,
                    phase: GamePhase::Main
                })
            );
        } else {
            assert!(game.completion.is_none());
        }
        let report = report_log(&cards, &log, 1);
        assert_eq!(log.mana_spent, [1.0]);
        assert_eq!(report.outcomes.requested_games, 1);
        assert_eq!(report.outcomes.finished_games, u32::from(!incomplete));
        assert_eq!(report.outcomes.incomplete_games, u32::from(incomplete));
        assert_eq!(report.outcomes.player_loss_games, 0);
        assert_eq!(
            report.phase_observations.end_turn_by_turn[&1],
            f64::from(u32::from(!incomplete))
        );
        let rate = &report.sampling["win_conditions.percent_games_with_extra_turn"];
        assert_eq!(rate.observed_games, u32::from(!incomplete));
        assert_eq!(rate.event_games, (!incomplete).then_some(0));
        if incomplete {
            assert_eq!(
                report.outcomes.incomplete_reasons["trigger_settlement_limit"],
                1
            );
            let observation = &report.outcomes.incomplete_observations[0];
            assert_eq!(
                observation.reason,
                IncompleteReason::TriggerSettlement {
                    configured_limit: 1024,
                    remaining_work: 1
                }
            );
            assert_eq!(observation.turn, 1);
            assert_eq!(observation.phase, GamePhase::Main);
            assert_eq!(observation.games, 1);
            assert_eq!(rate.status, ObservationStatus::NoObservations);
            assert!(rate.percent.is_none());
            assert!(report.velocity.avg_cards_seen.is_empty());
            assert!(report.mana.avg_cast_spent_by_turn.is_empty());
            assert!(report.draw.avg_life_gained.is_none());
        } else {
            assert!(report.outcomes.incomplete_observations.is_empty());
            assert_eq!(report.draw.avg_life_gained, Some(f64::from(resolved)));
            assert_eq!(report.mana.avg_cast_spent_by_turn[&1], 1.0);
        }
    }
    assert_linked_entry_settlement();
}

/// The returned creature draws only after its causing spell finishes its mill instruction.
fn assert_linked_entry_settlement() {
    for mill in [true, false] {
        let cards = deck(&[
            row(
                "Return spell",
                "{1}",
                "Sorcery",
                if mill {
                    "Return target creature card from your graveyard to the battlefield. Mill a card."
                } else {
                    "Return target creature card from your graveyard to the battlefield."
                },
            ),
            row(
                "Returned listener",
                "{99}",
                "Creature",
                "When this creature enters, draw a card.",
            ),
            row("Top card", "{99}", "Sorcery", ""),
            row("Next card", "{99}", "Sorcery", ""),
            row("Funding land", "", "Basic Land", "{T}: Add {C}."),
        ]);
        let mut game = state(vec![0], vec![3, 2]);
        game.graveyard.push(CardIdx(1));
        put_permanents(&cards, &mut game, &[4]);
        let rng = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.hand, [CardIdx(if mill { 3 } else { 2 })]);
        assert_eq!(game.library, if mill { vec![] } else { vec![CardIdx(3)] });
        assert_eq!(
            game.graveyard,
            if mill {
                vec![CardIdx(2), CardIdx(0)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
        assert_eq!(game.battlefield[1].uid, 2);
        assert_eq!(game.milled_self, u32::from(mill));
        assert_library_rng(&game.library_rng, &rng);
        let report = report_log(&cards, &log, 1);
        assert_eq!(log.card_first_battlefield.get(&1), Some(&1));
        assert_eq!(
            report.velocity.self_milled_by_turn[&1],
            f64::from(u32::from(mill))
        );
        assert_eq!(
            report.velocity.avg_cards_seen[&1],
            if mill { 2.0 } else { 1.0 }
        );
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], 1.0);
        assert_eq!(report.outcomes.player_loss_games, 0);
    }
}

/// Failed draws finish linked effects, stop cascade parents, and retain pre-loss access.
pub(super) fn assert_loss_boundary_report() {
    assert_draw_and_mill_loss_boundary();
    assert_cascade_child_boundary();
    assert_pre_loss_land_access();
}

/// An empty mill or failed search is not the failed-draw loss event.
fn assert_draw_and_mill_loss_boundary() {
    for (text, lost, gain) in [
        ("Draw a card. Gain three life.", true, 3),
        ("Mill two cards. Gain three life.", false, 3),
        (
            "Search your library for a basic land card, put it onto the battlefield, then shuffle.",
            false,
            0,
        ),
    ] {
        let cards = deck(&[
            row("Boundary spell", "{1}", "Sorcery", text),
            row("Funding land", "", "Basic Land", "{T}: Add {C}."),
        ]);
        let mut game = state(vec![0], vec![]);
        put_permanents(&cards, &mut game, &[1]);
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.failed_draw, lost);
        assert_eq!(game.life, 20 + gain);
        assert_eq!(game.life_gained, gain as u32);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.battlefield.len(), 1);
        assert!(game.hand.is_empty() && game.library.is_empty());
        if lost {
            assert_eq!(
                game.completion,
                Some(GameCompletion::PlayerLost {
                    reasons: vec![PlayerLossReason::FailedDraw],
                    turn: 1,
                    phase: GamePhase::Main
                })
            );
        } else {
            assert!(game.completion.is_none());
        }
        let report = report_log(&cards, &log, 1);
        assert_eq!(report.outcomes.finished_games, 1);
        assert_eq!(report.outcomes.player_loss_games, u32::from(lost));
        assert_eq!(report.draw.avg_life_gained, Some(f64::from(gain)));
        assert_eq!(
            report.phase_observations.main_by_turn[&1],
            f64::from(u32::from(!lost))
        );
        assert_eq!(
            report.phase_observations.end_turn_by_turn[&1],
            f64::from(u32::from(!lost))
        );
        if lost {
            assert!(report.velocity.avg_cards_seen.is_empty());
        } else {
            assert_eq!(report.velocity.avg_cards_seen[&1], 0.0);
        }
    }
}

/// A terminal or incomplete cascade child cannot create a parent battlefield visit.
fn assert_cascade_child_boundary() {
    for mode in ["failed_draw", "complete", "incomplete"] {
        let child_text = if mode == "incomplete" {
            "When this artifact enters, the Ring tempts you.\nWhenever the Ring tempts you, the Ring tempts you."
        } else {
            "Draw a card. Gain three life."
        };
        let cards = deck(&[
            row(
                "Cascade parent",
                "{3}",
                "Artifact",
                "As an additional cost to cast this spell, pay 1 life.\nCascade\nWhen this artifact enters, gain 7 life.\nAt the beginning of your upkeep, draw a card.",
            ),
            row(
                "Cascade child",
                "{1}",
                if mode == "incomplete" {
                    "Artifact"
                } else {
                    "Sorcery"
                },
                child_text,
            ),
            row("Draw marker", "{99}", "Sorcery", ""),
            row("Funding one", "", "Basic Land", "{T}: Add {C}."),
            row("Funding two", "", "Basic Land", "{T}: Add {C}."),
            row("Funding three", "", "Basic Land", "{T}: Add {C}."),
            row("Funding four", "", "Basic Land", "{T}: Add {C}."),
        ]);
        let complete = mode == "complete";
        let incomplete = mode == "incomplete";
        let mut game = state(vec![0], if complete { vec![2, 1] } else { vec![1] });
        put_permanents(&cards, &mut game, &[3, 4, 5, 6]);
        let rng = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 3, vec![]);
        // Only the successful one-turn case can continue; its next upkeep draw is terminal.
        assert_eq!(game.life_paid, 1);
        assert_eq!(
            game.life_gained,
            if complete {
                10
            } else if incomplete {
                0
            } else {
                3
            }
        );
        assert_eq!(
            game.life,
            if complete {
                29
            } else if incomplete {
                19
            } else {
                22
            }
        );
        assert_eq!(game.battlefield_seen.contains_key(&CardIdx(0)), complete);
        assert_eq!(log.card_first_battlefield.get(&0), complete.then_some(&1));
        assert_eq!(game.spells_cast_this_turn, if complete { 0 } else { 2 });
        assert_eq!(
            game.graveyard,
            if incomplete { vec![] } else { vec![CardIdx(1)] }
        );
        assert_eq!(game.hand, if complete { vec![CardIdx(2)] } else { vec![] });
        assert_library_rng(&game.library_rng, &rng);
        let report = report_log(&cards, &log, 3);
        assert_eq!(log.mana_spent, [3.0, 0.0, 0.0]);
        assert_eq!(report.outcomes.finished_games, u32::from(!incomplete));
        assert_eq!(report.outcomes.incomplete_games, u32::from(incomplete));
        assert_eq!(report.outcomes.player_loss_games, u32::from(!incomplete));
        assert_eq!(
            report.phase_observations.end_turn_by_turn[&1],
            f64::from(u32::from(complete))
        );
        assert_eq!(report.phase_observations.end_turn_by_turn[&2], 0.0);
        if incomplete {
            assert_eq!(game.ring_tempts, 1024);
            assert_eq!(
                report.outcomes.incomplete_reasons["trigger_settlement_limit"],
                1
            );
            assert!(report.draw.avg_life_paid.is_none());
        } else {
            assert_eq!(report.draw.avg_life_paid, Some(1.0));
        }
        if !complete {
            assert!(report.velocity.avg_cards_seen.is_empty());
        }
        let short = &report.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"];
        assert_eq!(short.status, ObservationStatus::OutsideHorizon);
        assert_eq!((short.event_games, short.observed_games), (None, 0));
    }
}

/// A terminal spell retains the actual six-land access event without an end-turn mean.
fn assert_pre_loss_land_access() {
    for six in [true, false] {
        let mut rows = vec![row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."); 6];
        rows.push(row(
            "Terminal access",
            "{0}",
            "Sorcery",
            "Draw two cards. Mill three cards. You lose 1 life.",
        ));
        let cards = deck(&rows);
        let mut game = state(
            vec![0, 6],
            if six {
                vec![1, 2, 3, 4, 5]
            } else {
                vec![2, 3, 4, 5]
            },
        );
        if !six {
            game.exile.push(CardIdx(1));
        }
        game.life = 1;
        game.seen = 2;
        game.awareness_cards = 2;
        for index in game.hand.clone() {
            record_card_seen(&mut game, index, 1);
        }
        let rng = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 4, vec![]);
        assert_eq!(
            game.completion,
            Some(GameCompletion::PlayerLost {
                reasons: vec![PlayerLossReason::ZeroLife],
                turn: 1,
                phase: GamePhase::Main
            })
        );
        assert!(!game.failed_draw);
        assert_eq!(game.milled_self, if six { 3 } else { 2 });
        assert_eq!(game.hand, [CardIdx(5), CardIdx(4)]);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(
            game.graveyard,
            if six {
                vec![CardIdx(3), CardIdx(2), CardIdx(1), CardIdx(6)]
            } else {
                vec![CardIdx(3), CardIdx(2), CardIdx(6)]
            }
        );
        assert_library_rng(&game.library_rng, &rng);
        assert_eq!(log.lands_seen_by_turn_4, if six { 6 } else { 5 });
        assert_eq!(log.cards_seen_by_4, if six { 7 } else { 6 });
        assert_eq!(log.turns_observed, 0);
        let report = report_log(&cards, &log, 4);
        let rate = &report.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"];
        assert_eq!(
            (rate.event_games, rate.observed_games),
            (Some(u32::from(six)), 1)
        );
        assert_eq!(
            rate.percent.expect("finished denominator").value(),
            if six { 100.0 } else { 0.0 }
        );
        assert_eq!(report.outcomes.player_loss_games, 1);
        assert!(report.velocity.avg_cards_seen.is_empty());
        assert_eq!(report.phase_observations.end_turn_by_turn[&4], 0.0);
    }
}
