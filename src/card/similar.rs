use super::{ambiguous_error, card_json};
use anyhow::Context;
/// One tag-overlap neighbor: the card, its shared tag labels, and the count.
#[derive(Debug, Clone)]
pub struct SimilarHit {
    pub card: crate::db::CardRow,
    pub shared_count: i64,
    pub shared_tags: Vec<String>,
    /// Hybrid rank in `[0, 1]` (reciprocal-rank fusion of the tag leg and
    /// the stored-vector cosine leg); `None` when only the tag leg ran.
    pub score: Option<f32>,
}

/// Rank oracle cards by oracle-tag overlap with `seed`.
///
/// Counts shared tag ids via `card_tags` (join on oracle_id), excludes the
/// seed itself, breaks ties toward lower EDHREC rank then name — the same
/// tie-break order as query fusion.
///
/// When `restrict` is set, the SQL over-selects (4× the window) so the
/// Rust-side filter cannot empty the result before the true `limit` best
/// owned matches are reached; the final list is truncated here.
///
/// # Errors
/// Propagates SQLite failures.
pub fn rank_similar(
    conn: &rusqlite::Connection,
    seed_oracle_id: &str,
    limit: usize,
    restrict: Option<&std::collections::HashSet<String>>,
) -> anyhow::Result<Vec<SimilarHit>> {
    // Over-select when a Rust-side filter will drop rows after the SQL cut.
    let sql_limit = if restrict.is_some() {
        limit.saturating_mul(4).max(50)
    } else {
        limit
    };
    let mut stmt = conn.prepare(
        "SELECT c.name, c.oracle_id, c.mana_cost, c.cmc, c.type_line, c.colors,
                c.color_identity, c.keywords, c.power, c.toughness, c.loyalty,
                c.oracle_text, c.rarity, c.edhrec_rank, c.legalities,
                c.set_code, c.collector_number, c.scryfall_id, c.released_at,
                c.game_changer,
                COUNT(ct2.tag_id) AS shared,
                GROUP_CONCAT(t2.label, '\u{1}') AS labels
         FROM card_tags ct
         JOIN card_tags ct2
           ON ct2.tag_id = ct.tag_id AND ct2.oracle_id != ct.oracle_id
         JOIN cards c ON c.oracle_id = ct2.oracle_id
         JOIN tags t2 ON t2.id = ct2.tag_id
         WHERE ct.oracle_id = ?1
         GROUP BY ct2.oracle_id
         ORDER BY shared DESC, c.edhrec_rank IS NULL, c.edhrec_rank ASC, c.name ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![seed_oracle_id, sql_limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
            row.get::<_, Option<i64>>(13)?,
            row.get::<_, String>(14)?,
            row.get::<_, String>(15)?,
            row.get::<_, String>(16)?,
            row.get::<_, String>(17)?,
            row.get::<_, String>(18)?,
            row.get::<_, Option<bool>>(19)?,
            row.get::<_, i64>(20)?,
            row.get::<_, String>(21)?,
        ))
    })?;
    let mut hits = Vec::new();
    for row in rows {
        let (
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
            legalities,
            set_code,
            collector_number,
            scryfall_id,
            released_at,
            game_changer,
            shared,
            labels,
        ) = row.context("reading similar card")?;
        if restrict.is_some_and(|set| !set.contains(&name)) {
            continue;
        }
        let mut shared_tags: Vec<String> = labels.split('\u{1}').map(str::to_string).collect();
        shared_tags.sort_by_key(|a| a.to_lowercase());
        hits.push(SimilarHit {
            card: crate::db::CardRow {
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
                legalities,
                set_code,
                collector_number,
                scryfall_id,
                released_at,
                game_changer,
            },
            shared_count: shared,
            shared_tags,
            score: None,
        });
    }
    hits.truncate(limit);
    Ok(hits)
}

/// The seed's vector store row, when the store and the row both exist.
/// Reading the stored row needs no model load: the seed was embedded at
/// sync time.
fn seed_vector(
    paths: &crate::paths::Paths,
    seed_name: &str,
) -> Option<(crate::embed::VectorStore, usize, Vec<f32>)> {
    let store = crate::embed::VectorStore::load(paths.root()).ok()?;
    let idx = store.meta.index_of(seed_name)?;
    let row = store.row(idx).to_vec();
    Some((store, idx, row))
}

