//! `stm sync`: keep card data, prices, and oracle tags fresh from one
//! download pass.
//!
//! Scryfall's oracle bulk already carries prices on every card object, so one
//! download serves both: stream the bulk once, diff cards against the stored
//! table, harvest prices for every print seen, then embed only new/changed
//! cards. Tag changes and document-layout changes also queue new vectors.
//! Pending document revisions survive failed embedding and index publication.
//! Setup uses the same card ingest pipeline.
//!
//! Read commands trigger this stale-while-revalidate when older than 24h,
//! unless `--offline`. Bulk files re-download when their mtime is older than
//! the staleness window, so a daily refresh actually sees new data.

use anyhow::Context;
use rusqlite::Connection;

/// Durable embedding work and restart-safe publication.
pub(crate) mod recovery;
/// Status-file and vector-store bookkeeping for the sync pass.
mod status;
/// Store-wide exclusion for writers and paired index loads.
pub(crate) mod store_lock;
pub use status::{is_stale, stamp_combos_synced, stamp_synced, upsert_vector, write_status};

/// Refresh window for the stale-while-revalidate trigger.
pub const STALE_AFTER: chrono::Duration = chrono::Duration::hours(24);

/// Knobs for a sync run.
#[derive(Debug, Clone, Copy, Default)]
pub struct SyncOptions {
    /// Re-download the bulk and re-apply everything even when fresh.
    pub force: bool,
}

/// Run `stm sync`: refresh card data + prices from the oracle bulk, refresh
/// oracle tags, and embed any new or changed cards.
///
/// Bulk files re-download when missing or older than [`STALE_AFTER`] (or
/// when Scryfall's own timestamp for a file is newer than the local copy),
/// so the daily stale-while-revalidate path actually refreshes prices.
/// Interrupted runs leave `scryfall_synced_at` untouched, so the store stays marked
/// stale and the next command retries.
///
/// Tag changes refresh `tags`/`card_tags` and re-embed cards whose selected
/// document labels changed. Card content and document-layout changes also
/// trigger re-embedding.
/// Holds the store lock through SQL changes, publication, and acknowledgement.
///
/// # Errors
/// Propagates lock, network, SQLite, and embedding failures.
pub fn run_sync(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    options: &SyncOptions,
) -> anyhow::Result<i32> {
    let lock = store_lock::StoreLock::acquire(paths)?;
    crate::db::reopen(conn, &paths.db())?;
    recovery::recover_publication(&lock)?;
    if !paths.is_setup() {
        anyhow::bail!("card index not built yet");
    }
    let start = std::time::Instant::now();

    let index_start = std::time::Instant::now();
    out.start_task("Fetching", "Scryfall bulk index", 0);
    let bulk_files = crate::scryfall::fetch_bulk_files();
    out.clear_progress();
    let (cards_bulk, tags_bulk) = bulk_files?;
    out.finish("Fetched", "Scryfall bulk index", index_start.elapsed());
    let cards_dest = paths.bulk_file();
    let tags_dest = paths.tags_file();
    if options.force {
        crate::scryfall::download_to_named(
            &cards_bulk.uri,
            &cards_dest,
            cards_bulk.compressed_size,
            out,
            "Scryfall cards",
        )?;
        crate::scryfall::download_to_named(
            &tags_bulk.uri,
            &tags_dest,
            tags_bulk.compressed_size,
            out,
            "Scryfall tags",
        )?;
    } else {
        crate::scryfall::ensure_fresh_bulk_named(&cards_bulk, &cards_dest, out, "Scryfall cards")?;
        crate::scryfall::ensure_fresh_bulk_named(&tags_bulk, &tags_dest, out, "Scryfall tags")?;
    }
    // The freshness stamp lands after the downloads: a slow sync must not
    // shorten the store's effective 24h freshness window.
    let now = chrono::Utc::now();

    let started = std::time::Instant::now();
    out.start_task("Loading", "cards and prices", 0);
    let delta = sync_cards_locked(&lock, conn, &cards_dest, out, &now.to_rfc3339());
    out.clear_progress();
    let delta = delta?;
    out.finish("Loaded", "cards and prices", started.elapsed());
    for code in unknown_universe_sets(conn)? {
        out.warning(&format!(
            "unknown Universes Beyond set {code:?}: add a mapping in src/universe.rs"
        ));
    }

    // Document revision checks include the labels committed by this ingest.
    out.status("Preparing", "tag search");
    let tags_delta = crate::tags::ingest(conn, &tags_dest)?;

    // Push the new labels into the full-text index: tag labels are FTS
    // content (weight second to name), so role words resolve through the
    // community vocabulary.
    let tagged = crate::db::refresh_tags_text(conn)?;
    out.status(
        "Indexed",
        &format!("{tagged} tagged cards", tagged = tagged),
    );

    // Combos refresh their own tables wholesale; embeddings ignore them.
    // A failed refresh warns and continues: combos are additive
    // diagnostics, never a blocker for card data.
    let combos_refresh =
        crate::spellbook::ensure_fresh_variants(&paths.combos_file(), out, options.force);
    let combos_delta = match &combos_refresh {
        Ok(_) => crate::spellbook::ingest(conn, &paths.combos_file(), out, &now.to_rfc3339())?,
        Err(err) => {
            out.warning(&format!("combo refresh failed, continuing: {err:#}"));
            0
        }
    };

    let embedded = recovery::embed_pending(&lock, conn, out, || {
        crate::embed::load_model(&paths.models_dir(), false)
    })?;
    if embedded == 0 {
        out.status("Embedding", "no card content changed, index up to date");
    } else {
        out.status(
            "Embedded",
            &format!("{} cards", crate::output::grouped_int(embedded as i64)),
        );
    }

    anyhow::ensure!(
        load_pending_embeddings(conn)?.is_empty(),
        "embedding work remains pending; run stm sync again"
    );
    stamp_synced(paths, now)?;
    // Stamp the combo refresh only when it actually succeeded: a failed
    // refresh must leave the store looking unsynced so the next run retries.
    if combos_refresh.is_ok() {
        stamp_combos_synced(paths, now)?;
    }
    out.finish(
        "Finished",
        &format!(
            "sync: {} added, {} updated, {} rank-only, {} priced in-bulk, {} tags, {} combos",
            crate::output::grouped_int(delta.added as i64),
            crate::output::grouped_int(delta.changed as i64),
            crate::output::grouped_int(delta.rank_only as i64),
            crate::output::grouped_int(delta.priced as i64),
            crate::output::grouped_int(tags_delta.tags as i64),
            crate::output::grouped_int(combos_delta as i64)
        ),
        start.elapsed(),
    );
    Ok(crate::cli::codes::OK)
}

