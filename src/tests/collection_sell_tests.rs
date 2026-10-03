use super::*;
use crate::paths::{Paths, Status};

/// Create an isolated store with a current, nonempty combo snapshot.
fn setup() -> (tempfile::TempDir, Paths, Connection) {
    let tmp = tempfile::tempdir().expect("temporary store");
    let paths = Paths::new(tmp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).expect("deck directory");
    let conn = crate::db::open(&tmp.path().join("t.db")).expect("database");
    let mut status = Status::empty();
    status.setup_complete = true;
    status.combos_synced_at = chrono::Utc::now().to_rfc3339();
    status.write(&paths.status_file()).expect("status");
    conn.execute(
        "INSERT INTO combos (id, updated_at) VALUES ('unrelated', ?1)",
        [chrono::Utc::now().to_rfc3339()],
    )
    .expect("snapshot marker");
    (tmp, paths, conn)
}

/// Seed oracle demand metadata with a known colorless identity.
fn card(conn: &Connection, name: &str, rank: Option<i64>, changer: bool) {
    conn.execute("INSERT INTO cards (name, oracle_id, rarity, edhrec_rank, game_changer, reserved, legalities, color_identity) VALUES (?1, ?1, 'common', ?2, ?3, 0, '{\"commander\":\"legal\",\"modern\":\"legal\"}', '[]')", rusqlite::params![name, rank, changer]).expect("card");
}

/// Seed a priced printing and a binder holding with a real finish quote.
fn print(conn: &Connection, name: &str, set: &str, finish: &str, price: f64, quantity: i64) {
    let set_code = set.to_ascii_lowercase();
    conn.execute("INSERT INTO card_prints (scryfall_id, name, set_code, collector_number, lang, rarity, finishes, released_at, usd, usd_foil, updated_at) VALUES (?1, ?2, ?3, '1', 'en', 'common', '[\"nonfoil\",\"foil\"]', '2020-01-01', ?4, ?4, 't')", rusqlite::params![format!("{name}-{set}"), name, set_code, price]).expect("print");
    conn.execute("INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity) VALUES (?1, ?2, '1', ?3, ?5, 'binder', ?4)", rusqlite::params![name, set_code, finish, quantity, set]).expect("binder");
}

/// Add a deck assignment that cannot cover another deck's shortage.
fn assigned(conn: &Connection, name: &str, deck: &str, quantity: i64) {
    conn.execute("INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity) VALUES (?1, 'tst', '1', 'normal', ?2, 'deck', ?3)", rusqlite::params![name, deck, quantity]).expect("assignment");
}

/// Write playable demand and optional commander or maybeboard sections.
fn deck(paths: &Paths, name: &str, body: &str) {
    std::fs::write(paths.deck_file(name), body).expect("decklist");
}

