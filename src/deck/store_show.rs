//! `deck show` and `deck <name>`: the human overview, the JSON view, and
//! the per-card table. Split from `store.rs` to stay under the module
//! size limit; `super::*` re-exports the store namespace's helpers.

use super::store::{load_deck, primer_file};
use anyhow::Context;
use rusqlite::Connection;

/// Per-name print prices for one finish: `(foil_usd, nonfoil_usd)`, either
/// possibly missing.
type FinishPrices = (Option<f64>, Option<f64>);

/// Foil and nonfoil cheapest prices for every deck card name, keyed by
/// name, so each deck entry prices at its own finish.
///
/// A price-query failure degrades to an empty map with a warning; it never
/// aborts `deck show`.
pub(crate) fn deck_finish_prices(
    conn: &Connection,
    deck: &super::Deck,
    out: &mut crate::output::Output,
) -> std::collections::HashMap<String, FinishPrices> {
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in deck.entries().map(|e| e.name.clone()) {
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }
    // One batched query per finish kind instead of four statements per
    // card name.
    let ranges = match crate::prints::price_ranges(conn, &names) {
        Ok(ranges) => ranges,
        Err(err) => {
            out.warning(&format!("price lookup failed; prices omitted: {err:#}"));
            return Default::default();
        }
    };
    ranges
        .into_iter()
        .map(|(name, range)| {
            let foil = range
                .cheapest_foil
                .as_ref()
                .and_then(|p| p.usd_foil)
                // No foil print priced: fall back to the nonfoil price so
                // foil copies still contribute a real value.
                .or_else(|| range.cheapest.as_ref().and_then(|p| p.usd));
            let nonfoil = range
                .cheapest
                .as_ref()
                .and_then(|p| p.usd)
                // Symmetric fallback: no nonfoil price, use the foil one.
                .or_else(|| range.cheapest_foil.as_ref().and_then(|p| p.usd_foil));
            (name, (foil, nonfoil))
        })
        .collect()
}

/// Price for one deck entry at its printed finish.
///
/// Foil entries price at the foil rate, nonfoil entries at the nonfoil
/// rate; a missing finish price falls back to the available one (the
/// fallback is applied in `deck_finish_prices`, so this only reads what
/// is there).
pub(crate) fn entry_unit_price(
    entry: &crate::deck::grammar::DeckEntry,
    prices: &std::collections::HashMap<String, FinishPrices>,
) -> Option<f64> {
    let (foil, nonfoil) = prices.get(&entry.name).copied().unwrap_or((None, None));
    if entry.foil { foil } else { nonfoil }
}

/// Overview stats block under the `deck show` header: completion, curve,
/// ramp, colors, types. Pure display; JSON output is unaffected.
///
/// DB failures degrade with a printed note: an unresolvable card map
/// renders the stat blocks without per-card data, a failed ownership map
/// drops the Own/Value lines — both visible in the output, never a silent
/// abort.
fn print_overview(
    styles: &crate::output::Styles,
    conn: &Connection,
    deck: &super::Deck,
    deck_name: &str,
    out: &mut crate::output::Output,
) {
    let cards_by_name = match super::stats::lookup_names(conn, deck) {
        Ok(map) => map,
        Err(err) => {
            out.warning(&format!(
                "card lookup failed; overview stats are incomplete: {err:#}"
            ));
            // Degraded render: the deck still has structure worth showing
            // (sections, counts), so emit what the row data supports —
            // nothing here needs the oracle map.
            let stats = super::stats::compute(deck, &Default::default());
            if stats.total > 0 {
                println!();
                print_stat_blocks(styles, &stats, super::legal::is_commander(deck, None));
                println!();
            }
            return;
        }
    };
    let available = match super::ownership::available_map(conn, deck_name) {
        Ok(map) => map,
        Err(err) => {
            out.warning(&format!(
                "ownership lookup failed; the Own line is omitted: {err:#}"
            ));
            return print_overview_without_ownership(styles, conn, deck, &cards_by_name);
        }
    };
    print_overview_body(styles, conn, deck, &cards_by_name, &available, out);
}

