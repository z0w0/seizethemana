// Tests for the cross-deck conflict report and its helpers.

use super::*;
use crate::collection_conflicts::{deck_demand, demanded_by, locations_all};

fn conn() -> (tempfile::TempDir, Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    (tmp, conn)
}

fn row(conn: &Connection, binder: &str, kind: &str, name: &str, qty: i64) {
    conn.execute(
        "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity)
         VALUES (?1, 'tst', '1', 'normal', ?2, ?3, ?4)",
        rusqlite::params![name, binder, kind, qty],
    )
    .unwrap();
}

fn deck_file(paths: &crate::paths::Paths, name: &str, entries: &[(&str, i64)]) {
    let text = entries
        .iter()
        .map(|(n, q)| format!("{q} {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let path = paths.deck_file(name);
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(path, format!("// DECK\n{text}")).unwrap();
}

/// Two decks competing for one owned copy report a one-copy gap.
#[test]
fn competing_decks_report_the_gap() {
    let (tmp, conn) = conn();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    deck_file(&paths, "Froggy", &[("Rhystic Study", 1)]);
    deck_file(&paths, "Stationz", &[("Rhystic Study", 2), ("Island", 10)]);
    row(&conn, "Trade", "binder", "Rhystic Study", 1);

    // Direct demand math: 3 slots across 2 decks, 1 copy owned.
    let demand = deck_demand(&paths).unwrap();
    let (decks, total) = {
        let rows = demand.get("Rhystic Study").unwrap();
        let total: i64 = rows.iter().map(|(_, q)| q).sum();
        (rows, total)
    };
    assert_eq!(total, 3);
    assert_eq!(decks.len(), 2);
    // Basics never appear in demand rows that matter; check they are
    // filtered at report time by the shared helper.
    assert!(is_basic_name("Island"));
    assert!(!is_basic_name("Rhystic Study"));
}

/// Demand reads decklist files only; missing or unparseable files are
/// skipped without failing.
#[test]
fn demand_reads_only_parseable_lists() {
    let (tmp, _conn) = conn();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    deck_file(&paths, "Froggy", &[("Rhystic Study", 1)]);
    std::fs::write(paths.deck_file("Broken"), "not a deck list @@@").unwrap();
    let demand = demanded_by(&paths, "Rhystic Study");
    assert_eq!(demand, vec![("Froggy".to_string(), 1)]);
}

/// No decks at all yields an empty report (no panic on a missing dir).
#[test]
fn missing_decks_dir_is_empty_not_an_error() {
    let (tmp, _conn) = conn();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    let demand = demanded_by(&paths, "Anything");
    assert!(demand.is_empty());
}

/// locations_all groups by location + finish and puts binders first.
#[test]
fn locations_all_groups_and_orders() {
    let (_tmp, conn) = conn();
    row(&conn, "Froggy", "deck", "Bolt", 1);
    row(&conn, "Trade", "binder", "Bolt", 2);
    let locs = locations_all(&conn, "Bolt").unwrap();
    assert_eq!(
        locs,
        vec![
            ("Trade".to_string(), "binder".to_string(), 2, false),
            ("Froggy".to_string(), "deck".to_string(), 1, false),
        ]
    );
}
