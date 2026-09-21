// Goldfish Monte Carlo simulation for decks: `stm deck simulate <name>`.
//
// Submodules:
// - `model`: card data model (costs, tap yields, station tiers, abilities)
// - `parse`: oracle-text → model (the sim's whole intelligence)
// - `deck`: deck construction (deck text → `SimDeck`)
// - `game`: the per-game turn loop (pure, seeded)
// - `aggregate`: log aggregation + problem findings
// - `report`: JSON payload + human stdout render
//
// Cards are modeled as data, not as rules: a card gets a tap yield (one
// tap = the listed mana), optional station tiers, an optional crew cost,
// and a list of abilities. Everything the model cannot execute is ignored
// at parse time; the documented limits ship in the output `assumptions`.
// This is a consistency diagnostic, not a win-rate predictor.

pub(crate) mod aggregate;
mod cast_phase;
mod combos;
pub(crate) mod deck;
mod findings;
mod format;
pub(crate) mod game;
mod game_combat;
mod game_effects;
mod game_mana;
mod game_run;
pub(crate) mod hypgeo;
pub(crate) mod model;
pub(crate) mod parse;
mod parse_cost;
mod parse_keywords;
mod parse_land;
mod report;
mod report_view;
mod trigger_activated;
mod trigger_landfall;
mod triggers;

#[cfg(test)]
#[path = "tests/aggregate_tests.rs"]
mod aggregate_tests;
#[cfg(test)]
#[path = "tests/commander_deck_tests.rs"]
mod commander_deck_tests;
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
#[path = "tests/game_mechanic_tests.rs"]
mod game_mechanic_tests;
#[cfg(test)]
#[path = "tests/game_tests.rs"]
mod game_tests;
#[cfg(test)]
#[path = "tests/lethal_tests.rs"]
mod lethal_tests;
#[cfg(test)]
#[path = "tests/mana_base_tests.rs"]
mod mana_base_tests;
#[cfg(test)]
#[path = "tests/mechanic_tests.rs"]
mod mechanic_tests;
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
#[path = "tests/parse_mechanic_tests.rs"]
mod parse_mechanic_tests;
#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod parse_tests;
#[cfg(test)]
#[path = "tests/pipeline_tests.rs"]
mod pipeline_tests;
#[cfg(test)]
#[path = "tests/report_tests.rs"]
mod report_tests;
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

