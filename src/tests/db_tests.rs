use super::*;

fn sample_card(conn: &Connection, name: &str) {
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, power, toughness, loyalty, oracle_text,
            rarity, edhrec_rank, legalities, set_code, collector_number,
            scryfall_id, released_at)
         VALUES (?1, 'oid', '{R}', 1.0, 'Instant', '[\"R\"]', '[\"R\"]', '[]',
            NULL, NULL, NULL, 'Deal 3 damage', 'uncommon', 42,
            '{\"modern\":\"legal\"}', '3ED', '200', 'id-1', '2006-10-01')",
        rusqlite::params![name],
    )
    .expect("insert");
}

#[test]
fn load_all_cards_maps_every_column() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    sample_card(&conn, "Lightning Bolt");

    let cards = load_all_cards(&conn).expect("load");
    assert_eq!(cards.len(), 1);
    let c = &cards[0];
    assert_eq!(c.name, "Lightning Bolt");
    assert_eq!(c.mana_cost, "{R}");
    assert_eq!(c.cmc, 1.0);
    assert_eq!(c.type_line, "Instant");
    assert_eq!(c.colors, "[\"R\"]");
    assert_eq!(c.power, None);
    assert_eq!(c.oracle_text, "Deal 3 damage");
    assert_eq!(c.rarity, "uncommon");
    assert_eq!(c.edhrec_rank, Some(42));
    assert_eq!(c.set_code, "3ED");
    assert_eq!(c.collector_number, "200");
    assert_eq!(c.scryfall_id, "id-1");
    assert_eq!(c.released_at, "2006-10-01");
    assert_eq!(c.game_changer, None);
    let legalities: serde_json::Value = serde_json::from_str(&c.legalities).unwrap();
    assert_eq!(legalities["modern"], serde_json::json!("legal"));
}

#[test]
fn load_all_cards_preserves_order_and_handles_many() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    for name in ["A Card", "B Card", "C Card"] {
        sample_card(&conn, name);
    }
    let cards = load_all_cards(&conn).expect("load");
    let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["A Card", "B Card", "C Card"]);
}

#[test]
fn load_all_cards_empty_db() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    assert!(load_all_cards(&conn).expect("load").is_empty());
}

#[test]
fn open_creates_schema_and_is_reopenable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("test.db");

    let conn = open(&db_path).expect("first open");
    let (n,): (i64,) = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name IN
                ('cards','collection','card_prints','sets','tags',
                 'card_tags','token_names','combos','combo_pieces')",
            [],
            |row| Ok((row.get(0).unwrap(),)),
        )
        .expect("query");
    assert_eq!(n, 9);
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("version");
    assert_eq!(version, 1);
    // The game_changer column must exist on the cards table.
    let gc: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name = 'game_changer'",
            [],
            |r| r.get(0),
        )
        .expect("pragma");
    assert_eq!(gc, 1);
    // Universe metadata belongs to the current schema.
    let ub: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('card_prints')
             WHERE name = 'universes_beyond'",
            [],
            |r| r.get(0),
        )
        .expect("pragma");
    assert_eq!(ub, 1);
    let franchise: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('sets') WHERE name = 'franchise'",
            [],
            |r| r.get(0),
        )
        .expect("pragma");
    assert_eq!(franchise, 1);
    // Sell signals belong to the current schema.
    let signals: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('cards')
             WHERE name IN ('penny_rank', 'reserved')",
            [],
            |r| r.get(0),
        )
        .expect("pragma");
    assert_eq!(signals, 2);
    drop(conn);

    // Reopening must not fail on existing schema.
    let conn = open(&db_path).expect("reopen");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get::<_, i64>(0))
            .expect("count"),
        0
    );
}

#[test]
fn card_name_is_unique() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES ('Bolt', 'x1')",
        [],
    )
    .expect("insert");
    assert!(
        conn.execute(
            "INSERT INTO cards (name, oracle_id) VALUES ('Bolt', 'x2')",
            []
        )
        .is_err()
    );
}

