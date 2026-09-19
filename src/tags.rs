// Scryfall oracle tags: download parsing, SQLite ingest, and lookup.
//
// Tags come from the Tagger project via the daily `oracle_tags` bulk file
// (JSONL, one tag object per line). Each tag carries `taggings`: card
// associations keyed by oracle_id. Tags describe functional roles ("removal",
// "ramp", "sacrifice outlet") rather than card text, so they sharpen both
// display (`stm card`) and the embedding documents.
//
// Only the tag `id` (a stable UUID) is treated as an identity; slugs and
// labels can change between daily files per Scryfall's guidance.

use anyhow::Context;
use rusqlite::Connection;
use serde::Deserialize;

/// One parsed tag row from the bulk file, with its card taggings.
#[derive(Debug, Deserialize, Clone)]
pub struct TagRecord {
    /// Stable tag UUID (Scryfall guidance: the only permanent identifier).
    pub id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub label: String,
    /// Card associations; oracle tags key by `oracle_id`.
    #[serde(default)]
    pub taggings: Vec<Tagging>,
}

/// One card-tag association.
#[derive(Debug, Deserialize, Clone)]
pub struct Tagging {
    #[serde(default)]
    pub oracle_id: String,
    #[serde(default)]
    pub weight: String,
}

/// Ingest summary for status lines and tests.
#[derive(Debug, Default, PartialEq)]
pub struct TagIngestSummary {
    pub tags: usize,
    /// Distinct cards that carry at least one tag.
    pub tagged_cards: usize,
    pub taggings: usize,
}

/// A tag's stable identity plus its display label and popularity.
#[derive(Debug, Clone, PartialEq)]
pub struct TagInfo {
    pub id: String,
    pub label: String,
    /// Global number of cards carrying this tag (popularity signal).
    pub use_count: i64,
}

/// Labels for every card, loaded once per process.
///
/// Lookup goes by oracle_id because taggings join on it and `cards.oracle_id`
/// is stored per card.
pub struct TagIndex {
    by_oracle: std::collections::HashMap<String, Vec<TagInfo>>,
}

impl TagIndex {
    /// Empty index (no tags): tests and tag-less stores.
    pub fn default_empty() -> Self {
        Self {
            by_oracle: std::collections::HashMap::new(),
        }
    }

