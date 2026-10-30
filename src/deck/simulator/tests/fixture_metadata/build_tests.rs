//! Offline supplement checks with local source-read failures and changed source values.

use super::super::fixture_metadata::SUPPLEMENT;
use super::*;

/// Read repository inputs without permitting the builder to write them.
fn read_saved(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .map_err(|error| error.to_string())
}

/// Derive the expected names from the current legacy deck snapshots.
fn current_names() -> BTreeSet<String> {
    legacy_names(Path::new(env!("CARGO_MANIFEST_DIR")), &mut read_saved)
        .expect("legacy fixture inventory is valid")
}

/// Inject changed bytes without touching saved sources or capture manifests.
fn build_with_overrides(
    names: &BTreeSet<String>,
    overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<Value, String> {
    build(names, &mut |path| match overrides.get(path) {
        Some(bytes) => Ok(bytes.clone()),
        None => read_saved(path),
    })
}

/// Give a changed source a matching manifest hash so metadata checks can run.
fn source_overrides(path: &str, value: &Value) -> BTreeMap<String, Vec<u8>> {
    let bytes = serde_json::to_vec(value).expect("changed source serializes");
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let mut overrides = BTreeMap::new();
    if let Some(directory) = path.strip_suffix("response.json") {
        let manifest_path = format!("{directory}capture.json");
        let (_, mut manifest) = read_json(&manifest_path, &mut read_saved).expect("capture parses");
        manifest["response_sha256"] = json!(hash);
        overrides.insert(
            manifest_path,
            serde_json::to_vec(&manifest).expect("capture serializes"),
        );
    } else if let Some(relative) =
        path.strip_prefix("src/deck/simulator/tests/popular_card_fixtures/")
    {
        let (_, mut manifest) =
            read_json(POPULAR_MANIFEST, &mut read_saved).expect("manifest parses");
        let entry = manifest["files"]
            .as_array_mut()
            .expect("manifest files")
            .iter_mut()
            .find(|file| file["path"] == relative)
            .expect("declared historical source");
        entry["raw_bytes_sha256"] = json!(hash);
        overrides.insert(
            POPULAR_MANIFEST.to_string(),
            serde_json::to_vec(&manifest).expect("manifest serializes"),
        );
    }
    overrides.insert(path.to_string(), bytes);
    overrides
}

/// Rebuilding covers all declared sources and preserves every input byte.
#[test]
fn rebuild_matches_pinned_supplement_and_keeps_inputs_unchanged() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut inputs = BTreeMap::new();
    let mut read = |path: &str| {
        let bytes = read_saved(path)?;
        inputs.insert(path.to_string(), bytes.clone());
        Ok(bytes)
    };
    let names = legacy_names(root, &mut read).expect("legacy names");
    assert_eq!(names.len(), 66);
    let rebuilt = build(&names, &mut read).expect("offline supplement rebuild");
    let pinned: Value = serde_json::from_str(SUPPLEMENT).expect("pinned JSON");
    assert_eq!(rebuilt, pinned);
    let paths: Vec<_> = rebuilt["sources"]
        .as_array()
        .expect("declared sources")
        .iter()
        .map(|source| source["path"].as_str().expect("source path"))
        .collect();
    assert_eq!(paths, SOURCE_PATHS);
    for path in SOURCE_PATHS {
        assert!(inputs.contains_key(path), "source was not read: {path}");
    }
    for (path, bytes) in inputs {
        assert_eq!(
            read_saved(&path).expect("input still exists"),
            bytes,
            "{path}"
        );
    }
    assert_eq!(
        read_saved("src/deck/simulator/tests/fixture_metadata/supplement.json")
            .expect("pinned supplement exists"),
        SUPPLEMENT.as_bytes(),
    );
}

/// Every declared source, request, and manifest read must succeed.
#[test]
fn rebuild_rejects_missing_or_failed_source_reads() {
    let names = current_names();
    let mut paths: Vec<_> = SOURCE_PATHS.iter().map(|path| path.to_string()).collect();
    paths.push(POPULAR_MANIFEST.to_string());
    for path in SOURCE_PATHS {
        if let Some(directory) = path.strip_suffix("response.json") {
            paths.push(format!("{directory}capture.json"));
            paths.push(format!("{directory}request.json"));
        }
    }
    for failed_path in paths {
        let error = build(&names, &mut |path| {
            if path == failed_path {
                Err("injected read failure".into())
            } else {
                read_saved(path)
            }
        })
        .expect_err("a failed input read must stop the rebuild");
        assert!(error.contains(&failed_path), "{error}");
        assert!(error.contains("injected read failure"), "{error}");
    }
}

