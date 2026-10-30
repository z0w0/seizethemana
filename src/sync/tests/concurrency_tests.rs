//! Deterministic writer interleaving at the production embedding boundary.

use super::*;
use std::io::{BufRead as _, Write as _};

/// Controlled vectors identify the exact document revision that was published.
struct RevisionModel;

impl crate::embed::DocumentEmbedder for RevisionModel {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0; crate::embed::DIM];
                vector[if text.contains("revision two") { 2 } else { 1 }] = 1.0;
                vector
            })
            .collect())
    }
}

/// Seed one indexed card so both writers overwrite the same physical row.
fn seed(paths: &crate::paths::Paths) {
    let lock = StoreLock::acquire(paths).unwrap();
    let mut conn = crate::db::open(&paths.db()).unwrap();
    conn.execute_batch(
        "INSERT INTO cards (name, oracle_id, oracle_text)
        VALUES ('Card', 'oid-card', 'initial');
        INSERT INTO collection (name, binder, binder_type, quantity, purchase_price)
        VALUES ('Card', 'Keep me', 'binder', 7, 2.25);",
    )
    .unwrap();
    let mut store = crate::embed::VectorStore::new();
    let mut vector = vec![0.0; crate::embed::DIM];
    vector[0] = 1.0;
    store.push("Card", vector).unwrap();
    store.save_vectors(paths.root()).unwrap();
    let mut status = crate::paths::Status::empty();
    status.setup_complete = true;
    status.model = store.meta.model.clone();
    status.dim = store.meta.dim;
    status.names = store.meta.names.clone();
    status.doc_version = crate::embed::DOC_VERSION;
    status.write(&paths.status_file()).unwrap();
    complete_setup(&lock, &mut conn).unwrap();
    for (mode, text) in [("a", "revision one"), ("b", "revision two")] {
        let mut writer = flate2::write::GzEncoder::new(
            std::fs::File::create(paths.root().join(format!("{mode}.jsonl.gz"))).unwrap(),
            flate2::Compression::fast(),
        );
        let card = serde_json::json!({
            "name": "Card", "oracle_id": "oid-card", "id": "sid-card", "layout": "normal",
            "games": ["paper"], "released_at": "2020-01-01", "oracle_text": text,
            "type_line": "Instant", "rarity": "common", "set": "tst", "collector_number": "1"
        });
        writeln!(writer, "{card}").unwrap();
        writer.finish().unwrap();
    }
}

/// Report a reached boundary before waiting for the parent process.
fn signal(message: &str) {
    println!("STORE_TEST:{message}");
    std::io::stdout().flush().unwrap();
}

/// Verify contention is a held store lock, not a filesystem access failure.
fn assert_contended(paths: &crate::paths::Paths) {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(paths.root().join("store.lock"))
        .unwrap();
    let error = fs2::FileExt::try_lock_exclusive(&file).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
}

/// Use the same guarded SQL and publication functions as normal sync.
#[test]
fn writer_worker() {
    let Some(root) = std::env::var_os("STM_STORE_CONCURRENCY_ROOT") else {
        return;
    };
    let paths = crate::paths::Paths::new(root.into());
    let mode = std::env::var("STM_STORE_CONCURRENCY_MODE").unwrap();
    if mode != "a" {
        assert_contended(&paths);
        signal(&format!("{mode}-waiting"));
    }
    if mode == "reader" {
        let store = crate::embed::VectorStore::load(paths.root()).unwrap();
        assert_eq!(store.meta.names, ["Card"]);
        assert_eq!(store.len(), 1);
        assert!(store.row(0)[1] == 1.0 || store.row(0)[2] == 1.0);
        return;
    }
    let mut conn = crate::db::open(&paths.db()).unwrap();
    let lock = StoreLock::acquire(&paths).unwrap();
    crate::db::reopen(&mut conn, &paths.db()).unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    crate::sync::sync_cards_locked(
        &lock,
        &mut conn,
        &paths.root().join(format!("{mode}.jsonl.gz")),
        &mut out,
        "now",
    )
    .unwrap();
    assert_eq!(
        embed_pending(&lock, &mut conn, &mut out, || {
            if mode == "a" {
                signal("a-snapshot");
                let mut command = String::new();
                std::io::stdin().read_line(&mut command).unwrap();
                assert_eq!(command.trim(), "publish");
            }
            Ok(Box::new(RevisionModel))
        })
        .unwrap(),
        1
    );
    assert!(pending_documents(&mut conn, false).unwrap().is_empty());
}

/// Start a worker with a bounded protocol channel instead of timing sleeps.
fn spawn(
    paths: &crate::paths::Paths,
    mode: &str,
) -> (std::process::Child, std::sync::mpsc::Receiver<String>) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sync::recovery::concurrency_tests::writer_worker",
            "--nocapture",
        ])
        .env("STM_STORE_CONCURRENCY_ROOT", paths.root())
        .env("STM_STORE_CONCURRENCY_MODE", mode)
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

/// Bound failures without making process scheduling part of the assertion.
fn wait_for(messages: &std::sync::mpsc::Receiver<String>, expected: &str) {
    assert_eq!(
        messages
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap(),
        expected
    );
}

/// A waiting second writer cannot invalidate the first writer's snapshot.
#[test]
fn concurrent_writers_keep_sql_vectors_and_acknowledged_revisions_aligned() {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    seed(&paths);
    let original = crate::embed::VectorStore::load(paths.root()).unwrap();
    let (mut a, a_messages) = spawn(&paths, "a");
    wait_for(&a_messages, "a-snapshot");
    let (mut b, b_messages) = spawn(&paths, "b");
    wait_for(&b_messages, "b-waiting");
    let (mut reader, reader_messages) = spawn(&paths, "reader");
    wait_for(&reader_messages, "reader-waiting");
    let mut conn = crate::db::open(&paths.db()).unwrap();
    assert_eq!(
        crate::db::get_card(&conn, "Card")
            .unwrap()
            .unwrap()
            .oracle_text,
        "revision one"
    );
    a.stdin.take().unwrap().write_all(b"publish\n").unwrap();
    assert!(a.wait().unwrap().success());
    assert!(b.wait().unwrap().success());
    assert!(reader.wait().unwrap().success());

    let lock = StoreLock::acquire(&paths).unwrap();
    let current = crate::embed::VectorStore::load_owned_locked(&lock).unwrap();
    let card = crate::db::get_card(&conn, "Card").unwrap().unwrap();
    assert_eq!(card.oracle_text, "revision two");
    assert_eq!(current.row(0)[2], 1.0);
    let published: String = conn
        .query_row(
            "SELECT revision FROM embedding_revisions WHERE name = 'Card'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        published,
        revision(&crate::embed::doc_for_row(
            &card,
            &crate::tags::TagIndex::default()
        ))
    );
    let mut out = crate::output::Output::new(true, false, false);
    assert_eq!(
        embed_pending(&lock, &mut conn, &mut out, || anyhow::bail!(
            "aligned retry must not load a model"
        ))
        .unwrap(),
        0
    );
    let owned: (i64, f64) = conn
        .query_row(
            "SELECT quantity, purchase_price FROM collection",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(owned, (7, 2.25));
    assert_eq!(original.meta.names, ["Card"]);
    assert_eq!(
        original.row(0)[0],
        1.0,
        "publication must not overwrite a mapped snapshot"
    );
}
