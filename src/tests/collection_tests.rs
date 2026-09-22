// Tests for collection import, stats, and owned search.

use super::*;
use crate::collection_stats::{Stats, color_key, compute_stats, stats_json};
use crate::output::Output;

fn csv_row(binder: &str, kind: &str, name: &str, qty: i64, foil: &str) -> CsvRow {
    CsvRow {
        binder: binder.into(),
        binder_type: if kind == "deck" {
            BinderType::Deck
        } else {
            BinderType::Binder
        },
        name: name.into(),
        set_code: "TST".into(),
        collector_number: "1".into(),
        foil: foil.into(),
        quantity: qty,
        purchase_price: qty as f64,
        scryfall_id: "sid".into(),
    }
}

fn write_csv(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
    let path = dir.join("collection.csv");
    std::fs::write(&path, contents).unwrap();
    path
}

const CSV_HEADER: &str = "Binder Name,Binder Type,Name,Set code,Set name,Collector number,Foil,Rarity,Quantity,ManaBox ID,Scryfall ID,Purchase price\n";

#[test]
fn parse_csv_handles_quotes_and_kinds() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write_csv(
        tmp.path(),
        &format!(
            "{CSV_HEADER}\
             Collect,binder,\"Lluwen, Imperfect\",TST,Test Set,1,normal,rare,2,1,sid1,0.5\n\
             Stationz,deck,Bolt,TST,Test Set,2,foil,rare,1,2,sid2,1.25\n\
             Wishlist,list,Ad Nauseam,2XM,X,76,normal,rare,1,3,sid3,22.38\n"
        ),
    );
    let (rows, skipped) = parse_csv(&path, &mut Output::new(true, false, false)).unwrap();
    assert_eq!(skipped, 1, "wishlist rows are counted as skipped");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "Lluwen, Imperfect");
    assert_eq!(rows[0].binder_type, BinderType::Binder);
    assert_eq!(rows[0].quantity, 2);
    // Per-copy 0.5 × qty 2 = the row total.
    assert!((rows[0].purchase_price - 1.0).abs() < 1e-9);
    assert_eq!(rows[1].binder, "Stationz");
    assert_eq!(rows[1].binder_type, BinderType::Deck);
    assert_eq!(rows[1].foil, "foil");
    assert!((rows[1].purchase_price - 1.25).abs() < 1e-9);
}

#[test]
fn parse_csv_skips_list_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write_csv(
        tmp.path(),
        &format!("{CSV_HEADER}Wishlist,list,X,TST,S,1,normal,c,1,1,sid,1.0\n"),
    );
    assert!(
        parse_csv(&path, &mut Output::new(true, false, false))
            .unwrap()
            .0
            .is_empty()
    );
}

#[test]
fn parse_csv_requires_headers() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write_csv(tmp.path(), "Name,Set code\nBolt,TST\n");
    assert!(parse_csv(&path, &mut Output::new(true, false, false)).is_err());
}

#[test]
fn aggregate_merges_duplicate_prints() {
    let rows = vec![
        csv_row("Collect", "binder", "Bolt", 1, "normal"),
        csv_row("Collect", "binder", "Bolt", 2, "normal"),
        csv_row("Collect", "binder", "Bolt", 1, "foil"),
        csv_row("Stationz", "deck", "Bolt", 1, "normal"),
    ];
    let merged = aggregate(&rows);
    assert_eq!(merged.len(), 3);
    let bolt = merged
        .iter()
        .find(|r| r.foil == "normal" && r.binder == "Collect")
        .unwrap();
    assert_eq!(bolt.quantity, 3);
    // Purchase prices are row totals; merges add them linearly (1 + 2 = 3).
    assert!((bolt.purchase_price - 3.0).abs() < 1e-9);
}

#[test]
fn color_key_sorts_wubrg_and_colorless() {
    assert_eq!(color_key(&["G".into(), "W".into()]), "WG");
    assert_eq!(color_key(&["U".into(), "B".into(), "R".into()]), "UBR");
    assert_eq!(color_key(&[]), "C");
}

