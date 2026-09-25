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
    // Deleting again is a missing-deck error (exit 1, matching the other
    // deck subcommands' deck-not-found handling).
    let code = delete(&paths, &conn, &mut out, "Froggy").unwrap();
    assert_eq!(code, crate::cli::codes::ERROR);
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
         VALUES ('a', 'Lightning Bolt', 'M11', '148', 'en', 'common',
            '[\"nonfoil\",\"foil\"]', '2020-01-01', 0.5, NULL, 't')",
        [],
    )
    .unwrap();
    let deck = crate::deck::Deck::parse("// DECK\n3 Lightning Bolt\n").unwrap();
    std::fs::write(paths.deck_file("TestDeck"), deck.to_text()).unwrap();

    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let prices = crate::deck::store_show::deck_finish_prices(
        &conn,
        &deck,
        &mut crate::output::Output::new(true, false, false),
    );
    let available = ownership::available_map(&conn, "TestDeck").unwrap();
    let available_by_finish = ownership::available_map_by_finish(&conn, "TestDeck").unwrap();
    let (_, missing_cost) =
        crate::deck::store_show::deck_value(&deck, &cards_by_name, &prices, &available);

    // Buylist path over the same collection state.
    let (rows, unknown) =
        super::super::buylist::missing_rows(&conn, &deck, &cards_by_name, &available_by_finish)
            .unwrap();
    assert!(unknown.is_empty());
    let buylist_total: f64 = rows
        .iter()
        .map(|r| r.price_usd.unwrap_or(0.0) * r.quantity as f64)
        .sum();
    assert_eq!(
        crate::output::round2(missing_cost),
        crate::output::round2(buylist_total),
        "show missing_cost must equal buylist total"
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].quantity, 1);
}

/// Mixed-foil value fixtures: a card with a priced nonfoil print, a priced
/// foil print, or both.
fn foil_conn() -> (tempfile::TempDir, Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES ('Lightning Bolt', 'oid')",
        [],
    )
    .unwrap();
    // Nonfoil-only print at $1, foil-only print at $10.
    conn.execute(
        "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
            lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
         VALUES ('a', 'Lightning Bolt', 'M11', '148', 'en', 'common',
            '[\"nonfoil\"]', '2020-01-01', 1.0, NULL, 't'),
                ('b', 'Lightning Bolt', 'ZEN', '50', 'en', 'mythic',
            '[\"foil\"]', '2020-01-01', NULL, 10.0, 't')",
        [],
    )
    .unwrap();
    (tmp, conn)
}

#[test]
fn mixed_foil_entries_each_price_at_their_finish() {
    // A name split 1 foil + 1 nonfoil must total f + n = 11.0, not the
    // old blended 2f + 2n = 22.0.
    let (_tmp, conn) = foil_conn();
    let deck = crate::deck::Deck::parse(
        "// DECK\n1 Lightning Bolt (M11) 148\n1 Lightning Bolt (ZEN) 50 *F*\n",
    )
    .unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let prices = crate::deck::store_show::deck_finish_prices(&conn, &deck, &mut silent_out());
    let empty: std::collections::HashMap<String, i64> = Default::default();
    let (owned_value, missing_cost) =
        crate::deck::store_show::deck_value(&deck, &cards_by_name, &prices, &empty);
    assert_eq!(crate::output::round2(owned_value), 0.0, "nothing available");
    assert_eq!(crate::output::round2(missing_cost), 11.0);
    // Per-entry prices: the foil entry reads the foil rate, the nonfoil
    // entry the nonfoil rate.
    let entries: Vec<_> = deck.entries().collect();
    assert_eq!(
        crate::deck::store_show::entry_unit_price(entries[0], &prices),
        Some(1.0)
    );
    assert_eq!(
        crate::deck::store_show::entry_unit_price(entries[1], &prices),
        Some(10.0)
    );
}

#[test]
fn uneven_foil_split_sums_each_entry_at_its_rate() {
    // 3 foil + 1 nonfoil: 3*10 + 1*1 = 31 (the old blend gave 4*(f+n)).
    let (_tmp, conn) = foil_conn();
    let deck = crate::deck::Deck::parse(
        "// DECK\n3 Lightning Bolt (ZEN) 50 *F*\n1 Lightning Bolt (M11) 148\n",
    )
    .unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let prices = crate::deck::store_show::deck_finish_prices(&conn, &deck, &mut silent_out());
    let empty: std::collections::HashMap<String, i64> = Default::default();
    let (owned_value, _) =
        crate::deck::store_show::deck_value(&deck, &cards_by_name, &prices, &empty);
    assert_eq!(crate::output::round2(owned_value), 0.0, "nothing available");
    // Same deck with the copies available: 3*10 + 1*1 = 31.
    let available = [("Lightning Bolt".to_string(), 4i64)].into_iter().collect();
    let (owned_value, _) =
        crate::deck::store_show::deck_value(&deck, &cards_by_name, &prices, &available);
    assert_eq!(crate::output::round2(owned_value), 31.0);
}

