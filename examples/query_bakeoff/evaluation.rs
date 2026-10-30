use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use anyhow::Context as _;
use seizethemana::paths::Paths;
use seizethemana::{db, embed, query};
use serde::Serialize;

use super::cache::{CacheMetadata, FlatMatrix};
use super::common::{self, GoldenQuery, Metrics};
use super::document::DocumentFormat;
use super::model::{BackendSelection, Candidate, LoadedCandidateModel, embed_query, model_name};
use super::{CUTOFF, FtsTokenizer};

pub(super) const REGRESSION_THRESHOLD: f64 = 0.05;

/// One query's metrics and top results for every retrieval leg.
#[derive(Debug, Serialize)]
struct QueryComparison {
    query_index: usize,
    query: String,
    group: String,
    split: &'static str,
    cli_baseline: Vec<String>,
    fts: Vec<String>,
    vector: Vec<String>,
    hybrid: Vec<String>,
    fts_metrics: Metrics,
    vector_metrics: Metrics,
    hybrid_metrics: Metrics,
    cli_baseline_ndcg: f64,
    hybrid_ndcg_delta: f64,
    review_pool: Vec<String>,
    unjudged_pool: Vec<String>,
    judgment_coverage: f64,
    query_embed_us: u64,
    shared_search_us: u64,
}

/// Aggregate metrics for one candidate, with group and split detail.
#[derive(Debug, Serialize)]
struct CandidateMetrics {
    fts: Metrics,
    vector: Metrics,
    hybrid: Metrics,
    by_group: BTreeMap<String, BTreeMap<String, Metrics>>,
    development_hybrid: Metrics,
    held_out_hybrid: Metrics,
}

/// One model candidate's reproducible bake-off result.
#[derive(Debug, Serialize)]
pub(super) struct CandidateReport {
    key: String,
    model: String,
    model_fingerprint: Option<String>,
    dimension: usize,
    max_length: usize,
    model_max_length: usize,
    document_prompt: String,
    query_prompt: String,
    pooling: String,
    model_card: String,
    requested_backend: String,
    effective_backend: String,
    query_backend: &'static str,
    batch_size: usize,
    intra_threads: usize,
    document_format: String,
    cached: bool,
    document_embedding_secs: Option<f64>,
    cache_metadata: CacheMetadata,
    metrics: CandidateMetrics,
    hybrid_ndcg_delta_vs_cli: f64,
    regressions: Vec<Regression>,
    parity: ParityInfo,
    queries: Vec<QueryComparison>,
}

/// Per-query regression from production hybrid ranking.
#[derive(Debug, Serialize)]
struct Regression {
    query_index: usize,
    query: String,
    group: String,
    baseline_ndcg: f64,
    candidate_ndcg: f64,
}

/// Production-model parity gate result.
#[derive(Debug, Serialize)]
struct ParityInfo {
    required: bool,
    passed: bool,
    compared_queries: usize,
    cutoff: usize,
}

/// Corpus, production reference, and candidate results.
#[derive(Debug, Serialize)]
pub(super) struct BakeoffReport {
    pub(super) generated_at: String,
    pub(super) evaluation_version: &'static str,
    pub(super) label_version: &'static str,
    pub(super) corpus: CorpusInfo,
    pub(super) database_schema_version: i64,
    pub(super) ranking_settings: SettingsInfo,
    pub(super) fts_index: FtsIndexInfo,
    pub(super) production_reference: ProductionReference,
    pub(super) candidates: Vec<CandidateReport>,
}

/// FTS tokenizer variant and temporary rebuild cost.
#[derive(Debug, Serialize)]
pub(super) struct FtsIndexInfo {
    pub(super) tokenizer: &'static str,
    pub(super) database_snapshot_us: Option<u64>,
    pub(super) index_rebuild_us: Option<u64>,
}

/// Corpus and fixed split identity shared by all candidates.
#[derive(Debug, Serialize)]
pub(super) struct CorpusInfo {
    pub(super) cards: usize,
    pub(super) golden_queries: usize,
    pub(super) cutoff: usize,
    pub(super) fingerprint: String,
    pub(super) query_fingerprint: String,
    pub(super) development_queries: usize,
    pub(super) held_out_queries: usize,
    pub(super) behavior_queries: usize,
    pub(super) setup_cost: &'static str,
}

