// Compare one hybrid search setting at a time on development queries, then
// check the selected setting once on held-out filtered queries.
//
//   cargo run --release --example query_rank_sweep -- --json

/// Shared golden-query data and metric helpers.
mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use seizethemana::paths::{Paths, Status};
use seizethemana::{db, embed, output::Output, query, tags};
use serde::Serialize;

use common::{GoldenQuery, Metrics};

const CUTOFF: usize = 10;
const CANDIDATE_CUTOFF: usize = 20;

/// A single parameter change from production defaults.
#[derive(Debug, Clone)]
struct Variant {
    name: String,
    changed_parameter: String,
    value: String,
    settings: query::SearchSettings,
}

/// Input and production-default results for one query index.
#[derive(Debug, Clone)]
struct QueryInput {
    index: usize,
    golden: GoldenQuery,
    filters: seizethemana::search::CardFilters,
    query_vector: Vec<f32>,
    production_names: Vec<String>,
    default: Observation,
}

/// Ranked legs and metrics for one query under one setting.
#[derive(Debug, Clone, Serialize)]
struct Observation {
    fts: Vec<String>,
    vector: Vec<String>,
    hybrid: Vec<String>,
    fts_metrics: Metrics,
    vector_metrics: Metrics,
    hybrid_metrics: Metrics,
    fts_candidate_recall20: f64,
    vector_candidate_recall20: f64,
    union_candidate_recall20: f64,
    search_us: u64,
}

/// Mean quality metrics for one leg or candidate pool.
#[derive(Debug, Clone, Copy, Default, Serialize)]
struct LegSummary {
    recall: f64,
    mrr: f64,
    ndcg: f64,
    candidate_recall20: f64,
}

/// Aggregate quality, by leg and query group.
#[derive(Debug, Clone, Serialize)]
struct Summary {
    query_count: usize,
    fts: LegSummary,
    vector: LegSummary,
    hybrid: LegSummary,
    union_candidate_recall20: f64,
    by_group: BTreeMap<String, BTreeMap<String, LegSummary>>,
}

/// One development setting result with paired ranks and scores.
#[derive(Debug, Serialize)]
struct VariantReport {
    name: String,
    changed_parameter: String,
    value: String,
    settings: SettingsInfo,
    summary: Summary,
    mean_shared_search_us: u64,
    paired_queries: Vec<PairedQuery>,
}

/// Per-query comparison to current production defaults.
#[derive(Debug, Serialize)]
struct PairedQuery {
    query_index: usize,
    query: String,
    group: String,
    split: &'static str,
    default_hybrid: Vec<String>,
    candidate_hybrid: Vec<String>,
    default_ndcg: f64,
    candidate_ndcg: f64,
    ndcg_delta: f64,
    default_candidate_recall20: f64,
    candidate_candidate_recall20: f64,
    fts_top20: Vec<String>,
    vector_top20: Vec<String>,
}