/// The name inventory comes from legacy rows and rejects malformed declarations.
#[test]
fn legacy_inventory_selects_commanders_and_vehicles_only() {
    let mut names = BTreeSet::new();
    let fixture = json!({
        "commanders": [{"name": "Commander"}],
        "cards": [
            {"name": "Vehicle", "type_line": "Artifact Vehicle"},
            {"name": "Creature", "type_line": "Creature"},
        ],
    });
    add_legacy_names(&fixture, "legacy.json", &mut names).expect("legacy rows");
    assert_eq!(names, ["Commander", "Vehicle"].map(str::to_string).into());
    let mut official = fixture.clone();
    official["source_kind"] = json!("official_precon");
    let mut skipped = BTreeSet::new();
    add_legacy_names(&official, "official.json", &mut skipped).expect("official rows are separate");
    assert!(skipped.is_empty());
    for invalid in [
        json!([]),
        json!({"source_kind": false}),
        json!({"commanders": [], "cards": null}),
        json!({"commanders": [{"name": ""}], "cards": []}),
        json!({"commanders": [], "cards": [{"name": "Bad type", "type_line": null}]}),
    ] {
        assert!(add_legacy_names(&invalid, "bad.json", &mut BTreeSet::new()).is_err());
    }
}

/// Failed legacy reads and missing target cards cannot produce a partial supplement.
#[test]
fn rebuild_rejects_failed_inventory_and_missing_metadata() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let error = legacy_names(root, &mut |_| Err("failed legacy read".into()))
        .expect_err("legacy reads are required");
    assert!(error.contains("failed legacy read"));
    assert!(build(&BTreeSet::new(), &mut read_saved).is_err());
    let mut names = current_names();
    names.insert("Uncaptured commander".into());
    let error = build(&names, &mut read_saved).expect_err("every legacy name needs a source");
    assert!(error.contains("missing pinned metadata"));
    assert!(error.contains("Uncaptured commander"));
    let path = SOURCE_PATHS[8];
    let (_, mut source) = read_json(path, &mut read_saved).expect("identity capture");
    source["data"]
        .as_array_mut()
        .expect("captured cards")
        .remove(0);
    let error = build_with_overrides(&current_names(), &source_overrides(path, &source))
        .expect_err("a missing captured card must fail");
    assert!(error.contains("Alesha, Who Smiles at Death"));
}

/// Changed Oracle IDs, colors, and printed stats are conflicts even within one source.
#[test]
fn rebuild_rejects_conflicting_metadata_and_reused_oracle_ids() {
    let names = current_names();
    let path = SOURCE_PATHS[3];
    let (_, original) = read_json(path, &mut read_saved).expect("historical source");
    for (field, value) in [
        ("oracle_id", json!("conflicting-id")),
        ("color_identity", json!(["G"])),
        ("power", json!("6")),
    ] {
        let mut source = original.clone();
        let mut duplicate = source["data"][4].clone();
        duplicate[field] = value;
        source["data"]
            .as_array_mut()
            .expect("cards")
            .push(duplicate);
        let error = build_with_overrides(&names, &source_overrides(path, &source))
            .expect_err("conflicting metadata must fail");
        assert!(error.contains("conflicting captured metadata"), "{error}");
    }
    let path = SOURCE_PATHS[8];
    let (_, mut source) = read_json(path, &mut read_saved).expect("identity capture");
    source["data"][1]["oracle_id"] = source["data"][0]["oracle_id"].clone();
    let error = build_with_overrides(&names, &source_overrides(path, &source))
        .expect_err("different names cannot share an Oracle ID");
    assert!(
        error.contains("duplicate or conflicting Oracle identity"),
        "{error}"
    );
}

