//! Goldfish Monte Carlo simulation for decks: `stm deck simulate <name>`.
//!
//! Cards are modeled as data, not as rules: the Oracle parser builds typed
//! syntax nodes, and lowering maps supported nodes to game actions. Unsupported
//! syntax stays inert and the documented limits ship in `assumptions`.
//! This is a consistency diagnostic, not a win-rate predictor.

/// Log aggregation and stats.
pub(crate) mod aggregate;
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
/// Commander casts, command-zone state, and commander damage.
mod game_commander;
/// SimEffect execution plus the tap budget.
mod game_effects;
/// ManaPool building and cost payment.
mod game_mana;
/// The per-game turn loop (pure, seeded).
mod game_run;
/// Hypergeometric cast ceilings.
mod hypgeo;
/// Card data model: costs, mana yields, striations, abilities.
pub(crate) mod model;
/// Oracle syntax tree: typed nodes for parsed Oracle text.
mod oracle_ast;
/// Lowering of Oracle syntax nodes and card metadata to runtime data.
pub(crate) mod oracle_lower;
/// Oracle text grammar and parsing helpers.
pub(crate) mod oracle_parser;
/// Card role and class classification.
mod role_classify;

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
#[cfg(test)]
#[path = "tests/turn_loop_tests.rs"]
mod turn_loop_tests;

#[cfg(test)]
#[path = "tests/aggregate_tests.rs"]
mod aggregate_tests;
#[cfg(test)]
#[path = "tests/cast_restriction_tests.rs"]
mod cast_restriction_tests;
#[cfg(test)]
#[path = "tests/combos_tests.rs"]
mod combos_tests;
#[cfg(test)]
#[path = "tests/commander_deck_tests.rs"]
mod commander_deck_tests;
#[cfg(test)]
#[path = "tests/counter_keyword_tests.rs"]
mod counter_keyword_tests;
#[cfg(test)]
#[path = "tests/cross_deck_tests.rs"]
mod cross_deck_tests;
#[cfg(test)]
#[path = "tests/deck_test_support.rs"]
mod deck_test_support;
#[cfg(test)]
#[path = "tests/deck_tests.rs"]
mod deck_tests;
#[cfg(test)]
#[path = "tests/defining_line_trace_tests.rs"]
mod defining_line_trace_tests;
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
#[cfg(test)]
#[path = "tests/lethal_tests.rs"]
mod lethal_tests;
#[cfg(test)]
#[path = "tests/mana_base_tests.rs"]
mod mana_base_tests;
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
#[cfg(test)]
#[path = "tests/pipeline_tests.rs"]
mod pipeline_tests;
#[cfg(test)]
#[path = "tests/report_cli_tests.rs"]
mod report_cli_tests;
#[cfg(test)]
#[path = "tests/report_tests.rs"]
mod report_tests;
#[cfg(test)]
#[path = "tests/report_view_tests.rs"]
mod report_view_tests;
#[cfg(test)]
#[path = "tests/standard_deck_tests.rs"]
mod standard_deck_tests;

pub use model::DEFAULT_RUNS;

/// Default cap on discovered-combo rows (complete + near-misses each).
const DEFAULT_COMBO_LIMIT: usize = 20;

/// Join the deck's cards against the combo store. `None` when the store
/// has no combo data (table empty): the metric is omitted with a note.
fn load_store_combos(
    conn: &rusqlite::Connection,
    deck: &model::SimDeck,
    format_key: &str,
) -> Option<Vec<combos::ComboCandidate>> {
    if !store_has_combos(conn) {
        return None;
    }
    let names: std::collections::HashSet<String> =
        deck.cards.iter().map(|c| c.name.clone()).collect();
    let variants = crate::combos::load_variants_for(conn, &names).ok()?;
    let (candidates, _excluded) = combos::candidates(variants, deck, format_key);
    Some(candidates)
}

