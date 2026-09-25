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
    rows_for_format(conn, deck_text, role, count, bracket, None)
}

/// [`rows_for`] with an explicit format pin.
fn rows_for_format(
    conn: &mut Connection,
    deck_text: &str,
    role: Option<Role>,
    count: usize,
    bracket: Option<u8>,
    format: Option<&str>,
) -> Vec<CutRow> {
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(conn, &deck).unwrap();
    cut_rows(
        conn,
        &deck,
        &cards_by_name,
        role,
        count,
        bracket,
        format,
        None,
    )
    .unwrap()
}

#[test]
fn bench_cards_are_never_cut_candidates() {
    let (_tmp, mut conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "",
        4.0,
        None,
        Some(1),
    );
    // A weak spell in the maindeck and a clearly weaker one only on the
    // sideboard: the sideboard card must not surface as a cut (cutting
    // it frees no maindeck slot).
    insert_card_cost(&conn, "Deck Filler", "Instant", "", "{6}{U}{U}", 8.0);
    insert_card_cost(&conn, "Bench Filler", "Instant", "", "{9}{U}{U}", 10.0);
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n10 Deck Filler\n20 Island\n// SIDEBOARD\n4 Bench Filler\n";
    let rows = rows_for(&mut conn, deck_text, None, 10, None);
    assert!(
        !rows.iter().any(|r| r.name == "Bench Filler"),
        "sideboard cards must not be cut candidates: {:?}",
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()
    );
    assert!(
        rows.iter().any(|r| r.name == "Deck Filler"),
        "maindeck filler should be ranked: {:?}",
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()
    );
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
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let rows = cut_rows(
        &conn,
        &deck,
        &cards_by_name,
        Some(Role::Draw),
        5,
        Some(3),
        None,
        None,
    )
    .unwrap();
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
fn for_role_fills_respect_identity_and_legality() {
    // `--for` fills must stay inside the commander's color identity and
    // the deck's format: an off-identity or banned tagged card is never
    // a fill candidate, even when it is the most-owned match.
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
    conn.execute(
        "UPDATE cards SET color_identity = '[\"W\"]' WHERE name = 'Test Commander'",
        [],
    )
    .unwrap();
    insert_card(
        &conn,
        "Dead Filler",
        "Creature — Giant",
        "Haste.",
        8.0,
        None,
        Some(500),
    );
    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('t1', 'card-draw', 'card draw', 1)",
        [],
    )
    .unwrap();
    // Three tagged draw candidates: off-identity (G), banned in commander,
    // and a legal on-identity one.
    for (name, identity, legalities) in [
        ("Off Identity Draw", "[\"G\"]", "{\"commander\":\"legal\"}"),
        ("Banned Draw", "[]", "{\"commander\":\"banned\"}"),
        ("Good Draw", "[]", "{\"commander\":\"legal\"}"),
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at, game_changer, edhrec_rank)
             VALUES (?1, 'oid-'||?1, '{2}', 2.0, 'Instant', '[]', ?2, '[]', 'Draw a card.',
                'common', ?3, 'tst', '1', 'sid-'||?1, '2020-01-01', NULL, NULL)",
            rusqlite::params![name, identity, legalities],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_tags (oracle_id, tag_id) VALUES ('oid-'||?1, 't1')",
            rusqlite::params![name],
        )
        .unwrap();
    }
    // The off-identity and banned cards are the most-owned, so without the
    // gates they would win the "owned first" ordering.
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity, foil)
         VALUES ('Off Identity Draw', 'binder', 'binder', 5, 'normal')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity, foil)
         VALUES ('Banned Draw', 'binder', 'binder', 4, 'normal')",
        [],
    )
    .unwrap();
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Dead Filler\n10 Island\n";
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let rows = cut_rows(
        &conn,
        &deck,
        &cards_by_name,
        Some(Role::Draw),
        5,
        Some(3),
        None,
        None,
    )
    .unwrap();
    let candidates: Vec<String> = rows
        .iter()
        .filter_map(|r| r.replace_with.as_ref())
        .flat_map(|p| p.candidates.clone())
        .collect();
    assert!(
        !candidates.iter().any(|c| c.contains("Off Identity")),
        "off-identity cards are never fill candidates"
    );
    assert!(
        !candidates.iter().any(|c| c.contains("Banned")),
        "format-illegal cards are never fill candidates"
    );
    assert!(
        candidates.iter().any(|c| c == "Good Draw"),
        "the legal on-identity card fills the role"
    );
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
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let rows = cut_rows(&conn, &deck, &cards_by_name, None, 5, Some(3), None, None).unwrap();
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
fn modern_deck_does_not_pin_commander_banned_cards() {
    let (_tmp, mut conn) = seeded_conn();
    // A card banned in commander but legal in modern: a modern deck keeps
    // it, no pin.
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
        5.0,
        None,
        Some(10),
    );
    insert_card(
        &conn,
        "Commander Banned",
        "Creature — Beast",
        "Haste.",
        8.0,
        None,
        Some(900),
    );
    insert_banned(&conn, "Commander Banned");
    // Its modern legality stays legal.
    conn.execute(
        "UPDATE cards SET legalities = '{\"commander\":\"banned\",\"modern\":\"legal\"}'
         WHERE name = 'Commander Banned'",
        [],
    )
    .unwrap();
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Commander Banned\n10 Island\n";
    // Pinned format "modern": the banned-in-commander card is fine.
    let rows = rows_for_format(&mut conn, deck_text, None, 5, Some(3), Some("modern"));
    assert!(
        rows.iter()
            .all(|r| !(r.pinned && r.name == "Commander Banned")),
        "a modern deck does not pin commander bans"
    );
    // The same deck judged as commander pins the card.
    let rows = rows_for(&mut conn, deck_text, None, 5, Some(3));
    let banned = rows.iter().find(|r| r.name == "Commander Banned");
    assert!(
        banned.is_some_and(|r| r.pinned),
        "the commander deck pins the banned card"
    );
}

