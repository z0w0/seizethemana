// Deck overview stats for the human `stm deck show` view: mana curve, ramp
// sources, color identity, and type breakdown, all pure functions over the
// deck's card metadata. Ownership completion comes from the caller.
//
// Ramp classification is heuristic on purpose: lands, artifacts that add
// mana ("rocks"), creatures that add mana ("dorks"), and everything else
// ("other"). A card can serve more than one role (e.g. a land that also
// taps for colored mana counts only as a land).

use crate::db::CardRow;
use crate::deck::Deck;
use anyhow::Context;
use std::collections::HashMap;

/// Mana curve bucket for a nonland card's CMC (`"0"`..`"6"`, `"7+"`).
fn cmc_bucket(cmc: f64, is_land: bool) -> Option<String> {
    if is_land {
        return None;
    }
    Some(if cmc >= 7.0 {
        "7+".to_string()
    } else {
        (cmc as i64).to_string()
    })
}

/// True when the card is a land: any face's type line contains the word
/// "Land" before the em-dash clause (covers "Basic Land — Plains" and
/// split cards like "Land // Creature").
pub fn is_land(card: &CardRow) -> bool {
    card.type_line
        .split(" // ")
        .any(|face| face.split('—').next().unwrap_or("").contains("Land"))
}

/// True when the card is a basic land: any face's type line contains
/// "Basic Land". Basics are assumed available in unlimited supply, so deck
/// tooling never counts them as missing from the collection.
pub fn is_basic_land(card: &CardRow) -> bool {
    card.type_line.contains("Basic Land")
}

/// True when the oracle text lets the deck run more than 4 copies
/// ("a deck can have any number of cards named …"). Covers Relentless Rats,
/// Shadowborn Apostle, Seven Dwarves in a Dwarven Deck, and similar.
pub fn is_unlimited_copies(card: &CardRow) -> bool {
    let text = card.oracle_text.to_ascii_lowercase();
    text.contains("any number of cards named")
}

/// One histogram line: bucket label, bar, count.
#[derive(Debug, Clone)]
pub struct BucketLine {
    /// Row label ("2", "rare", "W", ...).
    pub label: String,
    /// Count of card copies in the bucket.
    pub count: i64,
    /// Bar fill ratio in [0, 1] relative to the largest bucket.
    pub ratio: f64,
}

/// Aggregate deck stats for the overview block.
#[derive(Debug, Default)]
pub struct DeckStats {
    /// Total card copies in the deck.
    pub total: i64,
    /// Average nonland CMC.
    pub avg_cmc: f64,
    /// Copies by CMC bucket (nonland cards only).
    pub curve: Vec<BucketLine>,
    /// Ramp source counts: (lands, rocks, dorks, other producers).
    pub ramp: (i64, i64, i64, i64),
    /// Copies by color-identity letter (WUBRG order at render time).
    pub colors: Vec<BucketLine>,
    /// Copies by type line (top 6, most copies first).
    pub types: Vec<BucketLine>,
}

/// Card rows for every deck entry name, keyed by name.
///
/// Names not in the oracle are absent from the map, so the overview shows
/// them as data-less copies only. SQL failures propagate as errors; they
/// never silently empty the map and mark every card unknown.
///
/// # Errors
/// Propagates SQLite failures.
pub fn lookup_names(
    conn: &rusqlite::Connection,
    deck: &Deck,
) -> anyhow::Result<HashMap<String, CardRow>> {
    let mut map = HashMap::new();
    // Chunked IN-lookups over the deck's own names instead of materializing
    // every card in the store; a 40-card deck reads 40 rows, not 33,000.
    let names: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut unique = Vec::new();
        for name in deck.entries().map(|e| e.name.clone()) {
            if seen.insert(name.clone()) {
                unique.push(name);
            }
        }
        unique
    };
    if names.is_empty() {
        return Ok(map);
    }
    const CHUNK: usize = 400;
    let row_sql = "SELECT name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                keywords, power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
                legalities, set_code, collector_number, scryfall_id, released_at,
                game_changer
         FROM cards WHERE name IN";
    for chunk in names.chunks(CHUNK) {
        let mut stmt = conn
            .prepare(&format!(
                "{row_sql} ({})",
                chunk.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
            ))
            .context("preparing card lookup")?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok(CardRow {
                    name: row.get(0)?,
                    oracle_id: row.get(1)?,
                    mana_cost: row.get(2)?,
                    cmc: row.get(3)?,
                    type_line: row.get(4)?,
                    colors: row.get(5)?,
                    color_identity: row.get(6)?,
                    keywords: row.get(7)?,
                    power: row.get(8)?,
                    toughness: row.get(9)?,
                    loyalty: row.get(10)?,
                    oracle_text: row.get(11)?,
                    rarity: row.get(12)?,
                    edhrec_rank: row.get(13)?,
                    legalities: row.get(14)?,
                    set_code: row.get(15)?,
                    collector_number: row.get(16)?,
                    scryfall_id: row.get(17)?,
                    released_at: row.get(18)?,
                    game_changer: row.get(19)?,
                })
            })
            .context("reading card rows")?;
        for row in rows.flatten() {
            map.insert(row.name.clone(), row);
        }
    }
    Ok(map)
}