/// Seed a legal combo with independently stored physical requirements.
fn combo(conn: &Connection, id: &str, pieces: &[(&str, bool)]) {
    conn.execute("INSERT INTO combos (id, legalities, updated_at) VALUES (?1, '{\"commander\":true,\"modern\":true}', ?2)", rusqlite::params![id, chrono::Utc::now().to_rfc3339()]).expect("combo");
    for (ordinal, (name, required)) in pieces.iter().enumerate() {
        let ordinal = i64::try_from(ordinal).expect("test ordinal fits SQLite integer");
        conn.execute("INSERT INTO combo_pieces (combo_id, name, ordinal, must_be_commander) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![id, name, ordinal, required]).expect("piece");
    }
}

/// Build a typed report without capturing terminal output.
fn report(paths: &Paths, conn: &Connection, bulk: bool) -> SellReport {
    build_report(
        paths,
        conn,
        &SellOptions {
            bulk,
            ..SellOptions::default()
        },
    )
    .expect("report")
}

/// High value and low measured demand beat more useful last-copy sales.
#[test]
fn scoring_uses_value_and_independent_demand() {
    let (_tmp, paths, conn) = setup();
    for (name, rank, price) in [
        ("Idle", 25_000, 50.0),
        ("Niche", 10_000, 50.0),
        ("Small", 25_000, 2.0),
        ("Penny", 25_000, 50.0),
    ] {
        card(&conn, name, Some(rank), false);
        print(&conn, name, "tst", "normal", price, 1);
    }
    conn.execute("UPDATE cards SET penny_rank = 1 WHERE name = 'Penny'", [])
        .expect("penny rank");
    let result = report(&paths, &conn, false);
    assert_eq!(result.rows[0].name, "Idle");
    assert!(result.rows[0].sell_priority > result.rows[1].sell_priority);
    assert!(
        !result.rows.iter().any(|r| r.name == "Penny"),
        "positive Penny evidence reserves a spare despite low EDH demand"
    );
}

/// Useful singleton copies stay protected while premium duplicates can sell.
#[test]
fn game_changers_and_staples_keep_a_spare() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Changer", Some(20_000), true);
    print(&conn, "Changer", "cheap", "normal", 5.0, 1);
    print(&conn, "Changer", "premium", "foil", 80.0, 1);
    card(&conn, "Staple", Some(100), false);
    print(&conn, "Staple", "tst", "normal", 20.0, 1);
    let result = report(&paths, &conn, false);
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];
    assert_eq!(
        (row.name.as_str(), row.sell_quantity, row.keep_quantity),
        ("Changer", 1, 1)
    );
    assert_eq!(row.market_value, 80.0);
    assert_eq!(row.printings[0].keep_quantity, 1);
    assert_eq!(row.printings[1].sell_quantity, 1);
}

/// Each deck's unmet demand is protected before spare and sale allocation.
#[test]
fn assigned_copies_cover_only_their_own_deck() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Shared", Some(100), false);
    print(&conn, "Shared", "tst", "normal", 5.0, 4);
    deck(&paths, "A", "// DECK\n1 Shared");
    deck(&paths, "B", "// DECK\n2 Shared");
    assigned(&conn, "Shared", "A", 10);
    let result = report(&paths, &conn, false);
    let row = &result.rows[0];
    assert_eq!(
        (
            row.deck_needed,
            row.spare_reserve,
            row.sell_quantity,
            row.keep_quantity
        ),
        (2, 1, 1, 3)
    );
    assert_eq!(
        row.printings
            .iter()
            .map(|p| p.sell_quantity + p.keep_quantity)
            .sum::<i64>(),
        4
    );
}

/// A stack's total value cannot turn sub-dollar copies into singles.
#[test]
fn bulk_reserves_four_and_sorts_by_excess() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Stack", Some(20_000), false);
    print(&conn, "Stack", "tst", "normal", 0.9, 100);
    card(&conn, "Foils", Some(20_000), false);
    print(&conn, "Foils", "tst", "foil", 0.1, 20);
    deck(&paths, "A", "// DECK\n2 Stack");
    let singles = report(&paths, &conn, false);
    assert!(singles.rows.is_empty());
    assert_eq!(singles.bulk.copies, 110);
    assert_eq!(singles.bulk.estimated_proceeds, None);
    let bulk = report(&paths, &conn, true);
    assert_eq!(bulk.rows[0].name, "Stack");
    assert_eq!(
        (bulk.rows[0].sell_quantity, bulk.rows[0].keep_quantity),
        (94, 6)
    );
    assert_eq!(bulk.bulk.normal_commons_uncommons, 94);
    assert_eq!(bulk.bulk.foils, 16);
}

/// Mixed printings have independent view and price filters after protection.
#[test]
fn mixed_printings_and_boundary_filters() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Mixed", Some(25_000), false);
    print(&conn, "Mixed", "cheap", "normal", 0.1, 10);
    print(&conn, "Mixed", "premium", "foil", 12.0, 1);
    card(&conn, "Boundary", Some(25_000), false);
    print(&conn, "Boundary", "tst", "normal", 1.0, 1);
    let singles = report(&paths, &conn, false);
    assert_eq!(singles.singles.market_value, 13.0);
    assert_eq!(singles.bulk.copies, 6);
    let mixed = singles
        .rows
        .iter()
        .find(|r| r.name == "Mixed")
        .expect("mixed");
    assert_eq!((mixed.sell_quantity, mixed.keep_quantity), (1, 10));
    let filtered = build_report(
        &paths,
        &conn,
        &SellOptions {
            min_price: Some(10.0),
            ..SellOptions::default()
        },
    )
    .expect("filtered");
    assert_eq!(filtered.rows.len(), 1);
    assert_eq!(filtered.rows[0].market_value, 12.0);
}

