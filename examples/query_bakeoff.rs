// Compare embedding models while keeping retrieval and fusion on the shared
// production search path.
//
//   cargo run --release --example query_bakeoff -- --only bge-small,minilm-l6
//   cargo run --release --example query_bakeoff -- --only bge-small-full --format fielded,compact
//   cargo run --release --example query_bakeoff -- --backend coreml --json --save bakeoff.json

/// Shared golden-query data and metric helpers.
mod common;

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context as _;
use fastembed::ModelTrait as _;
use seizethemana::embed::DocumentEmbedder as _;
use seizethemana::paths::{Paths, Status};
use seizethemana::query::VectorRowProvider as _;
use seizethemana::{db, embed, query, tags};
use serde::{Deserialize, Serialize};

use common::{GoldenQuery, Metrics};

const CUTOFF: usize = 10;
const CACHE_MAGIC: u32 = 0x5354_4D42;
const CACHE_FORMAT_VERSION: u32 = 3;
const REGRESSION_THRESHOLD: f64 = 0.05;
const BGE_QUERY_PROMPT: &str = "Represent this sentence for searching relevant passages: ";

/// ONNX Runtime provider requested for a bake-off run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    /// Select the evaluator's default backend.
    Auto,
    /// Use the CPU execution provider.
    Cpu,
    /// Run fixed-shape ML Program document batches on Core ML.
    CoreMl,
}

impl Backend {
    /// Stable provider key for reports and cache identities.
    fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::CoreMl => "coreml",
        }
    }
}

/// Requested and selected providers for one model run.
#[derive(Debug, Clone, Copy)]
struct BackendSelection {
    requested: Backend,
    effective: Backend,
}

/// Cache identity fields that vary by backend and ordered corpus.
#[derive(Debug, Clone, Copy)]
struct VectorCacheIdentity<'a> {
    corpus_fingerprint: &'a str,
    backend: Backend,
}

/// Embedding backend used by one bake-off candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CandidateModel {
    /// ONNX model in fastembed's standard text registry.
    Onnx(fastembed::EmbeddingModel),
}

/// One model and its documented retrieval input contract.
#[derive(Debug, Clone)]
struct Candidate {
    /// Stable CLI key.
    key: &'static str,
    /// fastembed model implementation and registry entry.
    model: CandidateModel,
    /// Expected vector dimension.
    dim: usize,
    /// Token limit used for both documents and queries.
    max_length: usize,
    /// Maximum sequence length supported by the model card.
    model_max_length: usize,
    /// Model-specific document instruction, applied by `candidate_document`.
    document_prompt: &'static str,
    /// Prefix applied to every expanded search query.
    query_prompt: &'static str,
    /// Pooling used by the fastembed registry for this model.
    pooling: &'static str,
    /// Primary model-card URL used to verify the retrieval contract.
    model_card: &'static str,
}

const CANDIDATES: &[Candidate] = &[
    Candidate {
        key: "bge-small",
        model: CandidateModel::Onnx(fastembed::EmbeddingModel::BGESmallENV15Q),
        dim: 384,
        max_length: 128,
        model_max_length: 512,
        document_prompt: "",
        query_prompt: BGE_QUERY_PROMPT,
        pooling: "CLS",
        model_card: "https://huggingface.co/BAAI/bge-small-en-v1.5",
    },
    Candidate {
        key: "bge-small-256",
        model: CandidateModel::Onnx(fastembed::EmbeddingModel::BGESmallENV15Q),
        dim: 384,
        max_length: 256,
        model_max_length: 512,
        document_prompt: "",
        query_prompt: BGE_QUERY_PROMPT,
        pooling: "CLS",
        model_card: "https://huggingface.co/BAAI/bge-small-en-v1.5",
    },
    Candidate {
        key: "bge-small-full",
        model: CandidateModel::Onnx(fastembed::EmbeddingModel::BGESmallENV15),
        dim: 384,
        max_length: 128,
        model_max_length: 512,
        document_prompt: "",
        query_prompt: BGE_QUERY_PROMPT,
        pooling: "CLS",
        model_card: "https://huggingface.co/BAAI/bge-small-en-v1.5",
    },
    Candidate {
        key: "arctic-s",
        model: CandidateModel::Onnx(fastembed::EmbeddingModel::SnowflakeArcticEmbedSQ),
        dim: 384,
        max_length: 256,
        model_max_length: 512,
        document_prompt: "",
        query_prompt: BGE_QUERY_PROMPT,
        pooling: "CLS",
        model_card: "https://huggingface.co/Snowflake/snowflake-arctic-embed-s",
    },
    Candidate {
        key: "minilm-l6",
        model: CandidateModel::Onnx(fastembed::EmbeddingModel::AllMiniLML6V2Q),
        dim: 384,
        max_length: 256,
        model_max_length: 256,
        document_prompt: "",
        query_prompt: "",
        pooling: "mean",
        model_card: "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2",
    },
    Candidate {
        key: "minilm-l12",
        model: CandidateModel::Onnx(fastembed::EmbeddingModel::AllMiniLML12V2Q),
        dim: 384,
        max_length: 256,
        model_max_length: 256,
        document_prompt: "",
        query_prompt: "",
        pooling: "mean",
        model_card: "https://huggingface.co/sentence-transformers/all-MiniLM-L12-v2",
    },
    Candidate {
        key: "paraphrase-minilm-l12",
        model: CandidateModel::Onnx(fastembed::EmbeddingModel::ParaphraseMLMiniLML12V2Q),
        dim: 384,
        max_length: 128,
        model_max_length: 128,
        document_prompt: "",
        query_prompt: "",
        pooling: "mean",
        model_card: "https://huggingface.co/sentence-transformers/paraphrase-MiniLM-L12-v2",
    },
    Candidate {
        key: "arctic-xs",
        model: CandidateModel::Onnx(fastembed::EmbeddingModel::SnowflakeArcticEmbedXSQ),
        dim: 384,
        max_length: 256,
        model_max_length: 512,
        document_prompt: "",
        query_prompt: BGE_QUERY_PROMPT,
        pooling: "CLS",
        model_card: "https://huggingface.co/Snowflake/snowflake-arctic-embed-xs",
    },
];

