//! Durable document work and restart recovery for vector publication.

use super::store_lock::StoreLock;
use anyhow::Context;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

/// One exact document revision waiting for vector publication.
struct PendingDocument {
    name: String,
    revision: String,
    document: String,
}

/// Identify the document, layout, and model used to build a vector.
fn revision(document: &str) -> String {
    let input = format!(
        "{}\n{}\n{document}",
        crate::embed::DOC_VERSION,
        crate::embed::MODEL_NAME
    );
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

/// Queue card revisions inside the caller's card-update transaction.
pub(super) fn enqueue_cards<'a>(
    conn: &Connection,
    names: impl Iterator<Item = &'a str>,
) -> anyhow::Result<()> {
    let tags = crate::tags::TagIndex::load(conn)?;
    let mut insert = conn.prepare(
        "INSERT INTO pending_embeddings (name, revision) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET revision = excluded.revision",
    )?;
    for name in names {
        let card = crate::db::get_card(conn, name)?.context("queued card is missing")?;
        let document = crate::embed::doc_for_row(&card, &tags);
        insert.execute(rusqlite::params![name, revision(&document)])?;
    }
    Ok(())
}

/// Find all unpublished documents, including tag changes from an interrupted run.
/// A metadata mismatch queues every card, even if its hash matches.
fn pending_documents(
    conn: &mut Connection,
    reembed_all: bool,
) -> anyhow::Result<Vec<PendingDocument>> {
    let tags = crate::tags::TagIndex::load(conn)?;
    let cards = crate::db::load_all_cards(conn)?;
    let tx = conn.transaction()?;
    let mut pending = Vec::new();
    {
        let mut stored = tx.prepare("SELECT revision FROM embedding_revisions WHERE name = ?1")?;
        let mut queued = tx.prepare("SELECT 1 FROM pending_embeddings WHERE name = ?1")?;
        let mut insert = tx.prepare(
            "INSERT INTO pending_embeddings (name, revision) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET revision = excluded.revision",
        )?;
        for card in cards {
            let document = crate::embed::doc_for_row(&card, &tags);
            let revision = revision(&document);
            let published: Option<String> =
                stored.query_row([&card.name], |r| r.get(0)).optional()?;
            let already_queued = queued.exists([&card.name])?;
            if reembed_all || published.as_deref() != Some(&revision) || already_queued {
                insert.execute(rusqlite::params![card.name, revision])?;
                pending.push(PendingDocument {
                    name: card.name,
                    revision,
                    document,
                });
            }
        }
    }
    tx.commit()?;
    Ok(pending)
}

