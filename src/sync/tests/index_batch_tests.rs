//! Index repair and bounded embedding batches without downloading a model.

use super::*;
use crate::embed::{DocumentEmbedder, VectorStore};
use crate::paths::{Paths, Status};
use std::cell::RefCell;
use std::rc::Rc;

/// Documents received by each model invocation, in invocation order.
type RecordedBatches = Rc<RefCell<Vec<Vec<String>>>>;

/// Model failures that must not publish or acknowledge incomplete work.
#[derive(Clone, Copy, Debug)]
enum BatchFailure {
    Error,
    MissingVector,
    ExtraVector,
    ShortDimension,
    LongDimension,
}

/// Record document batches and return name-based fixture vectors.
struct RecordingModel {
    batches: RecordedBatches,
    failure: Option<(usize, BatchFailure)>,
}

impl DocumentEmbedder for RecordingModel {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let call = {
            let mut batches = self.batches.borrow_mut();
            batches.push(texts.to_vec());
            batches.len()
        };
        let mut vectors: Vec<Vec<f32>> = texts
            .iter()
            .map(|text| vector_for_name(text.split('.').next().expect("fixture card name")))
            .collect();
        if let Some((failed_call, failure)) = self.failure
            && call == failed_call
        {
            match failure {
                BatchFailure::Error => anyhow::bail!("injected batch failure"),
                BatchFailure::MissingVector => {
                    vectors.pop().expect("fixture batch is not empty");
                }
                BatchFailure::ExtraVector => vectors.push(vec![0.0; crate::embed::DIM]),
                BatchFailure::ShortDimension => {
                    vectors
                        .last_mut()
                        .expect("fixture batch is not empty")
                        .pop()
                        .expect("fixture vector is not empty");
                }
                BatchFailure::LongDimension => vectors
                    .last_mut()
                    .expect("fixture batch is not empty")
                    .push(0.0),
            }
        }
        Ok(vectors)
    }
}

/// Build an injectable encoder while retaining its input batches for assertions.
fn recording_model(
    batches: &RecordedBatches,
    failure: Option<(usize, BatchFailure)>,
) -> Box<dyn DocumentEmbedder> {
    Box::new(RecordingModel {
        batches: Rc::clone(batches),
        failure,
    })
}

/// Use a fixture card's numeric suffix to expose row alignment errors.
fn vector_for_name(name: &str) -> Vec<f32> {
    let index: usize = name
        .strip_prefix("Card ")
        .expect("fixture card prefix")
        .parse()
        .expect("fixture card number");
    let mut vector = vec![0.0; crate::embed::DIM];
    vector[index % crate::embed::DIM] = 1.0;
    vector
}

/// Seed acknowledged documents and a current matrix in nonalphabetical SQL order.
fn seed_current_index(count: usize) -> (tempfile::TempDir, Paths, Connection) {
    let temp = tempfile::tempdir().expect("create data directory");
    let paths = Paths::new(temp.path().to_path_buf());
    paths.ensure_dirs().expect("create store directories");
    let lock = StoreLock::acquire(&paths).expect("lock fixture store");
    let mut conn = crate::db::open(&paths.db()).expect("open fixture database");
    let tx = conn.transaction().expect("begin fixture transaction");
    let mut store = VectorStore::new();
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO cards (name, oracle_id, oracle_text) VALUES (?1, ?1, 'old rules')",
            )
            .expect("prepare fixture cards");
        for index in (0..count).rev() {
            let name = format!("Card {index:04}");
            insert.execute([&name]).expect("insert fixture card");
            store
                .push(&name, vec![0.5; crate::embed::DIM])
                .expect("append fixture vector");
        }
    }
    tx.execute_batch(
        "INSERT INTO collection
         (name, set_code, collector_number, foil, binder, binder_type, quantity, purchase_price)
         VALUES ('Card 0000', 'tst', '17', 'normal', 'Keep me', 'binder', 7, 2.25),
                ('Card 0000', 'old', '29', 'foil', 'Other deck', 'deck', 4, 5.0);",
    )
    .expect("seed collection rows");
    tx.commit().expect("commit fixture rows");
    store
        .save_vectors(paths.root())
        .expect("save fixture vectors");
    let mut status = Status::empty();
    status.setup_complete = true;
    status.ingested_cards = count;
    status.embedded_cards = count;
    status.model = crate::embed::MODEL_NAME.to_string();
    status.dim = crate::embed::DIM;
    status.doc_version = crate::embed::DOC_VERSION;
    status.names = store.meta.names;
    status.scryfall_synced_at = "2020-01-01T00:00:00Z".to_string();
    status
        .write(&paths.status_file())
        .expect("save fixture status");
    complete_setup(&lock, &mut conn).expect("acknowledge fixture documents");
    (temp, paths, conn)
}