/// Card-document layouts that can be compared without changing card facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentFormat {
    /// Production layout from `embed::doc_for_row`.
    Production,
    /// One explicit label per field, with line breaks between fields.
    Fielded,
    /// The same labeled fields on one compact line.
    Compact,
    /// Put rules text before secondary fields.
    RulesFirst,
    /// Keep name and type immediately before rules text.
    NameTypeRulesFirst,
    /// Put role tags before other card facts.
    TagsFirst,
    /// Put tags last after the rules text.
    TagsLast,
    /// Place card identity next to rules text in short prose.
    Contextual,
}

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

impl DocumentFormat {
    /// Stable cache and report key for this layout.
    fn key(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Fielded => "fielded",
            Self::Compact => "compact",
            Self::RulesFirst => "rules-first",
            Self::NameTypeRulesFirst => "name-type-rules-first",
            Self::TagsFirst => "tags-first",
            Self::TagsLast => "tags-last",
            Self::Contextual => "contextual",
        }
    }
}

/// Fingerprinted metadata that must match before a vector cache can be used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CacheMetadata {
    format_version: u32,
    candidate_key: String,
    #[serde(default)]
    backend: String,
    model: String,
    #[serde(default)]
    model_fingerprint: Option<String>,
    dimension: usize,
    card_count: usize,
    max_length: usize,
    document_version: u32,
    #[serde(default = "production_document_format")]
    document_format: String,
    document_prompt: String,
    query_prompt: String,
    pooling: String,
    corpus_fingerprint: String,
}

/// Preserve compatibility with version-2 caches created before layouts were selectable.
fn production_document_format() -> String {
    DocumentFormat::Production.key().to_string()
}

/// Row-major candidate embeddings aligned with the names in the card table.
#[derive(Debug)]
struct FlatMatrix {
    names: Vec<String>,
    dim: usize,
    data: Vec<f32>,
}

impl FlatMatrix {
    /// Validate and wrap vectors with their card names and row dimension.
    fn from_raw(names: Vec<String>, dim: usize, data: Vec<f32>) -> anyhow::Result<Self> {
        anyhow::ensure!(dim > 0, "vector dimension must be positive");
        anyhow::ensure!(
            data.len().is_multiple_of(dim),
            "vector data length {} is not a multiple of dimension {dim}",
            data.len()
        );
        anyhow::ensure!(
            data.len() / dim == names.len(),
            "vector data has {} rows but {} card names were provided",
            data.len() / dim,
            names.len()
        );
        Ok(Self { names, dim, data })
    }
}

