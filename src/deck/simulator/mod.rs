//! Goldfish Monte Carlo simulation for decks: `stm deck simulate <name>`.
//!
//! The Oracle parser builds typed syntax nodes. Lowering maps supported nodes
//! to game actions. Unsupported syntax stays inert. Reports state the model
//! limits in `assumptions`.
//! This is a consistency diagnostic, not a win-rate predictor.

/// Log aggregation and stats.
pub(crate) mod aggregate;
/// Opaque inputs and results for release-mode simulator benchmarks.
#[doc(hidden)]
pub mod benchmark;
/// The cast algorithm and land-drop helpers.
mod cast_pass;
/// Combo assembly measurement.
mod combos;
/// Opening-hand dealing plus the mulligan policy.
pub(crate) mod deal;
/// Deck construction: deck text to `SimDeck`.
pub(crate) mod deck;
/// Simulator findings and the mana-base verdict.
mod findings;
/// Finding detail helpers.
mod findings_detail;
/// Format rules: mulligan policy and turn count.
pub(crate) mod format;
/// Shared per-game types: `GameLog`, battlefield permanents, the mana pool.
pub(crate) mod game;
/// The combat phase.
mod game_combat;
/// Commander casts and command-zone state.
mod game_commander;
/// SimEffect execution plus the tap budget.
mod game_effects;
/// ManaPool building and cost payment.
mod game_mana;
/// The per-game turn loop (pure, seeded).
mod game_run;
/// Known-hand payment checks through turn two.
pub(crate) mod hand_payment;
/// Hypergeometric cast ceilings.
mod hypgeo;
/// Card data model: costs, mana yields, striations, abilities.
pub(crate) mod model;
/// Shared opponent facts and scheduled table activity.
pub mod opponents;
/// Oracle syntax tree: typed nodes for parsed Oracle text.
pub(crate) mod oracle_ast;
/// Lowering of Oracle syntax nodes and card metadata to runtime data.
pub(crate) mod oracle_lower;
/// Oracle text grammar and parsing helpers.
pub(crate) mod oracle_parser;
/// Streaming first-hand-access measurement for explicit combo pairs.
mod pair_access;
/// Trigger capture and settlement at supported action boundaries.
mod resolution;
/// Versioned per-game random streams and stable shuffle sampling.
#[doc(hidden)]
pub mod rng;
/// Card role and class classification.
mod role_classify;
/// Bounded, ordered execution of independent games.
mod runner;
/// Per-ability support evidence from Oracle lowering.
pub mod support;

#[cfg(test)]
#[path = "tests/cast_zone_tests.rs"]
mod cast_zone_tests;
#[cfg(test)]
#[path = "tests/library_effect_tests.rs"]
mod library_effect_tests;
#[cfg(test)]
#[path = "tests/parse_cost_tests.rs"]
mod parse_cost_tests;
/// JSON report payload.
mod report;
/// Typed JSON schema for simulator reports.
pub(crate) mod report_schema;
/// Human stdout render of the report.
pub(crate) mod report_view;
/// Turn events, terminal outcomes, and report-denominator boundaries.
#[cfg(test)]
#[path = "tests/turn_boundary_report_tests.rs"]
mod turn_boundary_report_tests;
#[cfg(test)]
#[path = "tests/turn_loop_tests.rs"]
mod turn_loop_tests;
/// Physical zone movement, access, and replay report boundaries.
#[cfg(test)]
#[path = "tests/zone_boundary_report_tests.rs"]
mod zone_boundary_report_tests;

