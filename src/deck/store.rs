// Deck file storage and read commands (`create`, `list`, `show`).
//
// Deck txt files under `decks/` are the single source of truth for deck
// contents; ownership comes from the collection (`binder_type = 'deck'`
// rows); primers are sibling markdown files the deckbuilding agent edits.

use rusqlite::Connection;

use super::grammar::Deck;
use anyhow::Context;

/// Primer file path for a deck (`decks/<name>.primer.md`).
pub fn primer_file(paths: &crate::paths::Paths, name: &str) -> std::path::PathBuf {
    paths.decks_dir().join(format!("{name}.primer.md"))
}

/// Validate a deck name: a single path component (no separators or dots).
pub fn valid_deck_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '\'' | '&' | '!' | '+' | ',')
        })
        && !name.starts_with('.')
}

/// Ensure a deck file exists (empty when absent); returns its path.
pub(crate) fn ensure_deck_file(
    paths: &crate::paths::Paths,
    name: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let path = paths.deck_file(name);
    if !path.exists() {
        std::fs::create_dir_all(paths.decks_dir()).context("creating decks directory")?;
        std::fs::write(&path, "")
            .with_context(|| format!("creating deck file {}", path.display()))?;
    }
    Ok(path)
}

/// Ensure the primer file exists (empty stub); returns its path.
pub(crate) fn ensure_primer_file(
    paths: &crate::paths::Paths,
    name: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let primer = primer_file(paths, name);
    if !primer.exists() {
        std::fs::create_dir_all(paths.decks_dir()).context("creating decks directory")?;
        std::fs::write(&primer, "")
            .with_context(|| format!("creating primer {}", primer.display()))?;
    }
    Ok(primer)
}

/// Load and parse a deck file.
pub(crate) fn load_deck(
    paths: &crate::paths::Paths,
    name: &str,
) -> anyhow::Result<(std::path::PathBuf, Deck)> {
    let path = paths.deck_file(name);
    if !path.exists() {
        // Distinguish a missing deck so callers can hint consistently.
        return Err(anyhow::anyhow!(DeckNotFound {
            name: name.to_string()
        }));
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let deck = Deck::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok((path, deck))
}

/// A referenced deck file does not exist; rendered as `error:` + `hint:`.
#[derive(Debug, thiserror::Error)]
#[error("deck {name:?} not found")]
pub struct DeckNotFound {
    pub name: String,
}

/// True when an error chain's root is a missing deck file.
pub fn is_deck_not_found(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|c| c.downcast_ref::<DeckNotFound>().is_some())
}

/// Save a deck back to its txt file (parents ensured).
pub(crate) fn save_deck(
    paths: &crate::paths::Paths,
    name: &str,
    deck: &Deck,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(paths.decks_dir()).context("creating decks directory")?;
    std::fs::write(paths.deck_file(name), deck.to_text())
        .with_context(|| format!("writing deck {name:?}"))
}

/// Entry point for `stm deck create <name>`.
pub fn create(
    paths: &crate::paths::Paths,
    out: &mut crate::output::Output,
    name: &str,
) -> anyhow::Result<i32> {
    ensure_valid_name(out, name)?;
    let path = paths.deck_file(name);
    if path.exists() {
        out.error(&format!("deck {name:?} already exists"));
        out.hint(&format!("use 'stm deck show {name}' to view it"));
        return Ok(crate::cli::codes::ERROR);
    }
    ensure_deck_file(paths, name)?;
    let primer = ensure_primer_file(paths, name)?;
    out.finish(
        "Created",
        &format!(
            "deck {name:?} ({}; primer {})",
            path.display(),
            primer.display()
        ),
        std::time::Duration::ZERO,
    );
    Ok(crate::cli::codes::OK)
}

pub(crate) fn ensure_valid_name(out: &mut crate::output::Output, name: &str) -> anyhow::Result<()> {
    if !valid_deck_name(name) {
        out.error(&format!("invalid deck name {name:?}"));
        out.hint("use letters, digits, spaces, or - _ ' & ! + , (no / or ..)");
        anyhow::bail!("invalid deck name");
    }
    Ok(())
}

/// Deck summary for `stm deck list`.
///
/// `cards` is the maindeck count; `sideboard_cards` reports the sideboard
/// separately (a commander wishlist, not a legal zone).
#[derive(Debug, Clone)]
pub struct DeckSummary {
    pub name: String,
    pub cards: i64,
    pub sideboard_cards: i64,
    pub owned: i64,
    pub has_primer: bool,
}