/// Snapshot every field so index repair cannot hide changes to stored user data.
fn sql_rows(conn: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
        .expect("prepare fixture snapshot");
    stmt.query_map([], |row| {
        (0..row.as_ref().column_count())
            .map(|column| row.get(column))
            .collect()
    })
    .expect("query fixture snapshot")
    .collect::<Result<_, _>>()
    .expect("read fixture snapshot")
}

/// Expected encoder input follows card IDs rather than lexical name order.
fn documents(conn: &Connection) -> Vec<String> {
    let tags = crate::tags::TagIndex::load(conn).expect("read fixture tags");
    crate::db::load_all_cards(conn)
        .expect("read fixture cards")
        .iter()
        .map(|card| crate::embed::doc_for_row(card, &tags))
        .collect()
}

/// Check every rebuilt vector and its associated SQL name after publication.
fn assert_encoded_index(lock: &StoreLock, conn: &Connection) {
    let store = VectorStore::load_owned_locked(lock).expect("read published index");
    let cards = crate::db::load_all_cards(conn).expect("read published cards");
    let names: Vec<String> = cards.iter().map(|card| card.name.clone()).collect();
    assert_eq!(store.meta.names, names);
    for (row, card) in cards.iter().enumerate() {
        assert_eq!(store.row(row), vector_for_name(&card.name));
    }
    assert!(
        crate::sync::load_pending_embeddings(conn)
            .expect("read pending work")
            .is_empty()
    );
}

/// Core ML boundary sizes and larger inputs use bounded, ordered model calls.
#[test]
fn embedding_batches_preserve_order_at_coreml_and_chunk_boundaries() {
    for count in [
        31,
        32,
        33,
        crate::setup::EMBED_CHUNK - 1,
        crate::setup::EMBED_CHUNK,
        crate::setup::EMBED_CHUNK + 1,
        2 * crate::setup::EMBED_CHUNK + 33,
    ] {
        let (_temp, paths, mut conn) = seed_current_index(count);
        let lock = StoreLock::acquire(&paths).expect("lock batch fixture");
        let collection = sql_rows(&conn, "collection");
        conn.execute("UPDATE cards SET oracle_text = 'updated rules'", [])
            .expect("change fixture documents");
        let expected = documents(&conn);
        let batches = RecordedBatches::default();
        let mut out = crate::output::Output::new(true, false, false);
        assert_eq!(
            embed_pending(&lock, &mut conn, &mut out, || {
                Ok(recording_model(&batches, None))
            })
            .expect("embed fixture batches"),
            count
        );
        let calls = batches.borrow();
        assert_eq!(
            calls.iter().map(Vec::len).collect::<Vec<_>>(),
            expected
                .chunks(crate::setup::EMBED_CHUNK)
                .map(<[String]>::len)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            calls.iter().flatten().cloned().collect::<Vec<_>>(),
            expected
        );
        assert_encoded_index(&lock, &conn);
        assert_eq!(sql_rows(&conn, "collection"), collection);
    }
}

