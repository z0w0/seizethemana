// Compare the production search path with its FTS and vector diagnostics.
//
//   cargo build --release && ./target/release/stm setup
//   cargo run --release --example query_baseline -- --json

/// Shared golden-query data and metric helpers.
mod common;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use seizethemana::paths::{Paths, Status};
use seizethemana::{db, embed, output::Output, query, tags};
use serde::Serialize;

use common::{GoldenQuery, Metrics};

const CUTOFF: usize = 10;

/// Machine-readable report for one CLI-faithful baseline run.
#[derive(Debug, Serialize)]
struct BaselineReport {
    generated_at: String,
    benchmark: BenchmarkInfo,
    store: StoreInfo,
    ranking_settings: SettingsInfo,
    parity: ParityInfo,
    overall: BTreeMap<String, Metrics>,
    by_group: BTreeMap<String, BTreeMap<String, Metrics>>,
    latency: LatencyInfo,
    behavior_checks: Vec<BehaviorCheck>,
    queries: Vec<QueryReport>,
}

/// Benchmark version, query split, and corpus identity.
#[derive(Debug, Serialize)]
struct BenchmarkInfo {
    cli_faithful: bool,
    label_version: &'static str,
    query_count: usize,
    cutoff: usize,
    development_queries: usize,
    held_out_queries: usize,
    behavior_queries: usize,
    corpus_fingerprint: String,
    query_fingerprint: String,
}

/// Store and model versions needed to reproduce the rankings.
#[derive(Debug, Serialize)]
struct StoreInfo {
    crate_version: &'static str,
    database_schema_version: i64,
    cards: usize,
    vector_rows: usize,
    model: String,
    dimension: usize,
    document_version: u32,
}

/// Ordered top-k parity result against `query::run_search`.
#[derive(Debug, Serialize)]
struct ParityInfo {
    passed: bool,
    checked_queries: usize,
    checked_top_k: usize,
}

/// Ranking parameters used by the shared library path.
#[derive(Debug, Serialize)]
struct SettingsInfo {
    fts_column_weights: [f64; 4],
    fts_candidate_depth: usize,
    vector_candidate_depth: usize,
    rrf_k: f64,
}

/// Latency samples and their measured path.
#[derive(Debug, Serialize)]
struct LatencyInfo {
    end_to_end_path: &'static str,
    end_to_end_samples: DurationStats,
    prepared_query_embed_us: DurationStats,
    prepared_shared_search_us: DurationStats,
}

/// Percentile summary for one set of elapsed-time samples.
#[derive(Debug, Serialize)]
struct DurationStats {
    samples: usize,
    p50_us: u64,
    p95_us: u64,
    p99_us: u64,
    mean_us: u64,
}

/// Output summary for a query that tests behavior rather than relevance.
#[derive(Debug, Serialize)]
struct BehaviorCheck {
    query_index: usize,
    query: String,
    note: String,
    fts_count: usize,
    vector_count: usize,
    hybrid_count: usize,
    hybrid_top: Vec<String>,
}

/// Rankings, scores, labels, and pooled review candidates for one query.
#[derive(Debug, Serialize)]
struct QueryReport {
    query_index: usize,
    query: String,
    note: String,
    group: String,
    split: &'static str,
    filters: Option<common::GoldenFilters>,
    relevant: Vec<String>,
    not_relevant: Vec<String>,
    metrics: BTreeMap<String, Metrics>,
    rankings: BTreeMap<String, Vec<String>>,
    review_pool: Vec<String>,
    unjudged_pool: Vec<String>,
    judgment_coverage: f64,
    latency_us: QueryLatency,
}

/// Timings collected for one query across production and diagnostic paths.
#[derive(Debug, Serialize)]
struct QueryLatency {
    run_search: u64,
    query_embedding: u64,
    shared_search: u64,
}

/// Resolve the store directory from the environment or the default home path.
fn data_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("STM_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".seizethemana"))
}

