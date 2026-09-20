use super::*;
use rusqlite::Connection;

fn seeded_conn() -> (tempfile::TempDir, Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    (tmp, conn)
}

fn insert_card(
    conn: &Connection,
    name: &str,
    type_line: &str,
    text: &str,
    cmc: f64,
    game_changer: Option<bool>,
    edhrec: Option<i64>,
) {
    // A generic-only cost of exactly `cmc` keeps readiness targets aligned
    // with CMC (no pips to color-screw a single-color fixture deck).
    let cost = format!("{{{}}}", cmc);
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES (?1, 'oid-'||?1, ?2, ?3, ?4, '[]', '[]', '[]', ?5,
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 'sid-'||?1,
            '2020-01-01', ?6, ?7)",
        rusqlite::params![name, cost, cmc, type_line, text, game_changer, edhrec],
    )
    .unwrap();
}

fn insert_banned(conn: &Connection, name: &str) {
    conn.execute(
        "UPDATE cards SET legalities = '{\"commander\":\"banned\"}' WHERE name = ?1",
        rusqlite::params![name],
    )
    .unwrap();
}

fn paths_with_deck(deck_text: &str) -> (tempfile::TempDir, crate::paths::Paths) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("TestDeck"), deck_text).unwrap();
    (tmp, paths)
}

fn insert_island(conn: &Connection, qty: usize) {
    for _ in 0..qty {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at, game_changer, edhrec_rank)
             VALUES ('Island', 'oid-Island', '', 0, 'Basic Land — Island', '[]', '[]', '[]',
                '({T}: Add {U}.)', 'common', '{\"commander\":\"legal\"}',
                'tst', '1', 'sid-Island', '2020-01-01', NULL, NULL)",
            [],
        )
        .unwrap();
    }
}

/// Insert a card with an explicit mana cost (for pip-based fixtures).
fn insert_card_cost(
    conn: &Connection,
    name: &str,
    type_line: &str,
    text: &str,
    mana_cost: &str,
    cmc: f64,
) {
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES (?1, 'oid-'||?1, ?2, ?3, ?4, '[]', '[]', '[]', ?5,
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 'sid-'||?1,
            '2020-01-01', NULL, NULL)",
        rusqlite::params![name, mana_cost, cmc, type_line, text],
    )
    .unwrap();
}

fn silent_out() -> crate::output::Output {
    crate::output::Output::new(true, false, false)
}

/// Load the test deck and compute its cut rows (the test-facing core).
fn rows_for(
    conn: &mut Connection,
    deck_text: &str,
    role: Option<Role>,
    count: usize,
    bracket: Option<u8>,
) -> Vec<CutRow> {
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(conn, &deck);
    cut_rows(conn, &deck, &cards_by_name, role, count, bracket).unwrap()
}

#[test]
fn dead_cards_rank_before_curve_outliers_and_basics_commander_skipped() {
    let (_tmp, mut conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
        5.0,
        None,
        Some(10),
    );
    // An off-color 8-drop: the deck's islands never pay {G}, so it never
    // becomes castable — the dead-card fixture (both a castability fault
    // and a curve outlier).
    insert_card_cost(
        &conn,
        "Big Dumb Finisher",
        "Creature — Giant",
        "Haste.",
        "{6}{G}{G}",
        8.0,
    );
    // A live cheap spell: no fault, never suggested.
    insert_card(
        &conn,
        "Cheap Draw",
        "Instant",
        "Draw a card.",
        2.0,
        None,
        Some(50),
    );
    insert_island(&conn, 1);
    let rows = rows_for(
        &mut conn,
        "// COMMANDER\n1 Test Commander\n// DECK\n1 Big Dumb Finisher\n1 Cheap Draw\n37 Island\n",
        None,
        5,
        Some(3),
    );
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["Big Dumb Finisher"],
        "only the dead card is suggested; the live filler is not"
    );
    assert!(
        rows[0].reasons.iter().any(|r| r.kind == "castability"),
        "the dead card carries a castability reason"
    );
}

