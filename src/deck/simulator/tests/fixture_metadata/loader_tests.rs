//! Legacy fixture enrichment preserves source rules and explicit metadata.

use super::*;
use std::collections::BTreeSet;

/// Read exact historical bytes without applying loader enrichment.
fn historical_fixtures() -> Vec<Vec<u8>> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/deck/simulator/tests/deck_fixtures");
    std::fs::read_dir(directory)
        .expect("fixture directory")
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| std::fs::read(path).expect("historical fixture bytes"))
        .collect()
}

/// Compare historical fields outside the supplement's allowed field set.
fn historical_fields(card: &FixtureCard) -> serde_json::Value {
    let mut value = serde_json::to_value(card).expect("fixture card serializes");
    let object = value.as_object_mut().expect("card fields");
    object.remove("color_identity");
    object.remove("oracle_id");
    object.remove("power");
    object.remove("toughness");
    object.remove("loyalty");
    value
}

/// Enrichment must leave historical rules and explicit fields unchanged.
#[test]
fn loader_preserves_historical_card_fields_and_explicit_metadata() {
    for bytes in historical_fixtures() {
        let raw: DeckFixture = serde_json::from_slice(&bytes).expect("historical fixture parses");
        let mut loaded: DeckFixture =
            serde_json::from_slice(&bytes).expect("historical fixture parses");
        enrich_fixture_metadata(&mut loaded);
        assert_eq!(raw.commanders.len(), loaded.commanders.len());
        assert_eq!(raw.cards.len(), loaded.cards.len());
        assert_eq!(raw.list, loaded.list);
        for (original, enriched) in raw
            .commanders
            .iter()
            .chain(&raw.cards)
            .zip(loaded.commanders.iter().chain(&loaded.cards))
        {
            assert_eq!(historical_fields(original), historical_fields(enriched));
            if original.color_identity.is_some() {
                assert_eq!(original.color_identity, enriched.color_identity);
            }
            if original.oracle_id.is_some() {
                assert_eq!(original.oracle_id, enriched.oracle_id);
            }
            for (before, after) in [
                (&original.power, &enriched.power),
                (&original.toughness, &enriched.toughness),
                (&original.loyalty, &enriched.loyalty),
            ] {
                if before.is_some() {
                    assert_eq!(before, after);
                }
            }
            let in_scope = raw.source_kind.is_none()
                && (original.type_line.contains("Vehicle")
                    || raw
                        .commanders
                        .iter()
                        .any(|commander| commander.name == original.name));
            if !in_scope {
                assert_eq!(original.power, enriched.power, "{}", original.name);
                assert_eq!(original.toughness, enriched.toughness, "{}", original.name);
                assert_eq!(original.loyalty, enriched.loyalty, "{}", original.name);
            }
        }
    }
    let loaded = fixture("shorikai vehicles");
    assert_eq!(
        loaded.commanders[0].color_identity,
        Some(vec!["U".into(), "W".into()])
    );
    assert_eq!(
        loaded.commanders[0].oracle_id.as_deref(),
        Some("e2962883-6daf-4a84-af73-917afc73092c")
    );
    let cards = fixture_map("shorikai vehicles");
    assert_eq!(
        cards["Shorikai, Genesis Engine"].color_identity,
        "[\"U\",\"W\"]"
    );
}

/// Every distinct legacy commander with missing metadata has an exact pinned record.
#[test]
fn all_missing_legacy_commander_identities_are_covered() {
    let mut missing = BTreeSet::new();
    for bytes in historical_fixtures() {
        let fixture: DeckFixture = serde_json::from_slice(&bytes).expect("historical fixture JSON");
        for mut commander in fixture.commanders {
            if commander.color_identity.is_none() {
                missing.insert(commander.name.clone());
                assert!(
                    fixture_metadata::enrich(
                        &commander.name,
                        &mut commander.oracle_id,
                        &mut commander.color_identity,
                    )
                    .expect("missing identity has an authoritative pinned source")
                );
                assert!(commander.oracle_id.is_some());
                assert!(commander.color_identity.is_some());
            }
        }
    }
    assert_eq!(missing.len(), 51);
    assert_eq!(fixture_metadata::supplement_sha256().len(), 64);
}

