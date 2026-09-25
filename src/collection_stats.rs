// Whole-collection stats for `stm collection`: counts, value, curve,
// colors, sets, binders, and universe breakdowns. Split out of
// `collection.rs` to keep each file under the size limit.

use anyhow::Context;
use rusqlite::Connection;
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
    pub binders: Vec<(String, String, i64)>,
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
    let owned = load_owned_rows(conn)?;
    // One batched price read for every owned printing, instead of one
    // query per row.
    let keys: Vec<(String, String, String, String)> = owned
        .iter()
        .map(
            |(name, _, _, foil, _, _, _, _, _, set_code, collector_number)| {
                (
                    name.clone(),
                    set_code.to_ascii_lowercase(),
                    collector_number.clone(),
                    foil.clone(),
                )
            },
        )
        .collect();
    let prices = crate::prints::prices_for_owned(conn, &keys)?;
    let mut sets: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut by_universe: std::collections::BTreeMap<String, Bucket> = Default::default();
    let mut by_franchise: std::collections::BTreeMap<String, Bucket> = Default::default();
    let mut binder_totals: std::collections::BTreeMap<(String, String), i64> =
        std::collections::BTreeMap::new();
    let mut unique_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for row in &owned {
        accumulate_row(
            conn,
            row,
            &prices,
            &mut stats,
            &mut sets,
            &mut by_universe,
            &mut by_franchise,
            &mut binder_totals,
            &mut unique_names,
        )?;
    }
    stats.unique_cards = unique_names.len();
    stats.top_sets = top_sets(conn, sets)?;
    stats.by_universe = by_universe;
    stats.by_franchise = by_franchise;
    stats.binders = binder_totals
        .into_iter()
        .map(|((name, kind), cards)| (name, kind, cards))
        .collect();
    Ok(stats)
}

/// OwnedRow tuple type: (name, binder, binder_type, foil, quantity,
/// purchase price, color identity, CMC, rarity, set, collector number).
type OwnedRow = (
    String,
    String,
    String,
    String,
    i64,
    f64,
    Option<String>,
    Option<f64>,
    Option<String>,
    String,
    String,
);

/// Every collection row joined to its card metadata, ordered by name.
fn load_owned_rows(conn: &Connection) -> anyhow::Result<Vec<OwnedRow>> {
    let mut stmt = conn.prepare(
        "SELECT c.name, c.binder, c.binder_type, c.foil, c.quantity,
                c.purchase_price, k.color_identity, k.cmc,
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
            row.get::<_, Option<f64>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
        ))
    })?;
    rows.map(|row| row.context("reading collection row"))
        .collect()
}

/// Fold one owned row into the running stats, facet maps, and universe
/// buckets. Returns nothing; every counter mutates in place.
#[allow(clippy::too_many_arguments)]
fn accumulate_row(
    conn: &Connection,
    row: &OwnedRow,
    prices: &std::collections::HashMap<(String, String, String, String), Option<f64>>,
    stats: &mut Stats,
    sets: &mut std::collections::BTreeMap<String, i64>,
    by_universe: &mut std::collections::BTreeMap<String, Bucket>,
    by_franchise: &mut std::collections::BTreeMap<String, Bucket>,
    binder_totals: &mut std::collections::BTreeMap<(String, String), i64>,
    unique_names: &mut std::collections::BTreeSet<String>,
) -> anyhow::Result<()> {
    let (
        name,
        binder,
        binder_type,
        foil,
        quantity,
        purchase,
        identity,
        cmc,
        rarity,
        set_code,
        collector_number,
    ) = row;
    let unit = prices
        .get(&(
            name.clone(),
            set_code.to_ascii_lowercase(),
            collector_number.clone(),
            foil.clone(),
        ))
        .copied()
        .flatten()
        .unwrap_or(0.0);
    stats.total_cards += *quantity;
    // purchase_price is a per-row total: import aggregation and the
    // --add upsert both sum it across merged rows, so the stat adds it
    // once per row.
    stats.purchase_total += *purchase;
    unique_names.insert(name.clone());
    if foil != "normal" {
        stats.foils += *quantity;
    }
    *binder_totals
        .entry((binder.clone(), binder_type.clone()))
        .or_insert(0) += *quantity;
    if let Some(identity_json) = identity
        && let Ok(colors) = serde_json::from_str::<Vec<String>>(identity_json)
    {
        let key = color_key(&colors);
        *stats.color_identity.entry(key).or_insert(0) += *quantity;
    }
    if let Some(cmc) = cmc {
        let bucket = if *cmc >= 7.0 {
            "7+".to_string()
        } else {
            (*cmc as i64).to_string()
        };
        *stats.curve.entry(bucket).or_insert(0) += *quantity;
    }
    if let Some(rarity) = rarity {
        *stats.rarity.entry(rarity.clone()).or_insert(0) += *quantity;
    }
    *sets.entry(set_code.clone()).or_insert(0) += *quantity;
    // Value from the exact owned printing's price snapshot.
    stats.total_value += unit * *quantity as f64;
    // Universe bucket: UB decision spans every print of the name.
    // Copies and value both roll up.
    let (universe_key, franchise) = universe_bucket(conn, name, set_code)?;
    let bucket = by_universe.entry(universe_key.to_string()).or_default();
    bucket.cards += *quantity;
    bucket.value += unit * *quantity as f64;
    if let Some(f) = franchise {
        let bucket = by_franchise.entry(f).or_default();
        bucket.cards += *quantity;
        bucket.value += unit * *quantity as f64;
    }
    Ok(())
}

