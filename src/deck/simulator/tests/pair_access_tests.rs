use super::*;
use crate::deck::simulator::{
    rng::{RngDomain, game_rng},
    turn_loop_tests::{deck, row},
};

/// Access through any physical copy counts, and repeated/reversed queries merge.
#[test]
fn duplicate_copies_and_queries_have_one_correct_access_row() {
    let first = row("First", "{1}", "Artifact", "");
    let second = row("Second", "{1}", "Artifact", "");
    let cards = deck(&[first.clone(), second.clone(), first, second]);
    let pairs = vec![
        ("First".into(), "Second".into()),
        ("second".into(), " first ".into()),
    ];
    let mut accumulator = PairAccumulator::new(&cards, &pairs, 1);
    let mut log =
        crate::deck::simulator::game::run_game(&cards, &mut game_rng(42, 0, RngDomain::Opening), 1);
    log.card_first_seen.clear();
    log.card_first_seen.insert(2, 1);
    log.card_first_seen.insert(3, 1);
    accumulator.push(&log);
    let rows = accumulator.finish();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].game_share, 1.0);
    assert_eq!(rows[0].pair, "First + Second");
}

/// A requested deadline beyond the horizon stays beyond the horizon.
#[test]
fn target_turn_is_not_clamped_to_observed_turns() {
    let cards = deck(&[
        row("First", "{6}", "Artifact", ""),
        row("Second", "{2}", "Artifact", ""),
    ]);
    let rows = PairAccumulator::new(&cards, &[("First".into(), "Second".into())], 2).finish();
    assert_eq!(rows[0].target_turn, 6);
    let reported =
        crate::deck::simulator::report_schema::PairAccessReport::from_access(&rows[0], 10, 2);
    assert!(reported.percent_of_games.is_none());
    assert_eq!(
        reported.access.status,
        crate::deck::simulator::report_schema::ObservationStatus::OutsideHorizon
    );
}

/// Benchmark batches use the same normalized queries and deadlines as streamed runs.
#[test]
fn benchmark_pair_wrapper_uses_the_streaming_predicate() {
    let cards = deck(&[
        row("First", "{6}", "Artifact", ""),
        row("Second", "{2}", "Artifact", ""),
    ]);
    let pairs = vec![
        ("First".to_string(), "Second".to_string()),
        ("second".to_string(), " first ".to_string()),
    ];
    let rows = crate::deck::simulator::findings::piece_pair_access(&[], &cards, &pairs, 2);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].target_turn, 6);
    assert_eq!(rows[0].pair, "First + Second");
}