impl query::VectorRowProvider for FlatMatrix {
    fn row_count(&self) -> usize {
        self.names.len()
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn card_name(&self, row: usize) -> Option<&str> {
        self.names.get(row).map(String::as_str)
    }

    fn vector(&self, row: usize) -> Option<&[f32]> {
        (row < self.row_count()).then(|| {
            let start = row * self.dim;
            &self.data[start..start + self.dim]
        })
    }
}

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
struct CandidateReport {
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
struct BakeoffReport {
    generated_at: String,
    evaluation_version: &'static str,
    label_version: &'static str,
    corpus: CorpusInfo,
    database_schema_version: i64,
    ranking_settings: SettingsInfo,
    fts_index: FtsIndexInfo,
    production_reference: ProductionReference,
    candidates: Vec<CandidateReport>,
}

/// FTS tokenizer variant and temporary rebuild cost.
#[derive(Debug, Serialize)]
struct FtsIndexInfo {
    tokenizer: &'static str,
    database_snapshot_us: Option<u64>,
    index_rebuild_us: Option<u64>,
}

/// Corpus and fixed split identity shared by all candidates.
#[derive(Debug, Serialize)]
struct CorpusInfo {
    cards: usize,
    golden_queries: usize,
    cutoff: usize,
    fingerprint: String,
    query_fingerprint: String,
    development_queries: usize,
    held_out_queries: usize,
    behavior_queries: usize,
    setup_cost: &'static str,
}

/// Shared ranking settings used in every candidate run.
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

/// Results and timing from the actual `query::run_search` wrapper.
#[derive(Debug, Serialize)]
struct ProductionReference {
    model: String,
    metrics: Metrics,
    end_to_end_us: DurationSummary,
    query_count: usize,
    ranking_source: &'static str,
}

/// Latency distribution across one production search for each query.
#[derive(Debug, Serialize)]
struct DurationSummary {
    samples: usize,
    p50_us: u64,
    p95_us: u64,
    p99_us: u64,
    mean_us: u64,
}

#[derive(Debug)]
/// Production results and timing for one golden query.
struct ProductionQuery {
    names: Vec<String>,
    metrics: Metrics,
    elapsed: std::time::Duration,
}

/// Shared store and benchmark inputs for one candidate's ranking pass.
struct EvaluationContext<'a> {
    golden: &'a [GoldenQuery],
    production: &'a [ProductionQuery],
    cards: &'a [db::CardRow],
    conn: &'a rusqlite::Connection,
    settings: query::SearchSettings,
    document_format: DocumentFormat,
    fts_tokenizer: FtsTokenizer,
}

/// Resolve the store directory from the environment or the default home path.
fn data_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("STM_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".seizethemana"))
}

/// Return the canonical model identifier from fastembed's model registry.
fn model_name(candidate: &Candidate) -> String {
    match &candidate.model {
        CandidateModel::Onnx(model) => fastembed::EmbeddingModel::get_model_info(model)
            .map(|info| info.model_code.clone())
            .unwrap_or_else(|| model.to_string()),
    }
}

/// Select candidate keys in registry order or from `--only` input order.
fn select_candidates(only: Option<&str>) -> anyhow::Result<Vec<&'static Candidate>> {
    let Some(keys) = only else {
        return Ok(CANDIDATES.iter().collect());
    };
    let keys: Vec<&str> = keys
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .collect();
    anyhow::ensure!(!keys.is_empty(), "--only needs at least one candidate key");
    let mut selected = Vec::with_capacity(keys.len());
    let mut seen = HashSet::new();
    for key in keys {
        anyhow::ensure!(
            seen.insert(key),
            "candidate {key:?} was selected more than once"
        );
        let candidate = CANDIDATES
            .iter()
            .find(|candidate| candidate.key == key)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown candidate {key:?}; known candidates: {}",
                    CANDIDATES
                        .iter()
                        .map(|candidate| candidate.key)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        selected.push(candidate);
    }
    Ok(selected)
}

/// Select document layouts in input order, defaulting to the production layout.
fn select_document_formats(only: Option<&str>) -> anyhow::Result<Vec<DocumentFormat>> {
    let Some(formats) = only else {
        return Ok(vec![DocumentFormat::Production]);
    };
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for key in formats
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        anyhow::ensure!(
            seen.insert(key),
            "document format {key:?} was selected twice"
        );
        let format = match key {
            "production" => DocumentFormat::Production,
            "fielded" => DocumentFormat::Fielded,
            "compact" => DocumentFormat::Compact,
            "rules-first" => DocumentFormat::RulesFirst,
            "name-type-rules-first" => DocumentFormat::NameTypeRulesFirst,
            "tags-first" => DocumentFormat::TagsFirst,
            "tags-last" => DocumentFormat::TagsLast,
            "contextual" => DocumentFormat::Contextual,
            _ => anyhow::bail!(
                "unknown document format {key:?}; expected production, fielded, compact, rules-first, name-type-rules-first, tags-first, tags-last, or contextual"
            ),
        };
        selected.push(format);
    }
    anyhow::ensure!(!selected.is_empty(), "--format needs at least one layout");
    Ok(selected)
}