/// Top-5 set codes with their full names beside them.
fn top_sets(
    conn: &Connection,
    sets: std::collections::BTreeMap<String, i64>,
) -> anyhow::Result<Vec<(String, String, i64)>> {
    let mut top: Vec<(String, i64)> = sets.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    // Full set names beside the codes (JSON keeps the code as the key and
    // gains `set_name`; the human line prints the full name).
    Ok(top
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
        .collect())
}

/// Universe bucket for one collection row: `("beyond", Some(franchise))`
/// when every stored print of the name is UB (franchise only when the set
/// maps to one), else `("multiverse", None)`.
///
/// # Errors
/// Propagates SQLite failures.
pub(crate) fn universe_bucket(
    conn: &Connection,
    name: &str,
    set_code: &str,
) -> anyhow::Result<(&'static str, Option<String>)> {
    let meta = crate::universe::card_universe(conn, name, set_code)?;
    match meta.universe {
        "beyond" => Ok(("beyond", meta.franchise)),
        _ => Ok(("multiverse", None)),
    }
}

/// WUBRG-sorted color key for grouping ("G,U" style).
pub(crate) fn color_key(colors: &[String]) -> String {
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
        if json {
            // Empty contract on stdout, matching the other read commands.
            println!("{}", serde_json::to_string_pretty(&empty_stats_json())?);
        } else {
            out.error("collection is empty");
            out.hint("import a ManaBox CSV: stm collection import <file>");
        }
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

/// One stats bucket as JSON: card count and rounded value.
pub(crate) fn bucket_map(
    m: &std::collections::BTreeMap<String, Bucket>,
) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::<String, serde_json::Value>::from_iter(m.iter().map(|(k, b)| {
        (
            k.clone(),
            serde_json::json!({"cards": b.cards, "value": crate::output::round2(b.value)}),
        )
    }))
}

/// The empty-collection JSON contract: the stats object with zeroed
/// counts and empty buckets, so an agent parsing `collection --json`
/// gets one shape on both sides of exit 3.
fn empty_stats_json() -> serde_json::Value {
    serde_json::json!({
        "unique_cards": 0,
        "currency": crate::output::CURRENCY,
        "total_cards": 0,
        "foils": 0,
        "total_value": 0.0,
        "purchase_total": 0.0,
        "color_identity": {},
        "curve": {},
        "rarity": {},
        "top_sets": [],
        "by_universe": {},
        "by_franchise": {},
        "locations": [],
    })
}

/// The `collection --json` payload: totals, counts per facet, and the
/// universe/location splits. Prices round to two decimals.
pub(crate) fn stats_json(stats: &Stats) -> serde_json::Value {
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
        "total_value": crate::output::round2(stats.total_value),
        "purchase_total": crate::output::round2(stats.purchase_total),
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
        "locations": stats.binders.iter().map(|(name, kind, cards)| serde_json::json!({
            "name": name, "type": kind, "cards": cards,
        })).collect::<Vec<_>>(),
    })
}