/// Cosine ranks against the seed's stored vector: `(name, score)` pairs,
/// best first, excluding the seed row. Rows are unit-normalized, so dot
/// product is cosine. When `restrict` is set the filter applies BEFORE
/// ranking, matching `run_search`: ranks drive RRF, so a filtered-out
/// store must not inflate the rank distance of the survivors.
fn vector_leg(
    store: &crate::embed::VectorStore,
    seed_row: usize,
    seed: &[f32],
    depth: usize,
    restrict: Option<&std::collections::HashSet<String>>,
) -> Vec<(String, f64)> {
    let mut scored: Vec<(usize, f32)> = (0..store.meta.names.len())
        .filter(|i| *i != seed_row)
        .filter(|i| {
            let name = &store.meta.names[*i];
            restrict.is_none_or(|set| set.contains(name))
        })
        .map(|i| {
            let row = store.row(i);
            let dot: f32 = seed.iter().zip(row).map(|(q, v)| q * v).sum();
            (i, dot)
        })
        .collect();
    scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    scored
        .into_iter()
        .take(depth)
        .map(|(i, score)| (store.meta.names[i].clone(), score as f64))
        .collect()
}

/// Entry point for `stm card similar <name>`.
///
/// Exit 3 when the seed card is unknown or has no tags, or when no neighbor
/// survives the filters. Exit 0 otherwise.
#[allow(clippy::too_many_arguments)]
pub fn run_similar(
    paths: &crate::paths::Paths,
    conn: &mut rusqlite::Connection,
    out: &mut crate::output::Output,
    typed: &str,
    limit: u32,
    owned_only: bool,
    json: bool,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        out.error("card index not built yet");
        out.hint("run 'stm setup' first");
        return Ok(crate::cli::codes::ERROR);
    }
    let seed = match crate::db::resolve_name(conn, typed).context("resolving card name")? {
        crate::db::NameMatch::Found(card) => card,
        crate::db::NameMatch::Ambiguous { candidates, total } => {
            return ambiguous_error(out, typed, &candidates, total);
        }
        crate::db::NameMatch::NotFound => {
            out.error(&format!("no card named {typed:?}"));
            out.hint("names resolve by exact match, case, or unique prefix");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
    };
    let restrict = if owned_only {
        let owned = crate::collection::owned_names_all(conn)?;
        if owned.is_empty() {
            out.error("collection is empty");
            out.hint("import a ManaBox CSV: stm collection import <file>");
            return Ok(crate::cli::codes::NO_RESULTS);
        }
        Some(owned)
    } else {
        None
    };
    let tag_hits = rank_similar(
        conn,
        &seed.oracle_id,
        (limit as usize).saturating_mul(2).max(30),
        restrict.as_ref(),
    )?;
    // Fused hybrid when the seed has a stored vector: tag-overlap ranks and
    // cosine-to-seed ranks fuse by reciprocal rank fusion, like `query`.
    // No stored vector (unembedded card, absent store): tags only, with a
    // note on the human view.
    let (seed_store, seed_row, seed_vec) = match seed_vector(paths, &seed.name) {
        Some(tuple) => tuple,
        None => {
            let mut hits = tag_hits;
            hits.truncate(limit as usize);
            return finish_similar(conn, out, &seed.name, hits, false, json);
        }
    };
    let vector_hits = vector_leg(
        &seed_store,
        seed_row,
        &seed_vec,
        (limit as usize).saturating_mul(2).max(30),
        restrict.as_ref(),
    );
    let tag_list: Vec<(String, f64)> = tag_hits
        .iter()
        .map(|h| (h.card.name.clone(), h.shared_count as f64))
        .collect();
    // EDHREC rank breaks fusion ties; the map covers both legs' names.
    let mut ranks: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for hit in &tag_hits {
        if let Some(rank) = hit.card.edhrec_rank {
            ranks.insert(hit.card.name.as_str(), rank);
        }
    }
    let mut vector_names: Vec<&str> = vector_hits.iter().map(|(n, _)| n.as_str()).collect();
    vector_names.sort_unstable();
    vector_names.dedup();
    let mut vector_rows: Vec<crate::db::CardRow> = Vec::new();
    for name in &vector_names {
        if let Some(card) = crate::db::get_card(conn, name)? {
            vector_rows.push(card);
        }
    }
    for card in &vector_rows {
        if let Some(rank) = card.edhrec_rank {
            ranks.entry(card.name.as_str()).or_insert(rank);
        }
    }
    let fused = crate::query::fuse_rrf(&tag_list, &vector_hits, limit as usize, |name| {
        ranks.get(name).copied()
    });
    // Re-attach cards + tags to the fused ranking. Vector-only names
    // (no shared tags) carry `shared_count: 0` / empty tags so the fused
    // window never shrinks below `--limit`. A fused name with no card row
    // (store ahead of the oracle) still keeps its slot with a stub card so
    // the result count stays stable.
    let mut hits: Vec<SimilarHit> = fused
        .into_iter()
        .filter_map(|(name, score)| {
            if let Some(hit) = tag_hits.iter().find(|h| h.card.name == name) {
                let mut hit = hit.clone();
                hit.score = Some(score);
                return Some(hit);
            }
            let card = vector_rows
                .iter()
                .find(|c| c.name == name)
                .cloned()
                .or_else(|| crate::db::get_card(conn, name.as_str()).ok().flatten())?;
            Some(SimilarHit {
                card,
                shared_count: 0,
                shared_tags: Vec::new(),
                score: Some(score),
            })
        })
        .collect();
    hits.truncate(limit as usize);
    finish_similar(conn, out, &seed.name, hits, true, json)
}