/// Render card facts using one stable document layout.
fn render_document(
    format: DocumentFormat,
    card: &db::CardRow,
    tag_index: &tags::TagIndex,
) -> String {
    if format == DocumentFormat::Production {
        return embed::doc_for_row(card, tag_index);
    }
    let keywords: Vec<String> = serde_json::from_str(&card.keywords).unwrap_or_default();
    let keywords = if keywords.is_empty() {
        "none".to_string()
    } else {
        keywords.join(", ")
    };
    let colors: Vec<String> = serde_json::from_str(&card.colors).unwrap_or_default();
    let colors = colors
        .iter()
        .filter_map(|color| color.chars().next())
        .collect::<String>();
    let colors = if colors.is_empty() {
        "colorless".to_string()
    } else {
        colors
    };
    let tags = tag_index
        .doc_tags_line(&card.oracle_id)
        .strip_prefix("Tags: ")
        .unwrap_or("none")
        .to_string();
    let stats = match (card.power.as_deref(), card.toughness.as_deref()) {
        (Some(power), Some(toughness)) => Some(format!("P/T: {power}/{toughness}")),
        _ => card
            .loyalty
            .as_deref()
            .map(|loyalty| format!("Loyalty: {loyalty}")),
    };
    let mut fields = vec![
        format!("Name: {}", card.name),
        format!("Mana cost: {}", card.mana_cost),
        format!("Type: {}", card.type_line),
        format!("Keywords: {keywords}"),
        format!("Colors: {colors}"),
        format!("Tags: {tags}"),
    ];
    if let Some(stats) = stats {
        fields.push(stats);
    }
    fields.push(format!("Rules text: {}", card.oracle_text));
    let rules_index = fields
        .iter()
        .position(|field| field.starts_with("Rules text:"))
        .expect("the rendered card fields include rules text");
    match format {
        DocumentFormat::Production => embed::doc_for_row(card, tag_index),
        DocumentFormat::Fielded => fields.join("\n"),
        DocumentFormat::Compact => fields.join(" | "),
        DocumentFormat::RulesFirst => {
            let mut ordered = vec![fields[rules_index].clone(), fields[0].clone()];
            ordered.extend(
                fields
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        *index != 0 && *index != 2 && *index != rules_index && *index != 5
                    })
                    .map(|(_, field)| field.clone()),
            );
            ordered.push(fields[2].clone());
            ordered.push(fields[5].clone());
            ordered.join("\n")
        }
        DocumentFormat::NameTypeRulesFirst => {
            let mut ordered = vec![
                fields[0].clone(),
                fields[2].clone(),
                fields[rules_index].clone(),
            ];
            ordered.extend(
                fields
                    .iter()
                    .enumerate()
                    .filter(|(index, field)| {
                        *index != 0
                            && *index != 2
                            && *index != rules_index
                            && !field.starts_with("Tags:")
                    })
                    .map(|(_, field)| field.clone()),
            );
            ordered.push(fields[5].clone());
            ordered.join("\n")
        }
        DocumentFormat::TagsFirst => {
            let mut ordered = vec![fields[5].clone()];
            ordered.extend(fields[..5].iter().cloned());
            ordered.extend(fields[6..].iter().cloned());
            ordered.join("\n")
        }
        DocumentFormat::TagsLast => {
            let mut ordered = fields[..5].to_vec();
            ordered.extend(fields[6..].iter().cloned());
            ordered.push(fields[5].clone());
            ordered.join("\n")
        }
        DocumentFormat::Contextual => {
            let name = card.name.as_str();
            format!(
                "{name} · {}\n{}\n{}",
                card.type_line,
                card.oracle_text,
                fields[1..6].join("\n")
            )
        }
    }
}

/// Apply model-specific document instructions after rendering card fields.
fn candidate_document(
    candidate: &Candidate,
    format: DocumentFormat,
    card: &db::CardRow,
    tag_index: &tags::TagIndex,
) -> String {
    let document = render_document(format, card, tag_index);
    format!("{}{document}", candidate.document_prompt)
}

