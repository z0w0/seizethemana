// `stm deck diff`: exact change instructions between two decks.
//
// The base operand is a deck name (the original list); the target is a deck
// name or a ManaBox deck txt file (the optimized list). Per section, cards
// are multisets keyed by name (any printing fills a slot; `--exact` keys on
// the full print identity). Basics and quantity changes render as qty
// deltas ("Forest: 16 → 12"); non-basics as remove/add rows. `--markdown`
// renders the change-log instruction table for `decks/<name>.changes.md`.

use super::grammar::{Deck, DeckEntry};
use anyhow::Context;

/// One section's multiset diff.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SectionDiff {
    /// Section header (`COMMANDER`, `DECK`, `SIDEBOARD`).
    pub section: String,
    /// Cards fully removed from this section (any print key).
    pub removed: Vec<(String, i64)>,
    /// Cards added by this diff.
    pub added: Vec<(String, i64)>,
    /// Cards present in both but at different quantities.
    pub changed: Vec<(String, i64, i64)>,
}

impl SectionDiff {
    /// True when the two lists are identical.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.added.is_empty() && self.changed.is_empty()
    }
}

/// Aggregate key for a diff: name only (default) or the full print key.
fn aggregate_key(entry: &DeckEntry, exact: bool) -> String {
    if exact {
        entry.key()
    } else {
        entry.name.clone()
    }
}

/// The card name behind an aggregate key (strip the exact-key suffixes).
fn display_name(key: &str) -> String {
    key.split('|').next().unwrap_or(key).to_string()
}

/// Diff one pair of sections. Basics report as qty deltas; a card that
/// changes both quantity and presence stays in removed/added.
pub fn diff_section(
    section: &str,
    a: &[DeckEntry],
    b: &[DeckEntry],
    exact: bool,
    is_basic: impl Fn(&str) -> bool,
) -> SectionDiff {
    let count = |entries: &[DeckEntry]| -> std::collections::BTreeMap<String, i64> {
        let mut map: std::collections::BTreeMap<String, i64> = Default::default();
        for entry in entries {
            *map.entry(aggregate_key(entry, exact)).or_insert(0) += entry.quantity;
        }
        map
    };
    let (ca, cb) = (count(a), count(b));
    let mut diff = SectionDiff {
        section: section.to_string(),
        ..Default::default()
    };
    let mut changed: Vec<(String, i64, i64)> = Vec::new();
    for (key, qty_a) in &ca {
        let qty_b = cb.get(key).copied().unwrap_or(0);
        match qty_b.cmp(&0) {
            std::cmp::Ordering::Equal => diff.removed.push((key.clone(), *qty_a)),
            _ if qty_b == *qty_a => {}
            // Basics (and any exact-keyed card) collapse into a qty delta.
            _ => {
                if !exact && is_basic(key) {
                    diff.changed.push((key.clone(), *qty_a, qty_b));
                } else {
                    changed.push((key.clone(), *qty_a, qty_b));
                }
            }
        }
    }
    for (key, qty_b) in &cb {
        if !ca.contains_key(key) {
            diff.added.push((key.clone(), *qty_b));
        }
    }
    // Quantity changes are net deltas: one direction per card. A 2-for-2
    // basic swap (−2 Forest, +2 Island) reads as two Remove halves in the
    // delta list; the per-card transitions below are the exact record.
    for (key, qty_a, qty_b) in changed {
        if qty_a < qty_b {
            diff.added.push((key.clone(), qty_b - qty_a));
        } else {
            diff.removed.push((key.clone(), qty_a - qty_b));
        }
    }
    diff
}

/// Collapse same-named sections (case-insensitive) into one section with
/// summed quantities, preserving first-appearance order.
fn merge_named_sections(deck: &Deck) -> Deck {
    let mut sections: Vec<(String, Vec<DeckEntry>)> = Vec::new();
    for (section, entries) in &deck.sections {
        if let Some((_, merged)) = sections
            .iter_mut()
            .find(|(s, _)| s.eq_ignore_ascii_case(section))
        {
            for entry in entries {
                if let Some(pos) = merged.iter().position(|e| e.name == entry.name) {
                    merged[pos].quantity += entry.quantity;
                } else {
                    merged.push(entry.clone());
                }
            }
        } else {
            sections.push((section.clone(), entries.clone()));
        }
    }
    Deck { sections }
}

