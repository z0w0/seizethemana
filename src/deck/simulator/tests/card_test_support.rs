//! Shared pinned inputs and ordinary Rust assertions for layered card fixtures.
//!
//! Source capture is a maintenance action, not a test dependency. Save a new dated
//! response rather than editing an old capture. Review its wording diff, then run
//! the ignored `write_card_provenance` test explicitly. Ordinary tests never write.

use crate::db::CardRow;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Compare the seeded library stream without advancing the actual game state.
pub(super) fn assert_library_rng(
    actual: &rand_chacha::ChaCha8Rng,
    expected: &rand_chacha::ChaCha8Rng,
) {
    use rand::RngCore;
    assert_eq!(actual.get_word_pos(), expected.get_word_pos());
    assert_eq!(actual.get_stream(), expected.get_stream());
    let mut actual = actual.clone();
    let mut expected = expected.clone();
    for _ in 0..8 {
        assert_eq!(actual.next_u64(), expected.next_u64());
    }
}

/// Set up a controlled main-phase action without duplicating the shared game-state builder.
pub(super) fn main_phase_state(
    hand: Vec<usize>,
    library: Vec<usize>,
) -> crate::deck::simulator::game::GameState {
    let mut game = crate::deck::simulator::scenario_test_support::state(hand, library);
    game.current_turn = 1;
    game.current_phase = crate::deck::simulator::game::GamePhase::Main;
    game
}

/// Read the preserved historical corpus without a store or network.
pub(super) fn corpus() -> Value {
    serde_json::from_str(include_str!("popular_card_fixtures/corpus.json"))
        .expect("the pinned popular-card corpus is valid JSON")
}

/// Convert metadata once through the production face-flattening path for every test layer.
pub(super) fn metadata_row(metadata: &Value) -> CardRow {
    let mut card: crate::scryfall::ScryfallCard =
        serde_json::from_value(metadata.clone()).expect("pinned Scryfall metadata parses");
    crate::scryfall::flatten_faces(&mut card);
    CardRow {
        name: card.name,
        oracle_id: card.oracle_id,
        mana_cost: card.mana_cost.unwrap_or_default(),
        cmc: card.cmc.expect("pinned cards have a mana value"),
        type_line: card.type_line.expect("pinned cards have a type line"),
        colors: serde_json::to_string(&card.colors.unwrap_or_default()).expect("colors serialize"),
        color_identity: serde_json::to_string(&card.color_identity.unwrap_or_default())
            .expect("color identity serializes"),
        keywords: serde_json::to_string(&card.keywords.unwrap_or_default())
            .expect("keywords serialize"),
        power: card.power,
        toughness: card.toughness,
        loyalty: card.loyalty,
        oracle_text: card.oracle_text.unwrap_or_default(),
        rarity: card.rarity.expect("pinned cards have a rarity"),
        edhrec_rank: None,
        penny_rank: None,
        reserved: None,
        legalities: serde_json::to_string(&card.legalities.expect("pinned cards have legalities"))
            .expect("legalities serialize"),
        set_code: card.set_code.expect("pinned cards have a set code"),
        collector_number: card
            .collector_number
            .expect("pinned cards have a collector number"),
        scryfall_id: card.id.expect("pinned cards have a print ID"),
        released_at: card.released_at.expect("pinned cards have a release date"),
        game_changer: None,
    }
}

/// Separate captures that extend, rather than rewrite, the October 23 selection.
pub(super) const ADDITIONAL_SOURCES: [(&str, &str); 5] = [
    ("sources/2026-10-29/trophy_mage.json", "Trophy Mage"),
    (
        "sources/2026-10-29/sisay.json",
        "Sisay, Weatherlight Captain",
    ),
    ("sources/2026-10-29/ox_of_agonas.json", "Ox of Agonas"),
    ("sources/2026-10-29/sift.json", "Sift"),
    (
        "sources/2026-10-29/gravebreaker_lamia.json",
        "Gravebreaker Lamia",
    ),
];

/// Locate immutable fixture inputs without a local collection or network.
pub(super) fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/deck/simulator/tests/popular_card_fixtures")
}

/// Read one source JSON file from the checked-in bundle.
pub(super) fn read_json(path: &str) -> Value {
    serde_json::from_slice(&std::fs::read(fixture_root().join(path)).expect("source file exists"))
        .expect("source file contains JSON")
}

/// Bind a callable fixture to its immutable input revision before reporting evidence.
pub(super) fn pinned_revision(source: &CardRow) -> Value {
    let provenance = read_json("provenance.json");
    let revision = provenance["cards"]
        .as_array()
        .expect("pinned revisions")
        .iter()
        .find(|revision| revision["oracle_id"] == source.oracle_id)
        .expect("fixture source revision is pinned")
        .clone();
    assert_eq!(
        revision["flattened_input_sha256"],
        crate::deck::simulator::support::card_source_hash(source)
    );
    revision
}

/// Hash raw bytes without interpreting or reformatting them.
pub(super) fn byte_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Hash parsed JSON separately from the raw file-byte contract.
pub(super) fn json_hash(value: &Value) -> String {
    byte_hash(&serde_json::to_vec(value).expect("JSON source serializes"))
}