/// Missing rank affects priority; stale snapshots do not block sale actions.
#[test]
fn unknown_and_stale_evidence_is_reviewed() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Unknown rank", None, false);
    print(&conn, "Unknown rank", "tst", "normal", 100.0, 1);
    card(&conn, "Known", Some(25_000), false);
    print(&conn, "Known", "tst", "normal", 20.0, 1);
    let fresh = report(&paths, &conn, false);
    assert_eq!(fresh.rows.len(), 2);
    assert!(fresh.review.is_empty());
    assert_eq!(fresh.rows[0].sell_priority, 65.0);
    conn.execute("UPDATE combos SET updated_at = '2020-01-01T00:00:00Z'", [])
        .expect("stale");
    let stale = report(&paths, &conn, false);
    assert_eq!(stale.rows.len(), 2);
    assert!(stale.review.is_empty());
    assert_eq!(stale.rows[0].combo_variants, None);
    conn.execute("DELETE FROM combos", [])
        .expect("empty snapshot");
    assert_eq!(report(&paths, &conn, false).rows.len(), 2);
}

/// Unknown oracle cards cannot become confident bulk or singles suggestions.
#[test]
fn unknown_metadata_and_unpriced_copies() {
    let (_tmp, paths, conn) = setup();
    print(&conn, "Ghost", "tst", "normal", 0.1, 10);
    card(&conn, "Unpriced", Some(25_000), false);
    assigned(&conn, "Unpriced", "unused", 1);
    conn.execute(
        "UPDATE collection SET binder_type = 'binder' WHERE name = 'Unpriced'",
        [],
    )
    .expect("unpriced inventory");
    let bulk = report(&paths, &conn, true);
    assert!(bulk.rows.is_empty());
    assert_eq!(bulk.review[0].sell_quantity, 6);
    let singles = report(&paths, &conn, false);
    assert!(singles.warnings.iter().any(|w| w.contains("Unpriced")));
}

/// General combo participation reduces priority; personal interest needs review.
#[test]
fn combo_participation_and_interest_are_visible() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Piece", Some(25_000), false);
    print(&conn, "Piece", "tst", "normal", 50.0, 1);
    card(&conn, "Partner", Some(25_000), false);
    combo(&conn, "one", &[("Piece", false), ("Partner", false)]);
    combo(&conn, "duplicate", &[("Piece", false), ("Partner", false)]);
    card(&conn, "Maybe", Some(25_000), false);
    print(&conn, "Maybe", "tst", "normal", 20.0, 1);
    deck(&paths, "A", "// MAYBEBOARD\n1 Maybe");
    let result = report(&paths, &conn, false);
    assert_eq!(result.rows.len(), 1);
    let piece = result
        .rows
        .iter()
        .find(|r| r.name == "Piece")
        .expect("piece");
    assert_eq!(
        (piece.combo_variants, piece.combo_piece_sets),
        (Some(2), Some(1))
    );
    assert!(piece.sell_priority < piece.market_value);
    assert!(
        result
            .review
            .iter()
            .find(|r| r.name == "Maybe")
            .expect("maybe")
            .hold_warnings
            .contains(&"maybeboard_interest")
    );
}

