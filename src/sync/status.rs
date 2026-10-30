//! Sync freshness, embedding progress, and vector-store metadata helpers.

use anyhow::Context;

/// Whether a large embedding batch has reached a plain-text checkpoint.
/// Batches below 20 cards stay silent; larger batches report about ten updates.
pub(crate) fn should_report_embed_progress(total: usize, pos: usize) -> bool {
    if total < 20 {
        return false;
    }
    let step = total / 10;
    pos.is_multiple_of(step) || pos == total
}

/// Report progress during a large embedding batch.
pub(crate) fn tick_embed_progress(out: &mut crate::output::Output, total: usize, pos: usize) {
    if out.supports_progress() {
        out.set_progress_position(pos as u64);
        return;
    }
    if !should_report_embed_progress(total, pos) {
        return;
    }
    out.status(
        "Embedding",
        &format!(
            "{} of {} cards",
            crate::output::grouped_int(pos as i64),
            crate::output::grouped_int(total as i64)
        ),
    );
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
