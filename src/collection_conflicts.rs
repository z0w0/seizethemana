// Cross-deck conflict report: cards wanted by more deck slots than the
// collection covers, plus the decklist demand helpers behind it. Split
// out of `collection.rs` to keep each file under the size limit.

use anyhow::Context;
use rusqlite::Connection;
/// One card wanted by more deck slots than you own copies.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConflictRow {
    pub name: String,
    /// Copies owned anywhere (binders + deck assignments).
    pub owned: i64,
    /// Total slots the card fills across all decklists.
    pub demanded: i64,
    /// Deck names demanding copies, with per-deck quantities.
    pub decks: Vec<ConflictDeck>,
    /// Copies still needed to cover every slot.
    pub gap: i64,
    /// Cheapest printing × gap; None when unpriced.
    pub buy_cost_usd: Option<f64>,
}

/// One deck's slot demand for a conflicted card.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConflictDeck {
    pub name: String,
    pub quantity: i64,
}

/// Cards wanted by more deck slots than the collection covers.
///
/// A card conflicts when the sum of its non-basic slot demand across every
/// decklist exceeds the copies owned anywhere. Basics are excluded
/// (unlimited). `demanded_by` reads decklist files; a registered deck
/// without a list contributes ownership but no demand.
pub fn conflicts(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    // Slot demand per card name across every decklist on disk.
    let demand = deck_demand(paths)?;

    let owned = crate::collection::owned_counts_all(conn)?;
    let mut rows: Vec<ConflictRow> = Vec::new();
    let mut priced_names: Vec<String> = Vec::new();
    for (name, decks) in &demand {
        // Basic lands are unlimited; never a conflict. The same exemption
        // `deck update`'s singleton guard uses.
        if crate::collection::is_basic_name(name) {
            continue;
        }
        let demanded: i64 = decks.iter().map(|(_, q)| q).sum();
        let owned_copies = owned.get(name).copied().unwrap_or(0);
        let gap = demanded - owned_copies;
        if gap <= 0 {
            continue;
        }
        rows.push(ConflictRow {
            name: name.clone(),
            owned: owned_copies,
            demanded,
            decks: decks
                .iter()
                .map(|(deck, qty)| ConflictDeck {
                    name: deck.clone(),
                    quantity: *qty,
                })
                .collect(),
            gap,
            buy_cost_usd: None,
        });
        priced_names.push(name.clone());
    }
    // Cheapest-printing cost for each gap, one batched query.
    let ranges = crate::prints::price_ranges(conn, &priced_names)?;
    for row in &mut rows {
        row.buy_cost_usd = ranges
            .get(&row.name)
            .and_then(crate::prints::PrintRange::price)
            .map(|p| row.gap as f64 * p);
    }
    rows.sort_by(|a, b| {
        b.buy_cost_usd
            .unwrap_or(0.0)
            .total_cmp(&a.buy_cost_usd.unwrap_or(0.0))
            .then_with(|| a.name.cmp(&b.name))
    });

    // Deck names registered in the collection but missing a decklist:
    // they own cards we cannot see into, so their demand is unverified.
    let mut unverified: Vec<String> = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT binder FROM collection
         WHERE binder_type = 'deck' ORDER BY binder COLLATE NOCASE",
    )?;
    let registered = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for deck in registered {
        if !paths.deck_file(&deck).exists() {
            unverified.push(deck);
        }
    }
    finish_conflicts(rows, unverified, out, json)
}

