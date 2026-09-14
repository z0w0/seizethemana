use anyhow::Context;
use rusqlite::Connection;

// Print storage and lookups.
//
// Per-printing price and print info live in `card_prints` (one row per
// physical printing), harvested from Scryfall's default-cards bulk by the
// sync pass in `crate::sync`. This module owns the read queries: cheapest /
// most expensive printing per card name, and batched print lookups. Set
// codes are lowercase everywhere in this store.

/// One physical printing with its price snapshot.
///
/// `set_name` comes from the `sets` table and can be empty when the set
/// code is unknown to this snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Print {
    pub scryfall_id: String,
    pub name: String,
    pub set_code: String,
    pub set_name: String,
    pub collector_number: String,
    pub lang: String,
    pub finishes: Vec<String>,
    pub released_at: String,
    pub usd: Option<f64>,
    pub usd_foil: Option<f64>,
    pub usd_etched: Option<f64>,
}

/// Cheapest and most expensive printing of one card, per finish kind.
#[derive(Debug, Clone, Default)]
pub struct PrintRange {
    /// Lowest-priced released English printing (normal finish), if priced.
    pub cheapest: Option<Print>,
    /// Highest-priced released English printing (normal finish), if priced.
    pub priciest: Option<Print>,
    /// Same, for foil.
    pub cheapest_foil: Option<Print>,
    /// Highest-priced released English foil printing.
    pub priciest_foil: Option<Print>,
}

const PRINT_COLUMNS: &str = "scryfall_id, name, set_code, collector_number, lang, finishes, released_at, usd, usd_foil, usd_etched, (SELECT set_name FROM sets WHERE sets.set_code = card_prints.set_code)";

fn map_print(row: &rusqlite::Row<'_>) -> rusqlite::Result<Print> {
    let finishes_raw: String = row.get(5)?;
    Ok(Print {
        scryfall_id: row.get(0)?,
        name: row.get(1)?,
        set_code: row.get(2)?,
        collector_number: row.get(3)?,
        lang: row.get(4)?,
        finishes: serde_json::from_str(&finishes_raw).unwrap_or_default(),
        released_at: row.get(6)?,
        usd: row.get(7)?,
        usd_foil: row.get(8)?,
        usd_etched: row.get(9)?,
        // The subquery is NULL when the set is missing from the `sets`
        // table; fall back to empty (never fail the whole print).
        set_name: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
    })
}

/// USD price for a print matching a finish kind (`normal`, `foil`, `etched`).
///
/// Falls back toward the plain price when the exact kind is unpriced, so a
/// foil entry still gets a usable estimate.
pub fn price_for_kind(print: &Print, foil: &str) -> Option<f64> {
    match foil {
        "foil" => print.usd_foil.or(print.usd),
        "etched" => print.usd_etched.or(print.usd_foil).or(print.usd),
        _ => print.usd,
    }
}

/// Read the released-English-prints filter: buyable/sellable only.
use rusqlite::OptionalExtension;

/// Cheapest and most expensive released English printings of one card name.
///
/// A card with no priced prints (e.g. only unreleased reprints in the bulk)
/// yields an empty range.
///
/// # Errors
/// Propagates SQLite failures.
pub fn price_range(conn: &Connection, name: &str) -> anyhow::Result<PrintRange> {
    let today = crate::release::today();
    let pick = |foil: bool| -> anyhow::Result<(Option<Print>, Option<Print>)> {
        let price_col = if foil { "usd_foil" } else { "usd" };
        let query = format!(
            "SELECT {PRINT_COLUMNS} FROM card_prints
             WHERE name = ?1 AND lang = 'en'
               AND (released_at = '' OR released_at <= ?2)
               AND {price_col} IS NOT NULL
             ORDER BY card_prints.{price_col} ASC LIMIT 1"
        );
        let cheapest = conn
            .prepare(&query)?
            .query_row(rusqlite::params![name, today], map_print)
            .ok();
        let query = format!(
            "SELECT {PRINT_COLUMNS} FROM card_prints
             WHERE name = ?1 AND lang = 'en'
               AND (released_at = '' OR released_at <= ?2)
               AND {price_col} IS NOT NULL
             ORDER BY card_prints.{price_col} DESC LIMIT 1"
        );
        let priciest = conn
            .prepare(&query)?
            .query_row(rusqlite::params![name, today], map_print)
            .ok();
        Ok((cheapest, priciest))
    };
    let (cheapest, priciest) = pick(false)?;
    let (cheapest_foil, priciest_foil) = pick(true)?;
    Ok(PrintRange {
        cheapest,
        priciest,
        cheapest_foil,
        priciest_foil,
    })
}

