//! Validate the registered fixture corpus before simulator sweeps use it.
use super::deck_test_support::*;
use sha2::Digest as _;
use std::collections::HashSet;

#[test]
fn registered_fixtures_have_unique_ids_positive_counts_and_resolved_names() {
    let mut ids = HashSet::new();
    let mut format_ids = HashSet::new();
    format_ids.extend(STANDARD_FIXTURES.iter().copied());
    format_ids.extend(MODERN_FIXTURES.iter().copied());
    format_ids.extend(COMMANDER_FIXTURES.iter().copied());
    assert_eq!(format_ids, ALL_FIXTURES.iter().copied().collect());
    for name in ALL_FIXTURES {
        assert!(ids.insert(*name), "duplicate fixture registry id: {name}");
        let fixture = fixture(name);
        assert!(
            fixture.commanders.len() + fixture.cards.len() > 0,
            "{name} has no card metadata"
        );
        assert!(!fixture.list.is_empty(), "{name} has no played cards");
        assert!(
            fixture.list.iter().all(|(_, quantity)| *quantity > 0),
            "{name} has a non-positive played quantity"
        );
        let unresolved = unresolved_played_cards(name);
        assert!(
            unresolved.is_empty(),
            "{name} has unresolved played names: {unresolved:?}"
        );

        if let Some(declared_name) = fixture.name {
            assert!(!declared_name.trim().is_empty());
        }
        if fixture.source_kind.is_some() {
            assert!(
                fixture
                    .source
                    .as_deref()
                    .is_some_and(|source| !source.is_empty())
            );
            assert!(
                fixture
                    .source_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
            );
            assert!(
                fixture
                    .retrieved_at
                    .as_deref()
                    .is_some_and(|date| !date.is_empty())
            );
            assert!(
                fixture
                    .product_date
                    .as_deref()
                    .is_some_and(|date| !date.is_empty())
            );
            assert!(
                fixture
                    .card_data_version
                    .as_deref()
                    .is_some_and(|version| !version.is_empty())
            );
            let hash = fixture
                .normalized_content_sha256
                .as_deref()
                .expect("sourced fixture has a normalized content hash");
            assert_eq!(hash.len(), 64, "{name} source hash must be SHA-256");
            assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
            let content = serde_json::json!({
                "cards": &fixture.cards,
                "commanders": &fixture.commanders,
                "list": &fixture.list,
            });
            let actual_hash = format!(
                "{:x}",
                sha2::Sha256::digest(serde_json::to_vec(&content).unwrap())
            );
            assert_eq!(hash, actual_hash, "{name} normalized deck hash changed");
            assert!(
                fixture
                    .sealed_product_uuids
                    .iter()
                    .all(|uuid| !uuid.trim().is_empty())
            );
            let manifest: serde_json::Value =
                serde_json::from_str(include_str!("sourced_deck_fixtures/source_manifest.json"))
                    .expect("source manifest parses");
            assert_eq!(
                manifest["retrieved_on"].as_str(),
                fixture.retrieved_at.as_deref()
            );
            let candidate = manifest["mtgjson_precon_candidates"]
                .as_array()
                .expect("precon candidates are a list")
                .iter()
                .find(|candidate| candidate["fixture_id"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("{name} is missing from the source manifest"));
            assert_eq!(
                candidate["source_id"].as_str(),
                fixture.source_id.as_deref()
            );
            assert_eq!(candidate["source_url"].as_str(), fixture.source.as_deref());
            assert_eq!(candidate["source_kind"].as_str(), Some("official_precon"));
            assert_eq!(fixture.source_kind.as_deref(), Some("official_precon"));
            assert_eq!(
                candidate["snapshot_status"].as_str(),
                Some("frozen_fixture")
            );
            assert_eq!(
                candidate["release_date"].as_str(),
                fixture.product_date.as_deref()
            );
            assert_eq!(
                candidate["retrieved_at"].as_str(),
                fixture.retrieved_at.as_deref()
            );
            assert_eq!(
                candidate["card_data_date"].as_str(),
                fixture.card_data_date.as_deref()
            );
            assert_eq!(
                candidate["card_data_version"].as_str(),
                fixture.card_data_version.as_deref()
            );
            let manifest_sealed = candidate["sealed_product_uuids"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                .or_else(|| {
                    candidate["sealed_product_uuid"]
                        .as_str()
                        .map(|value| vec![value])
                })
                .expect("manifest has sealed product IDs");
            let fixture_sealed: Vec<&str> = fixture
                .sealed_product_uuids
                .iter()
                .map(String::as_str)
                .collect();
            assert_eq!(manifest_sealed, fixture_sealed);
            assert_eq!(candidate["normalized_content_sha256"].as_str(), Some(hash));
        }
        // Provenance stays unknown when the fixture does not provide a source.
        let _source = fixture.source;
    }
}

#[test]
fn fixture_rows_keep_source_card_metadata() {
    for name in ALL_FIXTURES {
        let fixture = fixture(name);
        let cards = fixture_map(name);
        for source in fixture.commanders.iter().chain(&fixture.cards) {
            let row = cards.get(&source.name).expect("fixture card row exists");
            assert_eq!(row.mana_cost, source.cost, "{name}: {} cost", source.name);
            assert_eq!(
                row.type_line, source.type_line,
                "{name}: {} type",
                source.name
            );
            assert_eq!(
                row.keywords, source.keywords,
                "{name}: {} keywords",
                source.name
            );
            assert_eq!(
                row.oracle_text, source.text,
                "{name}: {} Oracle",
                source.name
            );
            assert_eq!(
                row.power,
                source.power.as_deref().and_then(|value| value.parse().ok())
            );
            assert_eq!(
                row.toughness,
                source
                    .toughness
                    .as_deref()
                    .and_then(|value| value.parse().ok())
            );
            assert_eq!(
                row.loyalty,
                source
                    .loyalty
                    .as_deref()
                    .and_then(|value| value.parse().ok())
            );
            assert_eq!(
                row.colors,
                serde_json::to_string(&source.colors.clone().unwrap_or_default()).unwrap()
            );
            assert_eq!(
                row.color_identity,
                serde_json::to_string(&source.color_identity.clone().unwrap_or_default()).unwrap()
            );
            if let Some(oracle_id) = &source.oracle_id {
                assert_eq!(
                    row.oracle_id, *oracle_id,
                    "{name}: {} Oracle ID",
                    source.name
                );
            }
            if source.scryfall_id.is_some() {
                let printing_ids: HashSet<&str> = fixture
                    .commanders
                    .iter()
                    .chain(&fixture.cards)
                    .filter(|candidate| candidate.name == source.name)
                    .filter_map(|candidate| candidate.scryfall_id.as_deref())
                    .collect();
                assert!(
                    printing_ids.contains(row.scryfall_id.as_str()),
                    "{name}: {} Scryfall ID is not one of its source printings",
                    source.name
                );
            }
        }
    }
}

#[test]
fn registered_fixture_totals_match_their_deck_shapes() {
    for name in ALL_FIXTURES {
        let fixture = fixture(name);
        let played_count: i64 = fixture.list.iter().map(|(_, quantity)| quantity).sum();
        if fixture.commanders.is_empty() {
            assert!(
                (60..=75).contains(&played_count),
                "{name} has {played_count} played cards; expected a 60–75 card constructed list"
            );
        } else {
            let total = played_count + fixture.commanders.len() as i64;
            assert_eq!(total, 100, "{name} has {total} cards including commanders");
        }

        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let sim_deck = build_sim_deck(&deck, &cards, fixture.format.as_deref());
        assert_eq!(
            sim_deck.cards.len() + sim_deck.commanders.len(),
            played_count as usize + fixture.commanders.len(),
            "{name} lost cards while building the simulation deck"
        );
    }
}

#[test]
fn oculus_fixture_is_explicitly_unverified_and_keeps_its_legacy_count() {
    let fixture = fixture("oculus manifest modern");
    let played_count: i64 = fixture.list.iter().map(|(_, quantity)| quantity).sum();
    assert_eq!(played_count, 75);
    assert_eq!(fixture.source, None);
}
