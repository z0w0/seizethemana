//! Ranked-query snapshots under a waiting writer and interrupted publication.

use super::*;
use std::sync::mpsc;
use std::time::Duration;

/// A known query vector avoids model loading and network access.
fn fake_query_embedding() -> Vec<f32> {
    let mut vector = vec![0.0; embed::DIM];
    vector[0] = 1.0;
    vector
}

/// Save a matrix and matching metadata for the test's current SQL card rows.
fn save_snapshot(paths: &Paths, conn: &rusqlite::Connection, complete: bool) -> anyhow::Result<()> {
    let mut store = VectorStore::new();
    for card in db::load_all_cards(conn)? {
        let mut vector = vec![0.0; embed::DIM];
        vector[usize::from(!card.oracle_text.contains("needle"))] = 1.0;
        store.push(&card.name, vector)?;
    }
    store.save_vectors(paths.root())?;
    let mut status = Status::empty();
    status.setup_complete = complete;
    status.model = embed::MODEL_NAME.to_string();
    status.dim = embed::DIM;
    status.doc_version = embed::DOC_VERSION;
    status.names = store.meta.names;
    status.ingested_cards = status.names.len();
    status.embedded_cards = status.names.len();
    status.write(&paths.status_file())
}

/// Seed a complete store with one full-text match and one owned holding.
fn seed(paths: &Paths) -> rusqlite::Connection {
    paths.ensure_dirs().expect("test directories");
    let _lock = StoreLock::acquire(paths).expect("seed lock");
    let conn = db::open(&paths.db()).expect("database");
    conn.execute_batch("INSERT INTO cards (name, oracle_id, oracle_text) VALUES ('Original', 'original', 'needle');
        INSERT INTO collection (name, binder, binder_type, quantity) VALUES ('Original', 'Keep', 'binder', 7);")
        .expect("seed card and holding");
    save_snapshot(paths, &conn, true).expect("seed index");
    conn
}

/// Run the public query path with a controlled embedding and ordinary filters.
fn search(paths: &Paths, conn: &rusqlite::Connection) -> Vec<Hit> {
    run_search(
        paths,
        conn,
        &Output::new(true, true, false),
        "needle",
        &CardFilters::default(),
        10,
        None,
    )
    .expect("ranked query snapshot")
}

/// A writer cannot change card IDs or FTS rows between index loading and retrieval.
#[test]
fn run_search_holds_one_snapshot_through_full_text_retrieval() {
    let temp = tempfile::tempdir().expect("snapshot store");
    let paths = Paths::new(temp.path().to_path_buf());
    let conn = seed(&paths);
    let (at_fts, reached_fts) = mpsc::channel();
    let (writer_blocked, wait_for_writer) = mpsc::channel();
    TEST_EMBEDDING.set(Some(fake_query_embedding));
    BEFORE_FTS.set(Some(Box::new(move || {
        at_fts.send(()).expect("full-text boundary");
        wait_for_writer
            .recv_timeout(Duration::from_secs(5))
            .expect("writer must observe the held query lock");
    })));
    let writer_paths = paths.clone();
    let writer = std::thread::spawn(move || {
        reached_fts
            .recv_timeout(Duration::from_secs(5))
            .expect("reader reached full-text retrieval");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(writer_paths.root().join("store.lock"))
            .expect("existing lock file");
        let error = fs2::FileExt::try_lock_exclusive(&file)
            .expect_err("query must still own the snapshot lock");
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        drop(file);
        writer_blocked.send(()).expect("report contention");
        let _lock = StoreLock::acquire(&writer_paths).expect("writer waits for query completion");
        let conn = db::open(&writer_paths.db()).expect("writer connection");
        conn.execute_batch("BEGIN;
            UPDATE cards SET oracle_text = 'replacement' WHERE name = 'Original';
            INSERT INTO cards (name, oracle_id, oracle_text) VALUES ('Newcomer', 'newcomer', 'needle');
            COMMIT;").expect("publish new card and FTS rows");
        save_snapshot(&writer_paths, &conn, true).expect("publish matching vectors and status");
    });
    let old = search(&paths, &conn);
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].card.name, "Original");
    assert_eq!(old[0].card.oracle_text, "needle");
    writer.join().expect("writer completed after query");
    let new = search(&paths, &conn);
    assert_eq!(new[0].card.name, "Newcomer");
    assert_eq!(new[0].card.oracle_text, "needle");
    assert_eq!(new.len(), 2);
    assert_eq!(
        db::load_all_cards(&conn).expect("new SQL snapshot").len(),
        2
    );
    let owned: i64 = conn
        .query_row(
            "SELECT quantity FROM collection WHERE name = 'Original'",
            [],
            |row| row.get(0),
        )
        .expect("holding retained");
    assert_eq!(owned, 7);
    assert_eq!(
        std::fs::read_dir(paths.models_dir())
            .expect("model directory")
            .count(),
        0
    );
    TEST_EMBEDDING.set(None);
}