/// Convert ranking settings into the stable report representation.
fn settings_info(settings: query::SearchSettings, tokenizer: FtsTokenizer) -> SettingsInfo {
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

/// Build the cache path for one candidate and ordered document corpus.
fn cache_path(
    data_dir: &Path,
    candidate: &Candidate,
    document_format: DocumentFormat,
    fingerprint: &str,
    backend: Backend,
) -> PathBuf {
    let format_part = match document_format {
        DocumentFormat::Production => String::new(),
        _ => format!("-{}", document_format.key()),
    };
    data_dir.join("bakeoff").join(format!(
        "{}{format_part}-{}-v{}-{fingerprint}.bin",
        candidate.key,
        backend.key(),
        CACHE_FORMAT_VERSION
    ))
}

/// Capture every model, prompt, document, and corpus field that defines a cache.
fn cache_metadata(
    candidate: &Candidate,
    cards: &[db::CardRow],
    corpus_fingerprint: &str,
    document_format: DocumentFormat,
    backend: Backend,
) -> CacheMetadata {
    CacheMetadata {
        format_version: CACHE_FORMAT_VERSION,
        candidate_key: candidate.key.to_string(),
        backend: backend.key().to_string(),
        model: model_name(candidate),
        model_fingerprint: None,
        dimension: candidate.dim,
        card_count: cards.len(),
        max_length: candidate.max_length,
        document_version: embed::DOC_VERSION,
        document_format: document_format.key().to_string(),
        document_prompt: candidate.document_prompt.to_string(),
        query_prompt: candidate.query_prompt.to_string(),
        pooling: candidate.pooling.to_string(),
        corpus_fingerprint: corpus_fingerprint.to_string(),
    }
}

/// Write a candidate vector cache through a temporary file and rename.
fn write_cache(path: &Path, metadata: &CacheMetadata, data: &[f32]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let header = serde_json::to_vec(metadata)?;
    let header_len = u32::try_from(header.len()).context("cache header is too large")?;
    let tmp = path.with_extension("bin.tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(&CACHE_MAGIC.to_le_bytes())?;
        writer.write_all(&header_len.to_le_bytes())?;
        writer.write_all(&header)?;
        for value in data {
            writer.write_all(&value.to_le_bytes())?;
        }
        writer.flush()?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Read a cache, returning no matrix when its identity is stale.
fn read_cache(path: &Path, expected: &CacheMetadata) -> anyhow::Result<Option<FlatMatrix>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("opening {}", path.display())),
    };
    let mut reader = std::io::BufReader::new(file);
    let mut fixed_header = [0u8; 8];
    reader
        .read_exact(&mut fixed_header)
        .context("reading cache header")?;
    let magic = u32::from_le_bytes(fixed_header[..4].try_into()?);
    anyhow::ensure!(
        magic == CACHE_MAGIC,
        "invalid cache magic in {}",
        path.display()
    );
    let header_len = u32::from_le_bytes(fixed_header[4..].try_into()?) as usize;
    anyhow::ensure!(
        header_len <= 16 * 1024,
        "cache metadata header is too large"
    );
    let mut header = vec![0u8; header_len];
    reader
        .read_exact(&mut header)
        .context("reading cache metadata")?;
    let metadata: CacheMetadata =
        serde_json::from_slice(&header).context("parsing cache metadata")?;
    if &metadata != expected {
        return Ok(None);
    }
    let value_count = metadata
        .card_count
        .checked_mul(metadata.dimension)
        .context("cache dimensions overflow")?;
    let mut raw = Vec::new();
    reader
        .read_to_end(&mut raw)
        .context("reading cached vectors")?;
    anyhow::ensure!(
        raw.len() == value_count * std::mem::size_of::<f32>(),
        "cache has {} vector bytes; expected {}",
        raw.len(),
        value_count * std::mem::size_of::<f32>()
    );
    let (chunks, remainder) = raw.as_chunks::<4>();
    anyhow::ensure!(
        remainder.is_empty(),
        "cache vector data has a partial value"
    );
    let values: Vec<f32> = chunks
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    anyhow::ensure!(
        values.iter().all(|value| value.is_finite()),
        "cache contains a non-finite vector value"
    );
    for row in values.chunks_exact(metadata.dimension) {
        let norm = row.iter().map(|value| value * value).sum::<f32>().sqrt();
        anyhow::ensure!(
            (norm - 1.0).abs() < 1e-3,
            "cache contains a vector row with norm {norm}"
        );
    }
    let names = vec![String::new(); metadata.card_count];
    Ok(Some(FlatMatrix::from_raw(
        names,
        metadata.dimension,
        values,
    )?))
}

