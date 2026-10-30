//! Guard source bytes and selection integrity without claiming runtime support.

use super::commander_test_support::{named_row, rows};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Locate the independent source capture without opening the user's store.
fn capture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/deck/simulator/tests/commander_card_fixtures/sources/2026-10-30")
}

/// Read one captured source without fetching or rewriting it.
fn source_json(path: &str) -> Value {
    serde_json::from_slice(&std::fs::read(capture_root().join(path)).expect("source file exists"))
        .expect("source file contains JSON")
}

/// Compare hashes against original response bytes, not normalized JSON.
fn assert_file_hash(path: &str, expected: &Value) {
    let bytes = std::fs::read(capture_root().join(path)).expect("pinned file exists");
    assert_eq!(
        expected,
        &json!(format!("{:x}", Sha256::digest(bytes))),
        "{path}"
    );
}

/// Bind every original response and request to one immutable capture manifest.
#[test]
fn commander_capture_preserves_original_bytes_and_all_files() {
    assert_file_hash(
        "capture.json",
        &json!("295d2f9156990ac6d8015f2193d7a4c67db6a4ba44c310d1849ced6fecba0dc7"),
    );
    let capture = source_json("capture.json");
    assert_eq!(capture["status"], "source_capture_only_not_runtime_support");
    assert_eq!(capture["reporting_window"], "Past 2 Years");
    assert!(capture["window_start"].is_null());
    assert!(capture["window_end"].is_null());
    let mut declared = BTreeSet::from([String::from("capture.json")]);
    for entry in capture["files"]
        .as_array()
        .expect("derived files are listed")
        .iter()
        .chain(capture["sources"].as_array().expect("sources are listed"))
    {
        let path = entry["path"].as_str().expect("source path is text");
        assert!(declared.insert(path.to_string()), "one hash per path");
        assert_file_hash(path, &entry["raw_bytes_sha256"]);
        if let Some(request) = entry["request_path"].as_str() {
            assert!(declared.insert(request.to_string()));
            assert_file_hash(request, &entry["request_sha256"]);
        }
        if let Some(captured_at) = entry["captured_at"].as_str() {
            assert!(captured_at >= capture["started_at"].as_str().expect("start is text"));
            assert!(captured_at <= capture["finished_at"].as_str().expect("end is text"));
        }
    }
    let actual: BTreeSet<String> = std::fs::read_dir(capture_root())
        .expect("capture directory exists")
        .map(|entry| {
            entry
                .expect("capture entry can be read")
                .file_name()
                .into_string()
                .expect("capture filenames are UTF-8")
        })
        .collect();
    assert_eq!(actual, declared);
}

/// Retain all observations in EDHREC's page order rather than sorting by another rank.
#[test]
fn commander_popularity_preserves_ten_consecutive_pages() {
    let capture = source_json("capture.json");
    let sources: Vec<_> = capture["sources"]
        .as_array()
        .expect("sources are listed")
        .iter()
        .filter(|source| source["source"] == "EDHREC")
        .collect();
    assert_eq!(sources.len(), 10);
    let mut observations = Vec::new();
    for (page_index, source) in sources.iter().enumerate() {
        let suffix = if page_index == 0 {
            String::from("top/year.json")
        } else {
            format!("top/year-past2years-{page_index}.json")
        };
        assert_eq!(source["page_index"], page_index);
        assert_eq!(
            source["url"],
            format!("https://json.edhrec.com/pages/{suffix}")
        );
        let page = source_json(source["path"].as_str().expect("page path is text"));
        let list = if page_index == 0 {
            assert_eq!(page["header"], "Top Cards (Past 2 Years)");
            let list = &page["container"]["json_dict"]["cardlists"][0];
            assert_eq!(list["header"], "Past 2 Years");
            assert_eq!(list["tag"], "past2years");
            list
        } else {
            &page
        };
        assert_eq!(
            list["more"],
            format!("top/year-past2years-{}.json", page_index + 1)
        );
        let cards = list["cardviews"].as_array().expect("card views are listed");
        assert_eq!(cards.len(), 100);
        for card in cards {
            let mut observation = card.clone();
            observation["source_position"] = json!(observations.len() + 1);
            observation["page_index"] = json!(page_index);
            observations.push(observation);
        }
    }
    assert_eq!(json!(observations), source_json("popularity.json"));
    assert_eq!(observations.len(), 1_000);
    assert_eq!(observations[0]["name"], "Sol Ring");
    assert_eq!(observations[999]["name"], "Thalia and The Gitrog Monster");
    for field in ["name", "id"] {
        let unique: BTreeSet<_> = observations
            .iter()
            .map(|card| card[field].as_str().expect("source field is text"))
            .collect();
        assert_eq!(unique.len(), 1_000, "EDHREC {field} is unique");
    }
}

