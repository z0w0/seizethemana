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
    assert_eq!(summary.set, 2);
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
fn dedupe_merges_same_name_lines_per_section() {
    let deck =
        Deck::parse("// DECK\n1 Bolt\n2 Bolt (M11) 148\n3 Bolt\n// SIDEBOARD\n2 Bolt\n").unwrap();
    let (mut out, merged, cards) = dedupe_deck(&deck);
    assert_eq!(merged, 2, "two duplicates merged");
    assert_eq!(
        cards,
        vec![("Bolt".to_string(), 2), ("Bolt".to_string(), 3)]
    );
    assert_eq!(out.total(), 8); // 6 Bolt + 2 sideboard Bolt
    // Quantities sum; first line's print info wins.
    let deck_entries: Vec<_> = out.section_entries_mut("DECK").clone();
    assert_eq!(deck_entries[0].name, "Bolt");
    assert_eq!(deck_entries[0].quantity, 6);
    assert_eq!(deck_entries[0].set_code, None, "first line had no print");
    assert_eq!(out.to_text(), "// DECK\n6 Bolt\n\n// SIDEBOARD\n2 Bolt\n");
}

#[test]
fn dedupe_collapses_over_singleton_quantities_in_commander_decks() {
    // A commander deck holds one copy per non-basic card: duplicate
    // lines merge AND over-limit single lines collapse to 1.
    let deck = Deck::parse(
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n2 Bolt (M11) 148\n3 Bolt\n// SIDEBOARD\n2 Bolt\n",
    )
    .unwrap();
    let (out, merged, cards) = dedupe_deck(&deck);
    // 2 duplicate lines merged + 1 collapsed excess in DECK + 1 in SIDEBOARD.
    assert_eq!(merged, 4);
    assert_eq!(out.total(), 3); // 1 Breya + 1 Bolt + 1 sideboard Bolt
    assert_eq!(
        out.to_text(),
        "// COMMANDER\n1 Breya\n\n// DECK\n1 Bolt\n\n// SIDEBOARD\n1 Bolt\n"
    );
    assert!(cards.contains(&("Bolt".to_string(), 2)));
    assert!(cards.contains(&("Bolt".to_string(), 3)));
    assert!(cards.contains(&("Bolt".to_string(), 1)));
    assert!(cards.contains(&("Bolt".to_string(), 1)));
}

#[test]
fn dedupe_keeps_basics_and_60_card_decks_at_summed_quantities() {
    // Basics are unlimited and a 60-card-style deck (no COMMANDER
    // section) may hold 4-ofs; neither collapses.
    let commander = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n20 Island\n").unwrap();
    let (out, merged, _) = dedupe_deck(&commander);
    assert_eq!(merged, 0);
    assert_eq!(out.total(), 21);
    let flat = Deck::parse("// DECK\n4 Bolt\n").unwrap();
    let (out, merged, _) = dedupe_deck(&flat);
    assert_eq!(merged, 0);
    assert_eq!(out.total(), 4);
}

#[test]
fn dedupe_keeps_distinct_prints_when_names_differ() {
    // Different names never merge, even with identical other fields.
    let deck = Deck::parse("// DECK\n1 Bolt\n1 Shock\n").unwrap();
    let (out, merged, _) = dedupe_deck(&deck);
    assert_eq!(merged, 0);
    assert_eq!(out.total(), 2);
}

#[test]
fn reject_singleton_adds_blocks_over_limit_adds() {
    // Commander deck: adding a second copy of an existing card rejects.
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n").unwrap();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 Bolt").unwrap()];
    assert_eq!(
        reject_singleton_adds(&mut out, &deck, &ops),
        Some(crate::cli::codes::NO_RESULTS)
    );
    // A 60-card deck never rejects.
    let flat = Deck::parse("// DECK\n1 Bolt\n").unwrap();
    let mut out = silent_out();
    assert_eq!(reject_singleton_adds(&mut out, &flat, &ops), None);
    // Basics are exempt.
    let commander = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Island\n").unwrap();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "5 Island").unwrap()];
    assert_eq!(reject_singleton_adds(&mut out, &commander, &ops), None);
    // Adding a new card (not in the deck) is fine.
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 Shock").unwrap()];
    assert_eq!(reject_singleton_adds(&mut out, &deck, &ops), None);
    // Copies held in another section count against the singleton total.
    let split =
        Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n// SIDEBOARD\n1 Shock\n").unwrap();
    let mut out = silent_out();
    let ops = vec![parse_op("add", "1 Shock").unwrap()];
    assert_eq!(
        reject_singleton_adds(&mut out, &split, &ops),
        Some(crate::cli::codes::NO_RESULTS)
    );
    // Set and Move ops are never rejected here.
    let mut out = silent_out();
    let ops = vec![parse_op("set", "2 Bolt").unwrap()];
    assert_eq!(reject_singleton_adds(&mut out, &deck, &ops), None);
}

#[test]
fn singleton_warnings_flag_only_commander_decks() {
    // No COMMANDER section: no warnings at all.
    let deck = Deck::parse("// DECK\n2 Bolt\n").unwrap();
    let ops = vec![parse_op("add", "2 Bolt").unwrap()];
    assert!(singleton_warnings(&ops, &deck).is_empty());
    // With a COMMANDER section, a post-apply hold of two warns.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n").unwrap();
    let ops = vec![parse_op("add", "2 Bolt").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(
        singleton_warnings(&ops, &deck),
        vec!["Bolt would exceed the singleton limit; commander decks hold one copy".to_string()]
    );
    // Basics are exempt.
    let ops = vec![parse_op("add", "20 Island").unwrap()];
    assert!(singleton_warnings(&ops, &deck).is_empty());
    // A set that lowers to 1 does not warn.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n4 Bolt\n").unwrap();
    let ops = vec![parse_op("set", "1 Bolt").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert!(singleton_warnings(&ops, &deck).is_empty());
    // A set that raises to 3 warns after apply.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n").unwrap();
    let ops = vec![parse_op("set", "3 Bolt").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(singleton_warnings(&ops, &deck).len(), 1);
}

#[test]
fn singleton_warnings_read_post_apply_deck_state() {
    // A first-copy add of an absent card must not warn: the deck now
    // holds one copy.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n").unwrap();
    let ops = vec![parse_op("add", "1 Secluded Courtyard").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert!(singleton_warnings(&ops, &deck).is_empty());
    // A multi-copy add still warns.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n").unwrap();
    let ops = vec![parse_op("add", "2 Secluded Courtyard").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(singleton_warnings(&ops, &deck).len(), 1);
    // Adding a second copy of a held card still warns.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Secluded Courtyard\n").unwrap();
    let ops = vec![parse_op("add", "1 Secluded Courtyard").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(singleton_warnings(&ops, &deck).len(), 1);
    // A move nets to one copy: no warning (the false-positive case
    // that motivated post-apply semantics).
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n// SIDEBOARD\n").unwrap();
    let ops = vec![parse_op("move", "1 Bolt to:sideboard").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert!(singleton_warnings(&ops, &deck).is_empty());
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
        singleton_warnings(&ops, &deck).is_empty(),
        "a legal move must not warn"
    );
    // A genuine double-copy hold still warns after apply.
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n").unwrap();
    let ops = vec![parse_op("add", "1 Bolt").unwrap()];
    let _ = apply_ops(&mut deck, &ops).unwrap();
    assert_eq!(singleton_warnings(&ops, &deck).len(), 1);
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
