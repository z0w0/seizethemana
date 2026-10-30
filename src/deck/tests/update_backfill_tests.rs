use super::*;

#[test]
fn backfill_basics_reaches_target_size() {
    // A trimmed commander deck backfills to 100 cards with an on-color
    // basic (Breya's identity starts with W → Plains).
    let (tmp, conn) = seeded_conn();
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Breya', 'o1', '', 0, 'Legendary Creature — Human', '[]', '[\"W\",\"U\",\"B\",\"R\"]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's1', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &[],
        &[],
        None,
        false,
        false,
        false,
        false,
        true,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    let deck = crate::deck::Deck::parse(&text).unwrap();
    assert_eq!(deck.total(), 100, "backfill reaches the commander size");
    let plains: i64 = deck
        .entries()
        .filter(|e| e.name == "Plains")
        .map(|e| e.quantity)
        .sum();
    assert_eq!(
        plains, 98,
        "the first identity color's basic fills the deficit"
    );
}

#[test]
fn backfill_ignores_sideboard_when_counting_deficit() {
    // The sideboard is a wishlist: a 95-card maindeck with 15 sideboard
    // cards must still backfill 5 basics.
    let (tmp, conn) = seeded_conn();
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Breya', 'o1', '', 0, 'Legendary Creature — Human', '[]', '[\"W\"]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's1', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n94 Plains\n// SIDEBOARD\n15 Bolt\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &[],
        &[],
        None,
        false,
        false,
        false,
        false,
        true,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    let deck = crate::deck::Deck::parse(&text).unwrap();
    assert_eq!(
        deck.maindeck_total(),
        100,
        "the deficit counts maindeck cards only"
    );
    assert_eq!(deck.sideboard_total(), 15, "sideboard is untouched");
}

#[test]
fn dry_run_backfill_previews_backfilled_deck() {
    // In dry-run, the backfill applies to the preview clone so --legal
    // judges the end state; the deck file stays untouched.
    let (tmp, conn) = seeded_conn();
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Breya', 'o1', '', 0, 'Legendary Creature — Human', '[]', '[\"W\"]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's1', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Plains', 'o4', '', 0, 'Basic Land — Plains', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's4', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &[],
        &[],
        None,
        false,
        true,
        false,
        true,
        true,
        false,
    )
    .unwrap();
    assert_eq!(
        code,
        crate::cli::codes::OK,
        "the backfilled preview deck is legal"
    );
    let text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    assert_eq!(
        text, "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n",
        "dry-run still writes nothing"
    );
}

#[test]
fn backfill_falls_back_to_wastes_when_identity_unresolvable() {
    // A commander missing from the oracle (or with an unparseable
    // identity): the backfill warns and uses Wastes instead of failing.
    let (tmp, conn) = seeded_conn();
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .unwrap();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Ghost Commander\n// DECK\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &[],
        &[],
        None,
        false,
        false,
        false,
        false,
        true,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    let deck = crate::deck::Deck::parse(&text).unwrap();
    let wastes: i64 = deck
        .entries()
        .filter(|e| e.name == "Wastes")
        .map(|e| e.quantity)
        .sum();
    assert_eq!(wastes, 99, "unresolvable identity backfills with Wastes");
}
