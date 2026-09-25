use anyhow::Context;

// Status-file and vector-store bookkeeping for the sync pass: staleness,
// the embed-target list, progress reporting, the vector upsert, and the
// status.json writers. Split from `sync.rs` to keep each file small.

/// True when the stored vectors were built with the current document layout.
pub(crate) fn doc_version_current(paths: &crate::paths::Paths) -> anyhow::Result<bool> {
    let status = crate::paths::Status::read(&paths.status_file())?;
    Ok(status.doc_version == crate::embed::DOC_VERSION)
}

/// Names to embed for one sync: all stored names when the document layout
/// is stale (re-embed everything), else only the sync delta. The delta
/// always joins the list — a layout bump that ships with new bulk data
/// must also embed cards the store does not hold yet.
pub(crate) fn embed_targets(
    stale_layout: bool,
    stored: &[String],
    to_embed: &[String],
) -> Vec<String> {
    let mut targets = if stale_layout {
        stored.to_vec()
    } else {
        Vec::new()
    };
    // A set makes the dedup linear over ~30k+ stored names; a contains
    // scan over the Vec is O(n²) on a full layout re-embed. The set
    // updates as names join `targets`, so the delta itself dedups too.
    let mut seen: std::collections::HashSet<String> = targets.iter().cloned().collect();
    for name in to_embed {
        if seen.insert(name.clone()) {
            targets.push(name.clone());
        }
    }
    targets
}

/// Emit roughly ten progress lines during re-embedding. `pos` is the
/// 1-based index of the card just embedded: a line reports when `pos`
/// is a multiple of `total / 10` (rounded down) or on the last card, so
/// uneven totals report every `step`-th card plus the tail. Small
/// batches (fewer than 20 cards) stay silent: below that, decile steps
/// of 1 would report every card.
pub(crate) fn tick_embed_progress(out: &mut crate::output::Output, total: usize, pos: usize) {
    if total < 20 {
        return;
    }
    let step = total / 10;
    if pos.is_multiple_of(step) || pos == total {
        out.status(
            "Embedding",
            &format!(
                "{} of {} cards",
                crate::output::grouped_int(pos as i64),
                crate::output::grouped_int(total as i64)
            ),
        );
    }
}

/// True when the store needs a sync: never synced, or older than
/// [`super::STALE_AFTER`].
pub fn is_stale(status: &crate::paths::Status, now: chrono::DateTime<chrono::Utc>) -> bool {
    if !status.setup_complete {
        return false;
    }
    match chrono::DateTime::parse_from_rfc3339(&status.scryfall_synced_at) {
        Ok(last) => now.signed_duration_since(last) > super::STALE_AFTER,
        Err(_) => true,
    }
}

/// Insert or update one card's vector at `name` in the in-memory store.
///
/// Appends when the name is new to the index; overwrites the row and keeps
/// row order otherwise (row order must stay aligned with `status.json.names`).
/// Saving the matrix to disk is the caller's job, once per loop.
///
/// # Errors
/// Propagates model or storage failures.
pub fn upsert_vector(
    store: &mut crate::embed::VectorStore,
    model: &mut dyn crate::embed::DocumentEmbedder,
    card: &crate::db::CardRow,
    tag_index: &crate::tags::TagIndex,
) -> anyhow::Result<()> {
    let vector = crate::embed::embed_texts(model, &[crate::embed::doc_for_row(card, tag_index)])?
        .pop()
        .context("model returned no embedding")?;
    match store.meta.index_of(&card.name) {
        Some(idx) => {
            // Push() normalizes; writing in place must too.
            store.row_mut(idx).copy_from_slice(&vector);
            crate::embed::normalize_row(store.row_mut(idx));
        }
        None => store.push(&card.name, vector)?,
    }
    Ok(())
}

/// Rewrite `status.json` names/counts from the store, keeping `scryfall_synced_at`.
pub fn write_status(
    paths: &crate::paths::Paths,
    store: &crate::embed::VectorStore,
) -> anyhow::Result<()> {
    let path = paths.status_file();
    let mut status = crate::paths::Status::read(&path)?;
    status.names = store.meta.names.clone();
    status.ingested_cards = store.len();
    status.embedded_cards = store.len();
    status.model = store.meta.model.clone();
    status.dim = store.meta.dim;
    status.doc_version = crate::embed::DOC_VERSION;
    status.write(&path)
}

/// Stamp `scryfall_synced_at` with `now` (RFC 3339) in status.json.
///
/// # Errors
/// Propagates read/write failures.
pub fn stamp_synced(
    paths: &crate::paths::Paths,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    let path = paths.status_file();
    let mut status = crate::paths::Status::read(&path)?;
    status.scryfall_synced_at = now.to_rfc3339();
    status.write(&path)
}
/// Stamp `combos_synced_at` with `now` (RFC 3339) in status.json.
///
/// # Errors
/// Propagates read/write failures.
pub fn stamp_combos_synced(
    paths: &crate::paths::Paths,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    let path = paths.status_file();
    let mut status = crate::paths::Status::read(&path)?;
    status.combos_synced_at = now.to_rfc3339();
    status.write(&path)
}
