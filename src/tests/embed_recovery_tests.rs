//! Direct index loads must recover a completed journal without running sync.

use super::*;
use crate::paths::{Paths, Status};
use std::io::BufRead as _;

/// Save metadata without changing the last successful sync time.
fn status_for(store: &VectorStore) -> Status {
    Status {
        setup_complete: true,
        ingested_cards: store.len(),
        embedded_cards: store.len(),
        model: store.meta.model.clone(),
        dim: store.meta.dim,
        names: store.meta.names.clone(),
        scryfall_synced_at: "2020-01-01T00:00:00Z".into(),
        doc_version: DOC_VERSION,
        combos_synced_at: String::new(),
    }
}

/// Leave the vector file ahead of metadata at the publication crash boundary.
fn interrupted_publication(paths: &Paths) {
    let conn = crate::db::open(&paths.db()).unwrap();
    conn.execute_batch(
        "INSERT INTO cards (name, oracle_id, oracle_text)
            VALUES ('Changed', 'oid-changed', 'new text'), ('Added', 'oid-added', 'fresh text');
         INSERT INTO collection (name, set_code, collector_number, foil, binder,
            binder_type, quantity, purchase_price)
            VALUES ('Changed', 'tst', '1', 'foil', 'Keep me', 'binder', 7, 2.25);
         INSERT INTO pending_embeddings (name, revision)
            VALUES ('Changed', 'changed-revision'), ('Added', 'added-revision');",
    )
    .unwrap();
    let mut old = VectorStore::new();
    old.push("Changed", vec![0.5; DIM]).unwrap();
    old.save_vectors(paths.root()).unwrap();
    status_for(&old).write(&paths.status_file()).unwrap();

    let mut next = VectorStore::new();
    for (name, index) in [("Changed", 0), ("Added", 1)] {
        let mut vector = vec![0.0; DIM];
        vector[index] = 1.0;
        next.push(name, vector).unwrap();
    }
    let staging = Paths::new(paths.root().join("sync-publication"));
    next.save_vectors(staging.root()).unwrap();
    status_for(&next).write(&staging.status_file()).unwrap();
    std::fs::write(staging.root().join("ready"), []).unwrap();
    std::fs::copy(staging.vectors_file(), paths.vectors_file()).unwrap();
}

/// Capture all collection fields before a direct load can modify any files.
fn collection_rows(conn: &rusqlite::Connection) -> Vec<Vec<rusqlite::types::Value>> {
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

/// Check the production search path without a model, network access, or sync.
fn assert_offline_load(paths: &Paths, owned: bool) {
    let conn = crate::db::open(&paths.db()).unwrap();
    let before = collection_rows(&conn);
    let store = if owned {
        VectorStore::load_owned(paths.root()).unwrap()
    } else {
        VectorStore::load(paths.root()).unwrap()
    };
    let cards = crate::db::load_all_cards(&conn).unwrap();
    let mut query = vec![0.0; DIM];
    query[1] = 1.0;
    let hits = crate::query::search_with_vectors(
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
    assert_eq!(hits.vector[0].card.name, "Added");
    assert_eq!(hits.fts[0].card.oracle_text, "fresh text");
    assert_eq!(store.row(store.meta.index_of("Changed").unwrap())[0], 1.0);
    assert_eq!(store.len(), cards.len());
    assert_eq!(collection_rows(&conn), before);
    let pending: i64 = conn
        .query_row("SELECT count(*) FROM pending_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        pending, 2,
        "a direct load must not acknowledge embedding revisions"
    );
    let status = Status::read(&paths.status_file()).unwrap();
    assert_eq!(status.scryfall_synced_at, "2020-01-01T00:00:00Z");
    assert!(!paths.root().join("sync-publication").exists());
    assert!(
        !paths.models_dir().exists(),
        "loading must not download a model"
    );
}

/// Separate processes prove recovery does not depend on retained Rust state.
#[test]
fn offline_load_worker() {
    let Some(root) = std::env::var_os("STM_EMBED_RECOVERY_ROOT") else {
        return;
    };
    let paths = Paths::new(root.into());
    let mode = std::env::var("STM_EMBED_RECOVERY_MODE").unwrap();
    if mode == "crash" {
        interrupted_publication(&paths);
        std::process::exit(23);
    }
    if mode == "repair-holder" {
        let lock = crate::sync::store_lock::StoreLock::acquire(&paths).unwrap();
        crate::sync::recovery::recover_publication(&lock).unwrap();
        signal_guard("repair-held");
        let mut command = String::new();
        std::io::stdin().read_line(&mut command).unwrap();
        assert_eq!(command.trim(), "load");
        let store = VectorStore::load_owned_locked(&lock).unwrap();
        assert_eq!(store.meta.names, ["Changed", "Added"]);
        assert_eq!(store.row(1)[1], 1.0);
        signal_guard("holder-loaded");
        return;
    }
    if mode == "waiting-reader" {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(paths.root().join("store.lock"))
            .unwrap();
        let error = fs2::FileExt::try_lock_exclusive(&file).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        signal_guard("reader-waiting");
    }
    assert_offline_load(&paths, mode == "owned");
    if mode == "waiting-reader" {
        signal_guard("reader-loaded");
    }
}

/// Flush a synchronization boundary before the child waits for the parent.
fn signal_guard(message: &str) {
    println!("STORE_TEST:{message}");
    std::io::stdout().flush().unwrap();
}

/// Capture child boundaries without relying on process timing.
fn spawn_guard_worker(
    paths: &Paths,
    mode: &str,
) -> (std::process::Child, std::sync::mpsc::Receiver<String>) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "embed::recovery_tests::offline_load_worker",
            "--nocapture",
        ])
        .env("STM_EMBED_RECOVERY_ROOT", paths.root())
        .env("STM_EMBED_RECOVERY_MODE", mode)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.unwrap();
            if let Some((_, message)) = line.split_once("STORE_TEST:") {
                send.send(message.to_string()).unwrap();
            }
        }
    });
    (child, receive)
}