/// Agreeing duplicates keep the first source record and its original color order.
#[test]
fn matching_duplicates_keep_source_precedence() {
    let path = SOURCE_PATHS[3];
    let (_, mut source) = read_json(path, &mut read_saved).expect("historical source");
    let mut duplicate = source["data"][4].clone();
    duplicate["color_identity"] = json!(["U", "G"]);
    duplicate["uri"] = json!("https://api.scryfall.com/cards/other-printing");
    source["data"]
        .as_array_mut()
        .expect("cards")
        .push(duplicate);
    let rebuilt = build_with_overrides(&current_names(), &source_overrides(path, &source))
        .expect("matching metadata is unambiguous");
    let card = &rebuilt["cards"][0];
    assert_eq!(card["source_pointer"], "/data/4");
    assert_eq!(card["color_identity"], json!(["G", "U"]));
    assert_eq!(card["source_uri"], source["data"][4]["uri"]);
}

/// Sources must contain valid JSON, declared provenance, and valid Scryfall records.
#[test]
fn rebuild_rejects_invalid_source_declarations() {
    let names = current_names();
    let invalid_json = BTreeMap::from([(SOURCE_PATHS[0].to_string(), b"not JSON".to_vec())]);
    assert!(
        build_with_overrides(&names, &invalid_json)
            .expect_err("invalid JSON")
            .contains("invalid JSON")
    );
    let (_, mut precon) = read_json(SOURCE_PATHS[0], &mut read_saved).expect("precon");
    precon
        .as_object_mut()
        .expect("precon object")
        .remove("card_data_version");
    let error = build_with_overrides(&names, &source_overrides(SOURCE_PATHS[0], &precon))
        .expect_err("missing declared MTGJSON provenance");
    assert!(error.contains("card_data_version"));
    for source in [
        json!({"object": "error", "details": "failed lookup"}),
        json!({"object": "list", "data": [], "not_found": [{"name": null}]}),
        json!({"object": "list", "data": [], "not_found": [{"name": ""}]}),
        json!({"object": "list", "data": [], "not_found": [{"name": "missing", "extra": true}]}),
        json!({"object": "list", "data": {}, "not_found": []}),
        json!({"object": "list", "data": [{"object": "error"}], "not_found": []}),
    ] {
        let error = build_with_overrides(&names, &source_overrides(SOURCE_PATHS[3], &source))
            .expect_err("bad source declaration");
        assert!(
            error.contains("invalid or incomplete Scryfall source"),
            "{error}"
        );
    }
}

/// Related-card links cannot supply metadata, but malformed full records must fail.
#[test]
fn matching_related_cards_are_not_full_metadata_records() {
    let path = SOURCE_PATHS[4];
    let (_, source) = read_json(path, &mut read_saved).expect("saved Scryfall source");
    let card = &source["data"][25];
    let related = &card["all_parts"][1];
    assert_eq!(related["object"], "related_card");
    assert_eq!(related["name"], card["name"]);
    assert!(related.get("oracle_id").is_none());
    let names = BTreeSet::from([text(card, "name", path)
        .expect("full card name")
        .to_string()]);
    let mut cards = BTreeMap::new();
    collect_cards(card, "/data/25", path, &names, &mut cards).expect("full card and related link");
    let record = &cards.values().next().expect("one full card").record;
    assert_eq!(cards.len(), 1);
    assert_eq!(record["oracle_id"], card["oracle_id"]);
    assert_eq!(record["source_pointer"], "/data/25");
    let mut related_only = BTreeMap::new();
    collect_cards(
        related,
        "/data/25/all_parts/1",
        path,
        &names,
        &mut related_only,
    )
    .expect("related links are not metadata");
    assert!(related_only.is_empty());
    for (field, message) in [
        ("oracle_id", "oracle_id"),
        ("color_identity", "color identity"),
    ] {
        let mut malformed = card.clone();
        malformed
            .as_object_mut()
            .expect("full card object")
            .remove(field);
        let error = collect_cards(&malformed, "/data/25", path, &names, &mut BTreeMap::new())
            .expect_err("full card records still require complete metadata");
        assert!(error.contains(message), "{error}");
    }
}

