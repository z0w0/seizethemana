//! Copy-level sell plans with deck protection, demand evidence, and bulk reserves.

use std::collections::BTreeMap;

use anyhow::Context;
use rusqlite::Connection;

mod evidence;
mod export;
mod render;

pub use export::SellOutput;

/// Minimum per-copy market price for the singles view.
const SINGLES_FLOOR: f64 = 1.0;
/// Commander popularity boundary used to reserve a useful spare.
pub const PLAYED_CUTOFF: i64 = 5_000;
/// Commander popularity boundary used for low-demand evidence.
pub const DEFAULT_RANK_FLOOR: i64 = 15_000;

/// Filters and valuation assumptions for a sell plan.
#[derive(Debug, Clone)]
pub struct SellOptions<'a> {
    pub output: Option<SellOutput>,
    pub exclude_binders: &'a [String],
    pub details: bool,
    pub review: bool,
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub rarity: Option<&'a str>,
    pub format: Option<&'a str>,
    pub rank_floor: i64,
    pub target: Option<f64>,
    pub limit: usize,
    pub bulk: bool,
    pub bulk_rate: Option<f64>,
}

impl Default for SellOptions<'_> {
    fn default() -> Self {
        Self {
            output: None,
            exclude_binders: &[],
            details: false,
            review: false,
            min_price: None,
            max_price: None,
            rarity: None,
            format: None,
            rank_floor: DEFAULT_RANK_FLOOR,
            target: None,
            limit: 20,
            bulk: false,
            bulk_rate: None,
        }
    }
}

/// Owned printing with an explicit sale and retention allocation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Printing {
    pub set_code: String,
    pub collector_number: String,
    pub foil: String,
    pub binder: String,
    pub quantity: i64,
    pub sell_quantity: i64,
    pub keep_quantity: i64,
    pub price: Option<f64>,
}

/// Card metadata and binder inventory before allocation.
#[derive(Debug, Default)]
struct Candidate {
    name: String,
    rarity: String,
    edhrec_rank: Option<i64>,
    penny_rank: Option<i64>,
    reserved: bool,
    game_changer: bool,
    legalities: String,
    oracle_known: bool,
    recent_release: bool,
    prints: Vec<Printing>,
}

/// Protected demand and additional loose-copy reserve for a card.
#[derive(Clone, Copy)]
struct Protection {
    deck_needed: i64,
    spare: i64,
}

/// One card's recommended copies and the evidence behind the action.
#[derive(Debug, serde::Serialize)]
pub struct SellRow {
    pub name: String,
    pub owned_binder: i64,
    pub sell_quantity: i64,
    pub keep_quantity: i64,
    pub deck_needed: i64,
    pub spare_reserve: i64,
    pub rarity: String,
    pub edhrec_rank: Option<i64>,
    pub penny_rank: Option<i64>,
    pub combo_variants: Option<usize>,
    pub combo_piece_sets: Option<usize>,
    pub owned_combo_options: usize,
    pub combo_examples: Vec<String>,
    pub game_changer: bool,
    pub reserved: bool,
    pub legal_in_format: Option<bool>,
    pub market_value: f64,
    pub sell_priority: f64,
    pub action: &'static str,
    pub reasons: Vec<&'static str>,
    pub hold_warnings: Vec<&'static str>,
    pub printings: Vec<Printing>,
}

/// Funding picks at market value, before transaction costs.
#[derive(Debug, serde::Serialize)]
pub struct FundReport {
    pub target_usd: f64,
    pub achieved_usd: f64,
    pub shortfall_usd: f64,
    pub valuation_basis: &'static str,
    pub picks: Vec<String>,
    pub allocations: Vec<FundPick>,
}

/// Exact funding allocation that remains available outside the display limit.
#[derive(Debug, serde::Serialize)]
pub struct FundPick {
    pub name: String,
    pub sell_quantity: i64,
    pub market_value: f64,
    pub printings: Vec<Printing>,
}

/// Inventory summary independent of the displayed row limit.
#[derive(Debug, Default, serde::Serialize)]
struct Summary {
    cards: usize,
    copies: i64,
    market_value: f64,
}