/// Compute the overview stats for a deck joined to card metadata.
///
/// Prints missing from the oracle contribute copies to totals but no curve,
/// ramp, color, or type data.
pub fn compute(deck: &Deck, cards_by_name: &HashMap<String, CardRow>) -> DeckStats {
    let mut stats = DeckStats::default();
    let mut curve: std::collections::BTreeMap<String, i64> = Default::default();
    let mut colors: std::collections::BTreeMap<String, i64> = Default::default();
    let mut types: std::collections::BTreeMap<String, i64> = Default::default();
    let mut cmc_sum = 0.0f64;
    let mut cmc_cards = 0i64;

    for entry in deck.entries() {
        stats.total += entry.quantity;
        let Some(card) = cards_by_name.get(&entry.name) else {
            continue;
        };
        let land = is_land(card);
        if let Some(bucket) = cmc_bucket(card.cmc, land) {
            *curve.entry(bucket).or_insert(0) += entry.quantity;
            cmc_sum += card.cmc * entry.quantity as f64;
            cmc_cards += entry.quantity;
        }
        if land {
            stats.ramp.0 += entry.quantity;
        } else if is_rock(card) {
            stats.ramp.1 += entry.quantity;
        } else if is_dork(card) {
            stats.ramp.2 += entry.quantity;
        } else if produces_mana(&card.oracle_text) {
            stats.ramp.3 += entry.quantity;
        }
        if let Ok(identity) = serde_json::from_str::<Vec<String>>(&card.color_identity) {
            for color in identity {
                *colors
                    .entry(color.chars().next().unwrap_or('C').to_string())
                    .or_insert(0) += entry.quantity;
            }
        }
        // Collapse the type line to its primary word for the breakdown.
        let primary = card
            .type_line
            .split('—')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !primary.is_empty() {
            *types.entry(primary).or_insert(0) += entry.quantity;
        }
    }

    stats.avg_cmc = if cmc_cards > 0 {
        cmc_sum / cmc_cards as f64
    } else {
        0.0
    };
    let normalize = |map: std::collections::BTreeMap<String, i64>| -> Vec<BucketLine> {
        let max = map.values().copied().max().unwrap_or(1).max(1) as f64;
        map.into_iter()
            .map(|(label, count)| BucketLine {
                label,
                count,
                ratio: count as f64 / max,
            })
            .collect()
    };
    stats.curve = normalize(curve);
    stats.colors = normalize(colors);
    let mut types = normalize(types);
    types.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    types.truncate(6);
    stats.types = types;
    stats
}

/// The curve-target sentence for the deck: format/archetype-aware from
/// the average-MV bands, three-way in both formats. Commander
/// decks anchor on the singleton turn scale; 60-card decks on the
/// Karsten band class.
pub fn curve_target(is_commander: bool, avg_cmc: f64) -> &'static str {
    if is_commander {
        if avg_cmc < 2.0 {
            "target: comes together by t4-t6"
        } else if avg_cmc < 3.0 {
            "target: comes together by t6-t8"
        } else {
            "target: comes together by t8-t10"
        }
    } else if avg_cmc < 2.0 {
        "target: does its thing by t4"
    } else if avg_cmc < 3.0 {
        "target: does its thing by t4-t6"
    } else {
        "target: does its thing by t6"
    }
}

/// The curve histogram indexed MV 0..6+ (7 slots; 6 and 7+ collapse
/// into the last slot). Shared by the human line and the JSON block.
pub fn curve_histogram(stats: &DeckStats) -> Vec<u32> {
    let mut histogram = vec![0u32; 7];
    for bucket in &stats.curve {
        let label = bucket.label.trim_end_matches('+');
        let mv: usize = label.parse().unwrap_or(6);
        let idx = mv.clamp(0, 6);
        histogram[idx] += bucket.count as u32;
    }
    histogram
}

/// The curve JSON block: average nonland MV and the histogram indexed
/// MV 0..6+. The target sentence follows the deck's format.
pub fn curve_json(stats: &DeckStats, is_commander: bool) -> serde_json::Value {
    serde_json::json!({
        "avg_mv": (stats.avg_cmc * 10.0).round() / 10.0,
        "histogram": curve_histogram(stats),
        "target": curve_target(is_commander, stats.avg_cmc),
    })
}

/// The ramp JSON block: the deck's mana-source census (lands, rocks,
/// dorks, other producers) as the machine-readable counterpart of the
/// human overview's `Ramp` lines.
pub fn ramp_json(stats: &DeckStats) -> serde_json::Value {
    serde_json::json!({
        "lands": stats.ramp.0,
        "rocks": stats.ramp.1,
        "dorks": stats.ramp.2,
        "other": stats.ramp.3,
    })
}

/// Matches the `{T}: Add {…}` shape plus prose forms like "Add one mana of
/// any color" (Birds of Paradise) and "adds one mana of any one color", which
/// the pip pattern misses.
fn produces_mana(oracle_text: &str) -> bool {
    let text = oracle_text.to_ascii_lowercase();
    text.contains("add {") || text.contains("add one mana") || text.contains("adds one mana")
}

/// True when the card is an artifact mana rock.
pub fn is_rock(card: &CardRow) -> bool {
    !is_land(card) && card.type_line.contains("Artifact") && produces_mana(&card.oracle_text)
}

/// True when the card is a creature mana dork.
pub fn is_dork(card: &CardRow) -> bool {
    !is_land(card) && card.type_line.contains("Creature") && produces_mana(&card.oracle_text)
}
#[cfg(test)]
#[path = "tests/stats_tests.rs"]
mod stats_tests;