/// Overview without the Own/Value lines (ownership lookup failed): the
/// remaining stats still render so the reader loses one line, not the
/// whole block.
fn print_overview_without_ownership(
    styles: &crate::output::Styles,
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
) {
    let stats = super::stats::compute(deck, cards_by_name);
    if stats.total == 0 {
        return;
    }
    println!();
    print_stat_blocks(styles, &stats, super::legal::is_commander(deck, None));
    if let Ok(audit) = super::mana::mana_audit_for(conn, deck) {
        super::mana::print_mana_audit(styles, &audit);
    }
    println!();
}

/// Overview with ownership known: the Own line, the value line, and the
/// stat blocks.
fn print_overview_body(
    styles: &crate::output::Styles,
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    available: &std::collections::HashMap<String, i64>,
    out: &mut crate::output::Output,
) {
    let stats = super::stats::compute(deck, cards_by_name);
    if stats.total == 0 {
        return;
    }
    println!();
    let owned: i64 = deck
        .entries()
        .map(|entry| {
            // Basic lands are assumed available in unlimited supply; they
            // always count as owned for completion.
            if cards_by_name
                .get(&entry.name)
                .is_some_and(super::stats::is_tracked_basic)
            {
                return entry.quantity;
            }
            // Available copies (deck-assigned + binders) fill the slot;
            // copies parked in other decks do not.
            let available = available.get(&entry.name).copied().unwrap_or(0);
            available.min(entry.quantity)
        })
        .sum();
    let label = |text: &str| -> String {
        if text.is_empty() {
            styles.dim(&" ".repeat(10))
        } else {
            styles.dim(&format!("{text:>9} "))
        }
    };
    println!(
        "{}{}",
        label("Own"),
        styles.success(&format!(
            "{}/{} ({:.0}%)",
            owned,
            stats.total,
            owned as f64 / stats.total as f64 * 100.0,
        )),
    );
    // Money view: what owned copies are worth and what buying the rest costs.
    let prices = deck_finish_prices(conn, deck, out);
    let (owned_value, missing_cost) = deck_value(deck, cards_by_name, &prices, available);
    if owned_value > 0.0 || missing_cost > 0.0 {
        println!(
            "{}{}{}{}",
            label("Value"),
            styles.money(owned_value),
            styles.dim(" owned · missing "),
            styles.money(missing_cost),
        );
    }
    print_stat_blocks(styles, &stats, super::legal::is_commander(deck, None));
    if let Ok(audit) = super::mana::mana_audit_for(conn, deck) {
        super::mana::print_mana_audit(styles, &audit);
    }
    println!();
}