/// Compact bulk summary with an optional explicit proceeds assumption.
#[derive(Debug, Default, serde::Serialize)]
struct BulkSummary {
    cards: usize,
    copies: i64,
    normal_commons_uncommons: i64,
    foils: i64,
    rares_mythics: i64,
    other: i64,
    rate_per_1000: Option<f64>,
    estimated_proceeds: Option<f64>,
}

/// Counts of protected binder copies, including reserves that fit supply.
#[derive(Debug, Default, serde::Serialize)]
struct Protected {
    deck_copies: i64,
    spare_copies: i64,
    game_changers: usize,
}

/// Typed response shared by the singles and bulk views.
#[derive(Debug, serde::Serialize)]
struct SellReport {
    currency: &'static str,
    view: &'static str,
    valuation_basis: &'static str,
    evidence_scope: &'static str,
    singles: Summary,
    bulk: BulkSummary,
    protected: Protected,
    rows: Vec<SellRow>,
    review: Vec<SellRow>,
    review_cards: usize,
    warnings: Vec<String>,
    scryfall_synced_at: String,
    combos_synced_at: String,
    excluded_binders: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fund: Option<FundReport>,
}

/// Produce a read-only sell plan. Empty views return exit code 3.
///
/// # Errors
/// Propagates inventory, metadata, deck-directory, and export writer failures.
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
    let report = build_report(paths, conn, options)?;
    let empty = if let Some(format) = options.output {
        export::write(&report, format, std::io::stdout().lock())? == 0
    } else if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        report.rows.is_empty() && report.review.is_empty()
    } else {
        render::text(out, &report, options);
        report.rows.is_empty() && report.review.is_empty()
    };
    Ok(if empty {
        crate::cli::codes::NO_RESULTS
    } else {
        crate::cli::codes::OK
    })
}

/// Load and price binder holdings in one batch, grouped by card identity.
fn load_candidates(conn: &Connection, excluded: &[String]) -> anyhow::Result<Vec<Candidate>> {
    let mut stmt = conn.prepare(
        "SELECT c.name, c.binder, c.foil, c.set_code, c.collector_number,
                c.quantity, k.name IS NOT NULL, k.rarity, k.edhrec_rank,
                k.penny_rank, k.reserved, k.game_changer, k.legalities,
                (SELECT MIN(p.released_at) FROM card_prints p WHERE p.name = c.name)
         FROM collection c LEFT JOIN cards k ON k.name = c.name
         WHERE c.binder_type = 'binder' AND c.quantity > 0 ORDER BY c.name",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                Candidate {
                    name: r.get(0)?,
                    oracle_known: r.get(6)?,
                    rarity: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    edhrec_rank: r.get(8)?,
                    penny_rank: r.get(9)?,
                    reserved: r.get::<_, Option<bool>>(10)?.unwrap_or(false),
                    game_changer: r.get::<_, Option<bool>>(11)?.unwrap_or(false),
                    legalities: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
                    recent_release: r.get::<_, Option<String>>(13)?.is_some_and(|date| {
                        chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_ok_and(|date| {
                            chrono::Utc::now()
                                .date_naive()
                                .signed_duration_since(date)
                                .num_days()
                                < 90
                        })
                    }),
                    prints: Vec::new(),
                },
                Printing {
                    binder: r.get(1)?,
                    foil: r.get(2)?,
                    set_code: r.get::<_, String>(3)?.to_ascii_lowercase(),
                    collector_number: r.get(4)?,
                    quantity: r.get(5)?,
                    sell_quantity: 0,
                    keep_quantity: 0,
                    price: None,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("reading sell inventory")?;
    let keys = rows
        .iter()
        .map(|(c, p)| {
            (
                c.name.clone(),
                p.set_code.clone(),
                p.collector_number.clone(),
                p.foil.clone(),
            )
        })
        .collect::<Vec<_>>();
    for name in excluded {
        if !rows
            .iter()
            .any(|(_, print)| print.binder.eq_ignore_ascii_case(name))
        {
            anyhow::bail!("binder '{name}' not found in the collection");
        }
    }
    let prices = crate::prints::prices_for_owned(conn, &keys)?;
    let mut cards = BTreeMap::new();
    for (card, mut print) in rows {
        if crate::collection::is_basic_name(&card.name) || binder_excluded(&print.binder, excluded)
        {
            continue;
        }
        print.price = prices
            .get(&(
                card.name.clone(),
                print.set_code.clone(),
                print.collector_number.clone(),
                print.foil.clone(),
            ))
            .copied()
            .flatten();
        cards
            .entry(card.name.clone())
            .or_insert(card)
            .prints
            .push(print);
    }
    Ok(cards.into_values().collect())
}

/// Allocate deck protection and spare copies to the cheapest known printings.
fn allocate(card: &mut Candidate, deck_needed: i64, spare: i64) {
    card.prints.sort_by(|a, b| {
        a.price
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.price.unwrap_or(f64::INFINITY))
            .then_with(|| {
                (&a.set_code, &a.collector_number, &a.foil, &a.binder).cmp(&(
                    &b.set_code,
                    &b.collector_number,
                    &b.foil,
                    &b.binder,
                ))
            })
    });
    let mut remaining = deck_needed + spare;
    for print in &mut card.prints {
        print.keep_quantity = remaining.min(print.quantity);
        remaining -= print.keep_quantity;
        print.sell_quantity = print.quantity - print.keep_quantity;
    }
}

