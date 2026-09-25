use super::*;

#[test]
fn candidate_prompts_match_model_cards() {
    let find = |key| {
        CANDIDATES
            .iter()
            .find(|candidate| candidate.key == key)
            .unwrap()
    };
    assert_eq!(find("bge-small").query_prompt, BGE_QUERY_PROMPT);
    assert_eq!(find("bge-small-full").query_prompt, BGE_QUERY_PROMPT);
    assert_eq!(find("bge-small-256").max_length, 256);
    assert_eq!(find("arctic-s").query_prompt, BGE_QUERY_PROMPT);
    assert_eq!(find("minilm-l6").query_prompt, "");
    assert_eq!(find("minilm-l12").pooling, "mean");
    assert_eq!(find("paraphrase-minilm-l12").pooling, "mean");
}

#[test]
fn candidate_dimensions_pooling_and_token_limits_match_model_registry() {
    for candidate in CANDIDATES {
        assert!(
            candidate.max_length <= candidate.model_max_length,
            "{}",
            candidate.key
        );
        let CandidateModel::Onnx(model) = &candidate.model;
        let info = fastembed::EmbeddingModel::get_model_info(model).unwrap();
        assert_eq!(candidate.dim, info.dim, "{}", candidate.key);
        let pooling =
            if candidate.key.starts_with("minilm") || candidate.key == "paraphrase-minilm-l12" {
                fastembed::Pooling::Mean
            } else {
                fastembed::Pooling::Cls
            };
        assert_eq!(
            fastembed::TextEmbedding::get_default_pooling_method(model),
            Some(pooling),
            "{}",
            candidate.key
        );
    }
}

#[test]
fn embedding_normalization_returns_unit_length_vector() {
    let mut vector = vec![2.0, 4.0, 6.0];
    normalize_embedding(&mut vector);
    assert!((vector.iter().map(|value| value * value).sum::<f32>() - 1.0).abs() < 1e-6);
}

#[test]
fn cache_identity_covers_prompts_model_and_corpus_order() {
    let candidate = CANDIDATES
        .iter()
        .find(|candidate| candidate.key == "minilm-l6")
        .unwrap();
    let cards = vec![db::CardRow {
        name: "Card".into(),
        oracle_id: "id".into(),
        mana_cost: "".into(),
        cmc: 0.0,
        type_line: "Instant".into(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: String::new(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: "tst".into(),
        collector_number: "1".into(),
        scryfall_id: "sid".into(),
        released_at: String::new(),
        game_changer: None,
    }];
    let original = cache_metadata(
        candidate,
        &cards,
        "fingerprint-a",
        DocumentFormat::Production,
        Backend::Cpu,
    );
    let other_backend = cache_metadata(
        candidate,
        &cards,
        "fingerprint-a",
        DocumentFormat::Production,
        Backend::CoreMl,
    );
    assert_ne!(original, other_backend);
    let mut changed = original.clone();
    changed.document_prompt = "other: ".into();
    assert_ne!(original, changed);
    changed = original.clone();
    changed.query_prompt = "other: ".into();
    assert_ne!(original, changed);
    changed = original.clone();
    changed.model = "different model".into();
    assert_ne!(original, changed);
    changed = original.clone();
    changed.model_fingerprint = Some("different weights".into());
    assert_ne!(original, changed);
    changed = original.clone();
    changed.corpus_fingerprint = "fingerprint-b".into();
    assert_ne!(original, changed);
    changed = original.clone();
    changed.dimension += 1;
    assert_ne!(original, changed);
    changed = original.clone();
    changed.max_length = 512;
    assert_ne!(original, changed);
    changed = original.clone();
    changed.document_version += 1;
    assert_ne!(original, changed);
    changed = original.clone();
    changed.document_format = "fielded".into();
    assert_ne!(original, changed);
}

#[test]
fn cache_roundtrip_and_corruption_detection() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("vectors.bin");
    let metadata = CacheMetadata {
        format_version: CACHE_FORMAT_VERSION,
        candidate_key: "test".into(),
        backend: "cpu".into(),
        model: "model".into(),
        model_fingerprint: None,
        dimension: 2,
        card_count: 2,
        max_length: 128,
        document_version: embed::DOC_VERSION,
        document_format: "production".into(),
        document_prompt: String::new(),
        query_prompt: String::new(),
        pooling: "CLS".into(),
        corpus_fingerprint: "corpus".into(),
    };
    let data = [1.0, 0.0, 0.0, 1.0];
    write_cache(&path, &metadata, &data).unwrap();
    let mut legacy_header = serde_json::to_value(&metadata).unwrap();
    legacy_header
        .as_object_mut()
        .unwrap()
        .remove("document_format");
    let legacy: CacheMetadata = serde_json::from_value(legacy_header).unwrap();
    assert_eq!(legacy.document_format, "production");
    let mut matrix = read_cache(&path, &metadata).unwrap().unwrap();
    matrix.names = vec!["a".into(), "b".into()];
    assert_eq!(matrix.dimension(), 2);
    assert_eq!(matrix.vector(1), Some([0.0, 1.0].as_slice()));
    let mut stale = metadata.clone();
    stale.query_prompt = "different".into();
    assert!(read_cache(&path, &stale).unwrap().is_none());
    std::fs::write(&path, b"broken").unwrap();
    assert!(read_cache(&path, &metadata).is_err());
}

#[test]
fn repeated_query_text_keeps_filter_cases_independent() {
    let queries: Vec<GoldenQuery> = serde_json::from_str(
        r#"[
            {"query":"bolt","relevant":["A"]},
            {"query":"bolt","relevant":["B"],"filters":{"rarity":"common"}}
        ]"#,
    )
    .unwrap();
    assert_eq!(queries.len(), 2);
    assert_ne!(
        queries[0].search_filters().unwrap(),
        queries[1].search_filters().unwrap()
    );
}