/// The histogram blocks (curve, ramp, colors, types) shared by both
/// overview variants.
fn print_stat_blocks(
    styles: &crate::output::Styles,
    stats: &super::stats::DeckStats,
    is_commander: bool,
) {
    let label = |text: &str| -> String {
        if text.is_empty() {
            styles.dim(&" ".repeat(10))
        } else {
            styles.dim(&format!("{text:>9} "))
        }
    };
    if !stats.curve.is_empty() {
        // One-line curve score: avg MV, the histogram, and the target
        // sentence (format/archetype-aware, same mapping as the JSON
        // `curve` block).
        let histogram = super::stats::curve_histogram(stats);
        println!(
            "{}{} {} · {} · {}",
            label("Curve score"),
            styles.dim(&format!("avg MV {:.1}", stats.avg_cmc)),
            styles.dim(
                &histogram
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join("/")
            ),
            super::stats::curve_target(is_commander, stats.avg_cmc),
            styles.dim(&format!(
                "{} nonland copies",
                stats.curve.iter().map(|b| b.count).sum::<i64>()
            )),
        );
        for (i, bucket) in stats.curve.iter().enumerate() {
            println!(
                "{}{} {} {}",
                if i == 0 { label("Curve") } else { label("") },
                styles.bar(bucket.ratio, 12),
                styles.dim(&format!("{:>2}", bucket.label)),
                styles.thousands(bucket.count),
            );
        }
    }
    let (lands, rocks, dorks, other) = stats.ramp;
    let ramp_total = lands + rocks + dorks + other;
    if ramp_total > 0 {
        let rows = [
            ("lands", lands),
            ("rocks", rocks),
            ("dorks", dorks),
            ("other", other),
        ];
        let max = rows.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f64;
        for (i, (kind, n)) in rows.iter().enumerate() {
            if *n == 0 {
                continue;
            }
            println!(
                "{}{} {} {}",
                if i == 0 { label("Ramp") } else { label("") },
                styles.bar(*n as f64 / max, 12),
                styles.dim(kind),
                n,
            );
        }
    }
    if !stats.colors.is_empty() {
        let order = ["W", "U", "B", "R", "G", "C"];
        let mut colors = stats.colors.clone();
        colors.sort_by_key(|b| order.iter().position(|o| *o == b.label).unwrap_or(6));
        let max = colors.iter().map(|b| b.count).max().unwrap_or(1).max(1) as f64;
        for (i, bucket) in colors.iter().enumerate() {
            println!(
                "{}{} {} {}",
                if i == 0 { label("Colors") } else { label("") },
                styles.bar(bucket.count as f64 / max, 12),
                styles.color_letters(&bucket.label),
                styles.thousands(bucket.count),
            );
        }
    }
    if !stats.types.is_empty() {
        let max = stats
            .types
            .iter()
            .map(|b| b.count)
            .max()
            .unwrap_or(1)
            .max(1) as f64;
        for (i, bucket) in stats.types.iter().enumerate() {
            println!(
                "{}{} {} {}",
                if i == 0 { label("Types") } else { label("") },
                styles.bar(bucket.count as f64 / max, 12),
                styles.dim(&bucket.label),
                styles.thousands(bucket.count),
            );
        }
    }
}

/// Owned counts keyed by card name: `(owned and assigned to this deck, owned
/// in binders)`.
///
/// Matching is by name, not by print: any printing you own fills a deck
/// line. Deck lines that name a specific print stay useful for tracking
/// which copy to sleeve, but a different set version covers the slot just
/// the same — that keeps reprints usable to avoid buying the exact print.
pub(crate) fn owned_map_for_deck(
    conn: &Connection,
    deck: &str,
) -> anyhow::Result<std::collections::HashMap<String, (i64, i64)>> {
    // Copies assigned to this deck in the collection, grouped by name.
    let mut stmt = conn.prepare(
        "SELECT c.name, SUM(c.quantity)
         FROM collection c JOIN cards k ON k.name = c.name
         WHERE c.binder_type = 'deck' AND c.binder = ?1
         GROUP BY c.name",
    )?;
    let rows = stmt.query_map([deck], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (name, qty) = row.context("reading owned deck rows")?;
        map.entry(name).or_insert((0i64, 0i64)).0 += qty;
    }
    // Copies in binders: the same name anywhere else (for the "elsewhere"
    // hint).
    let mut stmt = conn.prepare(
        "SELECT c.name, SUM(c.quantity)
         FROM collection c JOIN cards k ON k.name = c.name
         WHERE c.binder_type = 'binder'
         GROUP BY c.name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (name, qty) = row.context("reading binder rows")?;
        map.entry(name).or_insert((0i64, 0i64)).1 += qty;
    }
    Ok(map)
}

/// Deck value split: what the copies filling this deck are worth, and what
/// buying the rest costs at the cheapest printing.
///
/// A slot is filled by deck-assigned copies first, then binder copies —
/// the same rule `deck buylist` uses, so `missing_cost` here equals the
/// buylist total. Basic lands cost nothing (unlimited supply assumption).
/// Each entry prices at its own finish (`entry_unit_price`): a name split
/// across foil and nonfoil entries sums each entry at its correct rate.
pub(crate) fn deck_value(
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    prices: &std::collections::HashMap<String, FinishPrices>,
    available: &std::collections::HashMap<String, i64>,
) -> (f64, f64) {
    let mut owned_value = 0.0f64;
    let mut missing_cost = 0.0f64;
    for entry in deck.entries() {
        let Some(card) = cards_by_name.get(&entry.name) else {
            continue;
        };
        if super::stats::is_tracked_basic(card) {
            continue;
        }
        let Some(unit) = entry_unit_price(entry, prices) else {
            continue;
        };
        let filled = available
            .get(&entry.name)
            .copied()
            .unwrap_or(0)
            .min(entry.quantity);
        owned_value += unit * filled as f64;
        missing_cost += unit * (entry.quantity - filled) as f64;
    }
    (owned_value, missing_cost)
}