/// Complete fixed payments and rejected compound activation costs.
#[cfg(test)]
#[path = "tests/activation_payment_boundary_tests.rs"]
mod activation_payment_boundary_tests;
/// Activated graveyard targets retain pre-cost physical identity.
#[cfg(test)]
#[path = "tests/activation_target_boundary_tests.rs"]
mod activation_target_boundary_tests;
#[cfg(test)]
#[path = "tests/aggregate_tests.rs"]
mod aggregate_tests;
/// Pinned-source syntax and restriction assertions.
#[cfg(test)]
#[path = "tests/card_ast_tests.rs"]
mod card_ast_tests;
/// Pinned cost limits and bank-only Treasure behavior.
#[cfg(test)]
#[path = "tests/card_boundary_cost_tests.rs"]
mod card_boundary_cost_tests;
/// Paid-action and full-turn boundaries for unsupported searches.
#[cfg(test)]
#[path = "tests/card_boundary_search_tests.rs"]
mod card_boundary_search_tests;
/// Source-bound payment and mana-mode obligations.
#[cfg(test)]
#[path = "tests/card_cost_claim_tests.rs"]
mod card_cost_claim_tests;
/// Source-bound damage scopes and inert target boundaries.
#[cfg(test)]
#[path = "tests/card_damage_claim_tests.rs"]
mod card_damage_claim_tests;
/// Source-bound upkeep, death, and counter observations.
#[cfg(test)]
#[path = "tests/card_event_claim_tests.rs"]
mod card_event_claim_tests;
/// Executed card evidence registrations and coverage counts.
#[cfg(test)]
#[path = "tests/card_fixture_tests.rs"]
mod card_fixture_tests;
/// Source-bound conditional land entry and fetch boundaries.
#[cfg(test)]
#[path = "tests/card_land_claim_tests.rs"]
mod card_land_claim_tests;
/// Landfall, stacked land plays, and current-turn limits.
#[cfg(test)]
#[path = "tests/card_landfall_claim_tests.rs"]
mod card_landfall_claim_tests;
/// Source-bound draws, mill, dredge, loot, and cascade subsets.
#[cfg(test)]
#[path = "tests/card_library_claim_tests.rs"]
mod card_library_claim_tests;
/// Source-bound payment symbols and synthetic payment-report controls.
#[cfg(test)]
#[path = "tests/card_payment_claim_tests.rs"]
mod card_payment_claim_tests;
/// Source-bound graveyard permissions, costs, and physical returns.
#[cfg(test)]
#[path = "tests/card_replay_claim_tests.rs"]
mod card_replay_claim_tests;
/// Reports built from executed pinned-card scenarios.
#[cfg(test)]
#[path = "tests/card_report_tests.rs"]
mod card_report_tests;
/// Legal and rejected actions for pinned card sources.
#[cfg(test)]
#[path = "tests/card_runtime_tests.rs"]
mod card_runtime_tests;
/// Per-card obligation dispositions without whole-card support claims.
#[cfg(test)]
#[path = "tests/card_semantic_tests.rs"]
mod card_semantic_tests;
/// Offline source identity and capture integrity checks.
#[cfg(test)]
#[path = "tests/card_source_tests.rs"]
mod card_source_tests;
/// Shared source lookup and controlled card scenario setup.
#[cfg(test)]
#[path = "tests/card_test_support.rs"]
mod card_test_support;
/// Generic-body and bank-only token event boundaries.
#[cfg(test)]
#[path = "tests/card_token_claim_tests.rs"]
mod card_token_claim_tests;
/// Cascade child outcomes must stop later parent resolution.
#[cfg(test)]
#[path = "tests/cascade_boundary_tests.rs"]
mod cascade_boundary_tests;
/// Complete cast-choice and land-policy actions with report evidence.
#[cfg(test)]
#[path = "tests/cast_boundary_report_tests.rs"]
mod cast_boundary_report_tests;
#[cfg(test)]
#[path = "tests/cast_restriction_tests.rs"]
mod cast_restriction_tests;
/// Unsupported variable pump and hidden-face static power cannot affect combat.
#[cfg(test)]
#[path = "tests/combat_power_boundary_tests.rs"]
mod combat_power_boundary_tests;
#[cfg(test)]
#[path = "tests/combos_tests.rs"]
mod combos_tests;
/// Source-bound Behold costs, choices, entry, and linked fetch scenarios.
#[cfg(test)]
#[path = "tests/commander_behold_tests.rs"]
mod commander_behold_tests;
/// Compiler, resource, and report invariants across the expanded corpus.
#[cfg(test)]
#[path = "tests/commander_corpus_tests.rs"]
mod commander_corpus_tests;
/// Commander taxes, unsupported costs, and terminal payment reports.
#[cfg(test)]
#[path = "tests/commander_cost_boundary_report_tests.rs"]
mod commander_cost_boundary_report_tests;
#[cfg(test)]
#[path = "tests/commander_deck_tests.rs"]
mod commander_deck_tests;
/// Source-bound entry and activation gates for common land cycles.
#[cfg(test)]
#[path = "tests/commander_land_entry_tests.rs"]
mod commander_land_entry_tests;
/// Expanded Commander source selection and capture integrity.
#[cfg(test)]
#[path = "tests/commander_source_tests.rs"]
mod commander_source_tests;
/// Shared pinned inputs for popular Commander card scenarios.
#[cfg(test)]
#[path = "tests/commander_test_support.rs"]
mod commander_test_support;
/// Paid compiler-rejection boundaries and pinned scaled-reward controls.
#[cfg(test)]
#[path = "tests/compiler_safety_action_tests.rs"]
mod compiler_safety_action_tests;
/// Cost and commander-payment boundaries with executed report evidence.
#[cfg(test)]
#[path = "tests/cost_boundary_report_tests.rs"]
mod cost_boundary_report_tests;
/// Variable, optional, and restricted cast payments with executed reports.
#[cfg(test)]
#[path = "tests/cost_variable_report_tests.rs"]
mod cost_variable_report_tests;
#[cfg(test)]
#[path = "tests/counter_keyword_tests.rs"]
mod counter_keyword_tests;
#[cfg(test)]
#[path = "tests/cross_deck_tests.rs"]
mod cross_deck_tests;
/// Synthetic scope, resolution, and combat-potential report controls.
#[cfg(test)]
#[path = "tests/damage_core_report_tests.rs"]
mod damage_core_report_tests;
#[cfg(test)]
#[path = "tests/deck_test_support.rs"]
mod deck_test_support;
#[cfg(test)]
#[path = "tests/deck_tests.rs"]
mod deck_tests;
#[cfg(test)]
#[path = "tests/defining_line_trace_tests.rs"]
mod defining_line_trace_tests;
/// Successful draws and their replacements keep distinct payment metrics.
#[cfg(test)]
#[path = "tests/draw_outcome_boundary_tests.rs"]
mod draw_outcome_boundary_tests;
/// Complete Escape payments and safe normal-cost casting.
#[cfg(test)]
#[path = "tests/escape_payment_boundary_tests.rs"]
mod escape_payment_boundary_tests;
/// Fetch admission, payment, resolution, and report boundaries.
#[cfg(test)]
#[path = "tests/fetch_requirement_boundary_tests.rs"]
mod fetch_requirement_boundary_tests;
#[cfg(test)]
#[path = "tests/fixture_integrity_tests.rs"]
mod fixture_integrity_tests;
#[cfg(test)]
#[path = "tests/fixture_mechanics_tests.rs"]
mod fixture_mechanics_tests;
#[cfg(test)]
#[path = "tests/format_tests.rs"]
mod format_tests;
#[cfg(test)]
#[path = "tests/game_fix_tests.rs"]
mod game_fix_tests;
#[cfg(test)]
#[path = "tests/game_mana_edge_tests.rs"]
mod game_mana_edge_tests;
#[cfg(test)]
#[path = "tests/game_mechanic_tests.rs"]
mod game_mechanic_tests;
#[cfg(test)]
#[path = "tests/game_tests.rs"]
mod game_tests;
#[cfg(test)]
#[path = "tests/hypgeo_tests.rs"]
mod hypgeo_tests;
/// Commander-identity mana through payments, turns, and source reports.
#[cfg(test)]
#[path = "tests/identity_mana_tests.rs"]
mod identity_mana_tests;
#[cfg(test)]
#[path = "tests/lethal_tests.rs"]
mod lethal_tests;
/// Synthetic cycling, landcycling, and scaled-draw report contracts.
#[cfg(test)]
#[path = "tests/library_core_report_tests.rs"]
mod library_core_report_tests;
#[cfg(test)]
#[path = "tests/mana_base_tests.rs"]
mod mana_base_tests;
/// Mana grants, reuse, search restrictions, and their executed reports.
#[cfg(test)]
#[path = "tests/mana_boundary_report_tests.rs"]
mod mana_boundary_report_tests;
#[cfg(test)]
#[path = "tests/mechanic_tests.rs"]
mod mechanic_tests;
/// Tests for the strong-mechanic batch (mobilize, amass, Ring, …).
#[cfg(test)]
#[path = "tests/mechanics_tests.rs"]
mod mechanics_tests;
#[cfg(test)]
#[path = "tests/model_tests.rs"]
mod model_tests;
#[cfg(test)]
#[path = "tests/modern_deck_tests.rs"]
mod modern_deck_tests;
#[cfg(test)]
#[path = "tests/mulligan_tests.rs"]
mod mulligan_tests;
#[cfg(test)]
#[path = "tests/oracle_ast_tests.rs"]
mod oracle_ast_tests;
#[cfg(test)]
#[path = "tests/oracle_parser_regression_tests.rs"]
mod oracle_parser_regression_tests;
#[cfg(test)]
#[path = "tests/oracle_review_tests.rs"]
mod oracle_review_tests;
/// Exact-state tests for Oracle-driven simulator effects and triggers.
#[cfg(test)]
#[path = "tests/oracle_runtime_tests.rs"]
mod oracle_runtime_tests;
#[cfg(test)]
#[path = "tests/parse_mechanic_tests.rs"]
mod parse_mechanic_tests;
#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod parse_tests;
/// Permanent rules and cleanup with complete-action report evidence.
#[cfg(test)]
#[path = "tests/permanent_boundary_report_tests.rs"]
mod permanent_boundary_report_tests;
/// Synthetic combat, crew, station, Saga, and loyalty report contracts.
#[cfg(test)]
#[path = "tests/permanent_core_report_tests.rs"]
mod permanent_core_report_tests;
#[cfg(test)]
#[path = "tests/pipeline_tests.rs"]
mod pipeline_tests;
#[cfg(test)]
#[path = "tests/popular_corpus_tests.rs"]
mod popular_corpus_tests;
#[cfg(test)]
#[path = "tests/popular_runtime_tests.rs"]
mod popular_runtime_tests;
/// Known events before loss remain separate from missing phase observations.
#[cfg(test)]
#[path = "tests/preloss_event_tests.rs"]
mod preloss_event_tests;
/// Exact search types, result counts, and known front-face colors.
#[cfg(test)]
#[path = "tests/search_filter_boundary_tests.rs"]
mod search_filter_boundary_tests;
/// Conditional token replacements must not execute both branches.
#[cfg(test)]
#[path = "tests/token_replacement_boundary_tests.rs"]
mod token_replacement_boundary_tests;

