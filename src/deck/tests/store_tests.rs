use super::*;
use crate::deck::ownership;

#[test]
fn valid_names_reject_paths() {
    assert!(valid_deck_name("Stationz"));
    assert!(valid_deck_name("Token Triumph"));
    assert!(valid_deck_name("Froggy!"));
    assert!(!valid_deck_name("a/b"));
    assert!(!valid_deck_name(".."));
    assert!(!valid_deck_name(""));
    assert!(!valid_deck_name("x\ny"));
}

/// Connection plus its backing tempdir (must outlive the connection).
fn seeded_conn() -> (tempfile::TempDir, Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES ('Lightning Bolt', 'oid')",
        [],
    )
    .unwrap();
    (tmp, conn)
}

fn silent_out() -> crate::output::Output {
    crate::output::Output::new(true, false, false)
}

fn add_collection_row(
    conn: &Connection,
    binder: &str,
    binder_type: &str,
    name: &str,
    set: &str,
    cn: &str,
    qty: i64,
) {
    conn.execute(
        "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity)
         VALUES (?1, ?2, ?3, 'normal', ?4, ?5, ?6)",
        rusqlite::params![name, set, cn, binder, binder_type, qty],
    )
    .unwrap();
}

#[test]
fn any_printing_fills_a_deck_line() {
    let (_tmp, conn) = seeded_conn();
    // Owns a different set version than the deck line names.
    add_collection_row(
        &conn,
        "Collect",
        "binder",
        "Lightning Bolt",
        "m11",
        "148",
        4,
    );
    let owned = crate::deck::store_show::owned_map_for_deck(&conn, "TestDeck").unwrap();
    // Nothing assigned to the deck itself, but 4 sit in a binder.
    assert_eq!(owned.get("Lightning Bolt"), Some(&(0, 4)));

    add_collection_row(&conn, "TestDeck", "deck", "Lightning Bolt", "2xm", "124", 2);
    let owned = crate::deck::store_show::owned_map_for_deck(&conn, "TestDeck").unwrap();
    // Deck-assigned copies count regardless of print.
    assert_eq!(owned.get("Lightning Bolt"), Some(&(2, 4)));
}

#[test]
fn delete_removes_files_keeps_ownership() {
    let (tmp, conn) = seeded_conn();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Froggy"), "// DECK\n3 Lightning Bolt\n").unwrap();
    add_collection_row(&conn, "Froggy", "deck", "Lightning Bolt", "m11", "146", 2);
    let mut out = silent_out();

    let code = delete(&paths, &conn, &mut out, "Froggy").unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    assert!(!paths.deck_file("Froggy").exists());
    // Ownership rows are untouched.
    let copies: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(quantity), 0) FROM collection WHERE binder_type = 'deck'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(copies, 2);
    // Deleting again fails with no-results.
    let code = delete(&paths, &conn, &mut out, "Froggy").unwrap();
    assert_eq!(code, crate::cli::codes::NO_RESULTS);
}

#[test]
fn registered_deck_names_power_the_gap_display() {
    let (_tmp, conn) = seeded_conn();
    add_collection_row(
        &conn,
        "Ghost Deck",
        "deck",
        "Lightning Bolt",
        "m11",
        "146",
        2,
    );
    add_collection_row(
        &conn,
        "Collect",
        "binder",
        "Lightning Bolt",
        "m11",
        "148",
        1,
    );
    let names = registered_deck_names(&conn).unwrap();
    assert_eq!(names, vec!["Ghost Deck"]);
    // Binder copies of deck names count in the binder bucket (they can
    // fill deck slots).
    assert_eq!(owned_copies(&conn, "Ghost Deck").unwrap(), (2, 1));
}

/// A deck txt + primer pair inside a temp paths tree.
fn deck_paths() -> (tempfile::TempDir, crate::paths::Paths) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().join("root"));
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    (tmp, paths)
}

#[test]
fn missing_cost_matches_buylist_total() {
    // The two code paths must agree: deck_value (show) uses the same
    // deck+binders coverage as buylist's missing math.
    let (_tmp, paths) = deck_paths();
    let (tmp2, conn) = seeded_conn();
    let _keep_alive = tmp2;
    // 3 needed, 1 deck-assigned + 1 binder → 1 missing.
    add_collection_row(&conn, "TestDeck", "deck", "Lightning Bolt", "m11", "148", 1);
    add_collection_row(
        &conn,
        "Collect",
        "binder",
        "Lightning Bolt",
        "m11",
        "148",
        1,
    );
    conn.execute(
        "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
            lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
         VALUES ('a', 'Lightning Bolt', 'm11', '148', 'en', 'common',
            '[\"nonfoil\",\"foil\"]', '2020-01-01', 0.5, NULL, 't')",
        [],
    )
    .unwrap();
    let deck = crate::deck::Deck::parse("// DECK\n3 Lightning Bolt\n").unwrap();
    std::fs::write(paths.deck_file("TestDeck"), deck.to_text()).unwrap();

    let cards_by_name = super::super::stats::lookup_names(&conn, &deck);
    let prices = crate::deck::store_show::deck_prices(&conn, &deck);
    let available = ownership::available_map(&conn, "TestDeck").unwrap();
    let (_, missing_cost) =
        crate::deck::store_show::deck_value(&deck, &cards_by_name, &prices, &available);

    // Buylist path over the same collection state.
    let rows =
        super::super::buylist::missing_rows(&conn, &deck, &cards_by_name, &available).unwrap();
    let buylist_total: f64 = rows
        .iter()
        .map(|r| r.price_usd.unwrap_or(0.0) * r.quantity as f64)
        .sum();
    assert_eq!(
        crate::deck::store_show::round2(missing_cost),
        crate::deck::store_show::round2(buylist_total),
        "show missing_cost must equal buylist total"
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].quantity, 1);
}