/// Loaded model backends supported by the bake-off.
enum LoadedCandidateModel {
    /// Standard ONNX model.
    Onnx(Box<fastembed::TextEmbedding>),
    /// Core ML document encoder and matching CPU query encoder.
    #[cfg(target_os = "macos")]
    CoreMl {
        documents: Box<embed::coreml::CoreMlEmbedding>,
        queries: Box<fastembed::TextEmbedding>,
    },
}

impl LoadedCandidateModel {
    /// Embed a batch through the selected model backend.
    fn embed(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        match self {
            Self::Onnx(model) => {
                let mut vectors = model.embed(texts, Some(256))?;
                for vector in &mut vectors {
                    embed::normalize_row(vector);
                }
                Ok(vectors)
            }
            #[cfg(target_os = "macos")]
            Self::CoreMl { documents, .. } => documents.embed_documents(texts),
        }
    }

    /// Embed a single query using the same model weights as the documents.
    fn query(&mut self, text: &str) -> anyhow::Result<Vec<f32>> {
        match self {
            Self::Onnx(model) => {
                let mut vectors = model.embed([text], None)?;
                let mut vector = vectors.pop().context("model returned no query vector")?;
                embed::normalize_row(&mut vector);
                Ok(vector)
            }
            #[cfg(target_os = "macos")]
            Self::CoreMl { queries, .. } => {
                let mut vectors = queries.embed([text], None)?;
                let mut vector = vectors.pop().context("model returned no query vector")?;
                embed::normalize_row(&mut vector);
                Ok(vector)
            }
        }
    }

    /// Batch size used for document inference.
    fn batch_size(&self) -> usize {
        match self {
            Self::Onnx(_) => 256,
            #[cfg(target_os = "macos")]
            Self::CoreMl { documents, .. } => documents.batch_size(),
        }
    }

    /// Source-model hash for Core ML cache validation.
    fn model_fingerprint(&self) -> Option<&str> {
        match self {
            Self::Onnx(_) => None,
            #[cfg(target_os = "macos")]
            Self::CoreMl { documents, .. } => Some(documents.model_hash()),
        }
    }
}

/// Load a model with its candidate-specific backend and token limit.
fn load_candidate_model(
    candidate: &Candidate,
    models_dir: &Path,
    show_progress: bool,
    backend: Backend,
    profile_coreml: bool,
) -> anyhow::Result<LoadedCandidateModel> {
    #[cfg(not(target_os = "macos"))]
    let _ = profile_coreml;
    let CandidateModel::Onnx(model) = &candidate.model;
    if backend == Backend::CoreMl {
        #[cfg(target_os = "macos")]
        let documents = embed::coreml::CoreMlEmbedding::load(
            models_dir,
            model.clone(),
            candidate.max_length,
            profile_coreml,
        )?;
        let query_options = fastembed::InitOptions::new(model.clone())
            .with_cache_dir(models_dir.to_path_buf())
            .with_show_download_progress(show_progress)
            .with_max_length(candidate.max_length)
            .with_intra_threads(embed::INTRA_THREADS);
        let queries = fastembed::TextEmbedding::try_new(query_options)
            .context("initializing CPU query model")?;
        return Ok(LoadedCandidateModel::CoreMl {
            documents: Box::new(documents),
            queries: Box::new(queries),
        });
        #[cfg(not(target_os = "macos"))]
        anyhow::bail!("Core ML requires macOS");
    }
    let options = fastembed::InitOptions::new(model.clone())
        .with_cache_dir(models_dir.to_path_buf())
        .with_show_download_progress(show_progress)
        .with_max_length(candidate.max_length)
        .with_intra_threads(embed::INTRA_THREADS);
    fastembed::TextEmbedding::try_new(options)
        .context("initializing candidate ONNX model")
        .map(Box::new)
        .map(LoadedCandidateModel::Onnx)
}

/// Apply candidate-specific output processing and unit normalization.
fn normalize_embedding(vector: &mut [f32]) {
    embed::normalize_row(vector);
}

