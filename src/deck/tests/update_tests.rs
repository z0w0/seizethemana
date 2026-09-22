use super::super::maintain::dedupe_deck;
use super::*;

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
/// name as a regular limited card.
fn empty_conn() -> rusqlite::Connection {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    std::mem::forget(tmp);
    conn
}

/// Silent output for tests that call the reporting helpers.
fn silent_out() -> crate::output::Output {
    crate::output::Output::new(true, false, false)
}

#[test]
fn validation_flags_unknown_names() {
    let (_tmp, conn) = seeded_conn();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 Not A Real Card").unwrap()];
    let code = validate_names(&conn, &mut out, &ops).unwrap();
    assert_eq!(code, Some(crate::cli::codes::NO_RESULTS));
}

#[test]
fn validation_passes_known_names() {
    let (_tmp, conn) = seeded_conn();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 lightning bolt").unwrap()];
    assert_eq!(validate_names(&conn, &mut out, &ops).unwrap(), None);
    // Unique-prefix resolution counts too.
    let ops = vec![parse_op("set", "2 Lightning B").unwrap()];
    assert_eq!(validate_names(&conn, &mut out, &ops).unwrap(), None);
}

#[test]
fn validation_skips_removes_and_zero_sets() {
    let (_tmp, conn) = seeded_conn();
    let mut out = silent_out();
    // Removing a name that is not a card at all is allowed (deleting a
    // stale line); `--set 0` deletes likewise.
    let ops = vec![
        parse_op("remove", "1 Not A Card").unwrap(),
        parse_op("set", "0 Also Not A Card").unwrap(),
    ];
    assert_eq!(validate_names(&conn, &mut out, &ops).unwrap(), None);
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
fn reject_singleton_adds_blocks_over_limit_adds() {
    // Commander deck: adding a second copy of an existing card rejects.
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n").unwrap();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 Bolt").unwrap()];
    assert_eq!(
        reject_singleton_adds(&empty_conn(), &mut out, &deck, &ops).unwrap(),
        Some(crate::cli::codes::NO_RESULTS)
    );
    // A 60-card deck never rejects.
    let flat = Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let mut out = silent_out();
    assert_eq!(
        reject_singleton_adds(&empty_conn(), &mut out, &flat, &ops).unwrap(),
        None
    );
    // Basics are exempt.
    let commander = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Island\n").unwrap();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "5 Island").unwrap()];
    assert_eq!(
        reject_singleton_adds(&empty_conn(), &mut out, &commander, &ops).unwrap(),
        None
    );
    // Adding a new card (not in the deck) is fine.
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 Shock").unwrap()];
    assert_eq!(
        reject_singleton_adds(&empty_conn(), &mut out, &deck, &ops).unwrap(),
        None
    );
    // A sideboard copy does not count against the singleton total: the
    // sideboard is a wishlist, so a first maindeck copy is still legal.
    let split =
        Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n// SIDEBOARD\n1 Shock\n").unwrap();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 Shock").unwrap()];
    assert_eq!(
        reject_singleton_adds(&empty_conn(), &mut out, &split, &ops).unwrap(),
        None
    );
    // Set and Move ops are never rejected here.
    let mut out = silent_out();
    let ops = vec![parse_op("set", "2 Bolt").unwrap()];
    assert_eq!(
        reject_singleton_adds(&empty_conn(), &mut out, &deck, &ops).unwrap(),
        None
    );
}