/// Full deck diff, section by section. Sections match case-insensitively
/// by header; same-named sections within one deck merge (quantities
/// aggregate) before diffing, and a section in only one deck diffs against
/// an empty list.
pub fn diff_decks(
    a: &Deck,
    b: &Deck,
    exact: bool,
    is_basic: impl Fn(&str) -> bool,
) -> Vec<SectionDiff> {
    let a = merge_named_sections(a);
    let b = merge_named_sections(b);
    let mut out = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    for (section, entries_a) in &a.sections {
        let b_index = (0..b.sections.len())
            .find(|&i| !used.contains(&i) && b.sections[i].0.eq_ignore_ascii_case(section));
        match b_index {
            Some(i) => {
                used.push(i);
                out.push(diff_section(
                    section,
                    entries_a,
                    &b.sections[i].1,
                    exact,
                    &is_basic,
                ));
            }
            None => out.push(diff_section(section, entries_a, &[], exact, &is_basic)),
        }
    }
    for (i, (section, entries_b)) in b.sections.iter().enumerate() {
        if used.contains(&i) {
            continue;
        }
        if a.sections
            .iter()
            .any(|(s, _)| s.eq_ignore_ascii_case(section))
        {
            // A same-named section already matched above.
            continue;
        }
        out.push(diff_section(section, &[], entries_b, exact, &is_basic));
    }
    out
}

/// Markdown change-log for a full diff: per-section instruction tables.
pub fn markdown(diff: &[SectionDiff]) -> String {
    let mut out = String::from("# Deck changes\n");
    for section in diff {
        if section.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {}\n\n", section.section));
        let basics: Vec<&(String, i64, i64)> = section
            .changed
            .iter()
            .filter(|(name, _, _)| is_basic_display(name))
            .collect();
        if !basics.is_empty() {
            // Spec: "Remove 4 Forests and 2 Islands" — per-basic deltas
            // with the removals first; adds read "Add …".
            let parts: Vec<String> = basics
                .iter()
                .map(|(n, a, b)| {
                    let delta = b - a;
                    let abs = delta.abs();
                    let plural = if abs == 1 { "" } else { "s" };
                    format!("{abs} {}{plural}", display_name(n))
                })
                .collect();
            // Mixed add/remove basics are ambiguous in one sentence; the
            // dominant direction names the row and the per-card table
            // below keeps the exact transitions.
            let added: i64 = basics.iter().map(|(_, a, b)| (b - a).max(0)).sum();
            let removed: i64 = basics.iter().map(|(_, a, b)| (a - b).max(0)).sum();
            let verb = if added > removed { "Add" } else { "Remove" };
            let list = match parts.len() {
                1 => parts[0].clone(),
                2 => format!("{} and {}", parts[0], parts[1]),
                _ => format!(
                    "{}, and {}",
                    parts[..parts.len() - 1].join(", "),
                    parts[parts.len() - 1]
                ),
            };
            out.push_str(&format!("{verb} {list}.\n\n"));
        }
        let removals: Vec<&(String, i64)> = section.removed.iter().collect();
        let additions: Vec<&(String, i64)> = section.added.iter().collect();
        if !removals.is_empty() || !additions.is_empty() {
            out.push_str("| Remove | Qty | | Add | Qty |\n");
            out.push_str("| --- | --- | --- | --- | --- |\n");
            let rows = removals.len().max(additions.len());
            for i in 0..rows {
                let rem = removals
                    .get(i)
                    .map(|(n, q)| format!("| {} | {q} ", display_name(n)))
                    .unwrap_or_else(|| "|  |  ".to_string());
                let add = additions
                    .get(i)
                    .map(|(n, q)| format!("| {} | {q} |", display_name(n)))
                    .unwrap_or("|  |  |".to_string());
                out.push_str(&format!("{rem}{add}\n"));
            }
            out.push('\n');
        }
    }
    out
}

/// True for the unlimited basic names (display-side; diff keys are bare
/// names outside `--exact`).
fn is_basic_display(name: &str) -> bool {
    matches!(
        name,
        "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" | "Wastes"
    ) || name.starts_with("Snow-Covered")
}

/// Output formats for `deck diff`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiffFormat {
    /// Human stdout table.
    #[default]
    Human,
    /// JSON payload.
    Json,
    /// Change-log markdown (`--markdown`).
    Markdown,
}