#[test]
fn get_card_requires_exact_name() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    sample_card(&conn, "Lightning Bolt");
    assert!(get_card(&conn, "Lightning Bolt").expect("get").is_some());
    assert!(get_card(&conn, "lightning bolt").expect("get").is_none());
}

#[test]
fn resolve_name_falls_back_to_case_then_prefix() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    sample_card(&conn, "Lightning Bolt");
    sample_card(&conn, "Lightning Helix");
    sample_card(&conn, "Lightning Strike");

    match resolve_name(&conn, "lightning bolt").expect("resolve") {
        NameMatch::Found(c) => assert_eq!(c.name, "Lightning Bolt"),
        other => panic!("expected case-insensitive hit, got {other:?}"),
    }
    match resolve_name(&conn, "Lightning B").expect("resolve") {
        NameMatch::Found(c) => assert_eq!(c.name, "Lightning Bolt"),
        other => panic!("expected unique prefix, got {other:?}"),
    }
    match resolve_name(&conn, "Lightning").expect("resolve") {
        NameMatch::Ambiguous { candidates, total } => {
            assert_eq!(candidates.len(), 3);
            assert_eq!(total, 3);
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
    assert!(matches!(
        resolve_name(&conn, "Nope").expect("resolve"),
        NameMatch::NotFound
    ));
    // Wildcards in user input are literal.
    assert!(matches!(
        resolve_name(&conn, "Lightning %").expect("resolve"),
        NameMatch::NotFound
    ));
}

#[test]
fn resolve_name_resolves_flavor_name_aliases() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    sample_card(&conn, "Zilortha, Light of Ikoria");
    // An alias (whole string and unique prefix) resolves to the
    // oracle card of the print carrying it.
    let alias = |scryfall_id: &str, flavor: &str| {
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, flavor_name, updated_at)
             VALUES (?1, 'Zilortha, Light of Ikoria', 'iko', '1', 'en',
                'rare', '[\"nonfoil\"]', '2020-01-01', ?2, 'now')",
            rusqlite::params![scryfall_id, flavor],
        )
        .expect("insert print");
    };
    alias("godzilla", "Godzilla, King of the Monsters");
    match resolve_name(&conn, "Godzilla, King of the Monsters").expect("resolve alias") {
        NameMatch::Found(c) => assert_eq!(c.name, "Zilortha, Light of Ikoria"),
        other => panic!("expected whole-string alias hit, got {other:?}"),
    }
    match resolve_name(&conn, "Godzilla, King").expect("resolve alias prefix") {
        NameMatch::Found(c) => assert_eq!(c.name, "Zilortha, Light of Ikoria"),
        other => panic!("expected unique alias prefix, got {other:?}"),
    }
    // A card whose own name and its alias both prefix-match still
    // resolves (the dedup collapses the two paths to one card).
    match resolve_name(&conn, "Zilortha").expect("resolve shared prefix") {
        NameMatch::Found(c) => assert_eq!(c.name, "Zilortha, Light of Ikoria"),
        other => panic!("expected deduped single match, got {other:?}"),
    }
}

