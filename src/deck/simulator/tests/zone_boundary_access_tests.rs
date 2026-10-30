//! Terminal first-zone visits, public access rates, and incomplete denominators.

use crate::deck::simulator::aggregate::aggregate;
use crate::deck::simulator::card_test_support::assert_library_rng;
use crate::deck::simulator::game::{
    CardRef, GameCompletion, GameState, IncompleteReason, record_card_seen,
};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::pair_access::PairAccumulator;
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::report::json_report;
use crate::deck::simulator::report_schema::{ObservationStatus, PairAccessReport};
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// Keep hand entries, graveyard-only cards, and an ordinary land entry distinct.
fn terminal_cards() -> SimDeck {
    let mut rows = vec![row(
        "Terminal zone spell",
        "{0}",
        "Sorcery",
        "Draw two cards. Mill three cards. You lose 1 life.",
    )];
    rows.extend((1..=6).map(|index| row(&format!("Terminal zone land {index}"), "", "Land", "")));
    rows.push(row("Incomplete zone spell", "{0}", "Sorcery", "Draw two cards. Mill three cards. Create nine 1/1 white Soldier creature tokens. You lose 1 life."));
    deck(&rows)
}

/// Give a controlled kept opener the same exposure records as production setup.
fn kept_hand_state(hand: Vec<usize>, library: Vec<usize>) -> GameState {
    let mut game = state(hand, library);
    game.seen = game.hand.len() as u32;
    game.awareness_cards = game.seen;
    for index in game.hand.clone() {
        record_card_seen(&mut game, index, 1);
    }
    game
}

/// Actual terminal moves survive without filling census rows or claiming simultaneous access.
pub(super) fn assert_terminal_first_visits_report() {
    let cards = terminal_cards();
    let mut game = kept_hand_state(vec![0, 1], vec![2, 3, 4, 5, 6]);
    game.life = 1;
    let rng = game.library_rng.clone();
    let (game, lost) = execute_turns(&cards, game, 4, vec![]);
    assert!(matches!(
        game.completion,
        Some(GameCompletion::PlayerLost { turn: 1, .. })
    ));
    assert_eq!(game.life, 0);
    assert_eq!(game.hand, [CardIdx(6), CardIdx(5)]);
    assert!(game.library.is_empty());
    assert_eq!(
        game.graveyard,
        [CardIdx(4), CardIdx(3), CardIdx(2), CardIdx(0)]
    );
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(
        lost.card_first_seen,
        [(0, 1), (1, 1), (5, 1), (6, 1)].into_iter().collect()
    );
    assert_eq!(lost.card_first_battlefield, [(1, 1)].into_iter().collect());
    assert_eq!(
        lost.card_first_graveyard,
        [(0, 1), (2, 1), (3, 1), (4, 1)].into_iter().collect()
    );
    assert_eq!(game.graveyard_generations.len(), 4);
    assert!(
        game.graveyard_generations
            .values()
            .all(|generation| *generation == 1)
    );
    assert_eq!(lost.turns_observed, 0);
    assert!(lost.turn_observations.iter().all(|turn| !turn.end_turn));
    assert_eq!(lost.cards_seen, [0; 4]);
    assert_eq!(lost.cards_seen_by_4, 7);
    assert_eq!(lost.lands_seen_by_turn_4, 6);
    let report = report_log(&cards, &lost, 4);
    assert_eq!(report.outcomes.player_loss_games, 1);
    assert!(report.velocity.avg_cards_seen.is_empty());
    assert!(report.velocity.self_milled_by_turn.is_empty());
    assert!(report.graveyard.avg_size_by_turn.is_empty());
    assert_eq!(
        report
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4
            .expect("terminal fixed-window success")
            .value(),
        100.0
    );
    assert_eq!(
        report.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"].event_games,
        Some(1)
    );
    assert_library_rng(&game.library_rng, &rng);

    let pairs = [
        (cards.cards[5].name.clone(), cards.cards[6].name.clone()),
        (cards.cards[2].name.clone(), cards.cards[6].name.clone()),
        (cards.cards[0].name.clone(), cards.cards[6].name.clone()),
    ];
    let mut access = PairAccumulator::new(&cards, &pairs, 4);
    access.push(&lost);
    let rows = access.finish();
    for (row, expected) in rows.iter().zip([1, 0, 1]) {
        let public = PairAccessReport::from_access(row, 1, 4);
        assert_eq!(public.access.event_games, Some(expected));
        assert_eq!(public.access.observed_games, 1);
        assert_eq!(
            public
                .percent_of_games
                .expect("finished loss is observed")
                .value(),
            f64::from(expected) * 100.0
        );
    }
    assert_terminal_entry_visit();
    assert_incomplete_access_denominators(&cards, &lost);
}

