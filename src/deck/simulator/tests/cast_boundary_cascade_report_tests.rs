//! Synthetic cascade boundary controls supplement the pinned one-cascade packet.

use super::*;

/// A cascade hit with its own cascade flag cannot cast the next lower-value card.
fn assert_nonchaining_report() {
    let cards = deck(&[
        row("Synthetic outer cascade", "{3}", "Artifact", "Cascade"),
        row(
            "Synthetic inner cascade",
            "{2}",
            "Artifact",
            "Cascade\nWhen this artifact enters, gain 3 life.",
        ),
        row(
            "Uncast next hit",
            "{1}",
            "Artifact",
            "When this artifact enters, gain 11 life.",
        ),
        row("Cascade payment", "{99}", "Artifact", "{T}: Add {C}{C}{C}."),
    ]);
    assert!(cards.cards[0].has_cascade && cards.cards[1].has_cascade);
    let mut game = state(vec![0], vec![2, 1]);
    put_permanents(&cards, &mut game, &[3]);
    let random = game.library_rng.clone();
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert_eq!(game.spells_cast_this_turn, 2);
    assert_eq!(game.library, [CardIdx(2)]);
    assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
    assert_eq!(game.battlefield.len(), 3);
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(game.battlefield[2].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(game.life, 23);
    assert_eq!(game.life_gained, 3);
    assert_eq!(game.life_paid, 0);
    assert_library_rng(&game.library_rng, &random);
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(log.cards_seen, [1]);
    assert_eq!(log.milestones_by_turn[0].free_cast_permanents_entered, 1);
    let report = report_log(&cards, &log, 1);
    assert_finished_turn(&report, &log);
    assert_eq!(report.draw.avg_life_gained, Some(3.0));
    assert_eq!(report.velocity.avg_cards_seen[&1], 1.0);
    assert_eq!(
        report.milestones.free_cast_permanents_entered_avg_by_turn[&1],
        1.0
    );
    assert_assumption(&report, "does not chain");
}

/// Check free X zero beside the ordinary largest-payable-X path and missing-color control.
fn assert_free_x_report() {
    let cards = deck(&[
        row("Synthetic cascade parent", "{3}", "Artifact", "Cascade"),
        row("Synthetic X child", "{X}{U}", "Sorcery", "Draw X cards."),
        row("Library one", "{99}", "Sorcery", ""),
        row("Library two", "{99}", "Sorcery", ""),
        row("Library three", "{99}", "Sorcery", ""),
        row("Library four", "{99}", "Sorcery", ""),
        row("Blue funding", "{99}", "Artifact", "{T}: Add {C}{C}{C}{U}."),
        row(
            "Colorless funding",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}{C}{C}.",
        ),
    ]);
    assert!(cards.cards[0].has_cascade);
    assert!(matches!(
        cards.cards[1].spell_data.x_class,
        Some(crate::deck::simulator::model::XClass::Draw)
    ));
    for (cascade, blue, casts, spent, seen) in [
        (true, true, 2, 3.0, 1),
        (true, false, 2, 3.0, 1),
        (false, true, 1, 4.0, 3),
        (false, false, 0, 0.0, 0),
    ] {
        let mut game = state(
            vec![usize::from(!cascade)],
            if cascade {
                vec![2, 3, 4, 5, 1]
            } else {
                vec![2, 3, 4, 5]
            },
        );
        put_permanents(&cards, &mut game, &[if blue { 6 } else { 7 }]);
        let random = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.spells_cast_this_turn, casts);
        assert_eq!(game.seen, seen);
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert!(game.exile.is_empty());
        assert_eq!(
            game.graveyard,
            if casts > 0 { vec![CardIdx(1)] } else { vec![] }
        );
        assert_eq!(
            game.library,
            if !cascade && blue {
                vec![CardIdx(2)]
            } else {
                vec![CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(5)]
            }
        );
        assert_eq!(
            game.hand,
            if !cascade && blue {
                vec![CardIdx(5), CardIdx(4), CardIdx(3)]
            } else if casts == 0 {
                vec![CardIdx(1)]
            } else {
                vec![]
            }
        );
        assert_eq!(game.battlefield.len(), 1 + usize::from(cascade));
        assert!(game.battlefield[0].tapped);
        if cascade {
            assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(0)));
            assert_eq!(game.battlefield[1].uid, 2);
        }
        assert_library_rng(&game.library_rng, &random);
        assert_eq!(log.mana_spent, [spent]);
        assert_eq!(log.cards_seen, [seen]);
        assert_eq!(log.mana_remaining, [4.0 - spent]);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], spent);
        assert_eq!(report.velocity.avg_cards_seen[&1], f64::from(seen));
        assert_eq!(
            report.velocity.library_remaining_by_turn[&1],
            game.library.len() as f64
        );
        assert_eq!(report.draw.avg_life_gained, Some(0.0));
        assert_assumption(&report, "Casting without paying the mana cost uses X = 0");
    }
}

