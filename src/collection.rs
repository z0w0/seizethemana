use anyhow::Context;
use rusqlite::Connection;

// Collection: ManaBox CSV import, stats, and owned-only semantic search.
// CSV rows key by (binder name, binder type ∈ {binder, deck}, name, set, cn,
// foil); quantity aggregates on that key. Deck rows project onto
// `decks/<name>.txt` during import; `list` (wishlist) rows are ignored.

/// One parsed ManaBox row.
#[derive(Debug, Clone, PartialEq)]
pub struct CsvRow {
    /// Binder or deck name (`Binder Name` column).
    pub binder: String,
    pub binder_type: BinderType,
    pub name: String,
    pub set_code: String,
    pub collector_number: String,
    /// `normal`, `foil`, or `etched`.
    pub foil: String,
    pub quantity: i64,
    pub purchase_price: f64,
    pub scryfall_id: String,
}

/// ManaBox binder type; wishlist (`list`) rows are skipped on import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinderType {
    Binder,
    Deck,
}

impl BinderType {
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
pub fn parse_csv(path: &std::path::Path) -> anyhow::Result<(Vec<CsvRow>, usize)> {
    let file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
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
    for record in reader.records() {
        let record = record.with_context(|| format!("parsing {}", path.display()))?;
        let kind = match record
            .get(c_type)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "binder" => BinderType::Binder,
            "deck" => BinderType::Deck,
            "list" => {
                skipped_list_rows += 1;
                continue;
            }
            other => {
                anyhow::bail!("unknown binder type {other:?} in {}", path.display());
            }
        };
        let foil = match record.get(c_foil).unwrap_or_default().trim() {
            "foil" => "foil",
            "etched" => "etched",
            _ => "normal",
        };
        let quantity: i64 = record
            .get(c_qty)
            .unwrap_or("1")
            .trim()
            .parse()
            .with_context(|| {
                format!(
                    "unparseable quantity {:?} for card {:?}",
                    record.get(c_qty).unwrap_or_default(),
                    record.get(c_name).unwrap_or_default(),
                )
            })?;
        anyhow::ensure!(
            quantity > 0,
            "non-positive quantity {quantity} for card {:?}",
            record.get(c_name).unwrap_or_default()
        );
        let purchase_price = c_price
            .and_then(|i| record.get(i))
            .and_then(|p| p.trim().parse::<f64>().ok())
            .unwrap_or(0.0);
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
    Ok((rows, skipped_list_rows))
}

/// Aggregate CSV rows onto the collection key (duplicate rows add up).
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
    pub binders: usize,
    pub decks: usize,
    pub rows: usize,
    pub cards: i64,
    pub skipped_list_rows: usize,
    pub unknown_cards: Vec<String>,
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
    _force: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet; import needs the oracle to match names");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let started = std::time::Instant::now();
    out.status("Importing", &format!("collection from {}", file.display()));
    let (parsed, skipped_list_rows) = parse_csv(file)?;
    let aggregated = aggregate(&parsed);
    out.status(
        "Read",
        &format!(
            "{} rows ({} after aggregating duplicates)",
            parsed.len(),
            aggregated.len(),
        ),
    );

