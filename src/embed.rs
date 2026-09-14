use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::Write;

// Local text embedding via fastembed ONNX + a flat on-disk vector store.
//
// Cards are embedded into unit-normalized 384-dim vectors (bge-small-en-v1.5);
// at this corpus size (~32k) brute-force cosine over a flat f32 matrix is
// well under 50ms, so no ANN index is needed.

/// Model used for embeddings; also recorded in `status.json` so we can detect
/// a stale index after a model change.
///
/// Quantized variant: ~4x faster CPU inference than fp32 with the same
/// 384-dim output and negligible retrieval loss at this corpus size.
pub const MODEL: fastembed::EmbeddingModel = fastembed::EmbeddingModel::BGESmallENV15Q;

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

/// Row index of a card name, if present.
impl VectorMeta {
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
}

/// Flat vector store: row `i` is the embedding of `meta.names[i]`.
#[derive(Debug, Clone)]
pub struct VectorStore {
    /// Index metadata (model, dim, card names in row order).
    pub meta: VectorMeta,
    /// Row-major f32 matrix, unit-normalized rows (`meta.names.len() * dim`).
    pub vectors: Vec<f32>,
}

/// Version of the document layout [`build_doc`] produces.
///
/// Stored in `status.json` as `doc_version`. When the layout changes, bump
/// this constant; the next sync re-embeds every card whose stored version
/// lags behind.
pub const DOC_VERSION: u32 = 1;

/// Build the embedding document for a card.
///
/// Layout (name first so exact-name lookups rank top):
///
/// ```text
/// Lightning Bolt
/// {R} · Instant
/// Keywords: none
/// Colors: R
/// Tags: removal, burn
/// P/T: 2/2            (creatures; Loyalty: 4 for planeswalkers)
/// Deal 3 damage to any target.
/// ```
///
/// The structured lines carry facts the text model underweights (color
/// words, keyword names, stat lines) so queries like "black sacrifice
/// outlet" or "big green creature" match on more than oracle prose. The
/// Tags line carries Tagger's community role labels — vocabulary oracle
/// text alone does not surface. The line is omitted when the card has no
/// tags (or the tag index is empty).
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
    let mut doc = format!("{name}\n{mana_cost} · {type_line}\n{keyword_line}\n{color_line}");
    if !tags_line.is_empty() {
        doc.push('\n');
        doc.push_str(tags_line);
    }
    if let (Some(p), Some(t)) = (power, toughness) {
        doc.push_str(&format!("\nP/T: {p}/{t}"));
    } else if let Some(l) = loyalty {
        doc.push_str(&format!("\nLoyalty: {l}"));
    }
    doc.push('\n');
    doc.push_str(oracle_text);
    doc
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

/// Load the embedding model, cached under `cache_dir`.
///
/// First call downloads the quantized ONNX model (~35MB); later calls load
/// from cache. `show_progress` forwards fastembed's own download progress.
///
/// # Errors
/// Propagates model load/download failures.
/// Intra-op threads for ONNX Runtime.
///
/// 8 measured fastest on a 12-core machine (139/s @4 → 188/s @8); more
/// threads plateau and slightly regress.
pub const INTRA_THREADS: usize = 8;

pub fn load_model(
    cache_dir: &std::path::Path,
    show_progress: bool,
) -> anyhow::Result<fastembed::TextEmbedding> {
    let options = fastembed::InitOptions::new(MODEL)
        .with_cache_dir(cache_dir.to_path_buf())
        .with_show_download_progress(show_progress)
        .with_max_length(MAX_LENGTH)
        .with_intra_threads(INTRA_THREADS);
    fastembed::TextEmbedding::try_new(options)
        .context("initializing embedding model (first run downloads ~35MB)")
}