/// Prints for many card names at once (map keyed by card name).
///
/// Prints of any language or release state are returned; callers that need
/// buyable prints filter on `lang`/`released_at`. Set names resolve through
/// the `sets` table and can be absent (empty) for unknown codes.
///
/// # Errors
/// Propagates SQLite failures.
#[cfg_attr(not(test), expect(dead_code))] // exercised by tests; reserved for per-print UIs
pub fn prints_by_name(
    conn: &Connection,
    names: &[String],
) -> anyhow::Result<std::collections::HashMap<String, Vec<Print>>> {
    let mut map: std::collections::HashMap<String, Vec<Print>> = std::collections::HashMap::new();
    if names.is_empty() {
        return Ok(map);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT {PRINT_COLUMNS} FROM card_prints
         WHERE name = ?1 ORDER BY set_code, collector_number"
    ))?;
    for name in names {
        let rows = stmt
            .query_map([name], map_print)?
            .collect::<Result<Vec<_>, _>>()
            .context("reading prints")?;
        map.insert(name.clone(), rows);
    }
    Ok(map)
}

/// Price of one owned collection row: exact print by (name, set, cn) with a
/// finish-kind fallback.
///
/// Returns None when the snapshot lacks the printing or the print is
/// unpriced for the requested finish.
///
/// # Errors
/// Propagates SQLite failures.
pub fn price_for_owned(
    conn: &Connection,
    name: &str,
    set_code: &str,
    collector_number: &str,
    foil: &str,
) -> anyhow::Result<Option<f64>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {PRINT_COLUMNS} FROM card_prints
             WHERE name = ?1 AND set_code = ?2 AND collector_number = ?3
             LIMIT 1"
            ),
            rusqlite::params![name, set_code.to_ascii_lowercase(), collector_number],
            |row| {
                let print = map_print(row)?;
                Ok(price_for_kind(&print, foil))
            },
        )
        .optional()
        .context("reading owned print price")?
        .flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let tmp = tempfile::tempdir().unwrap();
        crate::db::open(&tmp.path().join("t.db")).unwrap()
    }

    /// Seed a print row directly (same shape the sync upsert writes).
    #[cfg(test)]
    #[expect(clippy::too_many_arguments)]
    fn seed(
        conn: &Connection,
        id: &str,
        name: &str,
        set: &str,
        set_name: &str,
        cn: &str,
        usd: Option<f64>,
        usd_foil: Option<f64>,
        lang: &str,
        released_at: &str,
    ) {
        conn.execute(
            "INSERT OR REPLACE INTO sets (set_code, set_name) VALUES (?1, ?2)",
            rusqlite::params![set, set_name],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'rare', '[\"nonfoil\"]', ?6, ?7, ?8, 't')",
            rusqlite::params![id, name, set, cn, lang, released_at, usd, usd_foil],
        )
        .unwrap();
    }

    #[test]
    fn price_range_picks_cheapest_and_priciest() {
        let conn = conn();
        seed(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            Some(0.5),
            Some(9.0),
            "en",
            "2010-01-01",
        );
        seed(
            &conn,
            "b",
            "Bolt",
            "2xm",
            "Double Masters",
            "100",
            Some(2.5),
            Some(4.0),
            "en",
            "2020-01-01",
        );
        let range = price_range(&conn, "Bolt").unwrap();
        assert_eq!(range.cheapest.as_ref().unwrap().set_code, "m11");
        assert_eq!(range.cheapest.as_ref().unwrap().usd, Some(0.5));
        assert_eq!(range.priciest.as_ref().unwrap().set_code, "2xm");
        assert_eq!(range.priciest_foil.as_ref().unwrap().usd_foil, Some(9.0));
        // Set name resolves through the sets table.
        assert_eq!(range.cheapest.as_ref().unwrap().set_name, "Magic 2011");
    }

    #[test]
    fn price_range_excludes_unpriced_unreleased_and_foreign() {
        let conn = conn();
        seed(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            None,
            None,
            "en",
            "2010-01-01",
        );
        // Unreleased reprint: excluded from picks.
        seed(
            &conn,
            "b",
            "Bolt",
            "trk",
            "Upcoming",
            "1",
            Some(0.01),
            None,
            "en",
            "2099-01-01",
        );
        // Japanese printing: excluded from picks.
        seed(
            &conn,
            "c",
            "Bolt",
            "m11",
            "Magic 2011",
            "148j",
            Some(0.02),
            None,
            "ja",
            "2010-01-01",
        );
        // Only an unpriced print of another card.
        seed(
            &conn,
            "d",
            "Fog",
            "m11",
            "Magic 2011",
            "1",
            None,
            None,
            "en",
            "2010-01-01",
        );

        let range = price_range(&conn, "Bolt").unwrap();
        assert!(range.cheapest.is_none());
        assert!(range.priciest.is_none());
        assert!(price_range(&conn, "Fog").unwrap().cheapest.is_none());
        assert!(price_range(&conn, "Nope").unwrap().cheapest.is_none());
    }

    #[test]
    fn price_for_kind_matches_finish() {
        let p = Print {
            scryfall_id: "x".into(),
            name: "Bolt".into(),
            set_code: "m11".into(),
            set_name: "Magic 2011".into(),
            collector_number: "148".into(),
            lang: "en".into(),
            finishes: vec!["foil".into()],
            released_at: "2010-01-01".into(),
            usd: Some(1.0),
            usd_foil: Some(4.0),
            usd_etched: None,
        };
        assert_eq!(price_for_kind(&p, "normal"), Some(1.0));
        assert_eq!(price_for_kind(&p, "foil"), Some(4.0));
        // Etched with no etched price falls back to foil, then normal.
        assert_eq!(price_for_kind(&p, "etched"), Some(4.0));
        let bare = Print {
            usd: None,
            usd_foil: None,
            usd_etched: None,
            ..p
        };
        assert_eq!(price_for_kind(&bare, "normal"), None);
    }

    #[test]
    fn price_for_owned_joins_exact_print() {
        let conn = conn();
        seed(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            Some(0.5),
            Some(9.0),
            "en",
            "2010-01-01",
        );
        seed(
            &conn,
            "b",
            "Bolt",
            "2xm",
            "Double Masters",
            "100",
            Some(2.0),
            None,
            "en",
            "2020-01-01",
        );
        let normal = price_for_owned(&conn, "Bolt", "m11", "148", "normal").unwrap();
        assert_eq!(normal, Some(0.5));
        // Case-insensitive on the set code (CSV exports uppercase).
        let upper = price_for_owned(&conn, "Bolt", "M11", "148", "foil").unwrap();
        assert_eq!(upper, Some(9.0));
        // Wrong collector number → no row.
        let missing = price_for_owned(&conn, "Bolt", "m11", "999", "normal").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn prints_by_name_groups_and_orders() {
        let conn = conn();
        seed(
            &conn,
            "b",
            "Bolt",
            "2xm",
            "Double Masters",
            "100",
            Some(2.5),
            None,
            "en",
            "2020-01-01",
        );
        seed(
            &conn,
            "a",
            "Bolt",
            "m11",
            "Magic 2011",
            "148",
            Some(0.5),
            Some(9.0),
            "en",
            "2010-01-01",
        );
        let map = prints_by_name(&conn, &["Bolt".into(), "Nope".into()]).unwrap();
        let prints = &map["Bolt"];
        assert_eq!(prints.len(), 2);
        assert_eq!(prints[0].set_code, "2xm");
        assert_eq!(prints[1].set_code, "m11");
        assert!(map["Nope"].is_empty());
    }
}
