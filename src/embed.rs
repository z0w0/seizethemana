//! Local text embeddings and a flat on-disk vector store.
//!
//! bge-small-en-v1.5 embeds cards into unit-length vectors with 384 values.
//! At about 32,000 cards, a measured cosine scan of the flat f32 matrix took
//! less than 50 ms. The store does not use a nearest-neighbor index.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::Write;

/// Core ML document embeddings on macOS.
#[cfg(target_os = "macos")]
pub mod coreml;

/// Cast raw little-endian bytes to `f32`s (vectors.bin layout).
mod f32slice {
    /// Interpret a byte slice as little-endian f32 values.
    pub fn cast(bytes: &[u8]) -> &[f32] {
        bytemuck::cast_slice(bytes)
    }

    /// Reinterpret `f32`s as little-endian bytes for `vectors.bin` writes.
    pub fn bytes(values: &[f32]) -> &[u8] {
        bytemuck::cast_slice(values)
    }
}

/// Full-precision model selected for card search and recorded in `status.json`.
pub const MODEL: fastembed::EmbeddingModel = fastembed::EmbeddingModel::BGESmallENV15;
/// Stable status-file identity for the selected embedding model.
pub const MODEL_NAME: &str = "BAAI/bge-small-en-v1.5";

/// Maximum tokens per embedded text.
///
/// Card names, costs, types, and Oracle text usually fit within 128 tokens.
/// Longer documents lose text at the end. In the recorded benchmark, 128 tokens
/// was 1.5 times faster than 256 with 100% top-20 overlap on representative
/// queries. At 64 tokens, embedding was faster but results changed by about 11%.
pub const MAX_LENGTH: usize = 128;

/// Number of values in each model embedding.
pub const DIM: usize = 384;

/// BGE v1.5 query-side instruction. Documents are embedded raw; queries are
/// prefixed per the model card for asymmetric retrieval.
pub const QUERY_INSTRUCTION: &str = "Represent this sentence for searching relevant passages: ";

/// Index metadata stored inside `status.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMeta {
    /// Name of the model used to build the embeddings.
    pub model: String,
    /// Vector dimension (columns of the matrix).
    pub dim: usize,
    /// Card names, in matrix row order.
    pub names: Vec<String>,
}

impl VectorMeta {
    /// Row index of a card name, if present.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
}

/// Row-major f32 matrix, unit-normalized rows (`meta.names.len() * dim`).
/// Backed by either an owned matrix (setup/sync writes) or a memory map of
/// `vectors.bin` (reads): a query process touches only the pages its scan
/// needs, so load cost drops from a 51MB heap read to one mapping.
#[derive(Debug)]
pub struct VectorStore {
    /// Index metadata (model, dim, card names in row order).
    pub meta: VectorMeta,
    /// Row-major f32 matrix, unit-normalized rows (`meta.names.len() * dim`).
    pub vectors: VectorMatrix,
}

/// The matrix behind a store: owned heap data or a file mapping.
#[derive(Debug)]
pub enum VectorMatrix {
    /// Owned matrix, used while building (setup, sync updates).
    Owned(Vec<f32>),
    /// Read-only map of `vectors.bin`; little-endian f32s, same layout.
    Mapped(memmap2::Mmap),
}

impl VectorMatrix {
    /// Matrix length in f32 values.
    fn len(&self) -> usize {
        match self {
            VectorMatrix::Owned(v) => v.len(),
            VectorMatrix::Mapped(m) => m.len() / 4,
        }
    }

    /// Slice of `count` f32 values starting at `start`.
    fn slice(&self, start: usize, count: usize) -> &[f32] {
        match self {
            VectorMatrix::Owned(v) => &v[start..start + count],
            VectorMatrix::Mapped(m) => f32slice::cast(&m[start * 4..(start + count) * 4]),
        }
    }