#[test]
fn production_bge_parity_gate_is_required_for_baseline_candidate() {
    assert_eq!(CANDIDATES[0].key, "bge-small");
    assert_eq!(REGRESSION_THRESHOLD, 0.05);
    assert!(
        CANDIDATES
            .iter()
            .all(|candidate| candidate.dim > 0 && candidate.max_length > 0)
    );
}

/// Keeps card facts visible across every supported document layout.
#[test]
fn document_formats_preserve_card_name_and_rules_text() {
    let card = db::CardRow {
        name: "Needle Adept".into(),
        oracle_id: "needle-oracle".into(),
        mana_cost: "{1}{R}".into(),
        cmc: 2.0,
        type_line: "Creature — Wizard".into(),
        colors: r#"["R"]"#.into(),
        color_identity: r#"["R"]"#.into(),
        keywords: r#"["Haste"]"#.into(),
        power: Some("2".into()),
        toughness: Some("1".into()),
        loyalty: None,
        oracle_text: "Draw a card.".into(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: "tst".into(),
        collector_number: "1".into(),
        scryfall_id: "needle-print".into(),
        released_at: String::new(),
        game_changer: None,
    };
    let tags = tags::TagIndex::default();
    let docs: Vec<String> = [
        DocumentFormat::Production,
        DocumentFormat::Fielded,
        DocumentFormat::Compact,
        DocumentFormat::RulesFirst,
        DocumentFormat::NameTypeRulesFirst,
        DocumentFormat::TagsFirst,
        DocumentFormat::TagsLast,
        DocumentFormat::Contextual,
    ]
    .into_iter()
    .map(|format| render_document(format, &card, &tags))
    .collect();
    assert!(docs[3].starts_with("Rules text:"));
    assert!(docs[4].starts_with("Name:"));
    assert_ne!(docs[3], docs[4]);
    assert_eq!(
        docs[0],
        "Needle Adept. Mana cost {1}{R}. Type Creature — Wizard. Keywords Haste. Colors R. Tags none. P/T 2/1. Rules text Draw a card."
    );
    for document in docs {
        assert!(document.contains("Needle Adept"), "{document}");
        assert!(document.contains("{1}{R}"), "{document}");
        assert!(document.contains("Creature — Wizard"), "{document}");
        assert!(document.contains("Haste"), "{document}");
        assert!(document.contains("Draw a card."), "{document}");
    }
    let candidate = CANDIDATES
        .iter()
        .find(|candidate| candidate.key == "bge-small")
        .unwrap();
    let input = candidate_document(candidate, DocumentFormat::Fielded, &card, &tags);
    assert!(input.starts_with("Name: Needle Adept"));
}

/// Confirms models, layouts, and ranking settings can be selected from flags.
#[test]
fn bakeoff_accepts_document_formats_and_ranking_settings() {
    let args = parse_args_from(
        [
            "--only",
            "minilm-l6",
            "--format",
            "fielded,compact",
            "--fts-weights",
            "8,2,2,1",
            "--fts-depth",
            "40",
            "--vector-depth",
            "30",
            "--rrf-k",
            "30",
            "--fts-rrf-weight",
            "1.5",
            "--vector-rrf-weight",
            "0.5",
            "--fts-terms",
            "all",
            "--fts-overfetch",
            "2",
            "--fts-min-rows",
            "25",
            "--fts-max-rounds",
            "3",
            "--fts-tokenizer",
            "unicode61",
            "--backend",
            "coreml",
            "--throughput-pilot",
        ]
        .map(str::to_string),
    )
    .unwrap();
    assert_eq!(args.only.as_deref(), Some("minilm-l6"));
    assert_eq!(
        select_document_formats(args.formats.as_deref()).unwrap(),
        vec![DocumentFormat::Fielded, DocumentFormat::Compact]
    );
    assert_eq!(args.settings.fts_column_weights, [8.0, 2.0, 2.0, 1.0]);
    assert_eq!(args.settings.fts_candidate_depth, 40);
    assert_eq!(args.settings.vector_candidate_depth, 30);
    assert_eq!(args.settings.rrf_k, 30.0);
    assert_eq!(args.settings.fts_rrf_weight, 1.5);
    assert_eq!(args.settings.vector_rrf_weight, 0.5);
    assert_eq!(args.settings.fts_term_operator, db::FtsTermOperator::All);
    assert_eq!(args.settings.fts_overfetch_multiplier, 2);
    assert_eq!(args.settings.fts_min_sql_rows, 25);
    assert_eq!(args.settings.fts_max_rounds, 3);
    assert_eq!(args.fts_tokenizer, FtsTokenizer::Unicode61);
    assert_eq!(args.backend, Backend::CoreMl);
    assert!(args.throughput_pilot);
}

/// Rejects unsupported document layouts and unsafe ranking values.
#[test]
fn bakeoff_rejects_unknown_or_repeated_formats_and_invalid_settings() {
    assert!(select_document_formats(Some("unknown")).is_err());
    assert!(select_document_formats(Some("production,production")).is_err());
    assert!(parse_args_from(["--fts-overfetch", "0"].map(str::to_string)).is_err());
    assert!(
        parse_args_from(["--fts-rrf-weight", "0", "--vector-rrf-weight", "0"].map(str::to_string))
            .is_err()
    );
    assert!(parse_args_from(["--fts-tokenizer", "porter2"].map(str::to_string)).is_err());
}

/// Confirms alternate tokenizers rebuild an isolated copy of the FTS index.
#[test]
fn alternate_fts_tokenizer_uses_a_temporary_index_copy() {
    let directory = tempfile::tempdir().unwrap();
    let source = db::open(&directory.path().join("source.db")).unwrap();
    source
        .execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES ('Stem Card', 'stem-id', '', 0, 'Artifact', '[]', '[]',
                '[]', 'destroys a creature', 'common', '{}', 'tst', '1', 'stem-print',
                '2020-01-01')",
            [],
        )
        .unwrap();

    let alternate = build_fts_index_variant(&source, FtsTokenizer::Unicode61).unwrap();
    assert!(alternate.snapshot_us > 0);
    assert!(alternate.rebuild_us > 0);
    assert_eq!(db::fts_search(&source, "\"destroy\"", 10).unwrap().len(), 1);
    assert!(
        db::fts_search(&alternate.conn, "\"destroy\"", 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        source
            .query_row("SELECT COUNT(*) FROM cards", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn production_bge_parity_gate_rejects_ordered_rank_changes() {
    let expected = vec!["A".to_string(), "B".to_string()];
    assert!(validate_bge_parity("bge-small-full", 1, "bolt", &expected, &expected).is_ok());
    let changed = vec!["B".to_string(), "A".to_string()];
    assert!(validate_bge_parity("bge-small-full", 1, "bolt", &changed, &expected).is_err());
    assert!(validate_bge_parity("arctic-s", 1, "bolt", &changed, &expected).is_ok());
}

#[test]
fn flat_matrix_requires_rows_to_match_names() {
    assert!(FlatMatrix::from_raw(vec!["a".into()], 2, vec![1.0, 0.0, 0.0, 1.0]).is_err());
    let matrix = FlatMatrix::from_raw(vec!["a".into()], 2, vec![1.0, 0.0]).unwrap();
    assert_eq!(matrix.row_count(), 1);
    assert_eq!(matrix.card_name(0), Some("a"));
    assert_eq!(matrix.vector(1), None);
}

#[test]
fn saved_reports_require_coreml() {
    assert!(validate_save_backend(None, Backend::Cpu).is_ok());
    assert!(validate_save_backend(Some(Path::new("report.json")), Backend::CoreMl).is_ok());
    assert!(validate_save_backend(Some(Path::new("report.json")), Backend::Cpu).is_err());
}
