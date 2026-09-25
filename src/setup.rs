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
fn remove_force_targets(paths: &Paths, out: &mut Output) -> anyhow::Result<()> {
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
/// `--force` deletes the database file and the vector index first. The
/// schema is a single in-place-edited migration (see `db.rs`), so a forced
/// rebuild is the only way a schema change reaches an existing store.
/// Deleting `status.json` and `vectors.bin` up front means a failed run
/// leaves the store marked un-set-up (reads fail loudly with "run
/// `stm setup`") instead of scoring new card ids against stale vectors.
pub fn run_setup(
    paths: &Paths,
    out: &mut Output,
    conn: &mut Connection,
    force: bool,
) -> anyhow::Result<i32> {
    if paths.is_setup() && !force {
        out.finish(
            "Already",
            "set up (use --force to rebuild)",
            std::time::Duration::ZERO,
        );
        return Ok(cli::codes::OK);
    }
    if force {
        remove_force_targets(paths, out)?;
        *conn = crate::db::open(&paths.db())?;
        out.status("Rebuilt", "empty database with the current schema");
    }
    paths.ensure_dirs()?;
    let setup_start = std::time::Instant::now();

    // 1. Find the current bulk files (cards + tags, one index call).
    out.status("Fetching", "Scryfall bulk index");
    let (cards_bulk, tags_bulk) = scryfall::fetch_bulk_files()?;

    // 2. Download what is missing or stale (mtime-based, so a daily
    // re-run actually picks up new data).
    let cards_dest = paths.bulk_file();
    scryfall::ensure_fresh_bulk(&cards_bulk, &cards_dest, out)?;
    let tags_dest = paths.tags_file();
    scryfall::ensure_fresh_bulk(&tags_bulk, &tags_dest, out)?;

    // 3. Load cards + prices, then tags (embedding needs tags, so tags
    // land first; identical pipeline to `stm sync`).
    out.status("Loading", "cards and prices");
    let delta = crate::sync::sync_cards(conn, &cards_dest, out, &chrono::Utc::now().to_rfc3339())?;
    out.finish(
        "Loaded",
        &format!("{} cards", crate::output::grouped_int(delta.added as i64)),
        std::time::Duration::ZERO,
    );
    out.status("Preparing", "tag search");
    let tags_delta = crate::tags::ingest(conn, &tags_dest)?;
    // Make the labels searchable: tag labels are FTS content (weight second
    // to name), so role words resolve through the community vocabulary.
    let tagged = crate::db::refresh_tags_text(conn)?;
    out.finish(
        "Indexed",
        &format!(
            "{} tagged cards ({} tags)",
            crate::output::grouped_int(tagged as i64),
            crate::output::grouped_int(tags_delta.tagged_cards as i64)
        ),
        std::time::Duration::ZERO,
    );

    // 3b. Combo variants (Commander Spellbook). A failed refresh warns and
    // continues: combos are additive diagnostics, never a blocker. The
    // status stamp below follows the sync policy: only a successful refresh
    // marks combos synced, so a failed one leaves the store retrying. The
    // ingested count is not the success signal: an empty-but-valid upstream
    // file would stamp a false failure and force a re-download every sync.
    out.status("Loading", "spell combos (commanderspellbook.com)");
    let now = chrono::Utc::now();
    let refresh = crate::spellbook::ensure_fresh_variants(&paths.combos_file(), out, false);
    let combos_delta = match &refresh {
        Ok(_) => crate::spellbook::ingest(conn, &paths.combos_file(), out, &now.to_rfc3339())?,
        Err(err) => {
            out.warning(&format!("combo refresh failed, continuing: {err:#}"));
            0
        }
    };
    let combos_ok = refresh.is_ok();

    // 4. Embed all cards into the flat vector store (docs carry tag lines).
    out.status("Loading", "search model (~35MB on first run)");

    // 5. Finalize: write status.json last so a failed run leaves the store
    // marked un-set-up (setup stats + vector index metadata in one file).
    let store = embed_all(paths, out, conn)?;

    // 6. Stamp setup state last so a failed run leaves the store un-set-up.
    // Setup prices all prints in-bulk: mark fresh so reads skip
    // revalidation.
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
    status.write(&paths.status_file())?;
    out.finish(
        "Setup done",
        &format!(
            "{} cards searchable, {} combos",
            crate::output::grouped_int(store.meta.names.len() as i64),
            crate::output::grouped_int(combos_delta as i64)
        ),
        setup_start.elapsed(),
    );
    Ok(cli::codes::OK)
}

/// Embed every card, writing `vectors.bin` under the data root.
///
/// Batches the embedding work in 256-document chunks; progress lines go to
/// stderr unconditionally (visible both piped and on a TTY). The index
/// metadata lands in `status.json`, written by the caller after this returns.
fn embed_all(
    paths: &Paths,
    out: &mut Output,
    conn: &Connection,
) -> anyhow::Result<crate::embed::VectorStore> {
    let cards = crate::db::load_all_cards(conn)?;
    let tag_index = crate::tags::TagIndex::load(conn)?;
    let model_start = std::time::Instant::now();
    let mut model = crate::embed::load_model(&paths.models_dir(), out.verbose)?;
    out.status(
        "Ready",
        &format!("in {:.1}s", model_start.elapsed().as_secs_f64()),
    );
    let mut store = crate::embed::VectorStore::new();

    let docs: Vec<String> = cards
        .iter()
        .map(|card| crate::embed::doc_for_row(card, &tag_index))
        .collect();
    let start = std::time::Instant::now();
    let total = docs.len() as u64;
    out.progress_bar("Embedding", "cards", total);
    for (chunk_index, chunk) in docs.chunks(EMBED_CHUNK).enumerate() {
        let vectors = crate::embed::embed_texts(model.as_mut(), chunk)?;
        for (doc_index, vector) in vectors.into_iter().enumerate() {
            // Chunk i starts at chunk_index * EMBED_ROW_BASE; doc_index is
            // the row inside the chunk. The helper is unit-tested because a
            // row misalignment silently scores the wrong card.
            let card_index = chunk_row_base(chunk_index) + doc_index;
            store.push(&cards[card_index].name, vector)?;
        }
        out.tick_progress(chunk.len() as u64);
        // Piped stderr gets periodic plain checkpoints from the bar itself;
        // TTY mode draws a live bar. No extra lines needed on either path.
    }
    out.clear_progress();
    let elapsed = start.elapsed().as_secs_f64();
    if elapsed > 1.0 {
        out.status(
            "Embedded",
            &format!(
                "{} cards in {elapsed:.0}s ({:.0}/s)",
                crate::output::grouped_int(total as i64),
                total as f64 / elapsed
            ),
        );
    }
    store.save_vectors(paths.root())?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        remove_force_targets(&paths, &mut out).unwrap();
        assert!(!paths.db().exists());
        assert!(!paths.db().with_extension("db-wal").exists());
        assert!(!paths.status_file().exists());
        assert!(!paths.vectors_file().exists());
        // Idempotent: a second pass over missing files succeeds.
        remove_force_targets(&paths, &mut out).unwrap();
    }
}
