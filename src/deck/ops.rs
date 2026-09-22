// Deck operations: the `DeckOp` verbs, their text parsing, and the
// pure `apply_ops` engine that mutates a `Deck`. Split out of
// `update.rs` to keep each file under the size limit.

use super::grammar::{Deck, DeckEntry};
use anyhow::Context;

/// One requested update operation.
#[derive(Debug, Clone, PartialEq)]
pub enum DeckOp {
    /// Add copies (`--add [section:]qty Name`).
    Add {
        section: Option<String>,
        entry: DeckEntry,
    },
    /// Remove a line entirely, or decrement by qty (`--remove [section:]qty Name`).
    Remove {
        section: Option<String>,
        entry: DeckEntry,
    },
    /// Set an exact quantity; 0 deletes the line (`--set [section:]qty Name`).
    Set {
        section: Option<String>,
        entry: DeckEntry,
    },
    /// Move copies between sections (`--move [section:]qty Name to:section`).
    /// Applied as an atomic remove-then-add so the net state (one copy, in
    /// the new section) is what every later check sees.
    Move {
        from: Option<String>,
        to: String,
        entry: DeckEntry,
    },
}

/// Parse one op spec: `[section:]qty Name [(SET) [cn]] [*F*]`.
///
/// The section prefix is matched when the token before the first space
/// contains a `:`; card names never contain colons in ManaBox exports.
/// `--set` allows qty 0 (delete the line); add/remove require qty > 0.
/// `--move` specs end with `to:<section>` (default DECK).
///
/// # Errors
/// Fails with a message naming the bad spec.
pub fn parse_op(kind: &str, spec: &str) -> anyhow::Result<DeckOp> {
    // `--move` carries a trailing `to:<section>` clause instead of the
    // qty+name body alone; split it off before the shared parsing.
    let (spec, to) = if kind == "move" {
        let (spec, to) = parse_move_target(spec);
        (spec, to)
    } else {
        (spec, "DECK".to_string())
    };
    let (section, body) = match spec.split_once(':') {
        Some((section, body)) => (Some(section.trim().to_string()), body.trim()),
        None => (None, spec),
    };
    let entry = if kind == "set" && body.trim() == "0" {
        // A bare `--set 0` has no card name; say so instead of the generic
        // "non-positive quantity" from parse_entry.
        anyhow::bail!("invalid --set spec {spec:?}: missing card name");
    } else if kind == "set" && body.trim_start().starts_with("0 ") {
        // `--set 0 Name` deletes the line; parse_entry rejects qty 0, so
        // parse the rest and force the quantity.
        let without_qty = body
            .split_once(' ')
            .with_context(|| format!("invalid --set spec {spec:?}: missing quantity"))?
            .1;
        let mut entry = super::grammar::parse_entry(&format!("1 {without_qty}"))
            .map_err(|e| e.context(format!("invalid --set spec {spec:?}")))?;
        entry.quantity = 0;
        entry
    } else {
        super::grammar::parse_entry(body)
            .map_err(|e| e.context(format!("invalid --{kind} spec {spec:?}")))?
    };
    Ok(match kind {
        "add" => DeckOp::Add { section, entry },
        "remove" => DeckOp::Remove { section, entry },
        "set" => DeckOp::Set { section, entry },
        "move" => DeckOp::Move {
            from: section,
            to,
            entry,
        },
        other => anyhow::bail!("unknown op kind {other:?}"),
    })
}

/// Split a trailing `to:<section>` off a move spec.
///
/// Returns `(spec_without_to, section)`. The `to:` clause is optional and
/// always last; DECK is the default target. A card name containing
/// " to:" followed by more words is left intact (only a trailing
/// single-word target splits).
fn parse_move_target(spec: &str) -> (&str, String) {
    let lower = spec.to_ascii_lowercase();
    if let Some(idx) = lower.rfind(" to:") {
        let target = spec[idx + 4..].trim();
        if !target.is_empty() && !target.contains(' ') {
            return (&spec[..idx], target.trim().to_string());
        }
    }
    (spec, "DECK".to_string())
}