#[test]
fn sixty_card_cut_rows_carry_remove_qty() {
    let (_tmp, mut conn) = seeded_conn();
    // A 60-card deck (no COMMANDER section) with a 4-of curve outlier and
    // a 2-of curve outlier.
    for (name, mana, cmc) in [
        ("Curve Topend", "{7}{G}{G}", 9.0),
        ("Topend Pair", "{6}{G}{G}", 8.0),
    ] {
        insert_card_cost(&conn, name, "Creature — Giant", "Haste.", mana, cmc);
        conn.execute(
            "UPDATE cards SET legalities = '{\"commander\":\"legal\",\"modern\":\"legal\"}'
             WHERE name = ?1",
            rusqlite::params![name],
        )
        .unwrap();
    }
    insert_island(&conn, 1);
    let deck_text = "// DECK\n4 Curve Topend\n2 Topend Pair\n20 Island\n";
    let rows = rows_for_format(&mut conn, deck_text, None, 5, None, Some("modern"));
    let four_of = rows.iter().find(|r| r.name == "Curve Topend");
    assert_eq!(
        four_of.map(|r| r.remove_qty),
        Some(2),
        "a 3-4-of scored cut removes half (rounded up)"
    );
    let two_of = rows.iter().find(|r| r.name == "Topend Pair");
    assert_eq!(
        two_of.map(|r| r.remove_qty),
        Some(1),
        "a 2-of scored cut removes 1 copy"
    );
    assert!(
        four_of.is_none_or(|r| !r.pinned),
        "a curve outlier is a scored cut, not a pin"
    );
}

