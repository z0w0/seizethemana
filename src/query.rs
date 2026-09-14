use anyhow::Context;

use crate::cli;
use crate::cli::codes;
use crate::db::{self, CardRow};
use crate::embed::{self, VectorStore};
use crate::output::Output;
use crate::paths::Paths;
use crate::search::CardFilters;

/// One ranked search hit: the card plus its similarity score.
pub struct Hit {
    pub card: CardRow,
    /// Hybrid score in `[0, 1]`: normalized reciprocal-rank fusion of the
    /// full-text and vector result lists (see `fuse_rrf`).
    pub score: f32,
}

/// Candidate depth each retrieval leg contributes to the fusion.
///
/// Deep enough that filters cutting candidates during fusion rarely starve
/// the final `--limit` window.
fn fusion_depth(limit: usize) -> usize {
    limit.max(20)
}

/// Reciprocal-rank fusion constant. The original paper's value (60) works
/// without tuning; larger values soften the advantage of top ranks.
const RRF_K: f64 = 60.0;

/// Fuse full-text and vector result lists with reciprocal rank fusion.
///
/// Each list contributes `1 / (k + rank)` per document; a card ranked well by
/// both legs beats a card ranked first by only one. Equal fusion scores break
/// toward lower EDHREC rank (more popular first), then by name for
/// determinism.
///
/// Inputs are `(card name, score)` pairs, best first (scores are ignored;
/// only ranks matter). Returns `(name, score)` with score normalized to
/// `[0, 1]`.
pub fn fuse_rrf(
    fts_hits: &[(String, f64)],
    vector_hits: &[(String, f64)],
    limit: usize,
    edhrec_rank: impl Fn(&str) -> Option<i64>,
) -> Vec<(String, f32)> {
    let mut scores: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for (rank, (name, _)) in fts_hits.iter().enumerate() {
        *scores.entry(name.as_str()).or_insert(0.0) += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    for (rank, (name, _)) in vector_hits.iter().enumerate() {
        *scores.entry(name.as_str()).or_insert(0.0) += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    // Normalize to [0, 1]: a card ranked first on both legs scores
    // 2 / (k + 1); everything else sits below that ceiling.
    let max_score = 2.0 / (RRF_K + 1.0);
    let mut ranked: Vec<(String, f32)> = scores
        .into_iter()
        .map(|(name, score)| (name.to_string(), (score / max_score) as f32))
        .collect();
    ranked.sort_unstable_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| {
                edhrec_rank(&a.0)
                    .unwrap_or(i64::MAX)
                    .cmp(&edhrec_rank(&b.0).unwrap_or(i64::MAX))
            })
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(limit);
    ranked
}

/// Run a hybrid search: full-text (BM25) + vector legs fused by RRF.
///
/// Shared by `stm query` and `stm collection query` (the latter restricts to
/// owned names via `restrict`).
///
/// # Errors
/// Fails when the vector store or model is missing (not set up) or embedding
/// fails.
pub fn run_search(
    paths: &Paths,
    conn: &rusqlite::Connection,
    _out: &Output,
    text: &str,
    filters: &CardFilters,
    limit: usize,
    restrict: Option<&std::collections::HashSet<String>>,
) -> anyhow::Result<Vec<Hit>> {
    if !paths.is_setup() {
        anyhow::bail!("card index not built yet");
    }
    let store =
        VectorStore::load(paths.root()).context("loading vector index (run 'stm setup' first)")?;
    let cards = db::load_all_cards(conn)?;
    let cards_by_name: std::collections::HashMap<&str, &CardRow> =
        cards.iter().map(|c| (c.name.as_str(), c)).collect();
    let names_by_id: std::collections::HashMap<i64, String> = cards
        .iter()
        .enumerate()
        .map(|(i, c)| (i as i64 + 1, c.name.clone()))
        .collect();
    let depth = fusion_depth(limit);
    let allowed = |card: &CardRow| -> bool {
        if let Some(names) = restrict
            && !names.contains(&card.name)
        {
            return false;
        }
        filters.matches(card)
    };

    // Leg 1: vector scan (unchanged semantics), filtered, cut to depth.
    let mut model = embed::load_model(&paths.models_dir(), false)?;
    let query = store.embed_query(&mut model, text)?;
    let vector_hits: Vec<(String, f64)> = store
        .search(&query, usize::MAX)
        .into_iter()
        .filter(|(idx, _)| allowed(&cards[*idx]))
        .take(depth)
        .map(|(idx, score)| (cards[idx].name.clone(), score as f64))
        .collect();

    // Leg 2: full-text BM25 over name/type/oracle text. Punctuation-only
    // queries skip the leg (nothing tokenizes).
    let fts_hits: Vec<(String, f64)> = match db::fts_query(text) {
        None => Vec::new(),
        Some(expr) => db::fts_search(conn, &expr, depth * 4)?
            .into_iter()
            .filter_map(|(id, _score)| {
                let name = names_by_id.get(&id)?;
                let card = cards_by_name.get(name.as_str())?;
                allowed(card).then(|| (name.clone(), 0.0))
            })
            .take(depth)
            .collect(),
    };

    // Fuse on rank, keep the requested window, then map back to cards.
    let fused = fuse_rrf(&fts_hits, &vector_hits, limit, |name| {
        cards_by_name.get(name).and_then(|c| c.edhrec_rank)
    });
    Ok(fused
        .into_iter()
        .filter_map(|(name, score)| {
            let card = cards_by_name.get(name.as_str())?;
            Some(Hit {
                card: (*card).clone(),
                score,
            })
        })
        .collect())
}

/// Entry point for `stm query`.
#[allow(clippy::too_many_arguments)]
pub fn run_query(
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    out: &mut Output,
    text: &str,
    cli_filters: &cli::CardFilters,
    limit: u32,
    json: bool,
) -> anyhow::Result<i32> {
    let filters = CardFilters::from_cli(cli_filters)?;
    let hits = match run_search(paths, conn, out, text, &filters, limit as usize, None) {
        Ok(hits) => hits,
        Err(err) => {
            // Distinguish "not set up" so the agent knows what to run.
            if !paths.is_setup() {
                out.error(&format!("{err:#}"));
                out.hint("run 'stm setup' first");
                return Ok(codes::ERROR);
            }
            return Err(err);
        }
    };
    if hits.is_empty() {
        return Ok(codes::NO_RESULTS);
    }
    if json {
        let names: Vec<String> = hits.iter().map(|h| h.card.name.clone()).collect();
        let ranges = names
            .iter()
            .map(|n| crate::prints::price_range(conn, n).unwrap_or_default())
            .collect::<Vec<_>>();
        print_json(&hits, &ranges)?;
    } else {
        print_text(out, &hits);
    }
    Ok(codes::OK)
}

/// JSON for one hit; `price_usd` carries the cheapest released English
/// printing's price when any print is priced.
fn hit_json(hit: &Hit, range: &crate::prints::PrintRange) -> serde_json::Value {
    serde_json::json!({
        "name": hit.card.name,
        "mana_cost": hit.card.mana_cost,
        "cmc": hit.card.cmc,
        "type_line": hit.card.type_line,
        "rarity": hit.card.rarity,
        "colors": serde_json::from_str::<serde_json::Value>(&hit.card.colors).unwrap_or_default(),
        "set": hit.card.set_code,
        "collector_number": hit.card.collector_number,
        "score": (hit.score * 10_000.0).round() / 10_000.0,
        "price_usd": range.cheapest.as_ref().and_then(|p| p.usd),
    })
}

/// Print results as a JSON array.
fn print_json(hits: &[Hit], ranges: &[crate::prints::PrintRange]) -> anyhow::Result<()> {
    let items: Vec<serde_json::Value> = hits
        .iter()
        .zip(ranges)
        .map(|(h, range)| hit_json(h, range))
        .collect();
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

/// Print results as a styled table.
fn print_text(out: &Output, hits: &[Hit]) {
    let styles = out.styles();
    for (i, hit) in hits.iter().enumerate() {
        let line = format!(
            "{:>2}. {} {} {} {} {}",
            i + 1,
            styles.card_name(&hit.card.name),
            styles.mana_pips(&hit.card.mana_cost),
            styles.rarity(&hit.card.rarity),
            styles.dim(&hit.card.type_line),
            styles.dim(&format!("({:.3})", hit.score)),
        );
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuse_rrf_rewards_consensus() {
        // A card ranked well on both legs beats a card ranked first on one.
        let fts = vec![("shared".to_string(), 0.0), ("fts_only".to_string(), 0.0)];
        let vector = vec![
            ("vector_only".to_string(), 0.0),
            ("shared".to_string(), 0.0),
        ];
        let fused = fuse_rrf(&fts, &vector, 10, |_| None);
        assert_eq!(fused[0].0, "shared");
        // Scores are normalized to [0, 1].
        assert!((0.0..=1.0).contains(&fused[0].1));
    }

    #[test]
    fn fuse_rrf_normalizes_top_score() {
        // First on both legs: (1/(k+1) + 1/(k+1)) / (2/(k+1)) == 1.0.
        let fts = vec![("bolt".to_string(), 0.0)];
        let vector = vec![("bolt".to_string(), 0.0)];
        let fused = fuse_rrf(&fts, &vector, 1, |_| None);
        assert!((fused[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fuse_rrf_ties_break_toward_popularity_then_name() {
        // zebra (fts #1) and apple (vector #1) tie at 1/(k+1); loved
        // (vector #2) and plain (fts #2) tie at 1/(k+2).
        let fts = vec![("zebra".to_string(), 0.0), ("plain".to_string(), 0.0)];
        let vector = vec![("apple".to_string(), 0.0), ("loved".to_string(), 0.0)];
        let fused = fuse_rrf(&fts, &vector, 10, |name| match name {
            "loved" => Some(5),
            _ => None,
        });
        // Top tie: no EDHREC data either side -> alphabetical.
        assert_eq!(fused[0].0, "apple");
        assert_eq!(fused[1].0, "zebra");
        // Second tie: EDHREC-ranked card first.
        assert_eq!(fused[2].0, "loved");
        assert_eq!(fused[3].0, "plain");
    }

    #[test]
    fn fuse_rrf_respects_limit() {
        let fts: Vec<(String, f64)> = (0..30).map(|i| (format!("card{i}"), 0.0)).collect();
        assert_eq!(fuse_rrf(&fts, &[], 5, |_| None).len(), 5);
    }

    #[test]
    fn hit_json_includes_score_and_price() {
        let card = CardRow {
            name: "Bolt".into(),
            oracle_id: "oid".into(),
            mana_cost: "{R}".into(),
            cmc: 1.0,
            type_line: "Instant".into(),
            colors: r#"["R"]"#.into(),
            color_identity: r#"["R"]"#.into(),
            keywords: "[]".into(),
            power: None,
            toughness: None,
            loyalty: None,
            oracle_text: "Deal 3".into(),
            rarity: "uncommon".into(),
            edhrec_rank: None,
            legalities: "{}".into(),
            set_code: "TST".into(),
            collector_number: "1".into(),
            scryfall_id: "sid-1".into(),
            released_at: String::new(),
            game_changer: None,
        };
        let hit = Hit {
            card,
            score: 0.912345,
        };
        let range = crate::prints::PrintRange {
            cheapest: Some(crate::prints::Print {
                scryfall_id: "sid-1".into(),
                name: "Bolt".into(),
                set_code: "tst".into(),
                set_name: "Test".into(),
                collector_number: "1".into(),
                lang: "en".into(),
                finishes: vec!["nonfoil".into()],
                released_at: "2020-01-01".into(),
                usd: Some(0.99),
                usd_foil: Some(4.5),
                usd_etched: None,
            }),
            priciest: None,
            cheapest_foil: None,
            priciest_foil: None,
        };
        let v = hit_json(&hit, &range);
        assert_eq!(v["name"], "Bolt");
        // Score is rounded to 4 decimals; f32 precision needs tolerance.
        let score = v["score"].as_f64().expect("score is numeric");
        assert!((score - 0.9123).abs() < 1e-4, "score was {score}");
        assert_eq!(v["set"], "TST");
        assert_eq!(v["price_usd"], 0.99);
        // An unpriced card renders null, not a missing field.
        let v = hit_json(&hit, &Default::default());
        assert_eq!(v["price_usd"], serde_json::Value::Null);
    }
}
