//! Collection: ManaBox CSV import and owned-only semantic search.
//! Stats live in `collection_stats`; the cross-deck conflict report in
//! `collection_conflicts`. CSV rows key by (binder name, binder type in
//! {binder, deck}, name, set, cn, foil); quantity aggregates on that key.
//! Deck rows record ownership assignments, not deck files. Import decklists
//! separately with `stm deck import`. Ignore `list` (wishlist) rows.

use anyhow::Context;
use rusqlite::Connection;

/// One parsed ManaBox row.
#[derive(Debug, Clone, PartialEq)]
pub struct CsvRow {
    /// Binder or deck name (`Binder Name` column).
    pub binder: String,
    /// Binder category (binder or deck).
    pub binder_type: BinderType,
    /// Oracle card name.
    pub name: String,
    /// Set code.
    pub set_code: String,
    /// Collector number within the set.
    pub collector_number: String,
    /// `normal`, `foil`, or `etched`.
    pub foil: String,
    /// Copies at this location.
    pub quantity: i64,
    /// Row total for the row (ManaBox's per-copy price × quantity).
    pub purchase_price: f64,
    /// Scryfall print ID, when the CSV carries one.
    pub scryfall_id: String,
}

/// ManaBox binder type; wishlist (`list`) rows are skipped on import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinderType {
    /// A physical binder.
    Binder,
    /// A deck assignment.
    Deck,
}

/// One binder location in a collection search result.
#[derive(Debug, Clone, serde::Serialize)]
struct CollectionLocationReport {
    /// Binder or deck name.
    binder: String,
    /// Binder category.
    binder_type: String,
    /// Copies at this location.
    quantity: i64,
}

/// Full card detail with collection-search score and locations.
#[derive(Debug, Clone, serde::Serialize)]
struct CollectionSearchReport {
    /// Card detail shared with `stm card`.
    #[serde(flatten)]
    card: crate::card::CardReport,
    /// Semantic search score, or zero for a filter-only listing.
    score: f64,
    /// Collection locations containing the card.
    locations: Vec<CollectionLocationReport>,
}

impl BinderType {
    /// The stored string form (`binder` or `deck`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Binder => "binder",
            Self::Deck => "deck",
        }
    }
}