/// Embed texts, normalizing each vector to unit length.
///
/// Unit normalization turns cosine similarity into a plain dot product,
/// which is what [`VectorStore::search`] exploits.
///
/// # Errors
/// Propagates embedding failures.
pub fn embed_texts(
    model: &mut fastembed::TextEmbedding,
    texts: &[String],
) -> anyhow::Result<Vec<Vec<f32>>> {
    let mut out = model.embed(texts, Some(256)).context("embedding texts")?;
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

#[allow(dead_code)]
impl VectorStore {
    /// Empty store with the standard metadata.
    pub fn new() -> Self {
        Self {
            meta: VectorMeta {
                model: model_name(),
                dim: DIM,
                names: Vec::new(),
            },
            vectors: Vec::new(),
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
        self.vectors.extend(vector);
        Ok(())
    }

    /// Number of stored vectors.
    pub fn len(&self) -> usize {
        self.meta.names.len()
    }

    /// True when no vectors are stored.
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
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
            for value in &self.vectors {
                writer.write_all(&value.to_le_bytes())?;
            }
            writer.flush()?;
        }
        std::fs::rename(&tmp, &vectors_path)?;
        Ok(())
    }

    /// Rebuild a store from `status.json` + `vectors.bin` on disk.
    ///
    /// # Errors
    /// Fails on missing/corrupt files, dimension mismatch, or a vector count
    /// that does not match the name count.
    pub fn load(dir: &std::path::Path) -> anyhow::Result<Self> {
        let status = crate::paths::Status::read(&dir.join("status.json"))?;
        let meta = VectorMeta {
            model: status.model,
            dim: status.dim,
            names: status.names,
        };
        let vectors_path = dir.join("vectors.bin");
        let bytes = std::fs::read(&vectors_path)
            .with_context(|| format!("reading {}", vectors_path.display()))?;
        anyhow::ensure!(
            bytes.len() % (DIM * 4) == 0,
            "vectors.bin size {} is not a multiple of {DIM}*4 bytes",
            bytes.len()
        );
        let count = bytes.len() / (DIM * 4);
        anyhow::ensure!(
            count == meta.names.len(),
            "vectors.bin holds {count} vectors but status.json lists {} names",
            meta.names.len()
        );
        let vectors = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Ok(Self { meta, vectors })
    }

    /// `(index, cosine score)` for the `limit` best matches, best first.
    ///
    /// Vectors are unit-normalized, so this is a dot-product scan.
    pub fn search(&self, query: &[f32], limit: usize) -> Vec<(usize, f32)> {
        let mut scored: Vec<(usize, f32)> = (0..self.meta.names.len())
            .map(|i| {
                let row = &self.vectors[i * DIM..(i + 1) * DIM];
                let score: f32 = query.iter().zip(row).map(|(q, v)| q * v).sum();
                (i, score)
            })
            .collect();
        scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(limit);
        scored
    }

    /// Embed one query string (with the BGE query instruction) into a
    /// normalized vector ready for [`VectorStore::search`].
    ///
    /// # Errors
    /// Propagates embedding failures.
    pub fn embed_query(
        &self,
        model: &mut fastembed::TextEmbedding,
        query: &str,
    ) -> anyhow::Result<Vec<f32>> {
        let mut out = model
            .embed([format!("{QUERY_INSTRUCTION}{query}")], None)
            .context("embedding query")?;
        let mut vector = out
            .pop()
            .ok_or_else(|| anyhow::anyhow!("model returned no embedding"))?;
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
    "BAAI/bge-small-en-v1.5-Q".to_string()
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
            "Lightning Bolt\n{R} · Instant\nKeywords: none\nColors: R\nDeal 3 damage."
        );
        // With tags: the Tags line sits between colors and stats/text.
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
        assert!(doc.contains("Colors: R\nTags: burn, removal\nDeal 3"));
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
        assert!(doc.contains("Keywords: Haste, Vigilance"));
        assert!(doc.contains("Colors: G"));
        assert!(doc.contains("P/T: 2/2"));
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
        assert!(doc.contains("Loyalty: 4"));
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
        assert!(doc.contains("Keywords: none"));
        assert!(doc.contains("Colors: colorless"));
        // Malformed JSON columns degrade to "none"/"colorless".
        let doc = build_doc(
            "X", "", "Land", "not json", "also not", None, None, None, "", "",
        );
        assert!(doc.contains("Keywords: none"));
        assert!(doc.contains("Colors: colorless"));
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
        assert!(doc.starts_with("Forest\n · Basic Land — Forest\nKeywords: none"));
    }

    #[test]
    fn push_normalizes_and_validates_dim() {
        let mut store = VectorStore::new();
        let v: Vec<f32> = vec![3.0; DIM];
        store.push("A", v).expect("push");
        let row = &store.vectors[..DIM];
        let norm: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm was {norm}");
        assert_eq!(store.len(), 1);
        assert!(store.push("B", vec![1.0, 2.0]).is_err());
    }

    #[test]
    fn search_ranks_by_dot_product() {
        let mut store = VectorStore::new();
        // Unit vectors in 384-dim space; use simple 2-element content.
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
        let hits = store.search(&q, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, store.meta.index_of("a").unwrap());
        assert_eq!(hits[1].0, store.meta.index_of("b").unwrap());
        assert!((hits[0].1 - 1.0).abs() < 1e-6);
        // limit larger than corpus returns everything
        assert_eq!(store.search(&q, 10).len(), 3);
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
                .vectors
                .iter()
                .zip(store.vectors.iter())
                .all(|(x, y)| (x - y).abs() < 1e-6)
        );
        assert_eq!(loaded.meta.model, "BAAI/bge-small-en-v1.5-Q");
        assert_eq!(loaded.meta.dim, DIM);
    }

    /// Write the matching status.json for a store, mirroring setup step 5.
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
