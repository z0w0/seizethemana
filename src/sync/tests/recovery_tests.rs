//! Exercise the SQL queue and real index files without downloading a model.

use super::*;
use std::io::Write as _;

/// Deterministic vectors that distinguish added and changed documents.
struct FakeModel {
    fail: bool,
}

impl crate::embed::DocumentEmbedder for FakeModel {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::ensure!(!self.fail, "injected embedding failure");
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0; crate::embed::DIM];
                vector[usize::from(text.contains("fresh text"))] = 1.0;
                vector
            })
            .collect())
    }
}

/// Save identical retry input in a local compressed bulk file.
fn write_bulk(paths: &crate::paths::Paths) {
    let mut encoder = flate2::write::GzEncoder::new(
        std::fs::File::create(paths.bulk_file()).unwrap(),
        flate2::Compression::fast(),
    );
    for (name, text) in [("Changed", "new text"), ("Added", "fresh text")] {
        let card = serde_json::json!({
            "name": name, "layout": "normal", "games": ["paper"],
            "id": format!("sid-{name}"), "oracle_id": format!("oid-{name}"),
            "released_at": "2020-01-01", "oracle_text": text,
            "type_line": "Instant", "rarity": "common", "set": "tst",
            "collector_number": "1", "prices": {"usd": "1.00"}
        });
        writeln!(encoder, "{card}").unwrap();
    }
    encoder.finish().unwrap();
}

/// Build a complete old index and preserve a representative collection row.
fn seed(paths: &crate::paths::Paths) {
    paths.ensure_dirs().unwrap();
    let mut conn = crate::db::open(&paths.db()).unwrap();
    let lock = StoreLock::acquire(paths).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, oracle_text, released_at)
        VALUES ('Changed', 'oid-Changed', 'old text', '2020-01-01')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity, purchase_price)
        VALUES ('Changed', 'Keep me', 'binder', 7, 2.25)",
        [],
    )
    .unwrap();
    let mut store = crate::embed::VectorStore::new();
    store.push("Changed", vec![0.5; crate::embed::DIM]).unwrap();
    store.save_vectors(paths.root()).unwrap();
    let mut status = crate::paths::Status::empty();
    status.setup_complete = true;
    status.names = store.meta.names.clone();
    status.model = store.meta.model.clone();
    status.dim = crate::embed::DIM;
    status.doc_version = crate::embed::DOC_VERSION;
    status.scryfall_synced_at = "2020-01-01T00:00:00Z".into();
    status.write(&paths.status_file()).unwrap();
    complete_setup(&lock, &mut conn).unwrap();
    write_bulk(paths);
}

/// Run in a separate process so failure recovery cannot use in-memory state.
#[test]
fn restart_worker() {
    let Some(root) = std::env::var_os("STM_SYNC_RESTART_ROOT") else {
        return;
    };
    let paths = crate::paths::Paths::new(root.into());
    let mode = std::env::var("STM_SYNC_RESTART_MODE").unwrap();
    let lock = StoreLock::acquire(&paths).unwrap();
    let mut conn = crate::db::open(&paths.db()).unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let delta =
        crate::sync::sync_cards_locked(&lock, &mut conn, &paths.bulk_file(), &mut out, "now")
            .unwrap();
    assert_eq!(delta.to_embed.len(), 2);
    if mode == "retry" {
        assert_eq!((delta.added, delta.changed), (0, 0));
    }
    let result = embed_pending(&lock, &mut conn, &mut out, || {
        anyhow::ensure!(mode != "model", "injected model load failure");
        Ok(Box::new(FakeModel {
            fail: mode == "embed",
        }))
    });
    if mode == "retry" {
        assert_eq!(result.unwrap(), 2);
        crate::sync::stamp_synced(&paths, chrono::Utc::now()).unwrap();
    } else {
        assert!(result.is_err(), "{mode} must fail");
        // Exit without destructors, as a killed command would.
        std::process::exit(23);
    }
}

/// Start the same test binary with only the restart worker enabled.
fn worker(paths: &crate::paths::Paths, mode: &str) -> std::process::ExitStatus {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sync::recovery::tests::restart_worker",
            "--nocapture",
        ])
        .env("STM_SYNC_RESTART_ROOT", paths.root())
        .env("STM_SYNC_RESTART_MODE", mode)
        .status()
        .unwrap()
}

