//! A ranked query uses one locked SQL/vector snapshot through full-text retrieval.

use super::{Hit, filter_only, prepare_query, search_with_vectors};
use crate::db::{self, CardRow};
use crate::embed::{self, VectorStore};
use crate::output::Output;
use crate::paths::{Paths, Status};
use crate::search::CardFilters;
use crate::sync::{recovery, store_lock::StoreLock};
use anyhow::Context;
use std::collections::HashSet;

/// Run ranked search, or a database-only listing without text.
///
/// Ranked queries hold the store lock through recovery, paired index loading,
/// SQL card/status reads, query embedding, and full-text retrieval. They open a
/// fresh connection after locking, so a completed forced reset cannot leave
/// them reading the old database file. Listings use the caller's connection.
///
/// # Errors
/// Returns database errors. With text, also returns lock, recovery, index,
/// model, and embedding failures.
pub fn run_search(
    paths: &Paths,
    conn: &rusqlite::Connection,
    out: &Output,
    text: &str,
    filters: &CardFilters,
    limit: usize,
    restrict: Option<&HashSet<String>>,
) -> anyhow::Result<Vec<Hit>> {
    if text.trim().is_empty() {
        return filter_only(conn, filters, limit, restrict);
    }
    let mut progress = out.task_output();
    if let Some(progress) = &mut progress {
        let task = "cards (preparing text search)";
        progress.status("Searching", task);
        progress.progress_bar("Searching", task, 0);
    }
    let result = RankedSnapshot::open(paths)
        .and_then(|snapshot| snapshot.search(text, filters, limit, restrict));
    if let Some(progress) = &mut progress {
        progress.clear_progress();
    }
    result
}

/// One locked database/index view, including CLI scopes and result detail.
/// Its SQL read transaction also isolates writes that do not take StoreLock.
pub(crate) struct RankedSnapshot {
    conn: rusqlite::Connection,
    lock: StoreLock,
}

impl RankedSnapshot {
    /// Recover the store and attach to its current database while locked.
    pub(crate) fn open(paths: &Paths) -> anyhow::Result<Self> {
        let lock = StoreLock::acquire(paths)?;
        recovery::recover_publication(&lock)?;
        let conn = db::open(&paths.db())?;
        conn.execute_batch("BEGIN DEFERRED")?;
        // Closing this connection ends the read transaction before unlocking.
        Ok(Self { conn, lock })
    }

    /// Use the ranked snapshot for ownership, price, and result-detail reads.
    pub(crate) fn connection(&self) -> &rusqlite::Connection {
        &self.conn
    }

    /// Rank cards without reacquiring the guard or reopening the database.
    pub(crate) fn search(
        &self,
        text: &str,
        filters: &CardFilters,
        limit: usize,
        restrict: Option<&HashSet<String>>,
    ) -> anyhow::Result<Vec<Hit>> {
        let paths = self.lock.paths();
        let conn = &self.conn;
        if !paths.is_setup() {
            anyhow::bail!("card index not built yet");
        }
        let store = VectorStore::load_locked(&self.lock)
            .context("cannot load the card search index; run 'stm sync' before retrying")?;
        let cards = db::load_all_cards(conn)?;
        let prepared = prepare_query(text);
        let query_vector = if store.is_empty() {
            Vec::new()
        } else {
            let status = Status::read(&paths.status_file())?;
            anyhow::ensure!(
                store.meta.model == embed::MODEL_NAME
                    && status.model == embed::MODEL_NAME
                    && status.doc_version == embed::DOC_VERSION,
                "the card search index needs an update; run 'stm sync' before searching"
            );
            validate_vector_names(&store, &cards)?;
            query_embedding(&store, paths, &prepared.expanded_text)?
        };
        let results = search_with_vectors(
            conn,
            &cards,
            &store,
            &query_vector,
            text,
            filters,
            limit,
            restrict,
        )?;
        Ok(results.hybrid)
    }
}

/// Reject row drift before loading a model or scoring vectors.
fn validate_vector_names(store: &VectorStore, cards: &[CardRow]) -> anyhow::Result<()> {
    anyhow::ensure!(
        store.len() == cards.len()
            && store
                .meta
                .names
                .iter()
                .zip(cards)
                .all(|(name, card)| name == &card.name),
        "the search index does not match the card database ({} indexed cards, {} stored cards); run 'stm sync' before retrying",
        store.len(),
        cards.len()
    );
    Ok(())
}

/// Embed query text, with a test-only replacement that cannot run in production.
fn query_embedding(store: &VectorStore, paths: &Paths, text: &str) -> anyhow::Result<Vec<f32>> {
    #[cfg(test)]
    if let Some(embed) = TEST_EMBEDDING.with(std::cell::Cell::get) {
        return Ok(embed());
    }
    let mut model = embed::load_query_model(&paths.models_dir(), false)?;
    store.embed_query(&mut model, text)
}

#[cfg(test)]
/// One test-only action at a query read boundary.
type SearchHook = Box<dyn FnOnce()>;

/// Offline query vectors supplied by snapshot tests.
#[cfg(test)]
type TestEmbedding = fn() -> Vec<f32>;

#[cfg(test)]
thread_local! {
    /// Offline vectors for controlled production-query tests.
    static TEST_EMBEDDING: std::cell::Cell<Option<TestEmbedding>> = const { std::cell::Cell::new(None) };
    /// A deterministic pause immediately before full-text SQL reads.
    static BEFORE_FTS: std::cell::RefCell<Option<SearchHook>> = const { std::cell::RefCell::new(None) };
    /// A deterministic write after command scope reads and before result reads.
    static AFTER_SCOPE: std::cell::RefCell<Option<SearchHook>> = const { std::cell::RefCell::new(None) };
}

/// Reach the controlled boundary between command scope and result reads.
#[cfg(test)]
pub(crate) fn after_scope_for_test() {
    if let Some(write) = AFTER_SCOPE.with(|hook| hook.borrow_mut().take()) {
        write();
    }
}

/// Reach the controlled full-text boundary only in query snapshot tests.
#[cfg(test)]
pub(super) fn before_fts_for_test() {
    if let Some(pause) = BEFORE_FTS.with(|hook| hook.borrow_mut().take()) {
        pause();
    }
}

#[cfg(test)]
#[path = "tests/run_tests.rs"]
mod run_tests;