/// A ready publication restores setup status before the ranked-query setup gate.
#[test]
fn run_search_recovers_ready_publication_before_setup_check() {
    let temp = tempfile::tempdir().expect("recovery store");
    let paths = Paths::new(temp.path().to_path_buf());
    let mut conn = seed(&paths);
    TEST_EMBEDDING.set(Some(fake_query_embedding));
    for owned in [false, true] {
        {
            let _lock = StoreLock::acquire(&paths).expect("stage publication");
            let staging = Paths::new(paths.root().join("sync-publication"));
            save_snapshot(&staging, &conn, true).expect("ready index");
            std::fs::write(staging.root().join("ready"), []).expect("ready marker");
            let mut incomplete = Status::read(&paths.status_file()).expect("old status");
            incomplete.setup_complete = false;
            incomplete
                .write(&paths.status_file())
                .expect("old setup state");
        }
        if owned {
            let code = crate::collection::run_query(
                &paths,
                &mut conn,
                &mut Output::new(true, true, false),
                "needle",
                &crate::cli::CardFilters::default(),
                &[],
                &[],
                10,
                true,
            )
            .expect("owned query recovers before setup check");
            assert_eq!(code, crate::cli::codes::OK);
        } else {
            let hits = search(&paths, &conn);
            assert_eq!(hits[0].card.name, "Original");
        }
        assert!(paths.is_setup());
        assert!(!paths.root().join("sync-publication/ready").exists());
    }
    assert_eq!(
        std::fs::read_dir(paths.models_dir())
            .expect("model directory")
            .count(),
        0
    );
    TEST_EMBEDDING.set(None);
}