/// Match each collection request to one raw returned identity before normalization.
fn collection_data(source: &Value) -> (Vec<Value>, Vec<Value>) {
    assert_eq!(source["method"], "POST");
    assert_eq!(source["url"], "https://api.scryfall.com/cards/collection");
    let request = source_json(
        source["request_path"]
            .as_str()
            .expect("request path is text"),
    );
    let response = source_json(source["path"].as_str().expect("response path is text"));
    let identifiers = request["identifiers"]
        .as_array()
        .expect("identifiers are listed");
    assert!((1..=75).contains(&identifiers.len()));
    assert_eq!(response["object"], "list");
    assert_eq!(response["not_found"], json!([]));
    let data = response["data"]
        .as_array()
        .expect("returned cards are listed");
    assert_eq!(data.len(), identifiers.len());
    for identifier in identifiers {
        let matching: Vec<_> = data
            .iter()
            .filter(|card| {
                if let Some(id) = identifier["id"].as_str() {
                    card["id"] == id
                } else {
                    card["name"] == identifier["name"]
                        || card["card_faces"][0]["name"] == identifier["name"]
                }
            })
            .collect();
        assert_eq!(matching.len(), 1, "{identifier}");
    }
    (identifiers.clone(), data.clone())
}

/// Keep every collection lookup within Scryfall's batch limit and match exact identities.
#[test]
fn commander_scryfall_requests_match_every_returned_card() {
    let capture = source_json("capture.json");
    let selection = source_json("selection.json");
    let selected = selection.as_array().expect("selection is listed");
    let sources: Vec<_> = capture["sources"]
        .as_array()
        .expect("sources are listed")
        .iter()
        .filter(|source| source["source"] == "Scryfall")
        .collect();
    assert_eq!(sources.len(), 16);
    let mut requested = Vec::new();
    let mut raw_cards = Vec::new();
    for source in sources {
        let (identifiers, data) = collection_data(source);
        requested.extend(identifiers);
        raw_cards.extend(data);
    }
    assert_eq!(
        requested,
        selected
            .iter()
            .map(|card| card["identifier"].clone())
            .collect::<Vec<_>>()
    );
    let compact = source_json("cards.json");
    let compact = compact.as_array().expect("compact metadata is listed");
    assert_eq!(raw_cards.len(), 1_185);
    assert_eq!(compact.len(), raw_cards.len());
    for (raw, normalized) in raw_cards.iter().zip(compact) {
        for (field, value) in normalized.as_object().expect("metadata is an object") {
            assert_eq!(value, &raw[field], "{}: {field}", raw["name"]);
        }
    }
    for field in ["id", "oracle_id", "name"] {
        let unique: BTreeSet<_> = compact
            .iter()
            .map(|card| card[field].as_str().expect("source field is text"))
            .collect();
        assert_eq!(unique.len(), 1_185, "Scryfall {field} is unique");
    }
    for selected in selected {
        let card = compact
            .iter()
            .find(|card| {
                if selected["edhrec_id"].is_null() {
                    card["name"] == selected["selection_name"]
                } else {
                    card["id"] == selected["edhrec_id"]
                }
            })
            .expect("each selected source has one card");
        assert!(
            card["name"] == selected["selection_name"]
                || card["card_faces"][0]["name"] == selected["selection_name"]
        );
        let row = named_row(card["name"].as_str().expect("card name is text"));
        assert_eq!(row.oracle_id, card["oracle_id"]);
        assert_eq!(row.scryfall_id, card["id"]);
    }
}

/// Require complete explicit land groups while leaving rule interpretation to Oracle data.
#[test]
fn commander_land_selection_is_complete_and_separate_from_popularity() {
    let groups = source_json("land_groups.json");
    let input = std::fs::read(capture_root().join("../../land_cycles.json"))
        .expect("land selection input exists");
    assert_eq!(
        serde_json::from_slice::<Value>(&input).expect("land selection parses"),
        groups
    );
    let expected = BTreeMap::from([
        ("shock", 10),
        ("verge", 10),
        ("pain", 10),
        ("slow", 10),
        ("surveil", 10),
        ("check", 10),
        ("fetch", 10),
        ("bond", 10),
        ("triome", 10),
        ("fast", 10),
        ("filter", 10),
        ("bounce", 10),
        ("pathway", 10),
        ("original_dual", 10),
        ("neo_channel", 5),
        ("original_kamigawa_legendary", 5),
        ("requested_utility", 5),
        ("basic", 11),
        ("other_fetch", 5),
        ("other_utility", 14),
    ]);
    let mut actual = BTreeMap::new();
    let mut land_names = BTreeSet::new();
    for group in groups["groups"].as_array().expect("land groups are listed") {
        let names = group["names"].as_array().expect("land names are listed");
        let id = group["id"].as_str().expect("group ID is text");
        assert!(actual.insert(id, names.len()).is_none());
        assert_eq!(group["expected_count"], names.len());
        for name in names {
            let name = name.as_str().expect("land name is text");
            assert!(land_names.insert(name.to_string()), "unique land selection");
            let row = named_row(name);
            assert!(row.type_line.contains("Land"), "{name}");
        }
    }
    assert_eq!(actual, expected);
    assert_eq!(land_names.len(), 185);
    let popular = source_json("popularity.json");
    let mut expected_names = land_names;
    expected_names.extend(
        popular
            .as_array()
            .expect("popular cards are listed")
            .iter()
            .map(|card| named_row(card["name"].as_str().expect("popular name is text")).name),
    );
    let supplemental = source_json("../2026-10-30-behold/cards.json");
    expected_names.extend(
        supplemental
            .as_array()
            .expect("supplemental sources are listed")
            .iter()
            .map(|card| {
                card["name"]
                    .as_str()
                    .expect("source name is text")
                    .to_string()
            }),
    );
    let actual_names: BTreeSet<_> = rows().into_iter().map(|row| row.name).collect();
    assert_eq!(actual_names, expected_names);
    assert_eq!(actual_names.len(), 1_208);
}

