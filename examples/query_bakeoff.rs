// Compare embedding models while keeping retrieval and fusion on the shared
// production search path.
//
//   cargo run --release --example query_bakeoff -- --only bge-small,minilm-l6
//   cargo run --release --example query_bakeoff -- --only bge-small-full --format fielded,compact
//   cargo run --release --example query_bakeoff -- --backend coreml --json --save bakeoff.json

/// Vector cache storage and validation.
#[path = "query_bakeoff/cache.rs"]
mod cache;
/// Shared golden-query data and metric helpers.
mod common;
/// Card document layouts and model instructions.
#[path = "query_bakeoff/document.rs"]
mod document;
/// Ranking metrics and bake-off reports.
#[path = "query_bakeoff/evaluation.rs"]
mod evaluation;
/// Candidate models, backends, and throughput measurements.
#[path = "query_bakeoff/model.rs"]
mod model;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context as _;
#[cfg(test)]
use fastembed::ModelTrait as _;
use seizethemana::paths::{Paths, Status};
#[cfg(test)]
use seizethemana::query::VectorRowProvider as _;
use seizethemana::{db, embed, query, tags};

#[cfg(test)]
use cache::{
    CACHE_FORMAT_VERSION, CacheMetadata, FlatMatrix, cache_metadata, read_cache, write_cache,
};
use cache::{VectorCacheIdentity, build_vectors};
#[cfg(test)]
use common::GoldenQuery;
#[cfg(test)]
use document::{DocumentFormat, render_document};
use document::{candidate_document, select_document_formats};
use evaluation::{
    BakeoffReport, CorpusInfo, EvaluationContext, FtsIndexInfo, ProductionReference,
    build_candidate_report, duration_summary, metric_mean, print_human, production_queries,
    settings_info,
};
#[cfg(test)]
use evaluation::{REGRESSION_THRESHOLD, validate_bge_parity};
#[cfg(test)]
use model::{BGE_QUERY_PROMPT, CANDIDATES, normalize_embedding};
use model::{
    Backend, BackendSelection, CandidateModel, load_candidate_model, run_throughput_pilot,
    select_candidates,
};

const CUTOFF: usize = 10;

/// FTS5 tokenizer profile built for a bake-off run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FtsTokenizer {
    /// Production Porter stemming over Unicode tokens.
    Porter,
    /// SQLite's Unicode tokenizer without stemming.
    Unicode61,
}

impl FtsTokenizer {
    /// Stable identifier used in reports and CLI arguments.
    fn key(self) -> &'static str {
        match self {
            Self::Porter => "porter",
            Self::Unicode61 => "unicode61",
        }
    }

    /// SQLite FTS5 tokenizer expression.
    fn expression(self) -> &'static str {
        match self {
            Self::Porter => "porter unicode61",
            Self::Unicode61 => "unicode61",
        }
    }
}

/// Resolve the store directory from the environment or the default home path.
fn data_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("STM_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".seizethemana"))
}

/// Temporary FTS5 profile and its measured database-copy costs.
struct FtsIndexVariant {
    _directory: tempfile::TempDir,
    conn: rusqlite::Connection,
    snapshot_us: u64,
    rebuild_us: u64,
}

/// Copy the store and rebuild only its FTS index with a selected tokenizer.
fn build_fts_index_variant(
    source: &rusqlite::Connection,
    tokenizer: FtsTokenizer,
) -> anyhow::Result<FtsIndexVariant> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("fts-profile.db");
    let snapshot_start = Instant::now();
    source.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
    let snapshot_us = snapshot_start.elapsed().as_micros() as u64;

    let conn = db::open(&path)?;
    conn.execute_batch("DROP TABLE cards_fts")?;
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE cards_fts USING fts5(
            name, tags_text, type_line, oracle_text,
            content='cards', content_rowid='id',
            tokenize='{}'
        )",
        tokenizer.expression()
    ))?;
    let rebuild_start = Instant::now();
    conn.execute("INSERT INTO cards_fts(cards_fts) VALUES ('rebuild')", [])?;
    let rebuild_us = rebuild_start.elapsed().as_micros() as u64;
    Ok(FtsIndexVariant {
        _directory: directory,
        conn,
        snapshot_us,
        rebuild_us,
    })
}