#[test]
fn reactive_removal_and_board_discount_exempt_from_castability() {
    let (_tmp, mut conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
        5.0,
        None,
        Some(10),
    );
    // Reactive removal: never fires in a goldfish, so its castability row
    // is low — but it must be exempt from the castability reason.
    insert_card(
        &conn,
        "Heroic Reaction",
        "Instant",
        "Destroy target creature.",
        2.0,
        None,
        Some(50),
    );
    // Improvise: board-discount card, casts far earlier in real games.
    insert_island(&conn, 1);
    insert_card(
        &conn,
        "Improvise Wall",
        "Artifact Creature — Wall",
        "Improvise. Defender.",
        9.0,
        None,
        Some(900),
    );
    let rows = rows_for(
        &mut conn,
        "// COMMANDER\n1 Test Commander\n// DECK\n1 Heroic Reaction\n1 Improvise Wall\n10 Island\n",
        None,
        5,
        Some(3),
    );
    for row in &rows {
        assert!(
            !row.reasons.iter().any(|r| r.kind == "castability"),
            "{} is exempt from the castability reason",
            row.name
        );
    }
    // Improvise Wall still shows a curve reason (CMC 9 vs median 2) but
    // never a castability one; Heroic Reaction shows nothing at all.
    assert!(rows.iter().all(|r| r.name != "Heroic Reaction"));
    let wall = rows.iter().find(|r| r.name == "Improvise Wall");
    if let Some(wall) = wall {
        assert!(wall.reasons.iter().any(|r| r.kind == "curve"));
    }
}

#[test]
fn illegal_and_over_cap_gc_pin_to_top_regardless_of_score() {
    let (_tmp, mut conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
        5.0,
        None,
        Some(10),
    );
    // A dead 8-drop (high expendability score) that is also banned.
    insert_card(
        &conn,
        "Banned Brute",
        "Creature — Beast",
        "Haste.",
        8.0,
        None,
        Some(900),
    );
    insert_banned(&conn, "Banned Brute");
    // Four legal Game Changers (under the bracket-3 cap of 3? no: 4 > 3,
    // so the fourth pins) plus a cheap live spell.
    for i in 0..4 {
        insert_card(
            &conn,
            &format!("GC Card {i}"),
            "Artifact",
            "Sacrifice: effect.",
            2.0,
            Some(true),
            Some(100 + i),
        );
    }
    insert_island(&conn, 1);
    insert_card(
        &conn,
        "Cheap Draw",
        "Instant",
        "Draw a card.",
        2.0,
        None,
        Some(50),
    );
    let deck_text = format!(
        "// COMMANDER\n1 Test Commander\n// DECK\n1 Banned Brute\n{}1 Cheap Draw\n10 Island\n",
        (0..4)
            .map(|i| format!("1 GC Card {i}\n"))
            .collect::<String>()
    );
    let rows = rows_for(&mut conn, &deck_text, None, 10, Some(3));
    // Pins (banned + the over-cap GC) come before any scored row.
    assert!(rows[0].pinned, "first row is a hard pin");
    let pinned_names: Vec<&str> = rows
        .iter()
        .filter(|r| r.pinned)
        .map(|r| r.name.as_str())
        .collect();
    assert!(
        pinned_names.contains(&"Banned Brute"),
        "the banned card is pinned"
    );
    // Exactly one GC is over the bracket-3 cap (4 in deck, cap 3).
    assert_eq!(
        pinned_names
            .iter()
            .filter(|n| n.starts_with("GC Card"))
            .count(),
        1,
        "one over-cap Game Changer pins"
    );
    // The unpinned row follows every pin.
    assert!(
        rows.iter().skip_while(|r| r.pinned).all(|r| !r.pinned),
        "pinned rows precede scored rows"
    );
    // Ranks are 1-based and sequential in render order.
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.rank, i + 1, "rank {i} is sequential");
    }
}

