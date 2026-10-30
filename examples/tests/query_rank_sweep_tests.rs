use super::*;

#[test]
fn sweep_variants_change_one_setting_at_a_time() {
    let variants = variants();
    assert_eq!(variants.len(), 26);
    assert_eq!(variants[0].name, "current-default");
    let defaults = query::DEFAULT_SEARCH_SETTINGS;
    for variant in variants.iter().skip(1) {
        let weight_changes = variant
            .settings
            .fts_column_weights
            .iter()
            .zip(defaults.fts_column_weights)
            .filter(|(actual, expected)| **actual != *expected)
            .count();
        let fts_depth_changed =
            variant.settings.fts_candidate_depth != defaults.fts_candidate_depth;
        let vector_depth_changed =
            variant.settings.vector_candidate_depth != defaults.vector_candidate_depth;
        let rrf_changed = variant.settings.rrf_k != defaults.rrf_k;
        let fts_rrf_weight_changed = variant.settings.fts_rrf_weight != defaults.fts_rrf_weight;
        let vector_rrf_weight_changed =
            variant.settings.vector_rrf_weight != defaults.vector_rrf_weight;
        let fts_operator_changed = variant.settings.fts_term_operator != defaults.fts_term_operator;
        let fts_overfetch_changed =
            variant.settings.fts_overfetch_multiplier != defaults.fts_overfetch_multiplier;
        let fts_rounds_changed = variant.settings.fts_max_rounds != defaults.fts_max_rounds;
        let fts_min_rows_changed = variant.settings.fts_min_sql_rows != defaults.fts_min_sql_rows;
        assert_eq!(
            weight_changes
                + usize::from(fts_depth_changed)
                + usize::from(vector_depth_changed)
                + usize::from(rrf_changed)
                + usize::from(fts_rrf_weight_changed)
                + usize::from(vector_rrf_weight_changed)
                + usize::from(fts_operator_changed)
                + usize::from(fts_overfetch_changed)
                + usize::from(fts_rounds_changed)
                + usize::from(fts_min_rows_changed),
            1,
            "{} changed more than one setting",
            variant.name
        );
    }
}

#[test]
fn candidate_recall_at_twenty_counts_positive_cards() {
    let item = GoldenQuery {
        query: "ramp".into(),
        relevant: vec!["A".into(), "B".into()],
        not_relevant: Vec::new(),
        filters: None,
        note: String::new(),
        group: Some("role_intent".into()),
    };
    let ranked: Vec<String> = (0..19)
        .map(|index| format!("noise {index}"))
        .chain(["A".into(), "B".into()])
        .collect();
    assert_eq!(candidate_recall20(&ranked, &item), 0.5);
}

/// A positive vector hit after twenty FTS misses still belongs to their union.
#[test]
fn union_recall_keeps_vector_hits_after_the_first_twenty_names() {
    let item = GoldenQuery {
        query: "ramp".into(),
        relevant: vec!["Relevant vector hit".into()],
        not_relevant: Vec::new(),
        filters: None,
        note: String::new(),
        group: None,
    };
    let fts: Vec<String> = (0..20).map(|index| format!("noise {index}")).collect();
    let vector = vec!["Relevant vector hit".into(), fts[0].clone()];
    let union = common::pool_names([fts.as_slice(), vector.as_slice()]);
    assert_eq!(union.len(), 21);
    assert_eq!(candidate_recall20(&fts, &item), 0.0);
    assert_eq!(candidate_recall20(&vector, &item), 1.0);
    assert_eq!(candidate_recall(&union, &item), 1.0);
}

#[test]
fn tie_on_development_scores_keeps_current_defaults() {
    let default = VariantReport {
        name: "current-default".into(),
        changed_parameter: "none".into(),
        value: "default".into(),
        settings: settings_info(query::DEFAULT_SEARCH_SETTINGS),
        summary: summary_with_score(0.5, 0.4),
        mean_shared_search_us: 10,
        paired_queries: Vec::new(),
    };
    let alternate = VariantReport {
        name: "rrf-k-30".into(),
        changed_parameter: "rrf_k".into(),
        value: "30".into(),
        settings: settings_info(query::SearchSettings {
            rrf_k: 30.0,
            ..query::DEFAULT_SEARCH_SETTINGS
        }),
        summary: summary_with_score(0.5, 0.4),
        mean_shared_search_us: 10,
        paired_queries: Vec::new(),
    };
    assert_eq!(select_best_variant(&[default, alternate]), 0);
}

/// Confirms rebuild timing uses a temporary database copy.
#[test]
fn fts_rebuild_measurement_leaves_the_source_database_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let conn = db::open(&directory.path().join("source.db")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at)
         VALUES ('Needle Card', 'needle-id', '', 0, 'Artifact', '[]', '[]',
            '[]', 'needle text', 'common', '{}', 'tst', '1', 'needle-print',
            '2020-01-01')",
        [],
    )
    .unwrap();

    let measured = measure_fts_rebuild(&conn).unwrap();
    assert!(measured.database_snapshot_us > 0);
    assert_eq!(db::fts_search(&conn, "\"needle\"", 10).unwrap().len(), 1);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM cards", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

fn summary_with_score(ndcg: f64, mrr: f64) -> Summary {
    let score = LegSummary {
        recall: 0.5,
        mrr,
        ndcg,
        candidate_recall20: 0.5,
    };
    Summary {
        query_count: 1,
        fts: score,
        vector: score,
        hybrid: score,
        union_candidate_recall20: 0.5,
        by_group: BTreeMap::new(),
    }
}