/// Select printing-level actions without treating unknown prices as bulk.
fn eligible(print: &Printing, card: &Candidate, options: &SellOptions<'_>, bulk: bool) -> bool {
    print.sell_quantity > 0
        && print.price.is_some_and(|p| {
            (if bulk {
                p < SINGLES_FLOOR
            } else {
                p >= SINGLES_FLOOR
            }) && options.min_price.is_none_or(|min| p >= min)
                && options.max_price.is_none_or(|max| p <= max)
        })
        && options
            .rarity
            .is_none_or(|r| card.rarity.eq_ignore_ascii_case(r))
}

/// Rank sales by value released and the strongest remaining usefulness signal.
fn priority(
    card: &Candidate,
    combo: &evidence::ComboEvidence,
    value: f64,
    retains_copy: bool,
    floor: i64,
) -> f64 {
    if retains_copy {
        return value;
    }
    let edh = card.edhrec_rank.map_or(0.35, |r| {
        if r <= PLAYED_CUTOFF {
            1.0
        } else {
            (1.0 - (r - PLAYED_CUTOFF) as f64 / (floor - PLAYED_CUTOFF) as f64).clamp(0.0, 1.0)
        }
    });
    let penny = card
        .penny_rank
        .map_or(0.0, |r| 0.6 / (1.0 + r.max(1) as f64 / 500.0));
    let combos = (0.15 * (1.0 + combo.piece_sets as f64).log2()).min(0.6);
    let recent = if card.recent_release { 0.15_f64 } else { 0.0 };
    value * (1.0 - edh.max(penny).max(combos).max(recent))
}

