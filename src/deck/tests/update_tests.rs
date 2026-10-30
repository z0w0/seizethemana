use super::super::maintain::dedupe_deck;
use super::*;

/// Basic-land backfill must fill the maindeck and preserve dry-run files.
#[path = "update_backfill_tests.rs"]
mod update_backfill_tests;

/// Update previews must report changes without writing deck files.
#[path = "update_preview_tests.rs"]
mod update_preview_tests;

/// Singleton checks and deduplication must respect card copy limits.
#[path = "update_singleton_tests.rs"]
mod update_singleton_tests;

/// Connection plus its backing tempdir (must outlive the connection).
fn seeded_conn() -> (tempfile::TempDir, rusqlite::Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, released_at) VALUES ('Lightning Bolt', 'oid', '2020-01-01')",
        [],
    )
    .unwrap();
    (tmp, conn)
}

/// Connection with an empty oracle: singleton checks treat every test
/// name as a regular limited card. In-memory, so no tempdir is leaked.
fn empty_conn() -> rusqlite::Connection {
    crate::db::open(std::path::Path::new(":memory:")).unwrap()
}

/// Silent output for tests that call the reporting helpers.
fn silent_out() -> crate::output::Output {
    crate::output::Output::new(true, false, false)
}

#[test]
fn validation_flags_unknown_names() {
    let (_tmp, conn) = seeded_conn();
    let mut out = silent_out();
    let mut ops = vec![parse_op("add", "1 Not A Real Card").unwrap()];
    let code = validate_names(&conn, &mut out, &mut ops).unwrap();
    assert_eq!(code, Some(crate::cli::codes::NO_RESULTS));
}

#[test]
fn validation_passes_known_names() {
    let (_tmp, conn) = seeded_conn();
    let mut out = silent_out();
    let mut ops = vec![parse_op("add", "1 lightning bolt").unwrap()];
    assert_eq!(validate_names(&conn, &mut out, &mut ops).unwrap(), None);
    assert!(matches!(&ops[0], DeckOp::Add { entry, .. } if entry.name == "Lightning Bolt"));
    // Unique-prefix resolution counts too.
    let mut ops = vec![parse_op("set", "2 Lightning B").unwrap()];
    assert_eq!(validate_names(&conn, &mut out, &mut ops).unwrap(), None);
    assert!(matches!(&ops[0], DeckOp::Set { entry, .. } if entry.name == "Lightning Bolt"));
}

#[test]
fn validation_skips_removes_and_zero_sets() {
    let (_tmp, conn) = seeded_conn();
    let mut out = silent_out();
    // Removing a name that is not a card at all is allowed (deleting a
    // stale line); `--set 0` deletes likewise.
    let mut ops = vec![
        parse_op("remove", "1 Not A Card").unwrap(),
        parse_op("set", "0 Also Not A Card").unwrap(),
    ];
    assert_eq!(validate_names(&conn, &mut out, &mut ops).unwrap(), None);
}

#[test]
fn update_saves_canonical_card_name_without_renaming_deck_file() {
    let (tmp, conn) = seeded_conn();
    conn.execute(
        "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
            lang, rarity, finishes, released_at, flavor_name, updated_at)
         VALUES ('bolt-alias', 'Lightning Bolt', 'tst', '1', 'en',
            'common', '[\"nonfoil\"]', '2020-01-01', 'Bolt Alias', 'now')",
        [],
    )
    .unwrap();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("My Deck"), "// DECK\n").unwrap();
    let mut out = silent_out();

    let code = update(
        &paths,
        &conn,
        &mut out,
        "My Deck",
        &["1 lightning b".to_string(), "1 Bolt Alias".to_string()],
        &[],
        &[],
        &[],
        None,
        false,
        false,
        false,
        false,
        false,
        false,
    )
    .unwrap();

    assert_eq!(code, crate::cli::codes::OK);
    assert!(paths.deck_file("My Deck").exists());
    assert!(!paths.deck_file("Lightning Bolt").exists());
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("My Deck")).unwrap(),
        "// DECK\n2 Lightning Bolt\n"
    );
}

