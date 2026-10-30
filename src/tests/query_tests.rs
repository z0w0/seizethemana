use super::*;

/// Small named vectors for shared-search tests.
struct TestVectors {
    names: Vec<String>,
    rows: Vec<Vec<f32>>,
    dim: usize,
}

impl VectorRowProvider for TestVectors {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn card_name(&self, row: usize) -> Option<&str> {
        self.names.get(row).map(String::as_str)
    }

    fn vector(&self, row: usize) -> Option<&[f32]> {
        self.rows.get(row).map(Vec::as_slice)
    }
}

/// Insert one card with text and color fields for search tests.
fn seed_search_card(conn: &rusqlite::Connection, name: &str, text: &str, colors: &str) {
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at)
         VALUES (?1, ?2, '{1}{U}', 2, 'Instant', ?3, ?3, '[]', ?4,
            'uncommon', '{}', 'tst', '1', ?2, '2020-01-01')",
        rusqlite::params![name, format!("oid-{name}"), colors, text],
    )
    .expect("insert search card");
}

/// Build two-dimensional rows with known scores against the first query axis.
fn test_vectors(cards: &[CardRow], first_scores: &[f32]) -> TestVectors {
    TestVectors {
        names: cards.iter().map(|card| card.name.clone()).collect(),
        rows: first_scores
            .iter()
            .map(|score| vec![*score, 1.0 - score])
            .collect(),
        dim: 2,
    }
}

#[test]
fn expansion_appends_tag_vocabulary() {
    let expanded = expanded_text("mana ramp");
    assert!(expanded.starts_with("mana ramp"), "{expanded}");
    assert!(expanded.contains("ramp"), "{expanded}");
    assert!(expanded.contains("mana acceleration"), "{expanded}");
    // No trigger: text passes through unchanged.
    assert_eq!(expanded_text("Lightning Bolt"), "Lightning Bolt");
    // Multiple triggers stack, each expansion once.
    let expanded = expanded_text("cheap counterspell");
    assert!(expanded.contains("counter target spell"), "{expanded}");
    // A trigger inside a larger phrase still expands.
    let expanded = expanded_text("board wipe");
    assert!(expanded.contains("sweeper"), "{expanded}");
    assert!(expanded.contains("destroy all creatures"), "{expanded}");
    // Word boundaries: "ramp" must not match inside "trample"; a query
    // of "trample" does trigger its own entry.
    let expanded = expanded_text("gives trample");
    assert!(!expanded.contains("mana acceleration"), "{expanded}");
    assert!(expanded.contains("evasion"), "{expanded}");
    // Punctuation and case do not block matching.
    let expanded = expanded_text("Board-Wipe!!");
    assert!(expanded.contains("sweeper"), "{expanded}");
}

#[test]
fn fuse_rrf_rewards_consensus() {
    // A card ranked well on both legs beats a card ranked first on one.
    let fts = vec![("shared".to_string(), 0.0), ("fts_only".to_string(), 0.0)];
    let vector = vec![
        ("vector_only".to_string(), 0.0),
        ("shared".to_string(), 0.0),
    ];
    let fused = fuse_rrf(&fts, &vector, 10, |_| None);
    assert_eq!(fused[0].0, "shared");
    // Scores are normalized to [0, 1].
    assert!((0.0..=1.0).contains(&fused[0].1));
}

#[test]
fn fuse_rrf_normalizes_top_score() {
    // First on both legs: (1/(k+1) + 1/(k+1)) / (2/(k+1)) == 1.0.
    let fts = vec![("bolt".to_string(), 0.0)];
    let vector = vec![("bolt".to_string(), 0.0)];
    let fused = fuse_rrf(&fts, &vector, 1, |_| None);
    assert!((fused[0].1 - 1.0).abs() < 1e-6);
}

