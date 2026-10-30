//! Metadata mismatches must rebuild the complete card index during normal sync.

use super::*;
use std::io::Write as _;

/// Last successful sync before a metadata rebuild attempt.
const PRIOR_SYNC_STAMP: &str = "2020-01-01T00:00:00Z";

/// Distinct vectors prove unchanged cards were rebuilt along with SQL changes.
struct MetadataRebuildModel {
    remaining: Option<usize>,
}

impl crate::embed::DocumentEmbedder for MetadataRebuildModel {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for text in texts {
            anyhow::ensure!(
                self.remaining != Some(0),
                "injected failure after one embedding"
            );
            if let Some(remaining) = &mut self.remaining {
                *remaining -= 1;
            }
            let index = if text.starts_with("Unchanged.") {
                1
            } else if text.starts_with("Added.") {
                2
            } else {
                0
            };
            let mut vector = vec![0.0; crate::embed::DIM];
            vector[index] = 1.0;
            vectors.push(vector);
        }
        Ok(vectors)
    }
}

/// Keep initial SQL rows and bulk rows identical except for the stated text.
fn card(name: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name, "layout": "normal", "games": ["paper"],
        "id": format!("sid-{name}"), "oracle_id": format!("oid-{name}"),
        "released_at": "2020-01-01", "oracle_text": text, "type_line": "Instant",
        "rarity": "common", "set": "tst", "collector_number": "1"
    })
}

