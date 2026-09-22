// Cause-level detail for sim problems: the offender lists and the
// plain-English problem strings. Split from `findings.rs` (already near
// its line limit); `find_problems` builds problems with empty offender
// lists and this module fills them from data the aggregation stage
// already computed.
//
// Plain-English policy: every user-facing finding follows
// **what is wrong → how often → which cards → what to do**, in short
// sentences with no jargon. The `kind` values in JSON stay stable — only
// the human strings change.

use super::aggregate::SimStats;
use super::findings::Problem;
use super::model::SimDeck;

/// One card or count behind a problem finding.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProblemOffender {
    /// Card name, or a census label ("lands", "rocks") for count-based
    /// causes.
    pub name: String,
    /// Plain-English role in the problem.
    pub detail: String,
    /// Share of games this card contributed to, when known (0-100).
    pub pct_games: Option<f64>,
}

/// Attach offender rows and cause-specific suggestions to a problem.
///
/// Reads only from data the simulation already aggregated: pip blocks
/// (color screw), per-card castability (slow cards), the land/rock/dork
/// census (screw and flood), and the commander's cast stats. Problems
/// without card-level causes keep their category-level suggestion.
/// [`explain`] with an explicit dead-card threshold, matching the bar the
/// finding itself used (0.60 for commander-shaped decks, 0.55 for
/// 60-card ones).
pub fn explain_with_threshold(
    problem: &mut Problem,
    stats: &SimStats,
    deck: &SimDeck,
    dead_cast_share: f64,
) {
    match problem.kind {
        "color_screw" => color_screw_offenders(problem, stats),
        "dead_cards" => dead_card_offenders(problem, stats, dead_cast_share),
        "mana_screw" => mana_screw_offenders(problem, deck),
        "mana_flood" => mana_flood_offenders(problem, deck),
        "commander_late" => commander_late_offenders(problem, stats, deck),
        _ => {}
    }
}

/// Color screw: name the cards whose pips in the missing color got
/// blocked most often, and make the suggestion name them.
fn color_screw_offenders(problem: &mut Problem, stats: &SimStats) {
    let Some(color) = problem.color else { return };
    let mut blocks: Vec<super::findings::PipBlock> = stats
        .pip_blocks
        .iter()
        .filter(|p| p.color == color)
        .cloned()
        .collect();
    // Worst blockers first; keep the top three.
    blocks.sort_by(|a, b| b.pct_games.total_cmp(&a.pct_games));
    blocks.truncate(3);
    if blocks.is_empty() {
        return;
    }
    problem.offenders = blocks
        .iter()
        .map(|p| ProblemOffender {
            name: p.name.clone(),
            detail: format!("its {} pips could not be paid", plain_color(p.color)),
            pct_games: Some(pct2(p.pct_games)),
        })
        .collect();
    let names: Vec<String> = blocks.iter().map(|b| b.name.clone()).collect();
    let color_name = plain_color(color);
    problem.suggestion = format!(
        "add 2-3 more lands or rocks that make {}, or cut one of the worst blockers: {}",
        color_name,
        names.join(", ")
    );
}

/// Dead cards: name the slowest cards and size the suggestion to them.
fn dead_card_offenders(problem: &mut Problem, stats: &SimStats, threshold: f64) {
    // The detail already names the worst 3; mirror them as offenders with
    // their castability shares.
    let mut worst: Vec<&super::aggregate::CardCast> = stats
        .card_castability
        .iter()
        .filter(|c| c.pct_by_target < threshold)
        .collect();
    worst.sort_by(|a, b| a.pct_by_target.total_cmp(&b.pct_by_target));
    let mut seen = HashSet::new();
    problem.offenders = worst
        .into_iter()
        .filter(|c| seen.insert(c.name.clone()))
        .take(3)
        .map(|c| ProblemOffender {
            name: c.name.clone(),
            detail: format!(
                "castable by turn {} in only {}% of games",
                c.target_turn,
                pct2(c.pct_by_target)
            ),
            pct_games: Some(pct2(c.pct_by_target)),
        })
        .collect();
}

/// Mana screw: the census labels behind the shortage (lands, rocks,
/// dorks) so the reader sees the deck's shape, not a bare percentage.
fn mana_screw_offenders(problem: &mut Problem, deck: &SimDeck) {
    let lands = deck.cards.iter().filter(|c| c.role == Role::Land).count();
    let rocks = deck.cards.iter().filter(|c| c.role == Role::Rock).count();
    let dorks = deck.cards.iter().filter(|c| c.role == Role::Dork).count();
    problem.offenders.push(ProblemOffender {
        name: "lands".to_string(),
        detail: format!("{lands} in the deck"),
        pct_games: None,
    });
    if rocks + dorks > 0 {
        problem.offenders.push(ProblemOffender {
            name: "mana rocks and mana creatures".to_string(),
            detail: format!("{rocks} rocks, {dorks} creatures"),
            pct_games: None,
        });
    }
}

/// Mana flood: the land count against the typical band.
fn mana_flood_offenders(problem: &mut Problem, deck: &SimDeck) {
    let lands = deck.cards.iter().filter(|c| c.role == Role::Land).count();
    problem.offenders.push(ProblemOffender {
        name: "lands".to_string(),
        detail: format!("{lands} in the deck (typical commander decks run 32-38)"),
        pct_games: None,
    });
}

/// Commander late: the commander's own cost and cast stats.
fn commander_late_offenders(problem: &mut Problem, stats: &SimStats, deck: &SimDeck) {
    let Some(cmd) = deck.commanders.first() else {
        return;
    };
    problem.offenders.push(ProblemOffender {
        name: cmd.name.clone(),
        detail: format!(
            "costs {} mana; first castable on turn {} on average",
            cmd.cost.total(),
            turn2(stats.avg_commander_cast_turn)
        ),
        pct_games: None,
    });
}

/// WUBRG letter as a spoken color.
fn plain_color(letter: char) -> &'static str {
    match letter {
        'W' => "white",
        'U' => "blue",
        'B' => "black",
        'R' => "red",
        'G' => "green",
        _ => "any",
    }
}

/// Percent with two decimals (0-100), the JSON percent scale.
fn pct2(share: f64) -> f64 {
    (share * 100.0 * 100.0).round() / 100.0
}

/// A turn count with two decimals (an average turn, not a percentage).
fn turn2(turns: f64) -> f64 {
    (turns * 100.0).round() / 100.0
}

use super::model::Role;
use std::collections::HashSet;

#[cfg(test)]
#[path = "tests/findings_detail_tests.rs"]
mod findings_detail_tests;
