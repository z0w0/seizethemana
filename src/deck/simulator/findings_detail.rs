//! Cause-level evidence for simulator findings: evidence rows and
//! plain-English problem strings. Split from `findings.rs` (already near
//! its line limit); `analyze_findings` builds findings with empty evidence
//! lists and this module fills them from data the aggregation stage
//! already computed.
//!
//! Plain-English policy: every user-facing finding follows
//! **what is wrong → how often → which cards → what to do**, in short
//! sentences with no jargon. The `kind` values in JSON stay stable — only
//! the human strings change.

use super::aggregate::SimStats;
use super::findings::Finding;
use super::model::{ManaColor, SimDeck};

/// One card or count behind a problem finding.
#[derive(Debug, Clone)]
pub struct FindingEvidence {
    /// Card name, or a census label ("lands", "rocks") for count-based
    /// causes.
    pub name: String,
    /// Plain-English role in the problem.
    pub explanation: String,
    /// Share of games this card contributed to, when known (0-100).
    pub game_share: Option<f64>,
}

/// Attach offender rows and cause-specific suggestions to a problem.
///
/// Reads only from data the simulation already aggregated: pip blocks
/// (color screw), per-card castability (slow cards), the land/rock/dork
/// census (screw and flood), and the commander's cast stats. Findings
/// without card-level causes keep their category-level suggestion.
/// [`explain`] with an explicit dead-card threshold, matching the bar the
/// finding itself used (0.60 for commander-shaped decks, 0.55 for
/// 60-card ones).
pub fn explain_with_threshold(
    problem: &mut Finding,
    stats: &SimStats,
    deck: &SimDeck,
    dead_cast_share: f64,
) {
    match problem.kind {
        "insufficient_color_mana" => color_screw_evidence(problem, stats),
        "low_castability" => dead_card_evidence(problem, stats, dead_cast_share),
        "insufficient_land_drops" => mana_screw_evidence(problem, deck),
        "excess_lands_seen" => mana_flood_evidence(problem, deck),
        "late_commander_cast" => commander_late_evidence(problem, stats, deck),
        _ => {}
    }
}

/// Color screw: name the cards whose pips in the missing color got
/// blocked most often, and make the suggestion name them.
fn color_screw_evidence(problem: &mut Finding, stats: &SimStats) {
    let Some(color) = problem.color else { return };
    let mut blocks: Vec<super::findings::PipBlock> = stats
        .pip_blocks
        .iter()
        .filter(|p| p.color == color)
        .cloned()
        .collect();
    // Worst blockers first; keep the top three.
    blocks.sort_by(|a, b| b.game_share.total_cmp(&a.game_share));
    blocks.truncate(3);
    if blocks.is_empty() {
        return;
    }
    problem.evidence = blocks
        .iter()
        .map(|p| FindingEvidence {
            name: p.name.clone(),
            explanation: format!("its {} pips could not be paid", plain_color(p.color)),
            game_share: Some(p.game_share),
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
fn dead_card_evidence(problem: &mut Finding, stats: &SimStats, threshold: f64) {
    // The detail already names the worst 3; mirror them as offenders with
    // their castability shares.
    let mut worst: Vec<&super::aggregate::CardCast> = stats
        .card_castability
        .iter()
        .filter(|c| c.pct_by_target < threshold)
        .collect();
    worst.sort_by(|a, b| a.pct_by_target.total_cmp(&b.pct_by_target));
    let mut seen = HashSet::new();
    problem.evidence = worst
        .into_iter()
        .filter(|c| seen.insert(c.name.clone()))
        .take(3)
        .map(|c| FindingEvidence {
            name: c.name.clone(),
            explanation: format!(
                "castable by turn {} in only {}% of games",
                c.target_turn,
                pct2(c.pct_by_target)
            ),
            game_share: Some(c.pct_by_target),
        })
        .collect();
}

/// Mana screw: the census labels behind the shortage (lands, rocks,
/// dorks) so the reader sees the deck's shape, not a bare percentage.
fn mana_screw_evidence(problem: &mut Finding, deck: &SimDeck) {
    let lands = deck
        .library_cards()
        .filter(|c| c.role == Role::Land)
        .count();
    let rocks = deck
        .library_cards()
        .filter(|c| c.role == Role::Rock)
        .count();
    let dorks = deck
        .library_cards()
        .filter(|c| c.role == Role::Dork)
        .count();
    problem.evidence.push(FindingEvidence {
        name: "lands".to_string(),
        explanation: format!("{lands} in the deck"),
        game_share: None,
    });
    if rocks + dorks > 0 {
        problem.evidence.push(FindingEvidence {
            name: "mana rocks and mana creatures".to_string(),
            explanation: format!("{rocks} rocks, {dorks} creatures"),
            game_share: None,
        });
    }
}

/// Mana flood: the land count against the typical band.
fn mana_flood_evidence(problem: &mut Finding, deck: &SimDeck) {
    let lands = deck
        .library_cards()
        .filter(|c| c.role == Role::Land)
        .count();
    problem.evidence.push(FindingEvidence {
        name: "lands".to_string(),
        explanation: format!("{lands} in the deck (typical commander decks run 32-38)"),
        game_share: None,
    });
}

/// Commander late: the commander's own cost and cast stats.
fn commander_late_evidence(problem: &mut Finding, stats: &SimStats, deck: &SimDeck) {
    let Some(cmd) = deck.commanders.first() else {
        return;
    };
    problem.evidence.push(FindingEvidence {
        name: cmd.name.clone(),
        explanation: format!(
            "costs {} mana; first castable on turn {} on average",
            cmd.cost.total(),
            turn2(stats.avg_commander_cast_turn)
        ),
        game_share: None,
    });
}

/// WUBRG letter as a spoken color.
fn plain_color(color: ManaColor) -> &'static str {
    color.name()
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