/// Summarize latency samples without mixing them with quality scores.
fn duration_stats(samples: &[Duration]) -> anyhow::Result<DurationStats> {
    anyhow::ensure!(!samples.is_empty(), "no latency samples to summarize");
    let mut micros: Vec<u64> = samples
        .iter()
        .map(|sample| sample.as_micros() as u64)
        .collect();
    micros.sort_unstable();
    let percentile = |fraction: f64| {
        let index = ((micros.len() - 1) as f64 * fraction).round() as usize;
        micros[index]
    };
    Ok(DurationStats {
        samples: micros.len(),
        p50_us: percentile(0.50),
        p95_us: percentile(0.95),
        p99_us: percentile(0.99),
        mean_us: micros.iter().sum::<u64>() / micros.len() as u64,
    })
}

/// Take the ordered top-k names from one shared-search leg.
fn hit_names(hits: &[query::Hit]) -> Vec<String> {
    hits.iter()
        .take(CUTOFF)
        .map(|hit| hit.card.name.clone())
        .collect()
}

/// Average each retrieval leg within its benchmark query group.
fn summarize_group(
    queries: &[QueryReport],
    source: &[GoldenQuery],
) -> BTreeMap<String, BTreeMap<String, Metrics>> {
    let mut grouped = BTreeMap::<String, BTreeMap<String, Vec<Metrics>>>::new();
    for (report, golden) in queries.iter().zip(source) {
        if golden.split() == "behavior" {
            continue;
        }
        let legs = grouped.entry(report.group.clone()).or_default();
        for (leg, score) in &report.metrics {
            legs.entry(leg.clone()).or_default().push(*score);
        }
    }
    grouped
        .into_iter()
        .map(|(group, legs)| {
            let means = legs
                .into_iter()
                .map(|(leg, scores)| (leg, mean_metrics(&scores)))
                .collect();
            (group, means)
        })
        .collect()
}

/// Compute the arithmetic mean of a non-empty metric list.
fn mean_metrics(scores: &[Metrics]) -> Metrics {
    let count = scores.len() as f64;
    Metrics {
        recall: scores.iter().map(|score| score.recall).sum::<f64>() / count,
        mrr: scores.iter().map(|score| score.mrr).sum::<f64>() / count,
        ndcg: scores.iter().map(|score| score.ndcg).sum::<f64>() / count,
    }
}

/// Parse the output options supported by this example.
fn parse_args() -> anyhow::Result<bool> {
    let mut json = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => json = true,
            _ => anyhow::bail!("unknown argument {arg:?}"),
        }
    }
    Ok(json)
}

