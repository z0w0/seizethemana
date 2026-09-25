// Suggest retrieval pipeline: tag matching, the oracle-id tag map, the
// fusion window constants, and RRF fusion. Split from `suggest.rs` to
// keep each file small; the role-fill and commander paths share every
// helper here.

use super::Role;
use crate::db::CardRow;

use anyhow::Context;
use rusqlite::Connection;

/// Tag ids whose labels match the role or the full query text.
///
/// # Errors
/// Propagates SQLite failures.
pub(in crate::deck) fn matched_tag_ids(
    conn: &Connection,
    role: Option<Role>,
    query: Option<&str>,
) -> anyhow::Result<Vec<(String, String)>> {
    let wanted: Vec<String> = role
        .map(|r| r.tag_labels().iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    // Free-text queries split into whole words so "frog payoff" matches
    // labels containing "frog" or "payoff" but "ramp" cannot match
    // "gives trample" and "draw" cannot match "drawback".
    let needle: Option<Vec<String>> = query
        .map(|q| {
            q.split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() >= 3)
                .map(|w| w.to_lowercase())
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty());
    if wanted.is_empty() && needle.is_none() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT id, label FROM tags")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, label) = row?;
        let lower = label.to_lowercase();
        let tokens: std::collections::HashSet<&str> =
            lower.split(|c: char| !c.is_alphanumeric()).collect();
        let label_matches = |words: &[String]| words.iter().all(|w| tokens.contains(w.as_str()));
        if wanted.iter().any(|w| tokens.contains(w.as_str()))
            || needle.as_ref().is_some_and(|words| label_matches(words))
        {
            out.push((id, label));
        }
    }
    Ok(out)
}

/// Oracle ids carrying any matched tag, with the shared labels.
///
/// # Errors
/// Propagates SQLite failures.
pub(super) fn tag_hits_by_oracle(
    conn: &Connection,
    tag_ids: &[(String, String)],
) -> anyhow::Result<std::collections::HashMap<String, Vec<String>>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    if tag_ids.is_empty() {
        return Ok(map);
    }
    let ids: Vec<String> = tag_ids.iter().map(|(id, _)| id.clone()).collect();
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT ct.oracle_id, t.label
         FROM card_tags ct JOIN tags t ON t.id = ct.tag_id
         WHERE ct.tag_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (oracle_id, tag_label) = row.context("reading tag hits")?;
        map.entry(oracle_id).or_default().push(tag_label);
    }
    Ok(map)
}

/// Fusion window when a price cap is active: 3x the limit so the cap
/// trims the deep pool before the limit cut. Plain searches fuse at the
/// limit (no cap, no waste).
pub(super) fn fetch_limit(limit: u32, capped: bool) -> usize {
    let l = limit as usize;
    if capped {
        l.saturating_mul(3).max(60)
    } else {
        l
    }
}

/// Depth for every fusion path (role fill and commander search): the
/// fetch_limit contract (3x when capped, minimum 60) with a 30 floor so
/// tiny uncapped searches still fuse across both legs. Both paths call
/// this helper; the depth must never drift between them.
pub(in crate::deck) fn fusion_depth(limit: u32, capped: bool) -> usize {
    fetch_limit(limit, capped).max(30)
}

/// Fuse semantic and tag/keyword candidate lists by RRF and re-attach the
/// cards with their labels, in fused-relevance order. EDHREC rank breaks
/// fusion ties (lower is more played), then name. `edhrec_tiebreak`
/// disables the rank tiebreak for non-commander searches (neutral key:
/// fusion score, then name).
pub(super) fn fuse_legs(
    semantic: &[CardRow],
    tag_leg: &[(CardRow, Vec<String>)],
    limit: usize,
    edhrec_tiebreak: bool,
) -> Vec<(CardRow, Vec<String>, f32)> {
    let semantic_list: Vec<(String, f64)> =
        semantic.iter().map(|c| (c.name.clone(), 0.0)).collect();
    let tag_list: Vec<(String, f64)> = tag_leg.iter().map(|(c, _)| (c.name.clone(), 0.0)).collect();
    let mut cards_by_name: std::collections::HashMap<String, CardRow> =
        std::collections::HashMap::new();
    let mut ranks: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut collect = |card: &CardRow| {
        if let Some(rank) = card.edhrec_rank
            && edhrec_tiebreak
        {
            ranks.entry(card.name.clone()).or_insert(rank);
        }
        cards_by_name.insert(card.name.clone(), card.clone());
    };
    for (card, _) in tag_leg {
        collect(card);
    }
    for card in semantic {
        collect(card);
    }
    crate::query::fuse_rrf(&semantic_list, &tag_list, limit, |name| {
        ranks.get(name).copied()
    })
    .into_iter()
    .filter_map(|(name, score)| match cards_by_name.remove(&name) {
        Some(card) => {
            let tags = tag_leg
                .iter()
                .find(|(c, _)| c.name == name)
                .map(|(_, labels)| labels.clone())
                .unwrap_or_default();
            Some((card, tags, score))
        }
        None => {
            // A fused name missing from the candidate pool cannot render;
            // say so instead of dropping the row silently.
            eprintln!("note: fused candidate {name:?} missing from the fetched pool; skipped");
            None
        }
    })
    .collect()
}