/// Parse a ManaBox collection CSV export.
///
/// Returns binder/deck rows in file order plus the number of skipped
/// wishlist (`list`) rows. Columns are matched by header name so column
/// order is irrelevant; missing purchase prices read as 0.
///
/// # Errors
/// Fails on unreadable files, missing required headers, or unparseable
/// quantity values.
pub fn parse_csv(
    path: &std::path::Path,
    out: &mut crate::output::Output,
) -> anyhow::Result<(Vec<CsvRow>, usize)> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("cannot open collection CSV {}", path.display()))?;
    let mut reader = csv::Reader::from_reader(file);
    let headers = reader.headers().context("reading CSV header row")?.clone();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let c_binder = col("Binder Name").context("CSV is missing the 'Binder Name' column")?;
    let c_type = col("Binder Type").context("CSV is missing the 'Binder Type' column")?;
    let c_name = col("Name").context("CSV is missing the 'Name' column")?;
    let c_set = col("Set code").context("CSV is missing the 'Set code' column")?;
    let c_cn = col("Collector number").context("CSV is missing the 'Collector number' column")?;
    let c_foil = col("Foil").context("CSV is missing the 'Foil' column")?;
    let c_qty = col("Quantity").context("CSV is missing the 'Quantity' column")?;
    let c_price = col("Purchase price");
    let c_scry = col("Scryfall ID");

    let mut rows = Vec::new();
    let mut skipped_list_rows = 0usize;
    let mut unknown_binder_rows = 0usize;
    let mut non_positive_qty_rows = 0usize;
    let mut unparseable_qty_rows = 0usize;
    let mut purchase_warns = 0usize;
    for record in reader.records() {
        let record = record.with_context(|| format!("parsing {}", path.display()))?;
        let binder_type_text = record
            .get(c_type)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let kind = match binder_type_text.as_str() {
            "binder" => BinderType::Binder,
            "deck" => BinderType::Deck,
            "list" => {
                skipped_list_rows += 1;
                continue;
            }
            // Unknown binder types skip like wishlist rows instead of
            // failing the whole import.
            _ => {
                unknown_binder_rows += 1;
                continue;
            }
        };
        let foil = match record.get(c_foil).unwrap_or_default().trim() {
            "foil" => "foil",
            "etched" => "etched",
            _ => "normal",
        };
        // An unparseable or blank quantity skips like any other bad row
        // instead of failing the whole import: one bad line must not
        // poison the import.
        let qty_text = record.get(c_qty).unwrap_or("1").trim();
        let quantity: i64 = match qty_text.parse() {
            Ok(q) => q,
            Err(_) => {
                unparseable_qty_rows += 1;
                continue;
            }
        };
        // A non-positive quantity skips like any other bad row instead of
        // failing the whole import: ManaBox exports carry wishlist rows
        // with quantity 0, and one bad line must not poison the import.
        if quantity <= 0 {
            non_positive_qty_rows += 1;
            continue;
        }
        let purchase_price = c_price
            .and_then(|i| record.get(i))
            .and_then(|text| {
                let text = text.trim();
                if text.is_empty() {
                    Some(0.0)
                } else {
                    let parsed = parse_locale_price(text);
                    // Locale-formatted values ("1.234,50") do not parse;
                    // warn so silent zero-priced rows are visible.
                    if parsed.is_none() {
                        purchase_warns += 1;
                    }
                    parsed
                }
            })
            .unwrap_or(0.0)
            * quantity as f64;
        rows.push(CsvRow {
            binder: record.get(c_binder).unwrap_or_default().trim().to_string(),
            binder_type: kind,
            name: record.get(c_name).unwrap_or_default().trim().to_string(),
            set_code: record.get(c_set).unwrap_or_default().trim().to_string(),
            collector_number: record.get(c_cn).unwrap_or_default().trim().to_string(),
            foil: foil.to_string(),
            quantity,
            purchase_price,
            scryfall_id: c_scry
                .and_then(|i| record.get(i))
                .unwrap_or_default()
                .to_string(),
        });
    }
    if unknown_binder_rows > 0 {
        out.warning(&format!(
            "{unknown_binder_rows} CSV rows with an unknown Binder Type were skipped (expected 'binder', 'deck', or 'list')"
        ));
    }
    if non_positive_qty_rows > 0 {
        out.warning(&format!(
            "skipped {non_positive_qty_rows} CSV rows with quantity 0 or less; use quantities greater than 0"
        ));
    }
    if unparseable_qty_rows > 0 {
        out.warning(&format!(
            "skipped {unparseable_qty_rows} CSV rows with a blank or unreadable quantity; use a whole number greater than 0"
        ));
    }
    if purchase_warns > 0 {
        out.warning(&format!(
            "{purchase_warns} purchase prices could not be read and were stored as 0; use a number such as '12.50' without thousands separators"
        ));
    }
    Ok((rows, skipped_list_rows))
}

