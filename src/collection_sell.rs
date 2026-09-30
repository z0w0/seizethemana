//! `stm collection sell`: rank binder cards by idle value — money sitting
//! unused because no decklist wants the card and it sees little play.
//!
//! Supply is binder copies only: a card assigned to a deck, or wanted by any
//! decklist, is never a candidate, and neither are basics. Price and value are
//! **per printing** (printings of the same card trade at very different
//! prices), while play demand, rarity, Reserved List, and Game Changer are
//! **per oracle card** and come from the `cards` table.
//!
//! Each row carries explainable `reasons` to sell (`not_in_deck`,
//! `rarely_played`, `reprint_risk`, `format_unplayed`) and `hold_warnings` to
//! keep (`reserved_list`, `game_changer`), never a black-box score.
//!
//! Play demand comes from two free Scryfall bulk signals: `edhrec_rank`
//! (Commander) and `penny_rank` (Penny Dreadful). A card with neither rank and
//! no deck demand is treated as unplayed. Because `edhrec_rank` is
//! Commander-only, `--format` gating exists to catch cards that see play in a
//! format you actually care about.

use anyhow::Context;
use rusqlite::Connection;

use crate::collection_conflicts::deck_demand;

/// Per-copy price at or under which a print is bulk ("boxful money"): bulk
/// rares trade around $0.15-0.25, and commons only sell by the thousand.
const BULK_FLOOR: f64 = 0.25;

/// EDHREC rank at or under which a card is considered actively played.
pub const PLAYED_CUTOFF: i64 = 5_000;

/// Default EDHREC rank past which a card counts as unplayed. Overridable
/// with `--rank-floor`. Roughly the tail of EDHREC's ranked set.
pub const DEFAULT_RANK_FLOOR: i64 = 15_000;

/// The play band a card falls in, from its popularity ranks.
const BAND_PLAYED: &str = "played";
/// See [`BAND_PLAYED`].
const BAND_NICHE: &str = "niche";
/// See [`BAND_PLAYED`].
const BAND_UNPLAYED: &str = "unplayed";

/// Knobs for a `collection sell` run. One struct keeps the entry point below
/// the argument-count lint.
#[derive(Debug, Clone)]
pub struct SellOptions<'a> {
    /// Keep only cards with an owned copy priced at or above this.
    pub min_price: Option<f64>,
    /// Keep only cards whose every owned copy prices at or below this.
    pub max_price: Option<f64>,
    /// Keep only this rarity (`common`, `uncommon`, `rare`, `mythic`).
    pub rarity: Option<&'a str>,
    /// Flag cards not legal in this format as `format_unplayed`.
    pub format: Option<&'a str>,
    /// EDHREC rank past which a card counts as unplayed.
    pub rank_floor: i64,
    /// Greedy-pick highest-value cards until their total reaches this USD.
    pub target: Option<f64>,
    /// Maximum rows shown.
    pub limit: usize,
}

impl Default for SellOptions<'_> {
    fn default() -> Self {
        Self {
            min_price: None,
            max_price: None,
            rarity: None,
            format: None,
            rank_floor: DEFAULT_RANK_FLOOR,
            target: None,
            limit: 50,
        }
    }
}

/// One owned printing of a candidate card.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Printing {
    /// Set code (lowercase).
    pub set_code: String,
    /// Collector number.
    pub collector_number: String,
    /// Finish: `normal`, `foil`, or `etched`.
    pub foil: String,
    /// Binder holding these copies.
    pub binder: String,
    /// Copies in this printing.
    pub quantity: i64,
    /// Per-copy price, or null when unpriced.
    pub price: Option<f64>,
}