#[test]
fn singleton_warnings_flag_only_commander_decks() {
    // No COMMANDER section: no warnings at all.
    let deck = Deck::parse("// DECK\n2 Bolt\n").unwrap();
    let ops = vec![parse_op("add", "2 Bolt").unwrap()];
    assert!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .is_empty()
    );
    // With a COMMANDER section, a post-apply hold of two warns.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n").unwrap();
    let ops = vec![parse_op("add", "2 Bolt").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(
        singleton_warnings(&empty_conn(), &ops, &deck).unwrap(),
        vec!["Bolt would exceed the singleton limit; commander decks hold one copy".to_string()]
    );
    // Basics are exempt.
    let ops = vec![parse_op("add", "20 Island").unwrap()];
    assert!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .is_empty()
    );
    // A set that lowers to 1 does not warn.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n4 Bolt\n").unwrap();
    let ops = vec![parse_op("set", "1 Bolt").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .is_empty()
    );
    // A set that raises to 3 warns after apply.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n").unwrap();
    let ops = vec![parse_op("set", "3 Bolt").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn singleton_warnings_read_post_apply_deck_state() {
    // A first-copy add of an absent card must not warn: the deck now
    // holds one copy.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n").unwrap();
    let ops = vec![parse_op("add", "1 Secluded Courtyard").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .is_empty()
    );
    // A multi-copy add still warns.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n").unwrap();
    let ops = vec![parse_op("add", "2 Secluded Courtyard").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .len(),
        1
    );
    // Adding a second copy of a held card still warns.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Secluded Courtyard\n").unwrap();
    let ops = vec![parse_op("add", "1 Secluded Courtyard").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .len(),
        1
    );
    // A move nets to one copy: no warning (the false-positive case
    // that motivated post-apply semantics).
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n// SIDEBOARD\n").unwrap();
    let ops = vec![parse_op("move", "1 Bolt to:sideboard").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .is_empty()
    );
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
fn singleton_warnings_read_post_apply_state() {
    // A legal remove+add pair nets to one copy; the post-apply deck
    // holds one copy, so no warning fires. (The old pre-apply reading
    // warned here — the regression this test pins.)
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n").unwrap();
    let mut deck = deck;
    let ops = vec![
        parse_op("remove", "1 Bolt").unwrap(),
        parse_op("add", "1 Bolt").unwrap(),
    ];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .is_empty(),
        "a legal move must not warn"
    );
    // A genuine double-copy hold still warns after apply.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n").unwrap();
    let ops = vec![parse_op("add", "1 Bolt").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(
        singleton_warnings(&empty_conn(), &ops, &deck)
            .unwrap()
            .len(),
        1
    );
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

/// Dry-run through the command surface: nothing is written and the exit
/// code is clean when no sim problems appear.
#[test]
fn update_dry_run_writes_nothing() {
    let (tmp, conn) = seeded_conn();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Froggy"), "// DECK\n1 Lightning Bolt\n").unwrap();
    let mut out = silent_out();
    let before_text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &["1 Lightning Bolt".to_string()],
        &[],
        &[],
        &[],
        None,
        false,
        true,
        false,
        false,
        false,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let after_text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    assert_eq!(before_text, after_text, "dry-run must not write the deck");
}

#[test]
fn dry_run_legal_flags_violation() {
    // Adding a second copy in a commander deck trips the singleton rule
    // in the --legal verdict, and the preview exits 1.
    let (tmp, conn) = seeded_conn();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Breya', 'o1', '', 0, 'Legendary Creature — Human', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's1', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    for i in 0..10 {
        conn.execute(
            &format!(
                "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                    keywords, oracle_text, rarity, legalities, set_code, collector_number,
                    scryfall_id, released_at, game_changer, edhrec_rank)
                 VALUES ('Island{i}', 'o3-{i}', '', 0, 'Basic Land — Island', '[]', '[]', '[]', '',
                    'common', '{{\"commander\":\"legal\"}}', 'tst', '1', 's3-{i}', '2020-01-01', NULL, NULL)"
            ),
            [],
        )
        .unwrap();
    }
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n10 Island\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &["2 Bolt".to_string()],
        &[],
        None,
        false,
        true,
        false,
        true,
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        code,
        crate::cli::codes::ERROR,
        "illegal post-change deck fails the preview"
    );
}

#[test]
fn backfill_basics_reaches_target_size() {
    // A trimmed commander deck backfills to 100 cards with an on-color
    // basic (Breya's identity starts with W → Plains).
    let (tmp, conn) = seeded_conn();
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Breya', 'o1', '', 0, 'Legendary Creature — Human', '[]', '[\"W\",\"U\",\"B\",\"R\"]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's1', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &[],
        &[],
        None,
        false,
        false,
        false,
        false,
        true,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    let deck = crate::deck::Deck::parse(&text).unwrap();
    assert_eq!(deck.total(), 100, "backfill reaches the commander size");
    let plains: i64 = deck
        .entries()
        .filter(|e| e.name == "Plains")
        .map(|e| e.quantity)
        .sum();
    assert_eq!(
        plains, 98,
        "the first identity color's basic fills the deficit"
    );
}

#[test]
fn dry_run_legal_uses_constructed_rules_for_60_card_decks() {
    // A 60-card-shaped deck must not be judged by commander rules in the
    // --legal verdict: singleton and exact-100 checks would reject it.
    let (tmp, conn) = seeded_conn();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"modern\":\"legal\",\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    for i in 0..56 {
        conn.execute(
            &format!(
                "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                    keywords, oracle_text, rarity, legalities, set_code, collector_number,
                    scryfall_id, released_at, game_changer, edhrec_rank)
                 VALUES ('Island{i}', 'o3-{i}', '', 0, 'Basic Land — Island', '[]', '[]', '[]', '',
                    'common', '{{\"modern\":\"legal\"}}', 'tst', '1', 's3-{i}', '2020-01-01', NULL, NULL)"
            ),
            [],
        )
        .unwrap();
    }
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    let mut deck_text = String::from("// DECK\n3 Bolt\n");
    for i in 0..56 {
        deck_text.push_str(&format!("1 Island{i}\n"));
    }
    std::fs::write(paths.deck_file("Storm"), deck_text).unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Storm",
        &["1 Bolt".to_string()],
        &[],
        &[],
        &[],
        None,
        false,
        true,
        false,
        true,
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        code,
        crate::cli::codes::OK,
        "a 59-card constructed deck adding its 60th card passes the preview"
    );
}