/// Shared ranking settings used in every candidate run.
#[derive(Debug, Serialize)]
pub(super) struct SettingsInfo {
    fts_tokenizer: &'static str,
    fts_column_weights: [f64; 4],
    fts_candidate_depth: usize,
    vector_candidate_depth: usize,
    rrf_k: f64,
    fts_rrf_weight: f64,
    vector_rrf_weight: f64,
    fts_term_operator: &'static str,
    fts_overfetch_multiplier: usize,
    fts_min_sql_rows: usize,
    fts_max_rounds: usize,
}

/// Results and timing from the actual `query::run_search` wrapper.
#[derive(Debug, Serialize)]
pub(super) struct ProductionReference {
    pub(super) model: String,
    pub(super) metrics: Metrics,
    pub(super) end_to_end_us: DurationSummary,
    pub(super) query_count: usize,
    pub(super) ranking_source: &'static str,
}

/// Latency distribution across one production search for each query.
#[derive(Debug, Serialize)]
pub(super) struct DurationSummary {
    samples: usize,
    p50_us: u64,
    p95_us: u64,
    p99_us: u64,
    mean_us: u64,
}

#[derive(Debug)]
/// Production results and timing for one golden query.
pub(super) struct ProductionQuery {
    names: Vec<String>,
    pub(super) metrics: Metrics,
    pub(super) elapsed: std::time::Duration,
}

/// Shared store and benchmark inputs for one candidate's ranking pass.
pub(super) struct EvaluationContext<'a> {
    pub(super) golden: &'a [GoldenQuery],
    pub(super) production: &'a [ProductionQuery],
    pub(super) cards: &'a [db::CardRow],
    pub(super) conn: &'a rusqlite::Connection,
    pub(super) settings: query::SearchSettings,
    pub(super) document_format: DocumentFormat,
    pub(super) fts_tokenizer: FtsTokenizer,
}

/// Convert ranking settings into the stable report representation.
pub(super) fn settings_info(
    settings: query::SearchSettings,
    tokenizer: FtsTokenizer,
) -> SettingsInfo {
    SettingsInfo {
        fts_tokenizer: tokenizer.key(),
        fts_column_weights: settings.fts_column_weights,
        fts_candidate_depth: settings.fts_candidate_depth.max(CUTOFF),
        vector_candidate_depth: settings.vector_candidate_depth.max(CUTOFF),
        rrf_k: settings.rrf_k,
        fts_rrf_weight: settings.fts_rrf_weight,
        vector_rrf_weight: settings.vector_rrf_weight,
        fts_term_operator: match settings.fts_term_operator {
            db::FtsTermOperator::Any => "any",
            db::FtsTermOperator::All => "all",
        },
        fts_overfetch_multiplier: settings.fts_overfetch_multiplier,
        fts_min_sql_rows: settings.fts_min_sql_rows,
        fts_max_rounds: settings.fts_max_rounds,
    }
}

/// Copy the ordered top-k names from one search leg.
fn names(hits: &[query::Hit]) -> Vec<String> {
    hits.iter()
        .take(CUTOFF)
        .map(|hit| hit.card.name.clone())
        .collect()
}

/// Require production BGE vectors to match the `query::run_search` output.
pub(super) fn validate_bge_parity(
    candidate_key: &str,
    query_index: usize,
    query_text: &str,
    candidate: &[String],
    production: &[String],
) -> anyhow::Result<()> {
    if candidate_key == "bge-small-full" {
        anyhow::ensure!(
            candidate == production,
            "production full-precision BGE failed ordered top-{CUTOFF} parity for query {query_index} ({query_text:?})\nrun_search: {production:?}\nbake-off: {candidate:?}"
        );
    }
    Ok(())
}

/// Run each golden query through the production `query::run_search` wrapper.
pub(super) fn production_queries(
    paths: &Paths,
    conn: &rusqlite::Connection,
    golden: &[GoldenQuery],
) -> anyhow::Result<Vec<ProductionQuery>> {
    let output = seizethemana::output::Output::new(false, true, false);
    golden
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let filters = item.search_filters()?;
            let started = Instant::now();
            let hits =
                query::run_search(paths, conn, &output, &item.query, &filters, CUTOFF, None)?;
            let ranked = names(&hits);
            let metrics = common::score(&ranked, item, CUTOFF);
            Ok::<ProductionQuery, anyhow::Error>(ProductionQuery {
                metrics,
                names: ranked,
                elapsed: started.elapsed(),
            })
            .with_context(|| format!("running production baseline for golden query {}", index + 1))
        })
        .collect()
}