/// Run production parity and collect quality, pooling, and latency reports.
fn main() -> anyhow::Result<()> {
    let json = parse_args()?;
    let root = data_dir()?;
    let paths = Paths::new(root.clone());
    anyhow::ensure!(
        paths.is_setup(),
        "no completed store at {} (run 'stm setup' in release mode first)",
        root.display()
    );
    let conn = db::open(&paths.db())?;
    let cards = db::load_all_cards(&conn)?;
    let store = embed::VectorStore::load(paths.root())?;
    let tag_index = tags::TagIndex::load(&conn)?;
    let docs: Vec<String> = cards
        .iter()
        .map(|card| embed::doc_for_row(card, &tag_index))
        .collect();
    let corpus_fingerprint = common::fingerprint_strings(
        cards
            .iter()
            .zip(&docs)
            .flat_map(|(card, doc)| [card.name.as_str(), doc.as_str()]),
    );
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let golden = common::load_queries(&manifest.join("benches/golden_queries.json"))?;
    common::validate_labels(&golden, &cards)?;
    let status = Status::read(&paths.status_file())?;
    anyhow::ensure!(
        store.meta.names.is_empty()
            || (store.meta.names.len() == cards.len()
                && store
                    .meta
                    .names
                    .iter()
                    .zip(&cards)
                    .all(|(name, card)| name == &card.name)),
        "production vector rows do not match cards in database order"
    );
    let mut model = if store.is_empty() {
        None
    } else {
        Some(embed::load_query_model(&paths.models_dir(), false)?)
    };
    let output = Output::new(false, true, false);
    let settings = query::DEFAULT_SEARCH_SETTINGS;
    let mut end_to_end_samples = Vec::with_capacity(golden.len());
    let mut embedding_samples = Vec::with_capacity(golden.len());
    let mut search_samples = Vec::with_capacity(golden.len());
    let mut reports = Vec::with_capacity(golden.len());

    for (index, item) in golden.iter().enumerate() {
        let filters = item.search_filters()?;
        let prepared = query::prepare_query(&item.query);
        let started = Instant::now();
        let query_vector = match model.as_mut() {
            Some(model) => store.embed_query(model, &prepared.expanded_text)?,
            None => Vec::new(),
        };
        let embedding_time = started.elapsed();

        let started = Instant::now();
        let cli_hits =
            query::run_search(&paths, &conn, &output, &item.query, &filters, CUTOFF, None)?;
        let end_to_end_time = started.elapsed();

        let started = Instant::now();
        let result = query::search_with_settings(
            &conn,
            &cards,
            &store,
            &query_vector,
            &item.query,
            &filters,
            CUTOFF,
            None,
            &settings,
        )?;
        let shared_search_time = started.elapsed();
        let cli_names = hit_names(&cli_hits);
        let diagnostic_names = hit_names(&result.hybrid);
        anyhow::ensure!(
            cli_names == diagnostic_names,
            "query {} ({:?}) failed ordered top-{CUTOFF} parity\nrun_search: {cli_names:?}\nshared: {diagnostic_names:?}",
            index + 1,
            item.query
        );
        end_to_end_samples.push(end_to_end_time);
        embedding_samples.push(embedding_time);
        search_samples.push(shared_search_time);

        let rankings = BTreeMap::from([
            ("fts".to_string(), hit_names(&result.fts)),
            ("vector".to_string(), hit_names(&result.vector)),
            ("hybrid".to_string(), diagnostic_names),
        ]);
        let metrics: BTreeMap<String, Metrics> = rankings
            .iter()
            .map(|(leg, ranked)| (leg.clone(), common::score(ranked, item, CUTOFF)))
            .collect();
        let review_pool = common::pool_names(rankings.values().map(Vec::as_slice));
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
        reports.push(QueryReport {
            query_index: index + 1,
            query: item.query.clone(),
            note: item.note.clone(),
            group: item.group(),
            split: item.split(),
            filters: item.filters.clone(),
            relevant: item.relevant.clone(),
            not_relevant: item.not_relevant.clone(),
            metrics,
            rankings,
            review_pool,
            unjudged_pool,
            judgment_coverage: common::round3(coverage),
            latency_us: QueryLatency {
                run_search: end_to_end_time.as_micros() as u64,
                query_embedding: embedding_time.as_micros() as u64,
                shared_search: shared_search_time.as_micros() as u64,
            },
        });
    }

    let metric_rows: BTreeMap<String, Metrics> = ["fts", "vector", "hybrid"]
        .into_iter()
        .map(|leg| {
            let scores: Vec<Metrics> = reports
                .iter()
                .zip(&golden)
                .filter(|(_, item)| item.split() != "behavior")
                .filter_map(|(report, _)| report.metrics.get(leg).copied())
                .collect();
            (leg.to_string(), mean_metrics(&scores))
        })
        .collect();
    let behavior_checks = reports
        .iter()
        .filter(|report| report.split == "behavior")
        .map(|report| BehaviorCheck {
            query_index: report.query_index,
            query: report.query.clone(),
            note: report.note.clone(),
            fts_count: report.rankings["fts"].len(),
            vector_count: report.rankings["vector"].len(),
            hybrid_count: report.rankings["hybrid"].len(),
            hybrid_top: report.rankings["hybrid"].iter().take(3).cloned().collect(),
        })
        .collect();
    let mut stmt = conn.prepare("PRAGMA user_version")?;
    let schema_version: i64 = stmt.query_row([], |row| row.get(0))?;
    let development_queries = golden
        .iter()
        .filter(|item| item.split() == "development")
        .count();
    let held_out_queries = golden
        .iter()
        .filter(|item| item.split() == "held_out")
        .count();
    let behavior_queries = golden
        .iter()
        .filter(|item| item.split() == "behavior")
        .count();
    let report = BaselineReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        benchmark: BenchmarkInfo {
            cli_faithful: true,
            label_version: common::LABEL_VERSION,
            query_count: golden.len(),
            cutoff: CUTOFF,
            development_queries,
            held_out_queries,
            behavior_queries,
            corpus_fingerprint,
            query_fingerprint: common::fingerprint_queries(&golden)?,
        },
        store: StoreInfo {
            crate_version: env!("CARGO_PKG_VERSION"),
            database_schema_version: schema_version,
            cards: cards.len(),
            vector_rows: store.len(),
            model: status.model,
            dimension: status.dim,
            document_version: status.doc_version,
        },
        ranking_settings: SettingsInfo {
            fts_column_weights: settings.fts_column_weights,
            fts_candidate_depth: settings.fts_candidate_depth.max(CUTOFF),
            vector_candidate_depth: settings.vector_candidate_depth.max(CUTOFF),
            rrf_k: settings.rrf_k,
        },
        parity: ParityInfo {
            passed: true,
            checked_queries: reports.len(),
            checked_top_k: CUTOFF,
        },
        overall: metric_rows,
        by_group: summarize_group(&reports, &golden),
        latency: LatencyInfo {
            end_to_end_path: "query::run_search; includes store reads, model initialization, query embedding, and shared ranking; excludes CLI parsing and rendering",
            end_to_end_samples: duration_stats(&end_to_end_samples)?,
            prepared_query_embed_us: duration_stats(&embedding_samples)?,
            prepared_shared_search_us: duration_stats(&search_samples)?,
        },
        behavior_checks,
        queries: reports,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report);
    }
    Ok(())
}