/// Apply parsed ops to a deck, returning a summary of what changed.
pub fn apply_ops(deck: &mut Deck, ops: &[DeckOp]) -> anyhow::Result<DeckOpSummary> {
    let mut summary = DeckOpSummary::default();
    for op in ops {
        match op {
            DeckOp::Add { section, entry } => {
                let entries = deck.section_entries_mut(section.as_deref().unwrap_or("DECK"));
                match entry_position(entries, entry) {
                    Some(pos) => entries[pos].quantity += entry.quantity,
                    None => entries.push(entry.clone()),
                }
                summary.added += entry.quantity;
            }
            DeckOp::Remove { section, entry } => {
                match remove_from_section(deck, section.as_deref(), entry) {
                    RemoveOutcome::Removed(n) => summary.removed += n,
                    RemoveOutcome::Absent => {
                        // The deck already matches the requested end state;
                        // worth surfacing so typos are caught.
                        summary.missing.push(entry.name.clone());
                    }
                    RemoveOutcome::Elsewhere(actual_section, n) => {
                        // Found in a different section; say so instead of
                        // failing, so unqualified specs stay usable.
                        summary.removed += n;
                        summary.relocated.push((entry.name.clone(), actual_section));
                    }
                }
            }
            DeckOp::Set { section, entry } => {
                let entries = deck.section_entries_mut(section.as_deref().unwrap_or("DECK"));
                if entry.quantity == 0 {
                    // Deleting an already-absent line is a no-op: it
                    // changes nothing, so it does not count as a set.
                    if let Some(pos) = entry_position(entries, entry) {
                        entries.remove(pos);
                        summary.set += 1;
                    } else if section.is_none() {
                        // Unqualified: fall back to other sections, same
                        // identity rule as remove. A qualified set stays
                        // strict (the named section has no such line).
                        if let Some(section_name) = delete_elsewhere(deck, entry) {
                            summary.set += 1;
                            summary.relocated.push((entry.name.clone(), section_name));
                        }
                    }
                } else if let Some(pos) = entry_position(entries, entry) {
                    if entries[pos].quantity != entry.quantity {
                        entries[pos].quantity = entry.quantity;
                        summary.set += 1;
                    }
                } else {
                    entries.push(entry.clone());
                    summary.set += 1;
                }
            }
            DeckOp::Move { from, to, entry } => {
                // Atomic remove-then-add: the net state (one line, in the
                // target section) is what later ops and checks see. The
                // moved quantity is clamped to what the source actually
                // holds, so an oversized move never invents copies.
                let moved_qty = match remove_from_section(deck, from.as_deref(), entry) {
                    RemoveOutcome::Removed(n) => n,
                    RemoveOutcome::Absent => {
                        summary.missing.push(entry.name.clone());
                        continue;
                    }
                    RemoveOutcome::Elsewhere(actual, n) => {
                        summary.relocated.push((entry.name.clone(), actual));
                        n
                    }
                };
                if moved_qty == 0 {
                    continue;
                }
                summary.moved += moved_qty;
                let mut moved_entry = entry.clone();
                moved_entry.quantity = moved_qty;
                let entries = deck.section_entries_mut(to);
                match entry_position(entries, &moved_entry) {
                    Some(pos) => entries[pos].quantity += moved_qty,
                    None => entries.push(moved_entry),
                }
            }
        }
    }
    Ok(summary)
}

/// The position in `entries` matched by `op`'s entry: exact print key when
/// the op names one, otherwise the first entry with the same card name. A
/// name without print info targets the card in any printing — the same
/// identity rule the collection uses ("any print fills a slot").
fn entry_position(entries: &[DeckEntry], entry: &DeckEntry) -> Option<usize> {
    let carries_print = entry.set_code.is_some() || entry.foil || entry.collector_number.is_some();
    entries.iter().position(|e| {
        if carries_print {
            e.key() == entry.key()
        } else {
            e.name == entry.name
        }
    })
}

/// Remove/decrement the first matching entry.
///
/// An unqualified op targets `DECK` first, then falls back to any other
/// section holding the print (reported via [`RemoveOutcome::Elsewhere`]).
fn remove_from_section(deck: &mut Deck, section: Option<&str>, entry: &DeckEntry) -> RemoveOutcome {
    let target = section.unwrap_or("DECK");
    let entries = deck.section_entries_mut(target);
    if let Some(pos) = entry_position(entries, entry) {
        let actual = decrement_or_remove(entries, pos, entry.quantity);
        return RemoveOutcome::Removed(actual);
    }
    if section.is_some() {
        // Explicitly qualified: honor it strictly.
        return RemoveOutcome::Absent;
    }
    // Unqualified: search every other section for the print.
    for idx in 0..deck.sections.len() {
        let (name, entries) = &mut deck.sections[idx];
        if name.eq_ignore_ascii_case(target) {
            continue;
        }
        if let Some(pos) = entry_position(entries, entry) {
            let actual = decrement_or_remove(entries, pos, entry.quantity);
            return RemoveOutcome::Elsewhere(name.clone(), actual);
        }
    }
    RemoveOutcome::Absent
}

/// Take up to `requested` copies off the line; remove it when nothing
/// remains. Returns the copies actually taken.
fn decrement_or_remove(entries: &mut Vec<DeckEntry>, pos: usize, requested: i64) -> i64 {
    let existing = &mut entries[pos];
    if existing.quantity > requested {
        existing.quantity -= requested;
        requested
    } else {
        let actual = existing.quantity;
        entries.remove(pos);
        actual
    }
}

/// Result of one remove op.
enum RemoveOutcome {
    /// Removed (or decremented by) this many copies.
    Removed(i64),
    /// The print is not in the deck at all.
    Absent,
    /// The print lives in another section (named here) and `n` copies were
    /// removed there.
    Elsewhere(String, i64),
}

/// Delete the first line matching `entry` outside the default section.
/// Returns the section name it was deleted from, if any.
fn delete_elsewhere(deck: &mut Deck, entry: &DeckEntry) -> Option<String> {
    for idx in 0..deck.sections.len() {
        let (name, entries) = &mut deck.sections[idx];
        if name.eq_ignore_ascii_case("DECK") {
            continue;
        }
        if let Some(pos) = entry_position(entries, entry) {
            entries.remove(pos);
            return Some(name.clone());
        }
    }
    None
}

/// Result of applying update ops.
#[derive(Debug, Default, PartialEq)]
pub struct DeckOpSummary {
    /// Card copies added.
    pub added: i64,
    /// Card copies removed (actual copies taken off lines).
    pub removed: i64,
    /// Lines set to an exact quantity.
    pub set: usize,
    /// Card copies moved between sections.
    pub moved: i64,
    /// Names referenced by remove ops that were not in the deck.
    pub missing: Vec<String>,
    /// `(name, section)` for removals that hit another section than the
    /// unqualified default; surfaced as a note, not an error.
    pub relocated: Vec<(String, String)>,
}

/// Clap's usage-exit equivalent for our own arg errors.
pub const USAGE_EXIT: i32 = 2;