#[test]
fn fuse_rrf_ties_break_toward_popularity_then_name() {
    // zebra (fts #1) and apple (vector #1) tie at 1/(k+1); loved
    // (vector #2) and plain (fts #2) tie at 1/(k+2).
    let fts = vec![("zebra".to_string(), 0.0), ("plain".to_string(), 0.0)];
    let vector = vec![("apple".to_string(), 0.0), ("loved".to_string(), 0.0)];
    let fused = fuse_rrf(&fts, &vector, 10, |name| match name {
        "loved" => Some(5),
        _ => None,
    });
    // Top tie: no EDHREC data either side -> alphabetical.
    assert_eq!(fused[0].0, "apple");
    assert_eq!(fused[1].0, "zebra");
    // Second tie: EDHREC-ranked card first.
    assert_eq!(fused[2].0, "loved");
    assert_eq!(fused[3].0, "plain");
}

#[test]
fn fuse_rrf_respects_limit() {
    let fts: Vec<(String, f64)> = (0..30).map(|i| (format!("card{i}"), 0.0)).collect();
    assert_eq!(fuse_rrf(&fts, &[], 5, |_| None).len(), 5);
}

#[test]
fn fuse_rrf_k_softens_top_ranks() {
    // A larger k compresses the score gap between rank 1 and rank 2.
    let fts: Vec<(String, f64)> = (0..5).map(|i| (format!("card{i}"), 0.0)).collect();
    let tight = fuse_rrf_k(&fts, &[], 5, 1.0, |_| None);
    let soft = fuse_rrf_k(&fts, &[], 5, 500.0, |_| None);
    let gap = |list: &[(String, f32)]| list[0].1 - list[1].1;
    assert!(gap(&tight) > gap(&soft), "larger k narrows the gap");
}

/// Fusion preserves weighted scores, popularity ties, and exact limits.
#[test]
fn borrowed_fusion_matches_owned_reference() {
    let fts: Vec<_> = (0..75).map(|i| (format!("Card {i}"), 0.0)).collect();
    let vector: Vec<_> = (20..100)
        .rev()
        .map(|i| (format!("Card {i}"), 0.0))
        .collect();
    let popularity = |name: &str| {
        name.strip_prefix("Card ")
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| value % 3 == 0)
    };
    for (fts, vector) in [
        (&fts[..], &vector[..]),
        (&fts[..], &[][..]),
        (&[][..], &vector[..]),
        (&[][..], &[][..]),
    ] {
        for (k, fts_weight, vector_weight) in [(1.0, 1.0, 1.0), (60.0, 1.0, 0.5), (500.0, 0.5, 1.0)]
        {
            let mut scores = std::collections::BTreeMap::<String, f64>::new();
            for (hits, weight) in [(fts, fts_weight), (vector, vector_weight)] {
                for (rank, (name, _)) in hits.iter().enumerate() {
                    *scores.entry(name.clone()).or_default() += weight / (k + rank as f64 + 1.0);
                }
            }
            let max_score = (fts_weight + vector_weight) / (k + 1.0);
            let mut expected: Vec<_> = scores
                .into_iter()
                .map(|(name, score)| (name, (score / max_score) as f32))
                .collect();
            expected.sort_by(|a, b| {
                b.1.total_cmp(&a.1)
                    .then_with(|| {
                        popularity(&a.0)
                            .unwrap_or(i64::MAX)
                            .cmp(&popularity(&b.0).unwrap_or(i64::MAX))
                    })
                    .then_with(|| a.0.cmp(&b.0))
            });
            for limit in [0, 1, 20, 100, 200] {
                let actual =
                    fuse_weighted_rrf(fts, vector, limit, k, fts_weight, vector_weight, popularity);
                assert_eq!(actual, expected[..limit.min(expected.len())]);
            }
        }
    }
}

