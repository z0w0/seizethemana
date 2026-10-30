use super::*;

/// Provide a current bake-off report with an explicit document format.
fn bakeoff_report() -> serde_json::Value {
    serde_json::json!({
        "label_version": common::LABEL_VERSION,
        "corpus": {"fingerprint": "corpus"},
        "candidates": [{
            "key": "current-model",
            "document_format": "production",
            "queries": [{
                "query_index": 1,
                "cli_baseline": ["Test Card"],
                "unjudged_pool": ["Test Card"],
                "fts": ["Test Card"],
                "vector": ["Test Card"],
                "hybrid": ["Test Card"]
            }]
        }]
    })
}

/// Current reports retain the candidate's declared document layout.
#[test]
fn bakeoff_report_parses_current_fields() {
    let report: BakeoffReport =
        serde_json::from_value(bakeoff_report()).expect("parse current bake-off report");
    assert_eq!(report.label_version, common::LABEL_VERSION);
    assert_eq!(report.corpus.fingerprint, "corpus");
    assert_eq!(report.candidates.len(), 1);
    let candidate = &report.candidates[0];
    assert_eq!(candidate.key, "current-model");
    assert_eq!(candidate.document_format, "production");
    assert_eq!(candidate.queries.len(), 1);
    assert_eq!(candidate.queries[0].query_index, 1);
    assert_eq!(candidate.queries[0].cli_baseline, ["Test Card"]);
}

/// Saved reports must state the document layout used by each candidate.
#[test]
fn bakeoff_report_requires_document_format() {
    let mut report = bakeoff_report();
    report["candidates"][0]
        .as_object_mut()
        .expect("candidate object")
        .remove("document_format");
    let error = serde_json::from_value::<BakeoffReport>(report)
        .expect_err("missing document format must fail");
    assert!(
        error
            .to_string()
            .contains("missing field `document_format`")
    );
}

fn batch() -> ReviewBatch {
    let item = ReviewItem {
        item_id: item_id(1, "Test Card"),
        query_index: 1,
        query: "draw cards".to_string(),
        intent: "card draw".to_string(),
        group: "role_intent".to_string(),
        filters: None,
        relevant_examples: Vec::new(),
        not_relevant_examples: Vec::new(),
        card: CardEvidence {
            name: "Test Card".to_string(),
            type_line: "Sorcery".to_string(),
            keywords: Vec::new(),
            tags: Vec::new(),
            oracle_text: "Draw two cards.".to_string(),
        },
        retrieval: vec![RetrievalEvidence {
            source: "hybrid".to_string(),
            rank: 1,
        }],
    };
    ReviewBatch {
        schema_version: 1,
        batch_index: 0,
        batch_count: 1,
        batch_id: batch_id("v1", "corpus", std::slice::from_ref(&item), 0, 1),
        label_version: "v1".to_string(),
        corpus_fingerprint: "corpus".to_string(),
        source_reports: vec!["report.json".to_string()],
        agent_instructions: String::new(),
        response_schema: String::new(),
        items: vec![item],
    }
}

fn response(batch: &ReviewBatch) -> AgentResponse {
    AgentResponse {
        schema_version: 1,
        batch_id: batch.batch_id.clone(),
        label_version: batch.label_version.clone(),
        corpus_fingerprint: batch.corpus_fingerprint.clone(),
        decisions: vec![Decision {
            item_id: batch.items[0].item_id.clone(),
            decision: "relevant".to_string(),
            reason: "The rules text says Draw two cards.".to_string(),
        }],
    }
}

#[test]
fn response_requires_every_item_and_matching_identity() {
    let batch = batch();
    let mut response = response(&batch);
    assert!(validate_response(&batch, &response).is_ok());
    response.batch_id.push_str("-stale");
    assert!(validate_response(&batch, &response).is_err());
}

#[test]
fn response_rejects_missing_duplicate_and_invalid_decisions() {
    let batch = batch();
    let mut missing = response(&batch);
    missing.decisions.clear();
    assert!(validate_response(&batch, &missing).is_err());
    let mut duplicate = response(&batch);
    duplicate.decisions.push(duplicate.decisions[0].clone());
    assert!(validate_response(&batch, &duplicate).is_err());
    let mut invalid = response(&batch);
    invalid.decisions[0].decision = "maybe".to_string();
    assert!(validate_response(&batch, &invalid).is_err());
}

#[test]
fn response_rejects_stale_label_and_corpus_versions() {
    let batch = batch();
    let mut stale_labels = response(&batch);
    stale_labels.label_version.push_str("-old");
    assert!(validate_response(&batch, &stale_labels).is_err());
    let mut stale_corpus = response(&batch);
    stale_corpus.corpus_fingerprint.push_str("-old");
    assert!(validate_response(&batch, &stale_corpus).is_err());
}

