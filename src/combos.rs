//! Shared reads from the Spellbook `combos` and `combo_pieces` tables.
//!
//! `spellbook.rs` parses and stores combos. This module joins variants to
//! their pieces for `card combos`, `deck suggest`, and `deck simulate`.
//! Queries use batches of candidate names and reuse prepared statements.

use anyhow::Context;
use rusqlite::Connection;

use crate::spellbook::{ComboPieceRow, ComboVariant};

/// Batch size for `IN (...)` clauses, inside SQLite's 999 host-parameter
/// limit.
const CHUNK: usize = 500;

/// True when at least one combo piece must be the deck's commander.
pub fn requires_commander(pieces: &[ComboPieceRow]) -> bool {
    pieces.iter().any(|p| p.must_be_commander)
}

/// True when the variant is legal in `format` per its Spellbook legality
/// map. Unknown format keys never pass. The lookup lowercases the key:
/// the Spellbook map stores lowercase formats ("modern"), while callers
/// pass user text ("Modern").
pub fn variant_legal_in(variant: &ComboVariant, format: &str) -> bool {
    variant
        .legalities
        .get(&format.to_ascii_lowercase())
        .copied()
        .unwrap_or(false)
}

/// Keep variants that are legal in `format`.
/// Formats without a commander also exclude variants that require one.
pub fn filter_for_format(
    variants: Vec<(ComboVariant, Vec<ComboPieceRow>)>,
    format: &str,
) -> Vec<(ComboVariant, Vec<ComboPieceRow>)> {
    variants
        .into_iter()
        .filter(|(variant, pieces)| {
            variant_legal_in(variant, format)
                && (is_commander_format(format) || !requires_commander(pieces))
        })
        .collect()
}

/// Formats that can satisfy a combo's "must be commander" requirement.
fn is_commander_format(format: &str) -> bool {
    matches!(format, "commander" | "brawl" | "oathbreaker")
}

/// Load variants that use any of `names`, including their pieces.
/// Results are sorted by ID. Empty input returns an empty list.
///
/// # Errors
/// Propagates SQLite failures.
pub fn load_variants_for(
    conn: &Connection,
    names: &std::collections::HashSet<String>,
) -> anyhow::Result<Vec<(ComboVariant, Vec<ComboPieceRow>)>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<String> = Vec::new();
    let mut all: Vec<&String> = names.iter().collect();
    all.sort();
    for chunk in all.chunks(CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT DISTINCT combo_id FROM combo_pieces WHERE name IN ({placeholders})"
        ))?;
        let params = rusqlite::params_from_iter(chunk.iter().map(|s| s.as_str()));
        let rows = stmt.query_map(params, |r| r.get::<_, String>(0))?;
        ids.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }
    ids.sort();
    ids.dedup();
    load_by_ids(conn, &ids)
}

/// Load `combos` rows plus their pieces by variant id.
fn load_by_ids(
    conn: &Connection,
    ids: &[String],
) -> anyhow::Result<Vec<(ComboVariant, Vec<ComboPieceRow>)>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut combo_stmt = conn.prepare_cached(&format!(
            "SELECT id, produces, mana_value_needed, bracket_tag, legalities, popularity
             FROM combos WHERE id IN ({placeholders}) ORDER BY id"
        ))?;
        let params = rusqlite::params_from_iter(chunk.iter());
        let rows = combo_stmt
            .query_map(params, |r| {
                Ok(ComboVariant {
                    id: r.get(0)?,
                    produces: serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or_default(),
                    mana_value_needed: r.get(2)?,
                    bracket_tag: r.get(3)?,
                    legalities: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                    popularity: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("reading combo variants")?;
        for variant in rows {
            let pieces = pieces_for(conn, &variant.id)?;
            out.push((variant, pieces));
        }
    }
    Ok(out)
}

/// Load one variant's pieces ordered by ordinal, then name.
fn pieces_for(conn: &Connection, id: &str) -> anyhow::Result<Vec<ComboPieceRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT name, ordinal, zones, must_be_commander
         FROM combo_pieces WHERE combo_id = ?1 ORDER BY ordinal, name",
    )?;
    let pieces = stmt
        .query_map([id], |r| {
            Ok(ComboPieceRow {
                name: r.get(0)?,
                ordinal: r.get(1)?,
                zones: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                must_be_commander: r.get::<_, i64>(3)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(pieces)
}

#[cfg(test)]
#[path = "tests/combos_tests.rs"]
mod combos_tests;