/// One sell candidate: an oracle card with its owned printings.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SellRow {
    /// Oracle card name.
    pub name: String,
    /// Binder copies owned, across all printings.
    pub owned_binder: i64,
    /// Rarity from the card row (empty when the snapshot lacks the card).
    pub rarity: String,
    /// EDHREC rank when known.
    pub edhrec_rank: Option<i64>,
    /// Penny Dreadful rank when known.
    pub penny_rank: Option<i64>,
    /// Play band: `played`, `niche`, or `unplayed`.
    pub play_band: &'static str,
    /// Highest per-copy price across owned printings; null when all unpriced.
    pub max_price: Option<f64>,
    /// Value of owned copies priced at or above the bulk floor.
    pub sellable_value: f64,
    /// Value of owned copies priced below the bulk floor.
    pub bulk_value: f64,
    /// Total value of all priced owned copies.
    pub total_value: f64,
    /// True when the card is on the Reserved List.
    pub reserved: bool,
    /// True when the card is a Commander Game Changer.
    pub game_changer: bool,
    /// Why it is safe to sell.
    pub reasons: Vec<&'static str>,
    /// Why you might keep it.
    pub hold_warnings: Vec<&'static str>,
    /// Transparent `[0, 1]` sell confidence (see `sell_confidence`).
    pub sell_confidence: f64,
    /// The owned printings, each with its own price.
    pub printings: Vec<Printing>,
}

/// The `--target` fund summary.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FundReport {
    /// Requested amount.
    pub target_usd: f64,
    /// Total reached by the picked cards.
    pub achieved_usd: f64,
    /// Names picked, highest value first.
    pub picks: Vec<String>,
}

/// Complete typed JSON response for `collection sell`.
#[derive(Debug, serde::Serialize)]
struct SellReport {
    /// Currency for every money field.
    currency: &'static str,
    /// Total value of owned copies worth selling (at or above the bulk floor).
    sellable_binder_value: f64,
    /// Total value of owned copies below the bulk floor (boxful money).
    bulk_value: f64,
    /// Candidate rows, highest value first.
    rows: Vec<SellRow>,
    /// Present only with `--target`.
    #[serde(skip_serializing_if = "Option::is_none")]
    fund: Option<FundReport>,
}

/// One binder collection row joined to its card metadata.
struct BinderRow {
    name: String,
    binder: String,
    foil: String,
    set_code: String,
    collector_number: String,
    quantity: i64,
    oracle_known: bool,
    rarity: String,
    edhrec_rank: Option<i64>,
    penny_rank: Option<i64>,
    reserved: bool,
    game_changer: bool,
    legalities: String,
}

/// Aggregated candidate data: one oracle card with its owned printings.
struct Candidate {
    name: String,
    rarity: String,
    edhrec_rank: Option<i64>,
    penny_rank: Option<i64>,
    reserved: bool,
    game_changer: bool,
    legalities: String,
    oracle_known: bool,
    prints: Vec<Printing>,
    owned_binder: i64,
    total_value: f64,
    sellable_value: f64,
    bulk_value: f64,
    max_price: Option<f64>,
}

/// Entry point for `stm collection sell`.
///
/// Exit 0 with candidates, exit 3 when nothing qualifies.
///
/// # Errors
/// Propagates SQLite and deck-directory read failures.
pub fn run(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    options: &SellOptions<'_>,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let rows = load_binder_rows(conn)?;
    if rows.is_empty() {
        return empty_result(out, json, Empty::NoBinder);
    }
    // Cards any decklist wants are spoken for: never suggest selling them.
    let demand = deck_demand(paths)?;
    let Some(report) = build_report(conn, rows, &demand, options)? else {
        return empty_result(out, json, Empty::Filtered);
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_text(out, &report);
    }
    Ok(crate::cli::codes::OK)
}

/// Build the sell report from binder rows and deck demand, or `None` when the
/// filters removed every candidate. Split from [`run`] so tests can assert the
/// typed report without capturing stdout.
fn build_report(
    conn: &Connection,
    rows: Vec<BinderRow>,
    demand: &std::collections::BTreeMap<String, Vec<(String, i64)>>,
    options: &SellOptions<'_>,
) -> anyhow::Result<Option<SellReport>> {
    let candidates = classify(conn, rows, demand)?;
    let mut candidates = apply_filters(candidates, options);
    candidates.sort_by(|a, b| {
        b.total_value
            .total_cmp(&a.total_value)
            .then_with(|| a.name.cmp(&b.name))
    });
    if candidates.is_empty() {
        return Ok(None);
    }
    // Totals cover every candidate, before the display limit, so the summary
    // is the true sellable figure rather than just what fits on screen.
    let sellable_binder_value: f64 = candidates.iter().map(|c| c.sellable_value).sum();
    let bulk_value: f64 = candidates.iter().map(|c| c.bulk_value).sum();
    let fund = options
        .target
        .map(|target| build_fund(&candidates, target, options.limit));
    candidates.truncate(options.limit);
    Ok(Some(SellReport {
        currency: crate::output::CURRENCY,
        sellable_binder_value: crate::output::round2(sellable_binder_value),
        bulk_value: crate::output::round2(bulk_value),
        rows: candidates
            .iter()
            .map(|c| build_row(c, options.rank_floor, options.format))
            .collect(),
        fund,
    }))
}