/// Human collection overview: aligned label column, colored counts,
/// histogram bars, and a locations list. Layout targets ~70 columns so it
/// stays readable on small terminals.
pub(crate) fn print_stats(out: &crate::output::Output, stats: &Stats) {
    let styles = out.styles();
    let label = |text: &str| -> String {
        if text.is_empty() {
            styles.dim(&" ".repeat(10))
        } else {
            styles.dim(&format!("{text:>9} "))
        }
    };
    let bar = |ratio: f64| styles.bar(ratio, 12);

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

    print_colors(&label, &bar, &styles, stats);
    print_curve(&label, &bar, &styles, stats);
    print_rarity(&label, &bar, &styles, stats);
    print_sets_and_buckets(&label, &styles, stats);
    print_locations(&label, &styles, stats);
}

/// Color identity histogram, WUBRG order with per-color pip styling.
fn print_colors(
    label: &dyn Fn(&str) -> String,
    bar: &dyn Fn(f64) -> String,
    styles: &crate::output::Styles,
    stats: &Stats,
) {
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
    if colors.is_empty() {
        return;
    }
    let color_max = colors.iter().map(|(_, n)| *n).max().unwrap_or(1);
    for (i, (key, n)) in colors.iter().enumerate() {
        println!(
            "{}{} {} {}",
            if i == 0 { label("Colors") } else { label("") },
            bar(*n as f64 / color_max as f64),
            styles.color_letters(key),
            styles.thousands(*n),
        );
    }
}

/// Mana-curve histogram, ascending CMC buckets.
fn print_curve(
    label: &dyn Fn(&str) -> String,
    bar: &dyn Fn(f64) -> String,
    styles: &crate::output::Styles,
    stats: &Stats,
) {
    let mut curve: Vec<(String, i64)> = stats.curve.iter().map(|(k, n)| (k.clone(), *n)).collect();
    curve.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or(u64::MAX));
    if curve.is_empty() {
        return;
    }
    let curve_max = curve.iter().map(|(_, n)| *n).max().unwrap_or(1);
    for (i, (bucket, n)) in curve.iter().enumerate() {
        println!(
            "{}{} {} {}",
            if i == 0 { label("Curve") } else { label("") },
            bar(*n as f64 / curve_max as f64),
            styles.dim(&format!("{bucket:>2}")),
            styles.thousands(*n),
        );
    }
}

/// Rarity histogram, most-played first, with rarity colors.
fn print_rarity(
    label: &dyn Fn(&str) -> String,
    bar: &dyn Fn(f64) -> String,
    styles: &crate::output::Styles,
    stats: &Stats,
) {
    let rarity_order = ["mythic", "rare", "uncommon", "common"];
    let mut rarities: Vec<(String, i64)> =
        stats.rarity.iter().map(|(k, n)| (k.clone(), *n)).collect();
    rarities.sort_by_key(|(k, _)| {
        rarity_order
            .iter()
            .position(|o| o.eq_ignore_ascii_case(k))
            .unwrap_or(4)
    });
    if rarities.is_empty() {
        return;
    }
    // Bar scale per section: the rarity max, not the color/curve max.
    let rarity_max = rarities.iter().map(|(_, n)| *n).max().unwrap_or(1);
    for (i, (kind, n)) in rarities.iter().enumerate() {
        println!(
            "{}{} {} {}",
            if i == 0 { label("Rarity") } else { label("") },
            bar(*n as f64 / rarity_max as f64),
            styles.rarity(kind),
            styles.thousands(*n),
        );
    }
}

/// Top sets line and the universe/franchise split.
fn print_sets_and_buckets(
    label: &dyn Fn(&str) -> String,
    styles: &crate::output::Styles,
    stats: &Stats,
) {
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
    if stats.by_universe.is_empty() {
        return;
    }
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

/// Binder and deck locations list.
fn print_locations(label: &dyn Fn(&str) -> String, styles: &crate::output::Styles, stats: &Stats) {
    println!("{}{}", label("Locations"), styles.dim("binder / deck"));
    for (name, kind, cards) in &stats.binders {
        println!(
            "{}  {} {} {}",
            label(""),
            styles.card_name(name),
            styles.dim(&format!("({kind})")),
            styles.thousands(*cards),
        );
    }
}
