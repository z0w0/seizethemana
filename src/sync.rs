use anyhow::Context;
use rusqlite::Connection;

mod status;
use status::{doc_version_current, embed_targets, tick_embed_progress};
pub use status::{is_stale, stamp_combos_synced, stamp_synced, upsert_vector, write_status};

// `stm sync`: keep card data, prices, and oracle tags fresh from one
// download pass.
//
// Scryfall's oracle bulk already carries prices on every card object, so one
// download serves both: stream the bulk once, diff cards against the stored
// table, harvest prices for every print seen, then embed only new/changed
// cards. The separate oracle-tags bulk refreshes the tag tables but never
// triggers re-embedding. Setup runs the identical pipeline where the diff
// finds everything.
//
// Read commands trigger this stale-while-revalidate when older than 24h,
// unless `--offline`. Bulk files re-download when their mtime is older than
// the staleness window, so a daily refresh actually sees new data.

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
///
/// # Errors
/// Propagates network, SQLite, and embedding failures.
pub fn run_sync(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    out: &mut crate::output::Output,
    options: &SyncOptions,
) -> anyhow::Result<i32> {
    if !paths.is_setup() {
        anyhow::bail!("card index not built yet");
    }
    let start = std::time::Instant::now();

    out.status("Fetching", "Scryfall bulk index");
    let (cards_bulk, tags_bulk) = crate::scryfall::fetch_bulk_files()?;
    let cards_dest = paths.bulk_file();
    let tags_dest = paths.tags_file();
    if options.force {
        out.status("Downloading", "card bulk data (forced)");
        crate::scryfall::download_to(
            &cards_bulk.uri,
            &cards_dest,
            cards_bulk.compressed_size,
            out,
        )?;
        out.status("Downloading", "tags bulk data (forced)");
        crate::scryfall::download_to(&tags_bulk.uri, &tags_dest, tags_bulk.compressed_size, out)?;
    } else {
        crate::scryfall::ensure_fresh_bulk(&cards_bulk, &cards_dest, out)?;
        crate::scryfall::ensure_fresh_bulk(&tags_bulk, &tags_dest, out)?;
    }
    // The freshness stamp lands after the downloads: a slow sync must not
    // shorten the store's effective 24h freshness window.
    let now = chrono::Utc::now();

    out.status("Comparing", "bulk data against stored cards");
    let delta = sync_cards(conn, &cards_dest, out, &now.to_rfc3339())?;

    // The universe migration may have arrived after the last full sync:
    // backfill set_type/franchise from the store + the curated mapping.
    backfill_universe(conn, out)?;

    // Tag labels are part of embedding documents. Keep the prior documents
    // so the tag ingest can identify cards whose selected labels changed.
    let cards_before_tags = crate::db::load_all_cards(conn)?;
    let tags_before = crate::tags::TagIndex::load(conn)?;
    out.status("Preparing", "tag search");
    let tags_delta = crate::tags::ingest(conn, &tags_dest)?;
    let tags_after = crate::tags::TagIndex::load(conn)?;
    let tag_targets = changed_tag_documents(&cards_before_tags, &tags_before, &tags_after);

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

    let stale_layout = !doc_version_current(paths)?;
    if delta.to_embed.is_empty() && tag_targets.is_empty() && !stale_layout {
        out.status("Embedding", "no card content changed, index up to date");
    } else {
        let reason = if delta.to_embed.is_empty() && tag_targets.is_empty() {
            "embedding document layout is outdated"
        } else if delta.to_embed.is_empty() {
            "tag labels changed"
        } else {
            "cards"
        };
        out.status(
            "Embedding",
            &format!(
                "{}: {} new/changed, {} added, {} updated",
                reason,
                delta.to_embed.len() + tag_targets.len(),
                delta.added,
                delta.changed
            ),
        );
        // Sync mutates the matrix (row overwrites + appends); load an
        // owned copy instead of the read-only query mapping.
        let mut store = crate::embed::VectorStore::load_owned(paths.root())?;
        let mut model = crate::embed::load_model(&paths.models_dir(), out.verbose)?;
        // A layout bump re-embeds every stored card once — plus every
        // name this sync added or changed, which the store may not hold
        // yet. The union keeps the vector store in step with the cards
        // table in both cases.
        let mut changed_content = delta.to_embed;
        changed_content.extend(tag_targets);
        let targets = embed_targets(stale_layout, &store.meta.names, &changed_content);
        for (pos, name) in targets.iter().enumerate() {
            if let Some(card) = crate::db::get_card(conn, name)? {
                upsert_vector(&mut store, model.as_mut(), &card, &tags_after)?;
                tick_embed_progress(out, targets.len(), pos + 1);
            }
        }
        // One save for the whole loop: saving per card rewrites the entire
        // matrix each time.
        store.save_vectors(paths.root())?;
        // write_status already stamps doc_version as current.
        write_status(paths, &store)?;
        out.status(
            "Embedded",
            &format!("{} cards", crate::output::grouped_int(targets.len() as i64)),
        );
    }

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

/// One parse pass over the bulk, producing everything the sync needs:
/// the card delta applied to `cards`, and print + set rows for pricing.
pub struct SyncDelta {
    /// Names whose embeddable content changed (changed + added).
    pub to_embed: Vec<String>,
    /// Names inserted into `cards` by this pass.
    pub added: usize,
    /// Names whose stored content changed.
    pub changed: usize,
    /// Names whose only change was the EDHREC rank (updated in place,
    /// never re-embedded).
    pub rank_only: usize,
    /// Print rows upserted from the same bulk.
    pub priced: usize,
}

/// Return cards whose selected embedding labels changed after a tag refresh.
fn changed_tag_documents(
    cards: &[crate::db::CardRow],
    before: &crate::tags::TagIndex,
    after: &crate::tags::TagIndex,
) -> Vec<String> {
    cards
        .iter()
        .filter(|card| {
            before.doc_tags_line(&card.oracle_id) != after.doc_tags_line(&card.oracle_id)
        })
        .map(|card| card.name.clone())
        .collect()
}

/// Content signature of a stored row (what re-ingest can change).
///
/// Includes every column the ingest can update *except* `edhrec_rank`:
/// the rank churns broadly on every bulk refresh (EDHREC recomputes
/// daily), it never enters the embedding document, and a rank-only diff
/// must not pay a full re-embed. Rank-only changes take the cheap
/// rank-update pass below.
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

/// The EDHREC rank alone, for the rank-only update pass.
fn rank_signature(card: &crate::db::CardRow) -> i64 {
    card.edhrec_rank.unwrap_or(-1)
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
    })
}