/// Fail a deadlocked child with a bounded wait, not an arbitrary sleep.
fn wait_for_guard(messages: &std::sync::mpsc::Receiver<String>, expected: &str) {
    assert_eq!(
        messages
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap(),
        expected
    );
}

/// Start only the child test, with no inherited model or sync state.
fn worker(paths: &Paths, mode: &str) -> std::process::Output {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "embed::recovery_tests::offline_load_worker",
            "--nocapture",
        ])
        .env("STM_EMBED_RECOVERY_ROOT", paths.root())
        .env("STM_EMBED_RECOVERY_MODE", mode)
        .output()
        .unwrap()
}

/// Both mapped and owned loads must repair the pair before checking row counts.
#[test]
fn direct_load_after_process_exit_recovers_search_alignment_and_preserves_collection() {
    for mode in ["mapped", "owned"] {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::new(temp.path().to_path_buf());
        let crash = worker(&paths, "crash");
        assert_eq!(
            crash.status.code(),
            Some(23),
            "{}",
            String::from_utf8_lossy(&crash.stderr)
        );
        assert_eq!(
            Status::read(&paths.status_file()).unwrap().names,
            ["Changed"]
        );
        assert_eq!(
            std::fs::metadata(paths.vectors_file()).unwrap().len(),
            (DIM * 4 * 2) as u64
        );
        let loaded = worker(&paths, mode);
        assert!(
            loaded.status.success(),
            "{}",
            String::from_utf8_lossy(&loaded.stderr)
        );
    }
}

/// A failed recovery write must remain an error, not use a stale pair.
#[test]
fn direct_load_propagates_publication_recovery_errors() {
    let temp = tempfile::tempdir().unwrap();
    let paths = Paths::new(temp.path().to_path_buf());
    interrupted_publication(&paths);
    let blocker = paths.root().join("status.json.tmp");
    std::fs::create_dir(&blocker).unwrap();
    assert!(VectorStore::load(paths.root()).is_err());
    assert!(paths.root().join("sync-publication/ready").exists());
    assert_eq!(
        Status::read(&paths.status_file()).unwrap().names,
        ["Changed"]
    );
    std::fs::remove_dir(blocker).unwrap();
    assert_offline_load(&paths, false);
}

/// The repair guard must remain held until metadata and vectors are both loaded.
#[test]
fn recovery_guard_covers_the_following_paired_load_without_nested_locking() {
    let temp = tempfile::tempdir().unwrap();
    let paths = Paths::new(temp.path().to_path_buf());
    interrupted_publication(&paths);
    let (mut holder, holder_messages) = spawn_guard_worker(&paths, "repair-holder");
    wait_for_guard(&holder_messages, "repair-held");
    assert_eq!(Status::read(&paths.status_file()).unwrap().names.len(), 2);
    let (mut reader, reader_messages) = spawn_guard_worker(&paths, "waiting-reader");
    wait_for_guard(&reader_messages, "reader-waiting");
    holder.stdin.take().unwrap().write_all(b"load\n").unwrap();
    wait_for_guard(&holder_messages, "holder-loaded");
    wait_for_guard(&reader_messages, "reader-loaded");
    assert!(holder.wait().unwrap().success());
    assert!(reader.wait().unwrap().success());
}

/// A lock-file error must not bypass exclusion or consume the recovery journal.
#[test]
fn direct_load_propagates_store_lock_errors_without_repairing_files() {
    let temp = tempfile::tempdir().unwrap();
    let paths = Paths::new(temp.path().to_path_buf());
    interrupted_publication(&paths);
    let blocker = paths.root().join("store.lock");
    std::fs::create_dir(&blocker).unwrap();
    let error = VectorStore::load(paths.root()).unwrap_err();
    assert!(
        error.to_string().contains("opening store lock"),
        "{error:#}"
    );
    assert!(paths.root().join("sync-publication/ready").exists());
    assert_eq!(
        Status::read(&paths.status_file()).unwrap().names,
        ["Changed"]
    );
    std::fs::remove_dir(blocker).unwrap();
    assert_offline_load(&paths, false);
}