/// Seed current-shape metadata with a mismatch and matching unchanged-card hashes.
fn seed_metadata_mismatch(paths: &crate::paths::Paths, mismatch: &str) {
    paths.ensure_dirs().unwrap();
    let lock = StoreLock::acquire(paths).unwrap();
    let mut conn = crate::db::open(&paths.db()).unwrap();
    for (name, text) in [("Changed", "old text"), ("Unchanged", "same text")] {
        let row = serde_json::from_value(card(name, text)).unwrap();
        crate::scryfall::insert_card(&conn, &row).unwrap();
    }
    conn.execute_batch(
        "INSERT INTO collection (name, binder, binder_type, quantity, purchase_price)
        VALUES ('Changed', 'Keep me', 'binder', 7, 2.25);
        INSERT INTO collection (name, foil, binder, binder_type, quantity, purchase_price)
        VALUES ('Unchanged', 'foil', 'Other deck', 'deck', 4, 5.0);",
    )
    .unwrap();
    complete_setup(&lock, &mut conn).unwrap();
    let mut status = crate::paths::Status::empty();
    status.setup_complete = true;
    status.model = if mismatch == "model" {
        "mismatched/model".to_string()
    } else {
        crate::embed::MODEL_NAME.to_string()
    };
    status.dim = if mismatch == "dimension" {
        8
    } else {
        crate::embed::DIM
    };
    status.doc_version = if mismatch == "layout" {
        crate::embed::DOC_VERSION - 1
    } else {
        crate::embed::DOC_VERSION
    };
    status.names = vec!["Changed".into(), "Unchanged".into()];
    status.ingested_cards = 2;
    status.embedded_cards = 2;
    status.scryfall_synced_at = PRIOR_SYNC_STAMP.into();
    let mut bytes = Vec::new();
    for _ in &status.names {
        for index in 0..status.dim {
            let value: f32 = if index + 1 == status.dim { 1.0 } else { 0.0 };
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(paths.vectors_file(), bytes).unwrap();
    status.write(&paths.status_file()).unwrap();
    let mut bulk = flate2::write::GzEncoder::new(
        std::fs::File::create(paths.bulk_file()).unwrap(),
        flate2::Compression::fast(),
    );
    for (name, text) in [
        ("Changed", "new text"),
        ("Unchanged", "same text"),
        ("Added", "fresh text"),
    ] {
        writeln!(
            bulk,
            "{}",
            serde_json::to_string(&card(name, text)).unwrap()
        )
        .unwrap();
    }
    bulk.finish().unwrap();
}

/// Capture every collection field, including physical printing and location.
fn collection_rows(conn: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut stmt = conn
        .prepare("SELECT * FROM collection ORDER BY id")
        .unwrap();
    stmt.query_map([], |row| {
        (0..row.as_ref().column_count())
            .map(|index| row.get(index))
            .collect()
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// Use normal guarded sync functions with local bulk data and a fake embedder.
#[test]
fn metadata_rebuild_worker() {
    let Some(root) = std::env::var_os("STM_METADATA_REBUILD_ROOT") else {
        return;
    };
    let paths = crate::paths::Paths::new(root.into());
    let mode = std::env::var("STM_METADATA_REBUILD_MODE").unwrap();
    let lock = StoreLock::acquire(&paths).unwrap();
    recover_publication(&lock).unwrap();
    let mut conn = crate::db::open(&paths.db()).unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let delta =
        crate::sync::sync_cards_locked(&lock, &mut conn, &paths.bulk_file(), &mut out, "now")
            .unwrap();
    assert_eq!(
        (delta.added, delta.changed),
        if matches!(mode.as_str(), "retry" | "unchanged") {
            (0, 0)
        } else {
            (1, 1)
        }
    );
    let result = embed_pending(&lock, &mut conn, &mut out, || {
        anyhow::ensure!(mode != "load", "injected model load failure");
        Ok(Box::new(MetadataRebuildModel {
            remaining: if mode == "embed" { Some(1) } else { None },
        }))
    });
    if matches!(mode.as_str(), "success" | "retry" | "unchanged") {
        assert_eq!(
            result.unwrap(),
            if mode == "unchanged" { 2 } else { 3 },
            "all cards must use the current model"
        );
        crate::sync::stamp_synced(&paths, chrono::Utc::now()).unwrap();
        assert_eq!(
            embed_pending(&lock, &mut conn, &mut out, || anyhow::bail!(
                "current retry must not load a model"
            ))
            .unwrap(),
            0
        );
    } else {
        assert!(result.is_err(), "{mode} must fail");
        std::process::exit(23);
    }
}

/// Exit between attempts so retry cannot use the previous in-memory matrix.
fn run_metadata_rebuild_worker(
    paths: &crate::paths::Paths,
    mode: &str,
) -> std::process::ExitStatus {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sync::recovery::metadata_rebuild_tests::metadata_rebuild_worker",
            "--nocapture",
        ])
        .env("STM_METADATA_REBUILD_ROOT", paths.root())
        .env("STM_METADATA_REBUILD_MODE", mode)
        .status()
        .unwrap()
}

/// Check current metadata and every vector through the production search path.
fn assert_rebuilt_index(paths: &crate::paths::Paths, before: &[Vec<rusqlite::types::Value>]) {
    let lock = StoreLock::acquire(paths).unwrap();
    let conn = crate::db::open(&paths.db()).unwrap();
    assert_eq!(collection_rows(&conn), before);
    let status = crate::paths::Status::read(&paths.status_file()).unwrap();
    assert_eq!(status.model, crate::embed::MODEL_NAME);
    assert_eq!(status.dim, crate::embed::DIM);
    assert_eq!(status.doc_version, crate::embed::DOC_VERSION);
    assert_ne!(status.scryfall_synced_at, PRIOR_SYNC_STAMP);
    let store = crate::embed::VectorStore::load_owned_locked(&lock).unwrap();
    assert_eq!(store.meta.model, crate::embed::MODEL_NAME);
    assert_eq!(store.len(), 3);
    for (name, index) in [("Changed", 0), ("Unchanged", 1), ("Added", 2)] {
        let row = store.row(store.meta.index_of(name).unwrap());
        assert_eq!(row[index], 1.0, "{name} must be re-embedded");
        assert_eq!(
            row[crate::embed::DIM - 1],
            0.0,
            "{name} must not retain a mismatched-metadata vector"
        );
    }
    let cards = crate::db::load_all_cards(&conn).unwrap();
    let mut query = vec![0.0; crate::embed::DIM];
    query[2] = 1.0;
    let hits = crate::query::search_with_vectors(
        &conn,
        &cards,
        &store,
        &query,
        "fresh",
        &crate::search::CardFilters::default(),
        3,
        None,
    )
    .unwrap();
    assert_eq!(hits.vector[0].card.name, "Added");
    assert_eq!(hits.fts[0].card.name, "Added");
    let pending: i64 = conn
        .query_row("SELECT count(*) FROM pending_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(pending, 0);
}

/// Each metadata mismatch must rebuild unchanged cards instead of relabeling them.
#[test]
fn normal_sync_rebuilds_mismatched_metadata_without_resetting_collection() {
    for mismatch in ["model", "dimension", "layout"] {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(temp.path().to_path_buf());
        seed_metadata_mismatch(&paths, mismatch);
        let conn = crate::db::open(&paths.db()).unwrap();
        let before = collection_rows(&conn);
        drop(conn);
        assert!(
            run_metadata_rebuild_worker(&paths, "success").success(),
            "{mismatch}"
        );
        assert_rebuilt_index(&paths, &before);
    }
}

/// Metadata alone must trigger a rebuild when SQL and recorded hashes match.
#[test]
fn mismatched_model_rebuilds_the_complete_index_without_an_sql_delta() {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    seed_metadata_mismatch(&paths, "model");
    let conn = crate::db::open(&paths.db()).unwrap();
    let before = collection_rows(&conn);
    drop(conn);
    let mut bulk = flate2::write::GzEncoder::new(
        std::fs::File::create(paths.bulk_file()).unwrap(),
        flate2::Compression::fast(),
    );
    for (name, text) in [("Changed", "old text"), ("Unchanged", "same text")] {
        writeln!(bulk, "{}", card(name, text)).unwrap();
    }
    bulk.finish().unwrap();
    assert!(run_metadata_rebuild_worker(&paths, "unchanged").success());
    let lock = StoreLock::acquire(&paths).unwrap();
    let conn = crate::db::open(&paths.db()).unwrap();
    assert_eq!(collection_rows(&conn), before);
    let store = crate::embed::VectorStore::load_owned_locked(&lock).unwrap();
    assert_eq!(store.meta.model, crate::embed::MODEL_NAME);
    assert_eq!(store.len(), 2);
    for (name, index) in [("Changed", 0), ("Unchanged", 1)] {
        let row = store.row(store.meta.index_of(name).unwrap());
        assert_eq!(row[index], 1.0);
        assert_eq!(row[crate::embed::DIM - 1], 0.0);
    }
    let status = crate::paths::Status::read(&paths.status_file()).unwrap();
    assert_eq!(status.model, crate::embed::MODEL_NAME);
    assert_eq!(status.dim, crate::embed::DIM);
    assert_eq!(status.doc_version, crate::embed::DOC_VERSION);
    assert_ne!(status.scryfall_synced_at, PRIOR_SYNC_STAMP);
}

/// Failed rebuild work and the prior freshness stamp must survive a process exit.
#[test]
fn metadata_rebuild_retries_failures_without_publishing_partial_mixed_vectors() {
    for mode in ["load", "embed", "vectors", "metadata"] {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(temp.path().to_path_buf());
        seed_metadata_mismatch(&paths, "model");
        let conn = crate::db::open(&paths.db()).unwrap();
        let before = collection_rows(&conn);
        drop(conn);
        let previous_vectors = std::fs::read(paths.vectors_file()).unwrap();
        let blocker = match mode {
            "vectors" => Some(paths.root().join("vectors.bin.tmp")),
            "metadata" => Some(paths.root().join("status.json.tmp")),
            _ => None,
        };
        if let Some(path) = &blocker {
            std::fs::create_dir(path).unwrap();
        }
        assert_eq!(
            run_metadata_rebuild_worker(&paths, mode).code(),
            Some(23),
            "{mode}"
        );
        let status = crate::paths::Status::read(&paths.status_file()).unwrap();
        assert_eq!(status.model, "mismatched/model");
        assert_eq!(status.scryfall_synced_at, PRIOR_SYNC_STAMP);
        if mode != "metadata" {
            assert_eq!(
                std::fs::read(paths.vectors_file()).unwrap(),
                previous_vectors
            );
        }
        let conn = crate::db::open(&paths.db()).unwrap();
        assert_eq!(collection_rows(&conn), before);
        assert_eq!(
            crate::sync::load_pending_embeddings(&conn).unwrap(),
            ["Added", "Changed", "Unchanged"]
        );
        drop(conn);
        if let Some(path) = blocker {
            std::fs::remove_dir(path).unwrap();
        }
        assert!(
            run_metadata_rebuild_worker(&paths, "retry").success(),
            "{mode}"
        );
        assert_rebuilt_index(&paths, &before);
    }
}