    /// Mutable slice of an owned matrix. Panics for read-only mapped matrices.
    fn slice_mut(&mut self, start: usize, count: usize) -> &mut [f32] {
        match self {
            VectorMatrix::Owned(v) => &mut v[start..start + count],
            VectorMatrix::Mapped(_) => panic!("vector store is read-only when memory-mapped"),
        }
    }

    /// Append one value to the owned matrix; panics when mapped (mutating
    /// code paths always build on `Owned`).
    fn push_value(&mut self, value: f32) {
        match self {
            VectorMatrix::Owned(v) => v.push(value),
            VectorMatrix::Mapped(_) => panic!("vector store is read-only when memory-mapped"),
        }
    }
}

/// Version of the document layout [`build_doc`] produces.
///
/// Stored in `status.json` as `doc_version`. When the layout changes, bump
/// this constant; the next sync re-embeds every card whose stored version
/// lags behind.
pub const DOC_VERSION: u32 = 2;

/// Build the embedding document for a card.
///
/// Natural-language layout selected by the search bake-off:
///
/// ```text
/// Lightning Bolt. Mana cost {R}. Type Instant. Keywords none. Colors R.
/// Tags removal, burn. P/T 2/2. Rules text Deal 3 damage to any target.
/// ```
///
/// The layout keeps card identity and structured facts near rules text. Tags
/// add role vocabulary that oracle text may not contain.
#[allow(clippy::too_many_arguments)]
pub fn build_doc(
    name: &str,
    mana_cost: &str,
    type_line: &str,
    keywords: &str,
    colors: &str,
    power: Option<&str>,
    toughness: Option<&str>,
    loyalty: Option<&str>,
    oracle_text: &str,
    tags_line: &str,
) -> String {
    let parse = |json: &str| -> Vec<String> {
        serde_json::from_str::<Vec<String>>(json).unwrap_or_default()
    };
    let keyword_line = match parse(keywords) {
        list if list.is_empty() => "Keywords: none".to_string(),
        list => format!("Keywords: {}", list.join(", ")),
    };
    let colors = parse(colors)
        .iter()
        .filter_map(|c| c.chars().next())
        .collect::<String>();
    let color_line = if colors.is_empty() {
        "Colors: colorless".to_string()
    } else {
        format!("Colors: {colors}")
    };
    let mut fields = vec![
        format!("Name: {name}"),
        format!("Mana cost: {mana_cost}"),
        format!("Type: {type_line}"),
        keyword_line,
        color_line,
        format!(
            "Tags: {}",
            tags_line.strip_prefix("Tags: ").unwrap_or("none")
        ),
    ];
    if let (Some(p), Some(t)) = (power, toughness) {
        fields.push(format!("P/T: {p}/{t}"));
    } else if let Some(l) = loyalty {
        fields.push(format!("Loyalty: {l}"));
    }
    fields.push(format!("Rules text: {oracle_text}"));
    let details = fields
        .into_iter()
        .skip(1)
        .map(|field| {
            field
                .replace(": ", " ")
                .trim_end_matches(['.', ' '])
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(". ");
    format!("{name}. {details}.")
}

/// Build a card row's embedding document with tags from `index`.
pub fn doc_for_row(row: &crate::db::CardRow, index: &crate::tags::TagIndex) -> String {
    build_doc(
        &row.name,
        &row.mana_cost,
        &row.type_line,
        &row.keywords,
        &row.colors,
        row.power.as_deref(),
        row.toughness.as_deref(),
        row.loyalty.as_deref(),
        &row.oracle_text,
        &index.doc_tags_line(&row.oracle_id),
    )
}

/// ONNX Runtime threads used within one operation.
///
/// Eight threads were fastest on a 12-core machine: 188 documents per second,
/// compared with 139 at four threads. Higher counts were slightly slower.
pub const INTRA_THREADS: usize = 8;

/// Embeds batches of card documents for the vector index.
pub trait DocumentEmbedder {
    /// Embed a batch of documents.
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

impl DocumentEmbedder for fastembed::TextEmbedding {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.embed(texts, Some(256)).context("embedding texts")
    }
}

/// Load the embedding model, cached under `cache_dir`.
///
/// macOS uses Core ML for document batches. Other platforms use CPU inference.
/// Query embeddings use the CPU encoder on every platform.
///
/// # Errors
/// Propagates model load/download failures.
pub fn load_model(
    cache_dir: &std::path::Path,
    show_progress: bool,
) -> anyhow::Result<Box<dyn DocumentEmbedder>> {
    #[cfg(target_os = "macos")]
    {
        coreml::CoreMlEmbedding::load_with_download_progress(
            cache_dir,
            MODEL,
            MAX_LENGTH,
            false,
            show_progress,
        )
        .map(|model| Box::new(model) as Box<dyn DocumentEmbedder>)
    }
    #[cfg(not(target_os = "macos"))]
    {
        load_query_model(cache_dir, show_progress)
            .map(|model| Box::new(model) as Box<dyn DocumentEmbedder>)
    }
}

/// Load the CPU query encoder with the same full-precision weights as documents.
///
/// # Errors
/// Propagates model load or download failures.
pub fn load_query_model(
    cache_dir: &std::path::Path,
    show_progress: bool,
) -> anyhow::Result<fastembed::TextEmbedding> {
    let options = fastembed::InitOptions::new(MODEL)
        .with_cache_dir(cache_dir.to_path_buf())
        .with_show_download_progress(show_progress)
        .with_max_length(MAX_LENGTH)
        .with_intra_threads(INTRA_THREADS);
    fastembed::TextEmbedding::try_new(options)
        .context("initializing CPU query model (first run downloads the full-precision model)")
}

/// Embed texts, normalizing each vector to unit length.
///
/// Unit-length vectors let the search scan use dot products for cosine similarity.
///
/// # Errors
/// Propagates embedding failures.
pub fn embed_texts(
    model: &mut dyn DocumentEmbedder,
    texts: &[String],
) -> anyhow::Result<Vec<Vec<f32>>> {
    let mut out = model.embed_documents(texts)?;
    for v in &mut out {
        normalize(v);
    }
    Ok(out)
}

/// Scale a nonzero vector to unit length.
fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v {
            *x /= norm;
        }
    }
}