/// Valid current files need neither a model nor a replacement publication.
#[test]
fn valid_index_without_pending_documents_does_not_load_a_model() {
    let (_temp, paths, mut conn) = seed_current_index(33);
    let lock = StoreLock::acquire(&paths).expect("lock current index");
    let vectors = std::fs::read(paths.vectors_file()).expect("read current vectors");
    let status = std::fs::read(paths.status_file()).expect("read current status");
    let mut out = crate::output::Output::new(true, false, false);
    assert_eq!(
        embed_pending(&lock, &mut conn, &mut out, || anyhow::bail!(
            "unchanged index must not load a model"
        ))
        .expect("validate current index"),
        0
    );
    assert_eq!(
        std::fs::read(paths.vectors_file()).expect("read vectors"),
        vectors
    );
    assert_eq!(
        std::fs::read(paths.status_file()).expect("read status"),
        status
    );
}

/// Missing bytes must queue every acknowledged document even without SQL changes.
#[test]
fn missing_and_invalid_matrix_sizes_rebuild_without_a_card_delta() {
    for damage in ["missing", "partial row", "missing row", "extra row"] {
        let count = 33;
        let (_temp, paths, mut conn) = seed_current_index(count);
        let lock = StoreLock::acquire(&paths).expect("lock damaged index");
        let cards = sql_rows(&conn, "cards");
        let collection = sql_rows(&conn, "collection");
        assert!(
            pending_documents(&mut conn, false)
                .expect("read current documents")
                .is_empty()
        );
        let mut bytes = std::fs::read(paths.vectors_file()).expect("read fixture matrix");
        match damage {
            "missing" => std::fs::remove_file(paths.vectors_file()).expect("remove matrix"),
            _ => {
                match damage {
                    "partial row" => bytes.truncate(bytes.len() - 1),
                    "missing row" => bytes.truncate(bytes.len() - crate::embed::DIM * 4),
                    "extra row" => bytes.extend(vec![0; crate::embed::DIM * 4]),
                    _ => unreachable!("known damage fixture"),
                }
                std::fs::write(paths.vectors_file(), bytes).expect("damage fixture matrix");
            }
        }
        let batches = RecordedBatches::default();
        let mut out = crate::output::Output::new(true, false, false);
        assert_eq!(
            embed_pending(&lock, &mut conn, &mut out, || {
                let inspection = crate::db::open(&paths.db()).expect("inspect durable queue");
                assert_eq!(
                    crate::sync::load_pending_embeddings(&inspection)
                        .expect("read durable queue")
                        .len(),
                    count
                );
                Ok(recording_model(&batches, None))
            })
            .expect("repair damaged matrix"),
            count,
            "{damage}"
        );
        assert_encoded_index(&lock, &conn);
        assert_eq!(batches.borrow()[0].len(), count);
        assert_eq!(sql_rows(&conn, "cards"), cards);
        assert_eq!(sql_rows(&conn, "collection"), collection);
        assert_eq!(
            embed_pending(&lock, &mut conn, &mut out, || anyhow::bail!(
                "repaired matrix must not load another model"
            ))
            .expect("validate repaired matrix"),
            0
        );
    }
}

/// An empty card table can restore its missing matrix without loading a model.
#[test]
fn missing_empty_matrix_is_recreated_without_a_model() {
    let (_temp, paths, mut conn) = seed_current_index(0);
    let lock = StoreLock::acquire(&paths).expect("lock empty index");
    let collection = sql_rows(&conn, "collection");
    std::fs::remove_file(paths.vectors_file()).expect("remove empty matrix");
    let mut out = crate::output::Output::new(true, false, false);
    assert_eq!(
        embed_pending(&lock, &mut conn, &mut out, || anyhow::bail!(
            "empty corpus must not load a model"
        ))
        .expect("repair empty matrix"),
        0
    );
    assert_eq!(
        std::fs::metadata(paths.vectors_file())
            .expect("read matrix metadata")
            .len(),
        0
    );
    assert!(
        !needs_rebuild(
            &lock,
            &Status::read(&paths.status_file()).expect("read status")
        )
        .expect("validate empty matrix")
    );
    assert_eq!(sql_rows(&conn, "collection"), collection);
}

