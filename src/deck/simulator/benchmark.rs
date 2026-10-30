//! Benchmark access to simulator phases without a database or terminal output.

use std::collections::HashMap;

use super::rng::{RngDomain, game_rng};
use super::runner::BatchRunner;
use super::{
    RunConfig, aggregate, apply_role_counts, combos, deal, deck, findings, game, model, report,
    report_schema,
};
use crate::db::CardRow;
use crate::deck::grammar::Deck;

/// Read fixture mana values through the production mana-cost parser.
pub fn mana_value(cost: &str) -> f64 {
    super::oracle_parser::cost::parse_cost(cost).total() as f64
}

/// An immutable compiled deck shared by the measured simulator phases.
pub struct BenchmarkDeck {
    deck: model::SimDeck,
}

/// Retained game logs used to measure analysis without replaying games.
#[derive(Debug, PartialEq)]
pub struct BenchmarkLogs {
    logs: Vec<game::GameLog>,
    turns: u32,
}

/// Aggregate values used to measure report construction separately.
pub struct BenchmarkStats {
    stats: aggregate::SimStats,
}

/// Compiled first-arrival specifications, not claims of executable combos.
pub struct BenchmarkCombos {
    candidates: Vec<combos::ComboCandidate>,
}

/// Own measured combo results without exposing private simulator types.
pub struct BenchmarkComboResults {
    assembly: combos::Assembly,
}

impl BenchmarkComboResults {
    /// Count specifications processed by the measured production analysis.
    pub fn variants_considered(&self) -> usize {
        self.assembly.variants_considered
    }
}

impl BenchmarkDeck {
    /// Compile fixture card data with the same builder as production runs.
    pub fn new(deck: &Deck, cards: &HashMap<String, CardRow>, format: Option<&str>) -> Self {
        Self {
            deck: deck::build_sim_deck(deck, cards, format),
        }
    }

    /// Check the physical card order recorded by a pinned deck snapshot.
    pub fn verify_card_order(
        &self,
        library: &[String],
        commanders: &[String],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.deck.cards.iter().map(|card| &card.name).eq(library),
            "compiled library order differs from the pinned snapshot"
        );
        anyhow::ensure!(
            self.deck
                .commanders
                .iter()
                .map(|card| &card.name)
                .eq(commanders),
            "compiled commander order differs from the pinned snapshot"
        );
        Ok(())
    }

    /// Deal one opening hand without playing its turns.
    pub fn deal(&self, seed: u64) -> impl std::fmt::Debug {
        let mut rng = game_rng(seed, 0, RngDomain::Opening);
        deal::deal_opener(&self.deck, &mut rng)
    }

    /// Execute games without aggregation or report construction.
    pub fn run(&self, runs: u32, turns: u32, seed: u64) -> anyhow::Result<BenchmarkLogs> {
        self.run_with_config(runs, turns, seed, RunConfig::default())
    }

    /// Execute a workload with an explicit worker count, batch bound, or scenario.
    pub fn run_with_config(
        &self,
        runs: u32,
        turns: u32,
        seed: u64,
        config: RunConfig,
    ) -> anyhow::Result<BenchmarkLogs> {
        let logs = BatchRunner::new(config)?.collect(&self.deck, runs, turns, seed)?;
        Ok(BenchmarkLogs { logs, turns })
    }

    /// Aggregate retained logs without playing more games.
    pub fn aggregate(&self, logs: &BenchmarkLogs) -> BenchmarkStats {
        let mut stats = aggregate::aggregate(&logs.logs, &self.deck, logs.turns);
        apply_role_counts(&mut stats, &self.deck);
        BenchmarkStats { stats }
    }

    /// Measure explicit pair access without database candidate loading.
    pub fn pair_access(
        &self,
        logs: &BenchmarkLogs,
        pairs: &[(String, String)],
    ) -> impl std::fmt::Debug {
        findings::piece_pair_access(&logs.logs, &self.deck, pairs, logs.turns)
    }

    /// Prepare synthetic pair specifications outside the measured analysis phase.
    pub fn combo_specs(&self, pairs: &[(String, String)]) -> BenchmarkCombos {
        let candidates = pairs
            .iter()
            .enumerate()
            .map(|(id, (left, right))| {
                let pieces: Vec<_> = [left, right]
                    .into_iter()
                    .map(|name| {
                        let indices: Vec<_> = self
                            .deck
                            .cards
                            .iter()
                            .enumerate()
                            .filter_map(|(index, card)| (&card.name == name).then_some(index))
                            .collect();
                        combos::PieceRef {
                            name: name.clone(),
                            starts_in_command_zone: self
                                .deck
                                .commanders
                                .iter()
                                .any(|card| &card.name == name),
                            starts_in_library: !indices.is_empty(),
                            indices,
                            zones: vec![combos::PieceZone::Hand],
                        }
                    })
                    .collect();
                let missing: Vec<_> = pieces
                    .iter()
                    .filter(|piece| !piece.starts_in_library && !piece.starts_in_command_zone)
                    .map(|piece| piece.name.clone())
                    .collect();
                combos::ComboCandidate {
                    id: format!("benchmark-pair-{id}"),
                    produces: Vec::new(),
                    mana_value_needed: 0,
                    bracket_tag: None,
                    complete: missing.is_empty(),
                    pieces,
                    missing,
                }
            })
            .collect();
        BenchmarkCombos { candidates }
    }

    /// Run production combo-access analysis on retained logs, without store lookups.
    pub fn combo_access(
        &self,
        logs: &BenchmarkLogs,
        specs: &BenchmarkCombos,
    ) -> BenchmarkComboResults {
        let mut accumulator =
            combos::ComboAccumulator::new(&specs.candidates, &self.deck, logs.turns);
        for log in &logs.logs {
            accumulator.push(log);
        }
        let mut assembly = accumulator.finish();
        combos::rank_complete(&mut assembly.complete);
        BenchmarkComboResults { assembly }
    }

    /// Play and aggregate bounded batches, as production does, without retained logs.
    pub fn run_report(
        &self,
        runs: u32,
        turns: u32,
        seed: u64,
        config: RunConfig,
    ) -> anyhow::Result<report_schema::SimReport> {
        let opponents = config.opponents.clone();
        let mut accumulator = aggregate::SimAccumulator::new(&self.deck, turns);
        BatchRunner::new(config)?.for_each_batch(&self.deck, runs, turns, seed, |_, logs| {
            for log in &logs {
                accumulator.push(log);
            }
        })?;
        let mut stats = accumulator.finish();
        apply_role_counts(&mut stats, &self.deck);
        let stats = BenchmarkStats { stats };
        let mut report = self.report(&stats, seed);
        if let Some(parameters) = opponents {
            report::set_opponent_model(&mut report, self.deck.format, parameters, &stats.stats);
        }
        Ok(report)
    }

    /// Build a serializable report from aggregate values.
    pub fn report(&self, stats: &BenchmarkStats, seed: u64) -> report_schema::SimReport {
        let findings = findings::analyze_findings(&stats.stats, &self.deck);
        let inferred = self.deck.format == model::Format::Commander;
        let bracket = if inferred { 2 } else { 0 };
        let mana_base = findings::mana_base(&self.deck, bracket, inferred);
        report::json_report(
            &stats.stats,
            &self.deck,
            "benchmark",
            seed,
            &findings,
            report::BenchCounts {
                sideboard_cards: 0,
                maybeboard_cards: 0,
            },
            &mana_base,
        )
    }
}
