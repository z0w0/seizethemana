//! Check pinned popular-card sources and expose compiler facts without support claims.

use super::deck::build_sim_deck;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use crate::db::CardRow;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;

pub(super) use super::card_test_support::{corpus, metadata_row, named_row as pinned_row};

/// Fingerprint canonical JSON using the same sorted object keys as the maintenance script.
fn content_hash(content: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(content).expect("fixture content serializes"))
    )
}

/// Preserve every simulator input from the exact read-only Lathril database rows.
pub(super) fn lathril_rows() -> HashMap<String, CardRow> {
    let rows: Vec<Value> =
        serde_json::from_str(include_str!("popular_card_fixtures/lathril_rows.json"))
            .expect("exact stored rows parse");
    rows.iter()
        .map(|value| {
            let text = |key: &str| {
                value[key]
                    .as_str()
                    .unwrap_or_else(|| panic!("stored row includes text field {key}"))
                    .to_string()
            };
            let optional_text = |key: &str| value[key].as_str().map(str::to_string);
            let row = CardRow {
                name: text("name"),
                oracle_id: text("oracle_id"),
                mana_cost: text("mana_cost"),
                cmc: value["cmc"].as_f64().expect("stored mana value is numeric"),
                type_line: text("type_line"),
                colors: text("colors"),
                color_identity: text("color_identity"),
                keywords: text("keywords"),
                power: optional_text("power"),
                toughness: optional_text("toughness"),
                loyalty: optional_text("loyalty"),
                oracle_text: text("oracle_text"),
                rarity: text("rarity"),
                edhrec_rank: value["edhrec_rank"].as_i64(),
                penny_rank: value["penny_rank"].as_i64(),
                reserved: value["reserved"].as_i64().map(|flag| flag != 0),
                legalities: text("legalities"),
                set_code: text("set_code"),
                collector_number: text("collector_number"),
                scryfall_id: text("scryfall_id"),
                released_at: text("released_at"),
                game_changer: value["game_changer"].as_i64().map(|flag| flag != 0),
            };
            (row.name.clone(), row)
        })
        .collect()
}

/// Keep exact source hashes and face text separate from compiler and runtime claims.
#[test]
fn popular_card_metadata_and_text_hashes_are_intact() {
    let source = corpus();
    for card in source["cards"].as_array().expect("corpus cards are a list") {
        let metadata = &card["metadata"];
        assert_eq!(card["metadata_sha256"], content_hash(metadata));
        let faces: Vec<Value> = metadata["card_faces"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|face| json!({"name": face["name"], "oracle_text": face["oracle_text"]}))
            .collect();
        let text = json!({"oracle_text": metadata["oracle_text"], "card_faces": faces});
        assert_eq!(card["oracle_text_sha256"], content_hash(&text));
        assert_eq!(card["audit_status"], "unreviewed");
        assert!(metadata["id"].as_str().is_some_and(|id| id.len() == 36));
        assert!(
            metadata["oracle_id"]
                .as_str()
                .is_some_and(|id| id.len() == 36)
        );
    }
}

/// Guard EDHREC source order, displayed rounding, and its actual reporting window.
#[test]
fn popular_observations_preserve_window_counts_and_rank_uncertainty() {
    let source = corpus();
    let popularity = &source["popularity"];
    assert_eq!(popularity["reporting_window"], "Past 2 Years");
    assert!(popularity["window_start"].is_null());
    assert!(popularity["window_end"].is_null());
    let cards = popularity["cards"]
        .as_array()
        .expect("popularity is a list");
    assert_eq!(cards.len(), 250);
    assert_eq!(cards[0]["name"], "Sol Ring");
    assert_eq!(cards[0]["deck_count_display"], "8.64M");
    assert_eq!(cards[0]["num_decks"], 8_636_126);
    assert_eq!(cards[0]["inclusion_display"], "83%");
    for (position, card) in cards.iter().enumerate() {
        assert_eq!(card["source_position"], position + 1);
        assert!(card["explicit_rank"].is_null());
        assert!(
            card["num_decks"]
                .as_u64()
                .expect("deck count is an integer")
                > 0
        );
        assert!(
            card["potential_decks"]
                .as_u64()
                .expect("eligible deck count is an integer")
                >= card["num_decks"]
                    .as_u64()
                    .expect("deck count is an integer")
        );
    }
}

/// Keep the selected deck's physical copy order and exact row snapshot reproducible.
#[test]
fn popular_lathril_fixture_preserves_original_list_and_execution_order() {
    let fixture: Value = serde_json::from_str(include_str!("popular_card_fixtures/lathril.json"))
        .expect("Lathril snapshot parses");
    let raw = include_str!("popular_card_fixtures/lathril.txt");
    assert_eq!(
        fixture["raw_deck_sha256"],
        format!("{:x}", Sha256::digest(raw))
    );
    let parsed = crate::deck::grammar::Deck::parse(raw).expect("selected Lathril list parses");
    let rows = lathril_rows();
    let deck = build_sim_deck(&parsed, &rows, Some("commander"));
    assert_eq!(deck.library_len(), 99);
    assert_eq!(deck.commanders.len(), 1);
    let library: Vec<&str> = deck.cards.iter().map(|card| card.name.as_str()).collect();
    let commanders: Vec<&str> = deck
        .commanders
        .iter()
        .map(|card| card.name.as_str())
        .collect();
    assert_eq!(json!(library), fixture["library_order"]);
    assert_eq!(json!(commanders), fixture["commander_order"]);
    assert_eq!(fixture["database_access"], "read_only");
    let stored: Value =
        serde_json::from_str(include_str!("popular_card_fixtures/lathril_rows.json"))
            .expect("stored rows parse");
    assert_eq!(fixture["card_rows_sha256"], content_hash(&stored));
    assert_eq!(rows.len(), 83);
}

