use super::*;

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