/// Settings serialized with every comparison.
#[derive(Debug, Serialize)]
struct SettingsInfo {
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

/// Final comparison between production defaults and the selected setting.
#[derive(Debug, Serialize)]
struct HeldOutCheck {
    selected_variant: String,
    default: Summary,
    selected: Summary,
    paired_queries: Vec<PairedQuery>,
}

/// Reproducible report for one search-setting sweep.
#[derive(Debug, Serialize)]
struct SweepReport {
    generated_at: String,
    evaluation_version: &'static str,
    label_version: &'static str,
    corpus: CorpusInfo,
    ranking_defaults: SettingsInfo,
    split_rule: &'static str,
    production_end_to_end: DurationSummary,
    query_embedding: DurationSummary,
    setup_cost: SetupCost,
    controls: Summary,
    development_variants: Vec<VariantReport>,
    selection: Selection,
    held_out_check: HeldOutCheck,
}

/// Corpus and store information for this run.
#[derive(Debug, Serialize)]
struct CorpusInfo {
    cards: usize,
    vectors: usize,
    model: String,
    dimension: usize,
    document_version: u32,
    database_schema_version: i64,
    query_count: usize,
    development_queries: usize,
    held_out_queries: usize,
    behavior_queries: usize,
    fingerprint: String,
    query_fingerprint: String,
}

/// The rule and development score used to select one candidate.
#[derive(Debug, Serialize)]
struct Selection {
    variant: String,
    rule: &'static str,
    development_hybrid_ndcg: f64,
    default_development_hybrid_ndcg: f64,
}

/// Query and index cost observations kept separate from ranking scores.
#[derive(Debug, Serialize)]
struct SetupCost {
    database_bytes: u64,
    vector_bytes: u64,
    database_snapshot_us: u64,
    fts_index_rebuild_us: u64,
    fts_rebuild_scope: &'static str,
    embedding_cost_reference: &'static str,
}

/// Snapshot and FTS rebuild timings measured on a temporary database copy.
#[derive(Debug)]
struct FtsBuildCost {
    database_snapshot_us: u64,
    fts_index_rebuild_us: u64,
}

/// Latency distribution across one sample per benchmark query.
#[derive(Debug, Serialize)]
struct DurationSummary {
    samples: usize,
    p50_us: u64,
    p95_us: u64,
    p99_us: u64,
    mean_us: u64,
}

/// Resolve the store directory from the environment or the default home path.
fn data_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("STM_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".seizethemana"))
}

/// Convert library settings into the report's stable JSON shape.
fn settings_info(settings: query::SearchSettings) -> SettingsInfo {
    SettingsInfo {
        fts_tokenizer: "porter",
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

/// Build one-at-a-time changes around the current production settings.
fn variants() -> Vec<Variant> {
    let default = query::DEFAULT_SEARCH_SETTINGS;
    let mut variants = vec![Variant {
        name: "current-default".to_string(),
        changed_parameter: "none".to_string(),
        value: "production defaults".to_string(),
        settings: default,
    }];
    for (column, label, values) in [
        (0, "name", [4.0, 12.0]),
        (1, "tags", [2.0, 6.0]),
        (2, "type_line", [1.0, 3.0]),
        (3, "oracle_text", [0.5, 2.0]),
    ] {
        for weight in values {
            let mut settings = default;
            settings.fts_column_weights[column] = weight;
            variants.push(Variant {
                name: format!("fts-{label}-{weight}"),
                changed_parameter: format!("fts_weight_{label}"),
                value: weight.to_string(),
                settings,
            });
        }
    }
    for depth in [50, 100] {
        variants.push(Variant {
            name: format!("fts-depth-{depth}"),
            changed_parameter: "fts_candidate_depth".to_string(),
            value: depth.to_string(),
            settings: query::SearchSettings {
                fts_candidate_depth: depth,
                ..default
            },
        });
        variants.push(Variant {
            name: format!("vector-depth-{depth}"),
            changed_parameter: "vector_candidate_depth".to_string(),
            value: depth.to_string(),
            settings: query::SearchSettings {
                vector_candidate_depth: depth,
                ..default
            },
        });
    }
    for k in [30.0, 100.0] {
        variants.push(Variant {
            name: format!("rrf-k-{k}"),
            changed_parameter: "rrf_k".to_string(),
            value: k.to_string(),
            settings: query::SearchSettings {
                rrf_k: k,
                ..default
            },
        });
    }
    variants.push(Variant {
        name: "fts-terms-all".to_string(),
        changed_parameter: "fts_term_operator".to_string(),
        value: "all".to_string(),
        settings: query::SearchSettings {
            fts_term_operator: db::FtsTermOperator::All,
            ..default
        },
    });
    for weight in [0.5, 2.0] {
        variants.push(Variant {
            name: format!("rrf-fts-weight-{weight}"),
            changed_parameter: "fts_rrf_weight".to_string(),
            value: weight.to_string(),
            settings: query::SearchSettings {
                fts_rrf_weight: weight,
                ..default
            },
        });
        variants.push(Variant {
            name: format!("rrf-vector-weight-{weight}"),
            changed_parameter: "vector_rrf_weight".to_string(),
            value: weight.to_string(),
            settings: query::SearchSettings {
                vector_rrf_weight: weight,
                ..default
            },
        });
    }
    for multiplier in [2, 8] {
        variants.push(Variant {
            name: format!("fts-overfetch-{multiplier}"),
            changed_parameter: "fts_overfetch_multiplier".to_string(),
            value: multiplier.to_string(),
            settings: query::SearchSettings {
                fts_overfetch_multiplier: multiplier,
                ..default
            },
        });
    }
    for rounds in [2, 6] {
        variants.push(Variant {
            name: format!("fts-rounds-{rounds}"),
            changed_parameter: "fts_max_rounds".to_string(),
            value: rounds.to_string(),
            settings: query::SearchSettings {
                fts_max_rounds: rounds,
                ..default
            },
        });
    }
    for rows in [25, 100] {
        variants.push(Variant {
            name: format!("fts-min-rows-{rows}"),
            changed_parameter: "fts_min_sql_rows".to_string(),
            value: rows.to_string(),
            settings: query::SearchSettings {
                fts_min_sql_rows: rows,
                ..default
            },
        });
    }
    variants
}

/// Copy the ordered leading names from a retrieval leg.
fn hit_names(hits: &[query::Hit], limit: usize) -> Vec<String> {
    hits.iter()
        .take(limit)
        .map(|hit| hit.card.name.clone())
        .collect()
}

/// Measure the fraction of labeled positives in one top-20 candidate list.
fn candidate_recall20(ranked: &[String], item: &GoldenQuery) -> f64 {
    if item.relevant.is_empty() {
        return 0.0;
    }
    let hits = ranked
        .iter()
        .take(CANDIDATE_CUTOFF)
        .filter(|name| item.relevant.contains(name))
        .count();
    hits as f64 / item.relevant.len() as f64
}

/// Run all shared retrieval legs once with one setting and one query vector.
fn observe(
    conn: &rusqlite::Connection,
    cards: &[db::CardRow],
    store: &embed::VectorStore,
    input: &QueryInput,
    settings: query::SearchSettings,
) -> anyhow::Result<Observation> {
    let started = Instant::now();
    let results = query::search_with_settings(
        conn,
        cards,
        store,
        &input.query_vector,
        &input.golden.query,
        &input.filters,
        CUTOFF,
        None,
        &settings,
    )?;
    let elapsed = started.elapsed().as_micros() as u64;
    let fts = hit_names(&results.fts, CANDIDATE_CUTOFF);
    let vector = hit_names(&results.vector, CANDIDATE_CUTOFF);
    let hybrid = hit_names(&results.hybrid, CUTOFF);
    let pool = common::pool_names([fts.as_slice(), vector.as_slice()]);
    Ok(Observation {
        fts_metrics: common::score(&fts, &input.golden, CUTOFF),
        vector_metrics: common::score(&vector, &input.golden, CUTOFF),
        hybrid_metrics: common::score(&hybrid, &input.golden, CUTOFF),
        fts_candidate_recall20: candidate_recall20(&fts, &input.golden),
        vector_candidate_recall20: candidate_recall20(&vector, &input.golden),
        union_candidate_recall20: candidate_recall20(&pool, &input.golden),
        fts,
        vector,
        hybrid,
        search_us: elapsed,
    })
}

/// Summarize elapsed-time samples by percentile and mean.
fn duration_summary(samples: &[Duration]) -> anyhow::Result<DurationSummary> {
    anyhow::ensure!(!samples.is_empty(), "no latency samples to summarize");
    let mut values: Vec<u64> = samples
        .iter()
        .map(|sample| sample.as_micros() as u64)
        .collect();
    values.sort_unstable();
    let percentile = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    Ok(DurationSummary {
        samples: values.len(),
        p50_us: percentile(0.50),
        p95_us: percentile(0.95),
        p99_us: percentile(0.99),
        mean_us: values.iter().sum::<u64>() / values.len() as u64,
    })
}

/// Measure FTS5 rebuild time without modifying the user's database.
fn measure_fts_rebuild(conn: &rusqlite::Connection) -> anyhow::Result<FtsBuildCost> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("fts-rebuild.db");
    let snapshot_start = Instant::now();
    conn.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
    let database_snapshot_us = snapshot_start.elapsed().as_micros() as u64;

    let copy = db::open(&path)?;
    let rebuild_start = Instant::now();
    copy.execute("INSERT INTO cards_fts(cards_fts) VALUES ('rebuild')", [])?;
    let fts_index_rebuild_us = rebuild_start.elapsed().as_micros() as u64;
    copy.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(FtsBuildCost {
        database_snapshot_us,
        fts_index_rebuild_us,
    })
}

/// Average metric triples for a non-empty query group.
fn mean_scores(values: &[Metrics]) -> Metrics {
    if values.is_empty() {
        return Metrics::default();
    }
    let count = values.len() as f64;
    Metrics {
        recall: values.iter().map(|value| value.recall).sum::<f64>() / count,
        mrr: values.iter().map(|value| value.mrr).sum::<f64>() / count,
        ndcg: values.iter().map(|value| value.ndcg).sum::<f64>() / count,
    }
}

/// Average candidate-recall values, returning zero for an empty group.
fn mean_recall(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// Aggregate leg metrics and candidate recall by query group.
fn summary_for(observations: &[(&Observation, &GoldenQuery)]) -> Summary {
    let mut group_rows = BTreeMap::<String, Vec<(&Observation, &GoldenQuery)>>::new();
    let mut valid = Vec::new();
    for (observation, item) in observations {
        if item.split() == "behavior" {
            continue;
        }
        group_rows
            .entry(item.group())
            .or_default()
            .push((observation, item));
        valid.push((*observation, *item));
    }
    let summarize_leg = |rows: &[(&Observation, &GoldenQuery)], leg: &str| {
        let (metrics, recall): (Vec<Metrics>, Vec<f64>) = rows
            .iter()
            .map(|(observation, _)| match leg {
                "fts" => (observation.fts_metrics, observation.fts_candidate_recall20),
                "vector" => (
                    observation.vector_metrics,
                    observation.vector_candidate_recall20,
                ),
                _ => (
                    observation.hybrid_metrics,
                    observation.union_candidate_recall20,
                ),
            })
            .unzip();
        let mean = mean_scores(&metrics);
        LegSummary {
            recall: mean.recall,
            mrr: mean.mrr,
            ndcg: mean.ndcg,
            candidate_recall20: mean_recall(&recall),
        }
    };
    let by_group = group_rows
        .into_iter()
        .map(|(group, rows)| {
            let values = ["fts", "vector", "hybrid"]
                .into_iter()
                .map(|leg| (leg.to_string(), summarize_leg(&rows, leg)))
                .collect();
            (group, values)
        })
        .collect();
    let union_recalls: Vec<f64> = valid
        .iter()
        .map(|(observation, _)| observation.union_candidate_recall20)
        .collect();
    Summary {
        query_count: valid.len(),
        fts: summarize_leg(&valid, "fts"),
        vector: summarize_leg(&valid, "vector"),
        hybrid: summarize_leg(&valid, "hybrid"),
        union_candidate_recall20: mean_recall(&union_recalls),
        by_group,
    }
}

/// Record ordered ranks and paired score changes for one query.
fn paired_query(input: &QueryInput, default: &Observation, candidate: &Observation) -> PairedQuery {
    PairedQuery {
        query_index: input.index,
        query: input.golden.query.clone(),
        group: input.golden.group(),
        split: input.golden.split(),
        default_hybrid: default.hybrid.clone(),
        candidate_hybrid: candidate.hybrid.clone(),
        default_ndcg: common::round3(default.hybrid_metrics.ndcg),
        candidate_ndcg: common::round3(candidate.hybrid_metrics.ndcg),
        ndcg_delta: common::round3(candidate.hybrid_metrics.ndcg - default.hybrid_metrics.ndcg),
        default_candidate_recall20: common::round3(default.union_candidate_recall20),
        candidate_candidate_recall20: common::round3(candidate.union_candidate_recall20),
        fts_top20: candidate.fts.clone(),
        vector_top20: candidate.vector.clone(),
    }
}

/// Parse the output mode.
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

/// Tune on development queries and check the chosen settings on held-out queries.
fn main() -> anyhow::Result<()> {
    let json = parse_args()?;
    let root = data_dir()?;
    let paths = Paths::new(root.clone());
    anyhow::ensure!(paths.is_setup(), "no completed store at {}", root.display());
    let conn = db::open(&paths.db())?;
    let fts_build_cost = measure_fts_rebuild(&conn)?;
    let cards = db::load_all_cards(&conn)?;
    let store = embed::VectorStore::load(paths.root())?;
    let status = Status::read(&paths.status_file())?;
    let tag_index = tags::TagIndex::load(&conn)?;
    let docs: Vec<String> = cards
        .iter()
        .map(|card| embed::doc_for_row(card, &tag_index))
        .collect();
    let fingerprint = common::fingerprint_strings(
        cards
            .iter()
            .zip(&docs)
            .flat_map(|(card, doc)| [card.name.as_str(), doc.as_str()]),
    );
    anyhow::ensure!(
        store.is_empty()
            || (store.len() == cards.len()
                && store
                    .meta
                    .names
                    .iter()
                    .zip(&cards)
                    .all(|(name, card)| name == &card.name)),
        "production vectors do not match database card order"
    );
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let golden = common::load_queries(&manifest.join("benches/golden_queries.json"))?;
    common::validate_labels(&golden, &cards)?;
    let mut model = if store.is_empty() {
        None
    } else {
        Some(embed::load_query_model(&paths.models_dir(), false)?)
    };
    let output = Output::new(false, true, false);
    let mut end_to_end = Vec::with_capacity(golden.len());
    let mut embed_times = Vec::with_capacity(golden.len());
    let mut inputs = Vec::with_capacity(golden.len());
    for (index, item) in golden.iter().enumerate() {
        let filters = item.search_filters()?;
        let prepared = query::prepare_query(&item.query);
        let start = Instant::now();
        let query_vector = match model.as_mut() {
            Some(model) => store.embed_query(model, &prepared.expanded_text)?,
            None => Vec::new(),
        };
        let embed_latency = start.elapsed();
        let start = Instant::now();
        let production_hits =
            query::run_search(&paths, &conn, &output, &item.query, &filters, CUTOFF, None)?;
        let production_latency = start.elapsed();
        end_to_end.push(production_latency);
        embed_times.push(embed_latency);

        let mut input = QueryInput {
            index: index + 1,
            golden: item.clone(),
            filters,
            query_vector,
            production_names: hit_names(&production_hits, CUTOFF),
            default: Observation {
                fts: Vec::new(),
                vector: Vec::new(),
                hybrid: Vec::new(),
                fts_metrics: Metrics::default(),
                vector_metrics: Metrics::default(),
                hybrid_metrics: Metrics::default(),
                fts_candidate_recall20: 0.0,
                vector_candidate_recall20: 0.0,
                union_candidate_recall20: 0.0,
                search_us: 0,
            },
        };
        input.default = observe(
            &conn,
            &cards,
            &store,
            &input,
            query::DEFAULT_SEARCH_SETTINGS,
        )?;
        anyhow::ensure!(
            input.production_names == input.default.hybrid,
            "production default parity failed for query {} ({:?})\nrun_search: {:?}\nshared: {:?}",
            index + 1,
            item.query,
            input.production_names,
            input.default.hybrid
        );
        inputs.push(input);
    }

    let variants = variants();
    let mut reports = Vec::with_capacity(variants.len());
    for variant in &variants {
        if variant.name == "current-default" {
            let paired_queries = inputs
                .iter()
                .filter(|input| input.golden.split() == "development")
                .map(|input| paired_query(input, &input.default, &input.default))
                .collect::<Vec<_>>();
            let observations: Vec<(&Observation, &GoldenQuery)> = inputs
                .iter()
                .filter(|input| input.golden.split() == "development")
                .map(|input| (&input.default, &input.golden))
                .collect();
            reports.push(VariantReport {
                name: variant.name.clone(),
                changed_parameter: variant.changed_parameter.clone(),
                value: variant.value.clone(),
                settings: settings_info(variant.settings),
                summary: summary_for(&observations),
                mean_shared_search_us: average_search_us(
                    inputs
                        .iter()
                        .filter(|input| input.golden.split() == "development"),
                ),
                paired_queries,
            });
            continue;
        }
        let mut observations_owned = Vec::new();
        let mut paired_queries = Vec::new();
        for input in inputs
            .iter()
            .filter(|input| input.golden.split() == "development")
        {
            let candidate = observe(&conn, &cards, &store, input, variant.settings)?;
            paired_queries.push(paired_query(input, &input.default, &candidate));
            observations_owned.push(candidate);
        }
        let observations: Vec<(&Observation, &GoldenQuery)> = inputs
            .iter()
            .filter(|input| input.golden.split() == "development")
            .zip(&observations_owned)
            .map(|(input, observation)| (observation, &input.golden))
            .collect();
        reports.push(VariantReport {
            name: variant.name.clone(),
            changed_parameter: variant.changed_parameter.clone(),
            value: variant.value.clone(),
            settings: settings_info(variant.settings),
            summary: summary_for(&observations),
            mean_shared_search_us: average_values(
                observations_owned
                    .iter()
                    .map(|observation| observation.search_us),
            ),
            paired_queries,
        });
    }

    let best_index = select_best_variant(&reports);
    let best_variant = &variants[best_index];
    let best_development_ndcg = reports[best_index].summary.hybrid.ndcg;
    let default_development_ndcg = reports[0].summary.hybrid.ndcg;
    let mut held_out_observations = Vec::new();
    let mut held_out_pairs = Vec::new();
    for input in inputs
        .iter()
        .filter(|input| input.golden.split() == "held_out")
    {
        let selected = if best_variant.name == "current-default" {
            input.default.clone()
        } else {
            observe(&conn, &cards, &store, input, best_variant.settings)?
        };
        held_out_pairs.push(paired_query(input, &input.default, &selected));
        held_out_observations.push((selected, &input.golden));
    }
    let held_out_refs: Vec<(&Observation, &GoldenQuery)> = held_out_observations
        .iter()
        .map(|(observation, item)| (observation, *item))
        .collect();
    let default_held_out: Vec<(&Observation, &GoldenQuery)> = inputs
        .iter()
        .filter(|input| input.golden.split() == "held_out")
        .map(|input| (&input.default, &input.golden))
        .collect();

    let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let report = SweepReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        evaluation_version: "shared-search-sweep-v1",
        label_version: common::LABEL_VERSION,
        corpus: CorpusInfo {
            cards: cards.len(),
            vectors: store.len(),
            model: status.model,
            dimension: status.dim,
            document_version: status.doc_version,
            database_schema_version: schema_version,
            query_count: golden.len(),
            development_queries: golden
                .iter()
                .filter(|item| item.split() == "development")
                .count(),
            held_out_queries: golden
                .iter()
                .filter(|item| item.split() == "held_out")
                .count(),
            behavior_queries: golden
                .iter()
                .filter(|item| item.split() == "behavior")
                .count(),
            fingerprint,
            query_fingerprint: common::fingerprint_queries(&golden)?,
        },
        ranking_defaults: settings_info(query::DEFAULT_SEARCH_SETTINGS),
        split_rule: "Filtered queries are held out; behavior probes are excluded; all other queries are development.",
        production_end_to_end: duration_summary(&end_to_end)?,
        query_embedding: duration_summary(&embed_times)?,
        setup_cost: SetupCost {
            database_bytes: std::fs::metadata(paths.db())?.len(),
            vector_bytes: std::fs::metadata(paths.vectors_file())?.len(),
            database_snapshot_us: fts_build_cost.database_snapshot_us,
            fts_index_rebuild_us: fts_build_cost.fts_index_rebuild_us,
            fts_rebuild_scope: "VACUUM INTO plus FTS5 rebuild on a temporary copy. Does not include Scryfall ingest or vector embedding.",
            embedding_cost_reference: "Candidate full-corpus embedding seconds are reported separately by query_bakeoff on cache misses.",
        },
        controls: summary_for(
            &inputs
                .iter()
                .map(|input| (&input.default, &input.golden))
                .collect::<Vec<_>>(),
        ),
        development_variants: reports,
        selection: Selection {
            variant: best_variant.name.clone(),
            rule: "Highest development hybrid nDCG@10; ties use MRR@10, then keep current defaults.",
            development_hybrid_ndcg: best_development_ndcg,
            default_development_hybrid_ndcg: default_development_ndcg,
        },
        held_out_check: HeldOutCheck {
            selected_variant: best_variant.name.clone(),
            default: summary_for(&default_held_out),
            selected: summary_for(&held_out_refs),
            paired_queries: held_out_pairs,
        },
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report);
    }
    Ok(())
}

/// Average shared-search timings from a set of prepared queries.
fn average_search_us<'a>(inputs: impl Iterator<Item = &'a QueryInput>) -> u64 {
    let values: Vec<u64> = inputs.map(|input| input.default.search_us).collect();
    average_values(values)
}