/// Record AST and lowered data as compiler observations, never as exact execution support.
fn compiler_observation(card: &Value) -> Value {
    let row = metadata_row(&card["metadata"]);
    let oracle = parse_oracle_card(&row);
    let lowered = parse_sim_card(&row);
    let ast: Vec<String> = oracle
        .abilities
        .iter()
        .map(|ability| format!("{ability:?}"))
        .collect();
    let runtime: Vec<String> = lowered
        .unlocked_abilities(0)
        .map(|ability| format!("{ability:?}"))
        .collect();
    json!({
        "name": card["name"], "oracle_id": row.oracle_id,
        "oracle_text_sha256": card["oracle_text_sha256"],
        "execution_status": "unreviewed", "reporting_status": "unreviewed",
        "ast_abilities": ast, "lowered_abilities_at_zero_counters": runtime,
        "tap_mana": format!("{:?}", lowered.tap),
        "spell_data": format!("{:?}", lowered.spell_data),
        "static_flags": format!("{:?}", lowered.flags),
        "keyword_flags": format!("{:?}", lowered.keyword_abilities),
        "role": format!("{:?}", lowered.role),
    })
}

/// Identify changed compiler records without dumping the whole historical inventory.
fn compiler_inventory_differences(current: &Value, pinned: &Value) -> Vec<String> {
    let mut differences = Vec::new();
    for section in ["corpus_cards", "exact_lathril_rows"] {
        let current_rows = current[section]
            .as_array()
            .expect("current rows are a list");
        let pinned_rows = pinned[section].as_array().expect("pinned rows are a list");
        if current_rows.len() != pinned_rows.len() {
            differences.push(format!("{section}: card count changed"));
        }
        for row in current_rows {
            let previous = pinned_rows
                .iter()
                .find(|old| old["oracle_id"] == row["oracle_id"]);
            let Some(previous) = previous else {
                differences.push(format!("{section}/{}: new Oracle identity", row["name"]));
                continue;
            };
            let fields: Vec<&str> = row
                .as_object()
                .expect("compiler row is an object")
                .keys()
                .filter(|field| row[*field] != previous[*field])
                .map(String::as_str)
                .collect();
            if !fields.is_empty() {
                differences.push(format!("{section}/{}: {}", row["name"], fields.join(", ")));
            }
        }
    }
    differences
}

/// Detect compiler changes for every corpus card and every exact Lathril input row.
#[test]
fn popular_compiler_inventory_matches_pinned_rows() {
    let source = corpus();
    let observations: Vec<Value> = source["cards"]
        .as_array()
        .expect("corpus cards are a list")
        .iter()
        .map(compiler_observation)
        .collect();
    let mut stored_rows: Vec<CardRow> = lathril_rows().into_values().collect();
    stored_rows.sort_by(|left, right| left.name.cmp(&right.name));
    let stored_observations: Vec<Value> = stored_rows
        .iter()
        .map(|row| {
            let oracle = parse_oracle_card(row);
            let lowered = parse_sim_card(row);
            json!({"name": row.name, "oracle_id": row.oracle_id,
            "ast": format!("{oracle:?}"), "lowered": format!("{lowered:?}"),
            "execution_status": "unreviewed", "reporting_status": "unreviewed"})
        })
        .collect();
    let inventory = json!({
        "status": "compiler_inventory_only_not_support_coverage",
        "reason": "AST nodes and lowered rewards do not prove complete costs, targets, timing, execution, or reporting.",
        "corpus_cards": observations, "exact_lathril_rows": stored_observations,
    });
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/deck/simulator/tests/popular_card_fixtures/compiler_inventory.json");
    if let Some(reviewed_path) = std::env::var_os("STM_REVIEWED_POPULAR_INVENTORY") {
        let reviewed: Value = serde_json::from_slice(
            &std::fs::read(reviewed_path).expect("the reviewed compiler candidate exists"),
        )
        .expect("the reviewed compiler candidate parses");
        assert!(
            inventory == reviewed,
            "compiler behavior changed after source review; review a new candidate before writing the pin"
        );
    }
    if let Some(candidate) = std::env::var_os("STM_POPULAR_INVENTORY_CANDIDATE") {
        std::fs::write(
            candidate,
            serde_json::to_string_pretty(&inventory).expect("inventory serializes") + "\n",
        )
        .expect("write the review candidate without changing the pinned inventory");
    }
    if std::env::var_os("STM_WRITE_POPULAR_INVENTORY").is_some() {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&inventory).expect("inventory serializes") + "\n",
        )
        .expect("write the explicitly requested compiler inventory");
    } else {
        let pinned: Value = serde_json::from_str(
            &std::fs::read_to_string(path)
                .expect("compiler inventory exists; refresh with STM_WRITE_POPULAR_INVENTORY=1"),
        )
        .expect("compiler inventory parses");
        assert!(
            inventory == pinned,
            "compiler behavior changed; review before an explicit refresh:\n{}",
            compiler_inventory_differences(&inventory, &pinned).join("\n")
        );
    }
}