/// Average metric rows after excluding behavior probes and an optional split.
pub(super) fn metric_mean<'a>(
    scores: impl Iterator<Item = (Metrics, &'a GoldenQuery)>,
    split: Option<&str>,
) -> Metrics {
    let scores: Vec<Metrics> = scores
        .filter(|(_, item)| item.split() != "behavior")
        .filter(|(_, item)| split.is_none_or(|required| item.split() == required))
        .map(|(score, _)| score)
        .collect();
    if scores.is_empty() {
        return Metrics::default();
    }
    let count = scores.len() as f64;
    Metrics {
        recall: scores.iter().map(|score| score.recall).sum::<f64>() / count,
        mrr: scores.iter().map(|score| score.mrr).sum::<f64>() / count,
        ndcg: scores.iter().map(|score| score.ndcg).sum::<f64>() / count,
    }
}

/// Aggregate one candidate's scores over groups and benchmark splits.
fn candidate_metrics(comparisons: &[QueryComparison], golden: &[GoldenQuery]) -> CandidateMetrics {
    let all = |select: fn(&QueryComparison) -> Metrics| {
        metric_mean(
            comparisons
                .iter()
                .zip(golden)
                .map(|(comparison, item)| (select(comparison), item)),
            None,
        )
    };
    let mut groups: BTreeMap<String, BTreeMap<String, Vec<Metrics>>> = BTreeMap::new();
    for (comparison, item) in comparisons.iter().zip(golden) {
        if item.split() == "behavior" {
            continue;
        }
        let group = groups.entry(item.group()).or_default();
        group
            .entry("fts".to_string())
            .or_default()
            .push(comparison.fts_metrics);
        group
            .entry("vector".to_string())
            .or_default()
            .push(comparison.vector_metrics);
        group
            .entry("hybrid".to_string())
            .or_default()
            .push(comparison.hybrid_metrics);
    }
    let by_group = groups
        .into_iter()
        .map(|(group, metrics)| {
            let values = metrics
                .into_iter()
                .map(|(leg, scores)| (leg, mean_values(&scores)))
                .collect();
            (group, values)
        })
        .collect();
    CandidateMetrics {
        fts: all(|comparison| comparison.fts_metrics),
        vector: all(|comparison| comparison.vector_metrics),
        hybrid: all(|comparison| comparison.hybrid_metrics),
        by_group,
        development_hybrid: metric_mean(
            comparisons
                .iter()
                .zip(golden)
                .map(|(comparison, item)| (comparison.hybrid_metrics, item)),
            Some("development"),
        ),
        held_out_hybrid: metric_mean(
            comparisons
                .iter()
                .zip(golden)
                .map(|(comparison, item)| (comparison.hybrid_metrics, item)),
            Some("held_out"),
        ),
    }
}

/// Average metric triples for one non-empty query group.
fn mean_values(scores: &[Metrics]) -> Metrics {
    let count = scores.len() as f64;
    Metrics {
        recall: scores.iter().map(|score| score.recall).sum::<f64>() / count,
        mrr: scores.iter().map(|score| score.mrr).sum::<f64>() / count,
        ndcg: scores.iter().map(|score| score.ndcg).sum::<f64>() / count,
    }
}

/// Summarize end-to-end production latency samples.
pub(super) fn duration_summary(samples: &[std::time::Duration]) -> anyhow::Result<DurationSummary> {
    anyhow::ensure!(!samples.is_empty(), "no production latency samples");
    let mut values: Vec<u64> = samples
        .iter()
        .map(|sample| sample.as_micros() as u64)
        .collect();
    values.sort_unstable();
    let pick = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    Ok(DurationSummary {
        samples: values.len(),
        p50_us: pick(0.50),
        p95_us: pick(0.95),
        p99_us: pick(0.99),
        mean_us: values.iter().sum::<u64>() / values.len() as u64,
    })
}

