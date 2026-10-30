//! Source reviews cover exact identities, faces, printed fields, and semantic groups.
//!
//! These records describe rules obligations. They do not certify execution.
//! Runtime claims use separate callable action and report assertions.

use super::card_test_support::{corpus, json_hash, oracle_hash, read_json};
use serde_json::Value;
use std::collections::HashSet;

/// The five independently reviewed slices of the preserved selected-card corpus.
const REVIEW_BATCHES: [(&str, usize, usize); 5] = [
    ("semantic_reviews/edhrec_001_050.json", 1, 50),
    ("semantic_reviews/edhrec_051_100.json", 51, 100),
    ("semantic_reviews/edhrec_101_150.json", 101, 150),
    ("semantic_reviews/edhrec_151_200.json", 151, 200),
    ("semantic_reviews/edhrec_201_250.json", 201, 250),
];

/// Load each reviewed batch once without generating substitute review records.
pub(super) fn reviewed_cards() -> Vec<Value> {
    let mut cards = Vec::new();
    for (path, start, end) in REVIEW_BATCHES {
        let batch = read_json(path);
        assert_eq!(batch["selection"], "edhrec_top250", "{path}");
        assert_eq!(batch["range"], serde_json::json!([start, end]), "{path}");
        assert_eq!(
            batch["evidence_kind"], "source_semantic_review_not_execution",
            "{path}"
        );
        let entries = batch["cards"].as_array().expect("reviewed batch has cards");
        assert_eq!(entries.len(), end - start + 1, "{path}");
        cards.extend(entries.iter().cloned());
    }
    cards
}

/// Reject placeholders without rejecting a specific statement that no replacement is printed.
fn assert_specific_note(note: &Value, context: &str) {
    let note = note
        .as_str()
        .unwrap_or_else(|| panic!("{context}: review note is text"));
    assert!(
        note.trim().len() >= 12,
        "{context}: missing specific rules note"
    );
    let lower = note.to_ascii_lowercase();
    for placeholder in [
        "unreviewed:",
        "determine this obligation",
        "source extraction alone",
        "same as above",
        "todo",
        "tbd",
        "not_verified",
    ] {
        assert!(
            !lower.contains(placeholder),
            "{context}: placeholder {placeholder}"
        );
    }
}

/// Require every printed face and all five field groups to have a real rules note.
fn assert_printed_review(review: &Value, metadata: &Value) -> usize {
    let faces: Vec<&Value> = metadata["card_faces"]
        .as_array()
        .map_or_else(|| vec![metadata], |faces| faces.iter().collect());
    let reviewed = review["printed_fields"]
        .as_array()
        .expect("printed reviews are a list");
    assert_eq!(
        reviewed.len(),
        faces.len(),
        "{}: all printed faces",
        review["name"]
    );
    let mut names = HashSet::new();
    for face in faces {
        let entry = reviewed
            .iter()
            .find(|entry| entry["face"] == face["name"])
            .expect("every source face has a printed-field review");
        assert!(names.insert(entry["face"].as_str().expect("face name is text")));
        let fields = entry["fields"]
            .as_object()
            .expect("printed fields form an object");
        assert_eq!(fields.len(), 5);
        for field in [
            "mana_cost",
            "type_line",
            "colors",
            "power_toughness_loyalty",
            "keywords",
        ] {
            assert_eq!(entry["fields"][field]["disposition"], "source_reviewed");
            assert_specific_note(&entry["fields"][field]["obligation"], field);
        }
    }
    names.len()
}

/// Check exact source origins and coverage while allowing separate keyword groups on one line.
fn assert_face_groups(review: &Value, face: &Value) -> usize {
    let text = face["oracle_text"].as_str().unwrap_or_default();
    let abilities = review["abilities"]
        .as_array()
        .expect("semantic groups are a list");
    let groups: Vec<_> = abilities
        .iter()
        .filter(|ability| ability["face"] == face["name"])
        .collect();
    assert_eq!(
        groups.is_empty(),
        text.is_empty(),
        "{}: face group coverage",
        face["name"]
    );
    let mut covered = vec![false; text.len()];
    for group in &groups {
        let source = group["source_text"]
            .as_str()
            .expect("review retains exact source text");
        assert!(!source.is_empty());
        let occurrences: Vec<_> = text.match_indices(source).collect();
        assert!(
            !occurrences.is_empty(),
            "{}: invented or changed source text {source}",
            face["name"]
        );
        for (start, _) in occurrences {
            covered[start..start + source.len()].fill(true);
        }
        assert_eq!(group["disposition"], "source_reviewed_execution_unverified");
        for kind in ["cost", "effect", "target", "timing", "replacement"] {
            assert_specific_note(&group[kind], kind);
        }
        assert_specific_note(&group["remaining_gap"], "remaining_gap");
    }
    for (index, character) in text.char_indices() {
        if !character.is_whitespace() && character != ',' {
            assert!(
                covered[index],
                "{}: source byte {index} has no semantic review",
                face["name"]
            );
        }
    }
    groups.len()
}