#[test]
fn resolve_name_flags_ambiguous_alias_prefix() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    sample_card(&conn, "Zilortha, Light of Ikoria");
    sample_card(&conn, "Zilortha, Apex of Ikoria");
    // Two oracle cards behind one alias prefix ("Godzilla,") must
    // report ambiguity, not resolve to the alphabetically first.
    let alias = |scryfall_id: &str, card: &str, flavor: &str| {
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, flavor_name, updated_at)
             VALUES (?1, ?2, 'iko', '1', 'en',
                'rare', '[\"nonfoil\"]', '2020-01-01', ?3, 'now')",
            rusqlite::params![scryfall_id, card, flavor],
        )
        .expect("insert print");
    };
    alias(
        "g1",
        "Zilortha, Light of Ikoria",
        "Godzilla, King of the Monsters",
    );
    alias(
        "g2",
        "Zilortha, Apex of Ikoria",
        "Godzilla, Primeval Champion",
    );
    match resolve_name(&conn, "Godzilla, ").expect("resolve shared alias prefix") {
        NameMatch::Ambiguous { candidates, total } => {
            assert_eq!(total, 2);
            assert_eq!(
                candidates,
                vec!["Zilortha, Apex of Ikoria", "Zilortha, Light of Ikoria"]
            );
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
    // The whole-string match still resolves to its own card.
    match resolve_name(&conn, "Godzilla, Primeval Champion").expect("resolve exact alias") {
        NameMatch::Found(c) => assert_eq!(c.name, "Zilortha, Apex of Ikoria"),
        other => panic!("expected exact alias hit, got {other:?}"),
    }
}

#[test]
fn collection_key_spans_binder_and_foil_kind() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    let insert = |binder: &str, foil: &str, qty: i64| -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity)
             VALUES ('Bolt', 'STA', '1', ?2, ?1, 'binder', ?3)",
            rusqlite::params![binder, foil, qty],
        )
    };
    insert("Collect", "normal", 2).expect("insert");
    // Foil kind and location make distinct rows.
    insert("Collect", "foil", 1).expect("insert");
    insert("Deck Box", "normal", 1).expect("insert");
    assert!(insert("Collect", "normal", 5).is_err());
    // binder_type is constrained.
    assert!(
        conn.execute(
            "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type)
             VALUES ('Bolt', 'STA', '1', 'normal', 'X', 'list')",
            [],
        )
        .is_err()
    );
}

#[test]
fn fts_query_quotes_terms_and_skips_punctuation_only() {
    assert_eq!(
        fts_query("lightning bolt"),
        Some("\"lightning\" OR \"bolt\"".to_string())
    );
    assert_eq!(
        fts_query("a  b\tc"),
        Some("\"a\" OR \"b\" OR \"c\"".to_string())
    );
    assert_eq!(
        fts_query_with_operator("lightning bolt", FtsTermOperator::All),
        Some("\"lightning\" AND \"bolt\"".to_string())
    );
    assert_eq!(
        fts_query("\"quoted\" name"),
        Some("\"\"\"quoted\"\"\" OR \"name\"".to_string())
    );
    assert_eq!(fts_query(""), None);
    assert_eq!(fts_query("   ... !!! ---   "), None);
}

#[test]
fn fts_index_stays_synced_with_cards() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    sample_card(&conn, "Lightning Bolt");
    // Stemmed term matches through the porter tokenizer.
    assert_eq!(
        fts_search(&conn, "\"lightning\" OR \"bolt\"", 10)
            .expect("fts")
            .len(),
        1
    );
    assert!(fts_search(&conn, "\"helix\"", 10).expect("fts").is_empty());

    // Update rewrites the index entry (old tokens vanish, new ones rank).
    conn.execute(
        "UPDATE cards SET oracle_text = 'Hurl helix of flame' WHERE name = 'Lightning Bolt'",
        [],
    )
    .expect("update");
    assert!(fts_search(&conn, "\"helix\"", 10).expect("fts").len() == 1);
    assert!(fts_search(&conn, "\"damage\"", 10).expect("fts").is_empty());

    // Delete removes the row from the index.
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .expect("delete");
    assert!(fts_search(&conn, "\"helix\"", 10).expect("fts").is_empty());
}