/// Execute linked child effects before testing whether the parent may continue.
fn assert_child_stop_report(child_type: &str, text: &str, life: i32, remaining: bool) {
    let cards = deck(&[
        row(
            "Synthetic cascade parent",
            "{3}",
            "Artifact",
            "As an additional cost to cast this spell, pay 1 life.\nCascade\nWhen this artifact enters, gain 7 life.\nAt the beginning of your upkeep, draw a card.",
        ),
        row("Synthetic cascade child", "{1}", child_type, text),
        row("Library marker", "{99}", "Sorcery", ""),
        row(
            "Parent payment source",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}{C}{C}.",
        ),
    ]);
    let recursive = text.contains("Whenever the Ring");
    let failed = text.to_ascii_lowercase().contains("draw a card") && !remaining;
    let lethal = text == "You lose 1 life." && life == 2;
    let stopped = recursive || failed || lethal;
    let mut game = state(vec![0], if remaining { vec![2, 1] } else { vec![1] });
    game.life = life;
    put_permanents(&cards, &mut game, &[3]);
    let random = game.library_rng.clone();
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert_eq!(game.spells_cast_this_turn, 2);
    assert_eq!(game.life_paid, 1);
    assert_eq!(game.hand, if remaining { vec![CardIdx(2)] } else { vec![] });
    assert!(game.library.is_empty() && game.exile.is_empty());
    assert_eq!(
        game.battlefield
            .iter()
            .filter(|perm| perm.card == CardRef::Deck(CardIdx(0)))
            .count(),
        usize::from(!stopped)
    );
    assert_eq!(
        game.battlefield_seen.get(&CardIdx(0)),
        (!stopped).then_some(&1)
    );
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.battlefield[0].uid, 1);
    assert_library_rng(&game.library_rng, &random);
    assert_eq!(log.life_paid, 1);
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(log.card_first_battlefield.get(&0), (!stopped).then_some(&1));
    let linked_gain = if text.contains("Gain three life") {
        3
    } else {
        0
    };
    let parent_gain = if stopped { 0 } else { 7 };
    assert_eq!(game.life_gained, linked_gain + parent_gain);
    assert_eq!(
        game.life,
        life - 1 - i32::from(text.contains("lose 1 life")) + (linked_gain + parent_gain) as i32
    );
    if child_type == "Artifact" {
        assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
        assert_eq!(game.battlefield[1].uid, 2);
        assert_eq!(game.battlefield_seen.get(&CardIdx(1)), Some(&1));
        assert!(game.graveyard.is_empty());
    } else {
        assert_eq!(game.graveyard, [CardIdx(1)]);
        assert_eq!(game.graveyard_seen.get(&CardIdx(1)), Some(&1));
    }
    assert_child_outcome_report(&cards, &game, &log, linked_gain);
}

