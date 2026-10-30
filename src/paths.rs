//! On-disk data locations and the `status.json` state file. [Paths](crate::paths::Paths) maps
//! one data root to every file the CLI reads or writes; [Status](crate::paths::Status) records
//! setup state and vector-index metadata.

use anyhow::Context;
use std::path::PathBuf;

/// Default data directory under the user's home directory.
pub const DEFAULT_DIR_NAME: &str = ".seizethemana";

/// All on-disk locations the CLI uses, rooted at one data directory.
///
/// Accessors derive each path from the root. Tests and `--data-dir` can use
/// a different root without changing the storage layout.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// Build paths from an explicit root directory.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolve the effective data directory: `override_dir` if given, else
    /// `$HOME/.seizethemana`.
    ///
    /// # Errors
    /// Returns an error when no home directory can be located.
    pub fn resolve(override_dir: Option<&std::path::Path>) -> anyhow::Result<Self> {
        let root = match override_dir {
            Some(dir) => dir.to_path_buf(),
            None => dirs_home().map(|home| home.join(DEFAULT_DIR_NAME))?,
        };
        Ok(Self { root })
    }

    /// Root of the data directory.
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// SQLite database holding `cards` and `collection`.
    pub fn db(&self) -> PathBuf {
        self.root.join("stm.db")
    }

    /// Directory for downloaded Scryfall bulk files.
    pub fn bulk_dir(&self) -> PathBuf {
        self.root.join("bulk")
    }

    /// Downloaded gzip default-cards file, including per-printing prices.
    pub fn bulk_file(&self) -> PathBuf {
        self.bulk_dir().join("default_cards.jsonl.gz")
    }

    /// Downloaded gzip Scryfall oracle tags file.
    pub fn tags_file(&self) -> PathBuf {
        self.bulk_dir().join("oracle_tags.jsonl.gz")
    }

    /// Downloaded gzip Commander Spellbook variants file.
    pub fn combos_file(&self) -> PathBuf {
        self.bulk_dir().join("variants.json.gz")
    }

    /// Binary matrix of card embedding vectors.
    pub fn vectors_file(&self) -> PathBuf {
        self.root.join("vectors.bin")
    }

    /// JSON sidecar for the vector matrix and setup state (`status.json`).
    pub fn status_file(&self) -> PathBuf {
        self.root.join("status.json")
    }

    /// Directory for the local embedding model cache.
    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    /// Directory holding deck text files.
    pub fn decks_dir(&self) -> PathBuf {
        self.root.join("decks")
    }

    /// Path of a single deck file.
    pub fn deck_file(&self, name: &str) -> PathBuf {
        self.decks_dir().join(format!("{name}.txt"))
    }

    /// Create the store directories that do not already exist.
    ///
    /// Decks are created lazily by deck commands, not here.
    pub fn ensure_dirs(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.bulk_dir())?;
        std::fs::create_dir_all(self.models_dir())?;
        Ok(())
    }

    /// True when `status.json` records that setup completed.
    pub fn is_setup(&self) -> bool {
        Status::read(&self.status_file())
            .map(|status| status.setup_complete)
            .unwrap_or(false)
    }
}

/// Setup state, vector-index metadata, and sync timestamps in `status.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Status {
    /// True when `stm setup` finished all steps.
    pub setup_complete: bool,
    /// Cards added by setup, or total indexed cards after sync publication.
    pub ingested_cards: usize,
    /// Vector count recorded by the last setup or sync publication.
    pub embedded_cards: usize,
    /// Embedding model name, e.g. "BAAI/bge-small-en-v1.5".
    pub model: String,
    /// Vector dimension (columns of the matrix).
    pub dim: usize,
    /// Card names, in matrix row order.
    pub names: Vec<String>,
    /// RFC 3339 timestamp of the last successful Scryfall sync.
    /// Empty before the first sync. Controls background refresh timing.
    pub scryfall_synced_at: String,
    /// Version of the embedding document layout the vectors were built with.
    /// Sync rebuilds the full index when this differs from [`crate::embed::DOC_VERSION`].
    pub doc_version: u32,
    /// RFC 3339 timestamp of the last successful combo bulk refresh.
    /// Empty before the first refresh. Used for status display only.
    pub combos_synced_at: String,
}

impl Status {
    /// State for a store that has not been set up yet.
    pub fn empty() -> Self {
        Self {
            setup_complete: false,
            ingested_cards: 0,
            embedded_cards: 0,
            model: String::new(),
            dim: 0,
            names: Vec::new(),
            scryfall_synced_at: String::new(),
            doc_version: 0,
            combos_synced_at: String::new(),
        }
    }