/// One malformed response cannot update even the earlier rows of its batch.
#[test]
fn invalid_batch_counts_and_dimensions_are_rejected_before_applying_rows() {
    for failure in [
        BatchFailure::MissingVector,
        BatchFailure::ExtraVector,
        BatchFailure::ShortDimension,
        BatchFailure::LongDimension,
    ] {
        let (_temp, paths, mut conn) = seed_current_index(33);
        let lock = StoreLock::acquire(&paths).expect("lock invalid-batch fixture");
        conn.execute("UPDATE cards SET oracle_text = 'updated rules'", [])
            .expect("change fixture documents");
        let pending = pending_documents(&mut conn, false).expect("queue fixture documents");
        let mut store = VectorStore::load_owned_locked(&lock).expect("read prior index");
        let before: Vec<Vec<f32>> = (0..store.len())
            .map(|row| store.row(row).to_vec())
            .collect();
        let batches = RecordedBatches::default();
        let mut model = recording_model(&batches, Some((1, failure)));
        let mut out = crate::output::Output::new(true, false, false);
        assert!(embed_batches(&mut store, &pending, model.as_mut(), &mut out).is_err());
        assert_eq!(batches.borrow().len(), 1);
        assert_eq!(
            (0..store.len())
                .map(|row| store.row(row).to_vec())
                .collect::<Vec<_>>(),
            before,
            "{failure:?}"
        );
    }
}

/// Successful earlier batches remain unpublished and unacknowledged after failure.
#[test]
fn partial_batch_failure_retries_all_exact_revisions_after_reopening() {
    for failure in [
        BatchFailure::Error,
        BatchFailure::MissingVector,
        BatchFailure::ExtraVector,
        BatchFailure::ShortDimension,
        BatchFailure::LongDimension,
    ] {
        let count = crate::setup::EMBED_CHUNK + 33;
        let (_temp, paths, mut conn) = seed_current_index(count);
        let lock = StoreLock::acquire(&paths).expect("lock retry fixture");
        let collection = sql_rows(&conn, "collection");
        let published = sql_rows(&conn, "embedding_revisions");
        let vectors = std::fs::read(paths.vectors_file()).expect("read previous vectors");
        let status = std::fs::read(paths.status_file()).expect("read previous status");
        conn.execute("UPDATE cards SET oracle_text = 'updated rules'", [])
            .expect("change fixture documents");
        pending_documents(&mut conn, false).expect("queue fixture documents");
        let queued = sql_rows(&conn, "pending_embeddings");
        let batches = RecordedBatches::default();
        let mut out = crate::output::Output::new(true, false, false);
        assert!(
            embed_pending(&lock, &mut conn, &mut out, || {
                Ok(recording_model(&batches, Some((2, failure))))
            })
            .is_err()
        );
        assert_eq!(
            batches.borrow().iter().map(Vec::len).collect::<Vec<_>>(),
            [crate::setup::EMBED_CHUNK, 33]
        );
        assert_eq!(sql_rows(&conn, "pending_embeddings"), queued);
        assert_eq!(sql_rows(&conn, "embedding_revisions"), published);
        assert_eq!(sql_rows(&conn, "collection"), collection);
        assert_eq!(
            std::fs::read(paths.vectors_file()).expect("read vectors"),
            vectors
        );
        assert_eq!(
            std::fs::read(paths.status_file()).expect("read status"),
            status
        );
        drop(conn);
        drop(lock);

        let lock = StoreLock::acquire(&paths).expect("lock reopened store");
        let mut conn = crate::db::open(&paths.db()).expect("reopen retry database");
        assert_eq!(sql_rows(&conn, "pending_embeddings"), queued);
        let retry_batches = RecordedBatches::default();
        assert_eq!(
            embed_pending(&lock, &mut conn, &mut out, || {
                Ok(recording_model(&retry_batches, None))
            })
            .expect("retry complete batches"),
            count
        );
        assert_eq!(
            retry_batches
                .borrow()
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            documents(&conn)
        );
        assert_encoded_index(&lock, &conn);
        assert_eq!(sql_rows(&conn, "collection"), collection);
    }
}