/// Pin face text independently of metadata and production flattened inputs.
pub(super) fn oracle_hash(metadata: &Value) -> String {
    let faces: Vec<Value> = metadata["card_faces"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|face| json!({"name": face["name"], "oracle_text": face["oracle_text"]}))
        .collect();
    json_hash(&json!({"oracle_text": metadata["oracle_text"], "card_faces": faces}))
}

/// Combine genuine source cards without changing the historical corpus.
pub(super) fn source_cards() -> Vec<Value> {
    let mut cards = corpus()["cards"]
        .as_array()
        .expect("historical corpus contains cards")
        .clone();
    cards.extend(ADDITIONAL_SOURCES.into_iter().map(|(path, name)| {
        let metadata = read_json(path);
        assert_eq!(metadata["name"], name);
        json!({
            "name": name, "source_path": path,
            "metadata_sha256": json_hash(&metadata),
            "oracle_text_sha256": oracle_hash(&metadata),
            "metadata": metadata, "selection_tags": ["phase3_candidates_2026_10_29"]
        })
    }));
    cards
}

/// Resolve a pinned source by Oracle identity and metadata revision.
pub(super) fn source_row(oracle_id: &str, metadata_hash: &str) -> CardRow {
    let card = source_cards()
        .into_iter()
        .find(|card| {
            card["metadata"]["oracle_id"] == oracle_id && card["metadata_sha256"] == metadata_hash
        })
        .expect("registered Oracle identity and revision exist");
    metadata_row(&card["metadata"])
}

/// Use readable names only to select a pinned revision, never as its sole identity.
pub(super) fn named_row(name: &str) -> CardRow {
    let provenance = read_json("provenance.json");
    let revisions: Vec<_> = provenance["cards"]
        .as_array()
        .expect("provenance contains card revisions")
        .iter()
        .filter(|card| {
            card["name"]
                .as_str()
                .is_some_and(|stored| stored == name || stored.split(" // ").next() == Some(name))
        })
        .collect();
    let [revision] = revisions.as_slice() else {
        panic!("{name}: one named source has a pinned identity");
    };
    source_row(
        revision["oracle_id"].as_str().expect("Oracle ID is text"),
        revision["metadata_sha256"]
            .as_str()
            .expect("metadata hash is text"),
    )
}

/// A readable front-face alias and its full MDFC name must resolve the same immutable input.
#[test]
fn front_face_aliases_preserve_the_full_source_revision() {
    for (front, full) in [
        (
            "Bala Ged Recovery",
            "Bala Ged Recovery // Bala Ged Sanctuary",
        ),
        ("Sink into Stupor", "Sink into Stupor // Soporific Springs"),
    ] {
        let alias = named_row(front);
        let canonical = named_row(full);
        assert_eq!(alias.name, full);
        assert_eq!(alias.oracle_id, canonical.oracle_id);
        assert_eq!(pinned_revision(&alias), pinned_revision(&canonical));
        assert_eq!(
            crate::deck::simulator::support::card_source_hash(&alias),
            crate::deck::simulator::support::card_source_hash(&canonical)
        );
    }
}

/// Pin current bytes and semantic inputs while retaining old declared hashes as history.
pub(super) fn build_provenance() -> Value {
    let legacy = read_json("integrity.json");
    let mut paths: Vec<String> = legacy["files"]
        .as_object()
        .expect("legacy manifest contains files")
        .keys()
        .cloned()
        .collect();
    paths.extend(ADDITIONAL_SOURCES.map(|(path, _)| path.to_string()));
    paths.sort();
    let files: Vec<Value> = paths
        .iter()
        .map(|path| {
            let bytes = std::fs::read(fixture_root().join(path)).expect("manifest file exists");
            let canonical = path
                .ends_with(".json")
                .then(|| json_hash(&serde_json::from_slice(&bytes).expect("manifest JSON parses")));
            json!({
                "path": path, "raw_bytes_sha256": byte_hash(&bytes),
                "canonical_json_sha256": canonical,
                "historical_declared_sha256": legacy["files"][path]
            })
        })
        .collect();
    let cards: Vec<Value> = source_cards()
        .iter()
        .map(|card| {
            let row = metadata_row(&card["metadata"]);
            json!({
                "name": row.name, "oracle_id": row.oracle_id,
                "source_path": card["source_path"],
                "metadata_sha256": card["metadata_sha256"],
                "oracle_text_sha256": card["oracle_text_sha256"],
                "flattened_input_sha256": crate::deck::simulator::support::card_source_hash(&row)
            })
        })
        .collect();
    json!({
        "schema_version": 2,
        "historical_capture": "2026-10-23T05:40:09+00:00",
        "additional_capture_date": "2026-10-29",
        "maintenance_note": "Keep old captures unchanged. Capture new response bytes in a new dated directory. Review source and compiler diffs before explicitly rebuilding this manifest. Tests are offline and do not refresh sources.",
        "additional_source_url_pattern": "https://api.scryfall.com/cards/named?exact=<URL-encoded exact name>",
        "files": files, "cards": cards
    })
}

/// Rebuild provenance only after an explicit source-diff review.
#[test]
#[ignore = "maintenance only: review source changes before replacing pinned revision hashes"]
fn write_card_provenance() {
    std::fs::write(
        fixture_root().join("provenance.json"),
        serde_json::to_string_pretty(&build_provenance()).expect("manifest serializes") + "\n",
    )
    .expect("write explicitly requested provenance");
}