#[test]
fn parse_op_variants() {
    let op = parse_op("add", "2 Lightning Bolt").unwrap();
    match op {
        DeckOp::Add { section, entry } => {
            assert_eq!(section, None);
            assert_eq!(entry.quantity, 2);
            assert_eq!(entry.name, "Lightning Bolt");
        }
        other => panic!("unexpected {other:?}"),
    }
    let op = parse_op("add", "commander:1 Breya (MH3) 372 *F*").unwrap();
    match op {
        DeckOp::Add { section, entry } => {
            assert_eq!(section.as_deref(), Some("commander"));
            assert_eq!(entry.name, "Breya");
            assert!(entry.foil);
        }
        other => panic!("unexpected {other:?}"),
    }
    let op = parse_op("remove", "2 Bolt").unwrap();
    assert!(matches!(op, DeckOp::Remove { .. }));
    let op = parse_op("set", "4 Bolt (TST) 1").unwrap();
    assert!(matches!(op, DeckOp::Set { .. }));
    assert!(parse_op("add", "Bolt").is_err());
    assert!(parse_op("nope", "1 Bolt").is_err());
}

#[test]
fn ops_do_increment_and_decrement_math() {
    let mut deck = Deck::parse("// DECK\n2 Bolt\n1 Breya\n").unwrap();
    let ops = vec![
        parse_op("add", "2 Bolt").unwrap(),
        parse_op("remove", "1 Breya").unwrap(),
        parse_op("set", "4 Bolt").unwrap(),
        parse_op("set", "0 Breya").unwrap(),
    ];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.added, 2);
    assert_eq!(summary.removed, 1);
    // Both sets are no-ops: Bolt already reads 4 after the add, and Breya
    // is gone after the remove. Only real changes count (bug 24).
    assert_eq!(summary.set, 0);
    assert!(summary.missing.is_empty());
    // Bolt was +2 then set to 4; Breya deleted by set 0.
    assert_eq!(deck.total(), 4);
    assert!(deck.entries().all(|e| e.name != "Breya"));
    assert_eq!(deck.to_text(), "// DECK\n4 Bolt\n");
}

#[test]
fn ops_remove_line_when_decrementing_to_zero() {
    let mut deck = Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let ops = vec![parse_op("remove", "1 Bolt").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.removed, 1);
    assert_eq!(deck.total(), 0);
}

#[test]
fn ops_report_missing_cards() {
    let mut deck = Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let ops = vec![parse_op("remove", "1 Nope").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.missing, vec!["Nope"]);
}

