use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use anyhow::Context as _;
use fastembed::ModelTrait as _;
#[cfg(target_os = "macos")]
use seizethemana::embed::DocumentEmbedder as _;
use seizethemana::{db, embed, query, tags};

use super::document::{DocumentFormat, candidate_document};

pub(super) const BGE_QUERY_PROMPT: &str =
    "Represent this sentence for searching relevant passages: ";

/// ONNX Runtime provider requested for a bake-off run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Backend {
    /// Select the evaluator's default backend.
    Auto,
    /// Use the CPU execution provider.
    Cpu,
    /// Run fixed-shape ML Program document batches on Core ML.
    CoreMl,
}

impl Backend {
    /// Stable provider key for reports and cache identities.
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::CoreMl => "coreml",
        }
    }
}

/// Requested and selected providers for one model run.
#[derive(Debug, Clone, Copy)]
pub(super) struct BackendSelection {
    pub(super) requested: Backend,
    pub(super) effective: Backend,
}

/// Embedding backend used by one bake-off candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CandidateModel {
    /// ONNX model in fastembed's standard text registry.
    Onnx(fastembed::EmbeddingModel),
}

/// One model and its documented retrieval input contract.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    /// Stable CLI key.
    pub(super) key: &'static str,
    /// fastembed model implementation and registry entry.
    pub(super) model: CandidateModel,
    /// Expected vector dimension.
    pub(super) dim: usize,
    /// Token limit used for both documents and queries.
    pub(super) max_length: usize,
    /// Maximum sequence length supported by the model card.
    pub(super) model_max_length: usize,
    /// Model-specific document instruction, applied by `candidate_document`.
    pub(super) document_prompt: &'static str,
    /// Prefix applied to every expanded search query.
    pub(super) query_prompt: &'static str,
    /// Pooling used by the fastembed registry for this model.
    pub(super) pooling: &'static str,
    /// Primary model-card URL used to verify the retrieval contract.
    pub(super) model_card: &'static str,
}

pub(super) const CANDIDATES: &[Candidate] = &[
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

/// Return the canonical model identifier from fastembed's model registry.
pub(super) fn model_name(candidate: &Candidate) -> String {
    match &candidate.model {
        CandidateModel::Onnx(model) => fastembed::EmbeddingModel::get_model_info(model)
            .map(|info| info.model_code.clone())
            .unwrap_or_else(|| model.to_string()),
    }
}

/// Select candidate keys in registry order or from `--only` input order.
pub(super) fn select_candidates(only: Option<&str>) -> anyhow::Result<Vec<&'static Candidate>> {
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

/// Loaded model backends supported by the bake-off.
pub(super) enum LoadedCandidateModel {
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
    pub(super) fn embed(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
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
    pub(super) fn batch_size(&self) -> usize {
        match self {
            Self::Onnx(_) => 256,
            #[cfg(target_os = "macos")]
            Self::CoreMl { documents, .. } => documents.batch_size(),
        }
    }

    /// Source-model hash for Core ML cache validation.
    pub(super) fn model_fingerprint(&self) -> Option<&str> {
        match self {
            Self::Onnx(_) => None,
            #[cfg(target_os = "macos")]
            Self::CoreMl { documents, .. } => Some(documents.model_hash()),
        }
    }
}

/// Load a model with its candidate-specific backend and token limit.
pub(super) fn load_candidate_model(
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
        {
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
        }
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
pub(super) fn normalize_embedding(vector: &mut [f32]) {
    embed::normalize_row(vector);
}

/// Embed an expanded query with the model-specific query instruction.
pub(super) fn embed_query(
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

/// Measure model load and representative document throughput without scoring queries.
pub(super) fn run_throughput_pilot(
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
