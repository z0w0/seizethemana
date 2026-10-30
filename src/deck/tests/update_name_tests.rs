//! Name resolution must happen before operation matching and singleton checks.

use super::*;

/// Oracle and alias rows used by each canonical-name test.
fn oracle() -> rusqlite::Connection {
    let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES ('Lightning Bolt', 'oid')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
        lang, rarity, finishes, released_at, flavor_name, updated_at)
        VALUES ('alias', 'Lightning Bolt', 'tst', '1', 'en', 'common', '[\"nonfoil\"]',
        '2020-01-01', 'Bolt Alias', 'now')",
        [],
    )
    .unwrap();
    conn
}

/// Existing and stale deck lines must remain addressable through update operations.
#[test]
fn canonical_names_apply_to_every_operation_and_keep_unknown_deletions_usable() {
    let conn = oracle();
    let mut out = crate::output::Output::new(true, false, false);
    for input in ["lightning bolt", "Lightning B", "Bolt Alias"] {
        for (kind, spec) in [
            ("add", format!("1 {input}")),
            ("remove", format!("1 {input}")),
            ("set", format!("0 {input}")),
            ("set", format!("2 {input}")),
            ("move", format!("1 {input} to:sideboard")),
        ] {
            let mut ops = vec![parse_op(kind, &spec).unwrap()];
            assert_eq!(validate_names(&conn, &mut out, &mut ops).unwrap(), None);
            let entry = match &ops[0] {
                DeckOp::Add { entry, .. }
                | DeckOp::Remove { entry, .. }
                | DeckOp::Set { entry, .. }
                | DeckOp::Move { entry, .. } => entry,
            };
            assert_eq!(entry.name, "Lightning Bolt", "{kind} {input}");
        }
    }
    let mut ops = vec![
        parse_op("remove", "1 Stale Unknown").unwrap(),
        parse_op("set", "0 Stale Unknown").unwrap(),
    ];
    assert_eq!(validate_names(&conn, &mut out, &mut ops).unwrap(), None);
}

/// An alias or abbreviation must not create a second commander-deck copy.
#[test]
fn all_resolved_spellings_hit_the_same_singleton_guard() {
    let conn = oracle();
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Lightning Bolt\n").unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    for input in ["lightning bolt", "Lightning B", "Bolt Alias"] {
        let mut ops = vec![parse_op("add", &format!("1 {input}")).unwrap()];
        assert_eq!(validate_names(&conn, &mut out, &mut ops).unwrap(), None);
        assert_eq!(
            reject_singleton_adds(&conn, &mut out, &deck, &ops).unwrap(),
            Some(crate::cli::codes::NO_RESULTS)
        );
    }
}

/// Moves and deletions must target the saved canonical line, not create another.
#[test]
fn canonical_move_and_delete_match_existing_lines() {
    let conn = oracle();
    let mut out = crate::output::Output::new(true, false, false);
    let mut deck = Deck::parse("// DECK\n2 Lightning Bolt\n").unwrap();
    let mut ops = vec![
        parse_op("move", "1 Bolt Alias to:sideboard").unwrap(),
        parse_op("set", "0 Lightning B").unwrap(),
    ];
    validate_names(&conn, &mut out, &mut ops).unwrap();
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert!(summary.missing.is_empty());
    assert_eq!(deck.maindeck_total(), 0);
    assert_eq!(deck.sideboard_total(), 1);
    assert_eq!(deck.entries().next().unwrap().name, "Lightning Bolt");
}

/// A preview or rejected operation batch must leave the saved deck unchanged.
#[test]
fn failed_batches_and_dry_runs_keep_the_original_file() {
    let conn = oracle();
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    let original = "// DECK\n1 Lightning Bolt\n";
    let path = paths.deck_file("User Deck Name");
    std::fs::write(&path, original).unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    for (add, remove, dry_run, expected) in [
        (
            vec!["1 Bolt Alias".to_string()],
            vec![],
            true,
            crate::cli::codes::OK,
        ),
        (
            vec!["1 Bolt Alias".to_string(), "1 Unknown".to_string()],
            vec![],
            false,
            crate::cli::codes::NO_RESULTS,
        ),
        (
            vec!["1 Bolt Alias".to_string()],
            vec!["1 Missing".to_string()],
            false,
            crate::cli::codes::NO_RESULTS,
        ),
    ] {
        let code = update(
            &paths,
            &conn,
            &mut out,
            "User Deck Name",
            &add,
            &remove,
            &[],
            &[],
            None,
            false,
            dry_run,
            false,
            false,
            false,
            false,
        )
        .unwrap();
        assert_eq!(code, expected);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
    assert_eq!(std::fs::read_dir(paths.decks_dir()).unwrap().count(), 1);
}