/// Print the concise human-readable baseline summary.
fn print_human(report: &BaselineReport) {
    println!(
        "CLI-faithful baseline · {} queries · {} cards · model {} · labels {}",
        report.benchmark.query_count,
        report.store.cards,
        report.store.model,
        report.benchmark.label_version
    );
    println!("Ordered top-{CUTOFF} parity passed for all queries.");
    println!(
        "\n{: <15} {:>8} {:>8} {:>8}",
        "group / leg", "recall", "mrr", "ndcg"
    );
    for leg in ["fts", "vector", "hybrid"] {
        if let Some(score) = report.overall.get(leg) {
            println!(
                "{:<15} {:>8.3} {:>8.3} {:>8.3}",
                format!("overall / {leg}"),
                score.recall,
                score.mrr,
                score.ndcg
            );
        }
    }
    for (group, legs) in &report.by_group {
        for (leg, score) in legs {
            println!(
                "{group: <15} {leg: <8} {:>8.3} {:>8.3} {:>8.3}",
                score.recall, score.mrr, score.ndcg
            );
        }
    }
    println!(
        "\nEnd-to-end run_search: p50 {}µs, p95 {}µs.",
        report.latency.end_to_end_samples.p50_us, report.latency.end_to_end_samples.p95_us
    );
    let unjudged = report
        .queries
        .iter()
        .map(|query| query.unjudged_pool.len())
        .sum::<usize>();
    println!(
        "Unjudged pooled hits: {unjudged}; inspect the JSON report before treating labels as complete."
    );
}