/// True when the store carries combo data at all (public for the
/// `deck combos` audit).
pub fn store_has_combos_pub(conn: &rusqlite::Connection) -> bool {
    store_has_combos(conn)
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
        // Maindeck only: the sideboard is a commander wishlist (the same
        // census `deck legal` uses).
        .filter(|(s, _)| !s.eq_ignore_ascii_case("SIDEBOARD"))
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

fn store_has_combos(conn: &rusqlite::Connection) -> bool {
    conn.query_row("SELECT COUNT(*) FROM combos", [], |r| r.get::<_, i64>(0))
        .map(|n| n > 0)
        .unwrap_or(false)
}

use anyhow::Context;

use super::stats::lookup_names;
use crate::output::Output;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Entry point for `stm deck simulate <name>`.
///
/// Exit 0 when no problems were found, exit 1 when the report has findings
/// (the result is the answer, not a crash). A missing deck file is the
/// shared deck-not-found error from the dispatcher. `baseline` (when given)
/// diffs the fresh report against that prior JSON: human output prints
/// deltas only; `--json` prints the `ReportDiff` as JSON. Either way the
/// exit code keys on new problems, not the raw problem list.
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
    let sideboard_cards = deck.sideboard_total();
    let cards = lookup_names(conn, &deck);
    let mut sim_deck = deck::build_sim_deck(&deck, &cards, format);

    if let Some(f) = format
        && !deck::apply_format_override(&mut sim_deck, f)
    {
        out.error(&format!("--format {f} needs a COMMANDER section"));
        return Ok(crate::cli::codes::USAGE);
    }

    let total_cards = sim_deck.cards.len() + sim_deck.commanders.len();
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
    stats.removal_count = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == model::Role::Removal)
        .count();
    stats.removal_wipes = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == model::Role::Removal && c.wipe)
        .count();
    stats.removal_targeted = stats.removal_count - stats.removal_wipes;
    stats.wincon_count = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == model::Role::Wincon)
        .count();
    stats.draw_count = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == model::Role::Draw)
        .count();
    stats.land_count = sim_deck
        .cards
        .iter()
        .filter(|c| c.role == model::Role::Land)
        .count();
    let problems = findings::find_problems(&stats, &sim_deck);
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
            let baseline: serde_json::Value = read_baseline_json(baseline_path)?;
            let current = report::json_report(
                &stats,
                &sim_deck,
                name,
                seed,
                &problems,
                sideboard_cards,
                &mana_base,
            );
            let diff = report_view::diff_reports(&baseline, &current);
            println!("{}", serde_json::to_string_pretty(&diff)?);
            // Diff mode exits on the delta: empty diff or only resolved
            // problems is clean; any new problem exits 1.
            if diff.problems.iter().any(|p| p.change == "new") {
                return Ok(crate::cli::codes::ERROR);
            }
            return Ok(crate::cli::codes::OK);
        }
        let mut v = report::json_report(
            &stats,
            &sim_deck,
            name,
            seed,
            &problems,
            sideboard_cards,
            &mana_base,
        );
        if !combo_rows.is_empty()
            && let Some(obj) = v.as_object_mut()
        {
            obj.insert("combo_access".into(), serde_json::json!(combo_rows));
        }
        // Static colored-source audit on the same census the sim loaded.
        if let Ok(audit) = super::mana::mana_audit_for(conn, &deck)
            && let Some(obj) = v.as_object_mut()
        {
            obj.insert(
                "colored_sources".into(),
                super::mana_audit::colored_sources_json(&audit),
            );
        }
        if let (Some(assembly), Some(obj)) = (&combo_report, v.as_object_mut()) {
            obj.insert(
                "combos".into(),
                report_view::combos_json(assembly, combo_limit),
            );
            obj.insert(
                "win_paths".into(),
                report_view::win_paths_json(assembly, combo_limit),
            );
        }
        if hypgeo {
            let ceilings = hypgeo::cast_ceilings(&sim_deck, turns);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("hypgeo".into(), ceilings);
            }
        }
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else if let Some(baseline_path) = baseline {
        // Diff mode: load the prior report and print only the deltas.
        let baseline = read_baseline_json(baseline_path)?;
        let current = report::json_report(
            &stats,
            &sim_deck,
            name,
            seed,
            &problems,
            sideboard_cards,
            &mana_base,
        );
        let diff = report_view::diff_reports(&baseline, &current);
        report_view::print_diff(out, &diff);
        // Diff mode exits on the delta: empty diff or only resolved
        // problems is clean; any new problem exits 1.
        if diff.problems.iter().any(|p| p.change == "new") {
            return Ok(crate::cli::codes::ERROR);
        }
        return Ok(crate::cli::codes::OK);
    } else {
        report::print_report(out, name, &sim_deck, &stats, &problems, &mana_base);
        if !combo_rows.is_empty() {
            report_view::print_combo_access(out, &combo_rows);
        }
        match &combo_report {
            Some(assembly) => {
                report_view::print_store_combos(out, assembly, combo_limit);
                report_view::print_win_paths(out, assembly, combo_limit);
            }
            None if !combos.is_empty() || store_has_combos(conn) => {}
            None => {}
        }
        if hypgeo {
            report_view::print_hypgeo(out, &hypgeo::cast_ceilings(&sim_deck, turns));
        }
    }
    if problems.is_empty() {
        Ok(crate::cli::codes::OK)
    } else {
        Ok(crate::cli::codes::ERROR)
    }
}

/// Read and parse a baseline report file as JSON.
fn read_baseline_json(path: &std::path::Path) -> anyhow::Result<serde_json::Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading baseline {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing baseline {}", path.display()))
}
