//! Current captured Commander sources, separate from historical popular-card fixtures.

use crate::db::CardRow;
use serde_json::Value;
use std::sync::OnceLock;

/// Load each captured input through production face flattening without a store or network.
fn source_rows() -> &'static [CardRow] {
    static ROWS: OnceLock<Vec<CardRow>> = OnceLock::new();
    ROWS.get_or_init(|| {
        [
            include_str!("commander_card_fixtures/sources/2026-10-30/cards.json"),
            include_str!("commander_card_fixtures/sources/2026-10-30-behold/cards.json"),
        ]
        .into_iter()
        .flat_map(|source| {
            serde_json::from_str::<Vec<Value>>(source)
                .expect("captured Commander metadata is valid JSON")
        })
        .map(|card| {
            let mut row = super::card_test_support::metadata_row(&card);
            row.edhrec_rank = card["edhrec_rank"].as_i64();
            row.penny_rank = card["penny_rank"].as_i64();
            row.reserved = card["reserved"].as_bool();
            row.game_changer = card["game_changer"].as_bool();
            row
        })
        .collect()
    })
}

/// Return captured rows for source-driven compiler and runtime checks.
pub(super) fn rows() -> Vec<CardRow> {
    source_rows().to_vec()
}

/// Select one captured Oracle identity by its full name or exact front-face name.
pub(super) fn named_row(name: &str) -> CardRow {
    let mut matching = source_rows()
        .iter()
        .filter(|row| row.name == name || row.name.split(" // ").next() == Some(name));
    let row = matching
        .next()
        .unwrap_or_else(|| panic!("captured Commander source exists: {name}"));
    assert!(matching.next().is_none(), "source name is unique: {name}");
    row.clone()
}