/// True when the store carries combo data at all: any row in the
/// `combos` table. Callers omit combo metrics with a note when false
/// (also public for the `deck combos` audit).
pub(crate) fn store_has_combos(conn: &rusqlite::Connection) -> bool {
    conn.query_row("SELECT COUNT(*) FROM combos", [], |r| r.get::<_, i64>(0))
        .map(|n| n > 0)
        .unwrap_or(false)
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
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Run the goldfish pipeline for an in-memory deck and return the JSON
/// report (no side effects, no output). Shared by `deck simulate` and the
/// `deck update --dry-run --sim` preview so both answers can never
/// disagree. `format` overrides the inferred format.
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
    let bench = report::BenchCounts {
        sideboard_cards: deck.sideboard_total(),
        maybeboard_cards: deck.maybeboard_total(),
    };
    let mut sim_deck = deck::build_sim_deck(deck, cards, format);
    if let Some(f) = format {
        let applied = deck::apply_format_override(&mut sim_deck, f);
        // Release builds keep the deck as parsed when the override name
        // is unknown; the CLI only passes validated format names.
        debug_assert!(applied, "unknown format override {f:?}");
    }
    let total_cards = sim_deck.library_len() + sim_deck.commanders.len();
    if total_cards == 0 {
        anyhow::bail!("deck has no cards");
    }
    let turns = turns.unwrap_or(sim_deck.rules.default_turns);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut logs = Vec::with_capacity(runs as usize);
    for _ in 0..runs {
        logs.push(game::run_game(&sim_deck, &mut rng, turns));
    }
    let mut stats = aggregate::aggregate(&logs, &sim_deck, turns);
    apply_role_counts(&mut stats, &sim_deck);
    let findings = findings::analyze_findings(&stats, &sim_deck);
    let (inferred_bracket, bracket) = if sim_deck.format == model::Format::Constructed {
        (false, 0)
    } else {
        (true, infer_bracket(deck, cards))
    };
    let mana_base = findings::mana_base(&sim_deck, bracket, inferred_bracket);
    Ok(report::json_report(
        &stats, &sim_deck, name, seed, &findings, bench, &mana_base,
    ))
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

/// Entry point for `stm deck simulate <name>`.
///
/// Exit 0 when no problems were found, exit 1 when the report has findings
/// (the result is the answer, not a crash). A missing deck file is the
/// shared deck-not-found error from the dispatcher. `baseline` (when given)
/// diffs the fresh report against that prior JSON: human output prints
/// deltas only; `--json` prints the `ReportDiff` as JSON. Exit codes:
/// without `--baseline` the run exits 1 on ANY problem found; with
/// `--baseline` it exits 1 only on NEW problems versus the baseline.
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
        out.error(&format!("--format {f} needs a COMMANDER section"));
        out.hint(&format!(
            "add a commander first: stm deck update {name} --add commander:1 <card>, or drop --format"
        ));
        return Ok(crate::cli::codes::USAGE);
    }

    let total_cards = sim_deck.library_len() + sim_deck.commanders.len();
    if total_cards == 0 {
        out.error("deck has no cards");
        out.hint("add cards with: stm deck update <name> --add <spec>");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let turns = turns.unwrap_or(sim_deck.rules.default_turns);
    let seed = seed.unwrap_or_else(rand::random::<u64>);

    out.status(
        "Simulating",
        &format!("{runs} games, {turns} turns, seed {seed}"),
    );

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut logs = Vec::with_capacity(runs as usize);
    for _ in 0..runs {
        logs.push(game::run_game(&sim_deck, &mut rng, turns));
    }
    let mut stats = aggregate::aggregate(&logs, &sim_deck, turns);
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

    // Combo assembly, two sources:
    // 1. Explicit "A + B" pairs (--combo, repeatable, measured always).
    // 2. Store-backed Spellbook variants joined against the deck
    //    (complete combos + near-misses; omitted with a note when the
    //    store has no combo data).
    let combo_pairs: Vec<(String, String)> = combos
        .iter()
        .filter_map(|spec| {
            let (a, b) = spec.split_once('+')?;
            Some((a.trim().to_string(), b.trim().to_string()))
        })
        .collect();
    // A malformed pair (missing '+') is visible, not silent: the spec
    // list would otherwise shrink with no trace.
    let dropped_specs = combos.len() - combo_pairs.len();
    if dropped_specs > 0 {
        out.warning(&format!(
            "{dropped_specs} --combo spec(s) missing '+' between the two cards were skipped"
        ));
    }
    let combo_rows = findings::piece_pair_access(&logs, &sim_deck, &combo_pairs, turns);
    let combo_limit = combo_limit.unwrap_or(DEFAULT_COMBO_LIMIT);
    let combo_report = load_store_combos(conn, &sim_deck, sim_deck.rules.key).map(|candidates| {
        let mut assembly = combos::measure(&candidates, &logs, &sim_deck, turns);
        combos::rank_complete(&mut assembly.complete);
        assembly
    });

    if json {
        if let Some(baseline_path) = baseline {
            // JSON diff mode: print the ReportDiff as JSON so agents can
            // gate on new problems without hand-diffing full reports.
            let baseline = read_baseline_report(baseline_path)?;
            let current =
                report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
            let diff =
                report_view::diff_reports(&baseline, &current).map_err(anyhow::Error::msg)?;
            println!("{}", serde_json::to_string_pretty(&diff)?);
            // Diff mode exits on the delta: empty diff or only resolved
            // problems is clean; any new problem exits 1.
            return Ok(report_view::baseline_exit_code(&diff));
        }
        let mut report =
            report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
        if !combo_rows.is_empty() {
            report.combo_access = Some(
                combo_rows
                    .iter()
                    .map(|row| report_schema::PairAccessReport {
                        pair: row.pair.clone(),
                        target_turn: row.target_turn,
                        percent_of_games: report_schema::Percent::from_share(row.game_share),
                    })
                    .collect(),
            );
        }
        // Static colored-source audit on the same census the sim loaded.
        let audit = super::mana::mana_audit_for(conn, &deck)?;
        report.colored_sources = Some(super::mana_audit::colored_sources_report(&audit));
        if let Some(assembly) = &combo_report {
            report.combos = Some(report_view::combos_report(assembly, combo_limit));
            report.win_paths = Some(report_view::win_paths_report(assembly, combo_limit));
        }
        if hypgeo {
            report.hypgeo = Some(hypgeo::cast_ceilings(&sim_deck, turns));
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if let Some(baseline_path) = baseline {
        // Diff mode: load the prior report and print only the deltas.
        let baseline = read_baseline_report(baseline_path)?;
        let current =
            report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
        let diff = report_view::diff_reports(&baseline, &current).map_err(anyhow::Error::msg)?;
        report_view::print_diff(out, &diff);
        // Diff mode exits on the delta: empty diff or only resolved
        // problems is clean; any new problem exits 1.
        return Ok(report_view::baseline_exit_code(&diff));
    } else {
        let mut report =
            report::json_report(&stats, &sim_deck, name, seed, &findings, bench, &mana_base);
        if !combo_rows.is_empty() {
            report.combo_access = Some(
                combo_rows
                    .iter()
                    .map(|row| report_schema::PairAccessReport {
                        pair: row.pair.clone(),
                        target_turn: row.target_turn,
                        percent_of_games: report_schema::Percent::from_share(row.game_share),
                    })
                    .collect(),
            );
        }
        let audit = super::mana::mana_audit_for(conn, &deck)?;
        report.colored_sources = Some(super::mana_audit::colored_sources_report(&audit));
        if let Some(assembly) = &combo_report {
            report.combos = Some(report_view::combos_report(assembly, combo_limit));
            report.win_paths = Some(report_view::win_paths_report(assembly, combo_limit));
        }
        if hypgeo {
            report.hypgeo = Some(hypgeo::cast_ceilings(&sim_deck, turns));
        }
        report::print_report(out, &report);
        if let Some(rows) = &report.combo_access {
            report_view::print_combo_access(out, rows);
        }
        if let Some(combos) = &report.combos {
            report_view::print_store_combos(out, combos);
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
                "parsing simulator report baseline {} (save a complete current report with `stm deck simulate --json`)",
                path.display()
            )
        })
}