#[test]
fn sixty_card_deck_without_format_pins_only_non_60_legal_cards() {
    let (_tmp, mut conn) = seeded_conn();
    // A card legal in no 60-card format (commander-only): the default
    // unpinned gate must pin it.
    insert_card(
        &conn,
        "Commander Only",
        "Creature — Beast",
        "Haste.",
        8.0,
        None,
        Some(900),
    );
    // A card banned in commander but legal in modern: no pin, and the
    // unpinned 60-card path must not flag it as illegal.
    insert_card(
        &conn,
        "Commander Banned",
        "Creature — Beast",
        "Haste.",
        2.0,
        None,
        Some(50),
    );
    insert_banned(&conn, "Commander Banned");
    conn.execute(
        "UPDATE cards SET legalities = '{\"commander\":\"banned\",\"modern\":\"legal\"}'
         WHERE name = 'Commander Banned'",
        [],
    )
    .unwrap();
    // A card banned in every 60-card format: pins on the default path.
    insert_card(
        &conn,
        "Modern Banned Too",
        "Creature — Beast",
        "Haste.",
        8.0,
        None,
        Some(900),
    );
    conn.execute(
        "UPDATE cards SET legalities = '{\"commander\":\"legal\",\"modern\":\"banned\",\"legacy\":\"banned\",\"vintage\":\"banned\",\"pauper\":\"banned\",\"pioneer\":\"banned\",\"standard\":\"banned\"}'
         WHERE name = 'Modern Banned Too'",
        [],
    )
    .unwrap();
    insert_island(&conn, 1);
    let deck_text =
        "// DECK\n1 Commander Only\n1 Commander Banned\n1 Modern Banned Too\n40 Island\n";
    let rows = rows_for(&mut conn, deck_text, None, 10, None);
    let pinned: Vec<&str> = rows
        .iter()
        .filter(|r| r.pinned)
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(
        pinned,
        vec!["Commander Only", "Modern Banned Too"],
        "the default 60-card gate pins cards illegal in every 60-card format, and only those"
    );
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

#[test]
fn mono_color_deck_cuts_off_color_lands() {
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
    // Mono-B: the commander and a Swamp carry the deck's printed colors.
    conn.execute(
        "UPDATE cards SET colors = '[\"B\"]' WHERE name = 'Test Commander'",
        [],
    )
    .unwrap();
    // Mono-B deck holding a W/G fetch: zero intersection → off_color_land.
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Windswept Heath', 'oid-wh', '', 0, 'Land', '[]', '[]', '[]',
            '{T}, Pay 1 life: Search your library for a Plains or Forest card, put it onto the battlefield, then shuffle.',
            'rare', '{\"commander\":\"legal\"}', 'tst', '1', 'sid-wh', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    // A partial fetch for the deck's own color (Polluted Delta in mono-B).
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Polluted Delta', 'oid-pd', '', 0, 'Land', '[]', '[]', '[]',
            '{T}, Pay 1 life: Search your library for an Island or Swamp card, put it onto the battlefield, then shuffle.',
            'rare', '{\"commander\":\"legal\"}', 'tst', '1', 'sid-pd', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    insert_island(&conn, 1);
    conn.execute(
        "UPDATE cards SET colors = '[\"B\"]' WHERE name = 'Island'",
        [],
    )
    .unwrap();
    let deck_text =
        "// COMMANDER\n1 Test Commander\n// DECK\n1 Windswept Heath\n1 Polluted Delta\n10 Island\n";
    let rows = rows_for(&mut conn, deck_text, None, 10, None);
    let heath = rows.iter().find(|r| r.name == "Windswept Heath");
    let delta = rows.iter().find(|r| r.name == "Polluted Delta");
    let off_color = |r: Option<&CutRow>| {
        r.map(|row| {
            row.reasons
                .iter()
                .any(|reason| reason.kind == "off_color_land")
        })
        .unwrap_or(false)
    };
    assert!(
        off_color(heath),
        "a zero-overlap fetch carries off_color_land"
    );
    assert!(
        off_color(delta),
        "a partial fetch in a mono-color deck carries off_color_land"
    );
    assert!(
        heath.is_none_or(|r| !r.pinned),
        "off-color lands rank high but are not pinned"
    );
    let heath_score = heath.map(|r| r.score).unwrap_or(0.0);
    let delta_score = delta.map(|r| r.score).unwrap_or(0.0);
    assert!(
        heath_score > delta_score,
        "zero-overlap lands score above partial fetches ({heath_score} vs {delta_score})"
    );
}

#[test]
fn over_cap_gc_pins_unranked_before_ranked() {
    // Bracket 3 allows 3 Game Changers; a 4-GC deck keeps the ranked
    // (played) ones and pins the unranked (obscure) one.
    let (_tmp, mut conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "At the beginning of your upkeep, draw a card.",
        5.0,
        None,
        Some(1),
    );
    insert_card(
        &conn,
        "Ranked GC",
        "Sorcery",
        "Draw two cards.",
        4.0,
        Some(true),
        Some(2),
    );
    insert_card(
        &conn,
        "Ranked GC Two",
        "Sorcery",
        "Draw two cards.",
        4.0,
        Some(true),
        Some(3),
    );
    insert_card(
        &conn,
        "Ranked GC Three",
        "Sorcery",
        "Draw two cards.",
        4.0,
        Some(true),
        Some(4),
    );
    insert_card(
        &conn,
        "Unranked GC",
        "Sorcery",
        "Draw two cards.",
        4.0,
        Some(true),
        None,
    );
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Ranked GC\n1 Ranked GC Two\n1 Ranked GC Three\n1 Unranked GC\n10 Island\n";
    let rows = rows_for(&mut conn, deck_text, None, 10, Some(3));
    let pinned: Vec<&str> = rows
        .iter()
        .filter(|r| r.pinned)
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(
        pinned,
        ["Unranked GC"],
        "the unranked GC pins, the ranked one stays"
    );
}

#[test]
fn make_room_for_sideboard_pairs_role_matched_cuts() {
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
    conn.execute(
        "UPDATE cards SET color_identity = '[\"W\"]' WHERE name = 'Test Commander'",
        [],
    )
    .unwrap();
    // Maindeck: a dead finisher (expendable, Wincon role) and a draw
    // source (serves the deck, Draw role).
    insert_card(
        &conn,
        "Dead Finisher",
        "Creature — Giant",
        "Haste.",
        8.0,
        None,
        Some(500),
    );
    insert_card(
        &conn,
        "Live Draw",
        "Creature — Human",
        "Draw a card at upkeep.",
        3.0,
        None,
        Some(80),
    );
    // Sideboard: a legal on-identity draw spell and an off-identity one.
    for (name, identity) in [("Side Draw", "[]"), ("Off Side Draw", "[\"G\"]")] {
        insert_card(&conn, name, "Instant", "Draw a card.", 2.0, None, None);
        conn.execute(
            "UPDATE cards SET color_identity = ?2 WHERE name = ?1",
            rusqlite::params![name, identity],
        )
        .unwrap();
    }
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Dead Finisher\n1 Live Draw\n10 Island\n// SIDEBOARD\n1 Side Draw\n1 Off Side Draw\n";
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let (rows, _qualifying) = make_room_rows(
        &conn,
        &deck,
        &cards_by_name,
        crate::cli::MakeRoomFor::Sideboard,
        5,
        None,
        None,
    )
    .unwrap();
    // The off-identity sideboard card never asks for a slot.
    assert!(
        rows.iter().all(|r| r.bench_card != "Off Side Draw"),
        "off-identity sideboard cards are skipped"
    );
    let swap = rows
        .iter()
        .find(|r| r.bench_card == "Side Draw")
        .expect("the on-identity sideboard card pairs");
    assert_eq!(swap.cut_candidate, "Dead Finisher");
    assert!(
        swap.reasons
            .iter()
            .any(|r| r.contains("same role") || r.contains("expendable")),
        "the pairing explains itself"
    );
}

#[test]
fn make_room_for_sideboard_empty_sideboard_is_empty() {
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
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n10 Island\n";
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let (rows, _qualifying) = make_room_rows(
        &conn,
        &deck,
        &cards_by_name,
        crate::cli::MakeRoomFor::Sideboard,
        5,
        None,
        None,
    )
    .unwrap();
    assert!(rows.is_empty(), "no sideboard means no swaps");
}

#[test]
fn make_room_for_maybeboard_pairs_maybeboard_cards() {
    // The maybeboard zone pairs like the sideboard: on-identity, legal
    // maybeboard cards get a maindeck cut; off-identity ones are skipped.
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
    conn.execute(
        "UPDATE cards SET color_identity = '[\"W\"]' WHERE name = 'Test Commander'",
        [],
    )
    .unwrap();
    insert_card(
        &conn,
        "Dead Filler",
        "Creature — Giant",
        "Haste.",
        8.0,
        None,
        Some(500),
    );
    for (name, identity) in [("Maybe Draw", "[]"), ("Off Maybe", "[\"G\"]")] {
        insert_card(&conn, name, "Instant", "Draw a card.", 2.0, None, None);
        conn.execute(
            "UPDATE cards SET color_identity = ?2 WHERE name = ?1",
            rusqlite::params![name, identity],
        )
        .unwrap();
    }
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Dead Filler\n10 Island\n// MAYBEBOARD\n1 Maybe Draw\n1 Off Maybe\n";
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let (rows, _qualifying) = make_room_rows(
        &conn,
        &deck,
        &cards_by_name,
        crate::cli::MakeRoomFor::Maybeboard,
        5,
        None,
        None,
    )
    .unwrap();
    assert!(
        rows.iter().all(|r| r.bench_card != "Off Maybe"),
        "off-identity maybeboard cards are skipped"
    );
    assert!(
        rows.iter().any(|r| r.bench_card == "Maybe Draw"),
        "the on-identity maybeboard card pairs with a cut"
    );
    // The sideboard zone does not read maybeboard cards.
    let (rows, _qualifying) = make_room_rows(
        &conn,
        &deck,
        &cards_by_name,
        crate::cli::MakeRoomFor::Sideboard,
        5,
        None,
        None,
    )
    .unwrap();
    assert!(rows.is_empty(), "zones never cross");
}

#[test]
fn unpinned_sixty_card_deck_fills_and_swaps_respect_default_legality() {
    let (_tmp, conn) = seeded_conn();
    insert_card(
        &conn,
        "Modern Staple",
        "Instant",
        "Draw a card.",
        1.0,
        None,
        Some(1),
    );
    conn.execute(
        "UPDATE cards SET legalities = '{\"modern\":\"legal\",\"commander\":\"banned\"}' WHERE name = 'Modern Staple'",
        [],
    ).unwrap();
    insert_card(
        &conn,
        "Illegal Card",
        "Creature",
        "Haste.",
        1.0,
        None,
        Some(1),
    );
    conn.execute(
        "UPDATE cards SET legalities = '{\"modern\":\"banned\",\"standard\":\"banned\",\"pioneer\":\"banned\",\"legacy\":\"banned\",\"vintage\":\"banned\",\"pauper\":\"banned\"}' WHERE name = 'Illegal Card'",
        [],
    ).unwrap();

    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('t1', 'card-draw', 'card draw', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO card_tags (oracle_id, tag_id) VALUES ('oid-Modern Staple', 't1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO card_tags (oracle_id, tag_id) VALUES ('oid-Illegal Card', 't1')",
        [],
    )
    .unwrap();

    insert_island(&conn, 1);
    let deck_text = "// DECK\n1 Dead Filler\n10 Island";
    insert_card(
        &conn,
        "Dead Filler",
        "Creature",
        "Haste.",
        8.0,
        None,
        Some(500),
    );

    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();

    let rows = super::cut_rows(
        &conn,
        &deck,
        &cards_by_name,
        Some(super::Role::Draw),
        5,
        None,
        None,
        None,
    )
    .unwrap();

    let candidates: Vec<String> = rows
        .iter()
        .filter_map(|r| r.replace_with.as_ref())
        .flat_map(|p| p.candidates.clone())
        .collect();

    assert!(
        candidates.contains(&"Modern Staple".to_string()),
        "modern staple should be a fill candidate"
    );
    assert!(
        !candidates.contains(&"Illegal Card".to_string()),
        "illegal card should be dropped"
    );

    let deck_with_bench =
        "// DECK\n1 Dead Filler\n10 Island\n// MAYBEBOARD\n1 Modern Staple\n1 Illegal Card";
    let deck_b = super::super::Deck::parse(deck_with_bench).unwrap();
    let cards_b = super::super::stats::lookup_names(&conn, &deck_b).unwrap();

    let (room_rows, qualifying) = super::make_room_rows(
        &conn,
        &deck_b,
        &cards_b,
        crate::cli::MakeRoomFor::Maybeboard,
        5,
        None,
        None,
    )
    .unwrap();

    assert_eq!(qualifying, 1, "only the legal card qualifies");
    assert!(
        room_rows.iter().any(|r| r.bench_card == "Modern Staple"),
        "Modern Staple should pair"
    );
    assert!(
        !room_rows.iter().any(|r| r.bench_card == "Illegal Card"),
        "Illegal Card should be skipped"
    );
}

#[test]
fn for_fills_survive_max_price_below_three_candidates() {
    // Three tagged fill candidates, two priced over the cap: the fill
    // list cuts to the survivors (fewer than 3) instead of erroring or
    // listing over-cap names.
    let (_tmp, conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "",
        5.0,
        None,
        Some(10),
    );
    insert_card(
        &conn,
        "Filler",
        "Creature — Giant",
        "Haste.",
        8.0,
        None,
        Some(500),
    );
    insert_island(&conn, 1);
    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('t1', 'card-draw', 'card draw', 1)",
        [],
    )
    .unwrap();
    for (name, usd) in [
        ("Fill A", Some(1.0_f64)),
        ("Fill B", Some(50.0)),
        ("Fill C", None),
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at, game_changer, edhrec_rank)
             VALUES (?1, 'oid-'||?1, '{2}', 2.0, 'Instant', '[]', '[]', '[]',
                'Draw a card.', 'common', '{\"commander\":\"legal\"}',
                'tst', '1', 'sid-'||?1, '2020-01-01', NULL, NULL)",
            rusqlite::params![name],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_tags (oracle_id, tag_id) VALUES ('oid-'||?1, 't1')",
            rusqlite::params![name],
        )
        .unwrap();
        if let Some(usd) = usd {
            conn.execute(
                "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                    lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
                 VALUES ('sid-'||?1, ?1, 'm11', '148', 'en', 'common', '[\"nonfoil\"]',
                    '2020-01-01', ?2, NULL, 't')",
                rusqlite::params![name, usd],
            )
            .unwrap();
        }
    }
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Filler\n10 Island\n";
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let rows = cut_rows(
        &conn,
        &deck,
        &cards_by_name,
        Some(Role::Draw),
        5,
        None,
        None,
        Some(5.0),
    )
    .unwrap();
    for row in &rows {
        let pair = row.replace_with.as_ref().expect("fill pairing present");
        // Only the under-cap priced candidate survives the $5 cap;
        // unpriced and over-cap names are dropped.
        assert_eq!(pair.candidates, vec!["Fill A".to_string()]);
    }
}

#[test]
fn for_fills_empty_tags_still_pair_rows_with_empty_candidates() {
    // No tags for the role at all: cut rows still carry the pairing (the
    // candidates list is empty), and no error surfaces.
    let (_tmp, conn) = seeded_conn();
    insert_card(
        &conn,
        "Test Commander",
        "Legendary Creature — Human",
        "",
        5.0,
        None,
        Some(10),
    );
    insert_card(
        &conn,
        "Filler",
        "Creature — Giant",
        "Haste.",
        8.0,
        None,
        Some(500),
    );
    insert_island(&conn, 1);
    let deck_text = "// COMMANDER\n1 Test Commander\n// DECK\n1 Filler\n10 Island\n";
    let deck = super::super::Deck::parse(deck_text).unwrap();
    let cards_by_name = super::super::stats::lookup_names(&conn, &deck).unwrap();
    let rows = cut_rows(
        &conn,
        &deck,
        &cards_by_name,
        Some(Role::Draw),
        5,
        None,
        None,
        None,
    )
    .unwrap();
    for row in &rows {
        let pair = row.replace_with.as_ref().expect("fill pairing present");
        assert!(pair.candidates.is_empty(), "no tags, no candidates");
    }
}