#[test]
fn backfill_ignores_sideboard_when_counting_deficit() {
    // The sideboard is a wishlist: a 95-card maindeck with 15 sideboard
    // cards must still backfill 5 basics.
    let (tmp, conn) = seeded_conn();
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Breya', 'o1', '', 0, 'Legendary Creature — Human', '[]', '[\"W\"]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's1', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n94 Plains\n// SIDEBOARD\n15 Bolt\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &[],
        &[],
        None,
        false,
        false,
        false,
        false,
        true,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    let deck = crate::deck::Deck::parse(&text).unwrap();
    assert_eq!(
        deck.maindeck_total(),
        100,
        "the deficit counts maindeck cards only"
    );
    assert_eq!(deck.sideboard_total(), 15, "sideboard is untouched");
}

#[test]
fn dry_run_backfill_previews_backfilled_deck() {
    // In dry-run, the backfill applies to the preview clone so --legal
    // judges the end state; the deck file stays untouched.
    let (tmp, conn) = seeded_conn();
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Breya', 'o1', '', 0, 'Legendary Creature — Human', '[]', '[\"W\"]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's1', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Plains', 'o4', '', 0, 'Basic Land — Plains', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's4', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &[],
        &[],
        None,
        false,
        true,
        false,
        true,
        true,
        false,
    )
    .unwrap();
    assert_eq!(
        code,
        crate::cli::codes::OK,
        "the backfilled preview deck is legal"
    );
    let text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    assert_eq!(
        text, "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n",
        "dry-run still writes nothing"
    );
}

#[test]
fn unlimited_copy_cards_escape_singleton_guards() {
    // "A deck can have any number of cards named ..." oracle text exempts
    // a card from the singleton guard, the warnings, and dedupe collapse.
    let (tmp, conn) = seeded_conn();
    conn.execute("DELETE FROM cards WHERE name = 'Lightning Bolt'", [])
        .unwrap();
    for (name, oid, text) in [
        ("Breya", "o1", ""),
        (
            "Shadowborn Apostle",
            "o2",
            "A deck can have any number of cards named Shadowborn Apostle.",
        ),
        ("Bolt", "o3", ""),
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                keywords, oracle_text, rarity, legalities, set_code, collector_number,
                scryfall_id, released_at, game_changer, edhrec_rank)
             VALUES (?1, ?2, '', 0, 'Creature', '[]', '[]', '[]', ?3,
                'common', '{\"commander\":\"legal\"}', 'tst', '1', 's', '2020-01-01', NULL, NULL)",
            rusqlite::params![name, oid, text],
        )
        .unwrap();
    }
    let _ = tmp;
    // Guard: adding a 2nd Apostle passes.
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Shadowborn Apostle\n").unwrap();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 Shadowborn Apostle").unwrap()];
    assert_eq!(
        reject_singleton_adds(&conn, &mut out, &deck, &ops).unwrap(),
        None
    );
    // Warnings: 30 Apostle copies stay silent.
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n30 Shadowborn Apostle\n").unwrap();
    assert!(singleton_warnings(&conn, &ops, &deck).unwrap().is_empty());
    // Dedupe: 2 duplicate Apostle lines merge, nothing collapses to 1.
    let deck = Deck::parse(
        "// COMMANDER\n1 Breya\n// DECK\n20 Shadowborn Apostle\n10 Shadowborn Apostle\n",
    )
    .unwrap();
    let result = dedupe_deck(&conn, &deck).unwrap();
    let out_deck = result.deck;
    let merged = result.merged_lines;
    let cards = result.merged_cards;
    assert_eq!(merged, 1);
    assert_eq!(out_deck.total(), 31, "Apostle keeps its summed copies");
    assert!(!cards.iter().any(|(n, _)| n == "Breya"));
    // A regular card still collapses to 1.
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n2 Bolt\n").unwrap();
    let result = dedupe_deck(&conn, &deck).unwrap();
    let out_deck = result.deck;
    let merged = result.merged_lines;
    assert_eq!(merged, 1);
    assert_eq!(out_deck.total(), 2);
}