/// Check all semantic groups against known face identities and exact pinned face text.
fn assert_ability_reviews(review: &Value, metadata: &Value) -> usize {
    let faces: Vec<&Value> = metadata["card_faces"]
        .as_array()
        .map_or_else(|| vec![metadata], |faces| faces.iter().collect());
    for group in review["abilities"]
        .as_array()
        .expect("semantic groups are a list")
    {
        assert!(
            faces.iter().any(|face| group["face"] == face["name"]),
            "{}: unknown reviewed face",
            review["name"]
        );
    }
    faces
        .iter()
        .map(|face| assert_face_groups(review, face))
        .sum()
}

/// Check semantic review identity, source revisions, and all face obligations as one gate.
fn validate_reviews(reviews: &[Value], selected: &[&Value]) -> (usize, usize) {
    assert_eq!(reviews.len(), 250);
    assert_eq!(selected.len(), 250);
    let mut identities = HashSet::new();
    let mut group_count = 0;
    let mut face_count = 0;
    for (review, source) in reviews.iter().zip(selected) {
        assert_eq!(review["name"], source["name"]);
        assert_eq!(review["oracle_id"], source["metadata"]["oracle_id"]);
        assert_eq!(review["metadata_sha256"], source["metadata_sha256"]);
        assert_eq!(review["metadata_sha256"], json_hash(&source["metadata"]));
        assert_eq!(
            source["oracle_text_sha256"],
            oracle_hash(&source["metadata"])
        );
        assert!(
            identities.insert(
                review["oracle_id"]
                    .as_str()
                    .expect("Oracle identity is text")
            )
        );
        face_count += assert_printed_review(review, &source["metadata"]);
        group_count += assert_ability_reviews(review, &source["metadata"]);
    }
    assert_eq!(identities.len(), 250);
    (face_count, group_count)
}

/// Exact selected identities and source revisions are required for valid semantic review counts.
#[test]
fn all_250_selected_cards_have_specific_source_semantic_reviews() {
    let reviews = reviewed_cards();
    let corpus = corpus();
    let selected: Vec<_> = corpus["cards"]
        .as_array()
        .expect("source corpus has cards")
        .iter()
        .filter(|card| {
            card["selection_tags"]
                .as_array()
                .expect("tags are a list")
                .iter()
                .any(|tag| tag == "edhrec_top250")
        })
        .collect();
    let (face_count, group_count) = validate_reviews(&reviews, &selected);
    println!(
        "reviewed source obligations: {} cards, {face_count} printed faces, {group_count} semantic groups, {} ability obligation notes, {} printed-field notes; zero execution certifications",
        reviews.len(),
        group_count * 5,
        face_count * 5
    );
}

/// Missing fields, substituted source identities, and blanket placeholders cannot pass the review gate.
#[test]
fn semantic_review_gate_rejects_missing_rules_and_duplicate_substitution() {
    let reviews = reviewed_cards();
    let source = corpus();
    let selected: Vec<_> = source["cards"]
        .as_array()
        .expect("source corpus has cards")
        .iter()
        .filter(|card| {
            card["selection_tags"]
                .as_array()
                .expect("tags are a list")
                .iter()
                .any(|tag| tag == "edhrec_top250")
        })
        .collect();
    validate_reviews(&reviews, &selected);
    let mut duplicate = reviews.clone();
    duplicate[249] = duplicate[248].clone();
    assert!(std::panic::catch_unwind(|| validate_reviews(&duplicate, &selected)).is_err());
    for (field, replacement) in [
        ("cost", Value::Null),
        (
            "effect",
            Value::String("Unreviewed: determine this obligation from source text.".into()),
        ),
        (
            "source_text",
            Value::String("Invented wording that is absent from the pinned card.".into()),
        ),
        ("face", Value::String("Invented face identity".into())),
    ] {
        let mut changed = reviews[0].clone();
        changed["abilities"][0][field] = replacement;
        assert!(
            std::panic::catch_unwind(|| assert_ability_reviews(&changed, &selected[0]["metadata"]))
                .is_err(),
            "{field}: corrupted semantic review passed"
        );
    }
}
