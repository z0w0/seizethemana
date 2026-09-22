use crate::output::round2;
// `stm deck buylist`: missing copies as a plain `2x Name` list (generic
// default) or a store CSV for Card Kingdom / TCGPlayer, plus a JSON view.
//
// Missing math: a deck slot is filled by copies assigned to this deck plus
// copies sitting in binders; copies assigned to other decks never count
// (that would deconstruct those decks). The cheapest released English
// printing of the right finish fills each missing slot.

use rusqlite::Connection;

use super::store::load_deck;

/// Buylist output formats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Store {
    /// Plain decklist lines: `2x Lightning Bolt` (default).
    Generic,
    /// Card Kingdom CSV import: `Name,Edition,Foil,Qty`.
    CardKingdom,
    /// TCGPlayer Mass Entry CSV.
    TCGPlayer,
}

impl Store {
    fn parse(s: &str) -> Option<Store> {
        match s.to_ascii_lowercase().as_str() {
            "generic" => Some(Store::Generic),
            "cardkingdom" | "ck" => Some(Store::CardKingdom),
            "tcgplayer" | "tcg" => Some(Store::TCGPlayer),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Store::Generic => "generic",
            Store::CardKingdom => "cardkingdom",
            Store::TCGPlayer => "tcgplayer",
        }
    }

    fn header(self) -> Option<&'static str> {
        match self {
            // Generic and Card Kingdom's CSV import take no header line;
            // CK's import tool prompts to match columns.
            Store::Generic | Store::CardKingdom => None,
            Store::TCGPlayer => Some(
                "Quantity,Name,Simple Name,Set,Card Number,Set Code,Printing,Condition,Language,Rarity",
            ),
        }
    }
}

/// One buylist line: cheapest printing for `quantity` missing copies.
///
/// `sections` names the deck sections that needed the copies (e.g. a card
/// wanted by both DECK and SIDEBOARD lists both).
#[derive(Debug, Clone)]
pub struct BuylistRow {
    pub name: String,
    pub set_code: String,
    pub set_name: Option<String>,
    pub collector_number: String,
    pub scryfall_id: String,
    pub foil: bool,
    pub quantity: i64,
    /// Sections holding this card: a BTreeSet-ordered list of section names.
    pub sections: Vec<String>,
    /// Cheapest print's USD price (finish-aware); None when unpriced.
    pub price_usd: Option<f64>,
}

/// Copies available to fill this deck's slots: assigned to the deck plus
/// copies in binders. Other decks' copies do not count. Shared with
/// `deck show` through `ownership::available_map`.
use super::ownership::available_map_by_finish;