/// Multi-face names match face records and reserve a personally useful option.
#[test]
fn owned_combo_faces_and_commander_requirements() {
    let (_tmp, paths, conn) = setup();
    for name in ["Front // Back", "Partner", "Leader", "Other leader"] {
        card(&conn, name, Some(25_000), false);
    }
    print(&conn, "Front // Back", "tst", "normal", 30.0, 2);
    assigned(&conn, "Partner", "A", 1);
    assigned(&conn, "Leader", "A", 1);
    combo(
        &conn,
        "faces",
        &[
            ("Front", false),
            ("Back", false),
            ("Partner", false),
            ("Leader", true),
        ],
    );
    deck(&paths, "A", "// COMMANDER\n1 Leader\n// DECK\n1 Partner");
    let result = report(&paths, &conn, false);
    let row = &result.rows[0];
    assert_eq!(
        (
            row.combo_variants,
            row.owned_combo_options,
            row.spare_reserve,
            row.sell_quantity
        ),
        (Some(1), 1, 1, 1)
    );
    deck(
        &paths,
        "A",
        "// COMMANDER\n1 Other leader\n// DECK\n1 Partner",
    );
    let wrong = report(&paths, &conn, false);
    assert_eq!(wrong.rows[0].owned_combo_options, 0);
    assert_eq!(wrong.rows[0].spare_reserve, 0);
}

/// Combo legality follows the selected scope without claiming format play rates.
#[test]
fn format_legality_and_combo_scope() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Piece", Some(25_000), false);
    print(&conn, "Piece", "tst", "normal", 5.0, 1);
    combo(&conn, "commander-only", &[("Piece", true)]);
    conn.execute("UPDATE cards SET legalities = '{\"commander\":\"legal\",\"modern\":\"banned\"}' WHERE name = 'Piece'", []).expect("legality");
    let result = build_report(
        &paths,
        &conn,
        &SellOptions {
            format: Some("Modern"),
            ..SellOptions::default()
        },
    )
    .expect("modern");
    assert_eq!(result.rows[0].combo_variants, Some(0));
    assert_eq!(result.rows[0].legal_in_format, Some(false));
}

/// Funding is display-independent and never counts bulk or review value.
#[test]
fn funding_and_totals_ignore_display_limit() {
    let (_tmp, paths, conn) = setup();
    for (name, price) in [("A", 30.0), ("B", 10.0), ("C", 5.0)] {
        card(&conn, name, Some(25_000), false);
        print(&conn, name, "tst", "normal", price, 1);
    }
    card(&conn, "Bulk", Some(25_000), false);
    print(&conn, "Bulk", "tst", "normal", 0.5, 100);
    let result = build_report(
        &paths,
        &conn,
        &SellOptions {
            target: Some(100.0),
            limit: 1,
            ..SellOptions::default()
        },
    )
    .expect("fund");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.singles.market_value, 45.0);
    let fund = result.fund.expect("funding plan");
    assert_eq!(fund.picks, ["A", "B", "C"]);
    assert_eq!(fund.allocations.len(), 3);
    assert_eq!(fund.allocations[2].sell_quantity, 1);
    assert_eq!((fund.achieved_usd, fund.shortfall_usd), (45.0, 55.0));
    let bulk = build_report(
        &paths,
        &conn,
        &SellOptions {
            bulk: true,
            bulk_rate: Some(5.0),
            ..SellOptions::default()
        },
    )
    .expect("bulk rate");
    assert_eq!(bulk.bulk.estimated_proceeds, Some(0.48));
}

/// Unreadable or invalid decklists prevent confident sale quantities.
#[test]
fn invalid_deck_demand_requires_review() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Idle", Some(25_000), false);
    print(&conn, "Idle", "tst", "normal", 10.0, 1);
    std::fs::write(paths.deck_file("broken"), [0xff]).expect("invalid UTF-8");
    let result = report(&paths, &conn, false);
    assert!(result.rows.is_empty());
    assert!(
        result.review[0]
            .hold_warnings
            .contains(&"unchecked_deck_demand")
    );
}

/// Basic lands are excluded, and an empty view preserves the JSON contract.
#[test]
fn basics_and_empty_contract() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Island", Some(25_000), false);
    print(&conn, "Island", "tst", "normal", 10.0, 100);
    let mut out = crate::output::Output::new(true, false, false);
    assert_eq!(
        run(&paths, &conn, &mut out, &SellOptions::default(), true).expect("run"),
        crate::cli::codes::NO_RESULTS
    );
    let value = serde_json::to_value(report(&paths, &conn, false)).expect("JSON");
    assert_eq!(value["rows"], serde_json::json!([]));
    assert_eq!(value["singles"]["market_value"], 0.0);
}