/// Entry point for `stm deck list`.
pub fn list(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    json: bool,
) -> anyhow::Result<i32> {
    let decks = discover_decks(paths, conn)?;
    // Collection decks with no decklist file: the import gap.
    let mut gaps = registered_deck_names(conn)?;
    for d in &decks {
        gaps.retain(|g| !g.eq_ignore_ascii_case(&d.name));
    }
    if decks.is_empty() && gaps.is_empty() {
        if json {
            println!("[]");
            return Ok(crate::cli::codes::OK);
        }
        out.error("no decks found");
        out.hint("create one: stm deck create <name>, or import: stm deck import <name> <file>");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if json {
        let mut items: Vec<serde_json::Value> = decks
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "has_decklist": true,
                    "cards": d.cards,
                    "sideboard_cards": d.sideboard_cards,
                    "owned": d.owned,
                    "has_primer": d.has_primer,
                })
            })
            .collect();
        for gap in &gaps {
            items.push(serde_json::json!({
                "name": gap,
                "has_decklist": false,
                "cards": 0,
                "sideboard_cards": 0,
                "owned": owned_count_for(conn, gap)?,
                "has_primer": false,
            }));
        }
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(crate::cli::codes::OK);
    }
    let styles = out.styles();
    println!("{}", styles.header("Decks"));
    for d in &decks {
        let primer = if d.has_primer { " primer" } else { "" };
        let count = if d.sideboard_cards > 0 {
            format!("{} cards + {} sideboard", d.cards, d.sideboard_cards)
        } else {
            format!("{} cards", d.cards)
        };
        // Completion coloring: green when fully owned, yellow when partial.
        let owned_display = if d.owned >= d.cards && d.cards > 0 {
            styles.success(&format!("own {}/{}", d.owned, d.cards))
        } else if d.owned > 0 {
            styles.warning(&format!("own {}/{}", d.owned, d.cards))
        } else {
            format!("own {}/{}", d.owned, d.cards)
        };
        println!(
            "  {} {}  {}{}",
            styles.card_name(&d.name),
            styles.dim(&count),
            owned_display,
            styles.dim(primer),
        );
    }
    if !gaps.is_empty() {
        println!();
        println!("{}", styles.note("no decklist (owned in the collection):"));
        for gap in &gaps {
            let owned = owned_count_for(conn, gap)?;
            println!(
                "  {}  {}  {}",
                styles.card_name(gap),
                styles.dim(&format!("{owned} copies owned")),
                styles.dim(&format!("→ stm deck import \"{gap}\" <manabox-deck.txt>")),
            );
        }
    }
    Ok(crate::cli::codes::OK)
}

/// Deck names registered in the collection (`binder_type = 'deck'`), sorted.
fn registered_deck_names(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT binder FROM collection
         WHERE binder_type = 'deck' ORDER BY binder COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()
        .context("reading deck names")?;
    Ok(rows)
}

/// Entry point for `stm deck delete <name>`.
///
/// Removes the decklist (txt + primer). Ownership rows in the collection
/// are never touched: the decklist and the ownership assignment are two
/// different things.
pub fn delete(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    name: &str,
) -> anyhow::Result<i32> {
    ensure_valid_name(out, name)?;
    let path = paths.deck_file(name);
    if !path.exists() {
        out.error(&format!("no decklist for {name:?}"));
        out.hint("list decklists: stm deck list");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let cards = Deck::parse(&std::fs::read_to_string(&path).unwrap_or_default())
        .map(|d| d.total())
        .unwrap_or(0);
    let primer = primer_file(paths, name);
    std::fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
    if primer.exists() {
        std::fs::remove_file(&primer).with_context(|| format!("deleting {}", primer.display()))?;
    }
    out.finish(
        "Deleted",
        &format!("deck {name:?} ({cards} cards, decklist + primer)"),
        std::time::Duration::ZERO,
    );
    let (in_deck, in_binders) = owned_copies(conn, name)?;
    if in_deck + in_binders > 0 {
        out.print_note(&format!(
            "{in_deck} deck-assigned + {in_binders} binder copies are still tracked for \
             {name:?} in the collection; ownership is unaffected"
        ));
    }
    Ok(crate::cli::codes::OK)
}

/// Deck files on disk (`*.txt`), with card/owned counts, alphabetical.
fn discover_decks(
    paths: &crate::paths::Paths,
    conn: &Connection,
) -> anyhow::Result<Vec<DeckSummary>> {
    let mut decks = Vec::new();
    let entries = match std::fs::read_dir(paths.decks_dir()) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(decks),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let path = entry.with_context(|| "reading decks directory")?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let name = deck_name_of(&path);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let deck = match Deck::parse(&text) {
            Ok(deck) => deck,
            // A half-written deck must not break `deck list`.
            Err(_) => continue,
        };
        decks.push(DeckSummary {
            cards: deck.maindeck_total(),
            sideboard_cards: deck.sideboard_total(),
            owned: owned_count_for(conn, &name)?,
            has_primer: primer_file(paths, &name).exists(),
            name,
        });
    }
    decks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(decks)
}

