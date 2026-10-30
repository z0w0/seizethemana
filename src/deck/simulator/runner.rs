//! Ordered execution of independent games with at most one batch in flight.

use std::num::NonZeroUsize;

use rayon::prelude::*;

use super::game::GameLog;
use super::game_run::run_game_with_config;
use super::model::SimDeck;
use super::opponents::OpponentConfig;
use super::rng::{RngDomain, game_rng};

/// Execution controls shared by command runs, previews, and benchmark callers.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Limit this run's Rayon pool. `None` uses the process-wide Rayon pool.
    pub workers: Option<NonZeroUsize>,
    /// Games per delivered batch. Analyses can retain logs from earlier batches.
    pub batch_size: NonZeroUsize,
    /// Override the format's opponent scenario. `None` uses its default.
    pub opponents: Option<OpponentConfig>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            workers: None,
            batch_size: NonZeroUsize::new(256).expect("default batch size is nonzero"),
            opponents: None,
        }
    }
}

/// A reusable worker pool and batch bound; all game state stays in worker calls.
pub(crate) struct BatchRunner {
    config: RunConfig,
    pool: Option<rayon::ThreadPool>,
}

/// Avoid parallel scheduling overhead for small batches.
const MIN_PARALLEL_GAMES: usize = 32;

/// Reject empty workloads before allocating any horizon-indexed statistics.
pub(crate) fn validate_workload(runs: u32, turns: u32) -> anyhow::Result<()> {
    anyhow::ensure!(runs > 0, "simulation requires at least one game");
    anyhow::ensure!(turns > 0, "simulation requires at least one turn");
    Ok(())
}

impl BatchRunner {
    /// Build a local pool only when the caller sets a multi-worker limit.
    pub(crate) fn new(config: RunConfig) -> anyhow::Result<Self> {
        if let Some(opponents) = &config.opponents {
            opponents.validate()?;
        }
        let pool = match config.workers {
            Some(workers) if workers.get() > 1 => Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(workers.get())
                    .build()?,
            ),
            _ => None,
        };
        Ok(Self { config, pool })
    }

    /// Consume batches in ascending game-ID order, with no database or output work.
    ///
    /// This method keeps at most `batch_size` full logs. A consumer that retains
    /// batches must account for its own memory. Zero games or turns is an error.
    pub(crate) fn for_each_batch(
        &self,
        deck: &SimDeck,
        runs: u32,
        turns: u32,
        seed: u64,
        mut consume: impl FnMut(u32, Vec<GameLog>),
    ) -> anyhow::Result<()> {
        validate_workload(runs, turns)?;
        if let Some(colors) = self
            .config
            .opponents
            .as_ref()
            .and_then(|config| config.land_colors.as_ref())
        {
            anyhow::ensure!(
                colors.len() == deck.format.life_loss_mult() as usize,
                "opponent land_colors must declare one entry per seat"
            );
        }
        let batch_size = self.config.batch_size.get();
        for start in (0..runs as usize).step_by(batch_size) {
            let end = start.saturating_add(batch_size).min(runs as usize);
            let play = |id: usize| {
                let mut rng = game_rng(seed, id as u64, RngDomain::Opening);
                run_game_with_config(
                    deck,
                    &mut rng,
                    turns,
                    self.config.opponents.as_ref(),
                    game_rng(seed, id as u64, RngDomain::Library),
                )
            };
            let logs = if self.config.workers.is_some_and(|n| n.get() == 1)
                || end - start < MIN_PARALLEL_GAMES
            {
                (start..end).map(play).collect()
            } else {
                let parallel = || (start..end).into_par_iter().map(play).collect();
                match &self.pool {
                    Some(pool) => pool.install(parallel),
                    None => parallel(),
                }
            };
            consume(start as u32, logs);
        }
        Ok(())
    }

    /// Collect ordered logs for analyses that still require the full run in memory.
    pub(super) fn collect(
        &self,
        deck: &SimDeck,
        runs: u32,
        turns: u32,
        seed: u64,
    ) -> anyhow::Result<Vec<GameLog>> {
        let mut logs = Vec::new();
        self.for_each_batch(deck, runs, turns, seed, |_, batch| logs.extend(batch))?;
        Ok(logs)
    }
}

#[cfg(test)]
#[path = "tests/runner_tests.rs"]
mod runner_tests;