/// Name restrictions preserve the unrestricted BM25 scores and tie order.
#[test]
fn fts_allowed_names_preserve_weights_scores_and_ties() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    for name in [
        "Needle First",
        "Needle Popular",
        "Needle Last",
        "Oracle Only",
    ] {
        sample_card(&conn, name);
    }
    conn.execute(
        "UPDATE cards SET edhrec_rank = 1 WHERE name = 'Needle Popular'",
        [],
    )
    .expect("set popularity");
    conn.execute(
        "UPDATE cards SET oracle_text = 'needle needle damage' WHERE name = 'Oracle Only'",
        [],
    )
    .expect("set oracle match");
    let allowed = ["Needle First".to_string(), "Needle Last".to_string()]
        .into_iter()
        .collect();
    let expression = "\"needle\" AND \"damage\"";
    for weights in [FTS_COLUMN_WEIGHTS, [1.0, 7.0, 2.0, 9.0]] {
        let sql = format!(
            "SELECT cards.id, cards_fts.rank FROM cards_fts
             JOIN cards ON cards.id = cards_fts.rowid
             WHERE cards_fts MATCH ?1
             ORDER BY bm25(cards_fts, {}, {}, {}, {}),
                 cards.edhrec_rank IS NULL, cards.edhrec_rank, cards.id
             LIMIT ?2",
            weights[0], weights[1], weights[2], weights[3],
        );
        let mut stmt = conn.prepare(&sql).expect("prepare unrestricted reference");
        let expected: Vec<(i64, f64)> = stmt
            .query_map(rusqlite::params![expression, 10], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("query unrestricted reference")
            .collect::<Result<_, _>>()
            .expect("read unrestricted reference");
        assert_eq!(expected.len(), 4);
        assert_eq!(
            fts_search_with_weights(&conn, expression, 10, weights).expect("unrestricted FTS"),
            expected
        );
        if weights == FTS_COLUMN_WEIGHTS {
            assert_eq!(
                fts_search(&conn, expression, 10).expect("default FTS"),
                expected
            );
            let tied_ids: Vec<_> = expected
                .iter()
                .map(|hit| hit.0)
                .filter(|id| *id <= 3)
                .collect();
            assert_eq!(tied_ids, [2, 1, 3]);
        }
        let restricted: Vec<_> = expected
            .into_iter()
            .filter(|(id, _)| [1, 3].contains(id))
            .collect();
        for limit in [0, 1, 10] {
            assert_eq!(
                fts_search_with_allowed_names(&conn, expression, limit, weights, Some(&allowed))
                    .expect("restricted FTS"),
                restricted[..limit.min(restricted.len())]
            );
        }
    }
}

/// JSON name parameters handle quotes and sets larger than SQLite's bind limit.
#[test]
fn fts_allowed_names_handle_quotes_large_sets_and_empty_sets() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open(&tmp.path().join("t.db")).expect("open");
    sample_card(&conn, "Needle First");
    let name = r#"Needle O'Brien "Quote" \ Path"#;
    sample_card(&conn, name);
    let allowed_id = conn.last_insert_rowid();
    let expression = "\"needle\"";
    let unrestricted = fts_search(&conn, expression, 10).expect("unrestricted FTS");
    let expected = *unrestricted
        .iter()
        .find(|hit| hit.0 == allowed_id)
        .expect("quoted card hit");
    let mut allowed: std::collections::HashSet<String> =
        (0..33_000).map(|i| format!("Missing Card {i}")).collect();
    allowed.insert(name.to_string());
    assert_eq!(
        fts_search_with_allowed_names(&conn, expression, 1, FTS_COLUMN_WEIGHTS, Some(&allowed))
            .expect("quoted name in large set"),
        vec![expected]
    );
    allowed.clear();
    assert!(
        fts_search_with_allowed_names(&conn, expression, 1, FTS_COLUMN_WEIGHTS, Some(&allowed))
            .expect("empty name restriction")
            .is_empty()
    );
    allowed.insert(name.to_lowercase());
    assert!(
        fts_search_with_allowed_names(&conn, expression, 1, FTS_COLUMN_WEIGHTS, Some(&allowed))
            .expect("exact canonical names")
            .is_empty()
    );
    assert_eq!(
        fts_search_with_allowed_names(&conn, expression, 10, FTS_COLUMN_WEIGHTS, None)
            .expect("unrestricted FTS after restricted calls"),
        unrestricted
    );
}