/// Full set name for a set code (`sets` table; `None` when unknown).
fn set_name_for(conn: &Connection, set_code: &str) -> Option<String> {
    conn.query_row(
        "SELECT set_name FROM sets WHERE set_code = ?1",
        [set_code.to_ascii_lowercase()],
        |r| r.get(0),
    )
    .ok()
}

/// Universes Beyond census over the deck's main sections (commander + deck,
/// not the sideboard): total copies per universe, per-franchise counts, and
/// the UB card names. `None` when the store has no set metadata yet.
///
/// Built in one pass over the maindeck entries: per-name copies come from
/// the same iteration (no per-card rescans), so the census is O(n).
pub(crate) fn universe_census(
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    out: &mut crate::output::Output,
) -> Option<serde_json::Value> {
    let mut multiverse = 0i64;
    let mut beyond = 0i64;
    let mut franchises: std::collections::BTreeMap<String, i64> = Default::default();
    let mut ub_cards: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut warned = false;
    // Copies per distinct name across maindeck sections, built in one
    // pass (the census is per distinct name, so 2+1 copies of one name
    // count once as 3).
    let mut qty_by_name: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    let ordered: Vec<&str> = deck
        .sections
        .iter()
        .filter(|(s, _)| !super::grammar::is_bench_section(s))
        .flat_map(|(_, e)| e.iter())
        .filter(|e| {
            let count = qty_by_name.entry(e.name.as_str()).or_insert(0);
            *count += e.quantity;
            seen.insert(e.name.as_str())
        })
        .map(|e| e.name.as_str())
        .collect();
    for name in ordered {
        let Some(card) = cards_by_name.get(name) else {
            continue;
        };
        // Quantity of the distinct name across maindeck sections, summed
        // during this pass (the census iterates entries directly).
        let Ok(meta) = crate::universe::card_universe(conn, &card.name, &card.set_code) else {
            // A failed universe lookup undercounts the census; warn once
            // so the reader knows the numbers are partial.
            if !warned {
                out.warning("universe metadata lookup failed; the census may be incomplete");
                warned = true;
            }
            continue;
        };
        let qty = qty_by_name[name];
        match meta.universe {
            "beyond" => {
                beyond += qty;
                ub_cards.push(card.name.clone());
                if let Some(franchise) = &meta.franchise {
                    *franchises.entry(franchise.clone()).or_insert(0) += qty;
                }
            }
            _ => multiverse += qty,
        }
    }
    Some(serde_json::json!({
        "multiverse": multiverse,
        "universes_beyond": beyond,
        "franchises": franchises,
        "ub_cards": ub_cards,
    }))
}

/// Entry point for `stm deck show <name>` (also the `stm deck <name>` sugar).
pub fn show(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    name: &str,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = load_deck(paths, name)?;
    let owned_map = owned_map_for_deck(conn, name)?;
    // Slot coverage uses the buylist rule: deck-assigned + binders fill
    // slots, so missing numbers agree across `show` and `buylist`.
    let assigned = super::ownership::deck_assigned_map(conn, name)?;
    let available = super::ownership::available_map(conn, name)?;
    let cards_by_name = super::stats::lookup_names(conn, &deck)?;
    let held_elsewhere = super::ownership::held_elsewhere_map(conn, name)?;
    let slots = super::ownership::slot_map(&deck, &available, &assigned, &held_elsewhere, |name| {
        cards_by_name
            .get(name)
            .is_some_and(super::stats::is_tracked_basic)
    });
    let prices = deck_finish_prices(conn, &deck, out);
    let primer = primer_file(paths, name);
    let census = universe_census(conn, &deck, &cards_by_name, out);
    if json {
        show_json(
            conn,
            &deck,
            name,
            &primer,
            &owned_map,
            &slots,
            &cards_by_name,
            &prices,
            &available,
            census,
        )?;
        return Ok(crate::cli::codes::OK);
    }
    show_human(
        conn,
        out,
        name,
        &primer,
        &deck,
        &slots,
        &cards_by_name,
        &prices,
        &held_elsewhere,
        census,
    );
    Ok(crate::cli::codes::OK)
}