/// Acknowledge only the exact revisions saved with the published matrix.
fn complete_documents(conn: &mut Connection, pending: &[PendingDocument]) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut published = tx.prepare(
            "INSERT INTO embedding_revisions (name, revision)
             SELECT name, revision FROM pending_embeddings WHERE name = ?1 AND revision = ?2
             ON CONFLICT(name) DO UPDATE SET revision = excluded.revision",
        )?;
        let mut clear =
            tx.prepare("DELETE FROM pending_embeddings WHERE name = ?1 AND revision = ?2")?;
        for item in pending {
            published.execute(rusqlite::params![item.name, item.revision])?;
            clear.execute(rusqlite::params![item.name, item.revision])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Decide whether the published matrix needs replacement without hiding I/O errors.
fn needs_rebuild(lock: &StoreLock, status: &crate::paths::Status) -> anyhow::Result<bool> {
    let path = lock.paths().vectors_file();
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let metadata = file
        .metadata()
        .with_context(|| format!("reading {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file(),
        "{} is not a regular file",
        path.display()
    );
    let row_bytes = u64::try_from(crate::embed::DIM * std::mem::size_of::<f32>())?;
    let valid_size = metadata.len().is_multiple_of(row_bytes)
        && metadata.len() / row_bytes == u64::try_from(status.names.len())?;
    Ok(!valid_size
        || status.model != crate::embed::MODEL_NAME
        || status.dim != crate::embed::DIM
        || status.doc_version != crate::embed::DOC_VERSION)
}

/// Embed bounded batches without publishing or acknowledging incomplete work.
fn embed_batches(
    store: &mut crate::embed::VectorStore,
    pending: &[PendingDocument],
    model: &mut dyn crate::embed::DocumentEmbedder,
    out: &mut crate::output::Output,
) -> anyhow::Result<()> {
    // Pending names are unique. New cards append in SQL order, so only the
    // existing rows need lookup entries for this embedding run.
    let rows_by_name: std::collections::HashMap<String, usize> = store
        .meta
        .names
        .iter()
        .cloned()
        .enumerate()
        .map(|(row, name)| (name, row))
        .collect();
    let mut embedded = 0;
    for chunk in pending.chunks(crate::setup::EMBED_CHUNK) {
        let documents: Vec<String> = chunk.iter().map(|item| item.document.clone()).collect();
        let vectors = crate::embed::embed_texts(model, &documents)?;
        anyhow::ensure!(
            vectors.len() == chunk.len(),
            "model returned {} embeddings for {} documents",
            vectors.len(),
            chunk.len()
        );
        anyhow::ensure!(
            vectors
                .iter()
                .all(|vector| vector.len() == crate::embed::DIM),
            "model returned the wrong vector dimension"
        );
        for (item, vector) in chunk.iter().zip(vectors) {
            match rows_by_name.get(&item.name).copied() {
                Some(row) => store.row_mut(row).copy_from_slice(&vector),
                None => store.push(&item.name, vector)?,
            }
            embedded += 1;
            super::status::tick_embed_progress(out, pending.len(), embedded);
        }
    }
    Ok(())
}

/// Validate the published matrix and embed durable work with an injectable model loader.
pub(super) fn embed_pending(
    lock: &StoreLock,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    load_model: impl FnOnce() -> anyhow::Result<Box<dyn crate::embed::DocumentEmbedder>>,
) -> anyhow::Result<usize> {
    recover_publication(lock)?;
    let status = crate::paths::Status::read(&lock.paths().status_file())?;
    let rebuild = needs_rebuild(lock, &status)?;
    let pending = pending_documents(conn, rebuild)?;
    if pending.is_empty() {
        if rebuild {
            publish_store(lock, &crate::embed::VectorStore::new())?;
        }
        return Ok(0);
    }
    let mut store = if rebuild {
        crate::embed::VectorStore::new()
    } else {
        crate::embed::VectorStore::load_owned_locked(lock)?
    };
    let model_started = std::time::Instant::now();
    out.start_task("Loading", "search model (downloads files if needed)", 0);
    let model = load_model();
    out.clear_progress();
    let mut model = model?;
    out.finish("Loaded", "search model", model_started.elapsed());
    out.progress_bar("Embedding", "cards", pending.len() as u64);
    let embedded = embed_batches(&mut store, &pending, model.as_mut(), out);
    out.clear_progress();
    embedded?;
    publish_store(lock, &store)?;
    complete_documents(conn, &pending)?;
    Ok(pending.len())
}

/// Stage a complete pair before replacing either public index file.
fn publish_store(lock: &StoreLock, store: &crate::embed::VectorStore) -> anyhow::Result<()> {
    anyhow::ensure!(
        store.meta.model == crate::embed::MODEL_NAME && store.meta.dim == crate::embed::DIM,
        "cannot publish vectors for a different embedding model or dimension"
    );
    let paths = lock.paths();
    let staging = crate::paths::Paths::new(paths.root().join("sync-publication"));
    store.save_vectors(staging.root())?;
    let mut status = crate::paths::Status::read(&paths.status_file())?;
    status.names = store.meta.names.clone();
    status.ingested_cards = store.len();
    status.embedded_cards = store.len();
    status.model = crate::embed::MODEL_NAME.to_string();
    status.dim = crate::embed::DIM;
    status.doc_version = crate::embed::DOC_VERSION;
    status.write(&staging.status_file())?;
    for path in [staging.vectors_file(), staging.status_file()] {
        sync_file(&path)?;
    }
    let marker = std::fs::File::create(staging.root().join("ready"))?;
    marker.sync_all()?;
    recover_publication(lock)
}

/// Finish an interrupted publication before loading the public vector matrix.
pub(crate) fn recover_publication(lock: &StoreLock) -> anyhow::Result<()> {
    let paths = lock.paths();
    let staging = crate::paths::Paths::new(paths.root().join("sync-publication"));
    let marker = staging.root().join("ready");
    if !marker.try_exists()? {
        return Ok(());
    }
    // Keep both staged files until the public pair is complete. A restart
    // repeats these replacements even if the previous process wrote one.
    let tmp = paths.root().join("vectors.bin.tmp");
    std::fs::copy(staging.vectors_file(), &tmp)?;
    std::fs::rename(tmp, paths.vectors_file())?;
    crate::paths::Status::read(&staging.status_file())?.write(&paths.status_file())?;
    for path in [paths.vectors_file(), paths.status_file()] {
        sync_file(&path)?;
    }
    discard_publication(lock)
}

/// Flush staged and public files through a writable handle on every platform.
fn sync_file(path: &std::path::Path) -> anyhow::Result<()> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}

/// Remove only sync's known staging files, including during a forced reset.
pub(crate) fn discard_publication(lock: &StoreLock) -> anyhow::Result<()> {
    let paths = lock.paths();
    let dir = paths.root().join("sync-publication");
    for name in [
        "ready",
        "vectors.bin",
        "status.json",
        "vectors.bin.tmp",
        "status.json.tmp",
    ] {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    match std::fs::remove_dir(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Retire setup's queue only after its full index and status have been saved.
pub(crate) fn complete_setup(_lock: &StoreLock, conn: &mut Connection) -> anyhow::Result<()> {
    let pending = pending_documents(conn, false)?;
    complete_documents(conn, &pending)
}

/// Offline failure and process-restart tests.
#[cfg(test)]
#[path = "tests/recovery_tests.rs"]
mod tests;

/// Store-lock coverage through SQL changes, publication, and acknowledgement.
#[cfg(test)]
#[path = "tests/concurrency_tests.rs"]
mod concurrency_tests;

/// Full-index rebuilds after metadata mismatch, including process-restart recovery.
#[cfg(test)]
#[path = "tests/metadata_rebuild_tests.rs"]
mod metadata_rebuild_tests;

/// Index integrity, bounded embedding batches, and retry alignment without a model.
#[cfg(test)]
#[path = "tests/index_batch_tests.rs"]
mod index_batch_tests;