/// Find flagged crossover sets that have no known franchise or promo mapping.
fn unknown_universe_sets(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT p.set_code, COALESCE(s.set_name, '')
         FROM card_prints p LEFT JOIN sets s ON s.set_code = p.set_code
         WHERE p.universes_beyond = 1 ORDER BY p.set_code",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut unknown = Vec::new();
    for row in rows {
        let (code, name) = row?;
        if crate::universe::unknown_ub_set(&code, &name, true) {
            unknown.push(code);
        }
    }
    Ok(unknown)
}

/// One parse pass over the bulk, producing everything the sync needs:
/// the card delta applied to `cards`, and print + set rows for pricing.
pub struct SyncDelta {
    /// Names added, changed, or still waiting for vector publication.
    pub to_embed: Vec<String>,
    /// Names inserted into `cards` by this pass.
    pub added: usize,
    /// Names whose stored content changed.
    pub changed: usize,
    /// Names whose only change was a churny signal column (EDHREC rank,
    /// Penny rank, or Reserved List); updated in place, never re-embedded.
    pub rank_only: usize,
    /// Print rows upserted from the same bulk.
    pub priced: usize,
}

/// Content signature of a stored row (what re-ingest can change).
///
/// Includes every column the ingest can update *except* the churny signal
/// columns `edhrec_rank`, `penny_rank`, and `reserved`: those churn on bulk
/// refresh, none enters the embedding document, and a signal-only diff must
/// not pay a full re-embed. Signal-only changes take the cheap update pass
/// below (`ingest_signature` omits them; `signal_signature` compares them).
fn ingest_signature(card: &crate::db::CardRow) -> String {
    format!(
        "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
        card.oracle_id,
        card.mana_cost,
        card.cmc,
        card.type_line,
        card.colors,
        card.color_identity,
        card.keywords,
        card.power.as_deref().unwrap_or(""),
        card.toughness.as_deref().unwrap_or(""),
        card.loyalty.as_deref().unwrap_or(""),
        card.rarity,
        card.oracle_text,
        card.legalities,
        card.released_at,
        card.game_changer.map(|g| g as i64).unwrap_or(-1),
    )
}

/// The churny signal columns alone (EDHREC rank, Penny rank, Reserved List),
/// for the cheap signal-only update pass. A tuple of `(edhrec, penny,
/// reserved)` with `-1` standing in for NULL.
fn signal_signature(card: &crate::db::CardRow) -> (i64, i64, i64) {
    (
        card.edhrec_rank.unwrap_or(-1),
        card.penny_rank.unwrap_or(-1),
        card.reserved.map(|r| r as i64).unwrap_or(-1),
    )
}