/// A creature enters and draws before its own lethal entry effect stops the phase.
fn assert_terminal_entry_visit() {
    let cards = deck(&[
        row(
            "Terminal entering creature",
            "{0}",
            "Creature",
            "When this creature enters, draw a card. You lose 1 life.",
        ),
        row("Terminal entry draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = kept_hand_state(vec![0], vec![1]);
    game.life = 1;
    let (game, log) = execute_turns(&cards, game, 4, vec![]);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(log.card_first_seen.get(&1), Some(&1));
    assert_eq!(log.card_first_battlefield.get(&0), Some(&1));
    assert!(log.card_first_graveyard.is_empty());
    assert_eq!(log.turns_observed, 0);
    assert!(matches!(
        game.completion,
        Some(GameCompletion::PlayerLost { turn: 1, .. })
    ));
    let report = report_log(&cards, &log, 4);
    assert_eq!(report.outcomes.finished_games, 1);
    assert_eq!(report.outcomes.player_loss_games, 1);
    assert!(report.creatures_by_turn.is_empty());
    assert!(report.mana.avg_available_by_turn.is_empty());
}

/// Incomplete physical access stays out of primary rates, including a mixed finished sample.
fn assert_incomplete_access_denominators(
    cards: &SimDeck,
    lost: &crate::deck::simulator::game::GameLog,
) {
    let mut game = kept_hand_state(vec![7, 1], vec![2, 3, 4, 5, 6]);
    game.life = 1;
    let (game, capped) = execute_turns(cards, game, 4, vec![]);
    assert!(matches!(
        game.completion,
        Some(GameCompletion::Incomplete {
            reason: IncompleteReason::EffectWork {
                configured_limit: 8,
                remaining_work: 1
            },
            turn: 1,
            ..
        })
    ));
    assert_eq!(game.life, 1);
    for index in [1, 5, 6] {
        assert_eq!(
            capped.card_first_seen.get(&index),
            lost.card_first_seen.get(&index)
        );
    }
    assert_eq!(capped.card_first_seen.get(&7), Some(&1));
    assert!(!capped.card_first_seen.contains_key(&0));
    assert_eq!(capped.card_first_battlefield, lost.card_first_battlefield);
    for index in [2, 3, 4] {
        assert_eq!(capped.card_first_graveyard.get(&index), Some(&1));
    }
    assert_eq!(capped.lands_seen_by_turn_4, 6);
    let report = report_log(cards, &capped, 4);
    let metric = "land_drops.percent_games_with_six_or_more_lands_by_turn_4";
    assert_eq!(
        report.sampling[metric].status,
        ObservationStatus::NoObservations
    );
    assert_eq!(report.sampling[metric].observed_games, 0);
    assert!(
        report
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4
            .is_none()
    );
    assert!(report.velocity.avg_cards_seen.is_empty());
    let stats = aggregate(&[lost.clone(), capped.clone()], cards, 4);
    let mixed = json_report(
        &stats,
        cards,
        "Terminal access sample",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert_eq!(mixed.outcomes.requested_games, 2);
    assert_eq!(mixed.outcomes.finished_games, 1);
    assert_eq!(mixed.outcomes.incomplete_games, 1);
    assert_eq!(mixed.sampling[metric].event_games, Some(1));
    assert_eq!(mixed.sampling[metric].observed_games, 1);
    let pairs = [(cards.cards[5].name.clone(), cards.cards[6].name.clone())];
    let mut access = PairAccumulator::new(cards, &pairs, 4);
    access.push(&capped);
    let rows = access.finish();
    let public = PairAccessReport::from_access(&rows[0], 1, 4);
    assert_eq!(public.access.status, ObservationStatus::NoObservations);
    assert_eq!(public.access.observed_games, 0);
    assert!(public.percent_of_games.is_none());
}