/// Both row loaders retain sourced stats for commanders and Vehicles.
#[test]
fn fixture_rows_retain_sourced_printed_stats() {
    for (fixture_name, card_name, power, toughness) in [
        ("shorikai vehicles", "Shorikai, Genesis Engine", "8", "8"),
        ("shorikai vehicles", "Consulate Dreadnought", "7", "11"),
        ("the ur-dragon dragons", "The Ur-Dragon", "10", "10"),
    ] {
        let rows = fixture_map(fixture_name);
        assert_eq!(rows[card_name].power.as_deref(), Some(power));
        assert_eq!(rows[card_name].toughness.as_deref(), Some(toughness));
        assert!(rows[card_name].loyalty.is_none());
        if let Some(row) = fixture_cards(fixture_name).get(card_name) {
            assert_eq!(row.power.as_deref(), Some(power));
            assert_eq!(row.toughness.as_deref(), Some(toughness));
            assert!(row.loyalty.is_none());
        }
    }
}

/// Numeric enrichment cannot change other card roles or already sourced deck snapshots.
#[test]
fn numeric_loader_scope_excludes_other_rows_and_sourced_decks() {
    let text = include_str!("../deck_fixtures/shorikai_vehicles.json");
    let mut fixture: DeckFixture = serde_json::from_str(text).expect("fixture JSON");
    let card = &mut fixture.cards[0];
    card.name = "The Ur-Dragon".into();
    card.type_line = "Legendary Creature".into();
    card.power = None;
    card.toughness = None;
    enrich_fixture_metadata(&mut fixture);
    assert!(fixture.cards[0].power.is_none());
    assert!(fixture.cards[0].toughness.is_none());
    let mut fixture: DeckFixture = serde_json::from_str(text).expect("fixture JSON");
    fixture.source_kind = Some("official_precon".into());
    enrich_fixture_metadata(&mut fixture);
    assert!(fixture.commanders[0].power.is_none());
    assert!(fixture.commanders[0].toughness.is_none());
}

/// Every eligible legacy name must have sourced stats, including known absent fields.
#[test]
fn all_legacy_commander_and_vehicle_stats_are_covered() {
    let mut names = BTreeSet::new();
    for bytes in historical_fixtures() {
        let raw: DeckFixture = serde_json::from_slice(&bytes).expect("historical fixture JSON");
        if raw.source_kind.is_some() {
            continue;
        }
        for mut card in raw.commanders.into_iter().chain(
            raw.cards
                .into_iter()
                .filter(|card| card.type_line.contains("Vehicle")),
        ) {
            names.insert(card.name.clone());
            fixture_metadata::enrich_stats(
                &card.name,
                card.oracle_id.as_deref(),
                &mut card.power,
                &mut card.toughness,
                &mut card.loyalty,
            )
            .expect("eligible legacy row has exact pinned stats");
        }
    }
    assert_eq!(names.len(), 66);
}

/// Validate capture bytes without network access.
fn checked_capture(relative: &str) -> (serde_json::Value, serde_json::Value) {
    use sha2::{Digest, Sha256};

    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/deck/simulator/tests/fixture_metadata/sources")
        .join(relative);
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.join("capture.json")).expect("capture manifest"),
    )
    .expect("manifest JSON");
    let parts: Vec<serde_json::Value> = ["request", "response"]
        .into_iter()
        .map(|part| {
            let bytes =
                std::fs::read(directory.join(format!("{part}.json"))).expect("capture bytes");
            assert_eq!(
                manifest[format!("{part}_sha256")],
                format!("{:x}", Sha256::digest(&bytes))
            );
            serde_json::from_slice(&bytes).expect("capture JSON")
        })
        .collect();
    let mut parts = parts.into_iter();
    (
        parts.next().expect("request"),
        parts.next().expect("response"),
    )
}