/// Runtime controls for audited popular-card grammar and payment paths.
#[cfg(test)]
#[path = "tests/popular_audit_tests.rs"]
mod popular_audit_tests;
#[cfg(test)]
#[path = "tests/report_cli_tests.rs"]
mod report_cli_tests;
#[cfg(test)]
#[path = "tests/report_tests.rs"]
mod report_tests;
#[cfg(test)]
#[path = "tests/report_view_tests.rs"]
mod report_view_tests;
/// Parsed Saga duration and final-chapter cleanup boundaries.
#[cfg(test)]
#[path = "tests/saga_chapter_boundary_tests.rs"]
mod saga_chapter_boundary_tests;
/// Controlled initial zones and card rows shared by simulator scenarios.
#[cfg(test)]
#[path = "tests/scenario_test_support.rs"]
mod scenario_test_support;
#[cfg(test)]
#[path = "tests/sourced_precon_report_tests.rs"]
mod sourced_precon_report_tests;
#[cfg(test)]
#[path = "tests/standard_deck_tests.rs"]
mod standard_deck_tests;

pub use model::DEFAULT_RUNS;
pub use report_schema::SimReport;
pub(crate) use runner::BatchRunner;
pub use runner::RunConfig;

#[cfg(test)]
#[path = "tests/store_combo_tests.rs"]
mod store_combo_tests;