/// Normalize a matrix row in place (sync updates overwrite vector rows
/// without going through [`VectorStore::push`]).
pub fn normalize_row(row: &mut [f32]) {
    normalize(row)
}

impl VectorStore {
    /// Empty store with the standard metadata.
    pub fn new() -> Self {
        Self {
            meta: VectorMeta {
                model: model_name(),
                dim: DIM,
                names: Vec::new(),
            },
            vectors: VectorMatrix::Owned(Vec::new()),
        }
    }

    /// Append one unit-normalized vector with its card name.
    ///
    /// # Errors
    /// Fails if the vector dimension does not match [`DIM`].
    pub fn push(&mut self, name: &str, mut vector: Vec<f32>) -> anyhow::Result<()> {
        anyhow::ensure!(
            vector.len() == DIM,
            "expected {DIM}-dim vector, got {}",
            vector.len()
        );
        normalize(&mut vector);
        self.meta.names.push(name.to_string());
        for value in &vector {
            self.vectors.push_value(*value);
        }
        Ok(())
    }

    /// Number of stored vectors.
    pub fn len(&self) -> usize {
        self.meta.names.len()
    }

    /// One row (unit-normalized vector) by index.
    pub fn row(&self, index: usize) -> &[f32] {
        self.vectors.slice(index * DIM, DIM)
    }

    /// Mutable row for in-place updates (sync overwrite path).
    pub fn row_mut(&mut self, index: usize) -> &mut [f32] {
        self.vectors.slice_mut(index * DIM, DIM)
    }

    /// True when no vectors are stored.
    pub fn is_empty(&self) -> bool {
        self.meta.names.is_empty()
    }