    // Resolve card names against the oracle (case-insensitive). Rows that
    // name a known token/art/emblem (recorded from the bulk) skip silently;
    // genuinely unknown names warn so the user can spot data problems.
    let mut unknown: Vec<String> = Vec::new();
    let mut skipped_tokens = 0usize;
    let mut resolved: Vec<CsvRow> = Vec::new();
    for row in &aggregated {
        match crate::db::resolve_name(conn, &row.name)? {
            crate::db::NameMatch::Found(card) => {
                let mut row = row.clone();
                row.name = card.name;
                resolved.push(row);
            }
            _ => {
                if crate::db::is_token_name(conn, &row.name)? {
                    skipped_tokens += 1;
                } else {
                    unknown.push(row.name.clone());
                }
            }
        }
    }
    if !unknown.is_empty() {
        out.warning(&format!(
            "{} CSV entries not in the oracle were skipped (e.g. {})",
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
            &format!("{skipped_tokens} token entries (tokens are not oracle cards)"),
        );
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
            unknown_cards: unknown,
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
        let deck_names = deck_names_from_aggregated(&aggregated);
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
            "{} entries ({} cards) into {} binders + {} decks{}",
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
fn deck_names_from_aggregated(aggregated: &[CsvRow]) -> Vec<String> {
    let mut decks: Vec<String> = aggregated
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

/// Whole-collection aggregates for `stm collection`.
#[derive(Debug, Default)]
pub struct Stats {
    pub unique_cards: usize,
    pub total_cards: i64,
    pub foils: i64,
    pub total_value: f64,
    pub purchase_total: f64,
    pub color_identity: std::collections::BTreeMap<String, i64>,
    pub curve: std::collections::BTreeMap<String, i64>,
    pub rarity: std::collections::BTreeMap<String, i64>,
    /// Most-represented sets: (full name, code, copies). The name is the
    /// code when the store has no set metadata yet.
    pub top_sets: Vec<(String, String, i64)>,
    pub binders: Vec<(String, String, i64, i64)>,
    /// Cards + value per universe ("multiverse" / "beyond"), then per
    /// franchise inside the beyond bucket.
    pub by_universe: std::collections::BTreeMap<String, Bucket>,
    pub by_franchise: std::collections::BTreeMap<String, Bucket>,
}

/// One census bucket: copies and their value at the owned printings.
#[derive(Debug, Default, Clone)]
pub struct Bucket {
    /// Copies in the bucket.
    pub cards: i64,
    /// Their value from the exact owned printings' price snapshots.
    pub value: f64,
}

/// Aggregate the whole collection (binders and decks).
///
/// Card metadata (colors, cmc, rarity) joins on name; each owned copy prices
/// by its exact printing via `card_prints`. Prints not in the snapshot are
/// still counted in totals but contribute no metadata or value.
///
/// # Errors
/// Propagates SQLite failures.
pub fn compute_stats(conn: &Connection) -> anyhow::Result<Stats> {
    let mut stats = Stats::default();
    let mut stmt = conn.prepare(
        "SELECT c.name, c.binder, c.binder_type, c.foil, c.quantity,
                c.purchase_price, k.type_line, k.colors, k.color_identity, k.cmc,
                k.rarity, c.set_code, c.collector_number
         FROM collection c LEFT JOIN cards k ON k.name = c.name
         ORDER BY c.name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<f64>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
        ))
    })?;
    let mut sets: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut by_universe: std::collections::BTreeMap<String, Bucket> = Default::default();
    let mut by_franchise: std::collections::BTreeMap<String, Bucket> = Default::default();
    let mut binder_totals: std::collections::BTreeMap<(String, String), i64> =
        std::collections::BTreeMap::new();
    let mut unique_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for row in rows {
        let (
            name,
            binder,
            binder_type,
            foil,
            quantity,
            purchase,
            type_line,
            colors,
            identity,
            cmc,
            rarity,
            set_code,
            collector_number,
        ) = row.context("reading collection row")?;
        stats.total_cards += quantity;
        stats.purchase_total += purchase;
        unique_names.insert(name.clone());
        if foil != "normal" {
            stats.foils += quantity;
        }
        *binder_totals
            .entry((binder.clone(), binder_type.clone()))
            .or_insert(0) += quantity;
        if let Some(colors_json) = &colors {
            if let Ok(colors) = serde_json::from_str::<Vec<String>>(colors_json) {
                let key: String = color_key(&colors);
                *stats.color_identity.entry(key).or_insert(0) += quantity;
            }
            if let Some(cmc) = cmc {
                let bucket = if cmc >= 7.0 {
                    "7+".to_string()
                } else {
                    (cmc as i64).to_string()
                };
                *stats.curve.entry(bucket).or_insert(0) += quantity;
            }
            if let Some(rarity) = rarity {
                *stats.rarity.entry(rarity).or_insert(0) += quantity;
            }
            *sets.entry(set_code.clone()).or_insert(0) += quantity;
            // Value from the exact owned printing's price snapshot.
            let unit =
                crate::prints::price_for_owned(conn, &name, &set_code, &collector_number, &foil)
                    .ok()
                    .flatten()
                    .unwrap_or(0.0);
            stats.total_value += unit * quantity as f64;
            // Universe bucket: UB decision spans every print of the name.
            // Copies and value both roll up (the plan's "cards + value").
            let (universe_key, franchise) = universe_bucket(conn, &name, &set_code);
            let bucket = by_universe.entry(universe_key.to_string()).or_default();
            bucket.cards += quantity;
            bucket.value += unit * quantity as f64;
            if let Some(f) = franchise {
                let bucket = by_franchise.entry(f).or_default();
                bucket.cards += quantity;
                bucket.value += unit * quantity as f64;
            }
        }
        let _ = (type_line, identity);
    }
    stats.unique_cards = unique_names.len();
    let mut top: Vec<(String, i64)> = sets.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    // Full set names beside the codes (JSON keeps the code as the key and
    // gains `set_name`; the human line prints the full name).
    stats.top_sets = top
        .into_iter()
        .take(5)
        .map(|(code, n)| {
            let name = conn
                .query_row(
                    "SELECT set_name FROM sets WHERE set_code = ?1",
                    [&code.to_ascii_lowercase()],
                    |r| r.get::<_, String>(0),
                )
                .ok()
                .unwrap_or_else(|| code.clone());
            (name, code, n)
        })
        .collect();
    stats.by_universe = by_universe;
    stats.by_franchise = by_franchise;
    stats.binders = binder_totals
        .into_iter()
        .map(|((name, kind), cards)| (name, kind, cards, 0))
        .collect();
    Ok(stats)
}

/// Universe bucket for one collection row: `("beyond", Some(franchise))`
/// when every stored print of the name is UB (franchise only when the set
/// maps to one), else `("multiverse", None)`.
fn universe_bucket(
    conn: &Connection,
    name: &str,
    set_code: &str,
) -> (&'static str, Option<String>) {
    let meta = crate::universe::card_universe(conn, name, set_code).unwrap_or_default();
    match meta.universe {
        "beyond" => ("beyond", meta.franchise),
        _ => ("multiverse", None),
    }
}

/// WUBRG-sorted color key for grouping ("G,U" style).
fn color_key(colors: &[String]) -> String {
    let mut chars: Vec<char> = colors
        .iter()
        .filter_map(|c| c.chars().next())
        .filter(|c| "WUBRG".contains(*c))
        .collect();
    let order = ['W', 'U', 'B', 'R', 'G'];
    chars.sort_by_key(|c| order.iter().position(|o| o == c).unwrap_or(5));
    if chars.is_empty() {
        "C".to_string()
    } else {
        chars.into_iter().collect()
    }
}

/// Show collection stats (text or JSON).
pub fn show_stats(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM collection", [], |r| r.get(0))
        .context("counting collection")?;
    if count == 0 {
        out.error("collection is empty");
        out.hint("import a ManaBox CSV: stm collection import <file>");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let stats = compute_stats(conn)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats_json(&stats))?);
    } else {
        print_stats(out, &stats);
    }
    Ok(crate::cli::codes::OK)
}

