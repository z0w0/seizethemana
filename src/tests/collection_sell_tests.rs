// Tests for the `stm collection sell` dead-money report.

use super::*;
use crate::paths::{Paths, Status};

fn setup() -> (tempfile::TempDir, Paths, Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::new(tmp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    Status {
        setup_complete: true,
        ingested_cards: 1,
        embedded_cards: 1,
        model: "m".into(),
        dim: 384,
        names: Vec::new(),
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    (tmp, paths, conn)
}

/// Seed a card row with the sell-relevant oracle signals.
#[allow(clippy::too_many_arguments)]
fn seed_card(
    conn: &Connection,
    name: &str,
    rarity: &str,
    edhrec: Option<i64>,
    penny: Option<i64>,
    reserved: Option<bool>,
    game_changer: Option<bool>,
    legalities: &str,
) {
    conn.execute(
        "INSERT INTO cards (name, oracle_id, rarity, edhrec_rank, penny_rank,
            reserved, game_changer, legalities, set_code, collector_number)
         VALUES (?1, 'oid', ?2, ?3, ?4, ?5, ?6, ?7, 'tst', '1')",
        rusqlite::params![
            name,
            rarity,
            edhrec,
            penny,
            reserved,
            game_changer,
            legalities
        ],
    )
    .unwrap();
}

/// Seed one released English print at the given printing key and price.
fn seed_print(conn: &Connection, name: &str, set: &str, cn: &str, foil: &str, usd: f64) {
    conn.execute(
        "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
            lang, rarity, finishes, released_at, usd, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'en', 'rare', '[\"nonfoil\"]', '2020-01-01', ?5, 't')",
        rusqlite::params![format!("sid-{name}-{set}-{cn}-{foil}"), name, set, cn, usd],
    )
    .unwrap();
}

/// One binder row for a specific printing.
fn seed_binder(conn: &Connection, name: &str, set: &str, cn: &str, foil: &str, qty: i64) {
    conn.execute(
        "INSERT INTO collection (name, set_code, collector_number, foil, binder,
            binder_type, quantity)
         VALUES (?1, ?2, ?3, ?4, 'Collect', 'binder', ?5)",
        rusqlite::params![name, set, cn, foil, qty],
    )
    .unwrap();
}

/// One deck assignment row (spoken for).
fn seed_deck_row(conn: &Connection, name: &str, qty: i64) {
    conn.execute(
        "INSERT INTO collection (name, set_code, collector_number, foil, binder,
            binder_type, quantity)
         VALUES (?1, 'tst', '1', 'normal', 'Froggy', 'deck', ?2)",
        rusqlite::params![name, qty],
    )
    .unwrap();
}