#[test]
fn import_add_multiplies_purchase_price_by_quantity() {
    // ManaBox's Purchase price column is per copy: parse_csv turns each row
    // into a price × quantity total, so importing the same row twice with
    // --add must total 0.5 × 2 copies × 2 imports = 4.0, not price × rows.
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let csv = write_csv(
        tmp.path(),
        &format!("{CSV_HEADER}Collect,binder,Bolt,TST,Test,1,normal,common,2,1,sid1,0.5\n"),
    );
    let mut db = crate::db::open(&paths.db()).unwrap();
    db.execute(
        "INSERT INTO cards (name, oracle_id, scryfall_id) VALUES ('Bolt', 'oid', 'sid1')",
        [],
    )
    .unwrap();
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 1,
        embedded_cards: 1,
        model: "m".into(),
        dim: 384,
        names: vec!["Bolt".into()],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    import(&paths, &mut db, &mut out, &csv, false).unwrap();
    import(&paths, &mut db, &mut out, &csv, true).unwrap();
    let (qty, total): (i64, f64) = db
        .query_row(
            "SELECT quantity, purchase_price FROM collection WHERE name = 'Bolt'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(qty, 4);
    // 4 copies × 0.5 per copy = 2.0.
    assert!((total - 2.0).abs() < 1e-9);
}

#[test]
fn stats_color_identity_uses_identity_column() {
    // Command Tower has empty `colors` but WUBRG `color_identity`: the
    // census must bucket by identity, not read colorless.
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let csv = write_csv(
        tmp.path(),
        &format!(
            "{CSV_HEADER}Collect,binder,Command Tower,TST,Test,1,normal,common,2,1,sid1,0.5\n"
        ),
    );
    let mut db = crate::db::open(&paths.db()).unwrap();
    db.execute(
        "INSERT INTO cards (name, oracle_id, scryfall_id, colors, color_identity)
         VALUES ('Command Tower', 'oid', 'sid1', '[]', '[\"W\",\"U\",\"B\",\"R\",\"G\"]')",
        [],
    )
    .unwrap();
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 1,
        embedded_cards: 1,
        model: "m".into(),
        dim: 384,
        names: vec!["Command Tower".into()],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let code = import(&paths, &mut db, &mut out, &csv, false).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let stats = compute_stats(&db).unwrap();
    assert_eq!(stats.color_identity.get("WUBRG"), Some(&2));
    assert!(!stats.color_identity.contains_key("C"));
}

#[test]
fn stats_json_shape() {
    let stats = Stats {
        unique_cards: 2,
        total_cards: 5,
        foils: 1,
        total_value: 1.234,
        purchase_total: 0.5,
        ..Default::default()
    };
    let v = stats_json(&stats);
    assert_eq!(v["unique_cards"], 2);
    assert_eq!(v["total_cards"], 5);
    assert_eq!(v["total_value"], 1.23);
    assert_eq!(v["currency"], "USD");
}

#[test]
fn import_keeps_ownership_only_no_deck_files() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let csv = write_csv(
        tmp.path(),
        &format!(
            "{CSV_HEADER}\
             Stationz,deck,Infinite Guideline Station,EOE,Edge,348,foil,rare,1,1,sid1,2.0\n\
             Stationz,deck,Bolt,TST,Test,1,normal,common,3,2,sid2,0.5\n\
             Collect,binder,Bolt,TST,Test,2,normal,common,1,3,sid3,0.1\n\
             Wishlist,list,Ad Nauseam,2XM,X,76,normal,rare,1,4,sid4,22.0\n"
        ),
    );
    // Seed the oracle so names resolve.
    let mut db = crate::db::open(&paths.db()).unwrap();
    for (name, sid) in [
        ("Infinite Guideline Station", "sid1"),
        ("Bolt", "sid2"),
        ("Ad Nauseam", "sid4"),
    ] {
        db.execute(
            "INSERT INTO cards (name, oracle_id, scryfall_id) VALUES (?1, 'oid', ?2)",
            rusqlite::params![name, sid],
        )
        .unwrap();
    }
    // status.json must claim setup so the import proceeds.
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 2,
        embedded_cards: 2,
        model: "m".into(),
        dim: 384,
        names: vec!["Bolt".into()],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();

    let mut out = crate::output::Output::new(true, false, false);
    let code = import(&paths, &mut db, &mut out, &csv, false).unwrap();
    assert_eq!(code, crate::cli::codes::OK);

    // Ownership rows land in the collection (deck assignments count).
    let deck_copies: i64 = db
        .query_row(
            "SELECT COALESCE(SUM(quantity), 0) FROM collection WHERE binder_type = 'deck'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(deck_copies, 4);
    // Wishlist rows are skipped.
    let list_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM collection WHERE binder = 'Wishlist'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(list_rows, 0);
    // No deck txt or primer is written by the collection import.
    assert!(!paths.deck_file("Stationz").exists());
    assert!(!crate::deck::store::primer_file(&paths, "Stationz").exists());
}

#[test]
fn import_notes_skip_existing_decklists() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let csv = write_csv(
        tmp.path(),
        &format!("{CSV_HEADER}Stationz,deck,Bolt,TST,Test,1,normal,common,3,2,sid2,0.5\n"),
    );
    let mut db = crate::db::open(&paths.db()).unwrap();
    db.execute(
        "INSERT INTO cards (name, oracle_id, scryfall_id) VALUES ('Bolt', 'oid', 'sid2')",
        [],
    )
    .unwrap();
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 1,
        embedded_cards: 1,
        model: "m".into(),
        dim: 384,
        names: vec!["Bolt".into()],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    // A decklist already exists for this deck: the second note is skipped.
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Stationz"), "// DECK\n3 Bolt\n").unwrap();

    let mut out = crate::output::Output::new(false, true, false);
    let code = import(&paths, &mut db, &mut out, &csv, false).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // The existing decklist is untouched (not rewritten, not deleted).
    let deck =
        crate::deck::Deck::parse(&std::fs::read_to_string(paths.deck_file("Stationz")).unwrap())
            .unwrap();
    assert_eq!(deck.total(), 3);
}