/// A saved partial lookup is valid only if all legacy names have metadata elsewhere.
#[test]
fn partial_capture_uses_later_source_and_still_requires_global_coverage() {
    let path = SOURCE_PATHS[9];
    let (_, partial) = read_json(path, &mut read_saved).expect("saved partial Vehicle capture");
    let missing_name = text(&partial["not_found"][0], "name", path).expect("missing lookup name");
    validate_scryfall_source(&partial, path).expect("valid partial lookup");
    let rebuilt =
        build(&current_names(), &mut read_saved).expect("all saved sources cover the inventory");
    let record = rebuilt["cards"]
        .as_array()
        .expect("supplement cards")
        .iter()
        .find(|card| card["name"] == missing_name)
        .expect("later capture supplies the missing card");
    assert_eq!(record["source_path"], SOURCE_PATHS[10]);
    assert_eq!(record["stats_pointer"], "/data/0/card_faces/0");
    assert_eq!(
        record["printed_stats"],
        json!({"power": null, "toughness": null, "loyalty": null})
    );
    let path = SOURCE_PATHS[10];
    let directory = path
        .strip_suffix("response.json")
        .expect("capture directory");
    let (_, request) = read_json(&format!("{directory}request.json"), &mut read_saved)
        .expect("front-face capture request");
    let unresolved = json!({"object": "list", "data": [], "not_found": request["identifiers"]});
    let error = build_with_overrides(&current_names(), &source_overrides(path, &unresolved))
        .expect_err("partial sources cannot leave a legacy name without metadata");
    assert!(error.contains("missing pinned metadata"), "{error}");
    assert!(error.contains(missing_name), "{error}");
}

/// Collection requests must account for returned cards and valid missing-name entries.
#[test]
fn capture_selection_accepts_partial_results_but_rejects_mismatches() {
    let request = json!({"identifiers": [{"name": "Front"}]});
    let card = json!({
        "object": "card", "name": "Front // Back",
        "card_faces": [{"name": "Front"}, {"name": "Back"}],
    });
    let response = json!({"object": "list", "data": [card], "not_found": []});
    validate_capture_selection(&request, &response, "request.json").expect("front-face request");
    let partial = json!({"object": "list", "data": [], "not_found": [{"name": "Front"}]});
    validate_capture_selection(&request, &partial, "request.json").expect("declared missing name");
    for invalid in [
        json!({"data": [], "not_found": []}),
        json!({"data": [], "not_found": [{"name": "Other"}]}),
        json!({"data": [], "not_found": [{"name": "Front"}, {"name": "Front"}]}),
        json!({"data": [card], "not_found": [{"name": "Front"}]}),
        json!({"data": [card, {"object": "card", "name": "Other"}], "not_found": []}),
    ] {
        assert!(
            validate_capture_selection(&request, &invalid, "request.json").is_err(),
            "{invalid}"
        );
    }
}

/// Identity fields and printed stats cannot use malformed values or invalid front faces.
#[test]
fn captured_cards_reject_invalid_metadata() {
    let original = json!({
        "name": "Commander", "oracle_id": "oracle-id", "color_identity": [],
        "scryfall_id": "printing-id", "power": "*", "toughness": "1+*",
    });
    for (field, value) in [
        ("oracle_id", Value::Null),
        ("color_identity", Value::Null),
        ("color_identity", json!(["W", "W"])),
        ("color_identity", json!(["C"])),
        ("uri", json!("http://example.com/card")),
        ("scryfall_id", Value::Null),
        ("power", json!(5)),
        ("toughness", json!(false)),
        ("loyalty", json!([])),
        ("card_faces", Value::Null),
        ("card_faces", json!([])),
        ("card_faces", json!([null])),
    ] {
        let mut source = original.clone();
        source[field] = value;
        assert!(
            captured_card(&source, "", "source.json").is_err(),
            "{source}"
        );
    }
}

/// Exact pointers and first-face nulls prevent accidental use of back-face stats.
#[test]
fn captured_cards_preserve_front_face_nulls_variable_stats_and_pointer_escapes() {
    let card = json!({
        "object": "card", "name": "Commander", "oracle_id": "oracle-id", "color_identity": [],
        "scryfall_id": "printing-id", "power": "99", "toughness": "99", "loyalty": "99",
        "card_faces": [
            {"name": "Front", "power": null, "toughness": "1+*"},
            {"name": "Back", "power": "42", "toughness": "42", "loyalty": "42"},
        ],
    });
    let source = json!({"~captured/cards": [card]});
    let names = BTreeSet::from(["Commander".to_string()]);
    let mut cards = BTreeMap::new();
    collect_cards(&source, "", "source.json", &names, &mut cards).expect("front-face metadata");
    let record = &cards["Commander"].record;
    assert_eq!(record["source_pointer"], "/~0captured~1cards/0");
    assert_eq!(record["stats_pointer"], "/~0captured~1cards/0/card_faces/0");
    assert_eq!(
        record["printed_stats"],
        json!({"power": null, "toughness": "1+*", "loyalty": null})
    );
    assert_eq!(
        record["source_uri"],
        "https://api.scryfall.com/cards/printing-id"
    );
    let (_, sisay) = read_json(SOURCE_PATHS[7], &mut read_saved).expect("root card source");
    let root = captured_card(&sisay, "", SOURCE_PATHS[7]).expect("root card metadata");
    assert_eq!(root.record["source_pointer"], "");
    assert_eq!(root.record["stats_pointer"], "");
}