/// Report a child's actual loss or work-limit boundary without supplying observations.
fn assert_child_outcome_report(cards: &SimDeck, game: &GameState, log: &GameLog, linked_gain: u32) {
    let recursive = matches!(log.completion, GameCompletion::Incomplete { .. });
    let stopped = !matches!(log.completion, GameCompletion::HorizonReached);
    let failed = game.failed_draw;
    let lethal = game.life == 0;
    let report = report_log(cards, log, 1);
    assert_eq!(report.outcomes.requested_games, 1);
    assert_eq!(report.outcomes.finished_games, u32::from(!recursive));
    assert_eq!(report.outcomes.incomplete_games, u32::from(recursive));
    assert_eq!(
        report.outcomes.player_loss_games,
        u32::from(failed || lethal)
    );
    if recursive {
        assert_eq!(game.ring_tempts, 1024);
        assert_eq!(
            log.completion,
            GameCompletion::Incomplete {
                reason: IncompleteReason::TriggerSettlement {
                    configured_limit: 1024,
                    remaining_work: 1
                },
                turn: 1,
                phase: GamePhase::Main,
            }
        );
        assert_eq!(report.outcomes.incomplete_observations.len(), 1);
        let observation = &report.outcomes.incomplete_observations[0];
        assert_eq!(
            observation.reason,
            IncompleteReason::TriggerSettlement {
                configured_limit: 1024,
                remaining_work: 1
            }
        );
        assert_eq!(observation.phase, GamePhase::Main);
        assert_eq!(observation.turn, 1);
        assert_eq!(observation.games, 1);
        assert_eq!(report.draw.avg_life_paid, None);
        assert_eq!(report.draw.avg_life_gained, None);
    } else if stopped {
        assert_eq!(
            log.completion,
            GameCompletion::PlayerLost {
                reasons: vec![if failed {
                    PlayerLossReason::FailedDraw
                } else {
                    PlayerLossReason::ZeroLife
                }],
                turn: 1,
                phase: GamePhase::Main,
            }
        );
        assert_eq!(report.draw.avg_life_paid, Some(1.0));
        assert_eq!(report.draw.avg_life_gained, Some(f64::from(linked_gain)));
    } else {
        assert_finished_turn(&report, log);
        assert_eq!(
            report.draw.avg_life_gained,
            Some(f64::from(linked_gain + 7))
        );
        assert_eq!(log.repeatable_sources_online, [1]);
    }
    assert_eq!(
        report.phase_observations.main_by_turn[&1],
        f64::from(!stopped)
    );
    if stopped {
        assert!(report.creatures_by_turn.is_empty());
        assert!(report.mana.avg_cast_spent_by_turn.is_empty());
        assert_eq!(report.interaction.ready_turn_observations, 0);
        assert_eq!(report.interaction.mana_held_avg, None);
        let estimate =
            &report.sampling["interaction.percent_games_with_ready_interaction_by_turn.1"];
        assert_eq!(estimate.observed_games, u32::from(!recursive));
        assert_eq!(estimate.event_games, (!recursive).then_some(0));
    }
}

/// Bind existing order/nonchaining evidence, then execute missing boundary reports.
pub(super) fn assert_cascade_boundaries_report() {
    let source = named_row("Maelstrom Wanderer");
    pinned_revision(&source);
    crate::deck::simulator::card_library_claim_tests::assert_wanderer_one_cascade_subset_report(
        &source,
    );
    assert_free_x_report();
    assert_nonchaining_report();
    for (kind, text, life, remaining) in [
        (
            "Artifact",
            "When this artifact enters, draw a card.",
            20,
            false,
        ),
        (
            "Artifact",
            "When this artifact enters, draw a card.",
            20,
            true,
        ),
        ("Sorcery", "Draw a card. Gain three life.", 20, false),
        ("Sorcery", "Draw a card. Gain three life.", 20, true),
        ("Sorcery", "You lose 1 life.", 2, false),
        ("Sorcery", "You lose 1 life. Gain three life.", 2, false),
        (
            "Artifact",
            "When this artifact enters, the Ring tempts you.\nWhenever the Ring tempts you, the Ring tempts you.",
            20,
            false,
        ),
    ] {
        assert_child_stop_report(kind, text, life, remaining);
    }
}