#[test]
fn query_json_is_the_full_card_shape_plus_score() {
    let card = CardRow {
        name: "Bolt".into(),
        oracle_id: "oid".into(),
        mana_cost: "{R}".into(),
        cmc: 1.0,
        type_line: "Instant".into(),
        colors: r#"["R"]"#.into(),
        color_identity: r#"["R"]"#.into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: "Deal 3".into(),
        rarity: "uncommon".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: "TST".into(),
        collector_number: "1".into(),
        scryfall_id: "sid-1".into(),
        released_at: String::new(),
        game_changer: None,
        penny_rank: None,
        reserved: None,
    };
    let empty_tags = crate::tags::TagIndex::default();
    let range = crate::prints::PrintRange {
        cheapest: Some(crate::prints::Print {
            scryfall_id: "sid-1".into(),
            name: "Bolt".into(),
            set_code: "tst".into(),
            set_name: "Test".into(),
            collector_number: "1".into(),
            lang: "en".into(),
            finishes: vec!["nonfoil".into()],
            released_at: "2020-01-01".into(),
            usd: Some(0.99),
            usd_foil: Some(4.5),
            usd_etched: None,
        }),
        priciest: None,
        cheapest_foil: None,
        priciest_foil: None,
    };
    let v = serde_json::to_value(crate::card::card_json(
        &card,
        &empty_tags,
        &range,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    ))
    .unwrap();
    // The full contract: every CardRow field an agent joins on.
    for key in [
        "name",
        "oracle_id",
        "mana_cost",
        "mana_value",
        "type_line",
        "colors",
        "color_identity",
        "keywords",
        "power",
        "toughness",
        "loyalty",
        "oracle_text",
        "rarity",
        "edhrec_rank",
        "legalities",
        "game_changer",
        "set",
        "collector_number",
        "scryfall_id",
        "released_at",
        "tags",
        "price",
        "price_foil",
        "max_price",
        "max_price_foil",
        "owned",
        "available",
    ] {
        assert!(v.get(key).is_some(), "missing {key}");
    }
    assert_eq!(v["oracle_id"], "oid");
    assert_eq!(v["price"], 0.99);
    // An unpriced card renders null, not a missing field.
    let v = serde_json::to_value(crate::card::card_json(
        &card,
        &empty_tags,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    ))
    .unwrap();
    assert_eq!(v["price"], serde_json::Value::Null);
}

#[test]
fn names_under_price_keeps_at_or_under_cap() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    for (name, usd, foil) in [
        ("Cheap Bolt", Some(1.0_f64), None),
        ("Pricy Bolt", Some(9.0), None),
        ("Unpriced Bolt", None, None),
        ("Foil Only", None, Some(1.5_f64)),
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '{1}{R}', 1, 'Instant', '[]', '[]', '[]',
                'bolt deals 3 damage', 'common', '{}', 'tst', '1', 'sid',
                '2020-01-01')",
            rusqlite::params![name, format!("oid-{name}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
             VALUES (?1, ?2, 'm11', '148', 'en', 'common', '[\"nonfoil\"]',
                '2020-01-01', ?3, ?4, 't')",
            rusqlite::params![format!("sid-{name}"), name, usd, foil],
        )
        .unwrap();
    }
    let under = crate::prints::names_under_price(&conn, 2.0).unwrap();
    assert_eq!(
        under,
        vec!["Cheap Bolt".to_string(), "Foil Only".to_string()],
        "unpriced excluded; foil-only prices at its foil print"
    );
    // A tighter cap drops the foil-only card.
    let under = crate::prints::names_under_price(&conn, 1.0).unwrap();
    assert_eq!(under, vec!["Cheap Bolt".to_string()]);
}

/// Seed name matches with one extra card for restricted-search tests.
fn seed_fts_store(n: usize) -> (tempfile::TempDir, rusqlite::Connection, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().to_path_buf());
    let conn = crate::db::open(&paths.db()).unwrap();
    for i in 0..n {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '{1}{R}', 1, 'Instant', '[]', '[]', '[]',
                'common filler text', 'common', '{}', 'tst', '1', 'sid',
                '2020-01-01')",
            rusqlite::params![format!("Filler Bolt {i}"), format!("oid-{i}")],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at)
         VALUES ('Special Bolt', 'oid-special', '{1}{R}', 1, 'Instant', '[]',
            '[]', '[]', 'common filler text', 'common', '{}', 'tst', '1',
            'sid', '2020-01-01')",
        [],
    )
    .unwrap();
    (dir, conn, paths)
}

