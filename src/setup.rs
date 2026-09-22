use rusqlite::Connection;

use crate::cli;
use crate::output::Output;
use crate::paths::Paths;
use crate::scryfall;

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
        *conn = crate::db::open(&db_path)?;
        out.status("Rebuilt", "empty database with the current schema");
        // Drop the old index and its status stamp before re-ingesting: the
        // new id order must never score against old vectors, and a failed
        // rebuild must leave the store un-set-up.
        let status_path = paths.status_file();
        if let Err(err) = std::fs::remove_file(&status_path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            return Err(anyhow::Error::new(err)
                .context(format!("removing status {}", status_path.display())));
        }
        let vectors_path = paths.vectors_file();
        if let Err(err) = std::fs::remove_file(&vectors_path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            return Err(anyhow::Error::new(err)
                .context(format!("removing vectors {}", vectors_path.display())));
        }
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

    // 3. Ingest cards + prices, then tags (embedding needs tags, so tags
    // land first; identical pipeline to `stm sync`).
    out.status("Ingesting", "cards and prices from bulk data");
    let delta = crate::sync::sync_cards(conn, &cards_dest, out, &chrono::Utc::now().to_rfc3339())?;
    out.finish(
        "Ingested",
        &format!("{} cards, {} priced", delta.added, delta.priced),
        std::time::Duration::ZERO,
    );
    out.status("Ingesting", "oracle tags from bulk data");
    let tags_delta = crate::tags::ingest(conn, &tags_dest, out)?;
    out.finish(
        "Ingested",
        &format!(
            "{} tags, {} tagged cards",
            tags_delta.tags, tags_delta.tagged_cards
        ),
        std::time::Duration::ZERO,
    );
    // Make the labels searchable: tag labels are FTS content (weight second
    // to name), so role words resolve through the community vocabulary.
    out.status("Indexing", "tag labels into full-text search");
    let tagged = crate::db::refresh_tags_text(conn)?;
    out.status(
        "Indexed",
        &format!("{tagged} tagged cards", tagged = tagged),
    );

    // 3b. Combo variants (Commander Spellbook). A failed refresh warns and
    // continues: combos are additive diagnostics, never a blocker.
    let now = chrono::Utc::now();
    let combos_delta =
        match crate::spellbook::ensure_fresh_variants(&paths.combos_file(), out, false) {
            Ok(_) => crate::spellbook::ingest(conn, &paths.combos_file(), out, &now.to_rfc3339())?,
            Err(err) => {
                out.warning(&format!("combo refresh failed, continuing: {err:#}"));
                0
            }
        };

    // 4. Embed all cards into the flat vector store (docs carry tag lines).
    out.status(
        "Embedding",
        "cards with bge-small-en-v1.5-Q (first run downloads ~35MB model)",
    );

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
        combos_synced_at: chrono::Utc::now().to_rfc3339(),
    };
    status.write(&paths.status_file())?;
    out.finish(
        "Finished",
        &format!(
            "setup: {} cards indexed, {} combos",
            store.meta.names.len(),
            combos_delta
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
        &format!(
            "embedding model in {:.1}s",
            model_start.elapsed().as_secs_f64()
        ),
    );
    let mut store = crate::embed::VectorStore::new();

    let docs: Vec<String> = cards
        .iter()
        .map(|card| crate::embed::doc_for_row(card, &tag_index))
        .collect();
    let start = std::time::Instant::now();
    let total = docs.len() as u64;
    out.progress_bar("Embedding", "cards", total);
    for (i, chunk) in docs.chunks(256).enumerate() {
        let vectors = crate::embed::embed_texts(&mut model, chunk)?;
        for (doc_index, vector) in vectors.into_iter().enumerate() {
            let card_index = i * 256 + doc_index;
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
                "{total} cards in {elapsed:.0}s ({:.0}/s)",
                total as f64 / elapsed
            ),
        );
    }
    store.save_vectors(paths.root())?;
    Ok(store)
}