/// The deck name is the file stem (the same string used for collection rows).
fn deck_name_of(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Owned copies assigned to this deck in the collection.
fn owned_count_for(conn: &Connection, name: &str) -> anyhow::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(quantity), 0) FROM collection
         WHERE binder_type = 'deck' AND binder = ?1",
        [name],
        |r| r.get(0),
    )
    .context("counting owned deck cards")
}

/// `(deck-assigned copies, binder copies)` for one deck name.
///
/// Binder copies can fill deck slots (they are the user's cards); other
/// decks' copies never count. Read-only.
pub fn owned_copies(conn: &Connection, name: &str) -> anyhow::Result<(i64, i64)> {
    let in_deck = owned_count_for(conn, name)?;
    let in_binders: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(quantity), 0) FROM collection
         WHERE binder_type = 'binder'
           AND name IN (SELECT name FROM collection WHERE binder_type = 'deck' AND binder = ?1)",
            [name],
            |r| r.get(0),
        )
        .context("counting binder copies for deck names")?;
    Ok((in_deck, in_binders))
}

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
            "{}{}{}",
            label("Value"),
            styles.money(owned_value),
            styles.dim(&format!(" owned · missing ${missing_cost:.2}")),
        );
    }
    if !stats.curve.is_empty() {
        for (i, bucket) in stats.curve.iter().enumerate() {
            let suffix = if i == 0 {
                format!(" · avg CMC {:.1}", stats.avg_cmc)
            } else {
                String::new()
            };
            println!(
                "{}{} {} {}{}",
                if i == 0 { label("Curve") } else { label("") },
                styles.bar(bucket.ratio, 12),
                styles.dim(&format!("{:>2}", bucket.label)),
                bucket.count,
                styles.dim(&suffix),
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
                bucket.count,
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
                bucket.count,
            );
        }
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
fn owned_map_for_deck(
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
fn deck_prices(
    conn: &Connection,
    deck: &super::Deck,
) -> std::collections::HashMap<String, Option<f64>> {
    let mut map = std::collections::HashMap::new();
    for name in deck.entries().map(|e| e.name.clone()).collect::<Vec<_>>() {
        if map.contains_key(&name) {
            continue;
        }
        let Ok(range) = crate::prints::price_range(conn, &name) else {
            map.insert(name, None);
            continue;
        };
        // A foil deck entry prefers the cheapest foil print when one exists.
        let is_foil = deck.entries().any(|e| e.name == name && e.foil);
        let price = if is_foil {
            range
                .cheapest_foil
                .as_ref()
                .and_then(|p| p.usd_foil)
                .or_else(|| range.cheapest.as_ref().and_then(|p| p.usd))
        } else {
            range.cheapest.as_ref().and_then(|p| p.usd)
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
fn deck_value(
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
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
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
    let slots = super::ownership::slot_map(&deck, &available, &assigned, |name| {
        cards_by_name
            .get(name)
            .is_some_and(super::stats::is_basic_land)
    });
    let prices = deck_prices(conn, &deck);
    let primer = primer_file(paths, name);

    if json {
        let sections: Vec<serde_json::Value> = deck
            .sections
            .iter()
            .map(|(section, entries)| {
                let lines: Vec<serde_json::Value> = entries
                    .iter()
                    .map(|entry| {
                        let (owned, elsewhere) =
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
                        serde_json::json!({
                            "quantity": entry.quantity,
                            "name": entry.name,
                            "set": entry.set_code,
                            "collector_number": entry.collector_number,
                            "foil": entry.foil,
                            "owned": owned,
                            "owned_elsewhere": elsewhere,
                            "covered_by": coverage,
                            "basic_land": basic,
                            "price_usd": prices.get(&entry.name).and_then(|p| *p),
                        })
                    })
                    .collect();
                serde_json::json!({ "section": section, "cards": lines })
            })
            .collect();
        let (owned_value, missing_cost) = deck_value(&deck, &cards_by_name, &prices, &available);
        let v = serde_json::json!({
            "name": name,
            "cards": deck.maindeck_total(),
            "sideboard_cards": deck.sideboard_total(),
            "primer": primer,
            "owned_value": round2(owned_value),
            "missing_cost": round2(missing_cost),
            "sections": sections,
        });
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
    let mut to_buy: Vec<(String, i64, f64)> = slots
        .iter()
        .filter(|(_, slot)| slot.missing > 0)
        .map(|(name, slot)| {
            (
                name.clone(),
                slot.missing,
                prices.get(name).copied().flatten().unwrap_or(0.0),
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
        for (card, qty, unit) in to_buy.iter().take(8) {
            println!(
                "  {:>2}  {}  {}",
                qty,
                styles.card_name(card),
                styles.dim(&format!("@${unit:.2}"))
            );
        }
        if to_buy.len() > 8 {
            println!(
                "  {}",
                styles.dim(&format!("… and {} more lines", to_buy.len() - 8))
            );
        }
        let total: f64 = to_buy.iter().map(|(_, q, p)| (*q as f64) * p).sum();
        println!(
            "  {}",
            styles.dim(&format!(
                "{} copies · est. ${total:.2} (full list: stm deck buylist {name})",
                to_buy.iter().map(|(_, q, _)| q).sum::<i64>()
            ))
        );
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
                styles.success(&format!("(own {owned}/{})", entry.quantity))
            } else if owned > 0 {
                styles.warning(&format!("(own {owned}/{})", entry.quantity))
            } else {
                format!("(own 0/{})", entry.quantity)
            };
            let elsewhere = if elsewhere > 0 && !basic {
                styles.dim(&format!(" (+{elsewhere} elsewhere)"))
            } else {
                String::new()
            };
            // Unit price next to the ownership note (skip basics: free).
            let price_note = match prices.get(&entry.name).copied().flatten() {
                Some(unit) if !basic => {
                    format!(" {}", styles.dim(&format!("@${unit:.2}")))
                }
                _ => String::new(),
            };
            println!(
                "  {:>2} {} {}  {owned_display}{elsewhere}{price_note}",
                entry.quantity,
                styles.card_name(&entry.name),
                styles.dim(rest),
            );
        }
    }
    Ok(crate::cli::codes::OK)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::ownership;

    #[test]
    fn valid_names_reject_paths() {
        assert!(valid_deck_name("Stationz"));
        assert!(valid_deck_name("Token Triumph"));
        assert!(valid_deck_name("Froggy!"));
        assert!(!valid_deck_name("a/b"));
        assert!(!valid_deck_name(".."));
        assert!(!valid_deck_name(""));
        assert!(!valid_deck_name("x\ny"));
    }

    /// Connection plus its backing tempdir (must outlive the connection).
    fn seeded_conn() -> (tempfile::TempDir, Connection) {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        conn.execute(
            "INSERT INTO cards (name, oracle_id) VALUES ('Lightning Bolt', 'oid')",
            [],
        )
        .unwrap();
        (tmp, conn)
    }

    fn silent_out() -> crate::output::Output {
        crate::output::Output::new(true, false, false)
    }

    fn add_collection_row(
        conn: &Connection,
        binder: &str,
        binder_type: &str,
        name: &str,
        set: &str,
        cn: &str,
        qty: i64,
    ) {
        conn.execute(
            "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity)
             VALUES (?1, ?2, ?3, 'normal', ?4, ?5, ?6)",
            rusqlite::params![name, set, cn, binder, binder_type, qty],
        )
        .unwrap();
    }

    #[test]
    fn any_printing_fills_a_deck_line() {
        let (_tmp, conn) = seeded_conn();
        // Owns a different set version than the deck line names.
        add_collection_row(
            &conn,
            "Collect",
            "binder",
            "Lightning Bolt",
            "m11",
            "148",
            4,
        );
        let owned = owned_map_for_deck(&conn, "TestDeck").unwrap();
        // Nothing assigned to the deck itself, but 4 sit in a binder.
        assert_eq!(owned.get("Lightning Bolt"), Some(&(0, 4)));

        add_collection_row(&conn, "TestDeck", "deck", "Lightning Bolt", "2xm", "124", 2);
        let owned = owned_map_for_deck(&conn, "TestDeck").unwrap();
        // Deck-assigned copies count regardless of print.
        assert_eq!(owned.get("Lightning Bolt"), Some(&(2, 4)));
    }

    #[test]
    fn delete_removes_files_keeps_ownership() {
        let (tmp, conn) = seeded_conn();
        let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
        std::fs::create_dir_all(paths.decks_dir()).unwrap();
        std::fs::write(paths.deck_file("Froggy"), "// DECK\n3 Lightning Bolt\n").unwrap();
        add_collection_row(&conn, "Froggy", "deck", "Lightning Bolt", "m11", "146", 2);
        let mut out = silent_out();

        let code = delete(&paths, &conn, &mut out, "Froggy").unwrap();
        assert_eq!(code, crate::cli::codes::OK);
        assert!(!paths.deck_file("Froggy").exists());
        // Ownership rows are untouched.
        let copies: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(quantity), 0) FROM collection WHERE binder_type = 'deck'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(copies, 2);
        // Deleting again fails with no-results.
        let code = delete(&paths, &conn, &mut out, "Froggy").unwrap();
        assert_eq!(code, crate::cli::codes::NO_RESULTS);
    }

    #[test]
    fn registered_deck_names_power_the_gap_display() {
        let (_tmp, conn) = seeded_conn();
        add_collection_row(
            &conn,
            "Ghost Deck",
            "deck",
            "Lightning Bolt",
            "m11",
            "146",
            2,
        );
        add_collection_row(
            &conn,
            "Collect",
            "binder",
            "Lightning Bolt",
            "m11",
            "148",
            1,
        );
        let names = registered_deck_names(&conn).unwrap();
        assert_eq!(names, vec!["Ghost Deck"]);
        // Binder copies of deck names count in the binder bucket (they can
        // fill deck slots).
        assert_eq!(owned_copies(&conn, "Ghost Deck").unwrap(), (2, 1));
    }

    /// A deck txt + primer pair inside a temp paths tree.
    fn deck_paths() -> (tempfile::TempDir, crate::paths::Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(tmp.path().join("root"));
        std::fs::create_dir_all(paths.decks_dir()).unwrap();
        (tmp, paths)
    }

    #[test]
    fn missing_cost_matches_buylist_total() {
        // The two code paths must agree: deck_value (show) uses the same
        // deck+binders coverage as buylist's missing math.
        let (_tmp, paths) = deck_paths();
        let (tmp2, conn) = seeded_conn();
        let _keep_alive = tmp2;
        // 3 needed, 1 deck-assigned + 1 binder → 1 missing.
        add_collection_row(&conn, "TestDeck", "deck", "Lightning Bolt", "m11", "148", 1);
        add_collection_row(
            &conn,
            "Collect",
            "binder",
            "Lightning Bolt",
            "m11",
            "148",
            1,
        );
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
             VALUES ('a', 'Lightning Bolt', 'm11', '148', 'en', 'common',
                '[\"nonfoil\",\"foil\"]', '2020-01-01', 0.5, NULL, 't')",
            [],
        )
        .unwrap();
        let deck = crate::deck::Deck::parse("// DECK\n3 Lightning Bolt\n").unwrap();
        std::fs::write(paths.deck_file("TestDeck"), deck.to_text()).unwrap();

        let cards_by_name = super::super::stats::lookup_names(&conn, &deck);
        let prices = deck_prices(&conn, &deck);
        let available = ownership::available_map(&conn, "TestDeck").unwrap();
        let (_, missing_cost) = deck_value(&deck, &cards_by_name, &prices, &available);

        // Buylist path over the same collection state.
        let rows =
            super::super::buylist::missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
        let buylist_total: f64 = rows
            .iter()
            .map(|r| r.price_usd.unwrap_or(0.0) * r.quantity as f64)
            .sum();
        assert_eq!(
            round2(missing_cost),
            round2(buylist_total),
            "show missing_cost must equal buylist total"
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].quantity, 1);
    }

    #[test]
    fn slot_map_classifies_coverage() {
        let deck = super::super::Deck::parse("// DECK\n3 Bolt\n2 Shock\n").unwrap();
        let assigned = [("Bolt".to_string(), 3i64)].into_iter().collect();
        let available = [("Bolt".to_string(), 4i64), ("Shock".to_string(), 1i64)]
            .into_iter()
            .collect();
        let slots = ownership::slot_map(&deck, &available, &assigned, |_| false);
        let bolt = &slots["Bolt"];
        assert_eq!(bolt.coverage, ownership::Coverage::Deck);
        assert_eq!(bolt.in_deck, 3);
        assert_eq!(bolt.in_binder, 1);
        assert_eq!(bolt.missing, 0);
        let shock = &slots["Shock"];
        assert_eq!(shock.coverage, ownership::Coverage::Missing);
        assert_eq!(shock.missing, 1);
        // Binder-only coverage: 3 needed, 0 deck, 3 binder.
        let assigned2: std::collections::HashMap<String, i64> = Default::default();
        let available2 = [("Bolt".to_string(), 3i64)].into_iter().collect();
        let slots = ownership::slot_map(
            &super::super::Deck::parse("// DECK\n3 Bolt\n").unwrap(),
            &available2,
            &assigned2,
            |_| false,
        );
        assert_eq!(slots["Bolt"].coverage, ownership::Coverage::Binder);
    }
}