/// The command search keeps the allowed FTS match without a vector store.
#[test]
fn run_search_finds_restricted_fts_survivor() {
    let (_dir, conn, paths) = seed_fts_store(200);
    // An empty vector store satisfies the store gate; the vector leg
    // contributes nothing, so the test does not need an embedding model.
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 0,
        embedded_cards: 0,
        model: "m".into(),
        dim: embed::DIM,
        names: vec![],
        scryfall_synced_at: String::new(),
        doc_version: embed::DOC_VERSION,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    std::fs::write(paths.root().join("vectors.bin"), Vec::<u8>::new()).unwrap();

    // The SQL name restriction excludes the other 200 FTS matches.
    let restrict: std::collections::HashSet<String> =
        ["Special Bolt".to_string()].into_iter().collect();
    let filters = CardFilters::default();
    let hits = run_search(
        &paths,
        &conn,
        &Output::new(true, false, false),
        "bolt",
        &filters,
        10,
        Some(&restrict),
    )
    .unwrap();
    assert_eq!(hits.len(), 1, "the restriction keeps the matching card");
    assert_eq!(hits[0].card.name, "Special Bolt");
}

#[test]
fn shared_search_maps_sparse_ids_with_restricted_fts() {
    let (_dir, conn, _) = seed_fts_store(200);
    conn.execute("DELETE FROM cards WHERE name = 'Filler Bolt 3'", [])
        .unwrap();
    let cards = db::load_all_cards(&conn).unwrap();
    let vectors = TestVectors {
        names: Vec::new(),
        rows: Vec::new(),
        dim: 0,
    };
    let restrict: std::collections::HashSet<String> =
        ["Special Bolt".to_string()].into_iter().collect();
    let results = search_with_vectors(
        &conn,
        &cards,
        &vectors,
        &[],
        "bolt",
        &CardFilters::default(),
        1,
        Some(&restrict),
    )
    .unwrap();
    assert_eq!(
        results
            .fts
            .iter()
            .map(|hit| hit.card.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Special Bolt"]
    );
    assert_eq!(results.hybrid[0].card.name, "Special Bolt");
}

/// Owned FTS matches remain visible beyond every unrestricted retry window.
#[test]
fn shared_search_restricts_fts_before_the_largest_sql_window() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = db::open(&dir.path().join("t.db")).expect("open");
    let transaction = conn.transaction().expect("start fixture transaction");
    for i in 0..8_000 {
        seed_search_card(
            &transaction,
            &format!("Needle {i:04}"),
            "shared effect",
            "[]",
        );
    }
    seed_search_card(&transaction, "Needle Last", "shared effect", "[]");
    let owned_id = transaction.last_insert_rowid();
    transaction.commit().expect("commit fixture");
    let cards = db::load_all_cards(&conn).expect("load cards");
    let settings = SearchSettings {
        fts_candidate_depth: 30,
        ..SearchSettings::default()
    };
    let max_window = (1..settings.fts_max_rounds).fold(
        (settings.fts_candidate_depth * settings.fts_overfetch_multiplier)
            .max(settings.fts_min_sql_rows),
        |window, _| window * settings.fts_overfetch_multiplier,
    );
    assert_eq!(max_window, 7_680);
    let expression = db::fts_query("needle").expect("FTS expression");
    let unrestricted = db::fts_search(&conn, &expression, 8_001).expect("all FTS matches");
    assert_eq!(unrestricted.len(), 8_001);
    assert_eq!(unrestricted.last().expect("last FTS match").0, owned_id);
    assert!(
        unrestricted[..max_window]
            .iter()
            .all(|hit| hit.0 != owned_id)
    );
    let vectors = TestVectors {
        names: Vec::new(),
        rows: Vec::new(),
        dim: 0,
    };
    let restrict = ["Needle Last".to_string()].into_iter().collect();
    let filters = CardFilters {
        rarity: Some("uncommon".into()),
        ..CardFilters::default()
    };
    let results = search_with_settings(
        &conn,
        &cards,
        &vectors,
        &[],
        "needle",
        &filters,
        1,
        Some(&restrict),
        &settings,
    )
    .expect("restricted search");
    assert_eq!(results.fts.len(), 1);
    assert_eq!(results.fts[0].card.name, "Needle Last");
    assert!(results.vector.is_empty());
    assert_eq!(results.hybrid.len(), 1);
    assert_eq!(results.hybrid[0].card.name, "Needle Last");
}