/// The same signal tuple computed from a bulk card, so the diff compares two
/// sides built by one convention (`-1` for NULL). Mirrors [`signal_signature`].
fn card_signals(card: &crate::scryfall::ScryfallCard) -> (i64, i64, i64) {
    (
        card.edhrec_rank.unwrap_or(-1),
        card.penny_rank.unwrap_or(-1),
        card.reserved.map(|r| r as i64).unwrap_or(-1),
    )
}

/// Content signature of a bulk card, computed the same way.
fn signature_of(card: &crate::scryfall::ScryfallCard) -> String {
    let identity =
        serde_json::to_string(&card.color_identity.clone().unwrap_or_default()).unwrap_or_default();
    let legalities =
        serde_json::to_string(&card.legalities.clone().unwrap_or_default()).unwrap_or_default();
    ingest_signature(&crate::db::CardRow {
        name: card.name.clone(),
        oracle_id: card.oracle_id.clone(),
        mana_cost: card.mana_cost.clone().unwrap_or_default(),
        cmc: card.cmc.unwrap_or(0.0),
        type_line: card.type_line.clone().unwrap_or_default(),
        colors: serde_json::to_string(&card.colors.clone().unwrap_or_default()).unwrap_or_default(),
        color_identity: identity,
        keywords: serde_json::to_string(&card.keywords.clone().unwrap_or_default())
            .unwrap_or_default(),
        power: card.power.clone(),
        toughness: card.toughness.clone(),
        loyalty: card.loyalty.clone(),
        oracle_text: card.oracle_text.clone().unwrap_or_default(),
        rarity: card.rarity.clone().unwrap_or_default(),
        edhrec_rank: card.edhrec_rank,
        legalities,
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: card.id.clone().unwrap_or_default(),
        released_at: card.released_at.clone().unwrap_or_default(),
        game_changer: card.game_changer,
        penny_rank: card.penny_rank,
        reserved: card.reserved,
    })
}

/// A stored card's churny signals, queued for the cheap in-place update pass:
/// name plus EDHREC rank, Penny rank, and Reserved List flag.
struct SignalUpdate {
    /// Card name (the update key).
    name: String,
    /// New EDHREC rank, or None to clear.
    edhrec_rank: Option<i64>,
    /// New Penny Dreadful rank, or None to clear.
    penny_rank: Option<i64>,
    /// New Reserved List flag, or None to clear.
    reserved: Option<bool>,
}

/// Stream the bulk file once; diff cards and harvest prints in one sweep.
///
/// Requires a file-backed connection. Acquires the store lock, then reopens
/// the connection so a forced reset during the wait cannot leave it attached
/// to the old database. Holds the lock through the SQL update pass.
/// Commits card changes and queues document revisions, but does not embed or
/// publish vectors. [`run_sync`] completes those steps.
///
/// The pass reuses setup's dedup rules (one row per name) and, per stored
/// name, compares an ingest signature to detect content changes. Every
/// passing print row is upserted into `card_prints` during the parse pass
/// itself (not just the per-name winner), so per-printing prices stay
/// complete without holding a second ~90k-row copy in memory. Prints that
/// vanished from the bulk keep their last known row (the snapshot is
/// additive in this mode).
///
/// # Errors
/// Fails for an unsuitable database path. Propagates lock, database, and
/// bulk-file read or decode failures.
pub fn sync_cards(
    conn: &mut Connection,
    bulk_path: &std::path::Path,
    out: &mut crate::output::Output,
    updated_at: &str,
) -> anyhow::Result<SyncDelta> {
    let db_path = std::path::PathBuf::from(
        conn.path()
            .context("sync requires a file-backed database")?,
    );
    let root = db_path
        .parent()
        .context("sync database has no parent directory")?;
    let lock = store_lock::StoreLock::acquire(&crate::paths::Paths::new(root.to_path_buf()))?;
    crate::db::reopen(conn, &db_path)?;
    sync_cards_locked(&lock, conn, bulk_path, out, updated_at)
}