/// Run a candidate's query vectors through shared retrieval and fusion.
fn score_candidate(
    candidate: &Candidate,
    matrix: &FlatMatrix,
    model: &mut LoadedCandidateModel,
    context: &EvaluationContext<'_>,
) -> anyhow::Result<Vec<QueryComparison>> {
    context
        .golden
        .iter()
        .zip(context.production)
        .enumerate()
        .map(|(index, (item, reference))| {
            let filters = item.search_filters()?;
            let started = Instant::now();
            let query_vector = embed_query(candidate, model, &item.query)?;
            let embedding_time = started.elapsed();
            let started = Instant::now();
            let results = query::search_with_settings(
                context.conn,
                context.cards,
                matrix,
                &query_vector,
                &item.query,
                &filters,
                CUTOFF,
                None,
                &context.settings,
            )?;
            let search_time = started.elapsed();
            let fts = names(&results.fts);
            let vector = names(&results.vector);
            let hybrid = names(&results.hybrid);
            if candidate.key == "bge-small-full"
                && context.document_format == DocumentFormat::Production
                && context.fts_tokenizer == FtsTokenizer::Porter
                && context.settings == query::DEFAULT_SEARCH_SETTINGS
            {
                validate_bge_parity(
                    candidate.key,
                    index + 1,
                    &item.query,
                    &hybrid,
                    &reference.names,
                )?;
            }
            let fts_metrics = common::score(&fts, item, CUTOFF);
            let vector_metrics = common::score(&vector, item, CUTOFF);
            let hybrid_metrics = common::score(&hybrid, item, CUTOFF);
            let review_pool =
                common::pool_names([fts.as_slice(), vector.as_slice(), hybrid.as_slice()]);
            let judged: HashSet<&str> = item
                .relevant
                .iter()
                .chain(&item.not_relevant)
                .map(String::as_str)
                .collect();
            let unjudged_pool: Vec<String> = review_pool
                .iter()
                .filter(|name| !judged.contains(name.as_str()))
                .cloned()
                .collect();
            let coverage = if review_pool.is_empty() {
                1.0
            } else {
                (review_pool.len() - unjudged_pool.len()) as f64 / review_pool.len() as f64
            };
            Ok(QueryComparison {
                query_index: index + 1,
                query: item.query.clone(),
                group: item.group(),
                split: item.split(),
                cli_baseline: reference.names.clone(),
                fts,
                vector,
                hybrid,
                fts_metrics,
                vector_metrics,
                hybrid_metrics,
                cli_baseline_ndcg: common::round3(reference.metrics.ndcg),
                hybrid_ndcg_delta: common::round3(hybrid_metrics.ndcg - reference.metrics.ndcg),
                review_pool,
                unjudged_pool,
                judgment_coverage: common::round3(coverage),
                query_embed_us: embedding_time.as_micros() as u64,
                shared_search_us: search_time.as_micros() as u64,
            })
        })
        .collect()
}

/// Add candidate metrics, paired regressions, and parity results.
pub(super) fn build_candidate_report(
    candidate: &Candidate,
    matrix: &FlatMatrix,
    model: &mut LoadedCandidateModel,
    build_secs: Option<f64>,
    metadata: CacheMetadata,
    context: &EvaluationContext<'_>,
    backend: BackendSelection,
) -> anyhow::Result<CandidateReport> {
    let comparisons = score_candidate(candidate, matrix, model, context)?;
    let baseline_ndcg = metric_mean(
        context
            .production
            .iter()
            .zip(context.golden)
            .map(|(reference, item)| (reference.metrics, item)),
        None,
    )
    .ndcg;
    let metrics = candidate_metrics(&comparisons, context.golden);
    let delta = common::round3(metrics.hybrid.ndcg - baseline_ndcg);
    let regressions = comparisons
        .iter()
        .filter(|comparison| {
            comparison.split != "behavior"
                && comparison.cli_baseline_ndcg - comparison.hybrid_metrics.ndcg
                    > REGRESSION_THRESHOLD
        })
        .map(|comparison| Regression {
            query_index: comparison.query_index,
            query: comparison.query.clone(),
            group: comparison.group.clone(),
            baseline_ndcg: comparison.cli_baseline_ndcg,
            candidate_ndcg: common::round3(comparison.hybrid_metrics.ndcg),
        })
        .collect();
    let required = candidate.key == "bge-small-full"
        && context.document_format == DocumentFormat::Production
        && context.fts_tokenizer == FtsTokenizer::Porter
        && context.settings == query::DEFAULT_SEARCH_SETTINGS;
    Ok(CandidateReport {
        key: candidate.key.to_string(),
        model: model_name(candidate),
        model_fingerprint: metadata.model_fingerprint.clone(),
        dimension: candidate.dim,
        max_length: candidate.max_length,
        model_max_length: candidate.model_max_length,
        document_prompt: candidate.document_prompt.to_string(),
        query_prompt: candidate.query_prompt.to_string(),
        pooling: candidate.pooling.to_string(),
        model_card: candidate.model_card.to_string(),
        requested_backend: backend.requested.key().to_string(),
        effective_backend: backend.effective.key().to_string(),
        query_backend: "cpu",
        batch_size: model.batch_size(),
        intra_threads: embed::INTRA_THREADS,
        document_format: context.document_format.key().to_string(),
        cached: build_secs.is_none(),
        document_embedding_secs: build_secs,
        cache_metadata: metadata,
        metrics,
        hybrid_ndcg_delta_vs_cli: delta,
        regressions,
        parity: ParityInfo {
            required,
            passed: !required || comparisons.len() == context.golden.len(),
            compared_queries: if required { comparisons.len() } else { 0 },
            cutoff: CUTOFF,
        },
        queries: comparisons,
    })
}