/// The bounded Vehicle captures resolve only the ten missing source names.
#[test]
fn vehicle_capture_scope_and_front_face_retry_are_exact() {
    let (request, response) = checked_capture("2026-10-29-vehicles");
    assert_eq!(
        request["identifiers"]
            .as_array()
            .expect("request names")
            .len(),
        10
    );
    assert_eq!(
        response["data"].as_array().expect("returned cards").len(),
        9
    );
    let full_name = "Spring-Loaded Sawblades // Bladewheel Chariot";
    assert_eq!(
        response["not_found"],
        serde_json::json!([{"name": full_name}])
    );
    let (retry, resolved) = checked_capture("2026-10-29-vehicle-front");
    assert_eq!(
        retry["identifiers"],
        serde_json::json!([{"name": "Spring-Loaded Sawblades"}])
    );
    assert_eq!(resolved["data"].as_array().expect("retry result").len(), 1);
    assert_eq!(resolved["data"][0]["name"], full_name);
    assert_eq!(resolved["not_found"], serde_json::json!([]));
    let requested: BTreeSet<_> = request["identifiers"]
        .as_array()
        .expect("request names")
        .iter()
        .map(|card| card["name"].as_str().expect("name"))
        .collect();
    let returned: BTreeSet<_> = response["data"]
        .as_array()
        .expect("returned cards")
        .iter()
        .chain(resolved["data"].as_array().expect("resolved card"))
        .map(|card| card["name"].as_str().expect("name"))
        .collect();
    assert_eq!(requested, returned);
}

/// Both fixture row loaders must retain every resolved commander identity.
#[test]
fn fixture_rows_retain_resolved_identities_and_duplicate_commanders() {
    for name in COMMANDER_FIXTURES {
        let loaded = fixture(name);
        let rows = fixture_map(name);
        let main_rows = fixture_cards(name);
        for commander in &loaded.commanders {
            let expected = serde_json::to_string(
                commander
                    .color_identity
                    .as_ref()
                    .expect("resolved commander identity"),
            )
            .expect("identity serializes");
            assert_eq!(rows[&commander.name].color_identity, expected, "{name}");
            if let Some(row) = main_rows.get(&commander.name) {
                assert_eq!(row.color_identity, expected, "{name}");
            }
        }
    }
}

/// Explicit empty identities remain colorless when copied to another historical row.
#[test]
fn duplicate_rows_preserve_explicit_colorless_identity() {
    let mut oracle_id = None;
    let mut colors = Some(Vec::new());
    fixture_metadata::inherit(
        "Explicit colorless commander",
        &mut oracle_id,
        &mut colors,
        None,
        &[],
    )
    .expect("matching explicit colorless identity");
    assert_eq!(colors, Some(Vec::new()));
    assert!(oracle_id.is_none());
}

/// Conflicting duplicate metadata must fail without replacing either field.
#[test]
fn duplicate_rows_reject_conflicts_without_partial_enrichment() {
    let mut oracle_id = None;
    let mut colors = Some(vec!["G".to_string()]);
    assert!(
        fixture_metadata::inherit(
            "Conflicting duplicate",
            &mut oracle_id,
            &mut colors,
            Some("known-oracle-id"),
            &["U".into(), "W".into()],
        )
        .is_err()
    );
    assert!(oracle_id.is_none());
    assert_eq!(colors, Some(vec!["G".into()]));
    oracle_id = Some("different-oracle-id".into());
    colors = None;
    assert!(
        fixture_metadata::inherit(
            "Conflicting duplicate",
            &mut oracle_id,
            &mut colors,
            Some("known-oracle-id"),
            &[],
        )
        .is_err()
    );
    assert_eq!(oracle_id.as_deref(), Some("different-oracle-id"));
    assert!(colors.is_none());
}

/// The new read-only capture must retain its complete request and response hashes.
#[test]
fn new_capture_has_exact_request_response_and_name_coverage() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/deck/simulator/tests/fixture_identities/sources/2026-10-29");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.join("capture.json")).expect("capture manifest"),
    )
    .expect("manifest JSON");
    for part in ["request", "response"] {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(directory.join(format!("{part}.json"))).expect("capture bytes");
        assert_eq!(
            manifest[format!("{part}_sha256")],
            format!("{:x}", Sha256::digest(bytes))
        );
    }
    let request: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.join("request.json")).expect("request bytes"),
    )
    .expect("request JSON");
    let response: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.join("response.json")).expect("response bytes"),
    )
    .expect("response JSON");
    let names = |array: &serde_json::Value| {
        array
            .as_array()
            .expect("named records")
            .iter()
            .map(|record| record["name"].as_str().expect("card name").to_string())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(names(&request["identifiers"]), names(&response["data"]));
    assert_eq!(names(&response["data"]).len(), 42);
    assert_eq!(response["not_found"], serde_json::json!([]));
}