/// Average integer timing samples, returning zero when there are none.
fn average_values(values: impl IntoIterator<Item = u64>) -> u64 {
    let values: Vec<u64> = values.into_iter().collect();
    if values.is_empty() {
        0
    } else {
        values.iter().sum::<u64>() / values.len() as u64
    }
}

/// Select by development nDCG, then MRR, keeping defaults on exact ties.
fn select_best_variant(reports: &[VariantReport]) -> usize {
    let mut best = 0;
    for index in 1..reports.len() {
        let current = reports[index].summary.hybrid;
        let selected = reports[best].summary.hybrid;
        if current.ndcg > selected.ndcg
            || (current.ndcg == selected.ndcg && current.mrr > selected.mrr)
        {
            best = index;
        }
    }
    best
}

/// Print development comparisons and the single held-out check.
fn print_human(report: &SweepReport) {
    println!(
        "Search-setting sweep · {} cards · {} queries · labels {}",
        report.corpus.cards, report.corpus.query_count, report.label_version
    );
    println!("Filtered queries are held out from development tuning.");
    println!(
        "\n{:<22} {:>8} {:>8} {:>8} {:>9} {:>10}",
        "setting", "recall", "mrr", "ndcg", "cand@20", "search µs"
    );
    for variant in &report.development_variants {
        println!(
            "{:<22} {:>8.3} {:>8.3} {:>8.3} {:>9.3} {:>10}",
            variant.name,
            variant.summary.hybrid.recall,
            variant.summary.hybrid.mrr,
            variant.summary.hybrid.ndcg,
            variant.summary.union_candidate_recall20,
            variant.mean_shared_search_us
        );
    }
    println!(
        "\nSelected on development: {} (nDCG {:.3}). Held-out default {:.3}; selected {:.3}.",
        report.selection.variant,
        report.selection.development_hybrid_ndcg,
        report.held_out_check.default.hybrid.ndcg,
        report.held_out_check.selected.hybrid.ndcg
    );
    println!(
        "Production run_search latency p50 {}µs, p95 {}µs. FTS rebuild took {}µs on a temporary copy.",
        report.production_end_to_end.p50_us,
        report.production_end_to_end.p95_us,
        report.setup_cost.fts_index_rebuild_us
    );
}

#[cfg(test)]
#[path = "tests/query_rank_sweep_tests.rs"]
mod tests;