    /// Write `vectors.bin` under `dir` atomically.
    ///
    /// Writes little-endian f32 values to a temporary file, then renames it.
    /// An interrupted write leaves the previous vector file intact.
    /// The caller writes matching metadata to `status.json` separately.
    ///
    /// # Errors
    /// Propagates filesystem failures.
    pub fn save_vectors(&self, dir: &std::path::Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        let vectors_path = dir.join("vectors.bin");
        let tmp = dir.join("vectors.bin.tmp");
        {
            let file = std::fs::File::create(&tmp)?;
            let mut writer = std::io::BufWriter::new(file);
            let all = self.vectors.slice(0, self.vectors.len());
            writer.write_all(f32slice::bytes(all))?;
            writer.flush()?;
        }
        std::fs::rename(&tmp, &vectors_path)?;
        Ok(())
    }

    /// Rebuild a store from `status.json` + `vectors.bin` on disk.
    ///
    /// Finishes any ready sync publication before reading metadata. Recovery
    /// can replace both index files and remove the journal. It does not load
    /// a model or refresh card data.
    /// Holds the store lock through recovery, metadata checks, and mapping.
    /// This protects only the index load. Combined SQL and index reads need
    /// one caller-held guard for the full read.
    ///
    /// Maps `vectors.bin` read-only, avoiding a 51 MB heap copy for the measured
    /// corpus. The operating system loads file pages as the scan needs them.
    /// Callers that update vectors must use [`VectorStore::load_owned`].
    /// Attempts to change a mapped matrix panic.
    ///
    /// # Errors
    /// Fails on missing/corrupt files, dimension mismatch, or a vector count
    /// that does not match the name count. Propagates lock and recovery failures.
    pub fn load(dir: &std::path::Path) -> anyhow::Result<Self> {
        let lock = crate::sync::store_lock::StoreLock::acquire(&crate::paths::Paths::new(
            dir.to_path_buf(),
        ))?;
        Self::load_locked(&lock)
    }

    /// Load a mapped snapshot without reacquiring the caller's store lock.
    /// Keep the guard held while reading related SQL rows and status metadata.
    pub(crate) fn load_locked(lock: &crate::sync::store_lock::StoreLock) -> anyhow::Result<Self> {
        let (meta, vectors) = Self::read_store(lock)?;
        Ok(Self {
            meta,
            vectors: VectorMatrix::Mapped(vectors),
        })
    }

    /// Load an owned, mutable matrix for sync updates.
    ///
    /// Copies the mapped bytes into heap memory once per sync; the copy is
    /// what makes `row_mut`/`push` legal on a loaded store.
    /// Like [`Self::load`], this can write files to finish a ready sync publication.
    ///
    /// # Errors
    /// Same as [`Self::load`].
    pub fn load_owned(dir: &std::path::Path) -> anyhow::Result<Self> {
        let lock = crate::sync::store_lock::StoreLock::acquire(&crate::paths::Paths::new(
            dir.to_path_buf(),
        ))?;
        Self::load_owned_locked(&lock)
    }

    /// Load a mutable snapshot without reacquiring the caller's store lock.
    pub(crate) fn load_owned_locked(
        lock: &crate::sync::store_lock::StoreLock,
    ) -> anyhow::Result<Self> {
        let (meta, vectors) = Self::read_store(lock)?;
        Ok(Self {
            meta,
            vectors: VectorMatrix::Owned(f32slice::cast(&vectors).to_vec()),
        })
    }