/// Restricting FTS names does not require vector candidates to match FTS text.
#[test]
fn shared_search_keeps_owned_vector_only_matches_with_quoted_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = db::open(&dir.path().join("t.db")).expect("open");
    let quoted_name = r#"Owner's "Needle" \ Card"#;
    seed_search_card(&conn, "Unowned Needle", "needle effect", "[]");
    seed_search_card(&conn, quoted_name, "needle effect", "[]");
    seed_search_card(&conn, "Owned Vector", "unrelated effect", "[]");
    let cards = db::load_all_cards(&conn).expect("load cards");
    let vectors = test_vectors(&cards, &[1.0, 0.6, 0.9]);
    let restrict = [quoted_name.to_string(), "Owned Vector".to_string()]
        .into_iter()
        .collect();
    let settings = SearchSettings {
        fts_candidate_depth: 1,
        vector_candidate_depth: 1,
        fts_overfetch_multiplier: 1,
        fts_min_sql_rows: 1,
        fts_max_rounds: 1,
        ..SearchSettings::default()
    };
    let results = search_with_settings(
        &conn,
        &cards,
        &vectors,
        &[1.0, 0.0],
        "needle",
        &CardFilters::default(),
        1,
        Some(&restrict),
        &settings,
    )
    .expect("restricted search");
    assert_eq!(results.fts.len(), 1);
    assert_eq!(results.fts[0].card.name, quoted_name);
    assert_eq!(results.vector.len(), 1);
    assert_eq!(results.vector[0].card.name, "Owned Vector");
    assert!(
        results
            .hybrid
            .iter()
            .all(|hit| restrict.contains(&hit.card.name))
    );
    let unrestricted = search_with_settings(
        &conn,
        &cards,
        &vectors,
        &[1.0, 0.0],
        "needle",
        &CardFilters::default(),
        1,
        None,
        &settings,
    )
    .expect("unrestricted search");
    assert_eq!(unrestricted.fts[0].card.name, "Unowned Needle");
    assert_eq!(unrestricted.vector[0].card.name, "Unowned Needle");
}

/// An empty name restriction excludes both retrieval legs and fused results.
#[test]
fn shared_search_with_empty_restriction_returns_no_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = db::open(&dir.path().join("t.db")).expect("open");
    seed_search_card(&conn, "Needle Card", "needle effect", "[]");
    let cards = db::load_all_cards(&conn).expect("load cards");
    let vectors = test_vectors(&cards, &[1.0]);
    let restrict = std::collections::HashSet::new();
    let results = search_with_vectors(
        &conn,
        &cards,
        &vectors,
        &[1.0, 0.0],
        "needle",
        &CardFilters::default(),
        1,
        Some(&restrict),
    )
    .expect("empty restricted search");
    assert!(results.fts.is_empty());
    assert!(results.vector.is_empty());
    assert!(results.hybrid.is_empty());
}