/// Maximum displayed rows for each discovered-combo category.
const DEFAULT_COMBO_LIMIT: usize = 20;

/// Join the deck's cards against the combo store. `None` when the store
/// has no combo data (table empty): the metric is omitted with a note.
fn load_store_combos(
    conn: &rusqlite::Connection,
    deck: &model::SimDeck,
    format_key: &str,
) -> anyhow::Result<Option<Vec<combos::ComboCandidate>>> {
    if !store_has_combos(conn)? {
        return Ok(None);
    }
    let names: std::collections::HashSet<String> =
        deck.cards.iter().map(|c| c.name.clone()).collect();
    let variants = crate::combos::load_variants_for(conn, &names)?;
    let (candidates, _excluded) = combos::candidates(variants, deck, format_key);
    Ok(Some(candidates))
}

/// Optional analyses attached to both normal and baseline reports.
struct ReportAnalyses<'a> {
    stats: &'a aggregate::SimStats,
    deck: &'a crate::deck::grammar::Deck,
    conn: &'a rusqlite::Connection,
    sim_deck: &'a model::SimDeck,
    combo_rows: &'a [findings::ComboAccess],
    combo_report: Option<&'a combos::Assembly>,
    combo_limit: usize,
    turns: u32,
    hypgeo: bool,
    opponent_config: &'a opponents::OpponentConfig,
}

