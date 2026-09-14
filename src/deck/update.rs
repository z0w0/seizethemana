// Deck update ops (`--add`/`--remove`/`--set`) and the `deck update` command.
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
}

/// Parse one op spec: `[section:]qty Name [(SET) [cn]] [*F*]`.
///
/// The section prefix is matched when the token before the first space
/// contains a `:`; card names never contain colons in ManaBox exports.
/// `--set` allows qty 0 (delete the line); add/remove require qty > 0.
///
/// # Errors
/// Fails with a message naming the bad spec.
pub fn parse_op(kind: &str, spec: &str) -> anyhow::Result<DeckOp> {
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
        other => anyhow::bail!("unknown op kind {other:?}"),
    })
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
                    }
                    summary.set += 1;
                } else if let Some(pos) = entry_position(entries, entry) {
                    entries[pos].quantity = entry.quantity;
                    summary.set += 1;
                } else {
                    entries.push(entry.clone());
                    summary.set += 1;
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

/// Result of applying update ops.
#[derive(Debug, Default, PartialEq)]
pub struct DeckOpSummary {
    /// Card copies added.
    pub added: i64,
    /// Card copies removed (requested amount).
    pub removed: i64,
    /// Lines set to an exact quantity.
    pub set: usize,
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
            // missing-from-deck path reports it.
            DeckOp::Remove { .. } => continue,
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
/// `--set` run first, then the file's.
#[allow(clippy::too_many_arguments)]
pub fn update(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    add: &[String],
    remove: &[String],
    set: &[String],
    from: Option<&std::path::Path>,
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
                 or a bare spec (treated as add)",
            );
            return Ok(USAGE_EXIT);
        }
    }
    if ops.is_empty() {
        out.error("no update operations given");
        out.hint("pass --add/--remove/--set specs, e.g. --add '2 Bolt'");
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
    // Singleton guard: commander-shape decks should hold one copy per
    // non-basic card. Compute against the pre-apply deck so a first-copy
    // add does not self-warn; the write still happens so agents keep
    // flowing, and `deck legal` reports the real violation.
    let warnings = singleton_warnings(&ops, &deck);
    let summary = apply_ops(&mut deck, &ops)?;
    if !summary.missing.is_empty() {
        out.error(&format!(
            "{} of the referenced cards are not in the deck: {}",
            summary.missing.len(),
            summary.missing.join(", ")
        ));
        out.hint("show the deck first: stm deck show");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
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
    if summary.set > 0 {
        parts.push(format!("{} set", summary.set));
    }
    for (card, section) in &summary.relocated {
        out.warning(&format!("{card} was in {section}; removed it there"));
    }
    out.finish(
        "Updated",
        &format!(
            "deck {name:?}: {} (now {} cards)",
            parts.join(", "),
            deck.total()
        ),
        std::time::Duration::ZERO,
    );
    Ok(crate::cli::codes::OK)
}

/// Parse one line from a `--from` spec file.
///
/// Lines are `add <spec>`, `remove <spec>`, `set <spec>`, or a bare spec
/// (treated as `add`).
///
/// # Errors
/// Fails with the line content for unknown verbs or bad specs.
fn parse_spec_line(line: &str) -> anyhow::Result<DeckOp> {
    let (kind, body) = match line.split_once(' ') {
        Some(("add", rest)) => ("add", rest),
        Some(("remove", rest)) => ("remove", rest),
        Some(("set", rest)) => ("set", rest),
        _ => ("add", line),
    };
    parse_op(kind, body.trim_start())
}

/// Warn when an op would push a non-basic card past one copy in a
/// commander-shaped deck (a COMMANDER section present).
///
/// Returns one warning per affected card name. The deck is still written:
/// the warning is a nudge, and `deck legal` reports the real violation.
fn singleton_warnings(ops: &[DeckOp], deck: &Deck) -> Vec<String> {
    if deck.section_index("COMMANDER").is_none() {
        return Vec::new();
    }
    let mut warned: Vec<String> = Vec::new();
    for op in ops {
        let (entry, kind) = match op {
            DeckOp::Add { entry, .. } => (entry, "add"),
            DeckOp::Set { entry, .. } => (entry, "set"),
            DeckOp::Remove { .. } => continue,
        };
        if warned.contains(&entry.name) {
            continue;
        }
        let existing = deck
            .entries()
            .filter(|e| e.name == entry.name)
            .map(|e| e.quantity)
            .max()
            .unwrap_or(0);
        let end_state = match kind {
            "set" => entry.quantity,
            _ => existing + entry.quantity,
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
/// Print info (set/cn/foil) of the first line wins; the merged line drops
/// nothing. Returns `(deck, merged_line_count, merged_cards)`; the caller
/// persists and reports.
pub fn dedupe_deck(deck: &Deck) -> (Deck, usize, Vec<(String, i64)>) {
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
            println!(
                "{}",
                serde_json::json!({
                    "name": name,
                    "duplicates": 0,
                    "merged_lines": 0,
                    "merged_copies": 0,
                    "cards": deck.total(),
                })
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
            serde_json::json!({
                "name": name,
                "merged_lines": merged_lines,
                "merged_copies": merged_copies,
                "cards": deduped.total(),
                "merged": merged_cards,
            })
        );
    } else {
        for (card, qty) in &merged_cards {
            out.status(
                "Merged",
                &format!("{card} (+{qty} copies onto its first line)"),
            );
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
mod tests {
    use super::*;

    /// Connection plus its backing tempdir (must outlive the connection).
    fn seeded_conn() -> (tempfile::TempDir, rusqlite::Connection) {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        conn.execute(
            "INSERT INTO cards (name, oracle_id, released_at) VALUES ('Lightning Bolt', 'oid', '2020-01-01')",
            [],
        )
        .unwrap();
        (tmp, conn)
    }

    fn silent_out() -> crate::output::Output {
        crate::output::Output::new(true, false, false)
    }

    #[test]
    fn validation_flags_unknown_names() {
        let (_tmp, conn) = seeded_conn();
        let mut out = silent_out();
        let ops = vec![parse_op("add", "1 Not A Real Card").unwrap()];
        let code = validate_names(&conn, &mut out, &ops).unwrap();
        assert_eq!(code, Some(crate::cli::codes::NO_RESULTS));
    }

    #[test]
    fn validation_passes_known_names() {
        let (_tmp, conn) = seeded_conn();
        let mut out = silent_out();
        let ops = vec![parse_op("add", "1 lightning bolt").unwrap()];
        assert_eq!(validate_names(&conn, &mut out, &ops).unwrap(), None);
        // Unique-prefix resolution counts too.
        let ops = vec![parse_op("set", "2 Lightning B").unwrap()];
        assert_eq!(validate_names(&conn, &mut out, &ops).unwrap(), None);
    }

    #[test]
    fn validation_skips_removes_and_zero_sets() {
        let (_tmp, conn) = seeded_conn();
        let mut out = silent_out();
        // Removing a name that is not a card at all is allowed (deleting a
        // stale line); `--set 0` deletes likewise.
        let ops = vec![
            parse_op("remove", "1 Not A Card").unwrap(),
            parse_op("set", "0 Also Not A Card").unwrap(),
        ];
        assert_eq!(validate_names(&conn, &mut out, &ops).unwrap(), None);
    }

    #[test]
    fn parse_op_variants() {
        let op = parse_op("add", "2 Lightning Bolt").unwrap();
        match op {
            DeckOp::Add { section, entry } => {
                assert_eq!(section, None);
                assert_eq!(entry.quantity, 2);
                assert_eq!(entry.name, "Lightning Bolt");
            }
            other => panic!("unexpected {other:?}"),
        }
        let op = parse_op("add", "commander:1 Breya (MH3) 372 *F*").unwrap();
        match op {
            DeckOp::Add { section, entry } => {
                assert_eq!(section.as_deref(), Some("commander"));
                assert_eq!(entry.name, "Breya");
                assert!(entry.foil);
            }
            other => panic!("unexpected {other:?}"),
        }
        let op = parse_op("remove", "2 Bolt").unwrap();
        assert!(matches!(op, DeckOp::Remove { .. }));
        let op = parse_op("set", "4 Bolt (TST) 1").unwrap();
        assert!(matches!(op, DeckOp::Set { .. }));
        assert!(parse_op("add", "Bolt").is_err());
        assert!(parse_op("nope", "1 Bolt").is_err());
    }

    #[test]
    fn ops_do_increment_and_decrement_math() {
        let mut deck = Deck::parse("// DECK\n2 Bolt\n1 Breya\n").unwrap();
        let ops = vec![
            parse_op("add", "2 Bolt").unwrap(),
            parse_op("remove", "1 Breya").unwrap(),
            parse_op("set", "4 Bolt").unwrap(),
            parse_op("set", "0 Breya").unwrap(),
        ];
        let summary = apply_ops(&mut deck, &ops).unwrap();
        assert_eq!(summary.added, 2);
        assert_eq!(summary.removed, 1);
        assert_eq!(summary.set, 2);
        assert!(summary.missing.is_empty());
        // Bolt was +2 then set to 4; Breya deleted by set 0.
        assert_eq!(deck.total(), 4);
        assert!(deck.entries().all(|e| e.name != "Breya"));
        assert_eq!(deck.to_text(), "// DECK\n4 Bolt\n");
    }

    #[test]
    fn ops_remove_line_when_decrementing_to_zero() {
        let mut deck = Deck::parse("// DECK\n1 Bolt\n").unwrap();
        let ops = vec![parse_op("remove", "1 Bolt").unwrap()];
        let summary = apply_ops(&mut deck, &ops).unwrap();
        assert_eq!(summary.removed, 1);
        assert_eq!(deck.total(), 0);
    }

    #[test]
    fn ops_report_missing_cards() {
        let mut deck = Deck::parse("// DECK\n1 Bolt\n").unwrap();
        let ops = vec![parse_op("remove", "1 Nope").unwrap()];
        let summary = apply_ops(&mut deck, &ops).unwrap();
        assert_eq!(summary.missing, vec!["Nope"]);
    }

    #[test]
    fn ops_target_the_named_section() {
        let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// SIDEBOARD\n1 Bolt\n").unwrap();
        let ops = vec![parse_op("add", "sideboard:1 Bare").unwrap()];
        let _ = apply_ops(&mut deck, &ops).unwrap();
        assert_eq!(deck.sections[1].1.len(), 2);
        // Existing section match is case-insensitive; unknown sections append.
        assert_eq!(deck.section_index("SIDEBOARD"), Some(1));
    }

    #[test]
    fn name_only_ops_match_lines_with_print_info() {
        // A spec without (SET) cn targets the card in any printing — the
        // collection's "any print fills a slot" identity, not exact prints.
        let mut deck =
            Deck::parse("// DECK\n1 Weapons Manufacturing (EOE) 311\n1 Island (SOS) 274\n")
                .unwrap();
        let ops = vec![parse_op("remove", "1 Weapons Manufacturing").unwrap()];
        let summary = apply_ops(&mut deck, &ops).unwrap();
        assert_eq!(summary.removed, 1);
        assert!(summary.missing.is_empty());
        assert_eq!(deck.total(), 1);

        let ops = vec![parse_op("set", "0 Island").unwrap()];
        let summary = apply_ops(&mut deck, &ops).unwrap();
        assert_eq!(summary.set, 1);
        assert!(summary.missing.is_empty());
        assert_eq!(deck.total(), 0);
    }

    #[test]
    fn print_qualified_ops_stay_print_exact() {
        // An op that names a print must not hit a line with a different one.
        let mut deck = Deck::parse("// DECK\n1 Bolt (M11) 148\n1 Bolt (2XM) 124\n").unwrap();
        let ops = vec![parse_op("remove", "1 Bolt (M11) 148").unwrap()];
        let summary = apply_ops(&mut deck, &ops).unwrap();
        assert_eq!(summary.removed, 1);
        assert_eq!(deck.total(), 1);
        let remaining: Vec<_> = deck.entries().collect();
        assert_eq!(remaining[0].set_code.as_deref(), Some("2XM"));
    }

    #[test]
    fn dedupe_merges_same_name_lines_per_section() {
        let deck = Deck::parse(
            "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n2 Bolt (M11) 148\n3 Bolt\n// SIDEBOARD\n2 Bolt\n",
        )
        .unwrap();
        let (mut out, merged, cards) = dedupe_deck(&deck);
        assert_eq!(merged, 2, "two duplicates merged");
        assert_eq!(
            cards,
            vec![("Bolt".to_string(), 2), ("Bolt".to_string(), 3)]
        );
        assert_eq!(out.total(), 9); // 1 Breya + 6 Bolt + 2 sideboard Bolt
        // Quantities sum; first line's print info wins.
        let deck_entries: Vec<_> = out.section_entries_mut("DECK").clone();
        assert_eq!(deck_entries[0].name, "Bolt");
        assert_eq!(deck_entries[0].quantity, 6);
        assert_eq!(deck_entries[0].set_code, None, "first line had no print");
        assert_eq!(
            out.to_text(),
            "// COMMANDER\n1 Breya\n\n// DECK\n6 Bolt\n\n// SIDEBOARD\n2 Bolt\n"
        );
    }

    #[test]
    fn dedupe_keeps_distinct_prints_when_names_differ() {
        // Different names never merge, even with identical other fields.
        let deck = Deck::parse("// DECK\n1 Bolt\n1 Shock\n").unwrap();
        let (out, merged, _) = dedupe_deck(&deck);
        assert_eq!(merged, 0);
        assert_eq!(out.total(), 2);
    }

    #[test]
    fn singleton_warnings_flag_only_commander_decks() {
        // No COMMANDER section: no warnings at all.
        let deck = Deck::parse("// DECK\n2 Bolt\n").unwrap();
        let ops = vec![parse_op("add", "2 Bolt").unwrap()];
        assert!(singleton_warnings(&ops, &deck).is_empty());
        // With a COMMANDER section, the raise warns.
        let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n").unwrap();
        let ops = vec![parse_op("add", "2 Bolt").unwrap()];
        assert_eq!(
            singleton_warnings(&ops, &deck),
            vec![
                "Bolt would exceed the singleton limit; commander decks hold one copy".to_string()
            ]
        );
        // Basincs are exempt.
        let ops = vec![parse_op("add", "20 Island").unwrap()];
        assert!(singleton_warnings(&ops, &deck).is_empty());
        // A set that lowers to 1 does not warn.
        let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n4 Bolt\n").unwrap();
        let ops = vec![parse_op("set", "1 Bolt").unwrap()];
        assert!(singleton_warnings(&ops, &deck).is_empty());
        // A set that raises does warn.
        let ops = vec![parse_op("set", "3 Bolt").unwrap()];
        assert_eq!(singleton_warnings(&ops, &deck).len(), 1);
    }

    #[test]
    fn singleton_warnings_use_pre_apply_deck_state() {
        // A first-copy add of an absent card must not warn: the warning
        // reads the deck before ops apply, not after.
        let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n").unwrap();
        let ops = vec![parse_op("add", "1 Secluded Courtyard").unwrap()];
        assert!(singleton_warnings(&ops, &deck).is_empty());
        // A multi-copy add still warns.
        let ops = vec![parse_op("add", "2 Secluded Courtyard").unwrap()];
        assert_eq!(singleton_warnings(&ops, &deck).len(), 1);
        // Adding a second copy of a held card still warns.
        let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Secluded Courtyard\n").unwrap();
        let ops = vec![parse_op("add", "1 Secluded Courtyard").unwrap()];
        assert_eq!(singleton_warnings(&ops, &deck).len(), 1);
    }

    #[test]
    fn parse_spec_line_reads_verbs_and_bare_specs() {
        match parse_spec_line("add 1 Bolt").unwrap() {
            DeckOp::Add { entry, .. } => assert_eq!(entry.name, "Bolt"),
            other => panic!("unexpected {other:?}"),
        }
        match parse_spec_line("remove 1 Bolt").unwrap() {
            DeckOp::Remove { entry, .. } => assert_eq!(entry.name, "Bolt"),
            other => panic!("unexpected {other:?}"),
        }
        match parse_spec_line("set 2 Bolt").unwrap() {
            DeckOp::Set { entry, .. } => assert_eq!(entry.quantity, 2),
            other => panic!("unexpected {other:?}"),
        }
        // Bare spec defaults to add.
        match parse_spec_line("3 Bolt").unwrap() {
            DeckOp::Add { entry, .. } => assert_eq!(entry.quantity, 3),
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse_spec_line("shuffle 1 Bolt").is_err());
    }
}