    /// Load every card's tags in one query.
    ///
    /// A missing or empty `tags` table (fresh schema, failed ingest) reads as
    /// an empty index; embedding docs simply omit the Tags line.
    ///
    /// # Errors
    /// Propagates SQLite failures.
    pub fn load(conn: &Connection) -> anyhow::Result<Self> {
        let mut by_oracle: std::collections::HashMap<String, Vec<TagInfo>> =
            std::collections::HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT ct.oracle_id, t.id, t.label, t.use_count
             FROM card_tags ct JOIN tags t ON t.id = ct.tag_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (oracle_id, id, label, use_count) = row.context("reading tag rows")?;
            by_oracle.entry(oracle_id).or_default().push(TagInfo {
                id,
                label,
                use_count,
            });
        }
        Ok(Self { by_oracle })
    }

    /// All tags for one card (any order).
    pub fn tags_for(&self, oracle_id: &str) -> &[TagInfo] {
        self.by_oracle
            .get(oracle_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Display labels for one card: every tag label, alphabetical.
    pub fn labels_for(&self, oracle_id: &str) -> Vec<String> {
        let mut labels: Vec<String> = self
            .tags_for(oracle_id)
            .iter()
            .map(|t| t.label.clone())
            .collect();
        labels.sort_by_key(|a| a.to_lowercase());
        labels.dedup();
        labels
    }

    /// The embedding-document tag line for one card: up to
    /// [`select_labels::MAX_DOC_TAGS`] labels chosen for spread and
    /// popularity, or an empty string when the card has no tags.
    pub fn doc_tags_line(&self, oracle_id: &str) -> String {
        let labels = select_labels::for_doc(self.tags_for(oracle_id));
        if labels.is_empty() {
            String::new()
        } else {
            format!("Tags: {}", labels.join(", "))
        }
    }
}

/// Pick the labels that go into an embedding document.
///
/// Max one label per tag (Scryfall's stable `id`), capped at a dozen for
/// token budget; the selection prefers popular, distinct tags over families
/// of near-duplicates.
pub mod select_labels {
    use super::TagInfo;

    /// Maximum labels embedded per card. Median card carries 6 tags; the cap
    /// only trims outlier decks of tags (max observed: 46) so oracle text
    /// keeps its token budget.
    pub const MAX_DOC_TAGS: usize = 12;

    /// Choose up to [`MAX_DOC_TAGS`] labels for one card's embedding doc.
    ///
    /// Greedy by global `use_count` (popular role words first), skipping a
    /// tag whose label shares its first word with an already-picked label
    /// (e.g. "activate from hand" blocks "activate from graveyard"). Ties
    /// break alphabetically so the choice is deterministic across runs.
    pub fn for_doc(tags: &[TagInfo]) -> Vec<String> {
        let mut ranked: Vec<&TagInfo> = tags.iter().collect();
        ranked.sort_by(|a, b| {
            b.use_count
                .cmp(&a.use_count)
                .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
        });
        let mut picked: Vec<String> = Vec::with_capacity(MAX_DOC_TAGS);
        let mut seen_first_words: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for tag in ranked {
            if picked.len() >= MAX_DOC_TAGS {
                break;
            }
            let label = tag.label.trim().to_string();
            if label.is_empty() {
                continue;
            }
            let first_word = label
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_lowercase();
            if !seen_first_words.insert(first_word) {
                continue;
            }
            picked.push(label);
        }
        picked
    }
}

/// Stream the tags bulk file and rebuild `tags` + `card_tags` in one
/// transaction.
///
/// The file is a complete daily snapshot, so the tables are replaced
/// wholesale: `tags` rows are upserted (ids are stable, labels may shift)
/// and `card_tags` is deleted and refilled. `use_count` is the tagging
/// count from this file.
///
/// # Errors
/// Fails on unreadable files or SQLite problems; a malformed tag line is
/// skipped.
pub fn ingest(
    conn: &mut Connection,
    bulk_path: &std::path::Path,
    out: &mut crate::output::Output,
) -> anyhow::Result<TagIngestSummary> {
    let file = std::fs::File::open(bulk_path)
        .with_context(|| format!("cannot open tags bulk file {}", bulk_path.display()))?;
    let decoder = flate2::read::MultiGzDecoder::new(std::io::BufReader::new(file));
    let reader = std::io::BufReader::new(decoder);

    let mut records: Vec<TagRecord> = Vec::new();
    for line in std::io::BufRead::lines(reader) {
        let line = line.with_context(|| format!("reading {}", bulk_path.display()))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(tag) = serde_json::from_str::<TagRecord>(line) {
            records.push(tag);
        }
    }

    let mut tagged_cards: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut taggings = 0usize;
    for tag in &records {
        for tg in &tag.taggings {
            if !tg.oracle_id.is_empty() {
                tagged_cards.insert(tg.oracle_id.as_str());
                taggings += 1;
            }
        }
    }
    out.status(
        "Parsed",
        &format!(
            "{} tags ({} taggings) from {}",
            records.len(),
            taggings,
            bulk_path.display()
        ),
    );

    conn.execute("BEGIN", []).context("begin tags ingest")?;
    let result = (|| -> anyhow::Result<TagIngestSummary> {
        conn.execute("DELETE FROM card_tags", [])?;
        let mut tag_stmt = conn.prepare(
            "INSERT INTO tags (id, slug, label, use_count)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (id) DO UPDATE SET
                slug = excluded.slug, label = excluded.label,
                use_count = excluded.use_count",
        )?;
        let mut ct_stmt = conn.prepare(
            "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id, weight) VALUES (?1, ?2, ?3)",
        )?;
        for tag in &records {
            let use_count = tag.taggings.len() as i64;
            tag_stmt.execute(rusqlite::params![tag.id, tag.slug, tag.label, use_count])?;
            for tg in &tag.taggings {
                if tg.oracle_id.is_empty() {
                    continue;
                }
                ct_stmt.execute(rusqlite::params![tg.oracle_id, tag.id, tg.weight])?;
            }
        }
        drop(ct_stmt);
        drop(tag_stmt);
        Ok(TagIngestSummary {
            tags: records.len(),
            tagged_cards: tagged_cards.len(),
            taggings,
        })
    })();
    match result {
        Ok(summary) => {
            conn.execute("COMMIT", [])
                .context("committing tags ingest")?;
            Ok(summary)
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(id: &str, label: &str, count: i64) -> TagInfo {
        TagInfo {
            id: id.to_string(),
            label: label.to_string(),
            use_count: count,
        }
    }

    #[test]
    fn for_doc_prefers_popular_and_spreads_families() {
        let tags = vec![
            tag("1", "activate from hand", 900),
            tag("2", "activate from graveyard", 850),
            tag("3", "activate from exile", 800),
            tag("4", "removal", 3000),
            tag("5", "ramp", 2500),
            tag("6", "ramp elves", 5),
            tag("7", "wheel", 1200),
        ];
        let picked = select_labels::for_doc(&tags);
        // Popular first, one per first-word family.
        assert_eq!(picked[0], "removal");
        assert_eq!(picked[1], "ramp");
        // "ramp elves" is skipped (family "ramp" already picked).
        assert!(!picked.iter().any(|l| l.contains("elves")));
        // Only one "activate from …" survives.
        assert_eq!(
            picked.iter().filter(|l| l.starts_with("activate")).count(),
            1
        );
        assert!(picked.len() <= select_labels::MAX_DOC_TAGS);
    }

    #[test]
    fn for_doc_is_deterministic_and_capped() {
        let tags: Vec<TagInfo> = (0..40)
            .map(|i| tag(&format!("id{i}"), &format!("tag{i}"), 100 - i))
            .collect();
        let a = select_labels::for_doc(&tags);
        let b = select_labels::for_doc(&tags);
        assert_eq!(a, b, "same input, same selection");
        assert_eq!(a.len(), select_labels::MAX_DOC_TAGS);
        assert_eq!(a[0], "tag0", "highest use_count first");
    }

    #[test]
    fn labels_for_sorts_alphabetically_case_insensitive() {
        let idx_tags = vec![tag("1", "Wheel", 10), tag("2", "draw", 20)];
        let mut by_oracle = std::collections::HashMap::new();
        by_oracle.insert("oid".to_string(), idx_tags);
        let index = TagIndex { by_oracle };
        assert_eq!(index.labels_for("oid"), vec!["draw", "Wheel"]);
        assert!(index.labels_for("missing").is_empty());
    }

    #[test]
    fn ingest_rebuilds_tables_and_counts() {
        use flate2::write::GzEncoder;
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tags.jsonl.gz");
        let enc = GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut enc = enc;
        writeln!(
            enc,
            r#"{{"id":"tag-1","slug":"removal","label":"Removal","type":"oracle","taggings":[{{"oracle_id":"oid-a","weight":"median"}},{{"oracle_id":"oid-b","weight":"very_strong"}}]}}"#
        )
        .unwrap();
        writeln!(
            enc,
            r#"{{"id":"tag-2","slug":"draw","label":"Card Draw","type":"oracle","taggings":[{{"oracle_id":"oid-a","weight":"median"}}]}}"#
        )
        .unwrap();
        writeln!(enc, "not json").unwrap();
        writeln!(
            enc,
            r#"{{"id":"tag-3","slug":"empty","label":"No Taggings","type":"oracle","taggings":[]}}"#
        )
        .unwrap();
        drop(enc);

        let mut conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        let mut out = crate::output::Output::new(true, false, false);
        let summary = ingest(&mut conn, &path, &mut out).unwrap();
        assert_eq!(summary.tags, 3);
        assert_eq!(summary.taggings, 3);
        assert_eq!(summary.tagged_cards, 2);

        // use_count reflects this file's tagging count.
        let count: i64 = conn
            .query_row("SELECT use_count FROM tags WHERE id = 'tag-1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);

        // Second ingest with a shrunk file replaces card_tags wholesale.
        let enc = GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut enc = enc;
        writeln!(
            enc,
            r#"{{"id":"tag-1","slug":"removal","label":"Removal","type":"oracle","taggings":[{{"oracle_id":"oid-a","weight":"median"}}]}}"#
        )
        .unwrap();
        drop(enc);
        let summary = ingest(&mut conn, &path, &mut out).unwrap();
        assert_eq!(summary.taggings, 1);
        let (n,): (i64,) = conn
            .query_row("SELECT COUNT(*) FROM card_tags", [], |r| {
                Ok((r.get(0).unwrap(),))
            })
            .unwrap();
        assert_eq!(n, 1);

        // TagIndex joins through cards-style oracle ids.
        let index = TagIndex::load(&conn).unwrap();
        let labels = index.labels_for("oid-a");
        assert_eq!(labels, vec!["Removal"]);
        assert_eq!(index.doc_tags_line("oid-a"), "Tags: Removal");
        assert_eq!(index.doc_tags_line("oid-missing"), "");
    }
}