/// Entry point for `stm deck diff <deckA> <deckB-or-file>`.
pub fn diff(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    deck_a: &str,
    deck_b: &str,
    exact: bool,
    format: DiffFormat,
) -> anyhow::Result<i32> {
    // Both operands accept a deck name or a ManaBox txt file; deck names
    // win when both resolve.
    let load_operand = |s: &str| -> anyhow::Result<(String, Deck)> {
        match super::store::load_deck(paths, s) {
            Ok((_p, deck)) => Ok((format!("deck {s:?}"), deck)),
            Err(err) if !super::store::is_deck_not_found(&err) => Err(err),
            Err(_) => {
                let text = std::fs::read_to_string(s)
                    .with_context(|| format!("reading {s} (not a deck name or file)"))?;
                let deck = Deck::parse(&text).with_context(|| format!("parsing {s}"))?;
                Ok((format!("file {s}"), deck))
            }
        }
    };
    let (deck_a_label, deck_a_parsed) = load_operand(deck_a)?;
    // Target: a real deck first, then a ManaBox txt path.
    let (deck_b_label, deck_b_parsed) = load_operand(deck_b)?;
    let cards_by_name = super::stats::lookup_names(conn, &deck_a_parsed)?;
    let sections = diff_decks(&deck_a_parsed, &deck_b_parsed, exact, |name| {
        cards_by_name
            .get(name)
            .is_some_and(super::stats::is_basic_land)
    });

    if format == DiffFormat::Json {
        let payload: Vec<serde_json::Value> = sections
            .iter()
            .map(|s| {
                serde_json::json!({
                    "section": s.section,
                    "removed": s.removed.iter().map(|(n, q)| serde_json::json!({"name": display_name(n), "qty": q})).collect::<Vec<_>>(),
                    "added": s.added.iter().map(|(n, q)| serde_json::json!({"name": display_name(n), "qty": q})).collect::<Vec<_>>(),
                    "changed": s.changed.iter().map(|(n, a, b)| serde_json::json!({"name": display_name(n), "from": a, "to": b})).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(crate::cli::codes::OK);
    }
    if format == DiffFormat::Markdown {
        print!("{}", self::markdown(&sections));
        return Ok(crate::cli::codes::OK);
    }

    let styles = out.styles();
    println!(
        "{}  {}  {}",
        styles.header("Diff"),
        styles.dim(&deck_a_label),
        styles.dim(&format!("→ {deck_b_label}")),
    );
    let mut any = false;
    for s in &sections {
        if s.is_empty() {
            continue;
        }
        any = true;
        println!();
        println!("{}", styles.header(&format!("// {}", s.section)));
        if !s.removed.is_empty() {
            println!("  {}:", styles.dim("Removed"));
            for (name, qty) in &s.removed {
                println!(
                    "    - {:>2}  {}",
                    qty,
                    styles.card_name(&display_name(name))
                );
            }
        }
        if !s.added.is_empty() {
            println!("  {}:", styles.dim("Added"));
            for (name, qty) in &s.added {
                println!(
                    "    + {:>2}  {}",
                    qty,
                    styles.card_name(&display_name(name))
                );
            }
        }
        if !s.changed.is_empty() {
            println!("  {}:", styles.dim("Quantity changed"));
            for (name, from, to) in &s.changed {
                println!(
                    "    ~ {}  {} → {}",
                    styles.card_name(&display_name(name)),
                    from,
                    to
                );
            }
        }
    }
    if !any {
        println!("{}", styles.success("identical decks"));
        return Ok(crate::cli::codes::OK);
    }
    Ok(crate::cli::codes::OK)
}

#[cfg(test)]
#[path = "tests/diff_tests.rs"]
mod diff_tests;

/// `--as-update`: print the diff as `deck update --from` op lines. Removes
/// first, then quantity changes as `set`, then adds — applying the file
/// reproduces B from A.
pub fn diff_as_update(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    deck_a: &str,
    deck_b: &str,
    exact: bool,
) -> anyhow::Result<i32> {
    let load_operand = |s: &str| -> anyhow::Result<Deck> {
        match super::store::load_deck(paths, s) {
            Ok((_p, deck)) => Ok(deck),
            Err(err) if !super::store::is_deck_not_found(&err) => Err(err),
            Err(_) => {
                let text = std::fs::read_to_string(s)
                    .with_context(|| format!("reading {s} (not a deck name or file)"))?;
                Deck::parse(&text).with_context(|| format!("parsing {s}"))
            }
        }
    };
    let deck_a_parsed = load_operand(deck_a)?;
    let deck_b_parsed = load_operand(deck_b)?;
    let cards_by_name = super::stats::lookup_names(conn, &deck_a_parsed)?;
    let sections = diff_decks(&deck_a_parsed, &deck_b_parsed, exact, |name| {
        cards_by_name
            .get(name)
            .is_some_and(super::stats::is_basic_land)
    });
    let mut any = false;
    for s in &sections {
        // Non-DECK sections carry the `section:` prefix the op grammar
        // supports; unqualified ops default to DECK and would land
        // sideboard/commander changes in the wrong place.
        let prefix = if s.section.eq_ignore_ascii_case("DECK") {
            String::new()
        } else {
            format!("{}:", s.section)
        };
        for (name, qty) in &s.removed {
            any = true;
            println!("remove {prefix}{qty} {}", display_name(name));
        }
        for (name, from, to) in &s.changed {
            any = true;
            if *to == 0 {
                println!("remove {prefix}{from} {}", display_name(name));
            } else {
                println!("set {prefix}{to} {}", display_name(name));
            }
        }
        for (name, qty) in &s.added {
            any = true;
            println!("add {prefix}{qty} {}", display_name(name));
        }
    }
    if !any {
        out.print_note("The decks already match; nothing to apply.");
    }
    Ok(crate::cli::codes::OK)
}