/// Which empty state the run hit, so the hint tells the user what to do.
enum Empty {
    /// The binder holds no cards at all.
    NoBinder,
    /// Candidates existed but the filters removed them all.
    Filtered,
}

/// Print the empty contract and return exit 3.
fn empty_result(out: &mut crate::output::Output, json: bool, empty: Empty) -> anyhow::Result<i32> {
    if json {
        let report = SellReport {
            currency: crate::output::CURRENCY,
            sellable_binder_value: 0.0,
            bulk_value: 0.0,
            rows: Vec::new(),
            fund: None,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    match empty {
        Empty::NoBinder => {
            out.error("no binder cards to sell");
            out.hint("import a ManaBox CSV: stm collection import <file>");
        }
        Empty::Filtered => {
            out.error("no cards matched the sell filters");
            out.hint("loosen --min-price/--max-price/--rarity, or drop --format");
        }
    }
    Ok(crate::cli::codes::NO_RESULTS)
}

/// Every binder collection row joined to its card metadata, ordered by name.
fn load_binder_rows(conn: &Connection) -> anyhow::Result<Vec<BinderRow>> {
    let mut stmt = conn.prepare(
        "SELECT c.name, c.binder, c.foil, c.set_code, c.collector_number,
                c.quantity, (k.name IS NOT NULL) AS oracle_known, k.rarity,
                k.edhrec_rank, k.penny_rank, k.reserved, k.game_changer,
                k.legalities
         FROM collection c LEFT JOIN cards k ON k.name = c.name
         WHERE c.binder_type = 'binder'
         ORDER BY c.name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(BinderRow {
            name: row.get(0)?,
            binder: row.get(1)?,
            foil: row.get(2)?,
            set_code: row.get(3)?,
            collector_number: row.get(4)?,
            quantity: row.get(5)?,
            oracle_known: row.get(6)?,
            rarity: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
            edhrec_rank: row.get(8)?,
            penny_rank: row.get(9)?,
            reserved: row.get::<_, Option<bool>>(10)?.unwrap_or(false),
            game_changer: row.get::<_, Option<bool>>(11)?.unwrap_or(false),
            legalities: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
        })
    })?;
    rows.map(|row| row.context("reading binder row")).collect()
}

/// Fold binder rows into candidates, dropping basics and any card a decklist
/// wants. Prices each owned printing through one batched query and keeps the
/// per-print detail, so value is print-accurate.
fn classify(
    conn: &Connection,
    rows: Vec<BinderRow>,
    demand: &std::collections::BTreeMap<String, Vec<(String, i64)>>,
) -> anyhow::Result<Vec<Candidate>> {
    let keys: Vec<(String, String, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r.name.clone(),
                r.set_code.to_ascii_lowercase(),
                r.collector_number.clone(),
                r.foil.clone(),
            )
        })
        .collect();
    let prices = crate::prints::prices_for_owned(conn, &keys)?;
    let mut by_name: std::collections::BTreeMap<String, Candidate> = Default::default();
    for row in rows {
        if crate::collection::is_basic_name(&row.name) || demand.contains_key(&row.name) {
            continue;
        }
        let price = prices
            .get(&(
                row.name.clone(),
                row.set_code.to_ascii_lowercase(),
                row.collector_number.clone(),
                row.foil.clone(),
            ))
            .copied()
            .flatten();
        let candidate = by_name
            .entry(row.name.clone())
            .or_insert_with(|| Candidate {
                name: row.name.clone(),
                rarity: row.rarity.clone(),
                edhrec_rank: row.edhrec_rank,
                penny_rank: row.penny_rank,
                reserved: row.reserved,
                game_changer: row.game_changer,
                legalities: row.legalities.clone(),
                oracle_known: row.oracle_known,
                prints: Vec::new(),
                owned_binder: 0,
                total_value: 0.0,
                sellable_value: 0.0,
                bulk_value: 0.0,
                max_price: None,
            });
        candidate.owned_binder += row.quantity;
        if let Some(price) = price {
            let value = price * row.quantity as f64;
            candidate.total_value += value;
            if price >= BULK_FLOOR {
                candidate.sellable_value += value;
            } else {
                candidate.bulk_value += value;
            }
            candidate.max_price = Some(
                candidate
                    .max_price
                    .map_or(price, |current| current.max(price)),
            );
        }
        candidate.prints.push(Printing {
            set_code: row.set_code.to_ascii_lowercase(),
            collector_number: row.collector_number.clone(),
            foil: row.foil.clone(),
            binder: row.binder.clone(),
            quantity: row.quantity,
            price,
        });
    }
    Ok(by_name.into_values().collect())
}