/// Recent releases reduce priority without blocking sales or aging reprints.
#[test]
fn recent_card_is_ranked_but_new_reprint_does_not_reset_age() {
    let (_tmp, paths, conn) = setup();
    for name in ["New card", "Old card"] {
        card(&conn, name, Some(25_000), false);
        print(&conn, name, "tst", "normal", 20.0, 1);
    }
    conn.execute(
        "UPDATE card_prints SET released_at = ?1 WHERE name = 'New card'",
        [chrono::Utc::now().date_naive().to_string()],
    )
    .expect("new release");
    print(&conn, "Old card", "reprint", "foil", 40.0, 1);
    conn.execute(
        "UPDATE card_prints SET released_at = ?1 WHERE set_code = 'reprint'",
        [chrono::Utc::now().date_naive().to_string()],
    )
    .expect("new printing");
    let result = report(&paths, &conn, false);
    assert_eq!(result.rows[0].name, "Old card");
    assert!(result.review.is_empty());
    assert_eq!(result.rows[1].name, "New card");
    assert_eq!(result.rows[1].sell_priority, 17.0);
}

/// Fully owned pieces cannot claim a personal combo fit outside commander colors.
#[test]
fn owned_combo_identity_must_match() {
    let (_tmp, paths, conn) = setup();
    for name in ["Blue piece", "White leader"] {
        card(&conn, name, Some(25_000), false);
    }
    conn.execute(
        "UPDATE cards SET color_identity = '[\"U\"]' WHERE name = 'Blue piece'",
        [],
    )
    .expect("blue identity");
    conn.execute(
        "UPDATE cards SET color_identity = '[\"W\"]' WHERE name = 'White leader'",
        [],
    )
    .expect("white identity");
    print(&conn, "Blue piece", "tst", "normal", 10.0, 2);
    assigned(&conn, "White leader", "A", 1);
    deck(&paths, "A", "// COMMANDER\n1 White leader");
    combo(&conn, "colors", &[("Blue piece", false)]);
    let mismatch = report(&paths, &conn, false);
    assert_eq!(mismatch.rows[0].owned_combo_options, 0);
    conn.execute(
        "UPDATE cards SET color_identity = '[\"W\",\"U\"]' WHERE name = 'White leader'",
        [],
    )
    .expect("matching identity");
    let matches = report(&paths, &conn, false);
    assert_eq!(matches.rows[0].owned_combo_options, 1);
    assert_eq!(matches.rows[0].sell_quantity, 1);
}

/// Collector copies cannot fill deckbuilding reserves, sale totals, or funding.
#[test]
fn excluded_binders_do_not_cover_reserves_or_sales() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Staple", Some(100), false);
    print(&conn, "Staple", "Collect", "foil", 50.0, 10);
    print(&conn, "Staple", "Play", "normal", 5.0, 3);
    card(&conn, "Bulk", Some(25_000), false);
    print(&conn, "Bulk", "Collect", "normal", 0.5, 100);
    print(&conn, "Bulk", "Play", "normal", 0.1, 10);
    deck(&paths, "A", "// DECK\n1 Staple\n2 Bulk");
    let excluded = ["Collect".to_string()];
    let options = SellOptions {
        exclude_binders: &excluded,
        target: Some(10.0),
        ..SellOptions::default()
    };
    let result = build_report(&paths, &conn, &options).expect("excluded plan");
    assert_eq!(result.rows[0].owned_binder, 3);
    assert_eq!(
        (result.rows[0].sell_quantity, result.rows[0].keep_quantity),
        (1, 2)
    );
    assert_eq!(result.singles.market_value, 5.0);
    assert_eq!(result.bulk.copies, 4);
    assert_eq!(result.fund.as_ref().expect("fund").shortfall_usd, 5.0);
    assert!(
        result
            .rows
            .iter()
            .chain(&result.review)
            .flat_map(|r| &r.printings)
            .all(|p| p.binder != "Collect")
    );
}