/// Load, invalidate, or rebuild one candidate's complete document matrix.
fn build_vectors(
    data_dir: &Path,
    candidate: &Candidate,
    document_format: DocumentFormat,
    model: &mut LoadedCandidateModel,
    cards: &[db::CardRow],
    docs: &[String],
    identity: VectorCacheIdentity<'_>,
) -> anyhow::Result<(FlatMatrix, Option<f64>, CacheMetadata)> {
    let path = cache_path(
        data_dir,
        candidate,
        document_format,
        identity.corpus_fingerprint,
        identity.backend,
    );
    let mut expected = cache_metadata(
        candidate,
        cards,
        identity.corpus_fingerprint,
        document_format,
        identity.backend,
    );
    expected.model_fingerprint = model.model_fingerprint().map(str::to_string);
    let cached = match read_cache(&path, &expected) {
        Ok(Some(mut matrix)) => {
            matrix.names = cards.iter().map(|card| card.name.clone()).collect();
            Some(matrix)
        }
        Ok(None) => None,
        Err(error) => {
            eprintln!(
                "[{}] invalid cache {}; rebuilding: {error:#}",
                candidate.key,
                path.display()
            );
            std::fs::remove_file(&path)
                .with_context(|| format!("removing invalid cache {}", path.display()))?;
            None
        }
    };
    if let Some(matrix) = cached {
        eprintln!(
            "[{}] loaded {} cached vectors",
            candidate.key,
            matrix.row_count()
        );
        return Ok((matrix, None, expected));
    }

    let started = Instant::now();
    let mut data = Vec::with_capacity(cards.len() * candidate.dim);
    for chunk in docs.chunks(256) {
        for vector in model.embed(chunk)? {
            anyhow::ensure!(
                vector.len() == candidate.dim,
                "{} returned {} dimensions; expected {}",
                candidate.key,
                vector.len(),
                candidate.dim
            );
            let mut vector = vector;
            normalize_embedding(&mut vector);
            data.extend_from_slice(&vector);
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let matrix = FlatMatrix::from_raw(
        cards.iter().map(|card| card.name.clone()).collect(),
        candidate.dim,
        data,
    )?;
    write_cache(&path, &expected, &matrix.data)?;
    Ok((matrix, Some(elapsed), expected))
}

/// Embed an expanded query with the model-specific query instruction.
fn embed_query(
    candidate: &Candidate,
    model: &mut LoadedCandidateModel,
    text: &str,
) -> anyhow::Result<Vec<f32>> {
    let prepared = query::prepare_query(text);
    let input = format!("{}{}", candidate.query_prompt, prepared.expanded_text);
    let mut vector = model.query(&input)?;
    anyhow::ensure!(
        vector.len() == candidate.dim,
        "{} returned {} query dimensions; expected {}",
        candidate.key,
        vector.len(),
        candidate.dim
    );
    normalize_embedding(&mut vector);
    Ok(vector)
}

/// Copy the ordered top-k names from one search leg.
fn names(hits: &[query::Hit]) -> Vec<String> {
    hits.iter()
        .take(CUTOFF)
        .map(|hit| hit.card.name.clone())
        .collect()
}

/// Require production BGE vectors to match the `query::run_search` output.
fn validate_bge_parity(
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
fn production_queries(
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
fn metric_mean<'a>(
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
fn duration_summary(samples: &[std::time::Duration]) -> anyhow::Result<DurationSummary> {
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
fn build_candidate_report(
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
        "RRF settings must be finite, non-negative, and not both leg weights zero"
    );
    anyhow::ensure!(
        settings.fts_overfetch_multiplier > 0
            && settings.fts_min_sql_rows > 0
            && settings.fts_max_rounds > 0,
        "FTS over-fetch settings must be positive"
    );
    Ok(())
}

/// Measure model load and representative document throughput without scoring queries.
fn run_throughput_pilot(
    selected: &[&Candidate],
    formats: &[DocumentFormat],
    cards: &[db::CardRow],
    tag_index: &tags::TagIndex,
    models_dir: &Path,
    requested_backend: Backend,
    save: Option<&Path>,
) -> anyhow::Result<()> {
    if requested_backend == Backend::CoreMl {
        configure_coreml_diagnostics();
    }
    let effective_backend = match requested_backend {
        Backend::Auto | Backend::Cpu => Backend::Cpu,
        Backend::CoreMl => Backend::CoreMl,
    };
    let mut reports = Vec::new();
    for candidate in selected {
        let load_started = Instant::now();
        let mut model =
            match load_candidate_model(candidate, models_dir, true, effective_backend, true) {
                Ok(model) => model,
                Err(error) => {
                    reports.push(serde_json::json!({
                        "candidate": candidate.key,
                        "status": "failed",
                        "requested_backend": requested_backend.key(),
                        "effective_backend": effective_backend.key(),
                        "reason": format!("{error:#}"),
                    }));
                    continue;
                }
            };
        let model_load_secs = load_started.elapsed().as_secs_f64();
        let mut format_reports = Vec::new();
        for format in formats {
            let mut docs: Vec<String> = cards
                .iter()
                .map(|card| candidate_document(candidate, *format, card, tag_index))
                .collect();
            docs.sort_by_key(String::len);
            let sample_count = docs.len().min(96);
            let mut indices = Vec::with_capacity(sample_count);
            for rank in 0..sample_count {
                let index = rank * docs.len() / sample_count.max(1);
                indices.push(index.min(docs.len().saturating_sub(1)));
            }
            let sample: Vec<String> = indices.iter().map(|index| docs[*index].clone()).collect();
            let mut output_count = 0usize;
            let started = Instant::now();
            for batch in sample.chunks(32) {
                let vectors = match model.embed(batch) {
                    Ok(vectors) => vectors,
                    Err(error) => {
                        format_reports.push(serde_json::json!({
                            "format": format.key(),
                            "status": "failed",
                            "reason": format!("{error:#}"),
                        }));
                        output_count = 0;
                        break;
                    }
                };
                anyhow::ensure!(
                    vectors.iter().all(|vector| vector.len() == candidate.dim),
                    "{} returned an unexpected vector dimension in the throughput pilot",
                    candidate.key
                );
                output_count += vectors.len();
            }
            let elapsed = started.elapsed().as_secs_f64();
            if output_count > 0 {
                let docs_per_sec = output_count as f64 / elapsed;
                format_reports.push(serde_json::json!({
                    "format": format.key(),
                    "status": "ok",
                    "sampled_cards": output_count,
                    "sampled_document_length_bytes": {
                        "min": sample.iter().map(String::len).min().unwrap_or(0),
                        "median": sample.iter().map(String::len).nth(sample.len() / 2).unwrap_or(0),
                        "max": sample.iter().map(String::len).max().unwrap_or(0),
                    },
                    "warm_sample_secs": elapsed,
                    "documents_per_second": docs_per_sec,
                    "estimated_full_corpus_secs": cards.len() as f64 / docs_per_sec,
                    "estimated_under_five_minutes": cards.len() as f64 / docs_per_sec < 300.0,
                }));
            }
        }
        reports.push(serde_json::json!({
            "candidate": candidate.key,
            "model": model_name(candidate),
            "model_fingerprint": model.model_fingerprint(),
            "status": "ok",
            "requested_backend": requested_backend.key(),
            "effective_backend": effective_backend.key(),
            "query_backend": "cpu",
            "dimension": candidate.dim,
            "max_length": candidate.max_length,
            "intra_threads": embed::INTRA_THREADS,
            "batch_size": if effective_backend == Backend::Cpu { 32 } else { model.batch_size() },
            "model_load_secs": model_load_secs,
            "layouts": format_reports,
        }));
    }
    let report = serde_json::json!({
        "pilot_version": 1,
        "corpus_cards": cards.len(),
        "hardware": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        "requested_backend": requested_backend.key(),
        "candidates": reports,
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    println!("{encoded}");
    if let Some(path) = save {
        std::fs::write(path, &encoded)
            .with_context(|| format!("writing throughput pilot to {}", path.display()))?;
    }
    Ok(())
}

/// Print ONNX Runtime diagnostics that identify Core ML provider placement.
fn configure_coreml_diagnostics() {
    ort::init()
        .with_logger(std::sync::Arc::new(
            |level: ort::logging::LogLevel,
             category: &str,
             _id: &str,
             _location: &str,
             message: &str| {
                let lower = message.to_ascii_lowercase();
                if lower.contains("coreml")
                    || lower.contains("execution provider")
                    || lower.contains("compute plan")
                    || lower.contains("assigned")
                    || lower.contains("partition")
                {
                    eprintln!("{level:?} [{category}] {message}");
                }
            },
        ))
        .commit();
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
        "no completed store at {} (run 'stm setup' in release mode first)",
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
    let production_docs: Vec<String> = cards
        .iter()
        .map(|card| embed::doc_for_row(card, &tag_index))
        .collect();
    let fingerprint = common::fingerprint_strings(
        cards
            .iter()
            .zip(&production_docs)
            .flat_map(|(card, doc)| [card.name.as_str(), doc.as_str()]),
    );
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

/// Print a compact model comparison and parity summary.
fn print_human(report: &BakeoffReport) {
    println!(
        "Embedding bake-off · {} queries · {} cards · labels {} · current CLI nDCG {:.3}",
        report.corpus.golden_queries,
        report.corpus.cards,
        report.label_version,
        report.production_reference.metrics.ndcg
    );
    println!("Filtered queries are held out. Behavior probes are excluded from relevance means.");
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

#[cfg(test)]
#[path = "tests/query_bakeoff_tests.rs"]
mod tests;