#[test]
fn ops_target_the_named_section() {
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// SIDEBOARD\n1 Bolt\n").unwrap();
    let ops = vec![parse_op("add", "sideboard:1 Bare").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(deck.sections[1].1.len(), 2);
    // Existing section match is case-insensitive; unknown sections append.
    assert_eq!(deck.section_index("SIDEBOARD"), Some(1));
}

#[test]
fn name_only_ops_match_lines_with_print_info() {
    // A spec without (SET) cn targets the card in any printing — the
    // collection's "any print fills a slot" identity, not exact prints.
    let mut deck =
        Deck::parse("// DECK\n1 Weapons Manufacturing (EOE) 311\n1 Island (SOS) 274\n").unwrap();
    let ops = vec![parse_op("remove", "1 Weapons Manufacturing").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.removed, 1);
    assert!(summary.missing.is_empty());
    assert_eq!(deck.total(), 1);

    let ops = vec![parse_op("set", "0 Island").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.set, 1);
    assert!(summary.missing.is_empty());
    assert_eq!(deck.total(), 0);
}

#[test]
fn print_qualified_ops_stay_print_exact() {
    // An op that names a print must not hit a line with a different one.
    let mut deck = Deck::parse("// DECK\n1 Bolt (M11) 148\n1 Bolt (2XM) 124\n").unwrap();
    let ops = vec![parse_op("remove", "1 Bolt (M11) 148").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.removed, 1);
    assert_eq!(deck.total(), 1);
    let remaining: Vec<_> = deck.entries().collect();
    assert_eq!(remaining[0].set_code.as_deref(), Some("2XM"));
}

#[test]
fn parse_spec_line_reads_verbs_and_bare_specs() {
    match parse_spec_line("add 1 Bolt").unwrap() {
        DeckOp::Add { entry, .. } => assert_eq!(entry.name, "Bolt"),
        other => panic!("unexpected {other:?}"),
    }
    match parse_spec_line("remove 1 Bolt").unwrap() {
        DeckOp::Remove { entry, .. } => assert_eq!(entry.name, "Bolt"),
        other => panic!("unexpected {other:?}"),
    }
    match parse_spec_line("set 2 Bolt").unwrap() {
        DeckOp::Set { entry, .. } => assert_eq!(entry.quantity, 2),
        other => panic!("unexpected {other:?}"),
    }
    match parse_spec_line("move 1 Bolt to:sideboard") {
        Ok(DeckOp::Move { to, entry, .. }) => {
            assert_eq!(entry.name, "Bolt");
            assert_eq!(to, "sideboard");
        }
        other => panic!("unexpected {other:?}"),
    }
    // Bare spec defaults to add.
    match parse_spec_line("3 Bolt").unwrap() {
        DeckOp::Add { entry, .. } => assert_eq!(entry.quantity, 3),
        other => panic!("unexpected {other:?}"),
    }
    assert!(parse_spec_line("shuffle 1 Bolt").is_err());
}

#[test]
fn move_ops_parse_from_and_to() {
    // Explicit from + explicit to.
    match parse_op("move", "sideboard:1 Bolt to:deck").unwrap() {
        DeckOp::Move { from, to, entry } => {
            assert_eq!(from.as_deref(), Some("sideboard"));
            assert_eq!(to, "deck");
            assert_eq!(entry.name, "Bolt");
            assert_eq!(entry.quantity, 1);
        }
        other => panic!("unexpected {other:?}"),
    }
    // No to: clause defaults to DECK.
    match parse_op("move", "sideboard:1 Bolt").unwrap() {
        DeckOp::Move { from, to, .. } => {
            assert_eq!(from.as_deref(), Some("sideboard"));
            assert_eq!(to, "DECK");
        }
        other => panic!("unexpected {other:?}"),
    }
    // Unqualified moves DECK→DECK (harmless but valid).
    match parse_op("move", "2 Bolt to:sideboard").unwrap() {
        DeckOp::Move { from, to, .. } => {
            assert_eq!(from, None);
            assert_eq!(to, "sideboard");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn ops_move_copies_between_sections() {
    let mut deck =
        Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n// SIDEBOARD\n1 Strix\n").unwrap();
    let ops = vec![parse_op("move", "1 Bolt to:sideboard").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.moved, 1);
    assert!(summary.missing.is_empty());
    let sb: Vec<_> = deck.section_entries_mut("SIDEBOARD").clone();
    assert_eq!(sb.len(), 2, "Bolt joined Strix in the sideboard");
    let main = deck.section_entries_mut("DECK").len();
    assert_eq!(main, 0);
    // 1 commander + 1 Bolt + 1 Strix: a move never changes the count.
    assert_eq!(deck.total(), 3, "net count unchanged by a move");
}

#[test]
fn move_missing_card_records_missing() {
    let mut deck = Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let ops = vec![parse_op("move", "1 Strix to:sideboard").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.missing, vec!["Strix"]);
    assert_eq!(deck.total(), 1, "nothing changed");
}

#[test]
fn set_zero_falls_back_to_other_sections() {
    // Unqualified `--set 0` deletes a sideboard-only line (parity with
    // unqualified remove).
    let mut deck = Deck::parse("// DECK\n1 Bolt\n// SIDEBOARD\n1 Strix\n").unwrap();
    let ops = vec![parse_op("set", "0 Strix").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.set, 1);
    assert!(
        summary
            .relocated
            .contains(&("Strix".to_string(), "SIDEBOARD".to_string()))
    );
    assert_eq!(deck.total(), 1);
    // Qualified set-0 stays strict: no fallback.
    let mut deck = Deck::parse("// DECK\n1 Bolt\n// SIDEBOARD\n1 Strix\n").unwrap();
    let ops = vec![parse_op("set", "deck:0 Strix").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(
        deck.total(),
        2,
        "qualified set-0 leaves other sections alone"
    );
}

#[test]
fn move_onto_existing_line_sums_quantities() {
    let mut deck = Deck::parse("// DECK\n1 Bolt\n// SIDEBOARD\n2 Strix\n").unwrap();
    let ops = vec![parse_op("move", "sideboard:1 Strix to:deck").unwrap()];
    let summary = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(summary.moved, 1);
    let main = deck.section_entries_mut("DECK").clone();
    assert_eq!(main.len(), 2);
    assert_eq!(main[1].name, "Strix");
    assert_eq!(main[1].quantity, 1);
}

/// Newly added basic lands never appear in the buylist, even when the
/// before-deck has no such basic (the name check must not depend on the
/// oracle rows of the before-deck).
#[test]
fn cost_delta_ignores_newly_added_basics() {
    let (tmp, conn) = seeded_conn();
    let before = Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let after = Deck::parse("// DECK\n1 Bolt\n3 Island\n").unwrap();
    let impact = cost_delta(&conn, &before, &after).unwrap();
    assert!(impact.to_buy.items.is_empty());
    let _ = &tmp;
}

/// A failed price read surfaces as an error, not a silent $0.00 cost.
#[test]
fn cost_delta_propagates_price_read_failure() {
    let (tmp, conn) = seeded_conn();
    conn.execute_batch("DROP TABLE card_prints;").unwrap();
    let before = Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let after = Deck::parse("// DECK\n1 Bolt\n2 Bolt\n").unwrap();
    assert!(
        cost_delta(&conn, &before, &after).is_err(),
        "a dropped price table must error, not report $0.00"
    );
    let _ = &tmp;
}

/// The dry-run cost delta: an extra demanded copy the collection cannot
/// cover prices at the cheapest printing; a released slot frees its value.
#[test]
fn cost_delta_matches_buy_and_freed() {
    let (tmp, conn) = seeded_conn();
    conn.execute(
        "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity)
         VALUES ('Lightning Bolt', 'tst', '1', 'normal', 'Trade', 'binder', 1)",
        [],
    )
    .unwrap();
    let before = Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let after = Deck::parse("// DECK\n3 Bolt\n1 Force of Will\n").unwrap();
    // Force of Will has no card row here, so it is unpriced; the "Bolt"
    // prefix line is its own name entry and stays short (owned copies
    // key under "Lightning Bolt"). BTreeMap sorts the item names.
    let impact = cost_delta(&conn, &before, &after).unwrap();
    let names: Vec<&str> = impact
        .to_buy
        .items
        .iter()
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(names, vec!["Bolt", "Force of Will"]);
    // Removing Bolt frees its one owned copy's slot.
    let freed = cost_delta(&conn, &after, &before).unwrap();
    assert_eq!(freed.to_buy.items.len(), 0);
    assert!(freed.freed_usd >= 0.0, "freed value is not negative");
    let _ = &tmp;
}

#[test]
fn allow_partial_continues_and_exits_error() {
    // `--allow-partial`: a remove that misses the deck is reported and
    // the batch still applies and saves; the exit code is ERROR (1).
    let (tmp, conn) = seeded_conn();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Froggy"), "// DECK\n1 Lightning Bolt\n").unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &["1 Ghost Card".to_string()],
        &[],
        &[],
        None,
        true,
        false,
        false,
        false,
        false,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::ERROR, "partial batch exits ERROR");
    // The deck was still written (the resolvable state stands).
    let text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    assert_eq!(text, "// DECK\n1 Lightning Bolt\n");

    // Without --allow-partial the same update stops with exit 3 and
    // writes nothing.
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &["1 Ghost Card".to_string()],
        &[],
        &[],
        None,
        false,
        false,
        false,
        false,
        false,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::NO_RESULTS);
}