#[test]
fn response_rejects_changed_review_evidence() {
    let mut batch = batch();
    let response = response(&batch);
    batch.items[0].card.oracle_text = "Changed after export.".to_string();
    assert!(validate_response(&batch, &response).is_err());
}

#[test]
fn label_version_increments_numeric_suffix() {
    assert_eq!(
        increment_label_version("2026-09-25.5").unwrap(),
        "2026-09-25.6"
    );
    assert!(increment_label_version("invalid").is_err());
}

#[test]
fn item_identity_separates_query_index_and_exact_name() {
    assert_ne!(item_id(1, "Card"), item_id(2, "Card"));
    assert_ne!(item_id(1, "Card"), item_id(1, "Other"));
}

#[test]
fn batch_identity_covers_batch_boundaries() {
    let batch = batch();
    assert_ne!(
        batch.batch_id,
        batch_id(
            &batch.label_version,
            &batch.corpus_fingerprint,
            &batch.items,
            1,
            2,
        )
    );
}

/// Provide reviewed and unreviewed cards without using the user's store.
fn review_cards() -> Vec<db::CardRow> {
    let reviewed = db::CardRow {
        name: "Test Card".into(),
        oracle_id: "test-oracle".into(),
        mana_cost: "{U}".into(),
        cmc: 1.0,
        type_line: "Sorcery".into(),
        colors: r#"["U"]"#.into(),
        color_identity: r#"["U"]"#.into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: "Draw two cards.".into(),
        rarity: "common".into(),
        edhrec_rank: Some(100),
        penny_rank: Some(200),
        reserved: Some(false),
        legalities: r#"{"modern":"legal"}"#.into(),
        set_code: "tst".into(),
        collector_number: "1".into(),
        scryfall_id: "test-print".into(),
        released_at: "2020-01-01".into(),
        game_changer: Some(false),
    };
    let unreviewed = db::CardRow {
        name: "Unreviewed Card".into(),
        oracle_id: "unreviewed-oracle".into(),
        ..reviewed.clone()
    };
    vec![reviewed, unreviewed]
}

/// Bind the existing response fixture to complete live corpus evidence.
fn live_batch(cards: &[db::CardRow]) -> ReviewBatch {
    let mut batch = batch();
    batch.label_version = common::LABEL_VERSION.into();
    batch.corpus_fingerprint =
        common::fingerprint_corpus(cards, &tags::TagIndex::default()).unwrap();
    batch.batch_id = batch_id(
        &batch.label_version,
        &batch.corpus_fingerprint,
        &batch.items,
        batch.batch_index,
        batch.batch_count,
    );
    batch
}

/// Supply a saved ranking source with the live batch's corpus identity.
fn baseline_for(batch: &ReviewBatch) -> BaselineSource {
    let item = &batch.items[0];
    BaselineSource {
        benchmark: BaselineBenchmark {
            label_version: batch.label_version.clone(),
            corpus_fingerprint: batch.corpus_fingerprint.clone(),
        },
        queries: vec![BaselineQuery {
            query_index: item.query_index,
            query: item.query.clone(),
            note: item.intent.clone(),
            group: item.group.clone(),
            filters: item.filters.clone(),
            relevant: Vec::new(),
            not_relevant: Vec::new(),
            unjudged_pool: vec![item.card.name.clone()],
            rankings: HashMap::from([("production".into(), vec![item.card.name.clone()])]),
        }],
    }
}

