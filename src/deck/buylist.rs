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
use super::ownership::available_map;

/// Missing rows with the available-copy map already computed.
///
/// Same-name entries across sections aggregate; basic lands are excluded
/// (unlimited supply); foil entries prefer the cheapest foil printing.
/// Returns rows sorted by name (BTreeMap order), or empty when nothing is
/// missing.
pub(super) fn missing_rows(
    conn: &Connection,
    deck: &super::Deck,
    cards_by_name: &std::collections::HashMap<String, crate::db::CardRow>,
    available: &std::collections::HashMap<String, i64>,
) -> anyhow::Result<Vec<BuylistRow>> {
    let mut needed: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut wants_foil: std::collections::BTreeMap<String, bool> =
        std::collections::BTreeMap::new();
    let mut sections: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (section, entries) in &deck.sections {
        for entry in entries {
            let Some(card) = cards_by_name.get(&entry.name) else {
                continue;
            };
            if super::stats::is_basic_land(card) {
                continue;
            }
            *needed.entry(entry.name.clone()).or_insert(0) += entry.quantity;
            wants_foil
                .entry(entry.name.clone())
                .and_modify(|f| *f = *f || entry.foil)
                .or_insert(entry.foil);
            let seen = sections.entry(entry.name.clone()).or_default();
            if !seen.contains(section) {
                seen.push(section.clone());
            }
        }
    }
    let mut rows = Vec::new();
    let missing_names: Vec<String> = needed
        .iter()
        .filter(|(name, qty)| {
            let owned = available.get(*name).copied().unwrap_or(0);
            **qty - owned > 0
        })
        .map(|(name, _)| name.clone())
        .collect();
    // One batched query per finish kind instead of four per card name.
    let ranges = crate::prints::price_ranges(conn, &missing_names)?;
    for (name, qty) in &needed {
        let owned = available.get(name).copied().unwrap_or(0);
        let missing = qty - owned;
        if missing <= 0 {
            continue;
        }
        let range = match ranges.get(name) {
            Some(range) => range,
            None => continue,
        };
        let print = if wants_foil[name] {
            range
                .cheapest_foil
                .clone()
                .or_else(|| range.cheapest.clone())
        } else {
            range.cheapest.clone()
        };
        let Some(print) = print else {
            // No released English printing priced in the snapshot; skip with
            // no line rather than a zero-priced line.
            continue;
        };
        rows.push(BuylistRow {
            name: name.clone(),
            set_code: print.set_code,
            set_name: Some(print.set_name).filter(|s| !s.is_empty()),
            collector_number: print.collector_number,
            scryfall_id: print.scryfall_id,
            foil: wants_foil[name],
            quantity: missing,
            sections: sections.get(name).cloned().unwrap_or_default(),
            price_usd: if wants_foil[name] {
                print.usd_foil.or(print.usd)
            } else {
                print.usd
            },
        });
    }
    Ok(rows)
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
    let cards_by_name = super::stats::lookup_names(conn, &deck);
    let available = available_map(conn, name)?;
    let rows = missing_rows(conn, &deck, &cards_by_name, &available)?;
    if rows.is_empty() {
        out.error("no missing cards; the deck is fully covered");
        out.hint("check 'stm deck show' for the ownership breakdown");
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
    if json {
        let v = serde_json::json!({
            "store": store.name(),
            "rows": rows.iter().map(|r| serde_json::json!({
                "name": r.name,
                "set": r.set_code,
                "set_name": r.set_name,
                "collector_number": r.collector_number,
                "scryfall_id": r.scryfall_id,
                "foil": r.foil,
                "quantity": r.quantity,
                "sections": r.sections,
                "price_usd": r.price_usd,
            })).collect::<Vec<_>>(),
            "total_usd": round2(total),
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
                    row_rarity(conn, &row.scryfall_id),
                ),
            }
        }
        out.status(
            "Total",
            &format!(
                "{} missing copies, est. market {}",
                rows.iter().map(|r| r.quantity).sum::<i64>(),
                out.styles().money(total)
            ),
        );
    }
    Ok(crate::cli::codes::OK)
}

/// Round to two decimals for money output.
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Rarity of one print (for the TCGPlayer Mass Entry column).
fn row_rarity(conn: &Connection, scryfall_id: &str) -> String {
    conn.query_row(
        "SELECT rarity FROM card_prints WHERE scryfall_id = ?1",
        [scryfall_id],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_default()
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
        let cards_by_name = super::super::stats::lookup_names(&conn, &deck);
        let available = available_map(&conn, "Buy").unwrap();
        assert_eq!(available.get("Bolt"), Some(&5), "own deck + binder only");
        let rows = missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
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
        let cards_by_name = super::super::stats::lookup_names(&conn, &deck);
        let available = available_map(&conn, "Deck").unwrap();
        let rows = missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
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
        let cards_by_name = super::super::stats::lookup_names(&conn, &deck);
        let available = available_map(&conn, "Deck").unwrap();
        let rows = missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
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
        let cards_by_name = super::super::stats::lookup_names(&conn, &deck);
        let available = available_map(&conn, "Deck").unwrap();
        let rows = missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
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
