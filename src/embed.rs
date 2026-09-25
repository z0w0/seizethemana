use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[cfg(target_os = "macos")]
/// Core ML embedding for document rebuild experiments.
pub mod coreml;

// Local text embedding via fastembed ONNX + a flat on-disk vector store.
//
// Cards are embedded into unit-normalized 384-dim vectors (bge-small-en-v1.5);
// at this corpus size (~32k) brute-force cosine over a flat f32 matrix is
// well under 50ms, so no ANN index is needed.

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

/// Max tokens per embedded text.
///
/// Card documents are short (name + cost/type + oracle text almost always fit
/// in 128 tokens; longer oracle text truncates harmlessly, losing only tail
/// detail). Benchmarked: 1.5x faster than 256 with 100% top-20 overlap on
/// representative queries; 64 tokens is faster still but drifts ~11%.
pub const MAX_LENGTH: usize = 128;

/// Expected embedding dimensionality of the model above.
pub const DIM: usize = 384;

/// BGE v1.5 query-side instruction. Documents are embedded raw; queries are
/// prefixed per the model card for asymmetric retrieval.
pub const QUERY_INSTRUCTION: &str = "Represent this sentence for searching relevant passages: ";

/// Index metadata stored inside `status.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMeta {
    /// Embedding model name, e.g. "BAAI/bge-small-en-v1.5-Q".
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

    /// Mutable owned slice; panics when mapped (mappings are read-only by
    /// design — mutating code paths always build on `Owned`).
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

/// Convenience: doc string for a loaded card row, with tags from `index`.
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

/// Intra-op threads for ONNX Runtime.
///
/// 8 measured fastest on a 12-core machine (139/s @4 → 188/s @8); more
/// threads plateau and slightly regress.
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
        let _ = show_progress;
        coreml::CoreMlEmbedding::load(cache_dir, MODEL, MAX_LENGTH, false)
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
/// Unit normalization turns cosine similarity into a plain dot product,
/// which is what [`VectorStore::search`] exploits.
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

/// In-place L2 normalization.
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
    /// Vectors are little-endian f32, written via temp file + rename so an
    /// interrupted run cannot leave a torn store. The matching metadata is
    /// written to `status.json` by the caller (setup) afterwards.
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
    /// `vectors.bin` is memory-mapped read-only: the scan touches only the
    /// pages it needs and the OS reclaims them freely, so a query process
    /// does not carry a 51MB heap copy. Callers that mutate the matrix
    /// (sync upserts) need [`VectorStore::load_owned`] instead — mapping
    /// mutations panic.
    ///
    /// # Errors
    /// Fails on missing/corrupt files, dimension mismatch, or a vector count
    /// that does not match the name count.
    pub fn load(dir: &std::path::Path) -> anyhow::Result<Self> {
        let (meta, vectors) = Self::read_store(dir)?;
        Ok(Self {
            meta,
            vectors: VectorMatrix::Mapped(vectors),
        })
    }

    /// [`load`] with an owned (mutable) matrix for the sync upsert path.
    ///
    /// Copies the mapped bytes into heap memory once per sync; the copy is
    /// what makes `row_mut`/`push` legal on a loaded store.
    ///
    /// # Errors
    /// Same as [`load`].
    pub fn load_owned(dir: &std::path::Path) -> anyhow::Result<Self> {
        let (meta, vectors) = Self::read_store(dir)?;
        Ok(Self {
            meta,
            vectors: VectorMatrix::Owned(f32slice::cast(&vectors).to_vec()),
        })
    }

    /// Read + validate `status.json` and `vectors.bin`.
    ///
    /// # Errors
    /// Fails on missing/corrupt files, dimension mismatch, or a vector count
    /// that does not match the name count.
    fn read_store(dir: &std::path::Path) -> anyhow::Result<(VectorMeta, memmap2::Mmap)> {
        let status = crate::paths::Status::read(&dir.join("status.json"))?;
        let meta = VectorMeta {
            model: status.model,
            dim: status.dim,
            names: status.names,
        };
        anyhow::ensure!(
            meta.dim == DIM,
            "status.json records dimension {}, but this build expects {DIM}; \
             rebuild the index with 'stm setup --force'",
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
        // mmap with private read-only semantics; the file length was
        // validated above, so mapping cannot fault on a truncated file.
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

/// Human-readable model name for `meta.json`.
fn model_name() -> String {
    MODEL_NAME.to_string()
}

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
        // Malformed JSON columns degrade to "none"/"colorless".
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
        // Status claiming a different dimension than the model produces:
        // the slice math would read garbage, so the load must fail loudly.
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
