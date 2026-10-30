use super::*;

#[test]
fn owned_only_survives_the_limit_cut() {
    // `--owned` must trim the pool to owned names BEFORE the limit cut:
    // an owned card ranked below --limit still surfaces (the collection
    // is the whole candidate pool). The store holds 6 identical frog
    // commanders; only the lowest-fused one is owned. With limit 3 the
    // old owned-after-truncate order dropped it; the new order keeps it.
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    for i in 0..6 {
        let name = format!("Frog Chief {i}");
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '', 4, 'Legendary Creature — Frog', '[]', '[\"G\"]',
                '[]', 'text', 'rare', '{\"commander\":\"legal\"}', 'tst', '1',
                ?2, '2020-01-01')",
            rusqlite::params![name, format!("oid-{i}")],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('t1', 'commander', 'commander', 1)",
        [],
    )
    .unwrap();
    for i in 0..6 {
        conn.execute(
            "INSERT INTO card_tags (oracle_id, tag_id) VALUES (?1, 't1')",
            rusqlite::params![format!("oid-{i}")],
        )
        .unwrap();
    }
    // Only "Frog Chief 5" is owned; the tag-leg order sorts by label count
    // then name, so it ranks last and lands outside the limit-3 window.
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity, foil)
         VALUES ('Frog Chief 5', 'binder', 'binder', 1, 'normal')",
        [],
    )
    .unwrap();
    let paths = crate::paths::Paths::new(dir.path().to_path_buf());
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 6,
        embedded_cards: 6,
        model: "m".into(),
        dim: 384,
        names: vec![],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    std::fs::write(paths.root().join("vectors.bin"), Vec::<u8>::new()).unwrap();
    let deck = crate::deck::Deck::parse("// DECK\n1 Forest\n").unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let owned: std::collections::HashSet<String> = ["Frog Chief 5".to_string()].into();
    let ranked = crate::deck::suggest::commander_candidates(
        &paths,
        &conn,
        &mut out,
        &deck,
        Some("frog"),
        None,
        None,
        None,
        3,
        &[],
        Some(&owned),
        false,
    )
    .unwrap();
    assert_eq!(
        ranked.len(),
        1,
        "the owned card outside the fused top-3 survives the --owned filter"
    );
    assert_eq!(ranked[0].0.name, "Frog Chief 5");
}

#[test]
fn exclude_resolves_files_and_filters_candidates() {
    // Names and a file of names both resolve; the retained list drops
    // excluded cards by name.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("banned.txt");
    std::fs::write(&file, "# never suggest these\nHeld A\nHeld B\n").unwrap();
    let excluded =
        resolve_exclusions(&["Wanted".to_string(), file.to_str().unwrap().to_string()]).unwrap();
    assert_eq!(
        excluded,
        ["Wanted", "Held A", "Held B"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
    let ranked = vec![
        (
            card("Wanted", "Creature", "GU", "text", Some(10)),
            Vec::<String>::new(),
            0.9f32,
        ),
        (
            card("Held A", "Creature", "GU", "text", Some(50)),
            Vec::new(),
            0.5,
        ),
        (
            card("Kept", "Creature", "GU", "text", Some(60)),
            Vec::new(),
            0.4,
        ),
    ];
    let mut ranked = ranked;
    retain_unexcluded(&mut ranked, &excluded);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].0.name, "Kept");
}

#[test]
fn exclude_dash_reads_names_from_stdin() {
    // "-" reads the name list from stdin: one per line, `#` comments
    // allowed, same as a file.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pipedin.txt");
    std::fs::write(&file, "# comments skipped\nPiped Name\n").unwrap();
    let mut piped = std::fs::File::open(&file).unwrap();
    let excluded = resolve_exclusions_with(&["-".to_string(), "Kept".to_string()], &mut piped);
    assert_eq!(
        excluded.unwrap(),
        ["Piped Name", "Kept"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn exclude_before_limit_keeps_full_result_count() {
    // Excluded cards never consume limit slots: excluding 2 of 6
    // commanders still returns the full --limit of 3 non-excluded rows.
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    for i in 0..6 {
        let name = format!("Frog Chief {i}");
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '', 4, 'Legendary Creature — Frog', '[]', '[\"G\"]',
                '[]', 'text', 'rare', '{\"commander\":\"legal\"}', 'tst', '1',
                ?2, '2020-01-01')",
            rusqlite::params![name, format!("oid-{i}")],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('t1', 'commander', 'commander', 1)",
        [],
    )
    .unwrap();
    for i in 0..6 {
        conn.execute(
            "INSERT INTO card_tags (oracle_id, tag_id) VALUES (?1, 't1')",
            rusqlite::params![format!("oid-{i}")],
        )
        .unwrap();
    }
    let paths = crate::paths::Paths::new(dir.path().to_path_buf());
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 6,
        embedded_cards: 6,
        model: "m".into(),
        dim: 384,
        names: vec![],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    std::fs::write(paths.root().join("vectors.bin"), Vec::<u8>::new()).unwrap();
    let deck = crate::deck::Deck::parse("// DECK\n1 Forest\n").unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    // The top-2 fused ranks are the first candidates; excluding them must
    // not shrink the result: 3 non-excluded rows still come back.
    let excluded = ["Frog Chief 0".to_string(), "Frog Chief 1".to_string()];
    let ranked = crate::deck::suggest::commander_candidates(
        &paths,
        &conn,
        &mut out,
        &deck,
        Some("frog"),
        None,
        None,
        None,
        3,
        &excluded,
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        ranked.len(),
        3,
        "exclusions drop before the limit cut; the limit is still filled"
    );
    assert!(
        !ranked.iter().any(|(c, _, _)| excluded.contains(&c.name)),
        "no excluded card survives"
    );
}
