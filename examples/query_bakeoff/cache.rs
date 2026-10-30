//! Current-format vector caches for repeatable search evaluations.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context as _;
use seizethemana::query::VectorRowProvider as _;
use seizethemana::{db, embed, query};
use serde::{Deserialize, Serialize};

use super::document::DocumentFormat;
use super::model::{Backend, Candidate, LoadedCandidateModel, model_name, normalize_embedding};

const CACHE_MAGIC: u32 = 0x5354_4D42;
pub(super) const CACHE_FORMAT_VERSION: u32 = 3;

/// Cache identity fields that vary by backend and ordered corpus.
#[derive(Debug, Clone, Copy)]
pub(super) struct VectorCacheIdentity<'a> {
    pub(super) corpus_fingerprint: &'a str,
    pub(super) backend: Backend,
}

/// Fingerprinted metadata that must match before a vector cache can be used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct CacheMetadata {
    pub(super) format_version: u32,
    pub(super) candidate_key: String,
    pub(super) backend: String,
    pub(super) model: String,
    pub(super) model_fingerprint: Option<String>,
    pub(super) dimension: usize,
    pub(super) card_count: usize,
    pub(super) max_length: usize,
    pub(super) document_version: u32,
    pub(super) document_format: String,
    pub(super) document_prompt: String,
    pub(super) query_prompt: String,
    pub(super) pooling: String,
    pub(super) corpus_fingerprint: String,
}

/// Row-major candidate embeddings aligned with the names in the card table.
#[derive(Debug)]
pub(super) struct FlatMatrix {
    pub(super) names: Vec<String>,
    dim: usize,
    data: Vec<f32>,
}

impl FlatMatrix {
    /// Validate and wrap vectors with their card names and row dimension.
    pub(super) fn from_raw(names: Vec<String>, dim: usize, data: Vec<f32>) -> anyhow::Result<Self> {
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
pub(super) fn cache_metadata(
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
pub(super) fn write_cache(
    path: &Path,
    metadata: &CacheMetadata,
    data: &[f32],
) -> anyhow::Result<()> {
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
pub(super) fn read_cache(
    path: &Path,
    expected: &CacheMetadata,
) -> anyhow::Result<Option<FlatMatrix>> {
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
        "cache vector data ends with an incomplete f32 value"
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
            "cache vector row has norm {norm}; expected approximately 1"
        );
    }
    let names = vec![String::new(); metadata.card_count];
    Ok(Some(FlatMatrix::from_raw(
        names,
        metadata.dimension,
        values,
    )?))
}

/// Load, invalidate, or rebuild one candidate's complete document matrix.
pub(super) fn build_vectors(
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