/// Apply the CLI's price and rarity filters.
///
/// Price bounds use the highest priced owned print: `--min-price` keeps cards
/// with at least one copy at or above it; `--max-price` keeps cards whose
/// every copy is at or below it. Unpriced cards are dropped whenever either
/// bound is set, because their price cannot be confirmed.
fn apply_filters(candidates: Vec<Candidate>, options: &SellOptions<'_>) -> Vec<Candidate> {
    candidates
        .into_iter()
        .filter(|c| match c.max_price {
            Some(price) => {
                options.min_price.is_none_or(|min| price >= min)
                    && options.max_price.is_none_or(|max| price <= max)
            }
            None => options.min_price.is_none() && options.max_price.is_none(),
        })
        .filter(|c| {
            options
                .rarity
                .is_none_or(|r| c.rarity.eq_ignore_ascii_case(r))
        })
        .collect()
}

/// Build the JSON row, computing reasons, warnings, and confidence.
fn build_row(candidate: &Candidate, rank_floor: i64, format: Option<&str>) -> SellRow {
    let band = play_band(candidate.edhrec_rank, candidate.penny_rank, rank_floor);
    let mut reasons: Vec<&'static str> = vec!["not_in_deck"];
    if band != BAND_PLAYED {
        reasons.push("rarely_played");
    }
    if !candidate.reserved && candidate.max_price.is_some_and(|p| p >= BULK_FLOOR) {
        reasons.push("reprint_risk");
    }
    // An unresolved card has no known legality; do not claim it is unplayed.
    if candidate.oracle_known
        && format.is_some_and(|f| !crate::search::legal_in(&candidate.legalities, f))
    {
        reasons.push("format_unplayed");
    }
    let mut hold_warnings: Vec<&'static str> = Vec::new();
    if candidate.reserved {
        hold_warnings.push("reserved_list");
    }
    if candidate.game_changer {
        hold_warnings.push("game_changer");
    }
    SellRow {
        name: candidate.name.clone(),
        owned_binder: candidate.owned_binder,
        rarity: candidate.rarity.clone(),
        edhrec_rank: candidate.edhrec_rank,
        penny_rank: candidate.penny_rank,
        play_band: band,
        max_price: candidate.max_price.map(crate::output::round2),
        sellable_value: crate::output::round2(candidate.sellable_value),
        bulk_value: crate::output::round2(candidate.bulk_value),
        total_value: crate::output::round2(candidate.total_value),
        reserved: candidate.reserved,
        game_changer: candidate.game_changer,
        sell_confidence: sell_confidence(&hold_warnings),
        reasons,
        hold_warnings,
        printings: candidate
            .prints
            .iter()
            .map(|p| Printing {
                price: p.price.map(crate::output::round2),
                ..p.clone()
            })
            .collect(),
    }
}