/// Exact binder exclusions are repeatable and case-insensitive, without changing assignments.
#[test]
fn exclusions_match_whole_names_and_preserve_assignments() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Card", Some(25_000), false);
    for binder in ["Collect", "collect extras", "Trade", "Play"] {
        print(&conn, "Card", binder, "normal", 10.0, 1);
    }
    assigned(&conn, "Card", "Collect", 1);
    deck(&paths, "Collect", "// DECK\n1 Card");
    let excluded = ["COLLECT".to_string(), "trade".to_string()];
    let result = build_report(
        &paths,
        &conn,
        &SellOptions {
            exclude_binders: &excluded,
            ..SellOptions::default()
        },
    )
    .expect("exclusions");
    assert_eq!(result.rows[0].sell_quantity, 2);
    assert_eq!(result.rows[0].deck_needed, 0);
    assert_eq!(result.excluded_binders, excluded);
    assert!(result.rows[0].printings.iter().any(|p| p.binder == "Play"));
    assert!(
        result.rows[0]
            .printings
            .iter()
            .any(|p| p.binder == "collect extras")
    );
    let invalid = ["Collect typo".to_string()];
    assert!(
        build_report(
            &paths,
            &conn,
            &SellOptions {
                exclude_binders: &invalid,
                ..SellOptions::default()
            }
        )
        .is_err()
    );
}

/// Collector pieces do not claim a personally owned combo in the sale pool.
#[test]
fn excluded_combo_pieces_do_not_create_owned_options() {
    let (_tmp, paths, conn) = setup();
    for name in ["Piece", "Partner", "Leader"] {
        card(&conn, name, Some(25_000), false);
    }
    print(&conn, "Piece", "Play", "normal", 10.0, 1);
    print(&conn, "Partner", "Collect", "normal", 10.0, 1);
    assigned(&conn, "Leader", "A", 1);
    deck(&paths, "A", "// COMMANDER\n1 Leader");
    combo(&conn, "owned", &[("Piece", false), ("Partner", false)]);
    assert!(
        !report(&paths, &conn, false)
            .rows
            .iter()
            .any(|r| r.name == "Piece")
    );
    let excluded = ["Collect".to_string()];
    let result = build_report(
        &paths,
        &conn,
        &SellOptions {
            exclude_binders: &excluded,
            ..SellOptions::default()
        },
    )
    .expect("excluded combo");
    assert_eq!(result.rows[0].name, "Piece");
    assert_eq!(result.rows[0].owned_combo_options, 0);
    assert_eq!(result.rows[0].sell_quantity, 1);
}

/// Default output is one row per card with optional details and review.
#[test]
fn compact_output_keeps_evidence_and_notes_opt_in() {
    let (_tmp, paths, conn) = setup();
    for i in 0..25 {
        let name = format!("Card {i:02}");
        card(&conn, &name, Some(25_000), false);
        print(&conn, &name, "Play", "normal", 10.0, 1);
    }
    card(&conn, "Maybe", None, false);
    print(&conn, "Maybe", "Play", "normal", 10.0, 1);
    deck(&paths, "A", "// MAYBEBOARD\n1 Maybe");
    let result = report(&paths, &conn, false);
    let styles = crate::output::Output::new(false, true, false).styles();
    let output = render::human(&result, &SellOptions::default(), &styles, 80);
    assert_eq!(result.rows.len(), 20);
    assert!(output.lines().count() < 32);
    assert!(output.contains("Showing 20 of 25. Use --limit 25 to see more."));
    assert!(!output.contains("EDHREC"));
    assert!(!output.contains("not ranked"));
    assert!(!output.contains("Maybe"));
    assert!(!output.contains("market estimates"));
    assert!(!output.contains('\u{1b}'));
    let detailed = render::human(
        &result,
        &SellOptions {
            details: true,
            review: true,
            ..SellOptions::default()
        },
        &styles,
        80,
    );
    assert!(detailed.contains("EDHREC #25000"));
    assert!(detailed.contains("Maybe"));
    assert!(detailed.contains("Play · PLAY 1"));
    assert!(!detailed.contains("not ranked"));
}