#[test]
fn shared_search_filters_before_vector_top_k() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db::open(&dir.path().join("t.db")).unwrap();
    seed_search_card(&conn, "Red Ramp One", "ramp spell", r#"["R"]"#);
    seed_search_card(&conn, "Blue Ramp", "ramp spell", r#"["U"]"#);
    seed_search_card(&conn, "Red Ramp Two", "ramp spell", r#"["R"]"#);
    let cards = db::load_all_cards(&conn).unwrap();
    let vectors = test_vectors(&cards, &[1.0, 0.8, 0.7]);
    let filters = CardFilters {
        color: Some("U".into()),
        ..CardFilters::default()
    };
    let settings = SearchSettings {
        fts_candidate_depth: 1,
        vector_candidate_depth: 1,
        ..SearchSettings::default()
    };
    let results = search_with_settings(
        &conn,
        &cards,
        &vectors,
        &[1.0, 0.0],
        "ramp",
        &filters,
        1,
        None,
        &settings,
    )
    .unwrap();
    assert_eq!(results.fts[0].card.name, "Blue Ramp");
    assert_eq!(results.vector[0].card.name, "Blue Ramp");
    assert_eq!(results.hybrid[0].card.name, "Blue Ramp");
}

/// Confirms experimental FTS matching and RRF weights affect shared search.
#[test]
fn shared_search_exposes_fts_term_mode_and_weighted_fusion() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db::open(&dir.path().join("t.db")).unwrap();
    seed_search_card(&conn, "Vector Card", "unrelated effect", "[]");
    seed_search_card(&conn, "FTS Card", "needle shared effect", "[]");
    seed_search_card(&conn, "Needle Only", "needle effect", "[]");
    conn.execute(
        "UPDATE cards SET edhrec_rank = 1 WHERE name = 'Vector Card'",
        [],
    )
    .unwrap();
    let cards = db::load_all_cards(&conn).unwrap();
    let vectors = test_vectors(&cards, &[1.0, 0.8, 0.7]);
    let defaults = SearchSettings {
        fts_candidate_depth: 1,
        vector_candidate_depth: 1,
        ..SearchSettings::default()
    };
    let baseline = search_with_settings(
        &conn,
        &cards,
        &vectors,
        &[1.0, 0.0],
        "needle shared",
        &CardFilters::default(),
        1,
        None,
        &defaults,
    )
    .unwrap();
    assert_eq!(baseline.hybrid[0].card.name, "Vector Card");

    let settings = SearchSettings {
        fts_candidate_depth: 1,
        vector_candidate_depth: 1,
        fts_term_operator: db::FtsTermOperator::All,
        fts_rrf_weight: 2.0,
        vector_rrf_weight: 0.5,
        ..SearchSettings::default()
    };
    let results = search_with_settings(
        &conn,
        &cards,
        &vectors,
        &[1.0, 0.0],
        "needle shared",
        &CardFilters::default(),
        1,
        None,
        &settings,
    )
    .unwrap();

    assert_eq!(
        results
            .fts
            .iter()
            .map(|hit| hit.card.name.as_str())
            .collect::<Vec<_>>(),
        vec!["FTS Card"]
    );
    assert_eq!(results.hybrid[0].card.name, "FTS Card");
}

#[test]
fn shared_search_handles_punctuation_and_empty_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db::open(&dir.path().join("t.db")).unwrap();
    seed_search_card(&conn, "Needle Card", "needle ability", "[]");
    seed_search_card(&conn, "Other Card", "other ability", "[]");
    let cards = db::load_all_cards(&conn).unwrap();
    let vectors = test_vectors(&cards, &[0.9, 0.2]);
    let punctuation = search_with_vectors(
        &conn,
        &cards,
        &vectors,
        &[1.0, 0.0],
        "!!!???",
        &CardFilters::default(),
        1,
        None,
    )
    .unwrap();
    assert!(punctuation.fts.is_empty());
    assert_eq!(punctuation.vector[0].card.name, "Needle Card");
    assert_eq!(punctuation.hybrid[0].card.name, "Needle Card");

    let empty = TestVectors {
        names: Vec::new(),
        rows: Vec::new(),
        dim: 0,
    };
    let fts_only = search_with_vectors(
        &conn,
        &cards,
        &empty,
        &[],
        "needle",
        &CardFilters::default(),
        1,
        None,
    )
    .unwrap();
    assert_eq!(fts_only.fts[0].card.name, "Needle Card");
    assert!(fts_only.vector.is_empty());
    assert_eq!(fts_only.hybrid[0].card.name, "Needle Card");
}