/// A failed repair keeps its full rebuild queue durable until a later retry.
#[test]
fn missing_matrix_repair_survives_a_later_batch_failure() {
    let count = crate::setup::EMBED_CHUNK + 33;
    let (_temp, paths, mut conn) = seed_current_index(count);
    let lock = StoreLock::acquire(&paths).expect("lock repair retry fixture");
    let cards = sql_rows(&conn, "cards");
    let collection = sql_rows(&conn, "collection");
    let published = sql_rows(&conn, "embedding_revisions");
    let status = std::fs::read(paths.status_file()).expect("read prior status");
    std::fs::remove_file(paths.vectors_file()).expect("remove fixture matrix");
    let batches = RecordedBatches::default();
    let mut out = crate::output::Output::new(true, false, false);
    assert!(
        embed_pending(&lock, &mut conn, &mut out, || {
            Ok(recording_model(&batches, Some((2, BatchFailure::Error))))
        })
        .is_err()
    );
    let queued = sql_rows(&conn, "pending_embeddings");
    assert_eq!(queued.len(), count);
    assert_eq!(sql_rows(&conn, "embedding_revisions"), published);
    assert!(!paths.vectors_file().exists());
    assert_eq!(
        std::fs::read(paths.status_file()).expect("read status"),
        status
    );
    drop(conn);
    drop(lock);

    let lock = StoreLock::acquire(&paths).expect("lock reopened repair fixture");
    let mut conn = crate::db::open(&paths.db()).expect("reopen repair fixture");
    assert_eq!(sql_rows(&conn, "pending_embeddings"), queued);
    let retry_batches = RecordedBatches::default();
    assert_eq!(
        embed_pending(&lock, &mut conn, &mut out, || {
            Ok(recording_model(&retry_batches, None))
        })
        .expect("retry missing matrix repair"),
        count
    );
    assert_encoded_index(&lock, &conn);
    assert_eq!(sql_rows(&conn, "cards"), cards);
    assert_eq!(sql_rows(&conn, "collection"), collection);
}

/// Incremental updates retain old row positions while new names append in SQL order.
#[test]
fn mixed_updates_and_appends_keep_card_vector_alignment() {
    let (_temp, paths, mut conn) = seed_current_index(33);
    let lock = StoreLock::acquire(&paths).expect("lock incremental fixture");
    let previous = VectorStore::load_owned_locked(&lock).expect("read old matrix");
    let collection = sql_rows(&conn, "collection");
    conn.execute_batch(
        "UPDATE cards SET oracle_text = 'updated rules' WHERE name IN ('Card 0031', 'Card 0001');
         INSERT INTO cards (name, oracle_id) VALUES ('Card 0033', 'new-33'), ('Card 0034', 'new-34');",
    )
    .expect("change and append fixture cards");
    let batches = RecordedBatches::default();
    let mut out = crate::output::Output::new(true, false, false);
    assert_eq!(
        embed_pending(&lock, &mut conn, &mut out, || {
            Ok(recording_model(&batches, None))
        })
        .expect("apply incremental batches"),
        4
    );
    let current = VectorStore::load_owned_locked(&lock).expect("read current matrix");
    let cards = crate::db::load_all_cards(&conn).expect("read current cards");
    assert_eq!(
        current.meta.names,
        cards
            .iter()
            .map(|card| card.name.clone())
            .collect::<Vec<_>>()
    );
    let updated = ["Card 0031", "Card 0001", "Card 0033", "Card 0034"];
    for (row, name) in current.meta.names.iter().enumerate() {
        if updated.contains(&name.as_str()) {
            assert_eq!(current.row(row), vector_for_name(name));
        } else {
            assert_eq!(current.row(row), previous.row(row));
        }
    }
    assert_eq!(batches.borrow()[0].len(), 4);
    assert_eq!(sql_rows(&conn, "collection"), collection);
}