#[test]
fn slot_map_classifies_coverage() {
    let deck = super::super::Deck::parse("// DECK\n3 Bolt\n2 Shock\n").unwrap();
    let assigned = [("Bolt".to_string(), 3i64)].into_iter().collect();
    let available = [("Bolt".to_string(), 4i64), ("Shock".to_string(), 1i64)]
        .into_iter()
        .collect();
    let empty_elsewhere: std::collections::HashMap<String, (i64, Vec<String>)> = Default::default();
    let slots = ownership::slot_map(&deck, &available, &assigned, &empty_elsewhere, |_| false);
    let bolt = &slots["Bolt"];
    assert_eq!(bolt.coverage, ownership::Coverage::Deck);
    assert_eq!(bolt.in_deck, 3);
    assert_eq!(bolt.in_binder, 1);
    assert_eq!(bolt.missing, 0);
    let shock = &slots["Shock"];
    assert_eq!(shock.coverage, ownership::Coverage::Missing);
    assert_eq!(shock.missing, 1);
    // Binder-only coverage: 3 needed, 0 deck, 3 binder.
    let assigned2: std::collections::HashMap<String, i64> = Default::default();
    let available2 = [("Bolt".to_string(), 3i64)].into_iter().collect();
    let slots = ownership::slot_map(
        &super::super::Deck::parse("// DECK\n3 Bolt\n").unwrap(),
        &available2,
        &assigned2,
        &empty_elsewhere,
        |_| false,
    );
    assert_eq!(slots["Bolt"].coverage, ownership::Coverage::Binder);
}

#[test]
fn slot_map_reports_held_elsewhere_reason() {
    let deck = super::super::Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let empty: std::collections::HashMap<String, i64> = Default::default();
    let empty_elsewhere: std::collections::HashMap<String, (i64, Vec<String>)> = Default::default();
    // No copies available to this deck, but one parked in another deck.
    let elsewhere = [("Bolt".to_string(), (1i64, vec!["Froggy".to_string()]))]
        .into_iter()
        .collect();
    let slots = ownership::slot_map(&deck, &empty, &empty, &elsewhere, |_| false);
    let bolt = &slots["Bolt"];
    assert_eq!(bolt.coverage, ownership::Coverage::Missing);
    assert_eq!(bolt.missing, 1);
    assert_eq!(bolt.held_elsewhere, 1, "the copy explains the reason");
    // Without the elsewhere map the reason degrades to not_owned.
    let slots = ownership::slot_map(&deck, &empty, &empty, &empty_elsewhere, |_| false);
    assert_eq!(slots["Bolt"].held_elsewhere, 0);
}

#[cfg(test)]
mod universe_tests {
    use super::*;

    fn seeded_conn() -> (tempfile::TempDir, Connection) {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        conn.execute(
            "INSERT INTO sets (set_code, set_name, set_type, block, franchise)
         VALUES ('msh', 'Marvel Super Heroes', 'expansion', NULL, 'Marvel'),
                ('mh3', 'Modern Horizons 3', 'draft_innovation', NULL, NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cards (name, oracle_id, set_code) VALUES
            ('Iron Test', 'o1', 'msh'),
            ('Both Prints', 'o2', 'mh3'),
            ('Bolt', 'o3', 'mh3')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, universes_beyond, updated_at)
         VALUES ('p1', 'Iron Test', 'msh', 1, 't'),
                ('p2', 'Both Prints', 'msh', 1, 't'),
                ('p3', 'Both Prints', 'mh3', 0, 't')",
            [],
        )
        .unwrap();
        (tmp, conn)
    }

    #[test]
    fn universe_census_splits_main_deck_from_sideboard() {
        let (_tmp, conn) = seeded_conn();
        let deck = super::super::Deck::parse(
        "// COMMANDER\n1 Iron Test\n// DECK\n2 Both Prints\n1 Bolt\n// SIDEBOARD\n1 Iron Test\n",
    )
    .unwrap();
        let cards_by_name = crate::deck::stats::lookup_names(&conn, &deck);
        let census =
            crate::deck::store_show::universe_census(&conn, &deck, &cards_by_name).unwrap();
        // Iron Test: all-UB → beyond. Both Prints: any in-universe print wins.
        assert_eq!(census["universes_beyond"], 1, "commander qty only");
        assert_eq!(census["multiverse"], 3, "2 Both Prints + 1 Bolt");
        assert_eq!(
            census["franchises"]["Marvel"], 1,
            "Iron Test only; Both Prints is multiverse"
        );
        let ub_cards: Vec<&str> = census["ub_cards"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c.as_str())
            .collect();
        assert_eq!(ub_cards, vec!["Iron Test"]);
    }
}

#[test]
fn held_elsewhere_map_names_the_holding_decks() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, set_code) VALUES ('Bolt', 'o1', 'tst')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity)
         VALUES ('Bolt', 'Froggy', 'deck', 1), ('Bolt', 'Teysa', 'deck', 2),
                ('Bolt', 'binder-main', 'binder', 5)",
        [],
    )
    .unwrap();
    let map = ownership::held_elsewhere_map(&conn, "Stationz").unwrap();
    let (qty, holders) = &map["Bolt"];
    assert_eq!(*qty, 3, "deck-assigned copies only, binders excluded");
    assert_eq!(holders, &vec!["Froggy".to_string(), "Teysa".to_string()]);
}