/// Create a temporary golden file whose next label comes from the batch.
fn write_review_goldens(path: &Path, batch: &ReviewBatch) -> Vec<u8> {
    let item = &batch.items[0];
    let queries = vec![common::GoldenQuery {
        query: item.query.clone(),
        relevant: Vec::new(),
        not_relevant: Vec::new(),
        filters: item.filters.clone(),
        note: item.intent.clone(),
        group: Some(item.group.clone()),
    }];
    let bytes = serde_json::to_vec_pretty(&queries).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

/// Export must reject changes to reviewed facts and other cards' filter inputs.
#[test]
fn export_rejects_stale_oracle_and_full_corpus_filter_evidence() {
    let cards = review_cards();
    let batch = live_batch(&cards);
    let tags = tags::TagIndex::default();
    assert!(
        build_batch(
            baseline_for(&batch),
            &HashMap::new(),
            Vec::new(),
            &cards,
            &tags
        )
        .is_ok()
    );
    let changes: &[fn(&mut [db::CardRow])] = &[
        |cards| cards[0].oracle_text = "Deal two damage.".into(),
        |cards| cards[0].rarity = "rare".into(),
        |cards| cards[1].legalities = r#"{"modern":"banned"}"#.into(),
        |cards| cards[1].penny_rank = Some(201),
    ];
    for change in changes {
        let mut changed = cards.clone();
        change(&mut changed);
        let error = build_batch(
            baseline_for(&batch),
            &HashMap::new(),
            Vec::new(),
            &changed,
            &tags,
        )
        .unwrap_err();
        assert!(error.to_string().contains("saved corpus"));
    }
}

/// Stale live facts must leave both labels and prior approvals byte-identical.
#[test]
fn apply_rejects_stale_corpus_before_writing_labels_or_audit() {
    let cards = review_cards();
    let batch = live_batch(&cards);
    let response = response(&batch);
    let directory = tempfile::tempdir().unwrap();
    let golden_path = directory.path().join("goldens.json");
    let audit_path = directory.path().join("audit.jsonl");
    let goldens = write_review_goldens(&golden_path, &batch);
    let audit = b"{\"prior_approval\":true}\n";
    std::fs::write(&audit_path, audit).unwrap();
    let changes: &[fn(&mut [db::CardRow])] = &[
        |cards| cards[0].oracle_text = "Deal two damage.".into(),
        |cards| cards[0].rarity = "rare".into(),
        |cards| cards[1].legalities = r#"{"modern":"banned"}"#.into(),
        |cards| cards[1].penny_rank = Some(201),
    ];
    for change in changes {
        let mut changed = cards.clone();
        change(&mut changed);
        let error = apply_response_to_files(
            &batch,
            &response,
            "Reviewed",
            &golden_path,
            &audit_path,
            &changed,
            &tags::TagIndex::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("saved corpus"));
        assert_eq!(std::fs::read(&golden_path).unwrap(), goldens);
        assert_eq!(std::fs::read(&audit_path).unwrap(), audit);
    }
}

/// A non-NotFound audit read error must occur before labels change.
#[test]
fn audit_read_failure_preserves_labels_and_the_existing_audit() {
    let cards = review_cards();
    let batch = live_batch(&cards);
    let directory = tempfile::tempdir().unwrap();
    let golden_path = directory.path().join("goldens.json");
    let audit_path = directory.path().join("audit.jsonl");
    let goldens = write_review_goldens(&golden_path, &batch);
    std::fs::create_dir(&audit_path).unwrap();
    let marker = audit_path.join("marker");
    std::fs::write(&marker, b"existing audit data").unwrap();
    let error = apply_response_to_files(
        &batch,
        &response(&batch),
        "Reviewed",
        &golden_path,
        &audit_path,
        &cards,
        &tags::TagIndex::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("reading approval audit"));
    assert_eq!(std::fs::read(&golden_path).unwrap(), goldens);
    assert_eq!(std::fs::read(marker).unwrap(), b"existing audit data");
    assert!(!golden_path.with_extension("tmp").exists());
    assert!(!audit_path.with_extension("tmp").exists());
}

/// Missing audits may be created, while existing approval bytes must survive.
#[test]
fn apply_creates_or_appends_the_audit_after_validating_live_evidence() {
    let cards = review_cards();
    let batch = live_batch(&cards);
    for prior in [b"".as_slice(), b"{\"prior_approval\":true}\n".as_slice()] {
        let directory = tempfile::tempdir().unwrap();
        let golden_path = directory.path().join("goldens.json");
        let audit_path = directory.path().join("audit.jsonl");
        write_review_goldens(&golden_path, &batch);
        if !prior.is_empty() {
            std::fs::write(&audit_path, prior).unwrap();
        }
        assert_eq!(
            apply_response_to_files(
                &batch,
                &response(&batch),
                "Reviewed",
                &golden_path,
                &audit_path,
                &cards,
                &tags::TagIndex::default(),
            )
            .unwrap(),
            1
        );
        let queries = common::load_queries(&golden_path).unwrap();
        assert_eq!(queries[0].relevant, ["Test Card"]);
        let audit = std::fs::read(&audit_path).unwrap();
        assert!(audit.starts_with(prior));
        let entry: serde_json::Value = serde_json::from_slice(&audit[prior.len()..]).unwrap();
        assert_eq!(entry["applied_count"], 1);
        assert_eq!(entry["batch_id"], batch.batch_id);
        assert!(!golden_path.with_extension("tmp").exists());
        assert!(!audit_path.with_extension("tmp").exists());
    }
}