/// Keep front-face aliases bound to the same full captured Oracle identity.
#[test]
fn commander_named_sources_use_production_face_flattening() {
    let cards = source_json("cards.json");
    for metadata in cards.as_array().expect("metadata is listed") {
        let name = metadata["name"].as_str().expect("source name is text");
        let row = named_row(name);
        let expected = super::card_test_support::metadata_row(metadata);
        assert_eq!(row.mana_cost, expected.mana_cost);
        assert_eq!(row.oracle_text, expected.oracle_text);
        assert_eq!(row.type_line, expected.type_line);
        assert_eq!(row.colors, expected.colors);
        assert_eq!(row.edhrec_rank, metadata["edhrec_rank"].as_i64());
        assert_eq!(row.reserved, metadata["reserved"].as_bool());
        assert_eq!(row.game_changer, metadata["game_changer"].as_bool());
        if let Some((front, _)) = name.split_once(" // ") {
            assert_eq!(named_row(front).oracle_id, row.oracle_id);
        }
    }
}

/// Preserve the complete small Behold query and the explicit Elven Passage selection.
#[test]
fn commander_behold_sources_preserve_query_and_collection_identity() {
    let prefix = "../2026-10-30-behold/";
    assert_file_hash(
        &format!("{prefix}capture.json"),
        &json!("698a40e1d47680b018c1e8e828ccae2d890da04f67e3fad4e82fcb7a2359d10b"),
    );
    let capture = source_json(&format!("{prefix}capture.json"));
    assert_eq!(capture["query"], "keyword:behold legal:commander");
    assert_eq!(capture["card_count"], 23);
    assert_eq!(capture["status"], "source_capture_only_not_runtime_support");
    for source in capture["sources"]
        .as_array()
        .expect("sources are listed")
        .iter()
        .chain(capture["files"].as_array().expect("files are listed"))
    {
        let path = source["path"].as_str().expect("source path is text");
        assert_file_hash(&format!("{prefix}{path}"), &source["raw_bytes_sha256"]);
        if let Some(request) = source["request_path"].as_str() {
            assert_file_hash(&format!("{prefix}{request}"), &source["request_sha256"]);
        }
    }
    assert_eq!(
        capture["sources"][0]["url"],
        "https://api.scryfall.com/cards/search?q=keyword%3Abehold%20legal%3Acommander&unique=cards&order=name"
    );
    let query = source_json(&format!("{prefix}behold_search_00.json"));
    assert_eq!(query["total_cards"], 23);
    assert_eq!(query["has_more"], false);
    let selection = source_json(&format!("{prefix}selection.json"));
    assert_eq!(
        selection["groups"][0],
        json!({"id":"supplemental_utility","expected_count":1,"names":["Elven Passage"]})
    );
    let names: Vec<_> = query["data"]
        .as_array()
        .expect("query results are listed")
        .iter()
        .map(|card| card["name"].clone())
        .collect();
    assert_eq!(selection["groups"][1]["id"], "behold");
    assert_eq!(selection["groups"][1]["names"], json!(names));
    let collection = collection_data(&json!({
        "method":"POST","url":"https://api.scryfall.com/cards/collection",
        "request_path":format!("{prefix}scryfall_request_00.json"),
        "path":format!("{prefix}scryfall_response_00.json")
    }))
    .1;
    assert_eq!(collection.len(), 23);
    for source in query["data"].as_array().expect("query results are listed") {
        let name = source["name"].as_str().expect("source name is text");
        let card = collection
            .iter()
            .find(|card| card["id"] == source["id"])
            .expect("every query result has a collection response");
        assert_eq!(card["oracle_id"], source["oracle_id"]);
        let row = named_row(name);
        assert_eq!(row.oracle_id, source["oracle_id"]);
        assert_eq!(
            row.oracle_text,
            super::card_test_support::metadata_row(card).oracle_text
        );
    }
    assert_eq!(
        named_row("Elven Passage").oracle_id,
        "97a2cd39-6b54-496b-b3ac-dab9dfed7edc"
    );
    let identities: BTreeSet<_> = rows().into_iter().map(|row| row.oracle_id).collect();
    assert_eq!(identities.len(), 1_208);
}
