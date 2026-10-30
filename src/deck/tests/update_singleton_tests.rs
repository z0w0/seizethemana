use super::*;

#[test]
fn lowercase_add_cannot_bypass_commander_singleton_guard() {
    let (tmp, conn) = seeded_conn();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Commander List"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Lightning Bolt\n",
    )
    .unwrap();
    let mut out = silent_out();

    let code = update(
        &paths,
        &conn,
        &mut out,
        "Commander List",
        &["1 lightning bolt".to_string()],
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

    assert_eq!(code, crate::cli::codes::NO_RESULTS);
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Commander List")).unwrap(),
        "// COMMANDER\n1 Breya\n// DECK\n1 Lightning Bolt\n"
    );
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

#[test]
fn dedupe_keeps_basic_lanes_per_section() {
    // Basics keep summed quantities; sections dedupe independently
    // (the same name in DECK and SIDEBOARD stays two lines).
    let (tmp, conn) = seeded_conn();
    let deck =
        Deck::parse("// DECK\n8 Plains\n8 Plains\n1 Bolt\n// SIDEBOARD\n1 Bolt\n2 Fog\n2 Fog\n")
            .unwrap();
    let mut result = dedupe_deck(&conn, &deck).unwrap();
    assert_eq!(result.merged_lines, 2);
    let main = result.deck.section_entries_mut("DECK");
    assert_eq!(main.len(), 2, "Plains + Bolt");
    assert_eq!(main[0].quantity, 16, "basic copies sum");
    let side = result.deck.section_entries_mut("SIDEBOARD");
    assert_eq!(side.len(), 2);
    assert_eq!(side[0].quantity, 1, "Bolt lines do not cross sections");
    let _ = &tmp;
}
