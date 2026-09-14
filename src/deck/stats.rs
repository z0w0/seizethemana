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
/// them as data-less copies only.
///
/// # Errors
/// Propagates SQLite failures.
pub fn lookup_names(conn: &rusqlite::Connection, deck: &Deck) -> HashMap<String, CardRow> {
    let mut map = HashMap::new();
    let mut stmt = match conn.prepare(
        "SELECT name, oracle_id, mana_cost, cmc, type_line, colors, color_identity, keywords,
                power, toughness, loyalty, oracle_text, rarity, edhrec_rank,
                legalities, set_code, collector_number, scryfall_id, released_at,
                game_changer
         FROM cards",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return map,
    };
    let rows = stmt.query_map([], |row| {
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
    });
    let Ok(rows) = rows else { return map };
    for row in rows.flatten() {
        map.insert(row.name.clone(), row);
    }
    // Keep only names this deck references.
    let names: std::collections::HashSet<&str> = deck.entries().map(|e| e.name.as_str()).collect();
    map.retain(|name, _| names.contains(name.as_str()));
    map
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

/// True when the oracle text produces mana.
///
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