#[test]
fn shared_search_rejects_vector_alignment_and_dimension_errors() {
    let dir = tempfile::tempdir().unwrap();
    let conn = db::open(&dir.path().join("t.db")).unwrap();
    seed_search_card(&conn, "Needle Card", "needle ability", "[]");
    let cards = db::load_all_cards(&conn).unwrap();
    let wrong_name = TestVectors {
        names: vec!["Other Card".into()],
        rows: vec![vec![1.0, 0.0]],
        dim: 2,
    };
    assert!(
        search_with_vectors(
            &conn,
            &cards,
            &wrong_name,
            &[1.0, 0.0],
            "needle",
            &CardFilters::default(),
            1,
            None,
        )
        .is_err()
    );

    let vectors = test_vectors(&cards, &[1.0]);
    let error = search_with_vectors(
        &conn,
        &cards,
        &vectors,
        &[1.0],
        "needle",
        &CardFilters::default(),
        1,
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("dimension"), "{error}");

    let wrong_row_dim = TestVectors {
        names: vec!["Needle Card".into()],
        rows: vec![vec![1.0]],
        dim: 2,
    };
    assert!(
        search_with_vectors(
            &conn,
            &cards,
            &wrong_row_dim,
            &[1.0, 0.0],
            "needle",
            &CardFilters::default(),
            1,
            None,
        )
        .is_err()
    );
}

/// A store shorter than `cards` must error at runtime, not silently
/// score mismatched rows (the old debug_assert path).
#[test]
fn run_search_errors_on_misaligned_store() {
    let (_dir, conn, paths) = seed_fts_store(3);
    // Store only two of the three cards: row order drifts from the
    // card table.
    let cards = db::load_all_cards(&conn).unwrap();
    let mut store = VectorStore::new();
    for card in &cards[..2] {
        store.push(&card.name, vec![0.0; embed::DIM]).unwrap();
    }
    store.save_vectors(paths.root()).unwrap();
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: store.len(),
        embedded_cards: store.len(),
        model: store.meta.model.clone(),
        dim: embed::DIM,
        names: store.meta.names.clone(),
        scryfall_synced_at: String::new(),
        doc_version: embed::DOC_VERSION,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();

    let filters = CardFilters::default();
    let result = run_search(
        &paths,
        &conn,
        &Output::new(true, false, false),
        "bolt",
        &filters,
        10,
        None,
    );
    assert!(result.is_err(), "misalignment must be a runtime error");
}

// expanded_text

#[test]
fn expanded_text_caps_expansion_words_at_24() {
    // A query loaded with triggers expands until the 24-word bulk cap:
    // the expansion tail must never drown the query's own words.
    let trigger_rich = "counter scry draw treasure sacrifice discard";
    let expanded = crate::query::expanded_text(trigger_rich);
    let expansion_words =
        expanded.split_whitespace().count() - trigger_rich.split_whitespace().count();
    assert!(
        expansion_words <= 24,
        "expansion bulk capped at 24 words, got {expansion_words}"
    );
    // The original text always survives intact at the head.
    assert!(expanded.starts_with(trigger_rich));
}

#[test]
fn expanded_text_without_triggers_is_identity() {
    let text = "smashing pumpkins";
    assert_eq!(crate::query::expanded_text(text), text);
}

#[test]
fn fusion_depth_floors_at_twenty() {
    assert_eq!(fusion_depth(1), 20);
    assert_eq!(fusion_depth(20), 20);
    assert_eq!(fusion_depth(50), 50);
    // The floor keeps filters from starving the fused window on tight
    // queries; above the floor the limit is the depth.
}
