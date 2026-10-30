//! Cross-deck conflict report: cards wanted by more deck slots than the
//! collection covers, plus the decklist demand helpers behind it. Split
//! out of `collection.rs` to keep each file under the size limit.

use anyhow::Context;
use rusqlite::Connection;

/// One card wanted by more deck slots than you own copies.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConflictRow {
    /// Oracle card name.
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
    /// Deck name.
    pub name: String,
    /// Slots the deck needs of the card.
    pub quantity: i64,
}

/// Complete typed JSON response for the shared-card conflict report.
#[derive(Debug, serde::Serialize)]
struct ConflictReport {
    /// Cards with uncovered deck demand.
    rows: Vec<ConflictRow>,
    /// Decks with a missing, unreadable, or invalid decklist.
    unverified_decks: Vec<String>,
}

/// Readable deck demand and deck names that could not be checked.
#[derive(Default)]
struct DeckDemand {
    /// Card name to per-deck quantities from valid decklists.
    cards: std::collections::BTreeMap<String, Vec<(String, i64)>>,
    /// Deck names whose files could not be read or parsed.
    unverified_decks: Vec<String>,
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
    let report = conflict_report(paths, conn)?;
    finish_conflicts(report.rows, report.unverified_decks, out, json)
}

/// Build a conflict report without treating failed deck reads as checked demand.
fn conflict_report(
    paths: &crate::paths::Paths,
    conn: &Connection,
) -> anyhow::Result<ConflictReport> {
    // Slot demand per card name across every decklist on disk.
    let demand = read_deck_demand(paths)?;

    let owned = crate::collection::owned_counts_all(conn)?;
    let mut rows: Vec<ConflictRow> = Vec::new();
    let mut priced_names: Vec<String> = Vec::new();
    for (name, decks) in &demand.cards {
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

    // Registered decks without a file also have unverified demand.
    let mut unverified = demand.unverified_decks;
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
    unverified.sort_by(|a, b| {
        a.to_ascii_lowercase()
            .cmp(&b.to_ascii_lowercase())
            .then_with(|| a.cmp(b))
    });
    unverified.dedup();
    Ok(ConflictReport {
        rows,
        unverified_decks: unverified,
    })
}

/// Render the conflict report as human-readable text or JSON.
fn finish_conflicts(
    rows: Vec<ConflictRow>,
    unverified: Vec<String>,
    out: &mut crate::output::Output,
    json: bool,
) -> anyhow::Result<i32> {
    if json {
        let payload = ConflictReport {
            rows,
            unverified_decks: unverified,
        };
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(crate::cli::codes::OK);
    }
    let styles = out.styles();
    if rows.is_empty() {
        println!(
            "{}",
            styles.success("No shared-card shortage found in the checked decklists.")
        );
        if !unverified.is_empty() {
            out.print_note(&format!(
                "these decks could not be checked: {}. Use 'stm deck show <name>' to check each decklist",
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
        out.print_wrapped(&format!(
            "  {}  {}",
            styles.card_name(&row.name),
            styles.dim(&format!(
                "owned {} copies, needed {} → {} short",
                row.owned, row.demanded, row.gap
            ))
        ));
        out.print_wrapped(&format!("    {decks}"));
        if let Some(cost) = row.buy_cost_usd {
            out.print_wrapped(&format!(
                "    estimated price for {} cop{}: {}",
                row.gap,
                if row.gap == 1 { "y" } else { "ies" },
                styles.money(cost)
            ));
        } else {
            out.print_wrapped("    estimated price: unpriced");
        }
    }
    out.print_note("price basis: cheapest stored released English nonfoil printing, or foil if nonfoil is unpriced. Actual purchase costs may differ");
    if !unverified.is_empty() {
        out.print_note(&format!(
            "these decks could not be checked: {}. Use 'stm deck show <name>' to check each decklist",
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

/// Deck names whose decklists include a card (read from the deck files).
///
/// Ownership rows say where copies sit; this answers "which decks want
/// the card". Missing, unreadable, or invalid decklists contribute no demand.
/// Directory read errors propagate instead of hiding unchecked demand.
///
/// # Errors
/// Propagates directory read failures behind `deck_demand`.
pub fn demanded_by(paths: &crate::paths::Paths, name: &str) -> anyhow::Result<Vec<(String, i64)>> {
    Ok(deck_demand(paths)?.get(name).cloned().unwrap_or_default())
}

/// Slot demand per card name from readable, valid decklists on disk.
/// Each value holds the deck names and quantities that need the card.
pub(crate) fn deck_demand(
    paths: &crate::paths::Paths,
) -> anyhow::Result<std::collections::BTreeMap<String, Vec<(String, i64)>>> {
    Ok(read_deck_demand(paths)?.cards)
}

/// Read deck demand and keep failed deck names separate from card quantities.
fn read_deck_demand(paths: &crate::paths::Paths) -> anyhow::Result<DeckDemand> {
    let mut out = DeckDemand::default();
    // A missing decks directory means "no decks yet" (empty demand), but
    // any other read failure must surface: swallowing it would undercount
    // demand and report a false "nothing is short".
    let entries = match std::fs::read_dir(paths.decks_dir()) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(err) => {
            return Err(err).context("reading decks directory for demand");
        }
    };
    for entry in entries {
        let entry = entry.context("reading a deck directory entry for demand")?;
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
            Err(_) => {
                out.unverified_decks.push(deck_name);
                continue;
            }
        };
        let deck = match crate::deck::grammar::Deck::parse(&text) {
            Ok(deck) => deck,
            Err(_) => {
                out.unverified_decks.push(deck_name);
                continue;
            }
        };
        // Playable sections only: maybeboard entries are loose
        // candidates, not slots that demand owned copies. Lines of the
        // same card in one deck sum into one demand entry.
        let mut demand: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for (section, entries) in &deck.sections {
            if crate::deck::grammar::is_maybeboard_section(section) {
                continue;
            }
            for card in entries {
                *demand.entry(card.name.clone()).or_default() += card.quantity;
            }
        }
        for (name, qty) in demand {
            out.cards
                .entry(name)
                .or_default()
                .push((deck_name.clone(), qty));
        }
    }
    Ok(out)
}

/// Regression tests for failed deck reads in conflict reports.
#[cfg(test)]
#[path = "tests/collection_conflicts_tests.rs"]
mod collection_conflicts_tests;