/// Inputs to the JSON view, grouped so the helper stays readable.
struct ShowJsonView<'a> {
    deck: &'a super::Deck,
    name: &'a str,
    primer: &'a std::path::Path,
    owned_map: &'a std::collections::HashMap<String, (i64, i64)>,
    slots: &'a std::collections::HashMap<String, super::ownership::SlotOwnership>,
    cards_by_name: &'a std::collections::HashMap<String, crate::db::CardRow>,
    prices: &'a std::collections::HashMap<String, FinishPrices>,
    available: &'a std::collections::HashMap<String, i64>,
    census: Option<serde_json::Value>,
}

/// One deck line as a JSON object (the `sections[].cards` rows).
#[allow(clippy::too_many_arguments)]
fn show_json(
    conn: &Connection,
    deck: &super::Deck,
    name: &str,
    primer: &std::path::Path,
    owned_map: &std::collections::HashMap<String, (i64, i64)>,
    slots: &std::collections::HashMap<String, super::ownership::SlotOwnership>,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    prices: &std::collections::HashMap<String, FinishPrices>,
    available: &std::collections::HashMap<String, i64>,
    census: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    let view = ShowJsonView {
        deck,
        name,
        primer,
        owned_map,
        slots,
        cards_by_name,
        prices,
        available,
        census,
    };
    show_json_body(conn, &view)
}

/// The JSON render itself (split from argument plumbing).
fn show_json_body(conn: &Connection, view: &ShowJsonView<'_>) -> anyhow::Result<()> {
    let ShowJsonView {
        deck,
        name,
        primer,
        owned_map,
        slots,
        cards_by_name,
        prices,
        available,
        census,
    } = view;
    // Full set names per code (a codes→names cache so a 100-card deck
    // reads ~2 set rows, not 100).
    let mut set_names: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let sections: Vec<serde_json::Value> = deck
        .sections
        .iter()
        .map(|(section, entries)| {
            let lines: Vec<serde_json::Value> = entries
                .iter()
                .map(|entry| {
                    show_json_entry(
                        conn,
                        entry,
                        slots,
                        owned_map,
                        cards_by_name,
                        prices,
                        &mut set_names,
                    )
                })
                .collect();
            serde_json::json!({ "section": section, "cards": lines })
        })
        .collect();
    let (owned_value, missing_cost) = deck_value(deck, cards_by_name, prices, available);
    let stats = super::stats::compute(deck, cards_by_name);
    let is_commander = super::legal::is_commander(deck, None);
    let mut v = serde_json::json!({
        "name": name,
        "cards": deck.maindeck_total(),
        "sideboard_cards": deck.sideboard_total(),
        "maybeboard_cards": deck.maybeboard_total(),
        "primer": primer,
        "currency": crate::output::CURRENCY,
        "owned_value": crate::output::round2(owned_value),
        "missing_cost": crate::output::round2(missing_cost),
        "sections": sections,
    });
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "curve".into(),
            super::stats::curve_json(&stats, is_commander),
        );
        obj.insert("ramp".into(), super::stats::ramp_json(&stats));
    }
    if let (Some(obj), Some(census)) = (v.as_object_mut(), census.as_ref()) {
        obj.insert("universe_census".into(), census.clone());
    }
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

