use anyhow::Context;
use rusqlite::Connection;

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
/// Tag changes refresh the `tags`/`card_tags` tables but never trigger
/// re-embedding: embeddings only change when the document layout version
/// changes or card content changes.
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
    let now = chrono::Utc::now();

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

    out.status("Comparing", "bulk data against stored cards");
    let delta = sync_cards(conn, &cards_dest, out, &now.to_rfc3339())?;

    // Tags refresh the lookup tables only; embeddings ignore them.
    out.status("Ingesting", "oracle tags from bulk data");
    let tags_delta = crate::tags::ingest(conn, &tags_dest, out)?;

    // Push the new labels into the full-text index: tag labels are FTS
    // content (weight second to name), so role words resolve through the
    // community vocabulary.
    out.status("Indexing", "tag labels into full-text search");
    let tagged = crate::db::refresh_tags_text(conn)?;
    out.status(
        "Indexed",
        &format!("{tagged} tagged cards", tagged = tagged),
    );

    // Combos refresh their own tables wholesale; embeddings ignore them.
    // A failed refresh warns and continues: combos are additive
    // diagnostics, never a blocker for card data.
    let combos_delta = match crate::spellbook::ensure_fresh_variants(&paths.combos_file(), out) {
        Ok(_) => crate::spellbook::ingest(conn, &paths.combos_file(), out, &now.to_rfc3339())?,
        Err(err) => {
            out.warning(&format!("combo refresh failed, continuing: {err:#}"));
            0
        }
    };

    if delta.to_embed.is_empty() && doc_version_current(paths)? {
        out.status("Embedding", "no card content changed, index up to date");
    } else {
        let reason = if delta.to_embed.is_empty() {
            "embedding document layout is outdated"
        } else {
            "cards"
        };
        out.status(
            "Embedding",
            &format!(
                "{}: {} new/changed, {} added, {} updated",
                reason,
                delta.to_embed.len(),
                delta.added,
                delta.changed
            ),
        );
        // Sync mutates the matrix (row overwrites + appends); load an
        // owned copy instead of the read-only query mapping.
        let mut store = crate::embed::VectorStore::load_owned(paths.root())?;
        let mut model = crate::embed::load_model(&paths.models_dir(), out.verbose)?;
        let tag_index = crate::tags::TagIndex::load(conn)?;
        // A layout bump re-embeds every stored card once; otherwise only the
        // names this sync found changed.
        let stale_layout = !doc_version_current(paths)?;
        let targets: Vec<String> = if stale_layout {
            store.meta.names.clone()
        } else {
            delta.to_embed.clone()
        };
        for name in &targets {
            if let Some(card) = crate::db::get_card(conn, name)? {
                upsert_vector(&mut store, &mut model, paths, &card, &tag_index)?;
                tick_embed_progress(out, &targets, name);
            }
        }
        write_status(paths, &store)?;
        mark_doc_version(paths)?;
        out.status("Embedded", &format!("{} cards", targets.len()));
    }

    stamp_synced(paths, now)?;
    stamp_combos_synced(paths, now)?;
    out.finish(
        "Finished",
        &format!(
            "sync: {} added, {} updated, {} priced in-bulk, {} tags, {} combos",
            delta.added, delta.changed, delta.priced, tags_delta.tags, combos_delta
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
    pub added: usize,
    pub changed: usize,
    /// Print rows upserted from the same bulk.
    pub priced: usize,
}

/// Stream the bulk file once; diff cards and harvest prints in one sweep.
///
/// The pass reuses setup's dedup rules (one row per name) and, per stored
/// name, compares an ingest signature to detect content changes. Every
/// passing print row is upserted into `card_prints` (not just the per-name
/// winner), so per-printing prices stay complete. Prints that vanished from
/// the bulk keep their last known row (the snapshot is additive in this
/// mode).
///
/// # Errors
/// Propagates SQLite failures.
///
/// Content signature of a stored row (what re-ingest can change).
fn ingest_signature(card: &crate::db::CardRow) -> String {
    format!(
        "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
        card.mana_cost,
        card.cmc,
        card.type_line,
        card.colors,
        card.keywords,
        card.power.as_deref().unwrap_or(""),
        card.toughness.as_deref().unwrap_or(""),
        card.oracle_text,
        card.released_at,
        card.game_changer.map(|g| g as i64).unwrap_or(-1),
    )
}

/// Content signature of a bulk card, computed the same way.
fn signature_of(card: &crate::scryfall::ScryfallCard) -> String {
    ingest_signature(&crate::db::CardRow {
        name: card.name.clone(),
        oracle_id: card.oracle_id.clone(),
        mana_cost: card.mana_cost.clone().unwrap_or_default(),
        cmc: card.cmc.unwrap_or(0.0),
        type_line: card.type_line.clone().unwrap_or_default(),
        colors: serde_json::to_string(&card.colors.clone().unwrap_or_default()).unwrap_or_default(),
        color_identity: String::new(),
        keywords: serde_json::to_string(&card.keywords.clone().unwrap_or_default())
            .unwrap_or_default(),
        power: card.power.clone(),
        toughness: card.toughness.clone(),
        loyalty: card.loyalty.clone(),
        oracle_text: card.oracle_text.clone().unwrap_or_default(),
        rarity: card.rarity.clone().unwrap_or_default(),
        edhrec_rank: card.edhrec_rank,
        legalities: String::new(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: card.id.clone().unwrap_or_default(),
        released_at: card.released_at.clone().unwrap_or_default(),
        game_changer: card.game_changer,
    })
}

/// One parse pass over the bulk that refreshes `cards`, `card_prints`, and
/// `sets`.
pub fn sync_cards(
    conn: &mut Connection,
    bulk_path: &std::path::Path,
    out: &mut crate::output::Output,
    updated_at: &str,
) -> anyhow::Result<SyncDelta> {
    // Stored rows for the diff.
    let existing = crate::db::load_all_cards(conn)?;
    let mut by_name: std::collections::HashMap<&str, &crate::db::CardRow> =
        existing.iter().map(|c| (c.name.as_str(), c)).collect();

    let mut added: Vec<crate::scryfall::ScryfallCard> = Vec::new();
    let mut changed: Vec<(crate::db::CardRow, crate::scryfall::ScryfallCard)> = Vec::new();
    let mut prints: Vec<crate::scryfall::ScryfallCard> = Vec::new();

    // Bulk rows repeat one name across many sets; keep the "best" print.
    // Non-card rows (tokens, art series) are recorded by name so imports can
    // skip them silently.
    let mut best: std::collections::HashMap<String, crate::scryfall::ScryfallCard> =
        std::collections::HashMap::new();
    let mut token_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut seen = 0usize;
    out.progress_bar("Parsing", "bulk records", 0);
    crate::scryfall::stream_records(bulk_path, |mut card| {
        seen += 1;
        out.tick_progress(1);
        // Never-released cards stay out of the store; the daily sync adds
        // them once their set releases.
        if crate::scryfall::should_skip(&card) || !crate::scryfall::should_ingest(&card) {
            token_names.insert(card.name);
            return;
        }
        crate::scryfall::flatten_faces(&mut card);
        // Every passing print row lands in `card_prints` (per-print prices),
        // while the per-name winner feeds the `cards` dedup.
        prints.push(card.clone());
        best.entry(card.name.clone())
            .and_modify(|current| {
                let better = crate::scryfall::prefer_row(current, &card);
                *current = better;
            })
            .or_insert(card);
    })
    .context("streaming bulk records")?;
    // The record count is unknown until the stream ends; restamp the bar with
    // the true total so the final frame shows a complete bar, not a partial.
    out.progress_bar("Parsing", "bulk records", seen as u64);
    out.tick_progress(seen as u64);
    out.clear_progress();
    out.status(
        "Read",
        &format!(
            "{seen} bulk records ({} tokens, {} cards)",
            token_names.len(),
            best.len(),
        ),
    );

    for (name, card) in &best {
        match by_name.remove(name.as_str()) {
            None => added.push(card.clone()),
            Some(stored) => {
                if ingest_signature(stored) != signature_of(card) {
                    changed.push((stored.clone(), card.clone()));
                }
            }
        }
    }

    let mut to_embed: Vec<String> = Vec::new();
    let changed_n = changed.len();
    if changed_n > 0 {
        out.status("Updating", &format!("{changed_n} changed cards"));
    }
    conn.execute("BEGIN", []).context("begin card updates")?;
    let result = (|| -> anyhow::Result<()> {
        for (stored, card) in &changed {
            crate::scryfall::update_card(conn, stored.name.as_str(), card)?;
            to_embed.push(stored.name.clone());
        }
        for card in &added {
            crate::scryfall::insert_card(conn, card)?;
            to_embed.push(card.name.clone());
        }
        // Token names refresh wholesale from this bulk.
        conn.execute("DELETE FROM token_names", [])?;
        let mut stmt = conn.prepare("INSERT OR IGNORE INTO token_names (name) VALUES (?1)")?;
        for name in &token_names {
            stmt.execute([name])?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute("COMMIT", [])
                .context("committing card updates")?;
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
    }

    // Prints are additive on top of the existing snapshot (prints missing
    // from this bulk keep their last known row).
    conn.execute("BEGIN", []).context("begin print upsert")?;
    let result = (|| -> anyhow::Result<()> {
        for card in &prints {
            crate::scryfall::upsert_print(conn, card, updated_at)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute("COMMIT", [])
                .context("committing print upsert")?;
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
    }

    Ok(SyncDelta {
        to_embed,
        added: added.len(),
        changed: changed_n,
        priced: prints.len(),
    })
}

/// True when the stored vectors were built with the current document layout.
fn doc_version_current(paths: &crate::paths::Paths) -> anyhow::Result<bool> {
    let status = crate::paths::Status::read(&paths.status_file())?;
    Ok(status.doc_version == crate::embed::DOC_VERSION)
}

/// Stamp `doc_version` as current in `status.json`.
fn mark_doc_version(paths: &crate::paths::Paths) -> anyhow::Result<()> {
    let path = paths.status_file();
    let mut status = crate::paths::Status::read(&path)?;
    status.doc_version = crate::embed::DOC_VERSION;
    status.write(&path)
}

/// Emit a one-per-fraction progress line during re-embedding.
fn tick_embed_progress(out: &mut crate::output::Output, targets: &[String], current: &str) {
    let total = targets.len();
    if total < 10 {
        return;
    }
    if let Some(pos) = targets.iter().position(|n| n == current)
        && (pos + 1).is_multiple_of(total / 10)
    {
        out.status("Embedding", &format!("{} of {total} cards", pos + 1));
    }
}

/// True when the store needs a sync: never synced, or older than
/// [`STALE_AFTER`].
pub fn is_stale(status: &crate::paths::Status, now: chrono::DateTime<chrono::Utc>) -> bool {
    if !status.setup_complete {
        return false;
    }
    match chrono::DateTime::parse_from_rfc3339(&status.scryfall_synced_at) {
        Ok(last) => now.signed_duration_since(last) > STALE_AFTER,
        Err(_) => true,
    }
}

/// Insert or update one card's vector at `name`.
///
/// Appends when the name is new to the index; overwrites the row and keeps
/// row order otherwise (row order must stay aligned with `status.json.names`).
///
/// # Errors
/// Propagates model or storage failures.
pub fn upsert_vector(
    store: &mut crate::embed::VectorStore,
    model: &mut fastembed::TextEmbedding,
    paths: &crate::paths::Paths,
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
    store.save_vectors(paths.root())?;
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
        // Unchanged name.
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
        // Different set/collector number/rarity (different chosen print) must
        // not count as a content change: oracle text etc. are identical.
        let mut card = bulk_card("Same", "Text");
        card.set_code = Some("other".into());
        card.collector_number = Some("9".into());
        card.rarity = Some("rare".into());
        card.edhrec_rank = Some(5);
        card.id = Some("sid-2".into());
        let existing = crate::db::load_all_cards(&conn).unwrap();
        let by_name: std::collections::HashMap<&str, &crate::db::CardRow> =
            existing.iter().map(|c| (c.name.as_str(), c)).collect();
        assert!(!by_name.is_empty());
        // Same content -> no change detected.
        let stored = by_name.get("Same").copied().unwrap();
        assert_eq!(ingest_signature(stored), signature_of(&card));
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
        drop(enc);

        let mut out = crate::output::Output::new(true, false, false);
        sync_cards(&mut conn, &bulk, &mut out, "now").unwrap();
        // Tokens are recorded by name and resolvable; real cards are not.
        assert!(crate::db::is_token_name(&conn, "Elf Warrior").unwrap());
        assert!(!crate::db::is_token_name(&conn, "Bolt").unwrap());
    }
}