/// Print a compact model comparison and parity summary.
pub(super) fn print_human(report: &BakeoffReport) {
    println!(
        "Embedding bake-off · {} queries · {} cards · labels {} · current CLI nDCG {:.3}",
        report.corpus.golden_queries,
        report.corpus.cards,
        report.label_version,
        report.production_reference.metrics.ndcg
    );
    println!(
        "Filtered queries are held out for final checks. Relevance averages exclude behavior probes."
    );
    println!(
        "Settings: FTS tokenizer {}, weights {:?}, depths {}/{}, RRF k {} weights {}/{}, terms {}, over-fetch ×{} (min {}, rounds {}).",
        report.ranking_settings.fts_tokenizer,
        report.ranking_settings.fts_column_weights,
        report.ranking_settings.fts_candidate_depth,
        report.ranking_settings.vector_candidate_depth,
        report.ranking_settings.rrf_k,
        report.ranking_settings.fts_rrf_weight,
        report.ranking_settings.vector_rrf_weight,
        report.ranking_settings.fts_term_operator,
        report.ranking_settings.fts_overfetch_multiplier,
        report.ranking_settings.fts_min_sql_rows,
        report.ranking_settings.fts_max_rounds
    );
    if let (Some(snapshot), Some(rebuild)) = (
        report.fts_index.database_snapshot_us,
        report.fts_index.index_rebuild_us,
    ) {
        println!(
            "Temporary FTS database copy {}µs; index rebuild {}µs.",
            snapshot, rebuild
        );
    }
    println!(
        "\n{:<24} {:<12} {:>5} {:>7} {:>8}  {:>17}  {:>17}  {:>8} {:>10}",
        "candidate",
        "format",
        "dim",
        "tokens",
        "embed",
        "vector (R/M/N)",
        "hybrid (R/M/N)",
        "Δ nDCG",
        "regressions"
    );
    for candidate in &report.candidates {
        let embed_time = candidate
            .document_embedding_secs
            .map(|seconds| format!("{seconds:.0}s"))
            .unwrap_or_else(|| "cached".to_string());
        println!(
            "{:<24} {:<12} {:>5} {:>7} {:>8}  {:>17}  {:>17}  {:+8.3} {:>10}",
            candidate.key,
            candidate.document_format,
            candidate.dimension,
            candidate.max_length,
            embed_time,
            metric_string(candidate.metrics.vector),
            metric_string(candidate.metrics.hybrid),
            candidate.hybrid_ndcg_delta_vs_cli,
            candidate.regressions.len()
        );
    }
    if let Some(bge) = report
        .candidates
        .iter()
        .find(|candidate| candidate.parity.required)
    {
        println!(
            "\nProduction full-precision BGE parity: {}/{} queries passed.",
            bge.parity.compared_queries, report.corpus.golden_queries
        );
    }
    let unjudged = report
        .candidates
        .iter()
        .flat_map(|candidate| candidate.queries.iter())
        .map(|query| query.unjudged_pool.len())
        .sum::<usize>();
    println!(
        "Unjudged pooled hits across reported model-format pairs: {unjudged}; review before treating labels as complete."
    );
}

/// Format recall, MRR, and nDCG for the human summary table.
fn metric_string(metrics: Metrics) -> String {
    format!(
        "{:.3}/{:.3}/{:.3}",
        metrics.recall, metrics.mrr, metrics.ndcg
    )
}