fn bucket_map(
    m: &std::collections::BTreeMap<String, Bucket>,
) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::<String, serde_json::Value>::from_iter(m.iter().map(|(k, b)| {
        (
            k.clone(),
            serde_json::json!({"cards": b.cards, "value": round2(b.value)}),
        )
    }))
}

fn stats_json(stats: &Stats) -> serde_json::Value {
    let map = |m: &std::collections::BTreeMap<String, i64>| {
        serde_json::Map::<String, serde_json::Value>::from_iter(
            m.iter().map(|(k, v)| (k.clone(), serde_json::json!(v))),
        )
    };
    serde_json::json!({
        "unique_cards": stats.unique_cards,
        "currency": crate::output::CURRENCY,
        "total_cards": stats.total_cards,
        "foils": stats.foils,
        "total_value": round2(stats.total_value),
        "purchase_total": round2(stats.purchase_total),
        "color_identity": map(&stats.color_identity),
        "curve": map(&stats.curve),
        "rarity": map(&stats.rarity),
        "top_sets": stats
            .top_sets
            .iter()
            .map(|(name, code, n)| {
                serde_json::json!({"set": code, "set_name": name, "cards": n})
            })
            .collect::<Vec<_>>(),
        "by_universe": bucket_map(&stats.by_universe),
        "by_franchise": bucket_map(&stats.by_franchise),
        "locations": stats.binders.iter().map(|(name, kind, cards, _)| serde_json::json!({
            "name": name, "type": kind, "cards": cards,
        })).collect::<Vec<_>>(),
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Human collection overview: aligned label column, colored counts,
/// histogram bars, and a locations list. Layout targets ~70 columns so it
/// stays readable on small terminals.
fn print_stats(out: &crate::output::Output, stats: &Stats) {
    let styles = out.styles();
    let label = |text: &str| -> String {
        if text.is_empty() {
            styles.dim(&" ".repeat(10))
        } else {
            styles.dim(&format!("{text:>9} "))
        }
    };
    let mut max_count: i64 = 1;
    for n in stats.color_identity.values().chain(stats.curve.values()) {
        max_count = max_count.max(*n);
    }
    let bar = |n: i64| styles.bar(n as f64 / max_count as f64, 12);

    println!(
        "{}: {} unique cards, {} total, {} foils",
        styles.header("Collection"),
        styles.thousands(stats.unique_cards as i64),
        styles.thousands(stats.total_cards),
        styles.thousands(stats.foils),
    );
    println!(
        "{}{} now (paid {})",
        label("Value"),
        styles.money(stats.total_value),
        styles.dim(&styles.money(stats.purchase_total)),
    );

    // Colors: WUBRG order with per-color pip styling.
    let order = ["W", "U", "B", "R", "G", "C"];
    let mut colors: Vec<(String, i64)> = stats
        .color_identity
        .iter()
        .map(|(k, n)| (k.clone(), *n))
        .collect();
    colors.sort_by_key(|(k, _)| {
        k.chars().next().map_or(6, |c| {
            order.iter().position(|o| o.starts_with(c)).unwrap_or(6)
        })
    });
    if !colors.is_empty() {
        for (i, (key, n)) in colors.iter().enumerate() {
            println!(
                "{}{} {} {}",
                if i == 0 { label("Colors") } else { label("") },
                bar(*n),
                styles.color_letters(key),
                styles.thousands(*n),
            );
        }
    }

    // Mana curve, ascending CMC buckets.
    let mut curve: Vec<(String, i64)> = stats.curve.iter().map(|(k, n)| (k.clone(), *n)).collect();
    curve.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or(u64::MAX));
    if !curve.is_empty() {
        for (i, (bucket, n)) in curve.iter().enumerate() {
            println!(
                "{}{} {} {}",
                if i == 0 { label("Curve") } else { label("") },
                bar(*n),
                styles.dim(&format!("{bucket:>2}")),
                styles.thousands(*n),
            );
        }
    }

    // Rarity, most-played first, with rarity colors.
    let rarity_order = ["mythic", "rare", "uncommon", "common"];
    let mut rarities: Vec<(String, i64)> =
        stats.rarity.iter().map(|(k, n)| (k.clone(), *n)).collect();
    rarities.sort_by_key(|(k, _)| {
        rarity_order
            .iter()
            .position(|o| o.eq_ignore_ascii_case(k))
            .unwrap_or(4)
    });
    if !rarities.is_empty() {
        for (i, (kind, n)) in rarities.iter().enumerate() {
            println!(
                "{}{} {} {}",
                if i == 0 { label("Rarity") } else { label("") },
                bar(*n),
                styles.rarity(kind),
                styles.thousands(*n),
            );
        }
    }

    if !stats.top_sets.is_empty() {
        let sets: Vec<String> = stats
            .top_sets
            .iter()
            .map(|(name, code, n)| {
                format!(
                    "{} {}",
                    styles.dim(&format!("{name} ({code})")),
                    styles.thousands(*n)
                )
            })
            .collect();
        println!("{}{}", label("Top sets"), sets.join(" · "));
    }
    if !stats.by_universe.is_empty() {
        let bits: Vec<String> = stats
            .by_universe
            .iter()
            .map(|(k, b)| format!("{} {}", styles.dim(k), styles.thousands(b.cards)))
            .collect();
        println!("{}{}", label("Universes"), bits.join(" · "));
        let bits: Vec<String> = stats
            .by_franchise
            .iter()
            .map(|(k, b)| format!("{} {}", styles.dim(k), styles.thousands(b.cards)))
            .collect();
        if !bits.is_empty() {
            println!("{}{}", label(""), bits.join(" · "));
        }
    }

    println!("{}{}", label("Locations"), styles.dim("binder / deck"));
    for (name, kind, cards, _) in &stats.binders {
        println!(
            "{}  {} {} {}",
            label(""),
            styles.card_name(name),
            styles.dim(&format!("({kind})")),
            styles.thousands(*cards),
        );
    }
}

/// One owned-search hit: card, score, and every location that owns it.
#[derive(Debug, Clone)]
pub struct OwnedHit {
    pub card: crate::db::CardRow,
    pub score: f32,
    pub owned: i64,
    pub locations: Vec<(String, String, i64)>,
}

/// Semantic search restricted to owned cards, with locations per hit.
///
/// Shares the embedding pipeline with `stm query` via
/// [`crate::query::run_search`], then decorates hits with collection rows.
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
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let filters = crate::search::CardFilters::from_cli(cli_filters)?;
    let owned = owned_names(conn, binders, decks)?;
    if owned.is_empty() {
        if binders.is_empty() && decks.is_empty() {
            out.error("collection is empty");
            out.hint("import a ManaBox CSV: stm collection import <file>");
        } else {
            out.error("no cards in the requested locations");
            out.hint("list your binders and decks: stm collection (see Locations)");
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let owned: std::collections::HashSet<String> = owned.into_iter().collect();
    let hits = crate::query::run_search(
        paths,
        conn,
        out,
        text,
        &filters,
        limit as usize,
        Some(&owned),
    )?;
    let mut out_hits: Vec<OwnedHit> = Vec::new();
    for hit in hits {
        let locations = locations_for(conn, &hit.card.name, binders, decks)?;
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
    if out_hits.is_empty() {
        if json {
            println!("[]");
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if json {
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let names: Vec<String> = out_hits.iter().map(|h| h.card.name.clone()).collect();
        // One batched query per finish kind instead of four per card name.
        let ranges = crate::prints::price_ranges(conn, &names)?;
        let items: Vec<serde_json::Value> = out_hits
            .iter()
            .map(|h| {
                let range = ranges.get(&h.card.name).cloned().unwrap_or_default();
                let universe = crate::universe::card_universe(conn, &h.card.name, &h.card.set_code)
                    .unwrap_or_default();
                let mut v = crate::card::card_json(&h.card, &tag_index, &range, &universe);
                v["score"] = serde_json::json!((f64::from(h.score) * 10_000.0).round() / 10_000.0);
                v["owned"] = serde_json::json!(h.owned);
                v["locations"] = serde_json::json!(
                    h.locations
                        .iter()
                        .map(|(b, t, q)| serde_json::json!({
                            "binder": b, "type": t, "quantity": q,
                        }))
                        .collect::<Vec<_>>()
                );
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        let styles = out.styles();
        for (i, hit) in out_hits.iter().enumerate() {
            let locs = hit
                .locations
                .iter()
                .map(|(b, _t, q)| format!("{b}×{q}"))
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "{:>2}. {} {} {} {} {}",
                i + 1,
                styles.card_name(&hit.card.name),
                styles.mana_pips(&hit.card.mana_cost),
                styles.rarity(&hit.card.rarity),
                styles.dim(&hit.card.type_line),
                styles.dim(&format!("({:.3}) [{locs}]", hit.score)),
            );
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
            OR (?3 > 0 AND binder IN (SELECT value FROM json_each(?4)))",
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

/// Set of every owned card name, regardless of location.
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

/// Collection rows for one card within the queried scope:
/// (location, type, quantity), binders first. `binders`/`decks` mirror the
/// query's filters so a `--binder` search does not advertise deck rows and
/// a bare search does not advertise deck assignments at all.
fn locations_for(
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

#[cfg(test)]
#[path = "tests/collection_tests.rs"]
mod collection_tests;

#[cfg(test)]
#[path = "tests/collection_location_tests.rs"]
mod collection_location_tests;