/// Unicode names and binder cells stay within narrow terminal widths.
#[test]
fn compact_tables_fit_narrow_terminals() {
    let (_tmp, paths, conn) = setup();
    let name = "Jötun Grunt // A very long card face name";
    card(&conn, name, Some(25_000), false);
    print(&conn, name, "Long binder name", "normal", 0.1, 10);
    let result = report(&paths, &conn, true);
    let styles = crate::output::Output::new(false, true, false).styles();
    let output = render::human(
        &result,
        &SellOptions {
            bulk: true,
            ..SellOptions::default()
        },
        &styles,
        40,
    );
    let row = output
        .lines()
        .find(|line| line.starts_with("Jötun"))
        .expect("table row");
    assert!(console::measure_text_width(row) <= 40);
    assert!(row.contains('…'));
}

/// Text exports merge source binders and round-trip through the ManaBox grammar.
#[test]
fn txt_export_preserves_sale_quantities_and_printings() {
    let (_tmp, paths, conn) = setup();
    let name = "Jötun, Grunt // Back";
    card(&conn, name, Some(25_000), false);
    print(&conn, name, "abc", "normal", 5.0, 3);
    print(&conn, name, "def", "foil", 8.0, 1);
    conn.execute(
        "UPDATE card_prints SET collector_number = '007' WHERE set_code = 'abc'",
        [],
    )
    .expect("print number");
    conn.execute(
        "UPDATE collection SET collector_number = '007' WHERE set_code = 'abc'",
        [],
    )
    .expect("owned number");
    conn.execute("INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity) VALUES (?1, 'abc', '007', 'normal', 'Other', 'binder', 2)", [name]).expect("second binder");
    let result = build_report(
        &paths,
        &conn,
        &SellOptions {
            output: Some(SellOutput::Txt),
            ..SellOptions::default()
        },
    )
    .expect("text plan");
    let mut output = Vec::new();
    assert_eq!(
        export::write(&result, SellOutput::Txt, &mut output).expect("text export"),
        2
    );
    let text = String::from_utf8(output).expect("UTF-8");
    let parsed = crate::deck::grammar::Deck::parse(&text).expect("ManaBox text");
    assert_eq!(parsed.total(), 6);
    let entries = parsed.entries().collect::<Vec<_>>();
    assert_eq!(entries[0].name, name);
    assert_eq!(entries[0].quantity, 5);
    assert_eq!(entries[0].collector_number.as_deref(), Some("007"));
    assert!(!entries[0].foil);
    assert!(entries[1].foil);
    assert!(!text.contains("Value"));
    assert!(!text.contains('\u{1b}'));
}

/// CSV quoting and finish distinctions remain importable as non-owning list rows.
#[test]
fn csv_export_quotes_names_and_preserves_finishes() {
    let (tmp, paths, conn) = setup();
    let name = "Card, \"quoted\"";
    card(&conn, name, Some(25_000), false);
    print(&conn, name, "abc", "normal", 5.0, 2);
    print(&conn, name, "def", "foil", 6.0, 3);
    print(&conn, name, "ghi", "etched", 7.0, 4);
    conn.execute(
        "UPDATE card_prints SET usd_etched = 7 WHERE set_code = 'ghi'",
        [],
    )
    .expect("etched quote");
    let result = build_report(
        &paths,
        &conn,
        &SellOptions {
            output: Some(SellOutput::Csv),
            ..SellOptions::default()
        },
    )
    .expect("CSV plan");
    let mut output = Vec::new();
    assert_eq!(
        export::write(&result, SellOutput::Csv, &mut output).expect("CSV export"),
        3
    );
    let mut reader = csv::Reader::from_reader(output.as_slice());
    assert_eq!(
        reader.headers().expect("headers").get(1),
        Some("Binder Type")
    );
    let records = reader
        .records()
        .collect::<Result<Vec<_>, _>>()
        .expect("CSV records");
    assert_eq!(
        records.iter().map(|r| r[5].to_string()).collect::<Vec<_>>(),
        ["normal", "foil", "etched"]
    );
    for record in &records {
        assert_eq!(&record[0], "Sell");
        assert_eq!(&record[1], "list");
        assert_eq!(&record[2], name);
    }
    assert_eq!(
        records
            .iter()
            .map(|r| r[6].parse::<i64>().expect("quantity"))
            .sum::<i64>(),
        9
    );
    let file = tmp.path().join("sell.csv");
    std::fs::write(&file, output).expect("CSV file");
    let (owned, lists) =
        crate::collection::parse_csv(&file, &mut crate::output::Output::new(true, true, false))
            .expect("ManaBox parser");
    assert!(owned.is_empty());
    assert_eq!(lists, 3);
}

