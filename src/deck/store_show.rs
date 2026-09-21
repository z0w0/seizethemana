//! `deck show` and `deck <name>`: the human overview, the JSON view, and
//! the per-card table. Split from `store.rs` to stay under the module
//! size limit; `super::*` re-exports the store namespace's helpers.

use super::store::{load_deck, primer_file};
use anyhow::Context;
use rusqlite::Connection;

/// Overview stats block under the `deck show` header: completion, curve,
/// ramp, colors, types. Pure display; JSON output is unaffected.
fn print_overview(
    styles: &crate::output::Styles,
    conn: &Connection,
    deck: &super::Deck,
    deck_name: &str,
) {
    let cards_by_name: std::collections::HashMap<String, crate::db::CardRow> =
        super::stats::lookup_names(conn, deck);
    let stats = super::stats::compute(deck, &cards_by_name);
    let Ok(owned_map) = owned_map_for_deck(conn, deck_name) else {
        return;
    };
    let Ok(available) = super::ownership::available_map(conn, deck_name) else {
        return;
    };
    let owned: i64 = deck
        .entries()
        .map(|entry| {
            // Basic lands are assumed available in unlimited supply; they
            // always count as owned for completion.
            if cards_by_name
                .get(&entry.name)
                .is_some_and(super::stats::is_basic_land)
            {
                return entry.quantity;
            }
            let (in_deck, _) = owned_map.get(&entry.name).copied().unwrap_or((0, 0));
            in_deck.min(entry.quantity)
        })
        .sum();
    if stats.total == 0 {
        return;
    }
    println!();
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
    let prices = deck_prices(conn, deck);
    let (owned_value, missing_cost) = deck_value(deck, &cards_by_name, &prices, &available);
    if owned_value > 0.0 || missing_cost > 0.0 {
        println!(
            "{}{}{}{}",
            label("Value"),
            styles.money(owned_value),
            styles.dim(" owned · missing "),
            styles.money(missing_cost),
        );
    }
    if !stats.curve.is_empty() {
        // One-line curve score: avg MV, the histogram, and the target
        // sentence (format/archetype-aware, same mapping as the JSON
        // `curve` block).
        let is_commander = super::legal::is_commander(deck, None);
        let histogram = super::stats::curve_histogram(&stats);
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
    if let Ok(audit) = super::mana::mana_audit_for(conn, deck) {
        super::mana::print_mana_audit(styles, &audit);
    }
    println!();
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

/// Cheapest print price for every deck card name, keyed by name.
///
/// A foil deck entry prices at the cheapest foil printing when one exists,
/// falling back to the cheapest normal print.
pub(crate) fn deck_prices(
    conn: &Connection,
    deck: &super::Deck,
) -> std::collections::HashMap<String, Option<f64>> {
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in deck.entries().map(|e| e.name.clone()) {
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }
    // One batched query per finish kind instead of four statements per
    // card name.
    let ranges =
        crate::prints::price_ranges(conn, &names).expect("price ranges query failed for deck show");
    let mut map = std::collections::HashMap::new();
    for name in names {
        let range = ranges.get(&name);
        // A foil deck entry prefers the cheapest foil print when one exists.
        let is_foil = deck.entries().any(|e| e.name == name && e.foil);
        let price = match range {
            Some(range) if is_foil => range
                .cheapest_foil
                .as_ref()
                .and_then(|p| p.usd_foil)
                .or_else(|| range.cheapest.as_ref().and_then(|p| p.usd)),
            Some(range) => range.cheapest.as_ref().and_then(|p| p.usd),
            None => None,
        };
        map.insert(name, price);
    }
    map
}

/// Deck value split: what the copies filling this deck are worth, and what
/// buying the rest costs at the cheapest printing.
///
/// A slot is filled by deck-assigned copies first, then binder copies —
/// the same rule `deck buylist` uses, so `missing_cost` here equals the
/// buylist total. Basic lands cost nothing (unlimited supply assumption).
pub(crate) fn deck_value(
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    prices: &std::collections::HashMap<String, Option<f64>>,
    available: &std::collections::HashMap<String, i64>,
) -> (f64, f64) {
    let mut owned_value = 0.0f64;
    let mut missing_cost = 0.0f64;
    for entry in deck.entries() {
        let Some(card) = cards_by_name.get(&entry.name) else {
            continue;
        };
        if super::stats::is_basic_land(card) {
            continue;
        }
        let Some(Some(unit)) = prices.get(&entry.name).copied() else {
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

/// Round to two decimals for JSON money fields.
pub(crate) fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
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
pub(crate) fn universe_census(
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
) -> Option<serde_json::Value> {
    let mut multiverse = 0i64;
    let mut beyond = 0i64;
    let mut franchises: std::collections::BTreeMap<String, i64> = Default::default();
    let mut ub_cards: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for entry in deck.sections.iter().flat_map(|(s, e)| {
        if s.eq_ignore_ascii_case("SIDEBOARD") {
            Vec::new()
        } else {
            e.clone()
        }
    }) {
        let Some(card) = cards_by_name.get(&entry.name) else {
            continue;
        };
        if !seen.insert(card.name.as_str()) {
            continue;
        }
        let meta = crate::universe::card_universe(conn, &card.name, &card.set_code).ok()?;
        let qty = deck
            .sections
            .iter()
            .filter(|(s, _)| !s.eq_ignore_ascii_case("SIDEBOARD"))
            .flat_map(|(_, e)| e.iter())
            .filter(|e| e.name == card.name)
            .map(|e| e.quantity)
            .sum::<i64>();
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
    let cards_by_name = super::stats::lookup_names(conn, &deck);
    let held_elsewhere = super::ownership::held_elsewhere_map(conn, name)?;
    let slots = super::ownership::slot_map(&deck, &available, &assigned, &held_elsewhere, |name| {
        cards_by_name
            .get(name)
            .is_some_and(super::stats::is_basic_land)
    });
    let prices = deck_prices(conn, &deck);
    let primer = primer_file(paths, name);
    let universe_census = universe_census(conn, &deck, &cards_by_name);
    // Full set names per code for the JSON entries (a codes→names cache so
    // a 100-card deck reads ~2 set rows, not 100).
    let mut set_names: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();

    if json {
        let sections: Vec<serde_json::Value> = deck
            .sections
            .iter()
            .map(|(section, entries)| {
                let lines: Vec<serde_json::Value> = entries
                    .iter()
                    .map(|entry| {
                        let (assigned_here, elsewhere_binder) =
                            owned_map.get(&entry.name).copied().unwrap_or((0, 0));
                        let basic = cards_by_name
                            .get(&entry.name)
                            .is_some_and(super::stats::is_basic_land);
                        let slot = slots.get(&entry.name);
                        let coverage = match slot.map(|s| s.coverage) {
                            Some(super::ownership::Coverage::Deck) => "deck",
                            Some(super::ownership::Coverage::Binder) => "binder",
                            _ if basic => "basic",
                            _ => "missing",
                        };
                        // `owned` = copies available to this deck (assigned
                        // here + binders), never contradicting coverage.
                        // `owned_elsewhere` = other decks' copies.
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
                            "price": prices.get(&entry.name).and_then(|p| *p),
                        })
                    })
                    .collect();
                serde_json::json!({ "section": section, "cards": lines })
            })
            .collect();
        let (owned_value, missing_cost) = deck_value(&deck, &cards_by_name, &prices, &available);
        let stats = super::stats::compute(&deck, &cards_by_name);
        let is_commander = super::legal::is_commander(&deck, None);
        let mut v = serde_json::json!({
            "name": name,
            "cards": deck.maindeck_total(),
            "sideboard_cards": deck.sideboard_total(),
            "primer": primer,
            "currency": crate::output::CURRENCY,
            "owned_value": round2(owned_value),
            "missing_cost": round2(missing_cost),
            "sections": sections,
        });
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "curve".into(),
                super::stats::curve_json(&stats, is_commander),
            );
            obj.insert("ramp".into(), super::stats::ramp_json(&stats));
        }
        if let Some(obj) = v.as_object_mut()
            && let Some(census) = universe_census
        {
            obj.insert("universe_census".into(), census);
        }
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(crate::cli::codes::OK);
    }

    let styles = out.styles();
    let sideboard = deck.sideboard_total();
    let count_display = if sideboard > 0 {
        format!("{} cards + {} sideboard", deck.maindeck_total(), sideboard)
    } else {
        format!("{} cards", deck.maindeck_total())
    };
    println!(
        "{}  {}  {}",
        styles.header(name),
        styles.dim(&count_display),
        styles.dim(&format!("primer: {}", primer.display())),
    );
    print_overview(&styles, conn, &deck, name);
    // To-buy block: the missing slots at the cheapest printing, most
    // expensive first, with the running total (same math as `deck buylist`).
    // The reason names the holding deck when the copy exists elsewhere.
    let mut to_buy: Vec<(String, i64, f64, String)> = slots
        .iter()
        .filter(|(_, slot)| slot.missing > 0)
        .map(|(name, slot)| {
            let reason = if slot.held_elsewhere > 0 {
                let (_, holders) = &held_elsewhere[name];
                if holders.len() == 1 {
                    format!("1 copy in {}", holders[0])
                } else {
                    format!(
                        "{} copies across {}",
                        slot.held_elsewhere,
                        holders.join(", ")
                    )
                }
            } else {
                "not owned".to_string()
            };
            (
                name.clone(),
                slot.missing,
                prices.get(name).copied().flatten().unwrap_or(0.0),
                reason,
            )
        })
        .collect();
    to_buy.sort_by(|a, b| {
        let a_cost = a.1 as f64 * a.2;
        let b_cost = b.1 as f64 * b.2;
        b_cost.total_cmp(&a_cost).then_with(|| a.0.cmp(&b.0))
    });
    if !to_buy.is_empty() {
        println!();
        println!("{}", styles.header("To buy"));
        for (card, qty, unit, reason) in to_buy.iter().take(8) {
            println!(
                "  {:>2}  {}  {} {}",
                qty,
                styles.card_name(card),
                styles.dim(&format!("@{}", styles.money(*unit))),
                styles.dim(&format!("({reason})"))
            );
        }
        if to_buy.len() > 8 {
            println!(
                "  {}",
                styles.dim(&format!("… and {} more lines", to_buy.len() - 8))
            );
        }
        let total: f64 = to_buy.iter().map(|(_, q, p, _)| (*q as f64) * p).sum();
        println!(
            "  {} {}",
            styles.dim(&format!(
                "{} copies · est.",
                to_buy.iter().map(|(_, q, _, _)| q).sum::<i64>()
            )),
            styles.money(total),
        );
    }
    if universe_census.is_some()
        && let Some(census) = &universe_census
    {
        let beyond = census["universes_beyond"].as_i64().unwrap_or(0);
        if beyond > 0 {
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
    }
    for (section, entries) in &deck.sections {
        println!();
        println!("{}", styles.header(&format!("// {section}")));
        for entry in entries {
            let (owned, elsewhere) = owned_map.get(&entry.name).copied().unwrap_or((0, 0));
            // Print identity suffix of the entry line (everything after the
            // name), so set/cn/foil stay visible without duplicating the name.
            let line = entry.to_line();
            let name_len = format!("{} ", entry.quantity).len() + entry.name.len();
            let rest = line.get(name_len..).unwrap_or_default().trim();
            let basic = cards_by_name
                .get(&entry.name)
                .is_some_and(super::stats::is_basic_land);
            let owned_display = if basic {
                styles.dim("(basics unlimited)")
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
            // Full set name when the deck line recorded a set and the store
            // knows its name (ManaBox txt keeps the code).
            let set_note = match &entry.set_code {
                Some(code) => {
                    let name = set_name_for(conn, code);
                    match name {
                        Some(full) => format!(" ({full})"),
                        None => String::new(),
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
            let price_note = match prices.get(&entry.name).copied().flatten() {
                Some(unit) if !basic => {
                    format!(" {}{}", styles.dim("@"), styles.money(unit))
                }
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
    }
    Ok(crate::cli::codes::OK)
}
