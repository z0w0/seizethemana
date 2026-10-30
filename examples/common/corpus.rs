//! Corpus identities cover complete card rows and production embedding documents.

use anyhow::Context as _;
use seizethemana::{db, embed, tags};

use super::fingerprint_strings;

/// Fingerprint all card facts and production documents in database row order.
///
/// This report identity is separate from document-only vector cache identities.
pub fn fingerprint_corpus(
    cards: &[db::CardRow],
    tag_index: &tags::TagIndex,
) -> anyhow::Result<String> {
    let evidence = cards
        .iter()
        .map(|card| {
            serde_json::to_string(&(card_evidence(card), embed::doc_for_row(card, tag_index)))
                .context("encoding corpus card evidence")
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(fingerprint_strings(
        std::iter::once("complete-card-row-and-production-document-v1")
            .chain(evidence.iter().map(String::as_str)),
    ))
}

/// Encode every stored field with fixed names and exact floating-point bits.
fn card_evidence(card: &db::CardRow) -> serde_json::Value {
    // Exhaustive destructuring requires an identity update when CardRow grows.
    let db::CardRow {
        name,
        oracle_id,
        mana_cost,
        cmc,
        type_line,
        colors,
        color_identity,
        keywords,
        power,
        toughness,
        loyalty,
        oracle_text,
        rarity,
        edhrec_rank,
        penny_rank,
        reserved,
        legalities,
        set_code,
        collector_number,
        scryfall_id,
        released_at,
        game_changer,
    } = card;
    serde_json::json!({
        "name": name,
        "oracle_id": oracle_id,
        "mana_cost": mana_cost,
        "cmc_bits": cmc.to_bits(),
        "type_line": type_line,
        "colors": colors,
        "color_identity": color_identity,
        "keywords": keywords,
        "power": power,
        "toughness": toughness,
        "loyalty": loyalty,
        "oracle_text": oracle_text,
        "rarity": rarity,
        "edhrec_rank": edhrec_rank,
        "penny_rank": penny_rank,
        "reserved": reserved,
        "legalities": legalities,
        "set_code": set_code,
        "collector_number": collector_number,
        "scryfall_id": scryfall_id,
        "released_at": released_at,
        "game_changer": game_changer,
    })
}

#[cfg(test)]
#[path = "../tests/corpus_tests.rs"]
mod tests;