/// Add optional analysis sections so baseline and normal reports compare alike.
fn add_optional_report_analyses(
    report: &mut report_schema::SimReport,
    analyses: &ReportAnalyses<'_>,
) -> anyhow::Result<()> {
    report::set_opponent_model(
        report,
        analyses.sim_deck.format,
        analyses.opponent_config.clone(),
        analyses.stats,
    );
    if !analyses.combo_rows.is_empty() {
        report.combo_access = Some(
            analyses
                .combo_rows
                .iter()
                .map(|row| {
                    report_schema::PairAccessReport::from_access(row, report.runs, analyses.turns)
                })
                .collect(),
        );
    }
    let audit = super::mana::mana_audit_for(analyses.conn, analyses.deck)?;
    report.colored_sources = Some(super::mana_audit::colored_sources_report(&audit));
    if let Some(assembly) = analyses.combo_report {
        report.combos = Some(report_view::combos_report(assembly, analyses.combo_limit));
        report.win_paths = Some(report_view::win_paths_report(
            assembly,
            analyses.combo_limit,
        ));
    }
    if analyses.hypgeo {
        report.hypgeo = Some(hypgeo::modeled_access(analyses.sim_deck, analyses.turns));
    }
    Ok(())
}

/// True when the store carries combo data at all: any row in the
/// `combos` table. Callers omit combo metrics with a note when false
/// (also public for the `deck combos` audit).
pub(crate) fn store_has_combos(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM combos)", [], |row| row.get(0))
}

