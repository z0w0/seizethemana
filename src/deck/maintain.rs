// Deck dedupe: collapse duplicate same-name lines, with the commander
// singleton cap. Split from `update.rs` to keep each file small.

use super::grammar::Deck;
use super::store::{load_deck, save_deck};

/// What `dedupe_deck` produced: the merged deck, the merged line count,
/// and the (card, copies) rows removed.
pub struct DedupeResult {
    /// Deck with duplicate lines collapsed.
    pub deck: Deck,
    /// Number of merged/collapsed lines.
    pub merged_lines: usize,
    /// (card name, copies removed) per merged line.
    pub merged_cards: Vec<(String, i64)>,
}

/// One merged card row in dedupe output.
#[derive(Debug, serde::Serialize)]
struct DedupeMergedCard {
    /// Card name.
    name: String,
    /// Copies removed from duplicate lines.
    copies: i64,
}

/// Typed response for `stm deck dedupe`.
#[derive(Debug, serde::Serialize)]
struct DedupeReport {
    /// Deck name.
    name: String,
    /// Duplicate lines merged.
    merged_lines: usize,
    /// Copies removed while merging.
    merged_copies: i64,
    /// Final deck card count.
    cards: i64,
    /// Per-card quantity changes.
    merged: Vec<DedupeMergedCard>,
}

/// Collapse duplicate lines within each section: entries with the same card
/// name merge into the first line, quantities summed.
///
/// In commander-shaped decks (a COMMANDER section), a non-basic merged line
/// is capped at one copy: the singleton rule makes any larger holding an
/// error, so extra copies collapse away instead of lingering in the file.
/// Basics, snow basics, and "any number of cards named X" oracle text keep
/// summed quantities. Print info (set/cn/foil) of the first line wins.
/// Returns `(deck, merged_line_count, merged_cards)`; the caller persists
/// and reports.
///
/// # Errors
/// Returns errors when SQLite cannot check singleton exemptions.
pub fn dedupe_deck(conn: &rusqlite::Connection, deck: &Deck) -> anyhow::Result<DedupeResult> {
    let commander = deck.section_index("COMMANDER").is_some();
    let mut out = Deck::default();
    let mut merged_lines = 0usize;
    let mut merged_cards: Vec<(String, i64)> = Vec::new();
    for (section, entries) in &deck.sections {
        let target = out.section_entries_mut(section);
        let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for entry in entries {
            if let Some(&pos) = index.get(&entry.name) {
                target[pos].quantity += entry.quantity;
                merged_lines += 1;
                merged_cards.push((entry.name.clone(), entry.quantity));
            } else {
                index.insert(entry.name.clone(), target.len());
                target.push(entry.clone());
            }
        }
        if commander {
            // Collapse over-limit lines: a single line holding 2+ copies of
            // a non-basic card is the same singleton breach as duplicate
            // lines, and dedupe is the repair command.
            let mut collapsed: Vec<(String, i64)> = Vec::new();
            for entry in target.iter_mut() {
                let excess = entry.quantity - 1;
                if excess > 0 && !super::update::is_singleton_exempt(conn, &entry.name)? {
                    entry.quantity = 1;
                    collapsed.push((entry.name.clone(), excess));
                }
            }
            for (name, qty) in collapsed {
                merged_cards.push((name, qty));
                merged_lines += 1;
            }
        }
    }
    Ok(DedupeResult {
        deck: out,
        merged_lines,
        merged_cards,
    })
}

/// Entry point for `stm deck dedupe <name>`.
///
/// Merges same-name lines per section (first line's print info wins) and
/// saves the deck. Exit 3 when the deck has no duplicates (nothing to do).
///
/// # Errors
/// Returns errors from loading or saving the deck, SQLite queries, or JSON serialization.
pub fn dedupe(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = load_deck(paths, name)?;
    let result = dedupe_deck(conn, &deck)?;
    let (deduped, merged_lines, merged_cards) =
        (result.deck, result.merged_lines, result.merged_cards);
    let merged_copies: i64 = total_merged(&merged_cards);
    if merged_lines == 0 {
        if json {
            // Same success shape either way: `merged` empty on a no-op, so
            // agents parse one contract. Exit 3 still flags "nothing done".
            println!(
                "{}",
                serde_json::to_string_pretty(&DedupeReport {
                    name: name.to_string(),
                    merged_lines: 0,
                    merged_copies: 0,
                    cards: deck.total(),
                    merged: Vec::new(),
                })?
            );
        } else {
            out.error("no duplicate lines; the deck is already one line per card");
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    save_deck(paths, name, &deduped)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&DedupeReport {
                name: name.to_string(),
                merged_lines,
                merged_copies,
                cards: deduped.total(),
                merged: merged_cards
                    .iter()
                    .map(|(card, copies)| DedupeMergedCard {
                        name: card.clone(),
                        copies: *copies,
                    })
                    .collect(),
            })?
        );
    } else {
        for (card, qty) in &merged_cards {
            out.status("Merged", &format!("{card} (+{qty} copies removed)"));
        }
        // Section-aware size, same as `deck update`.
        let mut size = format!("{} maindeck", deduped.maindeck_total());
        let sideboard = deduped.sideboard_total();
        if sideboard > 0 {
            use std::fmt::Write as _;
            let _ = write!(size, " + {sideboard} sideboard");
        }
        let maybeboard = deduped.maybeboard_total();
        if maybeboard > 0 {
            use std::fmt::Write as _;
            let _ = write!(size, " + {maybeboard} maybeboard");
        }
        let commander = deduped.commander_total();
        if commander > 0 {
            use std::fmt::Write as _;
            let _ = write!(size, " + {commander} commander");
        }
        out.finish(
            "Deduped",
            &format!("deck {name:?}: {merged_lines} line(s) merged (now {size})"),
            std::time::Duration::ZERO,
        );
    }
    Ok(crate::cli::codes::OK)
}

/// Total copies carried by merged-away lines.
fn total_merged(merged: &[(String, i64)]) -> i64 {
    merged.iter().map(|(_, q)| q).sum()
}
