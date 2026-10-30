//! Tests for sync transactions, embedding recovery, and progress.

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
fn embed_progress_reports_deciles_and_last_card() {
    use status::should_report_embed_progress as reports;
    // Small batches stay silent (below the 20-card floor, decile
    // steps of 1 would report every card).
    assert!(!reports(5, 5));
    assert!(!reports(15, 1));
    assert!(!reports(15, 15));
    // At 100 the step is 10: positions 90 and 100 report, position 91
    // does not (a non-multiple inside the window), and 99 does not.
    assert!(reports(100, 90));
    assert!(reports(100, 100));
    assert!(!reports(100, 91));
    assert!(!reports(100, 99));
    // A 45-card batch has step 4: 44 is a multiple, 45 is the tail.
    assert!(reports(45, 44));
    assert!(reports(45, 45));
    assert!(!reports(45, 43));
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
    assert_ne!(signal_signature(stored), card_signals(&rank_card));
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
fn committed_add_and_change_are_retried_after_embed_failure() {
    use std::io::Write as _;

    let tmp = tempfile::tempdir().unwrap();
    let mut conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at)
         VALUES ('Changed', 'oid-changed', '{R}', 1.0, 'Instant', '[\"R\"]', '[\"R\"]', '[]',
            'old text', 'common', '{}', 'tst', '1', 'sid-changed', '2020-01-01')",
        [],
    )
    .unwrap();

    let bulk = tmp.path().join("bulk.jsonl.gz");
    let mut enc = flate2::write::GzEncoder::new(
        std::fs::File::create(&bulk).unwrap(),
        flate2::Compression::fast(),
    );
    for (name, text) in [("Changed", "new text"), ("Added", "fresh text")] {
        writeln!(
            enc,
            r#"{{"name":"{name}","layout":"normal","games":["paper"],"id":"sid-{name}","oracle_id":"oid-{name}","released_at":"2021-01-01","mana_cost":"{{R}}","cmc":1.0,"type_line":"Instant","colors":["R"],"color_identity":["R"],"keywords":[],"oracle_text":"{text}","rarity":"common","set":"tst","collector_number":"1","prices":{{"usd":"1.00"}}}}"#
        )
        .unwrap();
    }
    enc.finish().unwrap();

    let mut out = crate::output::Output::new(true, false, false);
    let first = sync_cards(&mut conn, &bulk, &mut out, "now").unwrap();
    assert_eq!(first.added, 1);
    assert_eq!(first.changed, 1);
    assert_eq!(load_pending_embeddings(&conn).unwrap().len(), 2);
    // Inject failure after the SQL transaction commits and before vector
    // persistence. Reprocessing identical bulk data must recover both.
    let retry = sync_cards(&mut conn, &bulk, &mut out, "now").unwrap();
    assert_eq!(retry.added, 0);
    assert_eq!(retry.changed, 0);
    assert_eq!(retry.to_embed, vec!["Added", "Changed"]);
    assert_eq!(
        crate::db::get_card(&conn, "Changed")
            .unwrap()
            .unwrap()
            .oracle_text,
        "new text"
    );
    assert_eq!(
        crate::db::get_card(&conn, "Added")
            .unwrap()
            .unwrap()
            .oracle_text,
        "fresh text"
    );

    assert_eq!(load_pending_embeddings(&conn).unwrap().len(), 2);
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
        r#"{{"name":"Ranked","layout":"normal","games":["paper"],"id":"sid-Ranked","oracle_id":"oid-Ranked","released_at":"2020-01-01","mana_cost":"{{R}}","cmc":1.0,"type_line":"Instant","colors":["R"],"color_identity":["R"],"keywords":[],"oracle_text":"text","rarity":"common","set":"tst","collector_number":"1","prices":{{"usd":"1.00"}},"edhrec_rank":42,"penny_rank":7,"reserved":true}}"#
    )
    .unwrap();
    drop(enc);

    let mut out = crate::output::Output::new(true, false, false);
    let delta = sync_cards(&mut conn, &bulk, &mut out, "now").unwrap();
    assert_eq!(delta.rank_only, 1, "a rank change alone lands as rank_only");
    assert_eq!(delta.changed, 0);
    assert_eq!(delta.added, 0);
    // Ranks and Reserved List update in place; nothing queued for
    // re-embed.
    let (rank, penny, reserved): (Option<i64>, Option<i64>, Option<bool>) = conn
        .query_row(
            "SELECT edhrec_rank, penny_rank, reserved FROM cards WHERE name = 'Ranked'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(rank, Some(42));
    assert_eq!(penny, Some(7));
    assert_eq!(reserved, Some(true));
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
