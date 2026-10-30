use super::cuts_tests::{insert_card, insert_island, seeded_conn};
use super::*;

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
    conn.execute("UPDATE cards SET legalities = '{\"modern\":\"legal\",\"commander\":\"banned\"}' WHERE name = 'Modern Staple'", []).unwrap();
    insert_card(
        &conn,
        "Illegal Card",
        "Creature",
        "Haste.",
        1.0,
        None,
        Some(1),
    );
    conn.execute("UPDATE cards SET legalities = '{\"modern\":\"banned\",\"standard\":\"banned\",\"pioneer\":\"banned\",\"legacy\":\"banned\",\"vintage\":\"banned\",\"pauper\":\"banned\"}' WHERE name = 'Illegal Card'", []).unwrap();
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
        assert_eq!(pair.candidates, vec!["Fill A".to_string()]);
    }
}

#[test]
fn for_fills_empty_tags_still_pair_rows_with_empty_candidates() {
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