    /// Read `status.json`; a missing file reads as [`Status::empty`].
    ///
    /// # Errors
    /// Fails when an existing file cannot be read or parsed.
    pub fn read(path: &std::path::Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(err) => Err(err.into()),
        }
    }

    /// Replace the state file by writing and renaming a temporary file.
    ///
    /// # Errors
    /// Returns serialization and filesystem errors.
    pub fn write(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Read the user's home directory from `HOME`.
fn dirs_home() -> anyhow::Result<PathBuf> {
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home)),
        _ => Err(anyhow::anyhow!(
            "could not locate home directory; set $HOME or pass --data-dir"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_dir_wins_over_home() {
        let paths = Paths::new(PathBuf::from("/tmp/whatever"));
        assert_eq!(paths.root(), &PathBuf::from("/tmp/whatever"));
    }

    #[test]
    fn storage_layout_is_stable() {
        let paths = Paths::new(PathBuf::from("/data"));
        assert_eq!(paths.db(), PathBuf::from("/data/stm.db"));
        assert_eq!(
            paths.bulk_file(),
            PathBuf::from("/data/bulk/default_cards.jsonl.gz")
        );
        assert_eq!(
            paths.tags_file(),
            PathBuf::from("/data/bulk/oracle_tags.jsonl.gz")
        );
        assert_eq!(paths.vectors_file(), PathBuf::from("/data/vectors.bin"));
        assert_eq!(paths.status_file(), PathBuf::from("/data/status.json"));
        assert_eq!(paths.models_dir(), PathBuf::from("/data/models"));
        assert_eq!(
            paths.deck_file("Burn"),
            PathBuf::from("/data/decks/Burn.txt")
        );
    }

    #[test]
    fn status_defaults_to_empty_when_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let status = Status::read(&tmp.path().join("status.json")).expect("read");
        assert!(!status.setup_complete);
        assert!(status.names.is_empty());
    }

    #[test]
    fn status_roundtrips() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("status.json");
        let status = Status {
            setup_complete: true,
            ingested_cards: 100,
            embedded_cards: 99,
            model: "m".into(),
            dim: 384,
            names: vec!["Bolt".into()],
            scryfall_synced_at: String::new(),
            doc_version: 0,
            combos_synced_at: String::new(),
        };
        status.write(&path).expect("write");
        let loaded = Status::read(&path).expect("read");
        assert_eq!(loaded.names, vec!["Bolt"]);
        assert_eq!(loaded.ingested_cards, 100);
        assert_eq!(loaded.embedded_cards, 99);
        assert!(loaded.setup_complete);
    }

    /// Existing status files must include every field in the current schema.
    #[test]
    fn status_requires_all_current_fields() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("status.json");
        let current = serde_json::to_value(Status::empty()).expect("serialize status");
        for key in current.as_object().expect("status object").keys() {
            let mut incomplete = current.clone();
            incomplete
                .as_object_mut()
                .expect("status object")
                .remove(key);
            std::fs::write(
                &path,
                serde_json::to_vec(&incomplete).expect("serialize status"),
            )
            .expect("write status");
            let error = Status::read(&path).expect_err("missing status field must fail");
            assert!(
                format!("{error:#}").contains(&format!("missing field `{key}`")),
                "{key}: {error:#}"
            );
        }
    }

    #[test]
    fn is_setup_requires_complete_flag() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths::new(tmp.path().to_path_buf());
        assert!(!paths.is_setup());
        Status {
            setup_complete: true,
            ingested_cards: 1,
            embedded_cards: 1,
            model: "m".into(),
            dim: 384,
            names: vec!["x".into()],
            scryfall_synced_at: String::new(),
            doc_version: 0,
            combos_synced_at: String::new(),
        }
        .write(&paths.status_file())
        .expect("write");
        assert!(paths.is_setup());
    }

    #[test]
    fn resolve_uses_home_when_no_override() {
        // Check the default directory name when HOME is set.
        if std::env::var_os("HOME").is_some() {
            let paths = Paths::resolve(None).expect("home should resolve");
            assert!(paths.root().ends_with(DEFAULT_DIR_NAME));
        }
    }

    #[test]
    fn resolve_prefers_explicit_override() {
        let paths = Paths::resolve(Some(std::path::Path::new("/custom"))).expect("override");
        assert_eq!(paths.root(), &PathBuf::from("/custom"));
    }

    #[test]
    fn ensure_dirs_creates_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths::new(tmp.path().to_path_buf());
        paths.ensure_dirs().expect("mkdirs");
        assert!(paths.bulk_dir().is_dir());
        assert!(paths.models_dir().is_dir());
        assert!(!paths.is_setup());
    }
}