    /// Finish a ready sync publication, then read and validate the index files.
    ///
    /// # Errors
    /// Fails on missing/corrupt files, dimension mismatch, or a vector count
    /// that does not match the name count. Propagates lock and recovery failures.
    fn read_store(
        lock: &crate::sync::store_lock::StoreLock,
    ) -> anyhow::Result<(VectorMeta, memmap2::Mmap)> {
        crate::sync::recovery::recover_publication(lock)?;
        let dir = lock.paths().root();
        let status = crate::paths::Status::read(&dir.join("status.json"))?;
        let meta = VectorMeta {
            model: status.model,
            dim: status.dim,
            names: status.names,
        };
        anyhow::ensure!(
            meta.dim == DIM,
            "status.json records dimension {}, but this build expects {DIM}; \
             rebuild the index with 'stm sync'",
            meta.dim
        );
        let vectors_path = dir.join("vectors.bin");
        let file = std::fs::File::open(&vectors_path)
            .with_context(|| format!("reading {}", vectors_path.display()))?;
        let meta_len = file
            .metadata()
            .with_context(|| format!("reading {}", vectors_path.display()))?
            .len() as usize;
        anyhow::ensure!(
            meta_len.is_multiple_of(DIM * 4),
            "vectors.bin size {} is not a multiple of {DIM}*4 bytes",
            meta_len
        );
        let count = meta_len / (DIM * 4);
        anyhow::ensure!(
            count == meta.names.len(),
            "vectors.bin holds {count} vectors but status.json lists {} names",
            meta.names.len()
        );
        // Map the validated file read-only. It must not be truncated while mapped.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Ok((meta, mmap))
    }

    /// Embed one query string (with the BGE query instruction) into a
    /// normalized vector ready for the dot-product scan in `run_search`.
    ///
    /// # Errors
    /// Propagates embedding failures.
    pub fn embed_query(
        &self,
        model: &mut fastembed::TextEmbedding,
        query: &str,
    ) -> anyhow::Result<Vec<f32>> {
        let mut vectors = model
            .embed([format!("{QUERY_INSTRUCTION}{query}")], None)
            .context("embedding query")?;
        let mut vector = vectors
            .pop()
            .context("CPU model returned no query embedding")?;
        vector.truncate(DIM);
        normalize(&mut vector);
        Ok(vector)
    }
}

impl Default for VectorStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Model name stored in the vector metadata.
fn model_name() -> String {
    MODEL_NAME.to_string()
}