/// Stream the bulk file once; diff cards and harvest prints in one sweep.
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
/// Propagates SQLite failures.
pub fn sync_cards(
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
    let mut rank_only: Vec<(String, Option<i64>)> = Vec::new();

    // Bulk rows repeat one name across many sets; keep the "best" print.
    // Non-card rows (tokens, art series) are recorded by name so imports can
    // skip them silently.
    let mut best: std::collections::HashMap<String, crate::scryfall::ScryfallCard> =
        std::collections::HashMap::new();
    let mut token_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut malformed_prints = 0usize;
    let mut priced = 0usize;
    let mut seen = 0usize;
    out.progress_bar("Parsing", "bulk records", 0);
    conn.execute("BEGIN", []).context("begin sync pass")?;
    let mut stream_err: Option<anyhow::Error> = None;
    let result = (|| -> anyhow::Result<()> {
        let malformed = crate::scryfall::stream_records(bulk_path, |mut card| {
            seen += 1;
            out.set_progress_position(seen as u64);
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
            &mut rank_only,
        );
        apply_rank_updates(conn, &rank_only)?;
        apply_card_updates(conn, &changed, &added, &token_names)?;
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
    out.clear_progress();

    let mut to_embed: Vec<String> = Vec::new();
    for (name, _) in &changed {
        to_embed.push(name.clone());
    }
    for card in &added {
        to_embed.push(card.name.clone());
    }
    Ok(SyncDelta {
        to_embed,
        added: added.len(),
        changed: changed.len(),
        rank_only: rank_only.len(),
        priced,
    })
}

/// Classify each bulk winner against the stored rows: added, changed, or
/// rank-only. Pure bookkeeping over the two maps; no database access.
fn diff_against_stored<'a>(
    by_name: &mut std::collections::HashMap<&'a str, &'a crate::db::CardRow>,
    best: &std::collections::HashMap<String, crate::scryfall::ScryfallCard>,
    added: &mut Vec<crate::scryfall::ScryfallCard>,
    changed: &mut Vec<(String, crate::scryfall::ScryfallCard)>,
    rank_only: &mut Vec<(String, Option<i64>)>,
) {
    for (name, card) in best {
        match by_name.remove(name.as_str()) {
            None => added.push(card.clone()),
            Some(stored) => {
                if ingest_signature(stored) != signature_of(card) {
                    changed.push((stored.name.clone(), card.clone()));
                } else if rank_signature(stored) != card.edhrec_rank.unwrap_or(-1) {
                    rank_only.push((stored.name.clone(), card.edhrec_rank));
                }
            }
        }
    }
}

/// Apply rank-only updates in one prepared statement. Caller owns the
/// transaction.
///
/// # Errors
/// Propagates SQLite failures.
fn apply_rank_updates(
    conn: &Connection,
    rank_only: &[(String, Option<i64>)],
) -> anyhow::Result<()> {
    if rank_only.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare("UPDATE cards SET edhrec_rank = ?2 WHERE name = ?1")?;
    for (name, rank) in rank_only {
        stmt.execute(rusqlite::params![name, rank])?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bulk_card(name: &str, text: &str) -> crate::scryfall::ScryfallCard {
        serde_json::from_value(serde_json::json!({
            "name": name, "layout": "normal", "games": ["paper"],
            "id": "sid-1", "oracle_id": "oid", "released_at": "2020-01-01",
            "mana_cost": "{R}", "cmc": 1.0, "type_line": "Instant",
            "colors": ["R"], "color_identity": ["R"], "keywords": [],
            "oracle_text": text, "rarity": "common",
            "set": "tst", "collector_number": "1",
            "prices": {"usd": "1.00"}
        }))
        .unwrap()
    }

    #[test]
    fn stale_layout_embeds_delta_cards_too() {
        let stored = vec!["Old".to_string(), "Kept".to_string()];
        // Normal sync: only the delta.
        assert_eq!(
            embed_targets(false, &stored, &["Added".to_string()]),
            vec!["Added".to_string()]
        );
        // Layout bump with no delta: re-embed everything stored.
        assert_eq!(embed_targets(true, &stored, &[]), stored);
        // The bug this guards: a layout bump that ships with new cards must
        // embed the union, or the vector store falls behind the cards table.
        assert_eq!(
            embed_targets(true, &stored, &["Old".to_string(), "Added".to_string()]),
            vec!["Old".to_string(), "Kept".to_string(), "Added".to_string()]
        );
    }

    #[test]
    fn tag_document_changes_add_cards_to_embed_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES ('Tagged', 'oid-tagged', '', 0, 'Creature', '[]', '[]', '[]',
                '', 'common', '{}', '', '', '', '')",
            [],
        )
        .unwrap();
        let cards = crate::db::load_all_cards(&conn).unwrap();
        let before = crate::tags::TagIndex::load(&conn).unwrap();
        conn.execute(
            "INSERT INTO tags (id, slug, label, use_count) VALUES ('t1', 'ramp', 'Ramp', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_tags (oracle_id, tag_id, weight) VALUES ('oid-tagged', 't1', 'strong')",
            [],
        )
        .unwrap();
        let after = crate::tags::TagIndex::load(&conn).unwrap();
        assert_eq!(
            changed_tag_documents(&cards, &before, &after),
            vec!["Tagged"]
        );
    }

    #[test]
    fn embed_progress_reports_deciles_and_last_card() {
        let mut out = crate::output::Output::new(true, false, false);
        // Small batches stay silent (below the 20-card floor, decile
        // steps of 1 would report every card).
        tick_embed_progress(&mut out, 5, 5);
        tick_embed_progress(&mut out, 15, 1);
        tick_embed_progress(&mut out, 15, 15);
        // At 100 the step is 10: positions 90 and 100 both report, and
        // position 91 does not (a non-multiple inside the window).
        tick_embed_progress(&mut out, 100, 90);
        tick_embed_progress(&mut out, 100, 100);
    }

    #[test]
    fn stale_when_never_synced() {
        let mut status = crate::paths::Status::empty();
        // Not set up: never considered stale (nothing to sync yet).
        assert!(!is_stale(&status, chrono::Utc::now()));
        status.setup_complete = true;
        // Set up but never synced: stale.
        assert!(is_stale(&status, chrono::Utc::now()));
        status.scryfall_synced_at = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        assert!(!is_stale(&status, chrono::Utc::now()));
        status.scryfall_synced_at = (chrono::Utc::now() - chrono::Duration::hours(30)).to_rfc3339();
        assert!(is_stale(&status, chrono::Utc::now()));
        status.setup_complete = false;
        assert!(!is_stale(&status, chrono::Utc::now()));
    }

    #[test]
    fn diff_detects_added_and_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES ('Old', 'oid', '{R}', 1.0, 'Instant', '[\"R\"]', '[\"R\"]', '[]',
                'Old text', 'common', '{}', 'tst', '1', 'sid-1', '2020-01-01')",
            [],
        )
        .unwrap();

        let existing = crate::db::load_all_cards(&conn).unwrap();
        let mut by_name: std::collections::HashMap<&str, &crate::db::CardRow> =
            existing.iter().map(|c| (c.name.as_str(), c)).collect();
        let mut kept = std::collections::HashMap::new();
        kept.insert("Old".to_string(), bulk_card("Old", "Old text"));
        kept.insert("New".to_string(), bulk_card("New", "New text"));
        kept.insert("Old".to_string(), bulk_card("Old", "CHANGED"));

        let mut added = Vec::new();
        let mut changed = Vec::new();
        for (name, card) in &kept {
            match by_name.remove(name.as_str()) {
                None => added.push(card.name.clone()),
                Some(stored) => {
                    if ingest_signature(stored) != signature_of(card) {
                        changed.push(stored.name.clone());
                    }
                }
            }
        }
        // The second "Old" write wins, so the stored row differs -> changed,
        // and "New" -> added.
        assert_eq!(added, vec!["New"]);
        assert_eq!(changed, vec!["Old"]);
    }

    #[test]
    fn diff_ignores_print_and_price_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES ('Same', 'oid', '{R}', 1.0, 'Instant', '[\"R\"]', '[\"R\"]', '[]',
                'Text', 'common', '{}', 'tst', '1', 'sid-1', '2020-01-01')",
            [],
        )
        .unwrap();
        // Different set/collector number (different chosen print) must not
        // count as a content change: the signature covers card content only.
        let mut card = bulk_card("Same", "Text");
        card.set_code = Some("other".into());
        card.collector_number = Some("9".into());
        card.id = Some("sid-2".into());
        let existing = crate::db::load_all_cards(&conn).unwrap();
        let by_name: std::collections::HashMap<&str, &crate::db::CardRow> =
            existing.iter().map(|c| (c.name.as_str(), c)).collect();
        assert!(!by_name.is_empty());
        // Same content -> no change detected.
        let stored = by_name.get("Same").copied().unwrap();
        assert_eq!(ingest_signature(stored), signature_of(&card));
        // Rarity and loyalty are content: a change re-ingests. The EDHREC
        // rank is not: it takes the cheap rank-only pass, never a re-embed.
        for mutator in [
            |c: &mut crate::scryfall::ScryfallCard| c.rarity = Some("rare".into()),
            |c: &mut crate::scryfall::ScryfallCard| c.loyalty = Some("3".into()),
        ] {
            let mut changed_card = bulk_card("Same", "Text");
            mutator(&mut changed_card);
            assert_ne!(ingest_signature(stored), signature_of(&changed_card));
        }
        // A rank-only diff lands in the rank pass, not the re-embed list.
        let mut rank_card = bulk_card("Same", "Text");
        rank_card.edhrec_rank = Some(5);
        assert_eq!(ingest_signature(stored), signature_of(&rank_card));
        assert_ne!(rank_signature(stored), rank_card.edhrec_rank.unwrap_or(-1));
    }

    #[test]
    fn upsert_vector_appends_new_and_updates_in_place() {
        let mut store = crate::embed::VectorStore::new();
        let zeros = vec![0.0; crate::embed::DIM];
        store.push("A", zeros).unwrap();
        store.push("B", vec![0.0; crate::embed::DIM]).unwrap();

        // Appends and overwrites keep row order aligned with names.
        let vector = vec![0.5; crate::embed::DIM];
        store.push("C", vector).unwrap();
        assert_eq!(store.meta.index_of("C"), Some(2));
        let idx = store.meta.index_of("B").unwrap();
        let mut new_vec = vec![1.0; crate::embed::DIM];
        crate::embed::normalize_row(&mut new_vec);
        store.row_mut(idx).copy_from_slice(&new_vec);
        assert_eq!(store.meta.index_of("B"), Some(1));
    }

    #[test]
    fn stamp_synced_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
        let now = chrono::Utc::now();
        stamp_synced(&paths, now).unwrap();
        let status = crate::paths::Status::read(&paths.status_file()).unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&status.scryfall_synced_at).unwrap();
        assert_eq!(parsed.timestamp(), now.timestamp());
        assert!(!is_stale(&status, now));
    }

    #[test]
    fn sync_cards_end_to_end() {
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let mut conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        // One stored card with a stale print price; the bulk changes its text.
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES ('Existing', 'oid', '{R}', 1.0, 'Instant', '[\"R\"]', '[\"R\"]', '[]',
                'old text', 'common', '{}', 'tst', '1', 'sid-1', '2020-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, updated_at)
             VALUES ('sid-1', 'Existing', 'tst', '1', 'en', 'common', '[\"nonfoil\"]',
                '2020-01-01', 9.0, 'old')",
            [],
        )
        .unwrap();

        // Bulk: 'Existing' with new text + new price; 'Added' is brand new;
        // 'Existing' also carries a second (reprint) print with its own price.
        let bulk = tmp.path().join("bulk.jsonl.gz");
        let enc = flate2::write::GzEncoder::new(
            std::fs::File::create(&bulk).unwrap(),
            flate2::Compression::fast(),
        );
        let mut enc = enc;
        for (name, text, usd) in [
            ("Existing", "rewritten text", "3.50"),
            ("Added", "fresh card", "0.25"),
        ] {
            writeln!(
                enc,
                r#"{{"name":"{name}","layout":"normal","games":["paper"],"id":"sid-{name}","oracle_id":"oid-{name}","released_at":"2021-01-01","mana_cost":"{{R}}","cmc":1.0,"type_line":"Instant","colors":["R"],"color_identity":["R"],"keywords":[],"oracle_text":"{text}","rarity":"common","set":"tst","set_name":"Test Set","collector_number":"1","prices":{{"usd":"{usd}"}}}}"#
            )
            .unwrap();
        }
        writeln!(
            enc,
            r#"{{"name":"Existing","layout":"normal","games":["paper"],"id":"sid-Reprint","oracle_id":"oid-Existing","released_at":"2021-02-01","mana_cost":"{{R}}","cmc":1.0,"type_line":"Instant","colors":["R"],"color_identity":["R"],"keywords":[],"oracle_text":"rewritten text","rarity":"rare","set":"rpt","set_name":"Reprint Set","collector_number":"7","prices":{{"usd":"1.25","usd_foil":"6.00"}}}}"#
        )
        .unwrap();
        drop(enc);

        let mut out = crate::output::Output::new(true, false, false);
        let delta = sync_cards(&mut conn, &bulk, &mut out, "now").unwrap();
        assert_eq!(delta.added, 1);
        assert_eq!(delta.changed, 1);
        // Two prints per card name = three print rows total.
        assert_eq!(delta.priced, 3);
        assert_eq!(delta.to_embed, vec!["Existing", "Added"]);

        // Card row updated; its prints carry per-printing prices.
        let updated = crate::db::get_card(&conn, "Existing").unwrap().unwrap();
        assert_eq!(updated.oracle_text, "rewritten text");
        let print_usd: f64 = conn
            .query_row(
                "SELECT usd FROM card_prints WHERE scryfall_id = 'sid-Existing'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(print_usd, 3.5);
        // The stale seed row (a print not in this bulk) keeps its value.
        let stale_usd: f64 = conn
            .query_row(
                "SELECT usd FROM card_prints WHERE scryfall_id = 'sid-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stale_usd, 9.0);
        // New card row exists with its print priced.
        let added = crate::db::get_card(&conn, "Added").unwrap().unwrap();
        assert_eq!(added.oracle_text, "fresh card");
        let added_usd: f64 = conn
            .query_row(
                "SELECT usd FROM card_prints WHERE scryfall_id = 'sid-Added'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(added_usd, 0.25);
        // Cheapest pick spans every print of the name (1.25 beats 3.5).
        let range = crate::prints::price_range(&conn, "Existing").unwrap();
        assert_eq!(
            range.cheapest.unwrap().usd,
            Some(1.25),
            "cheapest comes from the reprint print"
        );
        // Set names harvested from the same bulk.
        let set_name: String = conn
            .query_row(
                "SELECT set_name FROM sets WHERE set_code = 'rpt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(set_name, "Reprint Set");
    }

    #[test]
    fn sync_rank_only_updates_rank_without_reembed() {
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let mut conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        // A stored card identical to the bulk except the EDHREC rank: the
        // bulk adds a rank where none was stored.
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES ('Ranked', 'oid-Ranked', '{R}', 1.0, 'Instant', '[\"R\"]', '[\"R\"]', '[]',
                'text', 'common', '{}', 'tst', '1', 'sid-Ranked', '2020-01-01')",
            [],
        )
        .unwrap();

        let bulk = tmp.path().join("bulk.jsonl.gz");
        let enc = flate2::write::GzEncoder::new(
            std::fs::File::create(&bulk).unwrap(),
            flate2::Compression::fast(),
        );
        let mut enc = enc;
        writeln!(
            enc,
            r#"{{"name":"Ranked","layout":"normal","games":["paper"],"id":"sid-Ranked","oracle_id":"oid-Ranked","released_at":"2020-01-01","mana_cost":"{{R}}","cmc":1.0,"type_line":"Instant","colors":["R"],"color_identity":["R"],"keywords":[],"oracle_text":"text","rarity":"common","set":"tst","collector_number":"1","prices":{{"usd":"1.00"}},"edhrec_rank":42}}"#
        )
        .unwrap();
        drop(enc);

        let mut out = crate::output::Output::new(true, false, false);
        let delta = sync_cards(&mut conn, &bulk, &mut out, "now").unwrap();
        assert_eq!(delta.rank_only, 1, "a rank change alone lands as rank_only");
        assert_eq!(delta.changed, 0);
        assert_eq!(delta.added, 0);
        // Rank updated in place; nothing queued for re-embed.
        let rank: Option<i64> = conn
            .query_row(
                "SELECT edhrec_rank FROM cards WHERE name = 'Ranked'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rank, Some(42));
        assert!(delta.to_embed.is_empty(), "rank-only must not re-embed");
    }

    #[test]
    fn sync_records_token_names() {
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let mut conn = crate::db::open(&tmp.path().join("t.db")).unwrap();

        let bulk = tmp.path().join("bulk.jsonl.gz");
        let enc = flate2::write::GzEncoder::new(
            std::fs::File::create(&bulk).unwrap(),
            flate2::Compression::fast(),
        );
        let mut enc = enc;
        writeln!(
            enc,
            r#"{{"name":"Elf Warrior","layout":"token","games":["paper"],"id":"sid-token"}}"#
        )
        .unwrap();
        writeln!(
            enc,
            r#"{{"name":"Bolt","layout":"normal","games":["paper"],"id":"sid-bolt","oracle_id":"oid","released_at":"2021-01-01","mana_cost":"{{R}}","cmc":1.0,"type_line":"Instant","colors":["R"],"color_identity":["R"],"keywords":[],"oracle_text":"Deal 3","rarity":"common","set":"tst","collector_number":"1","prices":{{"usd":"0.25"}}}}"#
        )
        .unwrap();
        writeln!(
            enc,
            r#"{{"name":"Bolt","layout":"normal","games":["arena"],"id":"sid-bolt-arena","oracle_id":"oid","released_at":"2021-01-01","mana_cost":"{{R}}","cmc":1.0,"type_line":"Instant","colors":["R"],"color_identity":["R"],"keywords":[],"oracle_text":"Deal 3","rarity":"common","set":"hbg","collector_number":"1","prices":{{}}}}"#
        )
        .unwrap();
        drop(enc);

        let mut out = crate::output::Output::new(true, false, false);
        sync_cards(&mut conn, &bulk, &mut out, "now").unwrap();
        // Tokens are recorded by name and resolvable; real cards are not —
        // not even when they carry a digital-only print alongside paper.
        assert!(crate::db::is_token_name(&conn, "Elf Warrior").unwrap());
        assert!(!crate::db::is_token_name(&conn, "Bolt").unwrap());
    }
}

/// Backfill the universe columns for sets that predate migration 0002.
///
/// Existing `sets` rows keep their old `set_type`/`franchise` values until
/// the bulk refresh upserts them; this pass derives what the store alone
/// can: franchise from the curated mapping, plus card-level UB flags for
/// D&D sets (honorary UB: their prints are never promo-flagged). Print
/// flags from the bulk refresh on the next full sync.
///
/// # Errors
/// Propagates SQLite failures.
pub fn backfill_universe(conn: &Connection, out: &mut crate::output::Output) -> anyhow::Result<()> {
    // Re-derive every set's franchise from code + name (idempotent).
    let rows: Vec<(String, String)> = {
        let mut stmt = conn.prepare("SELECT set_code, set_name FROM sets")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut updated = 0usize;
    let mut unknown: Vec<String> = Vec::new();
    for (code, name) in &rows {
        let franchise = crate::universe::franchise_for(code, name);
        conn.execute(
            "UPDATE sets SET franchise = ?2 WHERE set_code = ?1",
            rusqlite::params![code, franchise],
        )?;
        if franchise.is_none() && crate::universe::is_secret_lair(code) {
            continue;
        }
        updated += 1;
    }
    // Sets whose stored prints carry the UB flag but map to no franchise:
    // a curated-table gap (Secret Lair is deliberately franchise-less).
    let flagged: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT set_code FROM card_prints
             WHERE universes_beyond = 1",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for code in &flagged {
        let name: Option<String> = rows.iter().find(|(c, _)| c == code).map(|(_, n)| n.clone());
        if crate::universe::unknown_ub_set(code, name.as_deref().unwrap_or(""), true) {
            unknown.push(code.clone());
        }
    }
    // Honorary-UB D&D sets: flag their prints even without the promo mark.
    let dnd = conn.execute(
        "UPDATE card_prints SET universes_beyond = 1
             WHERE set_code IN ('afr', 'afc', 'clb')",
        [],
    )?;
    if dnd > 0 {
        out.status(
            "Universe",
            &format!("flagged {dnd} D&D prints as honorary Universes Beyond"),
        );
    }
    for code in &unknown {
        out.warning(&format!(
            "unknown Universes Beyond set {code:?}: add a mapping in src/universe.rs"
        ));
    }
    out.status(
        "Universe",
        &format!("franchise mapping refreshed over {updated} sets"),
    );
    Ok(())
}

#[cfg(test)]
mod universe_backfill_tests {
    use super::*;

    #[test]
    fn backfill_sets_franchise_and_flags_dnd() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
        conn.execute(
            "INSERT INTO sets (set_code, set_name) VALUES ('msh', 'Marvel Super Heroes'),
                    ('afr', 'Adventures in the Forgotten Realms'),
                    ('mh3', 'Modern Horizons 3')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, universes_beyond, updated_at)
             VALUES ('p1', 'X', 'afr', 0, 't')",
            [],
        )
        .unwrap();
        let mut out = crate::output::Output::new(true, false, false);
        backfill_universe(&conn, &mut out).unwrap();
        let (franchise, set_type): (Option<String>, String) = conn
            .query_row(
                "SELECT franchise, set_type FROM sets WHERE set_code = 'msh'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(franchise.as_deref(), Some("Marvel"));
        // set_type stays whatever the bulk last wrote (empty here): the
        // backfill derives what the store alone can.
        assert_eq!(set_type, "");
        let ub: i64 = conn
            .query_row(
                "SELECT universes_beyond FROM card_prints WHERE set_code = 'afr'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ub, 1, "D&D prints are honorary UB");
        // Idempotent: a second pass changes nothing.
        backfill_universe(&conn, &mut out).unwrap();
        let ub: i64 = conn
            .query_row(
                "SELECT universes_beyond FROM card_prints WHERE set_code = 'afr'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ub, 1);
    }
}