/// CLI options for one model, document-layout, and ranking comparison.
struct BakeoffArgs {
    only: Option<String>,
    formats: Option<String>,
    fts_tokenizer: FtsTokenizer,
    backend: Backend,
    throughput_pilot: bool,
    settings: query::SearchSettings,
    json: bool,
    save: Option<PathBuf>,
}

/// Parse candidate, document-format, ranking, and report options.
fn parse_args() -> anyhow::Result<BakeoffArgs> {
    parse_args_from(std::env::args().skip(1))
}

/// Parse one bake-off argument list.
fn parse_args_from(args: impl IntoIterator<Item = String>) -> anyhow::Result<BakeoffArgs> {
    let mut parsed = BakeoffArgs {
        only: None,
        formats: None,
        fts_tokenizer: FtsTokenizer::Porter,
        backend: Backend::Auto,
        throughput_pilot: false,
        settings: query::DEFAULT_SEARCH_SETTINGS,
        json: false,
        save: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => parsed.only = Some(args.next().context("--only needs candidate keys")?),
            "--format" => parsed.formats = Some(args.next().context("--format needs layout keys")?),
            "--fts-tokenizer" => {
                parsed.fts_tokenizer = match args
                    .next()
                    .context("--fts-tokenizer needs 'porter' or 'unicode61'")?
                    .as_str()
                {
                    "porter" => FtsTokenizer::Porter,
                    "unicode61" => FtsTokenizer::Unicode61,
                    value => anyhow::bail!(
                        "unknown FTS tokenizer {value:?}; expected porter or unicode61"
                    ),
                };
            }
            "--backend" => {
                parsed.backend = match args
                    .next()
                    .context("--backend needs auto, cpu, or coreml")?
                    .as_str()
                {
                    "auto" => Backend::Auto,
                    "cpu" => Backend::Cpu,
                    "coreml" => Backend::CoreMl,
                    value => {
                        anyhow::bail!("unknown backend {value:?}; expected auto, cpu, or coreml")
                    }
                };
            }
            "--throughput-pilot" => parsed.throughput_pilot = true,
            "--fts-weights" => {
                let value = args
                    .next()
                    .context("--fts-weights needs four comma-separated values")?;
                let weights = value
                    .split(',')
                    .map(|part| part.trim().parse::<f64>())
                    .collect::<Result<Vec<_>, _>>()
                    .context("parsing --fts-weights")?;
                anyhow::ensure!(
                    weights.len() == 4,
                    "--fts-weights needs name,tags,type,oracle"
                );
                parsed.settings.fts_column_weights =
                    [weights[0], weights[1], weights[2], weights[3]];
            }
            "--fts-depth" => {
                parsed.settings.fts_candidate_depth = parse_value(
                    args.next().context("--fts-depth needs a number")?,
                    "--fts-depth",
                )?;
            }
            "--vector-depth" => {
                parsed.settings.vector_candidate_depth = parse_value(
                    args.next().context("--vector-depth needs a number")?,
                    "--vector-depth",
                )?;
            }
            "--rrf-k" => {
                parsed.settings.rrf_k =
                    parse_value(args.next().context("--rrf-k needs a number")?, "--rrf-k")?;
            }
            "--fts-rrf-weight" => {
                parsed.settings.fts_rrf_weight = parse_value(
                    args.next().context("--fts-rrf-weight needs a number")?,
                    "--fts-rrf-weight",
                )?;
            }
            "--vector-rrf-weight" => {
                parsed.settings.vector_rrf_weight = parse_value(
                    args.next().context("--vector-rrf-weight needs a number")?,
                    "--vector-rrf-weight",
                )?;
            }
            "--fts-terms" => {
                parsed.settings.fts_term_operator = match args
                    .next()
                    .context("--fts-terms needs 'any' or 'all'")?
                    .as_str()
                {
                    "any" => db::FtsTermOperator::Any,
                    "all" => db::FtsTermOperator::All,
                    value => anyhow::bail!("unknown FTS term mode {value:?}; expected any or all"),
                };
            }
            "--fts-overfetch" => {
                parsed.settings.fts_overfetch_multiplier = parse_value(
                    args.next().context("--fts-overfetch needs a number")?,
                    "--fts-overfetch",
                )?;
            }
            "--fts-min-rows" => {
                parsed.settings.fts_min_sql_rows = parse_value(
                    args.next().context("--fts-min-rows needs a number")?,
                    "--fts-min-rows",
                )?;
            }
            "--fts-max-rounds" => {
                parsed.settings.fts_max_rounds = parse_value(
                    args.next().context("--fts-max-rounds needs a number")?,
                    "--fts-max-rounds",
                )?;
            }
            "--json" => parsed.json = true,
            "--save" => {
                parsed.save = Some(PathBuf::from(
                    args.next().context("--save needs a file path")?,
                ))
            }
            _ => anyhow::bail!("unknown argument {arg:?}"),
        }
    }
    validate_settings(parsed.settings)?;
    Ok(parsed)
}