#[test]
fn for_role_pairs_fills_and_discounts_serving_cards() {
    let (_tmp, conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "Draw a card at upkeep.",
        5.0,
        None,
        Some(10),
    );
    // An incumbent draw source (serves the --for role).
    insert_card(
        &conn,
        "Repeatable Draw Engine",
        "Enchantment",
        "At the beginning of your upkeep, draw a card.",
        3.0,
        None,
        Some(80),
    );
    // A dead filler to cut.
    insert_card(
        &conn,
        "Big Dumb Finisher",
        "Creature — Giant",
        "Haste.",
        8.0,
        None,
        Some(500),
    );
    // Tagged cards for the fill query.
    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('t1', 'card-draw', 'card draw', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO card_tags (oracle_id, tag_id) VALUES ('oid-Binder Draw Spell', 't1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Binder Draw Spell', 'oid-Binder Draw Spell', '{2}', 2.0, 'Instant',
            '[]', '[]', '[]', 'Draw a card.', 'common', '{\"commander\":\"legal\"}',
            'tst', '1', 'sid-Binder Draw Spell', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity)
         VALUES ('Cheap Draw', 'binder', 'binder', 2)",
        [],
    )
    .unwrap();
    insert_card(
        &conn,
        "Cheap Draw",
        "Instant",
        "Draw a card.",
        2.0,
        None,
        Some(50),
    );
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Repeatable Draw Engine\n1 Big Dumb Finisher\n10 Island\n";
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck);
    let rows = cut_rows(&conn, &deck, &cards_by_name, Some(Role::Draw), 5, Some(3)).unwrap();
    // The serving draw engine is not suggested for cutting.
    assert!(rows.iter().all(|r| r.name != "Repeatable Draw Engine"));
    // Every cut row carries the fill pairing with the deficit role.
    for row in &rows {
        let pair = row.replace_with.as_ref().expect("fill pairing present");
        assert!(pair.role.contains("Draw"));
        assert!(
            !pair.candidates.is_empty(),
            "the tagged draw card is a fill candidate"
        );
    }
}

#[test]
fn json_shape_carries_reasons_score_and_pins() {
    let (_tmp, conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "Draw a card at upkeep.",
        5.0,
        None,
        Some(10),
    );
    insert_card(
        &conn,
        "Slow Wall",
        "Creature — Wall",
        "Defender. This spell costs {2} more to cast.",
        9.0,
        None,
        Some(900),
    );
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Slow Wall\n10 Island\n";
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck);
    let rows = cut_rows(&conn, &deck, &cards_by_name, None, 5, Some(3)).unwrap();
    assert_eq!(rows.len(), 1);
    let json = serde_json::to_value(&rows[0]).unwrap();
    assert!(json["name"].is_string());
    assert!(json["qty"].is_i64());
    assert!(json["score"].is_f64());
    assert!(json["pinned"].is_boolean());
    let reasons = json["reasons"].as_array().unwrap();
    assert!(!reasons.is_empty());
    for reason in reasons {
        assert!(reason["kind"].is_string());
        assert!(reason["detail"].is_string());
    }
}

#[test]
fn unknown_role_is_usage_error() {
    let (_tmp, mut conn) = seeded_conn();
    let (_tmp2, paths) = paths_with_deck("// DECK\n1 Bolt\n");
    let mut out = silent_out();
    let code = cuts(
        &paths,
        &mut conn,
        &mut out,
        "TestDeck",
        &CutOptions {
            count: 5,
            for_role: Some("nonsense"),
            json: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::USAGE);
}

#[test]
fn bracket_4_game_changers_never_pin_and_sideboard_gcs_do_not_count() {
    let (_tmp, mut conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
        5.0,
        None,
        Some(10),
    );
    // Five Game Changers: unlimited at bracket 4 (no pin), one over the
    // cap at bracket 3 (one pin).
    for i in 0..5 {
        insert_card(
            &conn,
            &format!("GC Card {i}"),
            "Artifact",
            "Sacrifice: effect.",
            2.0,
            Some(true),
            Some(100 + i),
        );
    }
    insert_island(&conn, 1);
    let deck_text = format!(
        "// COMMANDER\n1 Test Commander\n// DECK\n{}10 Island\n",
        (0..5)
            .map(|i| format!("1 GC Card {i}\n"))
            .collect::<String>()
    );
    let rows = rows_for(&mut conn, &deck_text, None, 10, Some(4));
    assert!(
        rows.iter().all(|r| !r.pinned),
        "bracket 4 has no GC cap, so nothing pins"
    );
    // The same deck at bracket 3 pins exactly the two over-cap GCs.
    let rows = rows_for(&mut conn, &deck_text, None, 10, Some(3));
    assert_eq!(
        rows.iter().filter(|r| r.pinned).count(),
        2,
        "5 GCs minus the bracket-3 cap of 3 pins 2"
    );
    // Sideboard GCs never count toward the census (they are a wishlist).
    let sideboard_text = format!("{deck_text}// SIDEBOARD\n1 GC Card 0\n1 GC Card 1\n");
    let rows = rows_for(&mut conn, &sideboard_text, None, 10, Some(3));
    assert_eq!(
        rows.iter().filter(|r| r.pinned).count(),
        2,
        "sideboard Game Changers do not add to the census"
    );
}