/// Build exact card actions with raw evidence and a nonprobabilistic priority.
fn build_row(
    card: &Candidate,
    combo: &evidence::ComboEvidence,
    protection: Protection,
    options: &SellOptions<'_>,
    bulk: bool,
    evidence_known: bool,
) -> Option<SellRow> {
    let Protection { deck_needed, spare } = protection;
    let mut prints = card.prints.clone();
    for print in &mut prints {
        if !eligible(print, card, options, bulk) {
            print.keep_quantity += print.sell_quantity;
            print.sell_quantity = 0;
        }
    }
    let sold: i64 = prints.iter().map(|p| p.sell_quantity).sum();
    if sold == 0 {
        return None;
    }
    let owned: i64 = prints.iter().map(|p| p.quantity).sum();
    let retained = owned > sold;
    let value: f64 = prints
        .iter()
        .map(|p| p.price.unwrap_or(0.0) * p.sell_quantity as f64)
        .sum();
    let mut warnings = Vec::new();
    if !card.oracle_known {
        warnings.push("unknown_card_metadata");
    }
    if card.reserved {
        warnings.push("reserved_list");
    }
    if !retained && combo.interested {
        warnings.push("maybeboard_interest");
    }
    let mut reasons = vec![if deck_needed == 0 {
        "no_unmet_deck_demand"
    } else {
        "deck_needs_covered"
    }];
    if retained {
        reasons.push("surplus_copies");
    }
    if card.edhrec_rank.is_some_and(|r| r > options.rank_floor) {
        reasons.push("low_commander_demand");
    }
    if evidence_known && combo.variants == 0 {
        reasons.push("no_known_combos");
    }
    let action = if warnings.is_empty() {
        if bulk { "bulk" } else { "sell" }
    } else {
        "review"
    };
    Some(SellRow {
        name: card.name.clone(),
        owned_binder: owned,
        sell_quantity: sold,
        keep_quantity: owned - sold,
        deck_needed,
        spare_reserve: spare,
        rarity: card.rarity.clone(),
        edhrec_rank: card.edhrec_rank,
        penny_rank: card.penny_rank,
        combo_variants: evidence_known.then_some(combo.variants),
        combo_piece_sets: evidence_known.then_some(combo.piece_sets),
        owned_combo_options: combo.owned_options,
        combo_examples: combo.examples.clone(),
        game_changer: card.game_changer,
        reserved: card.reserved,
        legal_in_format: options
            .format
            .filter(|_| card.oracle_known)
            .map(|f| crate::search::legal_in(&card.legalities, f)),
        market_value: crate::output::round2(value),
        sell_priority: crate::output::round2(priority(
            card,
            combo,
            value,
            retained,
            options.rank_floor,
        )),
        action,
        reasons,
        hold_warnings: warnings,
        printings: prints,
    })
}

/// Build both inventory summaries, then limit only the selected view.
fn build_report(
    paths: &crate::paths::Paths,
    conn: &Connection,
    options: &SellOptions<'_>,
) -> anyhow::Result<SellReport> {
    let mut cards = load_candidates(conn, options.exclude_binders)?;
    let evidence = evidence::load(paths, conn, &cards, options)?;
    let status = crate::paths::Status::read(&paths.status_file())?;
    let mut report = SellReport {
        currency: crate::output::CURRENCY,
        view: if options.bulk { "bulk" } else { "singles" },
        valuation_basis: "market_value_before_selling_costs",
        evidence_scope: "Commander popularity, Penny Dreadful popularity, and known Spellbook combos; other format usage is unknown",
        singles: Summary::default(),
        bulk: BulkSummary::default(),
        protected: Protected::default(),
        rows: Vec::new(),
        review: Vec::new(),
        review_cards: 0,
        warnings: evidence.warnings.clone(),
        scryfall_synced_at: status.scryfall_synced_at,
        combos_synced_at: status.combos_synced_at,
        excluded_binders: options.exclude_binders.to_vec(),
        fund: None,
    };
    for card in &mut cards {
        let combo = evidence.cards.get(&card.name).cloned().unwrap_or_default();
        let needed = evidence.deck_needed.get(&card.name).copied().unwrap_or(0);
        let useful = card.game_changer
            || card.edhrec_rank.is_some_and(|r| r <= PLAYED_CUTOFF)
            || card.penny_rank.is_some()
            || combo.owned_options > 0
            || combo.deck_completion;
        let bulk_card = card
            .prints
            .iter()
            .any(|p| p.price.is_some_and(|v| v < SINGLES_FLOOR));
        let spare = if bulk_card { 4 } else { i64::from(useful) };
        allocate(card, needed, spare);
        let kept: i64 = card.prints.iter().map(|p| p.keep_quantity).sum();
        report.protected.deck_copies += kept.min(needed);
        report.protected.spare_copies += (kept - needed).max(0);
        report.protected.game_changers += usize::from(card.game_changer && kept > 0);
        for bulk in [false, true] {
            let Some(mut row) = build_row(
                card,
                &combo,
                Protection {
                    deck_needed: needed,
                    spare,
                },
                options,
                bulk,
                evidence.combos_known,
            ) else {
                continue;
            };
            if !evidence.decks_known {
                row.action = "review";
                row.hold_warnings.push("unchecked_deck_demand");
            }
            if row.action == "review" {
                if bulk == options.bulk {
                    report.review.push(row);
                }
            } else {
                summarize(&mut report, &row, bulk);
                if bulk == options.bulk {
                    report.rows.push(row);
                }
            }
        }
        if !options.bulk && card.prints.iter().any(|p| p.price.is_none()) {
            report.warnings.push(format!(
                "{} has unpriced copies; those copies are not sale candidates",
                card.name
            ));
        }
    }
    sort_rows(&mut report.rows, options.bulk);
    sort_rows(&mut report.review, options.bulk);
    report.review_cards = report.review.len();
    report.fund = options
        .target
        .map(|target| build_fund(&report.rows, target));
    if options.output.is_none() {
        report.rows.truncate(options.limit);
        report.review.truncate(options.limit);
    }
    report.singles.market_value = crate::output::round2(report.singles.market_value);
    report.bulk.rate_per_1000 = options.bulk_rate;
    report.bulk.estimated_proceeds = options
        .bulk_rate
        .map(|r| crate::output::round2(r * report.bulk.copies as f64 / 1000.0));
    Ok(report)
}