/// One JSON card row: ownership, coverage, and the entry's own per-finish
/// price (foil entries report the foil price, nonfoil the nonfoil price).
#[allow(clippy::too_many_arguments)]
fn show_json_entry(
    conn: &Connection,
    entry: &crate::deck::grammar::DeckEntry,
    slots: &std::collections::HashMap<String, super::ownership::SlotOwnership>,
    owned_map: &std::collections::HashMap<String, (i64, i64)>,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    prices: &std::collections::HashMap<String, FinishPrices>,
    set_names: &mut std::collections::HashMap<String, Option<String>>,
) -> serde_json::Value {
    let (assigned_here, elsewhere_binder) = owned_map.get(&entry.name).copied().unwrap_or((0, 0));
    let basic = cards_by_name
        .get(&entry.name)
        .is_some_and(super::stats::is_tracked_basic);
    let slot = slots.get(&entry.name);
    let coverage = match slot.map(|s| s.coverage) {
        Some(super::ownership::Coverage::Deck) => "deck",
        Some(super::ownership::Coverage::Binder) => "binder",
        _ if basic => "basic",
        _ => "missing",
    };
    // `owned` = copies available to this deck (assigned here + binders),
    // never contradicting coverage. `owned_elsewhere` = other decks' copies.
    let owned = slot
        .map(|s| s.in_deck + s.in_binder)
        .unwrap_or(assigned_here + elsewhere_binder);
    let missing_reason: serde_json::Value = if coverage == "missing" {
        match slot.map(|s| s.held_elsewhere > 0) {
            Some(true) => serde_json::json!("held_elsewhere"),
            _ => serde_json::json!("not_owned"),
        }
    } else {
        serde_json::Value::Null
    };
    serde_json::json!({
        "quantity": entry.quantity,
        "name": entry.name,
        "set": entry.set_code,
        "set_name": match &entry.set_code {
            None => serde_json::Value::Null,
            Some(code) if code.is_empty() => serde_json::Value::Null,
            Some(code) => set_names
                .entry(code.clone())
                .or_insert_with(|| set_name_for(conn, code))
                .clone()
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
        },
        "collector_number": entry.collector_number,
        "foil": entry.foil,
        "owned": owned,
        "assigned_to_this_deck": slot.map(|s| s.in_deck).unwrap_or(assigned_here),
        "owned_elsewhere": slot.map(|s| s.held_elsewhere).unwrap_or(0),
        "covered_by": coverage,
        "missing_reason": missing_reason,
        "basic_land": basic,
        "price": entry_unit_price(entry, prices),
    })
}

/// The human render: header, overview, to-buy block, census note, and the
/// per-section card table.
#[allow(clippy::too_many_arguments)]
fn show_human(
    conn: &Connection,
    out: &mut crate::output::Output,
    name: &str,
    primer: &std::path::Path,
    deck: &super::Deck,
    slots: &std::collections::HashMap<String, super::ownership::SlotOwnership>,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    prices: &std::collections::HashMap<String, FinishPrices>,
    held_elsewhere: &std::collections::HashMap<String, (i64, Vec<String>)>,
    census: Option<serde_json::Value>,
) {
    let styles = out.styles();
    let sideboard = deck.sideboard_total();
    let maybeboard = deck.maybeboard_total();
    let mut count_display = format!("{} cards", deck.maindeck_total());
    if sideboard > 0 {
        count_display.push_str(&format!(" + {sideboard} sideboard"));
    }
    if maybeboard > 0 {
        count_display.push_str(&format!(" + {maybeboard} maybeboard"));
    }
    println!(
        "{}  {}  {}",
        styles.header(name),
        styles.dim(&count_display),
        styles.dim(&format!("primer: {}", primer.display())),
    );
    print_overview(&styles, conn, deck, name, out);
    print_to_buy_block(&styles, slots, prices, held_elsewhere);
    print_universe_note(&styles, census.as_ref());
    // Set names resolve once per distinct code, not once per deck line
    // (the JSON path caches the same way).
    let mut set_names: std::collections::HashMap<String, String> = Default::default();
    for (section, entries) in &deck.sections {
        println!();
        println!("{}", styles.header(&format!("// {section}")));
        for entry in entries {
            print_deck_line(
                &styles,
                conn,
                entry,
                slots,
                cards_by_name,
                prices,
                &mut set_names,
            );
        }
    }
}

