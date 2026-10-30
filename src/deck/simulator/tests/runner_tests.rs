//! Worker, batch, prefix, and opener contracts for independent simulation games.

use super::super::deck_test_support::{fixture_deck, fixture_map};
use super::*;

/// The public report entry point returns errors for empty workloads, not panics.
#[test]
fn report_api_rejects_empty_workloads_before_aggregation() {
    let source = fixture_deck("mono-red-aggro");
    let cards = fixture_map("mono-red-aggro");
    for (runs, turns) in [(0, 2), (1, 0)] {
        assert!(
            super::super::sim_report_for_with_config(
                &source,
                &cards,
                "test",
                runs,
                Some(turns),
                42,
                None,
                RunConfig::default()
            )
            .is_err()
        );
    }
}

/// Create a small explicit pool and a chosen batch bound.
fn config(workers: usize, batch_size: usize) -> RunConfig {
    RunConfig {
        workers: Some(NonZeroUsize::new(workers).expect("test worker count is nonzero")),
        batch_size: NonZeroUsize::new(batch_size).expect("test batch size is nonzero"),
        opponents: None,
    }
}

/// Compile a real constructed list so tests include mulligans and card actions.
fn deck() -> SimDeck {
    super::super::deck::build_sim_deck(
        &fixture_deck("mono-red-aggro"),
        &fixture_map("mono-red-aggro"),
        Some("standard"),
    )
}

/// Compare every log field without depending on randomized map iteration order.
fn assert_log_eq(left: &GameLog, right: &GameLog) {
    assert_eq!(left, right);
}

/// Pool limits and batch boundaries must not change any game's result.
#[test]
fn games_match_across_workers_and_batch_sizes() {
    let deck = deck();
    let expected = BatchRunner::new(config(1, 17))
        .expect("serial runner")
        .collect(&deck, 81, 8, 42)
        .expect("valid workload");
    for execution in [
        config(2, 40),
        config(4, 64),
        config(2, 1),
        config(1, 256),
        RunConfig::default(),
    ] {
        let actual = BatchRunner::new(execution)
            .expect("bounded runner")
            .collect(&deck, 81, 8, 42)
            .expect("valid workload");
        assert_eq!(actual.len(), expected.len());
        for (left, right) in expected.iter().zip(&actual) {
            assert_log_eq(left, right);
        }
    }
}

/// Extending a run keeps every existing game unchanged.
#[test]
fn longer_runs_preserve_game_prefixes() {
    let deck = deck();
    let short = BatchRunner::new(config(1, 9))
        .expect("serial runner")
        .collect(&deck, 13, 8, 99)
        .expect("short workload");
    let long = BatchRunner::new(config(3, 37))
        .expect("parallel runner")
        .collect(&deck, 75, 8, 99)
        .expect("long workload");
    for (left, right) in short.iter().zip(&long) {
        assert_log_eq(left, right);
    }
}

/// Consumers receive stable IDs and never more full logs than the chosen bound.
#[test]
fn batches_are_bounded_and_ordered() {
    let deck = deck();
    let runner = BatchRunner::new(config(2, 32)).expect("bounded runner");
    let mut received = 0;
    let mut sizes = Vec::new();
    runner
        .for_each_batch(&deck, 73, 4, 42, |first_id, logs| {
            assert_eq!(first_id, received);
            assert!(logs.len() <= 32);
            for (offset, log) in logs.iter().enumerate() {
                let mut rng = game_rng(42, u64::from(first_id) + offset as u64, RngDomain::Opening);
                let expected = run_game_with_config(
                    &deck,
                    &mut rng,
                    4,
                    None,
                    game_rng(42, u64::from(first_id) + offset as u64, RngDomain::Library),
                );
                assert_log_eq(log, &expected);
            }
            received += logs.len() as u32;
            sizes.push(logs.len());
        })
        .expect("valid workload");
    assert_eq!(received, 73);
    assert_eq!(sizes, [32, 32, 9]);
}

/// Invalid workloads must not invoke a consumer or produce partial reports.
#[test]
fn empty_workloads_are_errors() {
    let deck = deck();
    let runner = BatchRunner::new(config(1, 32)).expect("serial runner");
    for (runs, turns) in [(0, 8), (1, 0)] {
        let result = runner.for_each_batch(&deck, runs, turns, 42, |_, _| {
            panic!("invalid workload reached its consumer");
        });
        assert!(result.is_err());
    }
}

/// Hand sampling and game zero must use the exact same opening stream.
#[test]
fn first_game_uses_shared_opener_contract() {
    let deck = deck();
    let mut rng = game_rng(99, 0, RngDomain::Opening);
    let opener = super::super::deal::deal_opener(&deck, &mut rng);
    let logs = BatchRunner::new(config(1, 1))
        .expect("serial runner")
        .collect(&deck, 1, 4, 99)
        .expect("one game");
    assert_eq!(logs[0].opener_lands, opener.lands);
    assert_eq!(logs[0].mulliganed, opener.mulliganed);
    for index in opener.hand {
        assert_eq!(logs[0].card_first_seen.get(&index.index()), Some(&1));
    }
}

/// Canonical report values must match after worker and batch configuration changes.
#[test]
fn reports_match_across_workers_and_batches() {
    let deck = fixture_deck("mono-red-aggro");
    let cards = fixture_map("mono-red-aggro");
    let make_report = |config| {
        let report = super::super::sim_report_for_with_config(
            &deck,
            &cards,
            "runner-test",
            81,
            Some(8),
            42,
            Some("standard"),
            config,
        )
        .expect("valid report workload");
        serde_json::to_value(report).expect("report serializes")
    };
    let expected = make_report(config(1, 17));
    assert_eq!(expected, make_report(config(2, 40)));
    assert_eq!(expected, make_report(config(4, 256)));
    assert_eq!(expected, make_report(config(2, 40)));
}