/// Offline loading after an interrupted sync publication.
#[cfg(test)]
#[path = "tests/embed_recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_builder_layout() {
        let doc = build_doc(
            "Lightning Bolt",
            "{R}",
            "Instant",
            "[]",
            r#"["R"]"#,
            None,
            None,
            None,
            "Deal 3 damage.",
            "",
        );
        assert_eq!(
            doc,
            "Lightning Bolt. Mana cost {R}. Type Instant. Keywords none. Colors R. Tags none. Rules text Deal 3 damage."
        );
        // Role tags stay with the other card facts.
        let doc = build_doc(
            "Lightning Bolt",
            "{R}",
            "Instant",
            "[]",
            r#"["R"]"#,
            None,
            None,
            None,
            "Deal 3 damage.",
            "Tags: burn, removal",
        );
        assert!(
            doc.contains("Colors R. Tags burn, removal. Rules text Deal 3"),
            "{doc}"
        );
    }

    #[test]
    fn doc_builder_enriched_lines() {
        let doc = build_doc(
            "Test Card",
            "{1}{G}",
            "Creature — Human",
            r#"["Haste","Vigilance"]"#,
            r#"["G"]"#,
            Some("2"),
            Some("2"),
            None,
            "Haste.",
            "",
        );
        assert!(doc.contains("Keywords Haste, Vigilance"));
        assert!(doc.contains("Colors G"));
        assert!(doc.contains("P/T 2/2"));
        // Loyalty replaces P/T for planeswalkers.
        let doc = build_doc(
            "Jace",
            "{2}{U}",
            "Legendary Planeswalker — Jace",
            "[]",
            r#"["U"]"#,
            None,
            None,
            Some("4"),
            "+1: Draw.",
            "",
        );
        assert!(doc.contains("Loyalty 4"));
        assert!(!doc.contains("P/T"));
        // Colorless and empty keywords render explicitly.
        let doc = build_doc(
            "Wastes",
            "",
            "Basic Land — Wastes",
            "[]",
            "[]",
            None,
            None,
            None,
            "",
            "",
        );
        assert!(doc.contains("Keywords none"));
        assert!(doc.contains("Colors colorless"));
        // Malformed JSON columns use "none" and "colorless".
        let doc = build_doc(
            "X", "", "Land", "not json", "also not", None, None, None, "", "",
        );
        assert!(doc.contains("Keywords none"));
        assert!(doc.contains("Colors colorless"));
    }

    #[test]
    fn doc_builder_empty_fields() {
        let doc = build_doc(
            "Forest",
            "",
            "Basic Land — Forest",
            "[]",
            r#"["G"]"#,
            None,
            None,
            None,
            "",
            "",
        );
        assert!(
            doc.starts_with("Forest. Mana cost. Type Basic Land — Forest. Keywords none"),
            "{doc}"
        );
    }

    #[test]
    fn push_normalizes_and_validates_dim() {
        let mut store = VectorStore::new();
        let v: Vec<f32> = vec![3.0; DIM];
        store.push("A", v).expect("push");
        let norm: f32 = store.row(0).iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm was {norm}");
        assert_eq!(store.len(), 1);
        assert!(store.push("B", vec![1.0, 2.0]).is_err());
    }

    #[test]
    fn rows_score_by_dot_product() {
        // The production scan (query.rs `run_search`) scores rows by dot
        // product and keeps the top-N. Pin the scoring semantics here.
        let mut store = VectorStore::new();
        let mut a = vec![0.0; DIM];
        a[0] = 1.0;
        let mut b = vec![0.0; DIM];
        b[0] = 0.6;
        b[1] = 0.8;
        let mut c = vec![0.0; DIM];
        c[1] = 1.0;
        store.push("a", a).unwrap();
        store.push("b", b).unwrap();
        store.push("c", c).unwrap();

        let mut q = vec![0.0; DIM];
        q[0] = 1.0; // closest to "a", then "b", never "c"
        let scores: Vec<(usize, f32)> = (0..store.len())
            .map(|i| {
                let row = store.row(i);
                let score: f32 = q.iter().zip(row).map(|(x, y)| x * y).sum();
                (i, score)
            })
            .collect();
        let mut top = scores;
        top.sort_unstable_by(|x, y| y.1.total_cmp(&x.1));
        top.truncate(2);
        assert_eq!(top[0].0, store.meta.index_of("a").unwrap());
        assert_eq!(top[1].0, store.meta.index_of("b").unwrap());
        assert!((top[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = VectorStore::new();
        let v: Vec<f32> = (0..DIM).map(|i| (i as f32).sin()).collect();
        store.push("Bolt", v).unwrap();
        store.save_vectors(tmp.path()).expect("save");
        write_status(&store, tmp.path());

        let loaded = VectorStore::load(tmp.path()).expect("load");
        assert_eq!(loaded.meta.names, vec!["Bolt"]);
        assert_eq!(loaded.vectors.len(), DIM);
        assert!(
            loaded
                .row(0)
                .iter()
                .zip(store.row(0).iter())
                .all(|(x, y)| (x - y).abs() < 1e-6)
        );
        assert_eq!(loaded.meta.model, "BAAI/bge-small-en-v1.5");
        assert_eq!(loaded.meta.dim, DIM);
    }

    /// Sync mutates a loaded store; `load_owned` must permit it while a
    /// mapped `load` would panic.
    #[test]
    fn load_owned_permits_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = VectorStore::new();
        store
            .push("Bolt", (0..DIM).map(|i| i as f32).collect())
            .unwrap();
        store.save_vectors(tmp.path()).expect("save");
        write_status(&store, tmp.path());

        let mut owned = VectorStore::load_owned(tmp.path()).expect("load owned");
        owned.row_mut(0).copy_from_slice(&vec![1.0; DIM]);
        crate::embed::normalize_row(owned.row_mut(0));
        owned
            .push("New", (0..DIM).map(|_| 0.5).collect())
            .expect("push on owned");
        assert_eq!(owned.len(), 2);
        assert_eq!(owned.meta.names, vec!["Bolt", "New"]);
    }

    /// A combined reader must reuse its guard instead of taking the lock twice.
    #[test]
    fn load_locked_uses_callers_guard_for_a_mapped_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
        let store = status_for(&["Bolt"]);
        store.save_vectors(paths.root()).unwrap();
        write_status(&store, paths.root());
        let lock = crate::sync::store_lock::StoreLock::acquire(&paths).unwrap();
        let loaded = VectorStore::load_locked(&lock).unwrap();
        assert!(matches!(&loaded.vectors, VectorMatrix::Mapped(_)));
        assert_eq!(loaded.meta.names, ["Bolt"]);
        let contender = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(paths.root().join("store.lock"))
            .unwrap();
        let error = fs2::FileExt::try_lock_exclusive(&contender).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    }

    /// Write the status.json that marks the store ready, matching what
    /// setup produces for a fully embedded store.
    fn write_status(store: &VectorStore, dir: &std::path::Path) {
        crate::paths::Status {
            setup_complete: true,
            ingested_cards: store.len(),
            embedded_cards: store.len(),
            model: store.meta.model.clone(),
            dim: store.meta.dim,
            names: store.meta.names.clone(),
            scryfall_synced_at: String::new(),
            doc_version: 0,
            combos_synced_at: String::new(),
        }
        .write(&dir.join("status.json"))
        .unwrap();
    }

    #[test]
    fn load_rejects_wrong_dim_status() {
        let tmp = tempfile::tempdir().unwrap();
        let store = status_for(&["x"]);
        store.save_vectors(tmp.path()).unwrap();
        // Reject a stored dimension that would make vector row offsets invalid.
        crate::paths::Status {
            setup_complete: true,
            ingested_cards: 1,
            embedded_cards: 1,
            model: store.meta.model.clone(),
            dim: 768,
            names: store.meta.names.clone(),
            scryfall_synced_at: String::new(),
            doc_version: crate::embed::DOC_VERSION,
            combos_synced_at: String::new(),
        }
        .write(&tmp.path().join("status.json"))
        .unwrap();
        let err = VectorStore::load(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("rebuild the index"), "{err}");
    }

    #[test]
    fn load_detects_corrupt_files() {
        let tmp = tempfile::tempdir().unwrap();
        // missing files
        assert!(VectorStore::load(tmp.path()).is_err());
        // wrong length
        std::fs::write(tmp.path().join("vectors.bin"), vec![0u8; 10]).unwrap();
        write_status(&status_for(&["x"]), tmp.path());
        assert!(VectorStore::load(tmp.path()).is_err());
        // name/vector count mismatch
        std::fs::write(tmp.path().join("vectors.bin"), vec![0u8; DIM * 4 * 2]).unwrap();
        assert!(VectorStore::load(tmp.path()).is_err());
    }

    fn status_for(names: &[&str]) -> VectorStore {
        let mut store = VectorStore::new();
        for name in names {
            store.push(name, vec![0.0; DIM]).unwrap();
        }
        store
    }

    #[test]
    fn save_replaces_atomically_on_rerun() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = VectorStore::new();
        store.push("A", vec![0.5; DIM]).unwrap();
        store.save_vectors(tmp.path()).unwrap();
        write_status(&store, tmp.path());
        // save again with different content; load must reflect the new store
        let mut store2 = VectorStore::new();
        store2.push("B", vec![0.1; DIM]).unwrap();
        store2.save_vectors(tmp.path()).unwrap();
        write_status(&store2, tmp.path());
        let loaded = VectorStore::load(tmp.path()).unwrap();
        assert_eq!(loaded.meta.names, vec!["B"]);
    }

    #[test]
    fn meta_index_lookup() {
        let mut store = VectorStore::new();
        store.push("Bolt", vec![0.0; DIM]).unwrap();
        store.push("Fanatic", vec![0.0; DIM]).unwrap();
        assert_eq!(store.meta.index_of("Fanatic"), Some(1));
        assert_eq!(store.meta.index_of("Nope"), None);
    }
}