/// The to-buy block: missing slots at the entry's per-finish price, most
/// expensive first, with the running total (same math as `deck buylist`).
/// Rows without a reason are simply unowned (implied); the reason names
/// the holding deck when the copy exists elsewhere.
fn print_to_buy_block(
    styles: &crate::output::Styles,
    slots: &std::collections::HashMap<String, super::ownership::SlotOwnership>,
    prices: &std::collections::HashMap<String, FinishPrices>,
    held_elsewhere: &std::collections::HashMap<String, (i64, Vec<String>)>,
) {
    print!("{}", to_buy_text(styles, slots, prices, held_elsewhere));
}

/// The rendered to-buy block text (empty when nothing is missing). The
/// builder the printer consumes; split out so ordering and summary are
/// assertable.
#[cfg(test)]
pub(crate) fn to_buy_block_text(
    styles: &crate::output::Styles,
    slots: &std::collections::HashMap<String, super::ownership::SlotOwnership>,
    prices: &std::collections::HashMap<String, (Option<f64>, Option<f64>)>,
    held_elsewhere: &std::collections::HashMap<String, (i64, Vec<String>)>,
) -> String {
    to_buy_text(styles, slots, prices, held_elsewhere)
}

/// The builder both the printer and the tests consume.
fn to_buy_text(
    styles: &crate::output::Styles,
    slots: &std::collections::HashMap<String, super::ownership::SlotOwnership>,
    prices: &std::collections::HashMap<String, FinishPrices>,
    held_elsewhere: &std::collections::HashMap<String, (i64, Vec<String>)>,
) -> String {
    let mut to_buy: Vec<(String, i64, Option<f64>, Option<String>)> = slots
        .iter()
        .filter(|(_, slot)| slot.missing > 0)
        .map(|(name, slot)| {
            let reason = (slot.held_elsewhere > 0).then(|| {
                // A slot only reports held_elsewhere when the map named a
                // holder; a missing entry degrades to a plain count.
                match held_elsewhere.get(name) {
                    Some((_, holders)) if holders.len() == 1 => {
                        format!("1 copy in {}", holders[0])
                    }
                    Some((_, holders)) => format!(
                        "{} copies across {}",
                        slot.held_elsewhere,
                        holders.join(", ")
                    ),
                    None => format!("{} copies in other decks", slot.held_elsewhere),
                }
            });
            (
                name.clone(),
                slot.missing,
                // A missing slot has no specific finish to price; the
                // nonfoil rate is the buy price (buying new is nonfoil).
                prices
                    .get(name)
                    .and_then(|(foil, nonfoil)| nonfoil.or(*foil)),
                reason,
            )
        })
        .collect();
    // Unpriced rows sort last (after every priced row), priced rows by
    // total cost descending.
    to_buy.sort_by(|a, b| match (a.2, b.2) {
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => {
            let a_cost = a.1 as f64 * a.2.unwrap_or(0.0);
            let b_cost = b.1 as f64 * b.2.unwrap_or(0.0);
            b_cost.total_cmp(&a_cost).then_with(|| a.0.cmp(&b.0))
        }
    });
    if to_buy.is_empty() {
        return String::new();
    }
    let mut text = String::new();
    text.push('\n');
    text.push_str(&styles.header("To buy"));
    text.push('\n');
    // Pad names so the price and reason columns line up.
    let name_width = to_buy
        .iter()
        .take(8)
        .map(|(card, _, _, _)| card.chars().count())
        .max()
        .unwrap_or(0);
    for (card, qty, unit, reason) in to_buy.iter().take(8) {
        let name = format!("{:<name_width$}", card, name_width = name_width);
        let price = match unit {
            Some(unit) => styles.money(*unit),
            None => styles.dim("unpriced"),
        };
        let reason_note = match reason {
            Some(reason) => format!(" {}", styles.dim(&format!("({reason})"))),
            None => format!(" {}", styles.dim("(not owned)")),
        };
        text.push_str(&format!(
            "  {:>2} {}  {}{}\n",
            qty,
            styles.card_name(&name),
            price,
            reason_note
        ));
    }
    if to_buy.len() > 8 {
        text.push_str(&styles.dim(&format!("… and {} more lines\n", to_buy.len() - 8)));
    }
    let priced_total: f64 = to_buy
        .iter()
        .map(|(_, q, p, _)| (*q as f64) * p.unwrap_or(0.0))
        .sum();
    let unpriced: i64 = to_buy
        .iter()
        .filter(|(_, _, p, _)| p.is_none())
        .map(|(_, q, _, _)| *q)
        .sum();
    let mut parts = vec![format!(
        "{} copies",
        to_buy.iter().map(|(_, q, _, _)| q).sum::<i64>()
    )];
    if unpriced > 0 {
        parts.push(format!("{unpriced} unpriced"));
    }
    text.push_str(&format!(
        "  {} {}\n",
        styles.dim(&format!("{} · est.", parts.join(", "))),
        styles.money(priced_total),
    ));
    text
}