/// Aggregate CSV rows onto the collection key (duplicate rows add up).
/// `purchase_price` is a per-row total, so merges add it linearly.
pub fn aggregate(rows: &[CsvRow]) -> Vec<CsvRow> {
    let mut map: std::collections::BTreeMap<
        (String, String, String, String, String, &'static str),
        CsvRow,
    > = std::collections::BTreeMap::new();
    for row in rows {
        let key = (
            row.binder.clone(),
            row.binder_type.as_str().to_string(),
            row.name.clone(),
            row.set_code.clone(),
            row.collector_number.clone(),
            match row.foil.as_str() {
                "foil" => "foil",
                "etched" => "etched",
                _ => "normal",
            },
        );
        map.entry(key)
            .and_modify(|existing| {
                existing.quantity += row.quantity;
                existing.purchase_price += row.purchase_price;
            })
            .or_insert_with(|| row.clone());
    }
    map.into_values().collect()
}

/// Import result summary.
#[derive(Debug, Default, PartialEq)]
pub struct ImportSummary {
    /// Distinct binder locations written.
    pub binders: usize,
    /// Distinct deck locations written.
    pub decks: usize,
    /// Rows written after aggregation.
    pub rows: usize,
    /// Total copies imported.
    pub cards: i64,
    /// Wishlist rows skipped.
    pub skipped_list_rows: usize,
}

/// Import a ManaBox collection CSV into the collection (replace by default,
/// `--add` merges on top).
///
/// Deck rows become ownership assignments (`binder_type = 'deck'`) so
/// `deck show`/`deck buylist` can count what you own; decklists are **not**
/// written here — the CSV carries no list structure. Import each deck's
/// ManaBox txt export separately with `stm deck import`. `list` (wishlist)
/// rows are counted as skipped. Card names are matched against `cards`
/// case-insensitively; unmatched rows are reported and skipped rather than
/// aborting the import.
///
/// # Errors
/// Propagates CSV/SQLite failures.
pub fn import(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    file: &std::path::Path,
    add: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet; collection import needs card data to match names");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let started = std::time::Instant::now();
    out.status("Importing", &format!("collection from {}", file.display()));
    let (parsed, skipped_list_rows) = parse_csv(file, out)?;
    let aggregated = aggregate(&parsed);
    out.status(
        "Read",
        &format!(
            "{} rows ({} after aggregating duplicates)",
            parsed.len(),
            aggregated.len(),
        ),
    );

    // Resolve card names against the oracle (case-insensitive). A real
    // card always imports — even when its name also sits in token_names
    // (staples like Vampiric Tutor have digital-only prints recorded as
    // token-ish names). Rows left unresolved that name a known
    // token/art/emblem skip silently; ambiguous names warn as ambiguous
    // (the card exists — the user can spell it out); genuinely unknown
    // names warn as unknown.
    let mut unknown: Vec<String> = Vec::new();
    let mut ambiguous: Vec<String> = Vec::new();
    let mut skipped_tokens = 0usize;
    let mut resolved: Vec<CsvRow> = Vec::new();
    for row in &aggregated {
        // Exact real-card match wins over a token_names entry (digital-only
        // prints of staples once landed there). But a token name that only
        // prefix-matches real cards ("Food" vs "Food Chain") skips before
        // resolution, or it would resolve as the card it prefix-matches.
        let matched = if let Some(card) = crate::db::get_card(conn, &row.name)? {
            crate::db::NameMatch::Found(Box::new(card))
        } else {
            if crate::db::is_token_name(conn, &row.name)?
                && !crate::db::card_exists(conn, &row.name)?
            {
                skipped_tokens += 1;
                continue;
            }
            crate::db::resolve_name(conn, &row.name)?
        };
        match matched {
            crate::db::NameMatch::Found(card) => {
                let mut row = row.clone();
                row.name = card.name;
                resolved.push(row);
            }
            crate::db::NameMatch::Ambiguous { total, .. } => {
                ambiguous.push(format!("{} (matches {total} card names)", row.name));
            }
            _ => unknown.push(row.name.clone()),
        }
    }
    for name in &ambiguous {
        out.warning(&format!(
            "skipped ambiguous entry {name}; use the full name in the CSV ('stm card <name>' lists matches)"
        ));
    }
    if !unknown.is_empty() {
        out.warning(&format!(
            "skipped {} CSV entries with no matching card name (examples: {})",
            unknown.len(),
            unknown
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if skipped_tokens > 0 {
        out.status(
            "Skipped",
            &format!("{skipped_tokens} token, emblem, or art entries"),
        );
    }

    if !add {
        out.warning("this import replaces all collection records; saved decklists stay unchanged. Back up the data directory first, or use --add to merge instead");
    }
    conn.execute("BEGIN", []).context("begin import")?;
    let result = (|| -> anyhow::Result<ImportSummary> {
        if !add {
            conn.execute("DELETE FROM collection", [])?;
        }
        let mut binders: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut decks: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut cards = 0i64;
        let mut stmt = conn.prepare(
            "INSERT INTO collection
                (name, set_code, collector_number, foil, binder, binder_type,
                 quantity, purchase_price)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (name, set_code, collector_number, foil, binder, binder_type)
             DO UPDATE SET quantity = quantity + excluded.quantity,
                           purchase_price = purchase_price + excluded.purchase_price",
        )?;
        for row in &resolved {
            stmt.execute(rusqlite::params![
                row.name,
                row.set_code,
                row.collector_number,
                row.foil,
                row.binder,
                row.binder_type.as_str(),
                row.quantity,
                row.purchase_price,
            ])?;
            cards += row.quantity;
            match row.binder_type {
                BinderType::Binder => {
                    binders.insert(row.binder.clone());
                }
                BinderType::Deck => {
                    decks.insert(row.binder.clone());
                }
            }
        }
        Ok(ImportSummary {
            binders: binders.len(),
            decks: decks.len(),
            rows: resolved.len(),
            cards,
            skipped_list_rows,
        })
    })();
    let summary = match result {
        Ok(summary) => {
            conn.execute("COMMIT", []).context("committing import")?;
            summary
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
    };

    // Deck assignments are ownership only. Point at the split so the next
    // step (importing the decklist) is obvious; skip the second note when
    // every deck already has a decklist (no nagging).
    if summary.decks > 0 {
        let deck_names = deck_names_from_resolved(&resolved);
        let copies: i64 = resolved
            .iter()
            .filter(|r| r.binder_type == BinderType::Deck)
            .map(|r| r.quantity)
            .sum();
        out.print_note(&format!(
            "deck ownership tracked for {} ({} copies)",
            deck_names.join(", "),
            copies
        ));
        missing_list_note(paths, &deck_names, out)?;
    }

    out.finish(
        "Imported",
        &format!(
            "{} entries ({} copies) across {} binders and {} deck locations{}",
            summary.rows,
            summary.cards,
            summary.binders,
            summary.decks,
            if add { " (merged)" } else { "" },
        ),
        started.elapsed(),
    );
    Ok(crate::cli::codes::OK)
}

/// Deck names seen in the parsed CSV (sorted, deduped).
fn deck_names_from_resolved(resolved: &[CsvRow]) -> Vec<String> {
    let mut decks: Vec<String> = resolved
        .iter()
        .filter(|r| r.binder_type == BinderType::Deck)
        .map(|r| r.binder.clone())
        .collect();
    decks.sort_unstable();
    decks.dedup();
    decks
}

/// Print the import-decklists note; true when every deck already has a
/// decklist file (nothing to say).
fn missing_list_note(
    paths: &crate::paths::Paths,
    decks: &[String],
    out: &mut crate::output::Output,
) -> anyhow::Result<bool> {
    let missing: Vec<&String> = decks
        .iter()
        .filter(|d| !paths.deck_file(d).exists())
        .collect();
    if missing.is_empty() {
        return Ok(true);
    }
    if missing.len() == decks.len() {
        out.print_note(&format!(
            "decklists are not in the collection CSV; import each separately: \
             stm deck import <name> <manabox-deck.txt> (missing: {})",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else {
        out.print_note(&format!(
            "decklists are not in the collection CSV; import each separately: \
             stm deck import <name> <manabox-deck.txt> (no decklist yet: {})",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(false)
}

/// One owned-search hit: card, score, and every location that owns it.
#[derive(Debug, Clone)]
pub struct OwnedHit {
    /// Matched card.
    pub card: crate::db::CardRow,
    /// Search score.
    pub score: f32,
    /// Copies owned in the queried scope.
    pub owned: i64,
    /// Locations holding the card: `(binder, binder_type, quantity)`.
    pub locations: Vec<(String, String, i64)>,
}

/// Search or list owned cards, with copies and locations in the requested scope.
///
/// Text queries share the ranked pipeline with `stm query`. Without text,
/// listings use only the database. Both modes add scoped copies and locations.
/// Scope, search, and result detail use one SQL read view.
///
/// # Errors
/// Propagates search or SQLite failures.
#[allow(clippy::too_many_arguments)]
pub fn run_query(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    text: &str,
    cli_filters: &crate::cli::CardFilters,
    binders: &[String],
    decks: &[String],
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    let filters = crate::search::CardFilters::from_cli(cli_filters)?;
    let snapshot = if text.trim().is_empty() {
        None
    } else {
        Some(crate::query::RankedSnapshot::open(paths)?)
    };
    let conn = snapshot
        .as_ref()
        .map_or(&*conn, crate::query::RankedSnapshot::connection);
    let _listing_read = if snapshot.is_none() {
        Some(conn.unchecked_transaction()?)
    } else {
        None
    };
    let owned = owned_names(conn, binders, decks)?;
    if owned.is_empty() {
        if !text.trim().is_empty() && !paths.is_setup() {
            out.error("card index not built yet");
            out.hint("run 'stm setup' first");
            return Ok(crate::cli::codes::ERROR);
        }
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&Vec::<CollectionSearchReport>::new())?
            );
        } else if binders.is_empty() && decks.is_empty() {
            out.error("no binder cards found");
            out.hint("import binder copies with 'stm collection import <file>', or add --deck <name> to include deck assignments");
        } else {
            out.error("no cards in the requested locations");
            out.hint("list your binders and deck assignments: stm collection (see Locations)");
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let owned: std::collections::HashSet<String> = owned.into_iter().collect();
    #[cfg(test)]
    crate::query::after_scope_for_test();
    let search = match &snapshot {
        Some(snapshot) => {
            let task = "owned cards (preparing text search)";
            out.status("Searching", task);
            out.progress_bar("Searching", task, 0);
            let result = snapshot.search(text, &filters, limit as usize, Some(&owned));
            out.clear_progress();
            result
        }
        None => crate::query::run_search(
            paths,
            conn,
            out,
            text,
            &filters,
            limit as usize,
            Some(&owned),
        ),
    };
    let hits = match search {
        Ok(hits) => hits,
        Err(error) if !paths.is_setup() => {
            out.error(&format!("{error:#}"));
            out.hint("run 'stm setup' first");
            return Ok(crate::cli::codes::ERROR);
        }
        Err(error) => return Err(error),
    };
    let mut out_hits: Vec<OwnedHit> = Vec::new();
    for hit in hits {
        let locations =
            crate::collection_conflicts::locations_for(conn, &hit.card.name, binders, decks)?;
        let owned_qty: i64 = locations.iter().map(|(_, _, q)| q).sum();
        if owned_qty == 0 {
            continue;
        }
        out_hits.push(OwnedHit {
            card: hit.card,
            score: hit.score,
            owned: owned_qty,
            locations,
        });
    }
    // Per-hit owned counts as a name map for `card_json` (the JSON path
    // reads counts from the map, not per-hit fields).
    let owned_counts: std::collections::HashMap<String, i64> = out_hits
        .iter()
        .map(|h| (h.card.name.clone(), h.owned))
        .collect();
    if out_hits.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&Vec::<CollectionSearchReport>::new())?
            );
        } else {
            out.error("no cards matched");
            out.hint(if text.trim().is_empty() {
                "remove card or location filters to list more owned cards"
            } else {
                "try broader words, or drop filters"
            });
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if json {
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let names: Vec<String> = out_hits.iter().map(|h| h.card.name.clone()).collect();
        // One batched query per finish kind instead of four per card name.
        let ranges = crate::prints::price_ranges(conn, &names)?;
        let available_all = available_counts_all(conn)?;
        let items: Vec<CollectionSearchReport> = out_hits
            .iter()
            .map(|h| {
                let range = ranges.get(&h.card.name).cloned().unwrap_or_default();
                let universe = crate::universe::card_universe(conn, &h.card.name, &h.card.set_code)
                    .unwrap_or_default();
                // `owned` and `available` come from card_json (collection
                // counts); `locations` is this command's per-hit extra.
                let card = crate::card::card_json(
                    &h.card,
                    &tag_index,
                    &range,
                    &universe,
                    &owned_counts,
                    &available_all,
                );
                CollectionSearchReport {
                    card,
                    score: (f64::from(h.score) * 10_000.0).round() / 10_000.0,
                    locations: h
                        .locations
                        .iter()
                        .map(|(binder, binder_type, quantity)| CollectionLocationReport {
                            binder: binder.clone(),
                            binder_type: binder_type.clone(),
                            quantity: *quantity,
                        })
                        .collect(),
                }
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        let styles = out.styles();
        out.print_note(if decks.is_empty() {
            "scope: binder copies only; deck-assigned copies are excluded"
        } else {
            "scope: binder copies plus copies assigned to the named decks"
        });
        for (i, hit) in out_hits.iter().enumerate() {
            let locs = hit
                .locations
                .iter()
                .map(|(b, t, q)| format!("{b} ({t}) ×{q}"))
                .collect::<Vec<_>>()
                .join(", ");
            out.print_wrapped(&format!(
                "{:>2}. {} {} {} {} {}",
                i + 1,
                styles.card_name(&hit.card.name),
                styles.mana_pips(&hit.card.mana_cost),
                styles.rarity(&hit.card.rarity),
                styles.dim(&hit.card.type_line),
                styles.dim(&if text.trim().is_empty() {
                    format!("[{locs}]")
                } else {
                    format!("(score {:.3}) [{locs}]", hit.score)
                }),
            ));
        }
    }
    Ok(crate::cli::codes::OK)
}

/// Set of owned card names for `collection query`.
///
/// Defaults to binder rows only (cards assigned to decks are spoken for;
/// deckbuilding searches want the available pool). `--binder` narrows the
/// binder pool; `--deck` adds that deck's rows (union, not intersection).
fn owned_names(
    conn: &Connection,
    binders: &[String],
    decks: &[String],
) -> anyhow::Result<std::collections::HashSet<String>> {
    let mut names = std::collections::HashSet::new();
    // Binder pool: all binders when unnamed, the named ones otherwise.
    // Named decks add their rows on top (--deck Froggy --binder Collect =
    // Froggy's cards OR the Collect binder).
    let mut stmt = conn.prepare(
        "SELECT DISTINCT name FROM collection
         WHERE (binder_type = 'binder'
                AND (?1 = 0 OR binder IN (SELECT value FROM json_each(?2))))
            OR (?3 > 0 AND binder_type = 'deck' AND binder IN (SELECT value FROM json_each(?4)))",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![
            binders.len() as i64,
            serde_json::to_string(binders)?,
            decks.len() as i64,
            serde_json::to_string(decks)?,
        ],
        |row| row.get::<_, String>(0),
    )?;
    for name in rows {
        names.insert(name.context("reading owned name")?);
    }
    Ok(names)
}

/// Names held in binders, excluding deck-assigned copies.
///
/// The public helper name is retained for existing callers. This membership
/// scope differs from [`owned_counts_all`], which also counts deck assignments.
///
/// # Errors
/// Propagates SQLite failures.
pub fn owned_names_all(conn: &Connection) -> anyhow::Result<std::collections::HashSet<String>> {
    owned_names(conn, &[], &[])
}

/// Owned copies per card name across all binders and deck assignments.
///
/// Deck suggest uses the counts so JSON `owned` reads as a number
/// everywhere (0 = none). Unlike `owned_names` (binder rows only unless
/// `--deck` names one), this scope is fixed: deck rows count, so a card
/// held only as part of a deck reports its copies. The `HashSet` variant
/// stays for membership checks that must exclude deck rows.
///
/// # Errors
/// Propagates SQLite failures.
pub fn owned_counts_all(
    conn: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut counts = std::collections::HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT name, SUM(quantity) FROM collection
         WHERE binder_type = 'binder' OR binder_type = 'deck'
         GROUP BY name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (name, qty) = row.context("reading owned counts")?;
        counts.insert(name, qty);
    }
    Ok(counts)
}

/// Binder-only copies per card name.
///
/// `available` in card JSON: copies you can trade or move without
/// dismantling a deck. Deck rows stay out of this count.
///
/// # Errors
/// Propagates SQLite failures.
pub fn available_counts_all(
    conn: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut counts = std::collections::HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT name, SUM(quantity) FROM collection
         WHERE binder_type = 'binder'
         GROUP BY name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (name, qty) = row.context("reading binder counts")?;
        counts.insert(name, qty);
    }
    Ok(counts)
}

/// Unlimited-copy basic land names (the same exemption `deck update`'s
/// singleton guard and the sim's findings use). Wastes and snow basics are
/// limited-supply cards, so they stay tracked.
pub(crate) fn is_basic_name(name: &str) -> bool {
    matches!(name, "Plains" | "Island" | "Swamp" | "Mountain" | "Forest")
}

#[cfg(test)]
#[path = "tests/collection_price_tests.rs"]
mod collection_price_tests;

#[cfg(test)]
#[path = "tests/collection_tests.rs"]
mod collection_tests;

/// Regression tests for collection import name lookup.
#[cfg(test)]
#[path = "tests/collection_import_tests.rs"]
mod collection_import_tests;

#[cfg(test)]
#[path = "tests/collection_location_tests.rs"]
mod collection_location_tests;

#[cfg(test)]
#[path = "tests/collection_conflict_tests.rs"]
mod collection_conflict_tests;

/// Parse a ManaBox purchase-price cell as a plain decimal.
///
/// Locale-formatted values ("1.234,50" = 1234.50 in de-DE, or "1,234" with
/// a thousands comma) parse as `None`: a naive parse reads "1.234" as
/// 1.234 — a 1000x error — so any value carrying more than one separator,
/// or a comma, is rejected and the row prices as 0.0 with a visible
/// warning instead of a silently wrong total.
fn parse_locale_price(text: &str) -> Option<f64> {
    let commas = text.matches(',').count();
    let dots = text.matches('.').count();
    if commas > 0 && dots > 0 {
        return None;
    }
    if commas == 1 {
        // "1,50" reads as 1.50 only when the tail is 1-2 digits; a
        // 3-digit tail is a thousands separator ("1,234"), which is
        // ambiguous by eye and rejected outright.
        let (head, tail) = text.split_once(',')?;
        if tail.len() <= 2
            && !tail.is_empty()
            && head.chars().all(|c| c.is_ascii_digit())
            && tail.chars().all(|c| c.is_ascii_digit())
        {
            return text.replace(',', ".").parse::<f64>().ok();
        }
        return None;
    }
    if dots > 1 {
        return None;
    }
    if dots == 1 {
        // A single dot with a 3-digit tail is ambiguous ("1.234" is
        // 1234 in de-DE but 1.234 in en-US): reject it the same way a
        // thousands comma is. A zero head is unambiguous — no
        // thousands notation starts with a lone zero — so "0.125"
        // stays a valid decimal.
        let (head, tail) = text.split_once('.')?;
        if head != "0" {
            let tail = tail.to_string();
            if tail.len() == 3 {
                return None;
            }
        }
    }
    text.parse::<f64>().ok()
}