#[test]
fn missing_finish_price_falls_back_to_the_available_one() {
    // Only a nonfoil print priced (the zen foil print removed): a foil
    // entry prices at the nonfoil rate, a nonfoil entry at its own rate.
    let (_tmp, conn) = foil_conn();
    conn.execute("DELETE FROM card_prints WHERE set_code = 'ZEN'", [])
        .unwrap();
    let deck =
        crate::deck::Deck::parse("// DECK\n2 Lightning Bolt *F*\n1 Lightning Bolt\n").unwrap();
    let prices = crate::deck::store_show::deck_finish_prices(&conn, &deck, &mut silent_out());
    let entries: Vec<_> = deck.entries().collect();
    // Foil entry falls back to the $1 nonfoil print; nonfoil stays $1.
    assert_eq!(
        crate::deck::store_show::entry_unit_price(entries[0], &prices),
        Some(1.0)
    );
    assert_eq!(
        crate::deck::store_show::entry_unit_price(entries[1], &prices),
        Some(1.0)
    );
    // Symmetric: only a foil print priced, nonfoil entry falls back.
    let (_tmp, conn) = foil_conn();
    conn.execute("DELETE FROM card_prints WHERE set_code = 'M11'", [])
        .unwrap();
    let deck =
        crate::deck::Deck::parse("// DECK\n1 Lightning Bolt\n2 Lightning Bolt *F*\n").unwrap();
    let prices = crate::deck::store_show::deck_finish_prices(&conn, &deck, &mut silent_out());
    let entries: Vec<_> = deck.entries().collect();
    assert_eq!(
        crate::deck::store_show::entry_unit_price(entries[0], &prices),
        Some(10.0),
        "nonfoil entry falls back to the foil price"
    );
    assert_eq!(
        crate::deck::store_show::entry_unit_price(entries[1], &prices),
        Some(10.0)
    );
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

#[test]
fn deck_line_ownership_shows_binder_and_elsewhere_copies() {
    // The deck-line display reads in_deck + in_binder as "own" (raw, no
    // cap) and held_elsewhere as "(+N elsewhere)". A binder-only copy is
    // owned, not elsewhere; another deck's copy is elsewhere, not owned.
    let deck = super::super::Deck::parse("// DECK\n1 Bolt\n1 Shock\n1 Fear\n").unwrap();
    let assigned = [("Bolt".to_string(), 2i64)].into_iter().collect();
    let available = [
        ("Bolt".to_string(), 3i64),
        ("Shock".to_string(), 1i64),
        ("Fear".to_string(), 0i64),
    ]
    .into_iter()
    .collect();
    let elsewhere = [("Fear".to_string(), (2i64, vec!["Froggy".to_string()]))]
        .into_iter()
        .collect();
    let slots = ownership::slot_map(&deck, &available, &assigned, &elsewhere, |_| false);
    let bolt = &slots["Bolt"];
    assert_eq!(bolt.in_deck + bolt.in_binder, 3, "raw total, over quantity");
    assert_eq!(bolt.missing, 0);
    let shock = &slots["Shock"];
    assert_eq!(shock.in_deck + shock.in_binder, 1, "binder copy is owned");
    assert_eq!(shock.held_elsewhere, 0, "binder copies are not elsewhere");
    let fear = &slots["Fear"];
    assert_eq!(fear.in_deck + fear.in_binder, 0);
    assert_eq!(fear.held_elsewhere, 2, "other decks' copies are elsewhere");
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
        let cards_by_name = crate::deck::stats::lookup_names(&conn, &deck).unwrap();
        let mut out = crate::output::Output::new(true, false, false);
        let census =
            crate::deck::store_show::universe_census(&conn, &deck, &cards_by_name, &mut out)
                .unwrap();
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

// print_to_buy_block

use crate::deck::store_show::to_buy_block_text;

fn slot(
    in_deck: i64,
    in_binder: i64,
    held_elsewhere: i64,
    missing: i64,
) -> ownership::SlotOwnership {
    ownership::SlotOwnership {
        in_deck,
        in_binder,
        held_elsewhere,
        missing,
        coverage: ownership::Coverage::Missing,
    }
}

#[test]
fn to_buy_block_sorts_priced_desc_and_truncates() {
    let slots = [
        ("Cheap".to_string(), slot(0, 0, 0, 2)),
        ("Pricy".to_string(), slot(0, 0, 0, 1)),
        ("Unpriced".to_string(), slot(0, 0, 0, 4)),
        ("Elsewhere".to_string(), slot(0, 0, 2, 1)),
        ("Full".to_string(), slot(1, 0, 0, 0)),
    ]
    .into_iter()
    .collect();
    let prices = [
        ("Cheap".to_string(), (None, Some(1.0))),
        ("Pricy".to_string(), (None, Some(10.0))),
        ("Elsewhere".to_string(), (None, Some(2.0))),
    ]
    .into_iter()
    .collect::<std::collections::HashMap<String, (Option<f64>, Option<f64>)>>();
    let held = [("Elsewhere".to_string(), (2i64, vec!["Teysa".to_string()]))]
        .into_iter()
        .collect();
    let styles = crate::output::Styles::colorless();
    let text = to_buy_block_text(&styles, &slots, &prices, &held);
    // Fully-owned "Full" never appears; Pricy (1×10) leads, then the
    // $2-total tie (Cheap 2×1 sorts before Elsewhere 1×2 by name),
    // unpriced last.
    let lines: Vec<&str> = text.lines().collect();
    let order: Vec<&str> = ["Pricy", "Cheap", "Elsewhere", "Unpriced"]
        .iter()
        .map(|name| {
            lines
                .iter()
                .position(|l| l.contains(name))
                .map(|p| (p, *name))
                .unwrap()
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    assert_eq!(order, ["Pricy", "Cheap", "Elsewhere", "Unpriced"]);
    assert!(text.contains("(1 copy in Teysa)"), "holder reason: {text}");
    assert!(text.contains("(not owned)"), "plain rows note the reason");
    assert!(
        text.contains("8 copies, 4 unpriced · est. $14.00 USD"),
        "summary line: {text}"
    );
}

#[test]
fn to_buy_block_empty_when_nothing_missing() {
    let slots = [("Owned".to_string(), slot(1, 0, 0, 0))]
        .into_iter()
        .collect();
    let styles = crate::output::Styles::colorless();
    let text = to_buy_block_text(&styles, &slots, &Default::default(), &Default::default());
    assert!(text.is_empty(), "no missing slots, no block: {text:?}");
}
