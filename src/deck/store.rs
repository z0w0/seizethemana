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
        // Completion coloring: green when fully owned, yellow when partial,
        // red when nothing is owned.
        let owned_display = if d.owned >= d.cards && d.cards > 0 {
            styles.glyph(
                &format!("own {}/{}", d.owned, d.cards),
                crate::output::GlyphKind::Good,
            )
        } else if d.owned > 0 {
            styles.glyph(
                &format!("own {}/{}", d.owned, d.cards),
                crate::output::GlyphKind::Warn,
            )
        } else {
            styles.glyph(
                &format!("own {}/{}", d.owned, d.cards),
                crate::output::GlyphKind::Bad,
            )
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
            "{}{}{}{}",
            label("Value"),
            styles.money(owned_value),
            styles.dim(" owned · missing "),
            styles.money(missing_cost),
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
                styles.thousands(bucket.count),
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
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in deck.entries().map(|e| e.name.clone()) {
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }
    // One batched query per finish kind instead of four statements per
    // card name.
    let Ok(ranges) = crate::prints::price_ranges(conn, &names) else {
        return names.into_iter().map(|n| (n, None)).collect();
    };
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
fn universe_census(
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
                            "price_usd": prices.get(&entry.name).and_then(|p| *p),
                        })
                    })
                    .collect();
                serde_json::json!({ "section": section, "cards": lines })
            })
            .collect();
        let (owned_value, missing_cost) = deck_value(&deck, &cards_by_name, &prices, &available);
        let mut v = serde_json::json!({
            "name": name,
            "cards": deck.maindeck_total(),
            "sideboard_cards": deck.sideboard_total(),
            "primer": primer,
            "owned_value": round2(owned_value),
            "missing_cost": round2(missing_cost),
            "sections": sections,
        });
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
                styles.dim(&format!("@${unit:.2}")),
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

#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod store_tests;