/// An unrelated filesystem failure must not be converted into a rebuild request.
#[cfg(unix)]
#[test]
fn index_read_errors_propagate_before_queueing_or_loading_a_model() {
    let (_temp, paths, mut conn) = seed_current_index(33);
    let lock = StoreLock::acquire(&paths).expect("lock I/O failure fixture");
    let collection = sql_rows(&conn, "collection");
    std::fs::remove_file(paths.vectors_file()).expect("remove fixture matrix");
    std::os::unix::fs::symlink("vectors.bin", paths.vectors_file()).expect("create symlink loop");
    let expected = std::fs::File::open(paths.vectors_file()).expect_err("symlink loop must fail");
    let mut out = crate::output::Output::new(true, false, false);
    let error = embed_pending(&lock, &mut conn, &mut out, || {
        anyhow::bail!("I/O failure must not load a model")
    })
    .expect_err("preserve filesystem failure");
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .expect("I/O error source")
            .kind(),
        expected.kind()
    );
    assert!(sql_rows(&conn, "pending_embeddings").is_empty());
    assert_eq!(sql_rows(&conn, "collection"), collection);
}

/// Permission failures retain their I/O cause when Unix modes restrict this runner.
#[cfg(unix)]
#[test]
fn unreadable_matrix_does_not_queue_a_rebuild() {
    use std::os::unix::fs::PermissionsExt as _;

    let (_temp, paths, mut conn) = seed_current_index(33);
    let lock = StoreLock::acquire(&paths).expect("lock permission fixture");
    let path = paths.vectors_file();
    let permissions = std::fs::metadata(&path)
        .expect("read original permissions")
        .permissions();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
        .expect("remove matrix read permission");
    let access = std::fs::File::open(&path);
    if access.is_ok() {
        // Privileged runners can bypass Unix file modes.
        std::fs::set_permissions(&path, permissions).expect("restore matrix permissions");
        return;
    }
    let mut out = crate::output::Output::new(true, false, false);
    let result = embed_pending(&lock, &mut conn, &mut out, || {
        anyhow::bail!("permission failure must not load a model")
    });
    std::fs::set_permissions(&path, permissions).expect("restore matrix permissions");
    assert_eq!(
        access.expect_err("matrix must be unreadable").kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        result
            .expect_err("preserve permission failure")
            .downcast_ref::<std::io::Error>()
            .expect("permission error source")
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(sql_rows(&conn, "pending_embeddings").is_empty());
}

/// A directory at the matrix path is an access problem, not missing index bytes.
#[test]
fn a_directory_at_the_matrix_path_does_not_queue_a_rebuild() {
    let (_temp, paths, mut conn) = seed_current_index(33);
    let lock = StoreLock::acquire(&paths).expect("lock directory fixture");
    std::fs::remove_file(paths.vectors_file()).expect("remove fixture matrix");
    std::fs::create_dir(paths.vectors_file()).expect("create matrix-path directory");
    let mut out = crate::output::Output::new(true, false, false);
    assert!(
        embed_pending(&lock, &mut conn, &mut out, || anyhow::bail!(
            "invalid file type must not load a model"
        ))
        .is_err()
    );
    assert!(paths.vectors_file().is_dir());
    assert!(sql_rows(&conn, "pending_embeddings").is_empty());
}
