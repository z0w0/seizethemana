// Deck update ops (`--add`/`--remove`/`--set`/`--move`) and the
// `deck update` command. The op verbs and the apply engine live in
// `deck/ops.rs`; this file is the command flow: name validation, the
// real update, the `--dry-run` preview, and `deck dedupe`.

use super::grammar::Deck;
pub(super) use super::ops::{DeckOp, USAGE_EXIT, apply_ops, parse_op};
pub(super) use super::store::{load_deck, save_deck, valid_deck_name};
use anyhow::Context;

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
        NameMatch::Ambiguous { candidates, .. } => Ok(NameCheck::Unknown(candidates)),
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
/// `dry_run` previews the change and writes nothing; `sim` (dry-run only)
/// adds a same-seed before/after consistency delta.
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
    dry_run: bool,
    sim: bool,
    legal: bool,
    backfill_basics: bool,
    json: bool,
) -> anyhow::Result<i32> {
    let ops = parse_update_ops(add, remove, set, move_specs, from, out)?;
    let ops = match ops {
        Some(ops) => ops,
        None => return Ok(USAGE_EXIT),
    };
    if ops.is_empty() && !backfill_basics {
        out.error("no update operations given");
        out.hint("pass --add/--remove/--set/--move specs, e.g. --add '2 Bolt'");
        return Ok(USAGE_EXIT);
    }
    if !valid_deck_name(name) {
        out.error(&format!("invalid deck name {name:?}"));
        out.hint("use letters, digits, spaces, or - _ ' & ! + , (no / or ..)");
        anyhow::bail!(crate::output::SILENT_ERROR);
    }
    let (_path, mut deck) = load_deck(paths, name)?;
    if let Some(code) = validate_names(conn, out, &ops)? {
        return Ok(code);
    }
    // Singleton guard for adds: in a commander-shaped deck an add that
    // pushes a non-basic card past one copy is rejected up front instead
    // of writing an illegal deck. The check reads the POST-apply state
    // (the ops run on a scratch copy in flag order), so a legal
    // remove+add pair (net one copy) passes. Set and Move ops keep the
    // post-apply warning path (an exact-quantity request is deliberate;
    // a move nets to zero new copies).
    if let Some(code) = reject_singleton_adds(conn, out, &deck, &ops)? {
        return Ok(code);
    }
    if dry_run {
        return dry_run_preview(
            DryRun {
                conn,
                out,
                name,
                before: &deck,
                sim,
                legal,
                json,
                backfill_basics,
            },
            &ops,
        );
    }
    let mut summary = apply_ops(&mut deck, &ops)?;
    let mut backfilled = 0i64;
    if backfill_basics {
        let added = backfill_basics_to_size(conn, out, &mut deck)?;
        if added > 0 {
            summary.added += added;
            backfilled = added;
        }
    }
    // Missing removes: partial mode reports and continues (exit 1 still
    // flags the incomplete batch); strict mode stops with exit 3.
    let mut had_missing = false;
    if !summary.missing.is_empty() {
        out.error(&format!(
            "{} of the referenced cards are not in the deck: {}",
            summary.missing.len(),
            summary.missing.join(", ")
        ));
        if !allow_partial {
            out.hint("show the deck first: stm deck show");
            out.hint("apply the resolvable ops anyway with --allow-partial");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
        // The applied ops already mutated the deck in place; report and
        // continue past the misses.
        out.warning("continuing without the missing cards (--allow-partial)");
        summary.missing.clear();
        had_missing = true;
    }
    report_applied_update(
        paths,
        conn,
        out,
        name,
        &deck,
        &ops,
        &summary,
        backfilled,
        had_missing,
        json,
    )
}

/// The post-apply report: singleton warnings, save, and the human or JSON
/// summary. Returns the exit code.
#[allow(clippy::too_many_arguments)]
fn report_applied_update(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    deck: &Deck,
    ops: &[DeckOp],
    summary: &super::ops::DeckOpSummary,
    backfilled: i64,
    had_missing: bool,
    json: bool,
) -> anyhow::Result<i32> {
    // Singleton guard: commander-shape decks should hold one copy per
    // non-basic card. Computed against the post-apply deck, so a legal
    // remove+add pair (net one copy) does not warn; the write still
    // happens so agents keep flowing, and `deck legal` reports the real
    // violation.
    let warnings = singleton_warnings(conn, ops, deck)?;
    for w in &warnings {
        out.warning(w);
    }
    save_deck(paths, name, deck)?;
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
    if json {
        let payload = serde_json::json!({
            "name": name,
            "added": summary.added,
            "removed": summary.removed,
            "moved": summary.moved,
            "set": summary.set,
            "relocated": summary
                .relocated
                .iter()
                .map(|(card, section)| serde_json::json!({"name": card, "from": section}))
                .collect::<Vec<_>>(),
            "backfilled_basics": backfilled,
            "warnings": warnings,
            "cards": deck.total(),
            "maindeck_cards": deck.maindeck_total(),
            "sideboard_cards": deck.sideboard_total(),
            "maybeboard_cards": deck.maybeboard_total(),
            "commander_cards": deck.commander_total(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        // Section-aware size: bench and commander zones change the deck's
        // shape, so the summary names each zone's count.
        let mut size = format!("{} maindeck", deck.maindeck_total());
        let sideboard = deck.sideboard_total();
        if sideboard > 0 {
            size.push_str(&format!(" + {sideboard} sideboard"));
        }
        let maybeboard = deck.maybeboard_total();
        if maybeboard > 0 {
            size.push_str(&format!(" + {maybeboard} maybeboard"));
        }
        let commander = deck.commander_total();
        if commander > 0 {
            size.push_str(&format!(" + {commander} commander"));
        }
        out.finish(
            "Updated",
            &format!("deck {name:?}: {} (now {size})", parts.join(", ")),
            std::time::Duration::ZERO,
        );
    }
    Ok(exit)
}

/// Parse every op spec (flags first, then the `--from` file).
///
/// Returns `None` after reporting an empty spec file (the caller exits
/// with the usage code).
fn parse_update_ops(
    add: &[String],
    remove: &[String],
    set: &[String],
    move_specs: &[String],
    from: Option<&std::path::Path>,
    out: &mut crate::output::Output,
) -> anyhow::Result<Option<Vec<DeckOp>>> {
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
            return Ok(None);
        }
    }
    Ok(Some(ops))
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

/// Inputs to the `--dry-run` preview, grouped so the helper stays under
/// the argument-count lint.
struct DryRun<'a> {
    conn: &'a rusqlite::Connection,
    out: &'a mut crate::output::Output,
    name: &'a str,
    before: &'a Deck,
    sim: bool,
    legal: bool,
    json: bool,
    backfill_basics: bool,
}

/// The `--dry-run` preview: apply the ops to a clone, print the change
/// list, the cost impact, (with `--sim`) the same-seed consistency delta,
/// and (with `--legal`) the post-change legality verdict. Nothing is
/// written, ever. Exit 1 when `--sim` reports a new problem or `--legal`
/// finds violations.
fn dry_run_preview(preview: DryRun<'_>, ops: &[DeckOp]) -> anyhow::Result<i32> {
    let DryRun {
        conn,
        out,
        name,
        before,
        sim,
        legal,
        json,
        backfill_basics,
    } = preview;
    let mut after = before.clone();
    let mut summary = apply_ops(&mut after, ops)?;
    if backfill_basics {
        let added = backfill_basics_to_size(conn, out, &mut after)?;
        if added > 0 {
            summary.added += added;
        }
    }
    for (card, section) in &summary.relocated {
        out.warning(&format!("{card} was in {section}; removed it there"));
    }
    // Missing removes resolve to the same exit codes as a real update:
    // a preview must never read as a clean no-op when an op could not
    // resolve.
    if !summary.missing.is_empty() {
        return dry_run_missing(out, name, &summary.missing, json);
    }
    if json {
        return dry_run_json(conn, name, before, &after, sim, legal);
    }
    dry_run_human(conn, out, name, before, &after, sim, legal)
}

/// Report unresolvable ops for a `--dry-run` preview.
fn dry_run_missing(
    out: &mut crate::output::Output,
    name: &str,
    missing: &[String],
    json: bool,
) -> anyhow::Result<i32> {
    out.error(&format!(
        "{} of the referenced cards are not in the deck: {}",
        missing.len(),
        missing.join(", ")
    ));
    out.hint("show the deck first: stm deck show");
    out.hint("apply the resolvable ops anyway with --allow-partial");
    if json {
        let payload = serde_json::json!({
            "name": name,
            "dry_run": true,
            "changes": [],
            "missing": missing,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    }
    Ok(crate::cli::codes::NO_RESULTS)
}

/// JSON output for a `--dry-run` preview: changes + cost + optional sim
/// diff + optional legality verdict in one object. Exit 1 when `--sim`
/// reports a new problem or `--legal` finds violations.
fn dry_run_json(
    conn: &rusqlite::Connection,
    name: &str,
    before: &Deck,
    after: &Deck,
    sim: bool,
    legal: bool,
) -> anyhow::Result<i32> {
    let diff = super::diff::diff_decks(before, after, false, crate::collection::is_basic_name);
    let changes = serde_json::to_value(&diff)?;
    let cost = cost_delta(conn, before, after)?;
    let sim_payload = if sim {
        let diff = sim_delta(conn, name, before, after)?;
        Some(serde_json::to_value(&diff)?)
    } else {
        None
    };
    let new_problems = sim_payload
        .as_ref()
        .and_then(|v| v.get("problems").and_then(|p| p.as_array()))
        .is_some_and(|problems| {
            problems
                .iter()
                .any(|p| p.get("change").and_then(|c| c.as_str()) == Some("new"))
        });
    let (legal_payload, legal_ok) = if legal {
        let payload = legal_verdict(conn, after)?;
        let ok = payload
            .get("legal")
            .and_then(|l| l.as_bool())
            .unwrap_or(false);
        (Some(payload), ok)
    } else {
        (None, true)
    };
    let payload = serde_json::json!({
        "name": name,
        "dry_run": true,
        "changes": changes,
        "cost": cost,
        "sim": sim_payload,
        "legal": legal_payload,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(if (sim && new_problems) || (legal && !legal_ok) {
        crate::cli::codes::ERROR
    } else {
        crate::cli::codes::OK
    })
}

/// Legality verdict for a post-change deck (the `--legal` dry-run block):
/// the same checks `deck legal` runs, keyed by the inferred format.
fn legal_verdict(conn: &rusqlite::Connection, deck: &Deck) -> anyhow::Result<serde_json::Value> {
    let cards = super::stats::lookup_names(conn, deck)?;
    let is_commander = super::legal::infer_format(deck) == super::legal::InferredFormat::Commander;
    let bracket = if is_commander {
        Some(super::simulator::infer_bracket(deck, &cards))
    } else {
        None
    };
    // The inferred-constructed case is structural-only, matching `deck legal`.
    let check_format = if is_commander {
        Some("commander")
    } else {
        None
    };
    let (violations, advisories) = super::legal::check(deck, &cards, check_format, bracket);
    let legal = violations.is_empty();
    let violations: Vec<serde_json::Value> = violations
        .iter()
        .map(|v| serde_json::json!({ "rule": v.rule, "cards": v.cards, "detail": v.detail }))
        .collect();
    Ok(serde_json::json!({
        "legal": legal,
        "violations": violations,
        "advisories": advisories,
    }))
}

/// Human output for a `--dry-run` preview: change list, cost impact, and
/// (with `sim`) the same-seed consistency delta.
fn dry_run_human(
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    before: &Deck,
    after: &Deck,
    sim: bool,
    legal: bool,
) -> anyhow::Result<i32> {
    let diff = super::diff::diff_decks(before, after, false, crate::collection::is_basic_name);
    let styles = out.styles();

    println!("{}", styles.header("This change would make"));
    let mut any_change = false;
    for section in &diff {
        if section.is_empty() {
            continue;
        }
        any_change = true;
        println!("  {}:", styles.header(&section.section));
        for (card, qty) in &section.removed {
            println!(
                "    {} {}",
                styles.glyph("-", crate::output::GlyphKind::Bad),
                card_qty_text(card, *qty)
            );
        }
        for (card, qty) in &section.added {
            println!(
                "    {} {}",
                styles.glyph("+", crate::output::GlyphKind::Good),
                card_qty_text(card, *qty)
            );
        }
        for (card, from, to) in &section.changed {
            let text = format!("{card}: {from} → {to}");
            println!(
                "    {} {}",
                styles.glyph("~", crate::output::GlyphKind::Dim),
                text
            );
        }
    }
    if !any_change {
        println!("    {}", styles.dim("nothing — the deck already matches"));
        out.print_note("Nothing was changed. The deck already matches this request.");
        return Ok(crate::cli::codes::OK);
    }

    print_cost_block(conn, out, before, after)?;

    // Sim block: same-seed before/after delta.
    let mut exit = crate::cli::codes::OK;
    if sim {
        let diff = sim_delta(conn, name, before, after)?;
        println!("{}", styles.header("Consistency impact (same seed)"));
        if diff.is_empty() {
            println!("    {}", styles.dim("no changes in the simulation"));
        } else {
            for d in &diff.shape {
                println!("    {}: {} → {}", d.name, d.old, d.new);
            }
            for d in &diff.metrics {
                println!("    {}: {} → {}", d.path, d.old, d.new);
            }
            for p in &diff.problems {
                match p.change {
                    "new" => {
                        println!(
                            "    {} {}: {}",
                            styles.glyph("+", crate::output::GlyphKind::Bad),
                            p.kind,
                            p.detail
                        );
                        exit = crate::cli::codes::ERROR;
                    }
                    _ => println!(
                        "    {} {}: {}",
                        styles.glyph("-", crate::output::GlyphKind::Good),
                        p.kind,
                        p.detail
                    ),
                }
            }
        }
    }
    // Legality block: the post-change deck against the inferred format.
    if legal {
        let verdict = legal_verdict(conn, after)?;
        let is_legal = verdict
            .get("legal")
            .and_then(|l| l.as_bool())
            .unwrap_or(false);
        println!("{}", styles.header("Legality"));
        if is_legal {
            println!("    {}", styles.dim("the changed deck stays legal"));
        } else {
            let violations = verdict
                .get("violations")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for v in &violations {
                let rule = v.get("rule").and_then(|r| r.as_str()).unwrap_or("");
                let detail = v.get("detail").and_then(|d| d.as_str()).unwrap_or("");
                println!(
                    "    {} {}: {}",
                    styles.glyph("!", crate::output::GlyphKind::Bad),
                    rule,
                    detail
                );
            }
            exit = crate::cli::codes::ERROR;
        }
    }
    out.print_note("Nothing was changed. Run the same command without --dry-run to apply.");
    Ok(exit)
}

/// The "Cost impact" block: what you would buy and what removals free up.
fn print_cost_block(
    conn: &rusqlite::Connection,
    out: &crate::output::Output,
    before: &Deck,
    after: &Deck,
) -> anyhow::Result<()> {
    let cost = cost_delta(conn, before, after)?;
    let styles = out.styles();
    println!("{}", styles.header("Cost impact"));
    if cost.to_buy.items.is_empty() {
        println!("    {}", styles.dim("Nothing new to buy."));
    } else {
        for item in &cost.to_buy.items {
            let owned_note = if item.owned > 0 {
                format!(" ({} owned)", item.owned)
            } else {
                String::new()
            };
            println!(
                "    {} {}",
                styles.card_name(&item.name),
                styles.dim(&format!(
                    "x{} @ {}{}",
                    item.quantity,
                    item.price_usd
                        .map(|p| format!("${p:.2}"))
                        .unwrap_or_else(|| "unpriced".into()),
                    owned_note
                ))
            );
        }
    }
    println!(
        "    net spend: {} (buy {}, freed {})",
        styles.money(cost.net_usd),
        styles.money(cost.to_buy.total_usd),
        styles.money(cost.freed_usd)
    );
    Ok(())
}

/// `qty Name` display for a diff row.
fn card_qty_text(card: &str, qty: i64) -> String {
    format!("{qty}x {card}")
}

/// The cost delta between two deck states: what you would buy (missing
/// copies at the cheapest printing) and what removals free up.
#[derive(serde::Serialize)]
struct CostImpact {
    to_buy: Buylist,
    /// Cheapest-printing value of the copies the change frees (owned
    /// copies that leave the deck's slots).
    freed_usd: f64,
    /// The running net spend.
    net_usd: f64,
}

/// One to-buy line.
#[derive(serde::Serialize)]
struct BuyItem {
    name: String,
    quantity: i64,
    price_usd: Option<f64>,
    /// Copies already owned anywhere in the collection (including this
    /// deck's own assigned copies).
    owned: i64,
}

/// Buy gap of the after-deck minus the before-deck, plus the freed value.
fn cost_delta(
    conn: &rusqlite::Connection,
    before: &Deck,
    after: &Deck,
) -> anyhow::Result<CostImpact> {
    let owned = crate::collection::owned_counts_all(conn)?;
    let is_basic = crate::collection::is_basic_name;
    let needed = |deck: &Deck| -> std::collections::BTreeMap<String, i64> {
        let mut needed = std::collections::BTreeMap::new();
        for entry in deck.entries() {
            if is_basic(&entry.name) {
                continue;
            }
            *needed.entry(entry.name.clone()).or_insert(0) += entry.quantity;
        }
        needed
    };
    let before_needed = needed(before);
    let after_needed = needed(after);
    // To buy: slots the after-deck demands that the before-deck did not
    // (or demanded less of) and the collection cannot cover.
    let mut items = Vec::new();
    let mut buy_total = 0.0f64;
    let mut buy_names: Vec<String> = Vec::new();
    for (card, qty) in &after_needed {
        let extra = qty - before_needed.get(card).copied().unwrap_or(0);
        if extra <= 0 {
            continue;
        }
        let short = extra.min((qty - owned.get(card).copied().unwrap_or(0)).max(0));
        if short <= 0 {
            continue;
        }
        items.push(BuyItem {
            name: card.clone(),
            quantity: short,
            price_usd: None,
            owned: owned.get(card).copied().unwrap_or(0),
        });
        buy_names.push(card.clone());
    }
    let ranges = crate::prints::price_ranges(conn, &buy_names)?;
    for item in &mut items {
        let price = ranges
            .get(&item.name)
            .and_then(crate::prints::PrintRange::price);
        if let Some(p) = price {
            buy_total += p * item.quantity as f64;
        }
        item.price_usd = price;
    }
    // Freed: slots the before-deck demanded that the after-deck does not,
    // valued at the cheapest printing. Only owned copies count: a slot
    // freed on a card you never owned frees nothing.
    let mut freed_names: Vec<String> = Vec::new();
    let mut freed: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for (card, qty) in &before_needed {
        let released = qty - after_needed.get(card).copied().unwrap_or(0);
        if released > 0 {
            let owned_released = released.min(owned.get(card).copied().unwrap_or(0));
            if owned_released > 0 {
                freed.insert(card.clone(), owned_released);
                freed_names.push(card.clone());
            }
        }
    }
    let freed_ranges = crate::prints::price_ranges(conn, &freed_names)?;
    let mut freed_total = 0.0f64;
    for (card, qty) in &freed {
        if let Some(p) = freed_ranges
            .get(card)
            .and_then(crate::prints::PrintRange::price)
        {
            freed_total += p * *qty as f64;
        }
    }
    Ok(CostImpact {
        to_buy: Buylist {
            items,
            total_usd: buy_total,
        },
        freed_usd: freed_total,
        net_usd: buy_total - freed_total,
    })
}

/// The grouped to-buy list inside the cost impact.
#[derive(serde::Serialize)]
struct Buylist {
    items: Vec<BuyItem>,
    total_usd: f64,
}

/// Simulate before and after at the same seed; returns the report diff
/// (callers key the exit code on its `new` problems).
fn sim_delta(
    conn: &rusqlite::Connection,
    name: &str,
    before: &Deck,
    after: &Deck,
) -> anyhow::Result<crate::deck::simulator::report_view::ReportDiff> {
    use crate::deck::simulator::report_view::diff_reports;
    let cards = super::stats::lookup_names(conn, before)?;
    let after_cards = super::stats::lookup_names(conn, after)?;
    let mut all_cards = cards.clone();
    for (k, v) in after_cards {
        all_cards.entry(k).or_insert(v);
    }
    let seed = 42u64;
    let runs = 2_000u32;
    let before_json =
        super::simulator::sim_report_for(before, &all_cards, name, runs, None, seed, None);
    let after_json =
        super::simulator::sim_report_for(after, &all_cards, name, runs, None, seed, None);
    Ok(diff_reports(&before_json, &after_json))
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
/// The guard reads the post-apply state: the ops run on a scratch copy of
/// the deck first, so a legal remove+add pair (net one copy) passes while
/// an incremental add onto an existing line fails. Returns the exit code
/// to use when at least one add must be rejected, or `None` when every
/// add is singleton-safe (or the deck is not commander-shaped).
fn reject_singleton_adds(
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    deck: &Deck,
    ops: &[DeckOp],
) -> anyhow::Result<Option<i32>> {
    if deck.section_index("COMMANDER").is_none() {
        return Ok(None);
    }
    let mut post = deck.clone();
    let summary = match apply_ops(&mut post, ops) {
        Ok(summary) => summary,
        Err(err) => {
            // The batch does not apply at all (a bad spec or a failed
            // apply): the update flow reports the parse/apply error
            // separately when it runs the ops for real, so the guard
            // only notes the skip instead of swallowing the failure.
            out.print_note(&format!(
                "singleton check skipped: the ops do not apply cleanly ({err:#})"
            ));
            return Ok(None);
        }
    };
    if !summary.missing.is_empty() {
        // Unresolvable ops make the post state unreliable; the normal
        // missing-op path reports them.
        return Ok(None);
    }
    let mut rejected: Vec<String> = Vec::new();
    for (name, qty) in super::legal::maindeck_copies_by_name(&post) {
        if qty <= 1 || rejected.contains(&name) || is_singleton_exempt(conn, &name)? {
            continue;
        }
        // Only names touched by an Add op are rejected: an oversized Set
        // or Move is the operator's exact request.
        let touched_by_add = ops.iter().any(|op| match op {
            DeckOp::Add { entry, .. } => entry.name == name,
            _ => false,
        });
        if touched_by_add {
            rejected.push(name);
        }
    }
    if rejected.is_empty() {
        return Ok(None);
    }
    out.error(&format!(
        "{} of the adds would exceed the singleton limit: {}",
        rejected.len(),
        rejected.join(", ")
    ));
    for name in &rejected {
        out.hint(&format!(
            "{name} is already in the deck; use --set to change its quantity, \
             or --remove it first"
        ));
    }
    Ok(Some(crate::cli::codes::NO_RESULTS))
}

/// Warn when an op would push a non-basic card past one copy in a
/// commander-shaped deck (a COMMANDER section present).
///
/// Called against the post-apply deck: a legal remove+add pair nets to one
/// copy and must not warn. Returns one warning per affected card name.
/// The deck is still written: the warning is a nudge, and `deck legal`
/// reports the real violation.
fn singleton_warnings(
    conn: &rusqlite::Connection,
    ops: &[DeckOp],
    deck: &Deck,
) -> anyhow::Result<Vec<String>> {
    if deck.section_index("COMMANDER").is_none() {
        return Ok(Vec::new());
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
        // Sum across maindeck sections: a commander deck holds one copy
        // outside the bench sections (sideboard/maybeboard are not extra
        // copies), so a move that splits 2 copies as 1+1 across maindeck
        // sections still breaches the singleton rule.
        let held = deck
            .sections
            .iter()
            .filter(|(s, _)| !super::grammar::is_bench_section(s))
            .flat_map(|(_, es)| es.iter())
            .filter(|e| e.name == entry.name)
            .map(|e| e.quantity)
            .sum();
        let end_state = match kind {
            OpKind::Set => entry.quantity,
            // Add and Move both read the post-apply deck: what it now
            // holds is the only state worth checking.
            OpKind::Add | OpKind::Move => held,
        };
        if end_state <= 1 || is_singleton_exempt(conn, &entry.name)? {
            continue;
        }
        warned.push(entry.name.clone());
    }
    Ok(warned
        .iter()
        .map(|name| {
            format!("{name} would exceed the singleton limit; commander decks hold one copy")
        })
        .collect())
}

/// True for the names that never break singleton (the five basic lands;
/// Wastes and snow basics are limited-supply cards, so they stay tracked;
/// "any number of cards named X" cards are rare enough that `deck legal`
/// is the authority).
fn is_unlimited_basics(name: &str) -> bool {
    matches!(name, "Plains" | "Island" | "Swamp" | "Mountain" | "Forest")
}

/// True when a card may hold any number of copies in a commander deck:
/// basic lands by name plus oracle-text cards ("a deck can have any number
/// of cards named ..."). Mirrors the exemption `deck legal` applies.
pub(super) fn is_singleton_exempt(conn: &rusqlite::Connection, name: &str) -> anyhow::Result<bool> {
    if is_unlimited_basics(name) {
        return Ok(true);
    }
    Ok(crate::db::get_card(conn, name)?.is_some_and(|c| super::stats::is_unlimited_copies(&c)))
}

/// Add basic lands after the ops until the deck reaches its format's
/// exact size (100 with a COMMANDER section, 60 for brawl-shape decks, 59
/// for oathbreaker, else 60). The basic name comes from the commander's
/// (or deck's) color identity: the first identity color's basic; colorless
/// decks get Wastes. Two-color commanders therefore backfill only one
/// basic type — fix the mana base by hand after `--backfill-basics`.
/// Returns the added count. A missing commander row or unparseable
/// identity warns and falls back to Wastes rather than failing the
/// update.
fn backfill_basics_to_size(
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    deck: &mut Deck,
) -> anyhow::Result<i64> {
    let target = super::legal::singleton_size_for_deck(deck);
    // Match the legality check: only maindeck cards count toward the
    // deck size (the sideboard is a wishlist, not legal maindeck).
    let deficit = target - deck.maindeck_total();
    if deficit <= 0 {
        return Ok(0);
    }
    // Identity colors: the COMMANDER section first, else every entry.
    let names: Vec<String> = match deck.section_index("COMMANDER") {
        Some(i) => deck.sections[i].1.iter().map(|e| e.name.clone()).collect(),
        None => deck.entries().map(|e| e.name.clone()).collect(),
    };
    let mut colors: Vec<char> = Vec::new();
    for name in &names {
        let Ok(card) = crate::db::get_card(conn, name) else {
            continue;
        };
        let Some(card) = card else { continue };
        if let Ok(identity) = serde_json::from_str::<Vec<String>>(&card.color_identity) {
            for color in identity {
                let c = color.chars().next().unwrap_or('C');
                if !"WUBRG".contains(c) || colors.contains(&c) {
                    continue;
                }
                colors.push(c);
            }
        }
    }
    let basic_for = |c: char| match c {
        'W' => "Plains",
        'U' => "Island",
        'B' => "Swamp",
        'R' => "Mountain",
        _ => "Forest",
    };
    let basic = match colors.first() {
        Some(&c) => basic_for(c),
        None => {
            // The commander is absent from the oracle or its identity is
            // unparseable; the fallback is visible, not silent.
            out.warning("color identity unknown; backfilling with Wastes");
            "Wastes"
        }
    };
    let list = deck.section_entries_mut("DECK");
    if let Some(existing) = list.iter_mut().find(|e| e.name == basic) {
        existing.quantity += deficit;
    } else {
        list.push(crate::deck::grammar::DeckEntry {
            quantity: deficit,
            name: basic.to_string(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    Ok(deficit)
}

#[cfg(test)]
#[path = "tests/update_tests.rs"]
mod update_tests;