/// Infer the commander bracket from the deck's Game Changer census: the
/// same allowance rule `deck legal` checks, run backwards. 0 changers
/// reads as bracket 2, up to 3 as bracket 3, more as bracket 4.
pub(crate) fn infer_bracket(
    deck: &super::Deck,
    cards: &std::collections::HashMap<String, crate::db::CardRow>,
) -> u8 {
    let changers = deck
        .sections
        .iter()
        // Maindeck only: the bench sections are not part of the deck (the
        // same census `deck legal` uses).
        .filter(|(s, _)| !crate::deck::grammar::is_bench_section(s))
        .flat_map(|(_, e)| e.iter())
        .filter(|e| {
            !e.name.is_empty()
                && cards
                    .get(&e.name)
                    .is_some_and(|c| c.game_changer == Some(true))
        })
        .map(|e| e.name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    match changers {
        0 => 2,
        1..=3 => 3,
        _ => 4,
    }
}

use anyhow::Context;

use super::stats::lookup_names;
use crate::output::Output;

/// Build a core report without database or terminal access.
///
/// Simulation and update previews share this pipeline. `format` overrides the
/// inferred format. Equal inputs and settings produce equal reports.
// The 7 parameters mirror the sim pipeline's inputs one to one; a struct
// would move the plumbing without removing any argument.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sim_report_for(
    deck: &super::Deck,
    cards: &std::collections::HashMap<String, crate::db::CardRow>,
    name: &str,
    runs: u32,
    turns: Option<u32>,
    seed: u64,
    format: Option<&str>,
) -> anyhow::Result<report_schema::SimReport> {
    sim_report_for_with_config(
        deck,
        cards,
        name,
        runs,
        turns,
        seed,
        format,
        RunConfig::default(),
    )
}

/// Build a core report with explicit worker, batch, and opponent settings.
///
/// Does not read a database or render output. Optional CLI analyses are absent.
/// `format` overrides the inferred format. `None` for `turns` uses that format's
/// default horizon. The game count and effective turn count must be positive.
///
/// # Errors
/// Returns errors for an empty deck, zero games or turns, a rejected format
/// override, invalid opponent settings, or worker-pool creation failure.
// These parameters match the existing report inputs plus execution settings.
#[allow(clippy::too_many_arguments)]
pub fn sim_report_for_with_config(
    deck: &super::Deck,
    cards: &std::collections::HashMap<String, crate::db::CardRow>,
    name: &str,
    runs: u32,
    turns: Option<u32>,
    seed: u64,
    format: Option<&str>,
    execution: RunConfig,
) -> anyhow::Result<SimReport> {
    let bench = report::BenchCounts {
        sideboard_cards: deck.sideboard_total(),
        maybeboard_cards: deck.maybeboard_total(),
    };
    let mut sim_deck = deck::build_sim_deck(deck, cards, format);
    if let Some(f) = format {
        let applied = deck::apply_format_override(&mut sim_deck, f);
        anyhow::ensure!(applied, "format {f} requires a COMMANDER section");
    }
    let total_cards = sim_deck.library_len() + sim_deck.commanders.len();
    if total_cards == 0 {
        anyhow::bail!("deck has no cards");
    }
    let turns = turns.unwrap_or(sim_deck.rules.default_turns);
    runner::validate_workload(runs, turns)?;
    let opponent_config = execution
        .opponents
        .clone()
        .unwrap_or_else(|| opponents::OpponentTable::for_rules(sim_deck.rules).config);
    let mut accumulator = aggregate::SimAccumulator::new(&sim_deck, turns);
    runner::BatchRunner::new(execution)?.for_each_batch(
        &sim_deck,
        runs,
        turns,
        seed,
        |_, logs| {
            for log in &logs {
                accumulator.push(log);
            }
        },
    )?;
    let mut stats = accumulator.finish();
    apply_role_counts(&mut stats, &sim_deck);
    let findings = findings::analyze_findings(&stats, &sim_deck);
    let (inferred_bracket, bracket) = if sim_deck.format == model::Format::Constructed {
        (false, 0)
    } else {
        (true, infer_bracket(deck, cards))
    };
    let mana_base = findings::mana_base(&sim_deck, bracket, inferred_bracket);
    let mut result =
        report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
    report::set_opponent_model(&mut result, sim_deck.format, opponent_config, &stats);
    Ok(result)
}