/// Replace the database and index files as a completed forced setup would.
fn replace_store(paths: &Paths) {
    let _lock = StoreLock::acquire(paths).expect("forced setup lock");
    for suffix in ["", "-wal", "-shm"] {
        let path = std::path::PathBuf::from(format!("{}{suffix}", paths.db().display()));
        if path.exists() {
            std::fs::remove_file(path).expect("remove old database file");
        }
    }
    let conn = db::open(&paths.db()).expect("replacement database");
    conn.execute_batch("INSERT INTO cards (name, oracle_id, oracle_text) VALUES ('Original', 'original', 'needle revision two');
        INSERT INTO collection (name, binder, binder_type, quantity) VALUES ('Original', 'New', 'binder', 3);
        INSERT INTO card_prints (scryfall_id, name, set_code, collector_number, lang, rarity, released_at, usd, updated_at)
        VALUES ('original', 'Original', 'tst', '1', 'en', 'common', '2020-01-01', 4, 'seed');")
        .expect("replacement card and holding");
    save_snapshot(paths, &conn, true).expect("replacement index");
}

/// A connection opened before forced setup must not supply the ranked SQL rows.
#[test]
fn run_search_uses_replacement_database_after_forced_setup() {
    let temp = tempfile::tempdir().expect("forced setup store");
    let paths = Paths::new(temp.path().to_path_buf());
    let conn = seed(&paths);
    replace_store(&paths);
    assert_eq!(
        db::load_all_cards(&conn).expect("old inode")[0].oracle_text,
        "needle"
    );
    TEST_EMBEDDING.set(Some(fake_query_embedding));
    let hits = search(&paths, &conn);
    assert_eq!(hits[0].card.oracle_text, "needle revision two");
    TEST_EMBEDDING.set(None);
}

/// Run a full command in a child so its real JSON can be checked by the parent.
fn command_snapshot_child(paths: &Paths, case: &str) {
    let mut conn = seed(paths);
    conn.execute("INSERT INTO card_prints (scryfall_id, name, set_code, collector_number, lang, rarity, released_at, usd, updated_at)
        VALUES ('original', 'Original', 'tst', '1', 'en', 'common', '2020-01-01', 2, 'seed')", []).expect("initial price");
    let reset = case.starts_with("reset");
    if reset {
        replace_store(paths);
    } else {
        let paths = paths.clone();
        AFTER_SCOPE.set(Some(Box::new(move || {
            let writer = db::open(&paths.db()).expect("scope writer");
            writer
                .execute_batch(
                    "BEGIN;
                UPDATE card_prints SET usd = 9;
                UPDATE collection SET quantity = 99;
                COMMIT;",
                )
                .expect("unlocked price and collection update");
            let price: f64 = writer
                .query_row("SELECT usd FROM card_prints", [], |row| row.get(0))
                .expect("committed price");
            assert_eq!(price, 9.0);
        })));
    }
    TEST_EMBEDDING.set(Some(fake_query_embedding));
    let text = if case.starts_with("listing") {
        ""
    } else {
        "needle"
    };
    let filters = crate::cli::CardFilters::default();
    let mut out = Output::new(true, true, false);
    println!("SNAPSHOT_JSON_BEGIN");
    let code = if case.ends_with("collection") {
        let binder = if reset { "New" } else { "Keep" }.to_string();
        crate::collection::run_query(
            paths,
            &mut conn,
            &mut out,
            text,
            &filters,
            &[binder],
            &[],
            10,
            true,
        )
    } else {
        crate::query::run_query(
            paths,
            &mut conn,
            &mut out,
            text,
            &filters,
            Some(5.0),
            10,
            true,
        )
    }
    .expect("full query command");
    println!("SNAPSHOT_JSON_END");
    assert_eq!(code, crate::cli::codes::OK);
    TEST_EMBEDDING.set(None);
    assert_eq!(
        std::fs::read_dir(paths.models_dir())
            .expect("no model cache")
            .count(),
        0
    );
}

/// Price caps, ownership, locations, and JSON detail must share one SQL read view.
#[test]
fn full_query_commands_keep_sql_snapshots_and_replacement_files() {
    const CHILD_CASE: &str = "STM_QUERY_SNAPSHOT_CASE";
    const CHILD_ROOT: &str = "STM_QUERY_SNAPSHOT_ROOT";
    if let Ok(case) = std::env::var(CHILD_CASE) {
        let root = std::env::var_os(CHILD_ROOT).expect("child store path");
        command_snapshot_child(&Paths::new(root.into()), &case);
        return;
    }
    for case in [
        "ranked_global",
        "listing_global",
        "ranked_collection",
        "listing_collection",
        "reset_global",
        "reset_collection",
    ] {
        let temp = tempfile::tempdir().expect("command snapshot store");
        let result = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "query::run::run_tests::full_query_commands_keep_sql_snapshots_and_replacement_files", "--nocapture"])
            .env(CHILD_CASE, case).env(CHILD_ROOT, temp.path())
            .output().expect("query command child");
        let stdout = String::from_utf8(result.stdout).expect("JSON output");
        assert!(
            result.status.success(),
            "{case}: {stdout}\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let json = stdout
            .split_once("SNAPSHOT_JSON_BEGIN\n")
            .expect("JSON start")
            .1
            .split_once("SNAPSHOT_JSON_END")
            .expect("JSON end")
            .0;
        let rows: Vec<serde_json::Value> = serde_json::from_str(json).expect("full command JSON");
        assert_eq!(rows.len(), 1, "{case}");
        let row = &rows[0];
        let reset = case.starts_with("reset");
        assert_eq!(row["name"], "Original", "{case}");
        assert_eq!(
            row["oracle_text"],
            if reset {
                "needle revision two"
            } else {
                "needle"
            },
            "{case}"
        );
        assert_eq!(
            row["price"],
            if reset { 4.0 } else { 2.0 },
            "{case}: price must match the scope read"
        );
        assert!(
            row["price"].as_f64().expect("known price") <= 5.0,
            "{case}: price cap"
        );
        assert_eq!(row["owned"], if reset { 3 } else { 7 }, "{case}");
        assert_eq!(row["available"], if reset { 3 } else { 7 }, "{case}");
        if case.ends_with("collection") {
            assert_eq!(
                row["locations"][0]["binder"],
                if reset { "New" } else { "Keep" },
                "{case}"
            );
            assert_eq!(
                row["locations"][0]["quantity"],
                if reset { 3 } else { 7 },
                "{case}"
            );
        }
    }
}
