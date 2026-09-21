//! The bracket checklist: the Game Changer allowance per bracket, the
//! bracket-signal scan ("this deck plays like a bracket N"), and the
//! Game Changer name table. Split from `legal.rs` to stay under the
//! module size limit; `super` is the legal module.

use super::grammar::Deck;
use super::legal::{BracketNote, Violation, is_game_changer, maindeck_copies_by_name};
use crate::db::CardRow;
use std::collections::HashMap;

/// Scan the deck's oracle text for the bracket's judgment-call signals.
///
/// Deterministic text search: library searchers (hard tutors vs soft
/// searchers), extra turns, mass land destruction, and "you win the game"
/// lines. Verdicts are PASS (no hits), CHECK (genuine bracket conflict),
/// or ADVISE (soft signal, official rules treat it as a judgment call).
/// Lines that start with any verdict prefix also land in the JSON
/// `advisories`/`notes` split. The Game Changer count is the hard check
/// elsewhere.
///
/// Checklist items listing the deck's own Game Changers, so the reader can
/// decide a bracket without re-querying every card.
pub(super) fn game_changer_checklist(deck: &Deck, cards: &HashMap<String, CardRow>) -> Vec<String> {
    let changers = game_changer_names(deck, cards);
    if changers.is_empty() {
        vec!["this deck has no Game Changers".to_string()]
    } else {
        vec![format!(
            "Game Changers in this deck: {}",
            changers.join(", ")
        )]
    }
}

/// Names of the deck's maindeck Game Changers, in first-seen deck order.
///
/// Sideboard Game Changers are excluded: the sideboard is a commander
/// wishlist, not part of the deck.
pub fn game_changer_names(deck: &Deck, cards: &HashMap<String, CardRow>) -> Vec<String> {
    maindeck_copies_by_name(deck)
        .into_iter()
        .filter(|(name, _)| cards.get(name).is_some_and(is_game_changer))
        .map(|(name, _)| name)
        .collect()
}

/// Human report on stdout.
///
/// Violations print as `error: <rule>: <detail>` with the offending card
/// names indented below; the non-deterministic checklist prints as a
/// `note:` block. Both are results on stdout, not stderr diagnostics —
/// the whole block is the command's answer.
#[allow(clippy::too_many_arguments)]
pub(super) fn print_report(
    out: &crate::output::Output,
    name: &str,
    format: &str,
    assumed: bool,
    bracket: Option<u8>,
    legal: bool,
    violations: &[Violation],
    note: &Option<BracketNote>,
    summary: &str,
) {
    let styles = out.styles();
    let mut format_line = format.to_string();
    if assumed {
        format_line.push_str(" (assumed; pass --format to override)");
    }
    println!(
        "{}  {}  {}",
        styles.header(name),
        styles.dim(&format_line),
        styles.dim(summary),
    );
    if let Some(bracket) = bracket {
        println!("  {} {bracket}", styles.dim("bracket"));
    }
    println!();
    if legal {
        println!("{}", styles.success("legal"));
    } else {
        println!(
            "{}",
            styles.error(&format!(
                "not legal ({} violation{})",
                violations.len(),
                if violations.len() == 1 { "" } else { "s" }
            ))
        );
        for v in violations {
            println!("  {}", styles.error(&format!("{}: {}", v.rule, v.detail)));
            for card in &v.cards {
                println!("    {}", styles.card_name(card));
            }
        }
    }
    if let Some(note) = note {
        println!();
        let has_verdicts = note
            .checks
            .iter()
            .any(|c| c.starts_with("PASS ") || c.starts_with("CHECK ") || c.starts_with("ADVISE "));
        if has_verdicts {
            println!("{}", styles.note("bracket checks:"));
            for check in &note.checks {
                if let Some(rest) = check.strip_prefix("PASS ") {
                    // Bare verdict glyph; success() would double the ✓.
                    println!(
                        "  {} {}",
                        styles.glyph("✓", crate::output::GlyphKind::Good),
                        rest
                    );
                } else if let Some(rest) = check.strip_prefix("CHECK ") {
                    println!(
                        "  {} {}",
                        styles.glyph("!", crate::output::GlyphKind::Warn),
                        rest
                    );
                } else if let Some(rest) = check.strip_prefix("ADVISE ") {
                    println!(
                        "  {} {}",
                        styles.glyph("ℹ", crate::output::GlyphKind::Info),
                        rest
                    );
                } else {
                    println!("  - {check}");
                }
            }
        } else {
            println!("{}", styles.note("not checked automatically:"));
            for check in &note.checks {
                println!("  - {check}");
            }
        }
    }
}