/// Raw-byte hash changes and missing manifest entries cannot masquerade as pinned inputs.
#[test]
fn rebuild_checks_raw_hashes_and_historical_manifest_entries() {
    let names = current_names();
    for path in [SOURCE_PATHS[3], SOURCE_PATHS[8]] {
        let mut bytes = read_saved(path).expect("saved source");
        bytes.push(b'\n');
        let error = build_with_overrides(&names, &BTreeMap::from([(path.to_string(), bytes)]))
            .expect_err("whitespace changes raw source hashes");
        assert!(
            error.contains("hash") || error.contains("provenance"),
            "{error}"
        );
    }
    let (_, original) = read_json(POPULAR_MANIFEST, &mut read_saved).expect("historical manifest");
    let relative = "sources/scryfall_1.json";
    for duplicate in [false, true] {
        let mut manifest = original.clone();
        let files = manifest["files"].as_array_mut().expect("manifest files");
        if duplicate {
            let entry = files
                .iter()
                .find(|file| file["path"] == relative)
                .expect("source entry")
                .clone();
            files.push(entry);
        } else {
            files.retain(|file| file["path"] != relative);
        }
        let overrides = BTreeMap::from([(
            POPULAR_MANIFEST.to_string(),
            serde_json::to_vec(&manifest).expect("manifest serializes"),
        )]);
        assert!(build_with_overrides(&names, &overrides).is_err());
    }
}

/// Capture declarations must agree with both saved payloads and the read-only endpoint.
#[test]
fn rebuild_rejects_invalid_capture_provenance() {
    let names = current_names();
    let path = "src/deck/simulator/tests/fixture_identities/sources/2026-10-29/capture.json";
    let (_, original) = read_json(path, &mut read_saved).expect("capture manifest");
    for (field, value) in [
        ("source", json!("Other source")),
        ("endpoint", json!("https://example.com/write")),
        ("method", json!("DELETE")),
        ("operation", json!("write")),
        ("captured_at", Value::Null),
        ("request_sha256", json!("0".repeat(64))),
        ("response_sha256", json!("0".repeat(64))),
    ] {
        let mut capture = original.clone();
        capture[field] = value;
        let overrides = BTreeMap::from([(
            path.to_string(),
            serde_json::to_vec(&capture).expect("capture serializes"),
        )]);
        assert!(build_with_overrides(&names, &overrides).is_err(), "{field}");
    }
    let request_path =
        "src/deck/simulator/tests/fixture_identities/sources/2026-10-29/request.json";
    let bytes = serde_json::to_vec(&json!({"identifiers": [{"name": null}]}))
        .expect("changed request serializes");
    let mut capture = original;
    capture["request_sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
    let overrides = BTreeMap::from([
        (request_path.to_string(), bytes),
        (
            path.to_string(),
            serde_json::to_vec(&capture).expect("capture serializes"),
        ),
    ]);
    assert!(build_with_overrides(&names, &overrides).is_err());
}

/// Write a new review candidate in a retained temporary directory, never over a pin.
#[test]
#[ignore = "writes a new temporary review file for explicit fixture maintenance"]
fn write_fixture_metadata_review() {
    use std::io::Write;

    let document = build(&current_names(), &mut read_saved).expect("offline supplement rebuild");
    let directory = tempfile::Builder::new()
        .prefix("stm-fixture-metadata-review-")
        .tempdir()
        .expect("fresh review directory");
    let path = directory.path().join("supplement.review.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create a new review candidate");
    serde_json::to_writer_pretty(&mut file, &document).expect("write review JSON");
    file.write_all(b"\n").expect("finish review JSON");
    let _ = directory.keep();
    println!("{}", path.display());
}
