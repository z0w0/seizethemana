//! Synthetic permanent events and payments use executed production census data.
//! Blink evidence covers only the declared deferred entry subset, not a zone change.
//! Loyalty checks cover stored payments, not zero-loyalty state-based destruction.
//! Ultimate affordability remains covered by the earlier core packet, not counted here.
//! The compound boundary uses three printed chapters. A separate two-chapter
//! count defect is not accepted as correct final-chapter cleanup by this packet.

use crate::deck::simulator::card_test_support::{assert_library_rng, main_phase_state};
use crate::deck::simulator::game::{CardRef, GameCompletion, GamePhase, ManaPool};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_effects::activation::{pick_best_activation, resolve_activation};
use crate::deck::simulator::game_run::{TurnCensus, census::finish_log, run_turn};
use crate::deck::simulator::library_core_report_tests::{executed_report, put_permanents};
use crate::deck::simulator::model::{CardIdx, LifeLossScope, SimEffect};
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// A modeled self-blink re-fires once while retaining its physical object and counters.
pub(super) fn assert_blink_subset_report() {
    let mut source = row(
        "Synthetic blink host",
        "{0}",
        "Creature — Wizard",
        "This creature enters with two +1/+1 counters on it.\nWhen this creature enters, draw a card.\nWhen this creature enters, exile it, then return it to the battlefield.",
    );
    source.power = Some("2".into());
    source.toughness = Some("3".into());
    let mut rows = vec![source];
    rows.extend((1..=8).map(|index| row(&format!("Blink draw {index}"), "{99}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut game = state(vec![0], (1..=8).collect());
    let rng = game.library_rng.clone();
    let mut census = TurnCensus::new(3, cards.cards.len());
    let mut sources = vec![];
    for turn in 1..=3 {
        run_turn(
            &cards,
            &mut game,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut sources,
            turn,
            false,
        );
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(game.battlefield[0].entered_turn, 1);
        assert_eq!(game.battlefield[0].counters.plus1, 2);
        assert_eq!(game.battlefield[0].returned_trigger_pending, turn == 1);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
    }
    assert_eq!(game.next_uid, 1);
    assert_eq!(game.hand, [CardIdx(8), CardIdx(7), CardIdx(6), CardIdx(5)]);
    assert_eq!(
        game.library,
        [CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4)]
    );
    assert_eq!(game.life, 20);
    assert_library_rng(&game.library_rng, &rng);
    let log = finish_log(&cards, game, census, 0, false, 3);
    assert_eq!(log.cards_seen, [1, 3, 4]);
    assert_eq!(log.attack_power, [0, 4, 4]);
    let report = report_log(&cards, &log, 3);
    assert_eq!(report.outcomes.finished_games, 1);
    assert_eq!(report.velocity.avg_cards_seen[&3], 4.0);
    assert_eq!(report.graveyard.avg_size_by_turn[&3], 0.0);
    assert_eq!(report.combat.attack_power_avg_by_turn[&2], 4.0);
    assert_ordinary_entry_search_report();
}

/// An ordinary entry search executes once and never sets the deferred entry flag.
fn assert_ordinary_entry_search_report() {
    let mut source = row(
        "Synthetic entry search",
        "{0}",
        "Creature",
        "When this creature enters, you may search your library for a basic land card, put that card onto the battlefield tapped, then shuffle.",
    );
    source.power = Some("2".into());
    let mut rows = vec![source, row("Wastes", "", "Basic Land", "{T}: Add {C}.")];
    rows.extend((2..=8).map(|index| row(&format!("Search draw {index}"), "{99}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut game = state(vec![0], (1..=8).collect());
    let mut census = TurnCensus::new(3, cards.cards.len());
    let mut sources = vec![];
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &CommanderProfile::new(&cards),
        &mut sources,
        1,
        false,
    );
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
    assert!(game.battlefield[1].tapped);
    assert_eq!(game.seen, 1);
    assert!(game.hand.is_empty());
    assert_eq!(game.library.len(), 7);
    assert!(!game.battlefield[0].returned_trigger_pending);
    let rng = game.library_rng.clone();
    for turn in 2..=3 {
        let drawn = *game.library.last().expect("the scheduled draw has a card");
        run_turn(
            &cards,
            &mut game,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut sources,
            turn,
            false,
        );
        assert_library_rng(&game.library_rng, &rng);
        assert!(!game.battlefield[0].returned_trigger_pending);
        assert_eq!(game.hand.last(), Some(&drawn));
    }
    assert_eq!(
        game.battlefield
            .iter()
            .map(|permanent| permanent.uid)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(game.next_uid, 2);
    assert_eq!(game.hand.len(), 2);
    assert_eq!(game.library.len(), 5);
    assert_eq!(game.battlefield_seen[&CardIdx(1)], 1);
    assert!(game.graveyard.is_empty() && game.exile.is_empty());
    let log = finish_log(&cards, game, census, 0, false, 3);
    assert_eq!(log.cards_seen, [1, 2, 3]);
    assert_eq!(log.land_drops, [0, 0, 0]);
    let report = report_log(&cards, &log, 3);
    assert_eq!(report.outcomes.finished_games, 1);
    assert_eq!(report.velocity.avg_cards_seen[&3], 3.0);
    assert_eq!(report.mana.avg_available_by_turn[&2], 1.0);
    assert_eq!(report.creatures_by_turn[&3], 1.0);
}

/// Stored loyalty rejects unfunded minus costs and cannot pay twice in one turn.
pub(super) fn assert_loyalty_rejection_report() {
    for initial in [0, 1, 2, 4] {
        let mut walker = row(
            "Synthetic minus source",
            "{0}",
            "Planeswalker",
            "-2: Draw a card.",
        );
        walker.loyalty = Some(initial.to_string());
        let mut rows = vec![walker];
        rows.extend(
            (1..=6).map(|index| row(&format!("Loyalty draw {index}"), "{99}", "Sorcery", "")),
        );
        let cards = deck(&rows);
        let mut game = main_phase_state(vec![], (1..=6).collect());
        put_permanents(&cards, &mut game, &[0]);
        assert_eq!(game.battlefield[0].counters.loyalty, initial);
        let mut action = game.clone();
        let mut pool = ManaPool::default();
        let chosen = pick_best_activation(&cards, &action, &pool, 1);
        assert_eq!(chosen.is_some(), initial >= 2);
        if let Some(activation) = chosen {
            resolve_activation(&cards, &mut action, &mut pool, 1, &activation);
            assert_eq!(action.battlefield[0].uid, 1);
            assert_eq!(action.battlefield[0].counters.loyalty, initial - 2);
            assert!(action.battlefield[0].fired);
            assert_eq!(action.hand, [CardIdx(6)]);
            resolve_activation(&cards, &mut action, &mut pool, 1, &activation);
            assert_eq!(action.hand, [CardIdx(6)]);
        } else {
            assert_eq!(action.battlefield[0].counters.loyalty, initial);
            assert!(!action.battlefield[0].fired);
            assert!(action.hand.is_empty());
            assert_eq!(action.library, game.library);
        }
        assert!(pick_best_activation(&cards, &action, &pool, 1).is_none());
        assert_eq!(pool, ManaPool::default());
        let (game, log, report) = executed_report(&cards, game, 2, vec![]);
        let rewards = if initial >= 4 {
            2
        } else {
            u32::from(initial >= 2)
        };
        assert_eq!(game.battlefield[0].counters.loyalty, initial - rewards * 2);
        assert_eq!(game.seen, 1 + rewards);
        assert_eq!(log.cards_seen[0], u32::from(initial >= 2));
        assert_eq!(
            report["velocity"]["avg_cards_seen"]["2"],
            f64::from(1 + rewards)
        );
        assert_eq!(
            report["win_conditions"]["percent_games_with_affordable_ultimate"],
            0.0
        );
        assert_eq!(game.life, 20);
    }
    assert_zero_loyalty_symbol_report();
}

/// A zero loyalty symbol is still one supported loyalty activation per turn.
fn assert_zero_loyalty_symbol_report() {
    let mut source = row(
        "Synthetic zero-symbol source",
        "{0}",
        "Planeswalker",
        "0: Draw a card.",
    );
    source.loyalty = Some("1".into());
    let mut rows = vec![source];
    rows.extend(
        (1..=5).map(|index| row(&format!("Zero-symbol draw {index}"), "{99}", "Sorcery", "")),
    );
    let cards = deck(&rows);
    let mut game = state(vec![], (1..=5).collect());
    put_permanents(&cards, &mut game, &[0]);
    let (game, log, report) = executed_report(&cards, game, 2, vec![]);
    assert_eq!(game.battlefield[0].uid, 1);
    assert_eq!(game.battlefield[0].counters.loyalty, 1);
    assert!(game.battlefield[0].fired);
    assert!(pick_best_activation(&cards, &game, &ManaPool::default(), 2).is_none());
    assert_eq!(log.cards_seen, [1, 3]);
    assert_eq!(game.hand, [CardIdx(5), CardIdx(4), CardIdx(3)]);
    assert_eq!(report["velocity"]["avg_cards_seen"]["2"], 3.0);
    assert_eq!(report["outcomes"]["player_loss_games"], 0);
}

/// Linked final-chapter effects finish before loss checks or final-chapter sacrifice.
pub(super) fn assert_compound_saga_report() {
    for gain in [0, 3] {
        let source = row(
            "Synthetic compound Saga",
            "{0}",
            "Enchantment — Saga",
            &format!(
                "Read ahead\nI — Exile target permanent.\nII — Gain 0 life.\nIII — Each player loses 1 life. Gain {gain} life."
            ),
        );
        let cards = deck(&[source.clone(), row("Saga turn draw", "{99}", "Sorcery", "")]);
        assert!(cards.cards[0].read_ahead);
        assert_eq!(cards.cards[0].saga.chapters.len(), 3);
        assert_eq!(cards.cards[0].chapter_count(), 3);
        assert!(cards.cards[0].saga.chapters[0].is_empty());
        assert_eq!(
            cards.cards[0].source_hash,
            crate::deck::simulator::support::card_source_hash(&source)
        );
        let unsupported = cards.cards[0]
            .support
            .iter()
            .find(|ability| ability.source.contains("I — Exile target permanent"))
            .expect("the unsupported chapter retains source evidence");
        let span = unsupported
            .span
            .expect("the printed chapter has a source span");
        assert_eq!(unsupported.source, source.oracle_text[span.start..span.end]);
        assert!(matches!(
            unsupported.status,
            crate::deck::simulator::support::AbilitySupportStatus::Unsupported { .. }
        ));
        assert!(matches!(
            cards.cards[0].saga.chapters[1].as_slice(),
            [SimEffect::GainLife(0)]
        ));
        assert!(matches!(cards.cards[0].saga.chapters[2].as_slice(),
            [SimEffect::LoseLife { amount: 1, scope: LifeLossScope::EachPlayer }, SimEffect::GainLife(amount)] if *amount == gain));
        let mut game = state(vec![0], vec![1]);
        game.life = 1;
        let (game, log) = execute_turns(&cards, game, 2, vec![]);
        assert_eq!(game.life, gain as i32);
        assert_eq!(game.life_gained, gain);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert!(game.library.is_empty() && game.exile.is_empty());
        assert_eq!(game.opponent_life_lost, 1);
        assert_eq!(log.opponent_life_loss, [0, 1]);
        let report = report_log(&cards, &log, 2);
        assert_eq!(report.outcomes.finished_games, 1);
        assert_eq!(report.outcomes.player_loss_games, u32::from(gain == 0));
        assert_eq!(report.outcomes.incomplete_games, 0);
        assert_eq!(report.draw.avg_life_gained, Some(f64::from(gain)));
        assert_eq!(report.phase_observations.draw_by_turn[&2], 1.0);
        assert_eq!(
            report.phase_observations.main_by_turn[&2],
            if gain == 0 { 0.0 } else { 1.0 }
        );
        assert_eq!(
            report.phase_observations.end_turn_by_turn[&2],
            if gain == 0 { 0.0 } else { 1.0 }
        );
        if gain == 0 {
            assert!(matches!(
                log.completion,
                GameCompletion::PlayerLost {
                    turn: 2,
                    phase: GamePhase::Main,
                    ..
                }
            ));
            assert_eq!(game.battlefield[0].uid, 1);
            assert_eq!(game.battlefield[0].saga_step, 3);
            assert!(game.graveyard.is_empty());
            assert!(!log.turn_observations[1].main && !log.turn_observations[1].end_turn);
            assert!(!report.creatures_by_turn.contains_key(&2));
            assert!(
                !report
                    .win_conditions
                    .opponent_life_loss_by_turn
                    .contains_key(&2)
            );
            assert!(!report.graveyard.avg_size_by_turn.contains_key(&2));
            assert!(!report.velocity.avg_cards_seen.contains_key(&2));
        } else {
            assert!(matches!(log.completion, GameCompletion::HorizonReached));
            assert!(game.battlefield.is_empty());
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(log.card_first_graveyard[&0], 2);
            assert_eq!(report.graveyard.avg_size_by_turn[&2], 1.0);
            assert_eq!(report.velocity.avg_cards_seen[&2], 1.0);
            assert_eq!(report.win_conditions.opponent_life_loss_by_turn[&2], 1.0);
        }
    }
}

/// The earlier legendary copy survives; distinct names and nonlegends keep both bodies.
pub(super) fn assert_legend_rule_report() {
    for (legendary, distinct) in [(true, false), (true, true), (false, false)] {
        let kind = if legendary {
            "Legendary Creature"
        } else {
            "Creature"
        };
        let mut earlier = row(
            "Earlier legend control",
            "{0}",
            kind,
            "This creature enters with two +1/+1 counters on it.",
        );
        earlier.power = Some("3".into());
        earlier.toughness = Some("3".into());
        let mut later = earlier.clone();
        if distinct {
            later.name = "Distinct legend control".into();
        }
        let cards = deck(&[
            earlier,
            later,
            row("Legend turn draw", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![1], vec![2]);
        put_permanents(&cards, &mut game, &[0]);
        let (game, log, report) = executed_report(&cards, game, 2, vec![]);
        let duplicate = legendary && !distinct;
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(game.battlefield[0].counters.plus1, 2);
        assert_eq!(game.battlefield[0].entered_turn, 0);
        assert_eq!(game.battlefield.len(), if duplicate { 1 } else { 2 });
        assert_eq!(
            game.graveyard,
            if duplicate { vec![CardIdx(1)] } else { vec![] }
        );
        assert_eq!(game.hand, [CardIdx(2)]);
        assert_eq!(game.next_uid, 2);
        if !duplicate {
            assert_eq!(game.battlefield[1].uid, 2);
            assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
            assert_eq!(game.battlefield[1].counters.plus1, 2);
        }
        assert_eq!(
            log.attack_power,
            if duplicate { vec![5, 5] } else { vec![5, 10] }
        );
        assert_eq!(
            report["creatures_by_turn"]["2"],
            if duplicate { 1.0 } else { 2.0 }
        );
        assert_eq!(
            report["graveyard"]["avg_size_by_turn"]["2"],
            if duplicate { 1.0 } else { 0.0 }
        );
        assert_eq!(
            report["combat"]["attack_power_avg_by_turn"]["2"],
            if duplicate { 5.0 } else { 10.0 }
        );
        if duplicate {
            assert_eq!(log.card_first_graveyard[&1], 1);
        }
        assert_eq!(game.life, 20);
    }
}