/// The Universes Beyond note under the overview (only when UB cards exist).
fn print_universe_note(styles: &crate::output::Styles, census: Option<&serde_json::Value>) {
    let Some(census) = census else {
        return;
    };
    let beyond = census["universes_beyond"].as_i64().unwrap_or(0);
    if beyond <= 0 {
        return;
    }
    let names = census["ub_cards"]
        .as_array()
        .map(|cards| {
            cards
                .iter()
                .filter_map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    println!();
    println!(
        "{}",
        styles.note(&format!("{beyond} Universes Beyond cards: {names}"))
    );
}

/// One deck-table line: print suffix, ownership note, set name, and the
/// entry's own per-finish unit price (skip basics: free).
fn print_deck_line(
    styles: &crate::output::Styles,
    conn: &Connection,
    entry: &crate::deck::grammar::DeckEntry,
    slots: &std::collections::HashMap<String, super::ownership::SlotOwnership>,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    prices: &std::collections::HashMap<String, FinishPrices>,
    set_names: &mut std::collections::HashMap<String, String>,
) {
    let basic = cards_by_name
        .get(&entry.name)
        .is_some_and(super::stats::is_tracked_basic);
    // Owned = copies available to this deck (assigned here plus binders),
    // shown raw; `elsewhere` counts only copies parked in other decks.
    // Same rule the To-buy block and JSON use.
    let slot = slots.get(&entry.name);
    let owned = slot.map(|s| s.in_deck + s.in_binder).unwrap_or(0);
    let elsewhere = slot.map(|s| s.held_elsewhere).unwrap_or(0);
    // Print identity suffix of the entry line (everything after the name),
    // so set/cn/foil stay visible without duplicating the name.
    let line = entry.to_line();
    let name_len = format!("{} ", entry.quantity).len() + entry.name.len();
    let rest = line.get(name_len..).unwrap_or_default().trim();
    let owned_display = if basic {
        styles.dim("(own ∞)")
    } else if owned >= entry.quantity {
        styles.glyph(
            &format!("(own {owned}/{})", entry.quantity),
            crate::output::GlyphKind::Good,
        )
    } else if owned > 0 {
        styles.glyph(
            &format!("(own {owned}/{})", entry.quantity),
            crate::output::GlyphKind::Warn,
        )
    } else {
        styles.glyph(
            &format!("(own 0/{})", entry.quantity),
            crate::output::GlyphKind::Bad,
        )
    };
    // Full set name when the deck line recorded a set and the store knows
    // its name (ManaBox txt keeps the code).
    let set_note = match &entry.set_code {
        Some(code) => {
            let full = set_names
                .entry(code.clone())
                .or_insert_with(|| set_name_for(conn, code).unwrap_or_default())
                .clone();
            if full.is_empty() {
                String::new()
            } else {
                format!(" ({full})")
            }
        }
        None => String::new(),
    };
    let elsewhere = if elsewhere > 0 && !basic {
        styles.dim(&format!(" (+{elsewhere} elsewhere)"))
    } else {
        String::new()
    };
    // Unit price next to the ownership note (skip basics: free).
    let price_note = match entry_unit_price(entry, prices) {
        Some(unit) if !basic => format!(" {}", styles.money(unit)),
        _ => String::new(),
    };
    println!(
        "  {:>2} {} {}{}  {owned_display}{elsewhere}{price_note}",
        entry.quantity,
        styles.card_name(&entry.name),
        styles.dim(rest),
        styles.dim(&set_note),
    );
}
