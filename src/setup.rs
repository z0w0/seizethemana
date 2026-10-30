//! `stm setup`: download the Scryfall bulk files, ingest cards and tags,
//! refresh combos, and embed every card into the vector index.

use rusqlite::Connection;

use crate::cli;
use crate::output::Output;
use crate::paths::Paths;
use crate::scryfall;

/// Delete the old store files a forced rebuild replaces, in the order
/// database (with SQLite sidecars) → `status.json` → `vectors.bin`: a
/// failure partway leaves the store marked un-set-up, so reads fail loudly
/// instead of scoring new card ids against stale vectors. Files that do
/// not exist are skipped.
///
/// # Errors
/// Propagates filesystem failures other than NotFound.
fn remove_force_targets(
    lock: &crate::sync::store_lock::StoreLock,
    out: &mut Output,
) -> anyhow::Result<()> {
    let paths = lock.paths();
    crate::sync::recovery::discard_publication(lock)?;
    let db_path = paths.db();
    for suffix in ["", "-wal", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", db_path.display()));
        match std::fs::remove_file(&sidecar) {
            Ok(()) => out.status("Removed", &format!("old database at {}", sidecar.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(anyhow::Error::new(err)
                    .context(format!("removing old database {}", sidecar.display())));
            }
        }
    }
    let status_path = paths.status_file();
    if let Err(err) = std::fs::remove_file(&status_path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        return Err(
            anyhow::Error::new(err).context(format!("removing status {}", status_path.display()))
        );
    }
    let vectors_path = paths.vectors_file();
    if let Err(err) = std::fs::remove_file(&vectors_path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        return Err(
            anyhow::Error::new(err).context(format!("removing vectors {}", vectors_path.display()))
        );
    }
    Ok(())
}

/// Documents embedded per batch; sized so one batch fits the model's
/// internal batching without overshooting memory.
pub const EMBED_CHUNK: usize = 256;

/// State the data loss before a forced setup removes the old store.
const FORCE_REBUILD_WARNING: &str = "--force deletes the collection database, card index, and setup status. Back up the data directory first. Saved decklists and primers are stored separately and remain.";

/// Reset the store without leaving an old connection attached to deleted files.
fn force_rebuild(
    lock: &crate::sync::store_lock::StoreLock,
    out: &mut Output,
    conn: &mut Connection,
) -> anyhow::Result<()> {
    let paths = lock.paths();
    out.warning(FORCE_REBUILD_WARNING);
    let old = std::mem::replace(conn, Connection::open_in_memory()?);
    old.close().map_err(|(_, err)| err)?;
    remove_force_targets(lock, out)?;
    *conn = crate::db::open(&paths.db())?;
    out.status("Created", "empty card and collection database");
    Ok(())
}

/// First card row of chunk `chunk_index`: `chunk_index * EMBED_CHUNK`.
///
/// Pure so the row-alignment math is unit-testable without a model.
fn chunk_row_base(chunk_index: usize) -> usize {
    chunk_index * EMBED_CHUNK
}

/// Run `stm setup`: download the bulk files and ingest all cards + tags.
///
/// Idempotent unless `--force` is given; writes status.json last so a failed
/// run leaves the store marked un-set-up.
///
/// `--force` deletes the database file and vector index first. Ordinary
/// schema migrations run in place and do not need a forced rebuild.
/// Deleting `status.json` and `vectors.bin` up front means a failed run
/// leaves the store marked un-set-up (reads fail loudly with "run
/// `stm setup`") instead of scoring new card ids against stale vectors.
pub fn run_setup(
    paths: &Paths,
    out: &mut Output,
    conn: &mut Connection,
    force: bool,
) -> anyhow::Result<i32> {
    std::fs::create_dir_all(paths.root())?;
    let lock = crate::sync::store_lock::StoreLock::acquire(paths)?;
    if !force {
        crate::db::reopen(conn, &paths.db())?;
        crate::sync::recovery::recover_publication(&lock)?;
    }
    if paths.is_setup() && !force {
        out.finish(
            "Ready",
            "card search is already set up; run 'stm sync' to update cards and prices",
            std::time::Duration::ZERO,
        );
        return Ok(cli::codes::OK);
    }
    if force {
        force_rebuild(&lock, out, conn)?;
    }
    paths.ensure_dirs()?;
    let setup_start = std::time::Instant::now();

    let started = std::time::Instant::now();
    out.start_task("Fetching", "Scryfall download links", 0);
    let bulk = scryfall::fetch_bulk_files();
    out.clear_progress();
    let (cards_bulk, tags_bulk) = bulk?;
    out.finish("Fetched", "Scryfall download links", started.elapsed());

    // A retry can reuse fresh downloads from an incomplete setup.
    let cards_dest = paths.bulk_file();
    scryfall::ensure_fresh_bulk_named(&cards_bulk, &cards_dest, out, "Scryfall cards")?;
    let tags_dest = paths.tags_file();
    scryfall::ensure_fresh_bulk_named(&tags_bulk, &tags_dest, out, "Scryfall tags")?;

    // Card tags must be searchable before building embedding documents.
    let started = std::time::Instant::now();
    out.start_task("Loading", "cards and prices", 0);
    let delta = crate::sync::sync_cards_locked(
        &lock,
        conn,
        &cards_dest,
        out,
        &chrono::Utc::now().to_rfc3339(),
    );
    out.clear_progress();
    let delta = delta?;
    let cards: i64 = conn.query_row("SELECT COUNT(*) FROM cards", [], |row| row.get(0))?;
    out.finish(
        "Loaded",
        &format!(
            "cards and prices ({} cards)",
            crate::output::grouped_int(cards)
        ),
        started.elapsed(),
    );
    let started = std::time::Instant::now();
    out.start_task("Indexing", "card tags", 0);
    // Make the labels searchable: tag labels are FTS content (weight second
    // to name), so role words resolve through the community vocabulary.
    let tags = crate::tags::ingest(conn, &tags_dest)
        .and_then(|delta| crate::db::refresh_tags_text(conn).map(|tagged| (delta, tagged)));
    out.clear_progress();
    let (tags_delta, tagged) = tags?;
    out.finish(
        "Indexed",
        &format!(
            "card tags ({} cards, {} tags)",
            crate::output::grouped_int(tagged as i64),
            crate::output::grouped_int(tags_delta.tags as i64)
        ),
        started.elapsed(),
    );

    // Combo downloads are optional. A failed refresh leaves their freshness
    // stamp empty so sync retries, but an empty valid source still counts as success.
    let now = chrono::Utc::now();
    let refresh = crate::spellbook::ensure_fresh_variants(&paths.combos_file(), out, false);
    let combos_delta = match &refresh {
        Ok(_) => crate::spellbook::ingest(conn, &paths.combos_file(), out, &now.to_rfc3339())?,
        Err(err) => {
            out.warning(&format!("could not update Commander Spellbook combos; setup will continue. Run 'stm sync' to retry: {err:#}"));
            0
        }
    };
    let combos_ok = refresh.is_ok();

    let store = embed_all(paths, out, conn)?;

    // Mark card and price data fresh only after the search index is ready.
    let status = crate::paths::Status {
        setup_complete: true,
        ingested_cards: delta.added,
        embedded_cards: store.meta.names.len(),
        model: store.meta.model.clone(),
        dim: store.meta.dim,
        names: store.meta.names.clone(),
        scryfall_synced_at: chrono::Utc::now().to_rfc3339(),
        doc_version: crate::embed::DOC_VERSION,
        // A failed combo refresh leaves the stamp empty: the store looks
        // unsynced for combos and the next sync retries them.
        combos_synced_at: if combos_ok {
            chrono::Utc::now().to_rfc3339()
        } else {
            Default::default()
        },
    };
    out.start_task("Finishing", "setup", 0);
    let saved = status
        .write(&paths.status_file())
        .and_then(|()| crate::sync::recovery::complete_setup(&lock, conn));
    out.clear_progress();
    saved?;
    out.finish(
        "Finished",
        &format!(
            "setup ({} searchable cards, {} combos loaded)",
            crate::output::grouped_int(store.meta.names.len() as i64),
            crate::output::grouped_int(combos_delta as i64)
        ),
        setup_start.elapsed(),
    );
    Ok(cli::codes::OK)
}

/// Embed every card, writing `vectors.bin` under the data root.
///
/// Batches the embedding work in 256-document chunks. Terminals draw a bar;
/// redirected or plain terminals get periodic status lines. The caller writes
/// index metadata to `status.json` after this returns.
fn embed_all(
    paths: &Paths,
    out: &mut Output,
    conn: &Connection,
) -> anyhow::Result<crate::embed::VectorStore> {
    embed_all_with_model(paths, out, conn, || {
        crate::embed::load_model(&paths.models_dir(), false)
    })
}

/// Build the initial index with a supplied model loader for offline checks.
pub(crate) fn embed_all_with_model(
    paths: &Paths,
    out: &mut Output,
    conn: &Connection,
    load_model: impl FnOnce() -> anyhow::Result<Box<dyn crate::embed::DocumentEmbedder>>,
) -> anyhow::Result<crate::embed::VectorStore> {
    let cards = crate::db::load_all_cards(conn)?;
    let tag_index = crate::tags::TagIndex::load(conn)?;
    let model_start = std::time::Instant::now();
    out.start_task("Loading", "search model (downloads files if needed)", 0);
    let model = load_model();
    out.clear_progress();
    let mut model = model?;
    out.finish("Loaded", "search model", model_start.elapsed());
    let mut store = crate::embed::VectorStore::new();

    let docs: Vec<String> = cards
        .iter()
        .map(|card| crate::embed::doc_for_row(card, &tag_index))
        .collect();
    let start = std::time::Instant::now();
    let total = docs.len() as u64;
    let plain_progress = !out.supports_progress();
    let mut last_reported = 0;
    out.start_task("Building", "card search index", total);
    let result = (|| -> anyhow::Result<()> {
        for (chunk_index, chunk) in docs.chunks(EMBED_CHUNK).enumerate() {
            let vectors = crate::embed::embed_texts(model.as_mut(), chunk)?;
            for (doc_index, vector) in vectors.into_iter().enumerate() {
                let card_index = chunk_row_base(chunk_index) + doc_index;
                store.push(&cards[card_index].name, vector)?;
            }
            out.tick_progress(chunk.len() as u64);
            let completed = chunk_row_base(chunk_index) + chunk.len();
            if plain_progress
                && should_report_embed_checkpoint(docs.len(), completed, last_reported)
            {
                last_reported = completed;
                out.status(
                    "Building",
                    &format!(
                        "card search index ({}/{total} cards)",
                        crate::output::grouped_int(completed as i64),
                        total = crate::output::grouped_int(total as i64)
                    ),
                );
            }
        }
        store.save_vectors(paths.root())
    })();
    out.clear_progress();
    result?;
    out.finish(
        "Built",
        &format!(
            "card search index ({} cards)",
            crate::output::grouped_int(total as i64)
        ),
        start.elapsed(),
    );
    Ok(store)
}

/// Keep plain embedding checkpoints near ten per run, including the final batch.
fn should_report_embed_checkpoint(total: usize, completed: usize, last_reported: usize) -> bool {
    total > 0 && (completed == total || completed - last_reported >= (total / 10).max(EMBED_CHUNK))
}

/// Offline checks for embedding checkpoint frequency and batch counts.
#[cfg(test)]
#[path = "tests/output_commands/setup_progress_tests.rs"]
mod setup_progress_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Child entry point for captured warnings from the real reset path.
    #[test]
    fn force_reset_worker() {
        let Some(root) = std::env::var_os("STM_FORCE_RESET_ROOT") else {
            return;
        };
        let paths = Paths::new(root.into());
        let mut conn = crate::db::open(&paths.db()).unwrap();
        let mut out = Output::new(false, false, false);
        let lock = crate::sync::store_lock::StoreLock::acquire(&paths).unwrap();
        force_rebuild(&lock, &mut out, &mut conn).unwrap();
        let contender = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(paths.root().join("store.lock"))
            .unwrap();
        let error = fs2::FileExt::try_lock_exclusive(&contender).unwrap_err();
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::WouldBlock,
            "reset must retain the locked file"
        );
        let count: i64 = conn
            .query_row("SELECT count(*) FROM collection", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    /// A scripted forced reset must warn, erase ownership, and retain deck files.
    #[test]
    fn force_reset_warns_before_deletion_and_keeps_deck_files_without_a_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::new(temp.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        std::fs::create_dir_all(paths.decks_dir()).unwrap();
        std::fs::write(paths.deck_file("Keep"), "// DECK\n1 Island\n").unwrap();
        let primer = paths.decks_dir().join("Keep.md");
        std::fs::write(&primer, "My primer\n").unwrap();
        let mut conn = crate::db::open(&paths.db()).unwrap();
        conn.execute(
            "INSERT INTO collection (name, binder, binder_type, quantity)
            VALUES ('Island', 'Keep', 'binder', 4)",
            [],
        )
        .unwrap();
        std::fs::write(paths.status_file(), b"old status").unwrap();
        std::fs::write(paths.vectors_file(), b"old vectors").unwrap();
        let publication = paths.root().join("sync-publication");
        std::fs::create_dir(&publication).unwrap();
        for name in ["ready", "vectors.bin", "status.json"] {
            std::fs::write(publication.join(name), b"old publication").unwrap();
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "setup::tests::force_reset_worker", "--nocapture"])
            .env("STM_FORCE_RESET_ROOT", paths.root())
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        let stderr = String::from_utf8(child.stderr).unwrap();
        let warning = stderr
            .find("--force deletes the collection database")
            .unwrap();
        let deletion = stderr.find("Removed").unwrap();
        assert!(
            warning < deletion,
            "warning must precede deletion: {stderr}"
        );
        assert!(stderr.contains("Back up the data directory first"));
        assert!(stderr.contains("decklists and primers"));
        assert_eq!(
            std::fs::read_to_string(paths.deck_file("Keep")).unwrap(),
            "// DECK\n1 Island\n"
        );
        assert_eq!(std::fs::read_to_string(primer).unwrap(), "My primer\n");
        assert!(!paths.status_file().exists());
        assert!(!paths.vectors_file().exists());
        assert!(
            !publication.exists(),
            "reset must not replay an old index publication"
        );
        let _lock = crate::sync::store_lock::StoreLock::acquire(&paths).unwrap();
        crate::db::reopen(&mut conn, &paths.db()).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM collection", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn chunk_row_base_aligns_docs_with_card_rows() {
        // The embed loop maps (chunk_index, doc_index) onto the card list.
        // Pin the exact math so an off-by-one cannot hide: every doc must
        // land on its own card row, in order, across a non-multiple chunk
        // count.
        let total = 1_000; // 3 full chunks of 256 + 1 tail chunk of 232
        let mut seen = Vec::new();
        for (chunk_index, chunk) in (0..total)
            .collect::<Vec<usize>>()
            .chunks(EMBED_CHUNK)
            .enumerate()
        {
            for (doc_index, doc) in chunk.iter().enumerate() {
                seen.push((chunk_row_base(chunk_index) + doc_index, *doc));
            }
        }
        // Every doc d (0-based) lands on row d, with no gaps or overlaps.
        for (row, doc) in &seen {
            assert_eq!(row, doc, "doc {doc} misaligned onto row {row}");
        }
        assert_eq!(seen.len(), total);
        assert_eq!(
            seen.last(),
            Some(&(total - 1, total - 1)),
            "the last card is embedded, not skipped"
        );
    }

    /// Force-rebuild removes the db (with sidecars), status.json, and
    /// vectors.bin, and skips files that do not exist. A leftover status
    /// or vector file after a failed rebuild would let reads score new
    /// card ids against stale vectors.
    #[test]
    fn force_rebuild_removes_db_status_and_vectors() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
        std::fs::write(paths.db(), b"x").unwrap();
        std::fs::write(paths.db().with_extension("db-wal"), b"x").unwrap();
        std::fs::write(paths.status_file(), b"x").unwrap();
        std::fs::write(paths.vectors_file(), b"x").unwrap();
        let mut out = crate::output::Output::new(true, false, false);
        let lock = crate::sync::store_lock::StoreLock::acquire(&paths).unwrap();
        remove_force_targets(&lock, &mut out).unwrap();
        assert!(!paths.db().exists());
        assert!(!paths.db().with_extension("db-wal").exists());
        assert!(!paths.status_file().exists());
        assert!(!paths.vectors_file().exists());
        // Idempotent: a second pass over missing files succeeds.
        remove_force_targets(&lock, &mut out).unwrap();
    }
}