/// Classify the play band from the two popularity ranks.
///
/// The EDHREC rank is primary; a Penny Dreadful rank only lifts a card out of
/// `unplayed` (it sees play somewhere, just not Commander). A card with
/// neither rank is unplayed.
fn play_band(edhrec: Option<i64>, penny: Option<i64>, rank_floor: i64) -> &'static str {
    if let Some(rank) = edhrec {
        if rank <= PLAYED_CUTOFF {
            return BAND_PLAYED;
        }
        if rank <= rank_floor {
            return BAND_NICHE;
        }
        return BAND_UNPLAYED;
    }
    if penny.is_some() {
        BAND_NICHE
    } else {
        BAND_UNPLAYED
    }
}

/// Transparent sell confidence in `[0, 1]`: full marks, minus a quarter per
/// hold warning. Not a promise of sale price, just how clean the sell is.
fn sell_confidence(hold_warnings: &[&str]) -> f64 {
    (1.0 - 0.25 * hold_warnings.len() as f64).max(0.0)
}

/// Greedy fund: pick highest-value candidates until the target is reached.
fn build_fund(candidates: &[Candidate], target: f64, limit: usize) -> FundReport {
    let mut achieved = 0.0;
    let mut picks = Vec::new();
    for candidate in candidates.iter().take(limit) {
        if achieved >= target {
            break;
        }
        achieved += candidate.total_value;
        picks.push(candidate.name.clone());
    }
    FundReport {
        target_usd: crate::output::round2(target),
        achieved_usd: crate::output::round2(achieved),
        picks,
    }
}

/// Human render: a summary line, then dead-money and bulk-cull sections.
fn render_text(out: &crate::output::Output, report: &SellReport) {
    let styles = out.styles();
    println!(
        "{}: {} sellable, {} bulk",
        styles.header("Sell candidates"),
        styles.money(report.sellable_binder_value),
        styles.dim(&styles.money(report.bulk_value)),
    );
    // A card with any copy worth selling goes under "Dead money"; the rest
    // (pure bulk or unpriced) go under "Bulk cull". One row per card only.
    let (bulk, singles): (Vec<&SellRow>, Vec<&SellRow>) =
        report.rows.iter().partition(|r| r.sellable_value <= 0.0);
    print_section(&styles, "Dead money", &singles);
    print_section(&styles, "Bulk cull", &bulk);
    if let Some(fund) = &report.fund {
        println!(
            "{} target {} → {} with {} card{}",
            styles.header("Fund"),
            styles.money(fund.target_usd),
            styles.money(fund.achieved_usd),
            fund.picks.len(),
            if fund.picks.len() == 1 { "" } else { "s" },
        );
        if !fund.picks.is_empty() {
            println!("  {}", styles.dim(&fund.picks.join(", ")));
        }
    }
}

/// Print one section of sell rows. Empty sections are skipped.
fn print_section(styles: &crate::output::Styles, title: &str, rows: &[&SellRow]) {
    if rows.is_empty() {
        return;
    }
    println!("{}", styles.header(title));
    for row in rows {
        let value = match (row.sellable_value > 0.0, row.bulk_value > 0.0) {
            (true, true) => format!(
                "{} sellable + {} bulk",
                styles.money(row.sellable_value),
                styles.money(row.bulk_value)
            ),
            (true, false) => styles.money(row.sellable_value),
            _ => styles.dim("unpriced"),
        };
        let tags: Vec<String> = row
            .reasons
            .iter()
            .chain(row.hold_warnings.iter())
            .map(|t| (*t).to_string())
            .collect();
        println!(
            "  {} ×{} {} {}",
            styles.card_name(&row.name),
            row.owned_binder,
            value,
            styles.dim(&format!("[{}]", tags.join(", "))),
        );
        let prints = row
            .printings
            .iter()
            .map(format_printing)
            .collect::<Vec<_>>()
            .join("; ");
        println!(
            "    {}",
            styles.dim(&format!("{} · {prints}", row.play_band))
        );
    }
}

/// One owned printing as `SET CN finish ×QTY $PRICE`.
fn format_printing(p: &Printing) -> String {
    let price = p
        .price
        .map_or_else(|| "unpriced".to_string(), |v| format!("${v:.2}"));
    format!(
        "{} {} {} ×{} {price}",
        p.set_code.to_ascii_uppercase(),
        p.collector_number,
        p.foil,
        p.quantity,
    )
}

#[cfg(test)]
#[path = "tests/collection_sell_tests.rs"]
mod collection_sell_tests;