/// Exports are complete while target exports contain only funding picks.
#[test]
fn exports_ignore_display_limits_and_use_funding_allocations() {
    let (_tmp, paths, conn) = setup();
    for (name, price) in [("A", 30.0), ("B", 10.0), ("C", 5.0)] {
        card(&conn, name, Some(25_000), false);
        print(&conn, name, "abc", "normal", price, 1);
    }
    card(&conn, "Hold", Some(25_000), false);
    print(&conn, "Hold", "abc", "normal", 50.0, 1);
    deck(&paths, "A", "// MAYBEBOARD\n1 Hold");
    let options = SellOptions {
        output: Some(SellOutput::Txt),
        limit: 1,
        ..SellOptions::default()
    };
    let result = build_report(&paths, &conn, &options).expect("complete plan");
    assert_eq!(result.rows.len(), 3);
    let mut output = Vec::new();
    assert_eq!(
        export::write(&result, SellOutput::Txt, &mut output).expect("complete export"),
        3
    );
    assert!(!String::from_utf8(output).expect("text").contains("Hold"));
    let fund = build_report(
        &paths,
        &conn,
        &SellOptions {
            target: Some(35.0),
            ..options
        },
    )
    .expect("funding");
    let mut output = Vec::new();
    assert_eq!(
        export::write(&fund, SellOutput::Txt, &mut output).expect("fund export"),
        2
    );
    assert!(!String::from_utf8(output).expect("text").contains("C (ABC)"));
}

/// Bulk exports contain only excess copies and empty files never invent entries.
#[test]
fn bulk_and_empty_exports() {
    let (_tmp, paths, conn) = setup();
    card(&conn, "Bulk", Some(25_000), false);
    print(&conn, "Bulk", "abc", "normal", 0.1, 10);
    let options = SellOptions {
        output: Some(SellOutput::Txt),
        bulk: true,
        ..SellOptions::default()
    };
    let result = build_report(&paths, &conn, &options).expect("bulk plan");
    let mut output = Vec::new();
    export::write(&result, SellOutput::Txt, &mut output).expect("bulk export");
    assert_eq!(String::from_utf8(output).expect("text"), "6 Bulk (ABC) 1\n");
    let empty = build_report(
        &paths,
        &conn,
        &SellOptions {
            bulk: false,
            ..options
        },
    )
    .expect("empty singles");
    let mut output = Vec::new();
    assert_eq!(
        export::write(&empty, SellOutput::Txt, &mut output).expect("empty text"),
        0
    );
    assert!(output.is_empty());
    assert_eq!(
        export::write(&empty, SellOutput::Csv, &mut output).expect("empty CSV"),
        0
    );
    assert_eq!(
        csv::Reader::from_reader(output.as_slice())
            .records()
            .count(),
        0
    );
}

/// CLI parsing separates file exports from JSON and human-only display flags.
#[test]
fn output_flag_accepts_only_supported_export_formats() {
    use crate::cli::{Cli, CollectionCommand, Command};
    use clap::Parser;
    for (key, format) in [("txt", SellOutput::Txt), ("csv", SellOutput::Csv)] {
        let cli = Cli::try_parse_from(["stm", "collection", "sell", "--output", key])
            .expect("output format");
        assert!(
            matches!(cli.command, Command::Collection { command: Some(CollectionCommand::Sell { output: Some(value), .. }), .. } if value == format)
        );
    }
    assert!(Cli::try_parse_from(["stm", "collection", "sell", "--output", "html"]).is_err());
    for flag in ["--json", "--details", "--review"] {
        assert!(
            Cli::try_parse_from(["stm", "collection", "sell", "--output", "csv", flag]).is_err()
        );
    }
}