/// Shared exit path for the conflicts report (kept small for the early
/// return when the decks directory is missing).
fn finish_conflicts(
    rows: Vec<ConflictRow>,
    unverified: Vec<String>,
    out: &mut crate::output::Output,
    json: bool,
) -> anyhow::Result<i32> {
    if json {
        let payload = serde_json::json!({
            "rows": rows,
            "unverified_decks": unverified,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(crate::cli::codes::OK);
    }
    let styles = out.styles();
    if rows.is_empty() {
        println!(
            "{}",
            styles.success("No shared cards are short. Every deck's copies are covered.")
        );
        if !unverified.is_empty() {
            out.print_note(&format!(
                "these decks have no decklist imported, so their cards were not checked: {}",
                unverified.join(", ")
            ));
        }
        return Ok(crate::cli::codes::OK);
    }
    println!("{}", styles.header("Shared cards running short"));
    for row in &rows {
        let decks = row
            .decks
            .iter()
            .map(|d| format!("{} ({})", d.name, d.quantity))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {}  {}",
            styles.card_name(&row.name),
            styles.dim(&format!(
                "own {}, slots {} → {} short",
                row.owned, row.demanded, row.gap
            ))
        );
        println!("    {decks}");
        if let Some(cost) = row.buy_cost_usd {
            println!(
                "    buying {} cop{} costs about {}",
                row.gap,
                if row.gap == 1 { "y" } else { "ies" },
                styles.money(cost)
            );
        }
    }
    if !unverified.is_empty() {
        out.print_note(&format!(
            "these decks have no decklist imported, so their cards were not checked: {}",
            unverified.join(", ")
        ));
    }
    Ok(crate::cli::codes::OK)
}

/// Collection rows for one card within the queried scope:
/// (location, type, quantity), binders first. `binders`/`decks` mirror the
/// query's filters so a `--binder` search does not advertise deck rows and
/// a bare search does not advertise deck assignments at all.
pub(crate) fn locations_for(
    conn: &Connection,
    name: &str,
    binders: &[String],
    decks: &[String],
) -> anyhow::Result<Vec<(String, String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT binder, binder_type, quantity FROM collection
         WHERE name = ?1
           AND (binder_type = 'binder'
                AND (?2 = 0 OR binder IN (SELECT value FROM json_each(?3))))
            OR (name = ?1 AND binder_type = 'deck'
                AND binder IN (SELECT value FROM json_each(?4)))",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                name,
                binders.len() as i64,
                serde_json::to_string(binders)?,
                serde_json::to_string(decks)?,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()
        .context("reading locations")?;
    Ok(rows)
}

/// Every location of one card across all binders and deck assignments:
/// (location, binder_type, quantity, foil), binders first. Read-only
/// lookup for `stm card where` and the conflict report.
///
/// # Errors
/// Propagates SQLite failures.
pub fn locations_all(
    conn: &Connection,
    name: &str,
) -> anyhow::Result<Vec<(String, String, i64, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT binder, binder_type, SUM(quantity), foil FROM collection
         WHERE name = ?1
           AND (binder_type = 'binder' OR binder_type = 'deck')
         GROUP BY binder, binder_type, foil
         ORDER BY CASE binder_type WHEN 'binder' THEN 0 ELSE 1 END, binder",
    )?;
    let rows = stmt
        .query_map([name], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                // Etched copies read as foil: they are premium finishes.
                row.get::<_, String>(3)? == "foil" || row.get::<_, String>(3)? == "etched",
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("reading card locations")?;
    Ok(rows)
}

/// Deck names whose decklists include a card (read from the deck files).
///
/// Ownership rows say where copies sit; this answers "which decks want
/// the card". A deck without a decklist file contributes nothing.
pub fn demanded_by(paths: &crate::paths::Paths, name: &str) -> Vec<(String, i64)> {
    deck_demand(paths)
        .map(|demand| demand.get(name).cloned().unwrap_or_default())
        .unwrap_or_default()
}

/// Slot demand per card name across every decklist on disk:
/// card name → (deck name, quantity) rows, sorted by deck name.
/// A decklist that exists but cannot be read or parsed reports a
/// `(<unparseable …>, deck_name)` row so the demand report shows the gap
/// instead of silently undercounting. Read-only.
pub(crate) fn deck_demand(
    paths: &crate::paths::Paths,
) -> anyhow::Result<std::collections::BTreeMap<String, Vec<(String, i64)>>> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(paths.decks_dir()) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let deck_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                out.entry(format!("<unreadable: {err}>"))
                    .or_default()
                    .push((deck_name, 0));
                continue;
            }
        };
        let deck = match crate::deck::grammar::Deck::parse(&text) {
            Ok(deck) => deck,
            Err(err) => {
                out.entry(format!("<unparseable: {err}>"))
                    .or_default()
                    .push((deck_name, 0));
                continue;
            }
        };
        for card in deck.entries() {
            out.entry(card.name.clone())
                .or_default()
                .push((deck_name.clone(), card.quantity));
        }
    }
    Ok(out)
}
