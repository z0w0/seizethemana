//! Offline integrity gates for raw sources, exact selection, and source revisions.

use super::card_test_support::*;
use serde_json::Value;
use std::collections::HashSet;

/// Formatting keeps the content contract but must fail the separate byte contract.
#[test]
fn formatting_cannot_hide_a_raw_byte_hash_change() {
    let path = fixture_root().join("sources/edhrec_1.json");
    let original = std::fs::read(path).expect("saved page exists");
    let mut reformatted = vec![b' '];
    reformatted.extend_from_slice(&original);
    assert_ne!(byte_hash(&original), byte_hash(&reformatted));
    let before: Value = serde_json::from_slice(&original).expect("saved JSON parses");
    let after: Value = serde_json::from_slice(&reformatted).expect("reformatted JSON parses");
    assert_eq!(json_hash(&before), json_hash(&after));
}

/// Validate every manifest entry, including the separate raw and canonical contracts.
#[test]
fn every_source_file_and_flattened_revision_matches_provenance() {
    assert_eq!(read_json("provenance.json"), build_provenance());
    let manifest = read_json("provenance.json");
    let files = manifest["files"]
        .as_array()
        .expect("file entries are a list");
    assert_eq!(files.len(), 22);
    assert!(files.iter().any(|file| {
        file["canonical_json_sha256"].is_string()
            && file["raw_bytes_sha256"] != file["canonical_json_sha256"]
    }));
    let sources = read_json("sources.json");
    for source in sources["sources"]
        .as_array()
        .expect("source declarations are a list")
    {
        let file = files
            .iter()
            .find(|file| file["path"] == source["path"])
            .expect("every source declaration has a checked file");
        assert_eq!(source["sha256"], file["historical_declared_sha256"]);
        assert_eq!(source["observed_at"], sources["observed_at"]);
        assert!(
            source["url"]
                .as_str()
                .expect("source URL is text")
                .contains("://")
        );
    }
}

/// Check metadata membership by Oracle ID and every retained source field.
#[test]
fn combined_sources_have_distinct_identities_and_valid_source_links() {
    let cards = source_cards();
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    for card in &cards {
        let metadata = &card["metadata"];
        assert!(names.insert(card["name"].as_str().expect("card has a name")));
        let id = metadata["oracle_id"]
            .as_str()
            .expect("card has an Oracle ID");
        assert!(ids.insert(id));
        assert_eq!(card["metadata_sha256"], json_hash(metadata));
        assert_eq!(card["oracle_text_sha256"], oracle_hash(metadata));
        let source = read_json(card["source_path"].as_str().expect("source path is text"));
        let original = if let Some(data) = source["data"].as_array() {
            data.iter()
                .find(|entry| entry["oracle_id"] == id)
                .expect("corpus Oracle identity belongs to its declared response")
        } else {
            &source
        };
        for (field, expected) in metadata.as_object().expect("metadata is an object") {
            assert_eq!(&original[field], expected, "{id}: {field}");
        }
    }
    assert_eq!(names.len(), 356);
    assert_eq!(ids.len(), 356);
    assert_eq!(cards[350]["name"], "Zulaport Cutthroat");
    assert_eq!(cards[351]["name"], "Trophy Mage");
    assert_eq!(cards[353]["name"], "Ox of Agonas");
    assert_eq!(cards[355]["name"], "Gravebreaker Lamia");
}

/// Read the saved page sequence, including its declared continuation links.
fn page_sequence() -> Vec<(usize, Value)> {
    let first = read_json("sources/edhrec_1.json");
    let list = &first["container"]["json_dict"]["cardlists"][0];
    assert_eq!(list["header"], "Past 2 Years");
    assert_eq!(list["tag"], "past2years");
    let mut pages = vec![list.clone()];
    pages.push(read_json("sources/edhrec_2.json"));
    pages.push(read_json("sources/edhrec_3.json"));
    for (index, page) in pages.iter().enumerate() {
        if index == 0 {
            assert!(page["is_paginated"].is_null());
        } else {
            assert_eq!(page["is_paginated"], true);
        }
        assert_eq!(page["total_pages"], 313);
        assert_eq!(
            page["more"],
            format!("top/year-past2years-{}.json", index + 1)
        );
        assert_eq!(
            page["cardviews"]
                .as_array()
                .expect("page cards are a list")
                .len(),
            100
        );
    }
    pages
        .into_iter()
        .enumerate()
        .flat_map(|(index, page)| {
            page["cardviews"]
                .as_array()
                .expect("page cards are a list")
                .clone()
                .into_iter()
                .map(move |card| (index + 1, card))
        })
        .collect()
}

/// Compare selection facts to the saved sequence before checking source-card identities.
fn selection_matches_pages(selected: &[Value], pages: &[(usize, Value)]) -> bool {
    selected.len() == 250
        && selected
            .iter()
            .zip(pages)
            .enumerate()
            .all(|(index, (card, (page, original)))| {
                ["name", "url", "num_decks", "potential_decks"]
                    .iter()
                    .all(|field| card[*field] == original[*field])
                    && card["source_position"] == index + 1
                    && card["page"] == *page
                    && card["explicit_rank"].is_null()
            })
}

/// Require exact first-250 equality; duplicate substitution cannot satisfy this gate.
#[test]
fn selected_identities_equal_the_first_250_saved_page_entries() {
    let corpus = corpus();
    let selected = corpus["popularity"]["cards"]
        .as_array()
        .expect("selection is a list");
    let source_cards = corpus["cards"].as_array().expect("corpus is a list");
    let pages = page_sequence();
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    assert_eq!(selected.len(), 250);
    assert!(selection_matches_pages(selected, &pages));
    let mut duplicate = selected.clone();
    duplicate[249] = duplicate[248].clone();
    assert!(!selection_matches_pages(&duplicate, &pages));
    let mut reordered = selected.clone();
    reordered.swap(248, 249);
    assert!(!selection_matches_pages(&reordered, &pages));
    for (index, (selection, (page, original))) in selected.iter().zip(&pages).enumerate() {
        for field in ["name", "url", "num_decks", "potential_decks"] {
            assert_eq!(selection[field], original[field]);
        }
        assert_eq!(selection["source_position"], index + 1);
        assert_eq!(selection["page"], *page);
        assert!(selection["explicit_rank"].is_null());
        let card = source_cards
            .iter()
            .find(|card| card["name"] == original["name"])
            .expect("every selected source card is in the corpus");
        assert!(names.insert(card["name"].as_str().expect("name is text")));
        assert!(ids.insert(card["metadata"]["oracle_id"].as_str().expect("ID is text")));
        assert!(
            card["selection_tags"]
                .as_array()
                .expect("tags are a list")
                .iter()
                .any(|tag| tag == "edhrec_top250")
        );
    }
    assert_eq!(
        source_cards
            .iter()
            .filter(|card| {
                card["selection_tags"]
                    .as_array()
                    .expect("tags are a list")
                    .iter()
                    .any(|tag| tag == "edhrec_top250")
            })
            .count(),
        250
    );
}
