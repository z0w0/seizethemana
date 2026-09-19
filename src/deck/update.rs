// Deck update ops (`--add`/`--remove`/`--set`/`--move`) and the
// `deck update` command.
//
// Ops are parsed from CLI specs (`[section:]qty Name [(SET) [cn]] [*F*]`)
// and applied to an in-memory deck; the caller persists the result.

use super::grammar::{self, Deck, DeckEntry};
use super::store::{load_deck, save_deck, valid_deck_name};
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
    let entry = if kind == "set" && body.trim_start().starts_with("0 ") {
        // `--set 0 Name` deletes the line; parse_entry rejects qty 0, so
        // parse the rest and force the quantity.
        let without_qty = body
            .split_once(' ')
            .with_context(|| format!("invalid --set spec {spec:?}: missing quantity"))?
            .1;
        let mut entry = grammar::parse_entry(&format!("1 {without_qty}"))
            .map_err(|e| e.context(format!("invalid --set spec {spec:?}")))?;
        entry.quantity = 0;
        entry
    } else {
        grammar::parse_entry(body)
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
                    RemoveOutcome::Elsewhere(actual_section) => {
                        // Found in a different section; say so instead of
                        // failing, so unqualified specs stay usable.
                        summary.relocated.push((entry.name.clone(), actual_section));
                    }
                }
            }
            DeckOp::Set { section, entry } => {
                let entries = deck.section_entries_mut(section.as_deref().unwrap_or("DECK"));
                if entry.quantity == 0 {
                    // Deleting an already-absent line is a no-op.
                    if let Some(pos) = entry_position(entries, entry) {
                        entries.remove(pos);
                        summary.set += 1;
                    } else if section.is_none() {
                        // Unqualified: fall back to other sections, same
                        // identity rule as remove. A qualified set stays
                        // strict (the named section has no such line).
                        match delete_elsewhere(deck, entry) {
                            Some(section_name) => {
                                summary.set += 1;
                                summary.relocated.push((entry.name.clone(), section_name));
                            }
                            None => summary.set += 1,
                        }
                    } else {
                        summary.set += 1;
                    }
                } else if let Some(pos) = entry_position(entries, entry) {
                    entries[pos].quantity = entry.quantity;
                    summary.set += 1;
                } else {
                    entries.push(entry.clone());
                    summary.set += 1;
                }
            }
            DeckOp::Move { from, to, entry } => {
                // Atomic remove-then-add: the net state (one line, in the
                // target section) is what later ops and checks see.
                match remove_from_section(deck, from.as_deref(), entry) {
                    RemoveOutcome::Removed(n) => summary.moved += n,
                    RemoveOutcome::Absent => {
                        summary.missing.push(entry.name.clone());
                        continue;
                    }
                    RemoveOutcome::Elsewhere(actual) => {
                        summary.moved += entry.quantity;
                        summary.relocated.push((entry.name.clone(), actual));
                    }
                }
                let entries = deck.section_entries_mut(to);
                match entry_position(entries, entry) {
                    Some(pos) => entries[pos].quantity += entry.quantity,
                    None => entries.push(entry.clone()),
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
        let existing = &mut entries[pos];
        if existing.quantity > entry.quantity {
            existing.quantity -= entry.quantity;
        } else {
            entries.remove(pos);
        }
        return RemoveOutcome::Removed(entry.quantity);
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
            let existing = &mut entries[pos];
            if existing.quantity > entry.quantity {
                existing.quantity -= entry.quantity;
            } else {
                entries.remove(pos);
            }
            return RemoveOutcome::Elsewhere(name.clone());
        }
    }
    RemoveOutcome::Absent
}