/// Fill the removal/wincon/draw/land census fields the aggregator leaves
/// unset: they come from static card data, not game logs. The companion
/// starts outside the game, so it is not part of the census.
fn apply_role_counts(stats: &mut aggregate::SimStats, sim_deck: &model::SimDeck) {
    stats.removal_count = sim_deck
        .library_cards()
        .filter(|c| c.role == model::Role::Removal)
        .count();
    stats.removal_wipes = sim_deck
        .library_cards()
        .filter(|c| c.role == model::Role::Removal && c.flags.sweeps)
        .count();
    stats.removal_targeted = stats.removal_count - stats.removal_wipes;
    stats.wincon_count = sim_deck
        .library_cards()
        .filter(|c| c.role == model::Role::Wincon)
        .count();
    stats.draw_count = sim_deck
        .library_cards()
        .filter(|c| c.role == model::Role::Draw)
        .count();
    stats.land_count = sim_deck
        .library_cards()
        .filter(|c| c.role == model::Role::Land)
        .count();
}

/// Run `stm deck simulate <name>` and return its documented exit status.
///
/// Normal output reports current findings. Baseline mode reports differences
/// from a saved current-schema report. JSON mode writes a report or `ReportDiff`
/// to stdout. Diagnostics use the shared output streams.
// The 12 parameters mirror the CLI surface one to one; a struct would
// move the clap plumbing without removing any argument.
#[allow(clippy::too_many_arguments)]
pub fn simulate(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut Output,
    name: &str,
    runs: u32,
    turns: Option<u32>,
    seed: Option<u64>,
    format: Option<&str>,
    baseline: Option<&std::path::Path>,
    hypgeo: bool,
    combos: Vec<String>,
    combo_limit: Option<usize>,
    bracket: Option<u8>,
    execution: RunConfig,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = super::store::load_deck(paths, name)?;
    let bench = report::BenchCounts {
        sideboard_cards: deck.sideboard_total(),
        maybeboard_cards: deck.maybeboard_total(),
    };
    let cards = lookup_names(conn, &deck)?;
    let mut sim_deck = deck::build_sim_deck(&deck, &cards, format);

    if let Some(f) = format
        && !deck::apply_format_override(&mut sim_deck, f)
    {
        out.error(&format!(
            "--format {f} requires a COMMANDER section in the decklist"
        ));
        out.hint(
            "add a commander: stm deck update <name> --add \"commander:1 <card>\", or omit --format",
        );
        return Ok(crate::cli::codes::USAGE);
    }

    let total_cards = sim_deck.library_len() + sim_deck.commanders.len();
    if total_cards == 0 {
        out.error("the deck has no cards to simulate");
        out.hint("add cards with: stm deck update <name> --add <spec>");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let turns = turns.unwrap_or(sim_deck.rules.default_turns);
    runner::validate_workload(runs, turns)?;
    let seed = seed.unwrap_or_else(rand::random::<u64>);

    out.status(
        "Simulating",
        &format!("{runs} games, up to {turns} turns per game, seed {seed}"),
    );

    let combo_pairs: Vec<(String, String)> = combos
        .iter()
        .filter_map(|spec| {
            let (first, second) = spec.split_once('+')?;
            Some((first.trim().to_string(), second.trim().to_string()))
        })
        .collect();
    if combos.len() != combo_pairs.len() {
        out.warning("--combo requires two names separated by '+'. Invalid pairs were skipped.");
    }
    let candidates = load_store_combos(conn, &sim_deck, sim_deck.rules.key)?;
    let mut stored_combos = candidates
        .as_ref()
        .map(|candidates| combos::ComboAccumulator::new(candidates, &sim_deck, turns));
    let mut explicit_pairs = pair_access::PairAccumulator::new(&sim_deck, &combo_pairs, turns);
    let opponent_config = execution
        .opponents
        .clone()
        .unwrap_or_else(|| opponents::OpponentTable::for_rules(sim_deck.rules).config);
    let mut accumulator = aggregate::SimAccumulator::new(&sim_deck, turns);
    let runner = runner::BatchRunner::new(execution)?;
    out.progress_bar("Simulating", "games", u64::from(runs));
    let simulation = runner.for_each_batch(&sim_deck, runs, turns, seed, |_, logs| {
        for log in &logs {
            accumulator.push(log);
            explicit_pairs.push(log);
            if let Some(combos) = &mut stored_combos {
                combos.push(log);
            }
        }
        out.tick_progress(logs.len() as u64);
    });
    out.clear_progress();
    simulation?;
    let mut stats = accumulator.finish();
    apply_role_counts(&mut stats, &sim_deck);
    let findings = findings::analyze_findings(&stats, &sim_deck);
    // Bracket for the mana-base band: explicit flag, else inferred from the
    // Game Changer census (the same signals `deck legal` checks). 60-card
    // decks skip bracket inference entirely (Karsten bands by curve).
    let (inferred_bracket, bracket) = if sim_deck.format == model::Format::Constructed {
        (false, 0)
    } else {
        match bracket {
            Some(b) => (false, b),
            None => (true, infer_bracket(&deck, &cards)),
        }
    };
    let mana_base = findings::mana_base(&sim_deck, bracket, inferred_bracket);

    let combo_rows = explicit_pairs.finish();
    let combo_limit = combo_limit.unwrap_or(DEFAULT_COMBO_LIMIT);
    let combo_report = stored_combos.map(|accumulator| {
        let mut assembly = accumulator.finish();
        combos::rank_complete(&mut assembly.complete);
        assembly
    });
    let report_analyses = ReportAnalyses {
        stats: &stats,
        deck: &deck,
        conn,
        sim_deck: &sim_deck,
        combo_rows: &combo_rows,
        combo_report: combo_report.as_ref(),
        combo_limit,
        turns,
        hypgeo,
        opponent_config: &opponent_config,
    };

    if json {
        if let Some(baseline_path) = baseline {
            // JSON diff mode: print the ReportDiff as JSON so agents can
            // gate on new problems without hand-diffing full reports.
            let baseline = read_baseline_report(baseline_path)?;
            let mut current =
                report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
            add_optional_report_analyses(&mut current, &report_analyses)?;
            let diff =
                report_view::diff_reports(&baseline, &current).map_err(anyhow::Error::msg)?;
            println!("{}", serde_json::to_string_pretty(&diff)?);
            // Diff mode exits on the delta: empty diff or only resolved
            // problems is clean; any new problem exits 1.
            return Ok(report_view::baseline_exit_code(&diff));
        }
        let mut report =
            report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
        add_optional_report_analyses(&mut report, &report_analyses)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if let Some(baseline_path) = baseline {
        // Diff mode: load the prior report and print only the deltas.
        let baseline = read_baseline_report(baseline_path)?;
        let mut current =
            report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
        add_optional_report_analyses(&mut current, &report_analyses)?;
        let diff = report_view::diff_reports(&baseline, &current).map_err(anyhow::Error::msg)?;
        report_view::print_diff(out, &diff);
        // Diff mode exits on the delta: empty diff or only resolved
        // problems is clean; any new problem exits 1.
        return Ok(report_view::baseline_exit_code(&diff));
    } else {
        let mut report =
            report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
        add_optional_report_analyses(&mut report, &report_analyses)?;
        report::print_report(out, &report);
        if let Some(rows) = &report.combo_access {
            report_view::print_combo_access(out, rows);
        }
        if let Some(combos) = &report.combos {
            report_view::print_store_combos(out, combos);
        } else {
            out.print_wrapped("Combo analysis unavailable: the local store has no combo data.");
        }
        if let Some(win_paths) = &report.win_paths {
            report_view::print_win_paths(out, win_paths);
        }
        if let Some(ceilings) = &report.hypgeo {
            report_view::print_hypgeo(out, ceilings);
        }
    }
    Ok(report_view::normal_exit_code(&findings))
}

/// Read and validate a complete typed baseline report.
fn read_baseline_report(path: &std::path::Path) -> anyhow::Result<report_schema::SimReport> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading baseline {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| {
            format!(
                "parsing simulator report baseline {} (save a current report with `stm deck simulate <name> --json`)",
                path.display()
            )
        })
}