/// Write a decklist demanding one copy of each name.
fn deck_demanding(paths: &Paths, deck: &str, names: &[&str]) {
    let body = names
        .iter()
        .map(|n| format!("1 {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(paths.deck_file(deck), format!("// DECK\n{body}")).unwrap();
}

/// Build the typed report the way `run` does, without capturing stdout.
fn sell_report(paths: &Paths, conn: &Connection, options: &SellOptions<'_>) -> Option<SellReport> {
    let rows = load_binder_rows(conn).unwrap();
    let demand = crate::collection_conflicts::deck_demand(paths).unwrap();
    build_report(conn, rows, &demand, options).unwrap()
}

/// Run the public entry point, discarding output; returns the exit code.
fn run_sell(paths: &Paths, conn: &Connection, options: &SellOptions<'_>) -> anyhow::Result<i32> {
    let mut out = crate::output::Output::new(true, false, false);
    run(paths, conn, &mut out, options, true)
}

/// Seed a simple one-print, one-binder-copy card.
fn simple_card(conn: &Connection, name: &str, rarity: &str, rank: i64, usd: f64) {
    seed_card(
        conn,
        name,
        rarity,
        Some(rank),
        None,
        Some(false),
        None,
        "{}",
    );
    seed_print(conn, name, "tst", "1", "normal", usd);
    seed_binder(conn, name, "tst", "1", "normal", 1);
}

/// Cards assigned to a deck — or wanted by a decklist — never appear.
#[test]
fn deck_cards_are_never_suggested() {
    let (_tmp, paths, conn) = setup();
    // Owned only inside a deck: never a candidate.
    seed_card(
        &conn,
        "Spoken",
        "rare",
        Some(20_000),
        None,
        Some(false),
        None,
        "{}",
    );
    seed_print(&conn, "Spoken", "tst", "1", "normal", 10.0);
    seed_deck_row(&conn, "Spoken", 1);
    // Binder copy of a card a decklist wants: also excluded.
    simple_card(&conn, "Wanted", "rare", 20_000, 10.0);
    deck_demanding(&paths, "Froggy", &["Wanted"]);
    // No deck wants this one, so it is a candidate.
    simple_card(&conn, "Free", "rare", 20_000, 10.0);

    let report = sell_report(&paths, &conn, &SellOptions::default()).expect("candidates exist");
    let names: Vec<&str> = report.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Free"],
        "decked and deck-wanted cards are excluded"
    );
}

/// Basics are unlimited and never candidates.
#[test]
fn basics_are_excluded() {
    let (_tmp, paths, conn) = setup();
    simple_card(&conn, "Island", "common", 20_000, 2.0);
    assert_eq!(
        run_sell(&paths, &conn, &SellOptions::default()).unwrap(),
        crate::cli::codes::NO_RESULTS
    );
}

/// Reasons and hold warnings reflect the oracle signals.
#[test]
fn reasons_and_hold_warnings() {
    let (_tmp, _paths, _conn) = setup();
    let unplayed = Candidate {
        name: "Dead".into(),
        rarity: "rare".into(),
        edhrec_rank: None,
        penny_rank: None,
        reserved: false,
        game_changer: false,
        legalities: "{}".into(),
        oracle_known: true,
        prints: Vec::new(),
        owned_binder: 1,
        total_value: 8.0,
        sellable_value: 8.0,
        bulk_value: 0.0,
        max_price: Some(8.0),
    };
    let row = build_row(&unplayed, DEFAULT_RANK_FLOOR, Some("modern"));
    assert!(row.reasons.contains(&"not_in_deck"));
    assert!(row.reasons.contains(&"rarely_played"));
    assert!(row.reasons.contains(&"reprint_risk"));
    assert!(row.reasons.contains(&"format_unplayed"));
    assert!(row.hold_warnings.is_empty());
    assert_eq!(row.sell_confidence, 1.0);

    let reserved_gc = Candidate {
        name: "Precious".into(),
        edhrec_rank: Some(10),
        reserved: true,
        game_changer: true,
        legalities: r#"{"modern":"legal"}"#.into(),
        ..unplayed
    };
    let row = build_row(&reserved_gc, DEFAULT_RANK_FLOOR, Some("modern"));
    assert!(!row.reasons.contains(&"rarely_played"), "rank 10 is played");
    assert!(
        !row.reasons.contains(&"reprint_risk"),
        "reserved has no reprint risk"
    );
    assert!(!row.reasons.contains(&"format_unplayed"));
    assert_eq!(row.hold_warnings, vec!["reserved_list", "game_changer"]);
    assert_eq!(row.sell_confidence, 0.5);
}

/// An unresolved card (absent from the oracle) is never flagged
/// `format_unplayed`: its legality is unknown, not unplayed.
#[test]
fn unresolved_card_is_not_format_unplayed() {
    let (_tmp, _paths, _conn) = setup();
    let unknown = Candidate {
        name: "Ghost".into(),
        rarity: String::new(),
        edhrec_rank: None,
        penny_rank: None,
        reserved: false,
        game_changer: false,
        legalities: String::new(),
        oracle_known: false,
        prints: Vec::new(),
        owned_binder: 1,
        total_value: 0.0,
        sellable_value: 0.0,
        bulk_value: 0.0,
        max_price: None,
    };
    let row = build_row(&unknown, DEFAULT_RANK_FLOOR, Some("modern"));
    assert!(!row.reasons.contains(&"format_unplayed"));
}

/// Play bands follow the EDHREC rank, with Penny rank as a fallback lift.
#[test]
fn play_band_classification() {
    assert_eq!(play_band(Some(100), None, DEFAULT_RANK_FLOOR), BAND_PLAYED);
    assert_eq!(
        play_band(Some(10_000), None, DEFAULT_RANK_FLOOR),
        BAND_NICHE
    );
    assert_eq!(
        play_band(Some(20_000), None, DEFAULT_RANK_FLOOR),
        BAND_UNPLAYED
    );
    assert_eq!(play_band(None, Some(5_000), DEFAULT_RANK_FLOOR), BAND_NICHE);
    assert_eq!(play_band(None, None, DEFAULT_RANK_FLOOR), BAND_UNPLAYED);
    // The floor is tunable.
    assert_eq!(play_band(Some(10_000), None, 8_000), BAND_UNPLAYED);
}

/// Dead money ranks by value; a $40 unplayed card beats a $1 one.
#[test]
fn high_value_unplayed_ranks_first() {
    let (_tmp, paths, conn) = setup();
    simple_card(&conn, "Big", "rare", 25_000, 40.0);
    simple_card(&conn, "Small", "rare", 25_000, 1.0);
    let report = sell_report(&paths, &conn, &SellOptions::default()).expect("candidates");
    assert_eq!(report.rows[0].name, "Big");
    assert_eq!(report.rows[1].name, "Small");
}

/// The bulk floor separates boxful money from singles money at the report
/// level.
#[test]
fn bulk_and_singles_values_are_split() {
    let (_tmp, paths, conn) = setup();
    simple_card(&conn, "Chase", "rare", 25_000, 12.0);
    seed_card(
        &conn,
        "Bulk",
        "common",
        Some(25_000),
        None,
        Some(false),
        None,
        "{}",
    );
    seed_print(&conn, "Bulk", "tst", "1", "normal", 0.10);
    seed_binder(&conn, "Bulk", "tst", "1", "normal", 10);

    let report = sell_report(&paths, &conn, &SellOptions::default()).expect("candidates");
    assert!(
        (report.sellable_binder_value - 12.0).abs() < 1e-9,
        "Chase only"
    );
    assert!((report.bulk_value - 1.0).abs() < 1e-9, "10 × $0.10");
}

/// A card owning both an expensive print and cheap prints reports each side
/// of its value from the real printings, not a single per-copy max.
#[test]
fn mixed_printings_split_per_print() {
    let (_tmp, paths, conn) = setup();
    seed_card(
        &conn,
        "Mixed",
        "rare",
        Some(25_000),
        None,
        Some(false),
        None,
        "{}",
    );
    // One foil @ $3.00 and ten normal @ $0.10.
    seed_print(&conn, "Mixed", "old", "7", "foil", 3.0);
    seed_print(&conn, "Mixed", "new", "9", "normal", 0.10);
    seed_binder(&conn, "Mixed", "old", "7", "foil", 1);
    seed_binder(&conn, "Mixed", "new", "9", "normal", 10);

    let report = sell_report(&paths, &conn, &SellOptions::default()).expect("candidates");
    let row = &report.rows[0];
    assert_eq!(row.owned_binder, 11);
    assert!((row.sellable_value - 3.0).abs() < 1e-9, "the $3 foil");
    assert!((row.bulk_value - 1.0).abs() < 1e-9, "10 × $0.10");
    assert!((row.total_value - 4.0).abs() < 1e-9);
    assert_eq!(row.printings.len(), 2);
    assert_eq!(row.max_price, Some(3.0));
    // Both value streams surface at the top level too.
    assert!((report.sellable_binder_value - 3.0).abs() < 1e-9);
    assert!((report.bulk_value - 1.0).abs() < 1e-9);
}

/// Reprint risk is a per-copy property: a stack of bulk is not at risk, but a
/// single valuable copy is.
#[test]
fn reprint_risk_is_per_copy() {
    let (_tmp, paths, conn) = setup();
    seed_card(
        &conn,
        "Heap",
        "common",
        Some(25_000),
        None,
        Some(false),
        None,
        "{}",
    );
    seed_print(&conn, "Heap", "tst", "1", "normal", 0.10);
    seed_binder(&conn, "Heap", "tst", "1", "normal", 100);
    simple_card(&conn, "Gem", "rare", 25_000, 5.0);

    let report = sell_report(&paths, &conn, &SellOptions::default()).expect("candidates");
    let heap = report.rows.iter().find(|r| r.name == "Heap").unwrap();
    assert!(
        !heap.reasons.contains(&"reprint_risk"),
        "100 bulk commons are already bulk"
    );
    let gem = report.rows.iter().find(|r| r.name == "Gem").unwrap();
    assert!(gem.reasons.contains(&"reprint_risk"));
}

/// `--format` flags cards with no legality in that format, end to end.
#[test]
fn format_gating_end_to_end() {
    let (_tmp, paths, conn) = setup();
    seed_card(
        &conn,
        "ModernOnly",
        "rare",
        Some(25_000),
        None,
        Some(false),
        None,
        r#"{"modern":"legal"}"#,
    );
    seed_print(&conn, "ModernOnly", "tst", "1", "normal", 5.0);
    seed_binder(&conn, "ModernOnly", "tst", "1", "normal", 1);

    let modern = sell_report(
        &paths,
        &conn,
        &SellOptions {
            format: Some("modern"),
            ..SellOptions::default()
        },
    )
    .expect("candidates");
    assert!(!modern.rows[0].reasons.contains(&"format_unplayed"));

    let legacy = sell_report(
        &paths,
        &conn,
        &SellOptions {
            format: Some("legacy"),
            ..SellOptions::default()
        },
    )
    .expect("candidates");
    assert!(legacy.rows[0].reasons.contains(&"format_unplayed"));
}

/// `--rarity` and price filters narrow the set. An unpriced card is dropped
/// whenever either price bound is set.
#[test]
fn rarity_and_price_filters() {
    let (_tmp, _paths, conn) = setup();
    seed_card(
        &conn,
        "Common",
        "common",
        Some(25_000),
        None,
        Some(false),
        None,
        "{}",
    );
    seed_print(&conn, "Common", "tst", "1", "normal", 0.10);
    seed_binder(&conn, "Common", "tst", "1", "normal", 5);
    simple_card(&conn, "Rare", "rare", 25_000, 20.0);
    // An unpriced card: in the collection, but no print price.
    seed_card(
        &conn,
        "Unpriced",
        "rare",
        Some(25_000),
        None,
        Some(false),
        None,
        "{}",
    );
    seed_binder(&conn, "Unpriced", "tst", "1", "normal", 1);

    let rows = load_binder_rows(&conn).unwrap();
    let demand = std::collections::BTreeMap::new();

    let commons = build_report(
        &conn,
        rows,
        &demand,
        &SellOptions {
            rarity: Some("common"),
            ..SellOptions::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(commons.rows.len(), 1);
    assert_eq!(commons.rows[0].name, "Common");

    let rows = load_binder_rows(&conn).unwrap();
    let expensive = build_report(
        &conn,
        rows,
        &demand,
        &SellOptions {
            min_price: Some(15.0),
            ..SellOptions::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(expensive.rows.len(), 1);
    assert_eq!(expensive.rows[0].name, "Rare");

    // A max-price bound drops the unpriced card.
    let rows = load_binder_rows(&conn).unwrap();
    let cheap = build_report(
        &conn,
        rows,
        &demand,
        &SellOptions {
            max_price: Some(1.0),
            ..SellOptions::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(cheap.rows.len(), 1);
    assert_eq!(cheap.rows[0].name, "Common");

    // No bound: the unpriced card is still a candidate.
    let rows = load_binder_rows(&conn).unwrap();
    let all = build_report(&conn, rows, &demand, &SellOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(all.rows.len(), 3);
}

/// `--limit` truncates displayed rows, but the summary totals cover every
/// candidate.
#[test]
fn limit_truncates_rows_not_totals() {
    let (_tmp, paths, conn) = setup();
    simple_card(&conn, "A", "rare", 25_000, 1.0);
    simple_card(&conn, "B", "rare", 25_000, 1.0);
    simple_card(&conn, "C", "rare", 25_000, 1.0);
    let report = sell_report(
        &paths,
        &conn,
        &SellOptions {
            limit: 2,
            ..SellOptions::default()
        },
    )
    .expect("candidates");
    assert_eq!(report.rows.len(), 2, "only two rows shown");
    assert!(
        (report.sellable_binder_value - 3.0).abs() < 1e-9,
        "all three counted"
    );
}

/// `--target` greedy-picks highest value first and reports honestly.
#[test]
fn target_greedy_fund() {
    let (_tmp, paths, conn) = setup();
    simple_card(&conn, "Big", "rare", 25_000, 30.0);
    simple_card(&conn, "Mid", "rare", 25_000, 10.0);
    simple_card(&conn, "Small", "rare", 25_000, 5.0);

    let report = sell_report(
        &paths,
        &conn,
        &SellOptions {
            target: Some(35.0),
            ..SellOptions::default()
        },
    )
    .expect("candidates");
    let fund = report.fund.expect("fund present with --target");
    assert_eq!(fund.picks, vec!["Big", "Mid"], "30 then 10 clears 35");
    assert_eq!(fund.achieved_usd, 40.0);

    // Unreachable target reports the honest partial sum.
    let report = sell_report(
        &paths,
        &conn,
        &SellOptions {
            target: Some(100.0),
            ..SellOptions::default()
        },
    )
    .expect("candidates");
    let fund = report.fund.unwrap();
    assert_eq!(fund.achieved_usd, 45.0);
    assert_eq!(fund.picks.len(), 3);
}

/// Reserved cards still appear but carry a hold warning (not suppressed).
#[test]
fn reserved_cards_are_flagged_not_hidden() {
    let (_tmp, paths, conn) = setup();
    seed_card(
        &conn,
        "Reserved Rock",
        "rare",
        Some(30_000),
        None,
        Some(true),
        None,
        "{}",
    );
    seed_print(&conn, "Reserved Rock", "tst", "1", "normal", 200.0);
    seed_binder(&conn, "Reserved Rock", "tst", "1", "normal", 1);

    let report = sell_report(&paths, &conn, &SellOptions::default()).expect("candidates");
    assert_eq!(report.rows.len(), 1, "reserved card is still listed");
    assert_eq!(report.rows[0].hold_warnings, vec!["reserved_list"]);
    assert_eq!(report.rows[0].sell_confidence, 0.75);
}

/// An empty binder yields the empty contract and exit 3.
#[test]
fn empty_collection_exits_no_results() {
    let (_tmp, paths, conn) = setup();
    let code = run_sell(&paths, &conn, &SellOptions::default()).unwrap();
    assert_eq!(code, crate::cli::codes::NO_RESULTS);
}