/// Missing rows with the available-copy map already computed.
///
/// Same-name entries across sections aggregate; basic lands are excluded
/// (unlimited supply); the foil/nonfoil need aggregates per finish, so one
/// foil line no longer prices the whole purchase as foil. Unknown card
/// names (not in the oracle) are reported through the returned warning
/// instead of being skipped silently. Returns rows sorted by name
/// (BTreeMap order), or empty when nothing is missing.
pub(super) fn missing_rows(
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    available: &std::collections::HashMap<(String, bool), i64>,
) -> anyhow::Result<(Vec<BuylistRow>, Vec<String>)> {
    let mut needed: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    // Per-finish need: each deck line contributes its copies to the finish
    // that line asked for; a card wanted in both finishes gets two rows.
    let mut needed_foil: std::collections::BTreeMap<String, i64> =
        std::collections::BTreeMap::new();
    let mut unknown_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut sections: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (section, entries) in &deck.sections {
        for entry in entries {
            let Some(card) = cards_by_name.get(&entry.name) else {
                unknown_names.insert(entry.name.clone());
                continue;
            };
            if super::stats::is_basic_land(card) {
                continue;
            }
            if entry.foil {
                *needed_foil.entry(entry.name.clone()).or_insert(0) += entry.quantity;
            } else {
                *needed.entry(entry.name.clone()).or_insert(0) += entry.quantity;
            }
            let seen = sections.entry(entry.name.clone()).or_default();
            if !seen.contains(section) {
                seen.push(section.clone());
            }
        }
    }
    let mut rows = Vec::new();
    let missing_names: Vec<String> = needed
        .iter()
        .chain(needed_foil.iter())
        .map(|(name, _)| name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    // One batched query per finish kind instead of four per card name.
    let ranges = crate::prints::price_ranges(conn, &missing_names)?;
    // Owned copies fill this deck's slots by finish: a foil line is
    // filled by owned foil/etched copies, a nonfoil line by nonfoil
    // copies. When a finish's owned pool runs dry, leftover owned copies
    // of the other finish fill the remainder (any printing sleeves the
    // same); the leftover pool is shared per name, foil lines first.
    let mut leftover: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (need, foil) in [(&needed_foil, true), (&needed, false)] {
        for (name, qty) in need.iter() {
            let owned = available.get(&(name.clone(), foil)).copied().unwrap_or(0);
            *leftover.entry(name.clone()).or_insert(0) += (owned - qty).max(0);
        }
    }
    for (need, foil) in [(&needed_foil, true), (&needed, false)] {
        for (name, qty) in need.iter() {
            let owned = available.get(&(name.clone(), foil)).copied().unwrap_or(0);
            let missing = qty - owned;
            if missing <= 0 {
                continue;
            }
            // Borrow same-name copies surplus in the other finish. The
            // entry exists: the first pass inserted every needed name.
            let borrowable = leftover.get(name).copied().unwrap_or(0);
            let pool = leftover
                .get_mut(name)
                .expect("leftover pool holds every needed name");
            let covered = borrowable.min(missing);
            *pool -= covered;
            let still_missing = missing - covered;
            if still_missing <= 0 {
                continue;
            }
            let range = match ranges.get(name) {
                Some(range) => range,
                None => continue,
            };
            let print = if foil {
                range
                    .cheapest_foil
                    .clone()
                    .or_else(|| range.cheapest.clone())
            } else {
                range.cheapest.clone()
            };
            let Some(print) = print else {
                // No released English printing priced in the snapshot; skip
                // with no line rather than a zero-priced line.
                continue;
            };
            rows.push(BuylistRow {
                name: name.clone(),
                set_code: print.set_code,
                set_name: Some(print.set_name).filter(|s| !s.is_empty()),
                collector_number: print.collector_number,
                scryfall_id: print.scryfall_id,
                foil,
                quantity: still_missing,
                sections: sections.get(name).cloned().unwrap_or_default(),
                price_usd: if foil {
                    print.usd_foil.or(print.usd)
                } else {
                    print.usd
                },
            });
        }
    }
    Ok((rows, unknown_names.into_iter().collect()))
}

/// Entry point for `stm deck buylist <name>`.
///
/// Owned copies (deck-assigned plus binders) fill deck slots automatically;
/// nothing is written to the collection. The output lists only what needs
/// to be purchased.
pub fn buylist(
    paths: &crate::paths::Paths,
    conn: &Connection,
    out: &mut crate::output::Output,
    name: &str,
    store: Option<&str>,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = load_deck(paths, name)?;
    let store = match store.map(Store::parse) {
        None => Store::Generic,
        Some(Some(s)) => s,
        Some(None) => {
            out.error(&format!("unknown store {store:?}"));
            out.hint("supported: generic, cardkingdom, tcgplayer");
            return Ok(crate::cli::codes::USAGE);
        }
    };
    let cards_by_name = super::stats::lookup_names(conn, &deck)?;
    let available = available_map_by_finish(conn, name)?;
    let (rows, unknown) = missing_rows(conn, &deck, &cards_by_name, &available)?;
    for name in &unknown {
        out.warning(&format!(
            "{name} is not in the card index; skipped in the buylist"
        ));
    }
    if rows.is_empty() {
        if json {
            // Empty contract on stdout, matching the other read commands:
            // `[]` with exit 3 signaling "nothing to buy".
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "store": store.name(),
                    "currency": crate::output::CURRENCY,
                    "rows": [],
                    "total": 0.0,
                }))?
            );
        } else {
            out.error("no missing cards; the deck is fully covered");
            out.hint("check 'stm deck show' for the ownership breakdown");
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if let Store::CardKingdom = store {
        // CK's CSV import caps at 500 unique lines.
        if rows.len() > 500 {
            out.warning(&format!(
                "{} lines exceed Card Kingdom's 500-line import cap; split the list",
                rows.len()
            ));
        }
    }
    let total: f64 = rows
        .iter()
        .map(|r| r.price_usd.unwrap_or(0.0) * r.quantity as f64)
        .sum();
    // One prepared rarity statement reused for all rows (an N-prepare
    // query loop would be slower); the TCGplayer Mass Entry column
    // needs rarity per row on a 500-line export.
    let rarities = rarities_for(conn, &rows)?;
    if json {
        let v = serde_json::json!({
            "store": store.name(),
            "currency": crate::output::CURRENCY,
            "rows": rows.iter().map(|r| serde_json::json!({
                "name": r.name,
                "set": r.set_code,
                "set_name": r.set_name,
                "collector_number": r.collector_number,
                "scryfall_id": r.scryfall_id,
                "foil": r.foil,
                "quantity": r.quantity,
                "sections": r.sections,
                "price": r.price_usd,
            })).collect::<Vec<_>>(),
            "total": round2(total),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        // Plain text on stdout for `| pbcopy`; errors/progress on stderr.
        if let Some(header) = store.header() {
            println!("{header}");
        }
        for row in &rows {
            match store {
                Store::Generic => println!("{}x {}", row.quantity, row.name),
                Store::CardKingdom => println!(
                    "{},{},{},{}",
                    csv_field(&row.name),
                    csv_field(row.set_name.as_deref().unwrap_or(&row.set_code)),
                    if row.foil { "Foil" } else { "" },
                    row.quantity
                ),
                Store::TCGPlayer => println!(
                    "{},{},{},{},{},{},{},Near Mint,English,{}",
                    row.quantity,
                    csv_field(&row.name),
                    csv_field(&row.name),
                    csv_field(row.set_name.as_deref().unwrap_or(&row.set_code)),
                    row.collector_number,
                    row.set_code.to_ascii_uppercase(),
                    if row.foil { "Foil" } else { "Normal" },
                    rarities
                        .get(&row.scryfall_id)
                        .map(String::as_str)
                        .unwrap_or(""),
                ),
            }
        }
        // The total is a result: it stays on stdout so the piped
        // `| pbcopy` flow keeps the copyable CSV and the cost line.
        let styles = out.styles();
        println!(
            "{} {} missing copies, est. market {}",
            styles.glyph("Total", crate::output::GlyphKind::Dim),
            rows.iter().map(|r| r.quantity).sum::<i64>(),
            styles.money(total)
        );
    }
    Ok(crate::cli::codes::OK)
}

/// Rarity of one print (for the TCGPlayer Mass Entry column). An
/// unpriced lookup yields an empty cell, not a failed export.
///
/// # Errors
/// Propagates SQLite failures; callers treat a failed export as broken
/// rather than silently missing rarity cells.
fn rarities_for(
    conn: &Connection,
    rows: &[BuylistRow],
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    let mut stmt = conn.prepare("SELECT rarity FROM card_prints WHERE scryfall_id = ?1")?;
    for row in rows {
        let rarity: Option<String> = stmt
            .query_row([&row.scryfall_id], |r| r.get(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                err => Err(err),
            })?;
        if let Some(rarity) = rarity {
            map.insert(row.scryfall_id.clone(), rarity);
        }
    }
    Ok(map)
}

/// Quote a CSV field when it contains a comma or quote.
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> (tempfile::TempDir, Connection) {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        (tmp, conn)
    }

    fn card(conn: &Connection, name: &str) {
        conn.execute(
            "INSERT INTO cards (name, oracle_id) VALUES (?1, 'oid')",
            [name],
        )
        .unwrap();
    }

    #[cfg(test)]
    #[expect(clippy::too_many_arguments)]
    fn print(
        conn: &Connection,
        id: &str,
        name: &str,
        set: &str,
        set_name: &str,
        cn: &str,
        usd: Option<f64>,
        usd_foil: Option<f64>,
    ) {
        conn.execute(
            "INSERT INTO sets (set_code, set_name) VALUES (?1, ?2)",
            rusqlite::params![set, set_name],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'en', 'rare', '[\"nonfoil\",\"foil\"]', '2020-01-01', ?5, ?6, 't')",
            rusqlite::params![id, name, set, cn, usd, usd_foil],
        )
        .unwrap();
    }

    #[cfg(test)]
    fn collection(conn: &Connection, binder: &str, kind: &str, name: &str, qty: i64) {
        conn.execute(
            "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity)
             VALUES (?1, 'm11', '148', 'normal', ?2, ?3, ?4)",
            rusqlite::params![name, binder, kind, qty],
        )
        .unwrap();
    }

    #[test]
    fn cheapest_printing_is_chosen() {
        let (_tmp, conn) = conn();
        card(&conn, "Bolt");
        print(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            Some(0.5),
            Some(9.0),
        );
        print(
            &conn,
            "b",
            "Bolt",
            "2xm",
            "Double Masters",
            "100",
            Some(2.5),
            Some(4.0),
        );
        let range = crate::prints::price_range(&conn, "Bolt").unwrap();
        let cheapest = range.cheapest.unwrap();
        assert_eq!(cheapest.set_code, "m11");
        assert_eq!(cheapest.set_name, "Magic 2011");
    }

    #[test]
    fn missing_math_counts_binder_not_other_decks() {
        let (_tmp, conn) = conn();
        card(&conn, "Bolt");
        print(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            Some(0.5),
            None,
        );
        // 2 copies in this deck, 3 in a binder, 5 in another deck.
        collection(&conn, "Buy", "deck", "Bolt", 2);
        collection(&conn, "Collect", "binder", "Bolt", 3);
        collection(&conn, "Other", "deck", "Bolt", 5);
        let deck = super::super::Deck::parse("// DECK\n8 Bolt\n").unwrap();
        let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
        let available = available_map_by_finish(&conn, "Buy").unwrap();
        assert_eq!(
            available.get(&("Bolt".to_string(), false)),
            Some(&5),
            "own deck + binder only"
        );
        let (rows, unknown) = missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
        assert!(unknown.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].quantity, 3, "8 needed - 5 available");
        assert_eq!(rows[0].set_code, "m11");
        assert_eq!(rows[0].price_usd, Some(0.5));
    }

    #[test]
    fn nothing_missing_gives_no_rows() {
        let (_tmp, conn) = conn();
        card(&conn, "Bolt");
        print(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            Some(0.5),
            None,
        );
        collection(&conn, "Collect", "binder", "Bolt", 4);
        let deck = super::super::Deck::parse("// DECK\n4 Bolt\n").unwrap();
        let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
        let available = available_map_by_finish(&conn, "Deck").unwrap();
        let (rows, unknown) = missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
        assert!(unknown.is_empty());
        assert!(rows.is_empty());
    }

    #[test]
    fn basic_lands_never_appear() {
        let (_tmp, conn) = conn();
        conn.execute(
            "INSERT INTO cards (name, oracle_id, type_line) VALUES ('Mountain', 'oid', 'Basic Land — Mountain')",
            [],
        )
        .unwrap();
        card(&conn, "Bolt");
        print(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            Some(0.5),
            None,
        );
        let deck = super::super::Deck::parse("// DECK\n20 Mountain\n2 Bolt\n").unwrap();
        let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
        let available = available_map_by_finish(&conn, "Deck").unwrap();
        let (rows, unknown) = missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
        assert!(unknown.is_empty());
        assert_eq!(rows.len(), 1, "only the non-basic card is listed");
        assert_eq!(rows[0].name, "Bolt");
    }

    #[test]
    fn same_name_entries_aggregate() {
        let (_tmp, conn) = conn();
        card(&conn, "Bolt");
        print(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            Some(0.5),
            None,
        );
        let deck = super::super::Deck::parse("// DECK\n2 Bolt\n// SIDEBOARD\n3 Bolt\n").unwrap();
        let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
        let available = available_map_by_finish(&conn, "Deck").unwrap();
        let (rows, unknown) = missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
        assert!(unknown.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].quantity, 5);
        assert_eq!(rows[0].sections, vec!["DECK", "SIDEBOARD"]);
    }

    #[test]
    fn csv_field_quotes_specials() {
        assert_eq!(csv_field("Bolt"), "Bolt");
        assert_eq!(
            csv_field("Breya, Etherium Shaper"),
            "\"Breya, Etherium Shaper\""
        );
        assert_eq!(csv_field("Say \"Hi\""), "\"Say \"\"Hi\"\"\"");
    }

    #[test]
    fn generic_and_store_parsing() {
        assert_eq!(Store::parse("generic"), Some(Store::Generic));
        assert_eq!(Store::parse("cardkingdom"), Some(Store::CardKingdom));
        assert_eq!(Store::parse("tcgplayer"), Some(Store::TCGPlayer));
        assert_eq!(Store::parse("nope"), None);
    }
}
