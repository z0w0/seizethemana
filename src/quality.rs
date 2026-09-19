// Query quality metrics and the golden query set for benchmarking the
// hybrid search. Metrics follow standard IR practice: Recall@k, MRR@k, and
// nDCG@k over binary relevance labels. The example binary
// `examples/query_quality.rs` consumes both halves against a real store.

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

/// One curated benchmark query with its relevant card names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenQuery {
    /// Free-text query as a user would type it.
    pub query: String,
    /// Card names that count as relevant (binary relevance).
    pub relevant: Vec<String>,
    /// Optional filter set applied with the query (`--type`, `--format`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<GoldenFilters>,
    /// Short note on what the query exercises.
    #[serde(default)]
    pub note: String,
}

/// Filters attached to a golden query (mirror of the CLI filter subset the
/// benchmarks use).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GoldenFilters {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub filter_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rarity: Option<String>,
}

/// Load the golden set from its JSON file.
///
/// # Errors
/// Propagates IO/parse failures.
pub fn load_golden_set(path: &std::path::Path) -> anyhow::Result<Vec<GoldenQuery>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading golden set {}", path.display()))?;
    let set: Vec<GoldenQuery> = serde_json::from_str(&raw)
        .with_context(|| format!("parsing golden set {}", path.display()))?;
    anyhow::ensure!(!set.is_empty(), "golden set {} is empty", path.display());
    Ok(set)
}

/// Turn a golden query into the CLI filter struct the search pipeline takes.
impl GoldenQuery {
    pub fn filters(&self) -> crate::search::CardFilters {
        let f = self.filters.clone().unwrap_or_default();
        crate::search::CardFilters {
            type_: f.filter_type,
            color: f.color,
            color_identity: None,
            cmc: f.cmc.as_deref().and_then(parse_cmc),
            power: None,
            toughness: None,
            rarity: f.rarity,
            set: None,
            keyword: None,
            oracle_text: None,
            format: f.format,
        }
    }
}

/// Parse a `<=N`-style cmc comparison for the golden set.
fn parse_cmc(s: &str) -> Option<crate::search::NumOp> {
    crate::search::parse_num_op(s).ok()
}

/// Recall@k: fraction of relevant documents present in the top `k` results.
///
/// An empty relevance set yields 0.0 (nothing to be right about).
pub fn recall_at_k(ranked: &[String], relevant: &[String], k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let hits = ranked
        .iter()
        .take(k)
        .filter(|n| relevant.contains(n))
        .count();
    hits as f64 / relevant.len() as f64
}

/// MRR@k: mean of `1/rank` for the first relevant hit inside the top `k`
/// (0.0 when none appears).
pub fn mrr_at_k(ranked: &[String], relevant: &[String], k: usize) -> f64 {
    ranked
        .iter()
        .take(k)
        .position(|n| relevant.contains(n))
        .map(|pos| 1.0 / (pos + 1) as f64)
        .unwrap_or(0.0)
}

/// nDCG@k with binary relevance: DCG gains are 1 for a relevant hit, and
/// normalization divides by the ideal DCG (all relevant docs in the top
/// ranks).
pub fn ndcg_at_k(ranked: &[String], relevant: &[String], k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let discount = |pos: usize| 1.0 / (pos as f64 + 2.0).log2();
    let dcg: f64 = ranked
        .iter()
        .take(k)
        .enumerate()
        .filter(|(_, n)| relevant.contains(n))
        .map(|(pos, _)| discount(pos))
        .sum();
    let ideal_hits = relevant.len().min(k);
    let idcg: f64 = (0..ideal_hits).map(discount).sum();
    if idcg <= 0.0 { 0.0 } else { dcg / idcg }
}

/// Aggregate per-leg scores for one query run.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LegScore {
    pub recall: f64,
    pub mrr: f64,
    pub ndcg: f64,
}

/// Average of per-query scores; `None` when there are no queries.
pub fn mean_leg_score(legs: &[LegScore]) -> Option<LegScore> {
    if legs.is_empty() {
        return None;
    }
    let n = legs.len() as f64;
    Some(LegScore {
        recall: legs.iter().map(|l| l.recall).sum::<f64>() / n,
        mrr: legs.iter().map(|l| l.mrr).sum::<f64>() / n,
        ndcg: legs.iter().map(|l| l.ndcg).sum::<f64>() / n,
    })
}

/// Score one ranked list against one query's relevance labels at cutoff `k`.
pub fn score_ranked(ranked: &[String], relevant: &[String], k: usize) -> LegScore {
    LegScore {
        recall: recall_at_k(ranked, relevant, k),
        mrr: mrr_at_k(ranked, relevant, k),
        ndcg: ndcg_at_k(ranked, relevant, k),
    }
}

/// Latency percentiles over warm repeats (microseconds), from samples.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LatencyStats {
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub mean_us: u64,
    pub samples: usize,
}

/// Compute p50/p95/p99/mean from duration samples.
///
/// # Errors
/// Fails when `samples` is empty (nothing to measure).
pub fn latency_stats(samples: &[std::time::Duration]) -> anyhow::Result<LatencyStats> {
    anyhow::ensure!(!samples.is_empty(), "no latency samples");
    let mut micros: Vec<u64> = samples.iter().map(|d| d.as_micros() as u64).collect();
    micros.sort_unstable();
    let pick = |q: f64| -> u64 {
        let idx = ((micros.len() - 1) as f64 * q).round() as usize;
        micros[idx]
    };
    let mean = micros.iter().sum::<u64>() / micros.len() as u64;
    Ok(LatencyStats {
        p50_us: pick(0.50),
        p95_us: pick(0.95),
        p99_us: pick(0.99),
        mean_us: mean,
        samples: micros.len(),
    })
}

#[cfg(test)]
#[path = "tests/quality_tests.rs"]
mod quality_tests;