/// Parse one numeric option with its name in any error message.
fn parse_value<T>(value: String, option: &str) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid {option} value {value:?}: {error}"))
}

/// Reject invalid settings before the bake-off opens the store or loads models.
fn validate_settings(settings: query::SearchSettings) -> anyhow::Result<()> {
    anyhow::ensure!(
        settings
            .fts_column_weights
            .iter()
            .all(|weight| weight.is_finite() && *weight >= 0.0),
        "FTS weights must be finite and non-negative"
    );
    anyhow::ensure!(
        settings.rrf_k.is_finite()
            && settings.rrf_k >= 0.0
            && settings.fts_rrf_weight.is_finite()
            && settings.fts_rrf_weight >= 0.0
            && settings.vector_rrf_weight.is_finite()
            && settings.vector_rrf_weight >= 0.0
            && settings.fts_rrf_weight + settings.vector_rrf_weight > 0.0,
        "RRF settings must be finite and non-negative. At least one search-leg weight must be positive"
    );
    anyhow::ensure!(
        settings.fts_overfetch_multiplier > 0
            && settings.fts_min_sql_rows > 0
            && settings.fts_max_rounds > 0,
        "FTS over-fetch settings must be positive"
    );
    Ok(())
}

/// Require Core ML for candidate reports written to disk.
fn validate_save_backend(save: Option<&Path>, backend: Backend) -> anyhow::Result<()> {
    anyhow::ensure!(
        save.is_none() || backend == Backend::CoreMl,
        "saved bake-off reports must use '--backend coreml'"
    );
    Ok(())
}