/// Result of one remove op.
enum RemoveOutcome {
    /// Removed (or decremented by) this many copies.
    Removed(i64),
    /// The print is not in the deck at all.
    Absent,
    /// The print lives in another section (named here) and was removed there.
    Elsewhere(String),
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
    /// Card copies removed (requested amount).
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

/// Outcome of resolving one op's card name against the oracle.
enum NameCheck {
    /// Card found; nothing to report.
    Ok,
    /// The name matches a known token/art piece, not an oracle card.
    Token,
    /// Nothing matched; carries the candidate names for the error hint.
    Unknown(Vec<String>),
}

/// Resolve an op's card name against the oracle.
///
/// # Errors
/// Propagates SQLite failures.
fn check_name(conn: &rusqlite::Connection, name: &str) -> anyhow::Result<NameCheck> {
    use crate::db::NameMatch;
    match crate::db::resolve_name(conn, name)? {
        NameMatch::Found(_) => Ok(NameCheck::Ok),
        NameMatch::Ambiguous(candidates) => Ok(NameCheck::Unknown(candidates)),
        NameMatch::NotFound => {
            if crate::db::is_token_name(conn, name)? {
                Ok(NameCheck::Token)
            } else {
                Ok(NameCheck::Unknown(Vec::new()))
            }
        }
    }
}

/// Validate every add/set op's card name against the oracle.
///
/// Unknown names fail with exit 3 and name the offenders; token names only
/// warn (the op proceeds). Removal and deletion targets are not checked: a
/// deck may legitimately reference cards the oracle lacks.
///
/// # Errors
/// Propagates SQLite failures.
fn validate_names(
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    ops: &[DeckOp],
) -> anyhow::Result<Option<i32>> {
    let mut unknown: Vec<String> = Vec::new();
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    for op in ops {
        let (name, is_zero_set) = match op {
            DeckOp::Add { entry, .. } => (&entry.name, false),
            DeckOp::Set { entry, .. } => (&entry.name, entry.quantity == 0),
            // `--remove` of a card missing from the oracle is fine; the
            // missing-from-deck path reports it. Moves target a card the
            // deck already holds; oracle validation is unnecessary.
            DeckOp::Remove { .. } | DeckOp::Move { .. } => continue,
        };
        // `--set 0 X` deletes a line; the name needs no oracle check.
        if is_zero_set {
            continue;
        }
        match check_name(conn, name)? {
            NameCheck::Ok => {}
            NameCheck::Token => out.warning(&format!(
                "{name} is a token or non-card piece, not an oracle card; added anyway"
            )),
            NameCheck::Unknown(hints) => {
                unknown.push(name.clone());
                if !hints.is_empty() {
                    candidates.push((name.clone(), hints));
                }
            }
        }
    }
    if unknown.is_empty() {
        return Ok(None);
    }
    out.error(&format!(
        "{} of the referenced cards are not in the oracle: {}",
        unknown.len(),
        unknown.join(", ")
    ));
    for (name, hints) in &candidates {
        out.hint(&format!("did you mean for {name:?}: {}", hints.join(", ")));
    }
    out.hint("card names resolve by exact match, case, or unique prefix");
    Ok(Some(crate::cli::codes::NO_RESULTS))
}

/// Entry point for `stm deck update <name>`.
///
/// `from_file` (when given) is a text file of extra specs, one per line;
/// blank lines and `#` comments are skipped. Specs from `--add`/`--remove`/
/// `--set`/`--move` run first, then the file's. With `allow_partial`,
/// remove ops that miss the deck are reported and skipped instead of
/// aborting the whole batch (exit 1 still signals that something missed).
#[allow(clippy::too_many_arguments)]
pub fn update(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    add: &[String],
    remove: &[String],
    set: &[String],
    move_specs: &[String],
    from: Option<&std::path::Path>,
    allow_partial: bool,
) -> anyhow::Result<i32> {
    let mut ops = Vec::new();
    for spec in add {
        ops.push(parse_op("add", spec)?);
    }
    for spec in remove {
        ops.push(parse_op("remove", spec)?);
    }
    for spec in set {
        ops.push(parse_op("set", spec)?);
    }
    for spec in move_specs {
        ops.push(parse_op("move", spec)?);
    }
    if let Some(file) = from {
        let text =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let mut file_ops = 0usize;
        for raw in text.lines() {
            let spec = raw.trim();
            if spec.is_empty() || spec.starts_with('#') {
                continue;
            }
            ops.push(parse_spec_line(spec)?);
            file_ops += 1;
        }
        if file_ops == 0 {
            out.error(&format!("no specs found in {}", file.display()));
            out.hint(
                "one op per line: 'add 1 Name', 'remove 1 Name', 'set 2 Name', \
                 'move 1 Name to:sideboard', or a bare spec (treated as add)",
            );
            return Ok(USAGE_EXIT);
        }
    }
    if ops.is_empty() {
        out.error("no update operations given");
        out.hint("pass --add/--remove/--set/--move specs, e.g. --add '2 Bolt'");
        return Ok(USAGE_EXIT);
    }
    if !valid_deck_name(name) {
        out.error(&format!("invalid deck name {name:?}"));
        out.hint("use letters, digits, spaces, or - _ ' & ! + , (no / or ..)");
        anyhow::bail!("invalid deck name");
    }
    let (_path, mut deck) = load_deck(paths, name)?;
    if let Some(code) = validate_names(conn, out, &ops)? {
        return Ok(code);
    }
    // Singleton guard for adds: in a commander-shaped deck an add that
    // pushes a non-basic card past one copy is rejected up front instead
    // of writing an illegal deck. Set and Move ops keep the post-apply
    // warning path (an exact-quantity request is deliberate; a move nets
    // to zero new copies).
    if let Some(code) = reject_singleton_adds(out, &deck, &ops) {
        return Ok(code);
    }
    let mut summary = apply_ops(&mut deck, &ops)?;
    let mut had_missing = false;
    if !summary.missing.is_empty() {
        out.error(&format!(
            "{} of the referenced cards are not in the deck: {}",
            summary.missing.len(),
            summary.missing.join(", ")
        ));
        if allow_partial {
            // Partial mode: the applied ops already mutated the deck
            // in place; report and continue past the misses. The exit
            // code still flags the incomplete batch.
            out.warning("continuing without the missing cards (--allow-partial)");
            summary.missing.clear();
            had_missing = true;
        } else {
            out.hint("show the deck first: stm deck show");
            out.hint("apply the resolvable ops anyway with --allow-partial");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
    }
    // Singleton guard: commander-shape decks should hold one copy per
    // non-basic card. Computed against the post-apply deck so a legal
    // remove+add pair (net one copy) does not warn; the write still
    // happens so agents keep flowing, and `deck legal` reports the real
    // violation.
    let warnings = singleton_warnings(&ops, &deck);
    for w in &warnings {
        out.warning(w);
    }
    save_deck(paths, name, &deck)?;
    let mut parts = Vec::new();
    if summary.added > 0 {
        parts.push(format!("+{}", summary.added));
    }
    if summary.removed > 0 {
        parts.push(format!("-{}", summary.removed));
    }
    if summary.moved > 0 {
        parts.push(format!("{} moved", summary.moved));
    }
    if summary.set > 0 {
        parts.push(format!("{} set", summary.set));
    }
    for (card, section) in &summary.relocated {
        out.warning(&format!("{card} was in {section}; removed it there"));
    }
    let exit = if had_missing {
        crate::cli::codes::ERROR
    } else {
        crate::cli::codes::OK
    };
    out.finish(
        "Updated",
        &format!(
            "deck {name:?}: {} (now {} cards)",
            parts.join(", "),
            deck.total()
        ),
        std::time::Duration::ZERO,
    );
    Ok(exit)
}

/// Parse one line from a `--from` spec file.
///
/// Lines are `add <spec>`, `remove <spec>`, `set <spec>`,
/// `move <spec> to:<section>`, or a bare spec (treated as `add`).
///
/// # Errors
/// Fails with the line content for unknown verbs or bad specs.
fn parse_spec_line(line: &str) -> anyhow::Result<DeckOp> {
    let (kind, body) = match line.split_once(' ') {
        Some(("add", rest)) => ("add", rest),
        Some(("remove", rest)) => ("remove", rest),
        Some(("set", rest)) => ("set", rest),
        Some(("move", rest)) => ("move", rest),
        _ => ("add", line),
    };
    parse_op(kind, body.trim_start())
}

/// Op kinds the singleton guard distinguishes: Set pins an exact count;
/// Add and Move both read the post-apply deck's actual holdings.
#[derive(Clone, Copy)]
enum OpKind {
    Add,
    Set,
    Move,
}

/// Reject add ops that would push a non-basic card past one copy in a
/// commander-shaped deck.
///
/// The pre-apply deck state decides: an add is incremental by intent, so
/// incrementing an existing line past one copy is a batch error, not a
/// warning. Returns the exit code to use when at least one add must be
/// rejected, or `None` when every add is singleton-safe (or the deck is
/// not commander-shaped).
fn reject_singleton_adds(
    out: &mut crate::output::Output,
    deck: &Deck,
    ops: &[DeckOp],
) -> Option<i32> {
    deck.section_index("COMMANDER")?;
    let mut rejected: Vec<String> = Vec::new();
    for op in ops {
        let DeckOp::Add { section, entry } = op else {
            continue;
        };
        if is_unlimited_basics(&entry.name) || rejected.contains(&entry.name) {
            continue;
        }
        let target = section.as_deref().unwrap_or("DECK");
        let held_in_target = deck
            .sections
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(target))
            .map(|(_, entries)| {
                entries
                    .iter()
                    .filter(|e| e.name == entry.name)
                    .map(|e| e.quantity)
                    .sum::<i64>()
            })
            .unwrap_or(0);
        let held_elsewhere = deck
            .entries()
            .filter(|e| e.name == entry.name)
            .map(|e| e.quantity)
            .sum::<i64>()
            - held_in_target;
        if held_in_target + entry.quantity + held_elsewhere > 1 {
            rejected.push(entry.name.clone());
        }
    }
    if rejected.is_empty() {
        return None;
    }
    out.error(&format!(
        "{} of the adds would exceed the singleton limit: {}",
        rejected.len(),
        rejected.join(", ")
    ));
    for name in &rejected {
        out.hint(&format!(
            "{name} is already in the deck; use --set to change its quantity, \
             or --allow-partial to skip this add"
        ));
    }
    Some(crate::cli::codes::NO_RESULTS)
}

/// Warn when an op would push a non-basic card past one copy in a
/// commander-shaped deck (a COMMANDER section present).
///
/// Called against the post-apply deck: a legal remove+add pair nets to one
/// copy and must not warn. Returns one warning per affected card name.
/// The deck is still written: the warning is a nudge, and `deck legal`
/// reports the real violation.
fn singleton_warnings(ops: &[DeckOp], deck: &Deck) -> Vec<String> {
    if deck.section_index("COMMANDER").is_none() {
        return Vec::new();
    }
    let mut warned: Vec<String> = Vec::new();
    for op in ops {
        let (entry, kind) = match op {
            DeckOp::Add { entry, .. } => (entry, OpKind::Add),
            DeckOp::Set { entry, .. } => (entry, OpKind::Set),
            // Post-apply: a move has already landed; the deck's total copy
            // count is unchanged, so only a genuine multi-copy hold warns.
            DeckOp::Move { entry, .. } => (entry, OpKind::Move),
            DeckOp::Remove { .. } => continue,
        };
        if warned.contains(&entry.name) {
            continue;
        }
        // Sum across sections: a commander deck holds one copy total, so
        // a move that splits 2 copies as 1+1 across sections still
        // breaches the singleton rule.
        let held = deck
            .entries()
            .filter(|e| e.name == entry.name)
            .map(|e| e.quantity)
            .sum();
        let end_state = match kind {
            OpKind::Set => entry.quantity,
            // Add and Move both read the post-apply deck: what it now
            // holds is the only state worth checking.
            OpKind::Add | OpKind::Move => held,
        };
        if end_state <= 1 || is_unlimited_basics(&entry.name) {
            continue;
        }
        warned.push(entry.name.clone());
    }
    warned
        .iter()
        .map(|name| {
            format!("{name} would exceed the singleton limit; commander decks hold one copy")
        })
        .collect()
}

/// True for the names that never break singleton (basics and snow basics;
/// "any number of cards named X" cards are rare enough that `deck legal`
/// is the authority).
fn is_unlimited_basics(name: &str) -> bool {
    matches!(
        name,
        "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" | "Wastes"
    ) || name.starts_with("Snow-Covered")
}

/// Collapse duplicate lines within each section: entries with the same card
/// name merge into the first line, quantities summed.
///
/// In commander-shaped decks (a COMMANDER section), a non-basic merged line
/// is capped at one copy: the singleton rule makes any larger holding an
/// error, so extra copies collapse away instead of lingering in the file.
/// Basics and 60-card-style decks (no COMMANDER section) keep summed
/// quantities. Print info (set/cn/foil) of the first line wins. Returns
/// `(deck, merged_line_count, merged_cards)`; the caller persists and
/// reports.
pub fn dedupe_deck(deck: &Deck) -> (Deck, usize, Vec<(String, i64)>) {
    let commander = deck.section_index("COMMANDER").is_some();
    let mut out = Deck::default();
    let mut merged_lines = 0usize;
    let mut merged_cards: Vec<(String, i64)> = Vec::new();
    for (section, entries) in &deck.sections {
        let target = out.section_entries_mut(section);
        let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for entry in entries {
            match index.get(&entry.name) {
                Some(&pos) => {
                    target[pos].quantity += entry.quantity;
                    merged_lines += 1;
                    merged_cards.push((entry.name.clone(), entry.quantity));
                }
                None => {
                    index.insert(entry.name.clone(), target.len());
                    target.push(entry.clone());
                }
            }
        }
        if commander {
            // Collapse over-limit lines: a single line holding 2+ copies of
            // a non-basic card is the same singleton breach as duplicate
            // lines, and dedupe is the repair command.
            let mut collapsed: Vec<(String, i64)> = Vec::new();
            for entry in target.iter_mut() {
                let excess = entry.quantity - 1;
                if excess > 0 && !is_unlimited_basics(&entry.name) {
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
    (out, merged_lines, merged_cards)
}

/// Entry point for `stm deck dedupe <name>`.
///
/// Merges same-name lines per section (first line's print info wins) and
/// saves the deck. Exit 3 when the deck has no duplicates (nothing to do).
pub fn dedupe(
    paths: &crate::paths::Paths,
    out: &mut crate::output::Output,
    name: &str,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = load_deck(paths, name)?;
    let (deduped, merged_lines, merged_cards) = dedupe_deck(&deck);
    let merged_copies: i64 = total_merged(&merged_cards);
    if merged_lines == 0 {
        if json {
            // Same success shape either way: `merged` empty on a no-op, so
            // agents parse one contract. Exit 3 still flags "nothing done".
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "name": name,
                    "merged_lines": 0,
                    "merged_copies": 0,
                    "cards": deck.total(),
                    "merged": [],
                }))?
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
            serde_json::to_string_pretty(&serde_json::json!({
                "name": name,
                "merged_lines": merged_lines,
                "merged_copies": merged_copies,
                "cards": deduped.total(),
                "merged": merged_cards
                    .iter()
                    .map(|(card, qty)| serde_json::json!({"name": card, "copies": qty}))
                    .collect::<Vec<_>>(),
            }))?
        );
    } else {
        for (card, qty) in &merged_cards {
            out.status("Merged", &format!("{card} (+{qty} copies removed)"));
        }
        out.finish(
            "Deduped",
            &format!(
                "deck {name:?}: {merged_lines} line(s) merged (now {} cards)",
                deduped.total()
            ),
            std::time::Duration::ZERO,
        );
    }
    Ok(crate::cli::codes::OK)
}

/// Total copies carried by merged-away lines.
fn total_merged(merged: &[(String, i64)]) -> i64 {
    merged.iter().map(|(_, q)| q).sum()
}

#[cfg(test)]
#[path = "tests/update_tests.rs"]
mod update_tests;