#[test]
fn by_universe_rolls_up_cards_and_value() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let csv = write_csv(
        tmp.path(),
        &format!(
            "{CSV_HEADER}\
             Collect,binder,Bolt,TST,Test,1,normal,common,2,3,sid2,0.5\n\
             Collect,binder,Iron Man,MSH,Marvel,58,normal,rare,1,4,sid5,3.0\n"
        ),
    );
    let mut db = crate::db::open(&paths.db()).unwrap();
    for (name, sid) in [("Bolt", "sid2"), ("Iron Man", "sid5")] {
        db.execute(
            "INSERT INTO cards (name, oracle_id, scryfall_id) VALUES (?1, 'oid', ?2)",
            rusqlite::params![name, sid],
        )
        .unwrap();
    }
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 2,
        embedded_cards: 2,
        model: "m".into(),
        dim: 384,
        names: vec!["Bolt".into()],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    // Set metadata: msh is a UB Marvel set, tst is in-universe.
    db.execute(
        "INSERT INTO sets (set_code, set_name, set_type, franchise)
         VALUES ('tst', 'Test Set', 'expansion', NULL),
                ('msh', 'Marvel Super Heroes', 'expansion', 'Marvel')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO card_prints (scryfall_id, name, set_code, universes_beyond, updated_at)
         VALUES ('sid2', 'Bolt', 'tst', 0, 't'), ('sid5', 'Iron Man', 'msh', 1, 't')",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE card_prints SET collector_number = '1' WHERE scryfall_id = 'sid2'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE card_prints SET collector_number = '58' WHERE scryfall_id = 'sid5'",
        [],
    )
    .unwrap();
    // Owned-print prices: Bolt $1, Iron Man $3 (foil).
    db.execute(
        "UPDATE card_prints SET usd = 1.0 WHERE scryfall_id = 'sid2'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE card_prints SET usd = 3.0 WHERE scryfall_id = 'sid5'",
        [],
    )
    .unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let code = import(&paths, &mut db, &mut out, &csv, false).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let stats = compute_stats(&db).unwrap();
    let multi = &stats.by_universe["multiverse"];
    let beyond = &stats.by_universe["beyond"];
    assert_eq!(multi.cards, 2, "2 copies of the in-universe card");
    assert_eq!(beyond.cards, 1, "1 copy of the UB card");
    assert!(multi.value > 0.0, "multiverse value rolls up from prices");
    assert!(beyond.value > 0.0, "beyond value rolls up from prices");
    assert!(stats.by_franchise.contains_key("Marvel"));
}