/// Verify saved rows through the production vector and full-text search path.
fn assert_recovered(paths: &crate::paths::Paths) {
    let conn = crate::db::open(&paths.db()).unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM pending_embeddings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let owned: (String, String, i64, f64) = conn
        .query_row(
            "SELECT name, binder, quantity, purchase_price FROM collection",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(owned, ("Changed".into(), "Keep me".into(), 7, 2.25));
    let store = crate::embed::VectorStore::load(paths.root()).unwrap();
    let cards = crate::db::load_all_cards(&conn).unwrap();
    let mut query = vec![0.0; crate::embed::DIM];
    query[1] = 1.0;
    let results = crate::query::search_with_vectors(
        &conn,
        &cards,
        &store,
        &query,
        "fresh",
        &crate::search::CardFilters::default(),
        2,
        None,
    )
    .unwrap();
    assert_eq!(results.vector[0].card.name, "Added");
    assert_eq!(results.fts[0].card.oracle_text, "fresh text");
    let changed = store.meta.index_of("Changed").unwrap();
    assert_eq!(
        store.row(changed)[0],
        1.0,
        "changed vector replaces the old vector"
    );
    assert_eq!(store.len(), cards.len());
}

/// SQL commits must remain recoverable after each failed publication stage.
#[test]
fn identical_bulk_recovers_model_embedding_vector_and_metadata_failures_after_process_exit() {
    for mode in ["model", "embed", "vectors", "metadata"] {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(temp.path().to_path_buf());
        seed(&paths);
        let blocker = match mode {
            "vectors" => Some(paths.root().join("vectors.bin.tmp")),
            "metadata" => Some(paths.root().join("status.json.tmp")),
            _ => None,
        };
        if let Some(path) = &blocker {
            std::fs::create_dir(path).unwrap();
        }
        assert_eq!(worker(&paths, mode).code(), Some(23), "{mode}");
        let conn = crate::db::open(&paths.db()).unwrap();
        assert_eq!(
            crate::sync::load_pending_embeddings(&conn).unwrap(),
            ["Added", "Changed"]
        );
        let quantity: i64 = conn
            .query_row("SELECT sum(quantity) FROM collection", [], |row| row.get(0))
            .unwrap();
        assert_eq!(quantity, 7, "failed {mode} must not change collection rows");
        drop(conn);
        let status = crate::paths::Status::read(&paths.status_file()).unwrap();
        assert_eq!(status.scryfall_synced_at, "2020-01-01T00:00:00Z");
        if let Some(path) = blocker {
            std::fs::remove_dir(path).unwrap();
        }
        assert!(worker(&paths, "retry").success(), "{mode}");
        assert_recovered(&paths);
    }
}

/// Publishing an old document must not acknowledge a newer queued revision.
#[test]
fn acknowledging_an_old_revision_does_not_clear_new_work() {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    seed(&paths);
    let mut conn = crate::db::open(&paths.db()).unwrap();
    conn.execute("UPDATE cards SET oracle_text = 'first change'", [])
        .unwrap();
    let old = pending_documents(&mut conn, false).unwrap();
    conn.execute("UPDATE cards SET oracle_text = 'second change'", [])
        .unwrap();
    let new = pending_documents(&mut conn, false).unwrap();
    assert_ne!(old[0].revision, new[0].revision);
    complete_documents(&mut conn, &old).unwrap();
    assert_eq!(
        crate::sync::load_pending_embeddings(&conn).unwrap(),
        ["Changed"]
    );
    assert_eq!(
        pending_documents(&mut conn, false).unwrap()[0].revision,
        new[0].revision
    );
}

/// Tag updates must survive a crash even when no card content changed.
#[test]
fn tag_change_is_found_after_restart_without_an_sql_card_delta() {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    seed(&paths);
    let conn = crate::db::open(&paths.db()).unwrap();
    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('t', 'ramp', 'Ramp', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO card_tags (oracle_id, tag_id) VALUES ('oid-Changed', 't')",
        [],
    )
    .unwrap();
    drop(conn);
    let mut conn = crate::db::open(&paths.db()).unwrap();
    let pending = pending_documents(&mut conn, false).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].document.contains("Tags Ramp"));
}

/// Queue failures must roll back card and price changes in the same transaction.
#[test]
fn failed_queue_write_rolls_back_the_card_update() {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    seed(&paths);
    let lock = StoreLock::acquire(&paths).unwrap();
    let mut conn = crate::db::open(&paths.db()).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER reject_pending BEFORE INSERT ON pending_embeddings
        BEGIN SELECT RAISE(ABORT, 'injected queue failure'); END;",
    )
    .unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let error =
        crate::sync::sync_cards_locked(&lock, &mut conn, &paths.bulk_file(), &mut out, "now")
            .err()
            .expect("queue write must fail");
    assert!(format!("{error:#}").contains("injected queue failure"));
    assert_eq!(
        crate::db::get_card(&conn, "Changed")
            .unwrap()
            .unwrap()
            .oracle_text,
        "old text"
    );
    assert!(crate::db::get_card(&conn, "Added").unwrap().is_none());
    assert!(
        crate::sync::load_pending_embeddings(&conn)
            .unwrap()
            .is_empty()
    );
    let count: i64 = conn
        .query_row("SELECT count(*) FROM card_prints", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

/// A layout change must queue stored cards and cards not yet in the index.
#[test]
fn layout_change_requeues_stored_and_added_cards_once() {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    seed(&paths);
    let mut conn = crate::db::open(&paths.db()).unwrap();
    let card = crate::db::get_card(&conn, "Changed").unwrap().unwrap();
    let doc = crate::embed::doc_for_row(&card, &crate::tags::TagIndex::default());
    let previous = format!(
        "{}\n{}\n{doc}",
        crate::embed::DOC_VERSION - 1,
        crate::embed::MODEL_NAME
    );
    let old_revision = format!("{:x}", Sha256::digest(previous.as_bytes()));
    conn.execute(
        "UPDATE embedding_revisions SET revision = ?1",
        [old_revision],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, oracle_text) VALUES ('Added', 'oid-Added', 'fresh text')",
        [],
    )
    .unwrap();
    let pending = pending_documents(&mut conn, false).unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["Changed", "Added"]
    );
    let mut out = crate::output::Output::new(true, false, false);
    let lock = StoreLock::acquire(&paths).unwrap();
    assert_eq!(
        embed_pending(&lock, &mut conn, &mut out, || Ok(Box::new(FakeModel {
            fail: false
        })))
        .unwrap(),
        2
    );
    assert!(pending_documents(&mut conn, false).unwrap().is_empty());
    assert_eq!(
        crate::embed::VectorStore::load_owned_locked(&lock)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        embed_pending(&lock, &mut conn, &mut out, || anyhow::bail!(
            "unchanged docs must not load a model"
        ))
        .unwrap(),
        0
    );
}