/// Print or emit the final similar list. `hybrid` says whether the vector
/// leg ran (drives the header note and the empty-set hint).
pub(super) fn finish_similar(
    conn: &mut rusqlite::Connection,
    out: &mut crate::output::Output,
    seed_name: &str,
    hits: Vec<SimilarHit>,
    hybrid: bool,
    json: bool,
) -> anyhow::Result<i32> {
    if hits.is_empty() {
        if json {
            println!("[]");
        } else {
            out.error(&format!("no cards share tags with {seed_name}"));
            out.hint("the card may be untagged, or the filters emptied the result set");
        }
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    if json {
        let tag_index = crate::tags::TagIndex::load(conn)?;
        let names: Vec<String> = hits.iter().map(|hit| hit.card.name.clone()).collect();
        // One batched query per finish kind instead of four per card name.
        let ranges = crate::prints::price_ranges(conn, &names)?;
        let owned_all = crate::collection::owned_counts_all(conn)?;
        let available_all = crate::collection::available_counts_all(conn)?;
        let items: Vec<serde_json::Value> = hits
            .iter()
            .map(|hit| {
                let range = ranges.get(&hit.card.name).cloned().unwrap_or_default();
                let universe =
                    crate::universe::card_universe(conn, &hit.card.name, &hit.card.set_code)
                        .unwrap_or(crate::universe::CardUniverse {
                            universe: "multiverse",
                            franchise: None,
                            set_name: None,
                            set_type: None,
                            block: None,
                        });
                let mut v = card_json(
                    &hit.card,
                    &tag_index,
                    &range,
                    &universe,
                    &owned_all,
                    &available_all,
                );
                // Null score when the seed had no stored vector (tags-only
                // ranking); agents can tell "no score" from "unranked".
                v["score"] = match hit.score {
                    Some(score) => {
                        serde_json::json!((f64::from(score) * 10_000.0).round() / 10_000.0)
                    }
                    None => serde_json::Value::Null,
                };
                v["shared_count"] = serde_json::json!(hit.shared_count);
                v["shared_tags"] = serde_json::json!(hit.shared_tags);
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        print_similar_text(out, seed_name, &hits, hybrid);
    }
    Ok(crate::cli::codes::OK)
}

/// Human table for `card similar`: rank, name, cost, type, score, shared
/// tags. Owned/price context joins via `--owned` runs.
fn print_similar_text(
    out: &crate::output::Output,
    seed_name: &str,
    hits: &[SimilarHit],
    hybrid: bool,
) {
    let styles = out.styles();
    let why = if hybrid {
        "tag overlap + meaning"
    } else {
        "tag overlap"
    };
    println!(
        "{} {} {}",
        styles.header("Similar to"),
        styles.card_name(seed_name),
        styles.dim(&format!("({why})"))
    );
    for (i, hit) in hits.iter().enumerate() {
        let score_note = match hit.score {
            Some(score) => format!("({:.3})", score),
            None => format!(
                "({} common tag{})",
                hit.shared_count,
                if hit.shared_count == 1 { "" } else { "s" }
            ),
        };
        let tags = if hit.shared_tags.is_empty() {
            String::new()
        } else {
            format!(
                " shares: {}",
                hit.shared_tags
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        println!(
            "{:>2}. {} {} {} {}{}",
            i + 1,
            styles.card_name(&hit.card.name),
            styles.mana_pips(&hit.card.mana_cost),
            styles.dim(&hit.card.type_line),
            styles.dim(&score_note),
            styles.dim(&tags),
        );
    }
}

#[cfg(test)]
#[path = "tests/similar_tests.rs"]
mod similar_tests;
