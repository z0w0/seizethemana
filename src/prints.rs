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

impl PrintRange {
    /// The card's effective price: the normal-finish cheapest, or the
    /// foil cheapest when the normal finish is unpriced. None when
    /// unpriced.
    pub fn price(&self) -> Option<f64> {
        self.cheapest
            .as_ref()
            .and_then(|p| p.usd)
            .or_else(|| self.cheapest_foil.as_ref().and_then(|p| p.usd_foil))
    }
}

/// The human note for a price cap: one wording across every command.
pub fn price_cap_note(max_price: f64, hidden: usize) -> String {
    format!("candidates capped at ${max_price:.2} USD; {hidden} unpriced or above-cap cards hidden")
}

/// Names whose effective price (normal-finish cheapest, else cheapest
/// foil, released English printings) exists and is at or under the cap.
/// Unpriced cards are excluded (strict budget reading). This is the
/// SQL-side budget filter for search: the search can pre-drop
/// above-cap candidates before ranking.
pub fn names_under_price(conn: &Connection, max_price: f64) -> anyhow::Result<Vec<String>> {
    let today = crate::release::today();
    let mut out: Vec<String> = Vec::new();
    // Cheapest normal finish per name; foil-only cards fall back to
    // their cheapest foil printing.
    let mut normal = conn.prepare(
        "SELECT name, MIN(CAST(usd AS REAL)) FROM card_prints
         WHERE lang = 'en' AND (released_at = '' OR released_at <= ?1)
           AND usd IS NOT NULL
         GROUP BY name",
    )?;
    let mut rows = normal.query(rusqlite::params![today])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let price: Option<f64> = row.get(1)?;
        if price.is_some_and(|p| p <= max_price) {
            out.push(name);
        }
    }
    let mut foil = conn.prepare(
        "SELECT name, MIN(CAST(usd_foil AS REAL)) FROM card_prints
         WHERE lang = 'en' AND (released_at = '' OR released_at <= ?1)
           AND usd_foil IS NOT NULL
           AND name NOT IN (
               SELECT name FROM card_prints
               WHERE lang = 'en' AND (released_at = '' OR released_at <= ?2)
                 AND usd IS NOT NULL
           )
         GROUP BY name",
    )?;
    let mut rows = foil.query(rusqlite::params![today, today])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let price: Option<f64> = row.get(1)?;
        if price.is_some_and(|p| p <= max_price) {
            out.push(name);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Retain only the items whose keyed name passes the price cap.
/// Unpriced cards are excluded (strict budget reading). Returns the
/// filtered items and how many rows the cap hid. Errors propagate: a
/// failed price read must not silently hide every result.
pub fn retain_by_price<T>(
    conn: &Connection,
    items: Vec<T>,
    name_of: impl Fn(&T) -> &str,
    max_price: f64,
) -> anyhow::Result<(Vec<T>, usize)> {
    let names: Vec<String> = items.iter().map(|i| name_of(i).to_string()).collect();
    let ranges = price_ranges(conn, &names)?;
    let mut kept = Vec::new();
    let mut hidden = 0usize;
    for item in items {
        if ranges
            .get(name_of(&item))
            .and_then(PrintRange::price)
            .is_some_and(|p| p <= max_price)
        {
            kept.push(item);
        } else {
            hidden += 1;
        }
    }
    Ok((kept, hidden))
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

/// Price ranges for many card names in one query per finish kind.
///
/// Equivalent to calling [`price_range`] once per name (same cheapest/
/// priciest picks under the same filter), but as two SQL statements
/// instead of two per name, so deck views stop paying one statement set
/// per card.
///
/// # Errors
/// Propagates SQLite failures.
pub fn price_ranges(
    conn: &Connection,
    names: &[String],
) -> anyhow::Result<std::collections::HashMap<String, PrintRange>> {
    let mut map: std::collections::HashMap<String, PrintRange> = names
        .iter()
        .map(|n| (n.clone(), PrintRange::default()))
        .collect();
    if names.is_empty() {
        return Ok(map);
    }
    let today = crate::release::today();
    let chunk = 400;
    for finish in ["usd", "usd_foil"] {
        for names_chunk in names.chunks(chunk) {
            let placeholders = names_chunk
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(", ");
            // Two window functions rank cheapest and priciest per name under
            // the released-English filter; keeping rn_cheap = 1 or
            // rn_expensive = 1 yields the same picks `price_range` makes.
            let query = format!(
                "SELECT name, rn_cheap, rn_expensive, {PRINT_COLUMNS} FROM (
                    SELECT *, ROW_NUMBER() OVER (
                        PARTITION BY name ORDER BY {finish} ASC
                    ) AS rn_cheap,
                    ROW_NUMBER() OVER (
                        PARTITION BY name ORDER BY {finish} DESC
                    ) AS rn_expensive
                    FROM card_prints
                    WHERE name IN ({placeholders})
                      AND lang = 'en'
                      AND (released_at = '' OR released_at <= ?{n})
                      AND {finish} IS NOT NULL
                ) AS card_prints
                WHERE rn_cheap = 1 OR rn_expensive = 1
                ORDER BY name, rn_cheap",
                n = names_chunk.len() + 1
            );
            let mut stmt = conn.prepare(&query)?;
            let params: Vec<&dyn rusqlite::ToSql> = names_chunk
                .iter()
                .map(|n| n as &dyn rusqlite::ToSql)
                .chain(std::iter::once(&today as &dyn rusqlite::ToSql))
                .collect();
            let rows = stmt.query_map(params.as_slice(), map_batched_print)?;
            for row in rows {
                let (name, is_cheap, is_expensive, print) =
                    row.context("reading batched price rows")?;
                let entry = map.entry(name).or_default();
                let normal = finish == "usd";
                if is_cheap {
                    if normal {
                        entry.cheapest = Some(print.clone());
                    } else {
                        entry.cheapest_foil = Some(print.clone());
                    }
                }
                if is_expensive {
                    if normal {
                        entry.priciest = Some(print);
                    } else {
                        entry.priciest_foil = Some(print);
                    }
                }
            }
        }
    }
    Ok(map)
}

/// Map a batched row: `(name, is_cheapest, is_priciest, print)`.
fn map_batched_print(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, bool, bool, Print)> {
    let name: String = row.get(0)?;
    let cheap: i64 = row.get(1)?;
    let expensive: i64 = row.get(2)?;
    let print = map_print_offset(row, 3)?;
    Ok((name, cheap == 1, expensive == 1, print))
}

/// [`map_print`] reading the standard print columns from `offset`.
fn map_print_offset(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Print> {
    let g = |i: usize| -> rusqlite::Result<rusqlite::types::Value> { row.get(i + offset) };
    let s = |i: usize| -> rusqlite::Result<String> {
        match g(i)? {
            rusqlite::types::Value::Text(t) => Ok(t),
            _ => Ok(String::new()),
        }
    };
    let f = |i: usize| -> rusqlite::Result<Option<f64>> {
        Ok(match g(i)? {
            rusqlite::types::Value::Real(r) => Some(r),
            rusqlite::types::Value::Integer(i) => Some(i as f64),
            _ => None,
        })
    };
    Ok(Print {
        scryfall_id: s(0)?,
        name: s(1)?,
        set_code: s(2)?,
        collector_number: s(3)?,
        lang: s(4)?,
        finishes: serde_json::from_str(&s(5)?).unwrap_or_default(),
        released_at: s(6)?,
        usd: f(7)?,
        usd_foil: f(8)?,
        usd_etched: f(9)?,
        set_name: match g(10)? {
            rusqlite::types::Value::Text(t) => t,
            _ => String::new(),
        },
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
// Exercised by tests; reserved for per-print UIs.
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
    fn price_ranges_matches_price_range_per_name() {
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
        seed(
            &conn,
            "c",
            "Fog",
            "m11",
            "Magic 2011",
            "1",
            Some(1.0),
            None,
            "en",
            "2010-01-01",
        );
        // Unpriced name present in the batch: must come back as an empty
        // range, not a missing entry.
        let names = vec!["Bolt".to_string(), "Fog".to_string(), "Nope".to_string()];
        let ranges = price_ranges(&conn, &names).unwrap();
        assert_eq!(ranges.len(), 3);
        for name in &names {
            let batched = &ranges[name];
            let single = price_range(&conn, name).unwrap();
            fn id(p: &Print) -> &str {
                &p.scryfall_id
            }
            assert_eq!(
                batched.cheapest.as_ref().map(id),
                single.cheapest.as_ref().map(id),
                "{name} cheapest"
            );
            assert_eq!(
                batched.priciest.as_ref().map(id),
                single.priciest.as_ref().map(id),
                "{name} priciest"
            );
            assert_eq!(
                batched.cheapest_foil.as_ref().map(id),
                single.cheapest_foil.as_ref().map(id),
                "{name} cheapest_foil"
            );
            assert_eq!(
                batched.priciest_foil.as_ref().map(id),
                single.priciest_foil.as_ref().map(id),
                "{name} priciest_foil"
            );
        }
        // Empty input returns an empty map without touching SQLite.
        assert!(price_ranges(&conn, &[]).unwrap().is_empty());
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

#[cfg(test)]
mod max_price_tests {
    use super::*;

    fn conn() -> Connection {
        let tmp = tempfile::tempdir().unwrap();
        crate::db::open(&tmp.path().join("t.db")).unwrap()
    }

    fn seed(conn: &Connection, name: &str, usd: Option<f64>, usd_foil: Option<f64>) {
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
             VALUES (?1, ?1, 'm11', '148', 'en', 'rare', '[\"nonfoil\",\"foil\"]',
                '2020-01-01', ?2, ?3, 't')",
            rusqlite::params![name, usd, usd_foil],
        )
        .unwrap();
    }

    #[test]
    fn cap_excludes_above_cap_and_unpriced() {
        let conn = conn();
        seed(&conn, "Cheap", Some(1.0), None);
        seed(&conn, "Pricy", Some(9.0), None);
        seed(&conn, "Ghost", None, None);
        let names = vec!["Cheap".to_string(), "Pricy".into(), "Ghost".into()];
        let (kept, hidden) = retain_by_price(&conn, names, |n| n, 2.0).unwrap();
        assert_eq!(kept, vec!["Cheap".to_string()]);
        assert_eq!(hidden, 2, "the above-cap and unpriced cards are hidden");
    }

    #[test]
    fn cap_uses_foil_price_for_foil_only_rows() {
        let conn = conn();
        // A foil-only printing: the normal finish is unpriced, the foil
        // carries the price the filter must read.
        seed(&conn, "Foil Only", None, Some(1.5));
        let names = vec!["Foil Only".to_string()];
        let (kept, hidden) = retain_by_price(&conn, names, |n| n, 2.0).unwrap();
        assert_eq!(kept, vec!["Foil Only".to_string()]);
        assert_eq!(hidden, 0);
        // And the cap excludes it when the foil price busts the cap.
        let names = vec!["Foil Only".to_string()];
        let (kept, _) = retain_by_price(&conn, names, |n| n, 1.0).unwrap();
        assert!(kept.is_empty(), "foil price above the cap excludes");
    }

    #[test]
    fn names_under_price_matches_retain_by_price() {
        let conn = conn();
        seed(&conn, "Cheap", Some(1.0), None);
        seed(&conn, "Pricy", Some(9.0), None);
        seed(&conn, "Foil Only", None, Some(1.5));
        seed(&conn, "Ghost", None, None);
        let under = names_under_price(&conn, 2.0).unwrap();
        assert_eq!(under, vec!["Cheap".to_string(), "Foil Only".to_string()]);
        // A tighter cap drops the foil-only card too.
        let under = names_under_price(&conn, 1.0).unwrap();
        assert_eq!(under, vec!["Cheap".to_string()]);
    }

    #[test]
    fn price_cap_note_reads_one_way() {
        assert_eq!(
            price_cap_note(2.0, 3),
            "candidates capped at $2.00 USD; 3 unpriced or above-cap cards hidden"
        );
    }
}