/// Match whole binder names without changing the stored inventory.
fn binder_excluded(binder: &str, excluded: &[String]) -> bool {
    excluded
        .iter()
        .any(|name| binder.eq_ignore_ascii_case(name))
}

/// Accumulate recommended quantities independently from review rows and limits.
fn summarize(report: &mut SellReport, row: &SellRow, bulk: bool) {
    if !bulk {
        report.singles.cards += 1;
        report.singles.copies += row.sell_quantity;
        report.singles.market_value += row.market_value;
        return;
    }
    report.bulk.cards += 1;
    report.bulk.copies += row.sell_quantity;
    for p in &row.printings {
        if p.foil != "normal" {
            report.bulk.foils += p.sell_quantity;
        } else if matches!(row.rarity.as_str(), "common" | "uncommon") {
            report.bulk.normal_commons_uncommons += p.sell_quantity;
        } else if matches!(row.rarity.as_str(), "rare" | "mythic") {
            report.bulk.rares_mythics += p.sell_quantity;
        } else {
            report.bulk.other += p.sell_quantity;
        }
    }
}

/// Use sale priority for singles and storage reduction for bulk.
fn sort_rows(rows: &mut [SellRow], bulk: bool) {
    rows.sort_by(|a, b| {
        if bulk {
            b.sell_quantity.cmp(&a.sell_quantity)
        } else {
            b.sell_priority
                .total_cmp(&a.sell_priority)
                .then_with(|| b.market_value.total_cmp(&a.market_value))
        }
        .then_with(|| a.name.cmp(&b.name))
    });
}

/// Fund only recommended singles, independent of the display limit.
fn build_fund(rows: &[SellRow], target: f64) -> FundReport {
    let mut achieved = 0.0;
    let mut picks = Vec::new();
    let mut allocations = Vec::new();
    for row in rows {
        if achieved >= target {
            break;
        }
        achieved += row.market_value;
        picks.push(row.name.clone());
        allocations.push(FundPick {
            name: row.name.clone(),
            sell_quantity: row.sell_quantity,
            market_value: row.market_value,
            printings: row
                .printings
                .iter()
                .filter(|p| p.sell_quantity > 0)
                .cloned()
                .collect(),
        });
    }
    FundReport {
        target_usd: target,
        achieved_usd: crate::output::round2(achieved),
        shortfall_usd: crate::output::round2((target - achieved).max(0.0)),
        valuation_basis: "market_value_before_selling_costs",
        picks,
        allocations,
    }
}

#[cfg(test)]
#[path = "tests/collection_sell_tests.rs"]
mod collection_sell_tests;