/// Compare selected embedding candidates against the production CLI path.
fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    validate_save_backend(args.save.as_deref(), args.backend)?;
    let selected = select_candidates(args.only.as_deref())?;
    anyhow::ensure!(
        args.backend != Backend::CoreMl
            || selected
                .iter()
                .all(|candidate| matches!(candidate.model, CandidateModel::Onnx(_))),
        "Core ML is available only for ONNX candidates"
    );
    let document_formats = select_document_formats(args.formats.as_deref())?;
    let root = data_dir()?;
    let paths = Paths::new(root.clone());
    anyhow::ensure!(
        paths.is_setup(),
        "no completed store at {}; run 'stm setup' in release mode first",
        root.display()
    );
    let conn = db::open(&paths.db())?;
    let fts_index = if args.fts_tokenizer == FtsTokenizer::Porter {
        None
    } else {
        Some(build_fts_index_variant(&conn, args.fts_tokenizer)?)
    };
    let search_conn = fts_index.as_ref().map_or(&conn, |variant| &variant.conn);
    let cards = db::load_all_cards(&conn)?;
    let tag_index = tags::TagIndex::load(&conn)?;
    if args.throughput_pilot {
        return run_throughput_pilot(
            &selected,
            &document_formats,
            &cards,
            &tag_index,
            &paths.models_dir(),
            args.backend,
            args.save.as_deref(),
        );
    }
    let fingerprint = common::fingerprint_corpus(&cards, &tag_index)?;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let golden = common::load_queries(&manifest.join("benches/golden_queries.json"))?;
    common::validate_labels(&golden, &cards)?;
    let status = Status::read(&paths.status_file())?;
    let settings = args.settings;
    let production = production_queries(&paths, &conn, &golden)?;
    let cli_scores = production
        .iter()
        .zip(&golden)
        .map(|(reference, item)| (reference.metrics, item))
        .collect::<Vec<_>>();
    let production_metrics = metric_mean(cli_scores.into_iter(), None);
    let mut reports = Vec::with_capacity(selected.len() * document_formats.len());
    for candidate in selected {
        let effective_backend = match args.backend {
            Backend::Auto | Backend::Cpu => Backend::Cpu,
            Backend::CoreMl => Backend::CoreMl,
        };
        let mut model = load_candidate_model(
            candidate,
            &paths.models_dir(),
            true,
            effective_backend,
            false,
        )?;
        for document_format in &document_formats {
            let docs: Vec<String> = cards
                .iter()
                .map(|card| candidate_document(candidate, *document_format, card, &tag_index))
                .collect();
            let document_fingerprint = common::fingerprint_strings(
                cards
                    .iter()
                    .zip(&docs)
                    .flat_map(|(card, doc)| [card.name.as_str(), doc.as_str()]),
            );
            let (matrix, build_secs, metadata) = build_vectors(
                &root,
                candidate,
                *document_format,
                &mut model,
                &cards,
                &docs,
                VectorCacheIdentity {
                    corpus_fingerprint: &document_fingerprint,
                    backend: effective_backend,
                },
            )?;
            let evaluation = EvaluationContext {
                golden: &golden,
                production: &production,
                cards: &cards,
                conn: search_conn,
                settings,
                document_format: *document_format,
                fts_tokenizer: args.fts_tokenizer,
            };
            let report = build_candidate_report(
                candidate,
                &matrix,
                &mut model,
                build_secs,
                metadata,
                &evaluation,
                BackendSelection {
                    requested: args.backend,
                    effective: effective_backend,
                },
            )?;
            reports.push(report);
        }
    }
    let production_store = embed::VectorStore::load(paths.root())?;
    anyhow::ensure!(
        production_store.is_empty()
            || (production_store.len() == cards.len()
                && production_store
                    .meta
                    .names
                    .iter()
                    .zip(&cards)
                    .all(|(name, card)| name == &card.name)),
        "production vectors do not align with cards in database order"
    );
    let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let report = BakeoffReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        evaluation_version: "shared-search-formats-v2",
        label_version: common::LABEL_VERSION,
        corpus: CorpusInfo {
            cards: cards.len(),
            golden_queries: golden.len(),
            cutoff: CUTOFF,
            fingerprint,
            query_fingerprint: common::fingerprint_queries(&golden)?,
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
            setup_cost: "This run reports candidate document embedding time on cache misses. A non-production FTS tokenizer uses a temporary database copy and reports its copy and rebuild times.",
        },
        ranking_settings: settings_info(settings, args.fts_tokenizer),
        fts_index: FtsIndexInfo {
            tokenizer: args.fts_tokenizer.key(),
            database_snapshot_us: fts_index.as_ref().map(|variant| variant.snapshot_us),
            index_rebuild_us: fts_index.as_ref().map(|variant| variant.rebuild_us),
        },
        production_reference: ProductionReference {
            model: status.model,
            metrics: production_metrics,
            end_to_end_us: duration_summary(
                &production.iter().map(|run| run.elapsed).collect::<Vec<_>>(),
            )?,
            query_count: production.len(),
            ranking_source: "query::run_search; includes store reads, model initialization, query embedding, and shared ranking",
        },
        candidates: reports,
        database_schema_version: schema_version,
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report);
    }
    if let Some(path) = args.save {
        let encoded = serde_json::to_string_pretty(&report)?;
        std::fs::write(&path, encoded)
            .with_context(|| format!("writing bake-off report to {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/query_bakeoff_tests.rs"]
mod tests;