/// Apply a bulk snapshot while its publication guard remains held by the caller.
pub(crate) fn sync_cards_locked(
    _lock: &store_lock::StoreLock,
    conn: &mut Connection,
    bulk_path: &std::path::Path,
    out: &mut crate::output::Output,
    updated_at: &str,
) -> anyhow::Result<SyncDelta> {
    // Stored rows for the diff (read before the transaction opens).
    let existing = crate::db::load_all_cards(conn)?;
    let mut by_name: std::collections::HashMap<&str, &crate::db::CardRow> =
        existing.iter().map(|c| (c.name.as_str(), c)).collect();

    let mut added: Vec<crate::scryfall::ScryfallCard> = Vec::new();
    let mut changed: Vec<(String, crate::scryfall::ScryfallCard)> = Vec::new();
    let mut signal_only: Vec<SignalUpdate> = Vec::new();

    // Bulk rows repeat one name across many sets; keep the "best" print.
    // Non-card rows (tokens, art series) are recorded by name so imports can
    // skip them silently.
    let mut best: std::collections::HashMap<String, crate::scryfall::ScryfallCard> =
        std::collections::HashMap::new();
    let mut token_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut malformed_prints = 0usize;
    let mut priced = 0usize;
    conn.execute("BEGIN", []).context("begin sync pass")?;
    let mut stream_err: Option<anyhow::Error> = None;
    let result = (|| -> anyhow::Result<()> {
        let malformed = crate::scryfall::stream_records(bulk_path, |mut card| {
            // Never-released cards stay out of the store; the daily sync
            // adds them once their set releases. Only non-card layouts
            // (tokens, art series…) count as "token names" — a
            // digital-only print of a real paper card must not, or imports
            // of that card would skip silently.
            if crate::scryfall::is_non_card_layout(&card.layout) {
                token_names.insert(card.name);
                return;
            }
            if crate::scryfall::should_skip(&card) || !crate::scryfall::should_ingest(&card) {
                return;
            }
            crate::scryfall::flatten_faces(&mut card);
            // Every passing print row lands in `card_prints` as it streams
            // (per-print prices, no full-card copy kept in memory), while
            // the per-name winner feeds the `cards` dedup. Id-less rows
            // are well-formed lines that cannot be upserted; they count
            // separately from malformed lines so the warning names the
            // real cause.
            if card.id.as_deref().is_none_or(str::is_empty) {
                malformed_prints += 1;
            } else if let Err(err) = crate::scryfall::upsert_print(conn, &card, updated_at) {
                // The visit callback cannot `?`; the first failure aborts
                // the pass after streaming ends.
                if stream_err.is_none() {
                    stream_err = Some(err);
                }
                return;
            } else {
                priced += 1;
            }
            best.entry(card.name.clone())
                .and_modify(|current| *current = crate::scryfall::prefer_row(current, &card))
                .or_insert(card);
        })
        .context("streaming bulk records")?;
        if let Some(err) = stream_err {
            return Err(err);
        }
        if malformed > 0 {
            out.warning(&format!("{malformed} malformed bulk line(s) skipped"));
        }
        if malformed_prints > 0 {
            out.warning(&format!(
                "{malformed_prints} print row(s) skipped (no scryfall id)"
            ));
        }
        diff_against_stored(
            &mut by_name,
            &best,
            &mut added,
            &mut changed,
            &mut signal_only,
        );
        apply_signal_updates(conn, &signal_only)?;
        apply_card_updates(conn, &changed, &added, &token_names)?;
        recovery::enqueue_cards(
            conn,
            changed
                .iter()
                .map(|(name, _)| name.as_str())
                .chain(added.iter().map(|card| card.name.as_str())),
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute("COMMIT", []).context("committing sync pass")?;
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
    }
    let mut to_embed: Vec<String> = Vec::new();
    for (name, _) in &changed {
        to_embed.push(name.clone());
    }
    for card in &added {
        to_embed.push(card.name.clone());
    }
    let mut seen: std::collections::HashSet<String> = to_embed.iter().cloned().collect();
    for name in load_pending_embeddings(conn)? {
        if seen.insert(name.clone()) {
            to_embed.push(name);
        }
    }
    Ok(SyncDelta {
        to_embed,
        added: added.len(),
        changed: changed.len(),
        rank_only: signal_only.len(),
        priced,
    })
}

/// Return every card whose vector has not yet been saved.
///
/// # Errors
/// Propagates SQLite failures.
fn load_pending_embeddings(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM pending_embeddings ORDER BY name")?;
    let names = stmt.query_map([], |row| row.get(0))?;
    Ok(names.collect::<Result<Vec<_>, _>>()?)
}

/// Classify each bulk winner against the stored rows: added, changed, or
/// signal-only (EDHREC rank / Penny rank / Reserved List). Pure bookkeeping
/// over the two maps; no database access.
fn diff_against_stored<'a>(
    by_name: &mut std::collections::HashMap<&'a str, &'a crate::db::CardRow>,
    best: &std::collections::HashMap<String, crate::scryfall::ScryfallCard>,
    added: &mut Vec<crate::scryfall::ScryfallCard>,
    changed: &mut Vec<(String, crate::scryfall::ScryfallCard)>,
    signal_only: &mut Vec<SignalUpdate>,
) {
    for (name, card) in best {
        match by_name.remove(name.as_str()) {
            None => added.push(card.clone()),
            Some(stored) => {
                if ingest_signature(stored) != signature_of(card) {
                    changed.push((stored.name.clone(), card.clone()));
                } else if signal_signature(stored) != card_signals(card) {
                    signal_only.push(SignalUpdate {
                        name: stored.name.clone(),
                        edhrec_rank: card.edhrec_rank,
                        penny_rank: card.penny_rank,
                        reserved: card.reserved,
                    });
                }
            }
        }
    }
}

/// Apply signal-only updates (rank + Reserved List) in one prepared
/// statement. Caller owns the transaction.
///
/// # Errors
/// Propagates SQLite failures.
fn apply_signal_updates(conn: &Connection, signal_only: &[SignalUpdate]) -> anyhow::Result<()> {
    if signal_only.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare(
        "UPDATE cards SET edhrec_rank = ?2, penny_rank = ?3, reserved = ?4 WHERE name = ?1",
    )?;
    for update in signal_only {
        stmt.execute(rusqlite::params![
            update.name,
            update.edhrec_rank,
            update.penny_rank,
            update.reserved
        ])?;
    }
    Ok(())
}

/// Apply changed/added card rows and the token-name refresh. Caller owns
/// the transaction.
///
/// # Errors
/// Propagates SQLite failures.
fn apply_card_updates(
    conn: &Connection,
    changed: &[(String, crate::scryfall::ScryfallCard)],
    added: &[crate::scryfall::ScryfallCard],
    token_names: &std::collections::BTreeSet<String>,
) -> anyhow::Result<()> {
    for (name, card) in changed {
        crate::scryfall::update_card(conn, name, card)?;
    }
    for card in added {
        crate::scryfall::insert_card(conn, card)?;
    }
    // Token names refresh wholesale from this bulk.
    conn.execute("DELETE FROM token_names", [])?;
    let mut stmt = conn.prepare("INSERT OR IGNORE INTO token_names (name) VALUES (?1)")?;
    for name in token_names {
        stmt.execute([name])?;
    }
    Ok(())
}

/// Tests for sync transactions and embedding recovery.
#[cfg(test)]
#[path = "tests/sync_tests.rs"]
mod tests;

/// Tests for read-only warnings about missing crossover mappings.
#[cfg(test)]
mod universe_warning_tests {
    use super::*;

    /// Unknown flagged sets warn once even when set metadata is missing.
    #[test]
    fn unknown_flagged_sets_are_reported_once_without_writing() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).expect("open database");
        conn.execute_batch(
            "INSERT INTO sets (set_code, set_name) VALUES ('zzz', 'New Crossover');
             INSERT INTO card_prints (scryfall_id, name, set_code, universes_beyond, updated_at)
             VALUES ('first', 'First', 'zzz', 1, 'now'),
                    ('second', 'Second', 'zzz', 1, 'now'),
                    ('missing', 'Missing metadata', 'orphan', 1, 'now');",
        )
        .expect("seed unknown sets");
        let before = conn.total_changes();
        conn.pragma_update(None, "query_only", true)
            .expect("require read-only queries");
        assert_eq!(
            unknown_universe_sets(&conn).expect("unknown sets"),
            ["orphan", "zzz"]
        );
        assert_eq!(conn.total_changes(), before);
    }

    /// Known franchises, promo sets, Secret Lair, and unflagged sets do not warn.
    #[test]
    fn known_and_unflagged_sets_are_excluded() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).expect("open database");
        for (code, name, flagged) in [
            ("msh", "Marvel Super Heroes", true),
            ("sld", "Secret Lair Drop", true),
            ("pmei", "Media and Collaboration Promos", true),
            ("future", "Marvel Crossover", true),
            ("afr", "Adventures in the Forgotten Realms", true),
            ("normal", "An unflagged set", false),
        ] {
            conn.execute(
                "INSERT INTO sets (set_code, set_name) VALUES (?1, ?2)",
                [code, name],
            )
            .expect("seed set");
            conn.execute(
                "INSERT INTO card_prints (scryfall_id, name, set_code, universes_beyond, updated_at)
                 VALUES (?1, 'Card', ?1, ?2, 'now')",
                rusqlite::params![code, flagged],
            )
            .expect("seed print");
        }
        assert!(
            unknown_universe_sets(&conn)
                .expect("unknown sets")
                .is_empty()
        );
    }
}
