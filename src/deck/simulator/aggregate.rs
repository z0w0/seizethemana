//! Ordered streaming reduction of game measurements into report statistics.

use super::game::GameLog;
use super::model::SimDeck;

mod accumulator;
/// Exact whole-game event counts used by report intervals.
mod events;
mod stats;

pub use accumulator::SimAccumulator;
pub use stats::{CardCast, PhaseObservationCounts, SimStats};

/// Reduce a borrowed set of game logs through the same streaming path as batches.
pub fn aggregate(logs: &[GameLog], deck: &SimDeck, turns: u32) -> SimStats {
    let mut accumulator = SimAccumulator::new(deck, turns);
    for log in logs {
        accumulator.push(log);
    }
    accumulator.finish()
}

/// Sort scalar observations once and read their nearest-rank percentile.
pub(super) fn percentile(values: &mut [u32], q: f64) -> u32 {
    values.sort_unstable();
    sorted_percentile(values, q)
}

/// Read a nearest-rank percentile from ascending samples; empty internal sets use zero.
fn sorted_percentile(values: &[u32], q: f64) -> u32 {
    if values.is_empty() {
        return 0;
    }
    let rank = ((q * values.len() as f64).ceil() as usize).clamp(1, values.len());
    values[rank - 1]
}

/// Completion, observation, and optional-analysis contract regressions.
#[cfg(test)]
#[path = "tests/outcome_report_tests.rs"]
mod outcome_report_tests;
