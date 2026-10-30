//! Saved card names must resolve before matching or counting copies.

use super::*;

/// Build a saved deck with one oracle card and its printed alias.
fn saved_deck(text: &str) -> (tempfile::TempDir, crate::paths::Paths, rusqlite::Connection) {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Bolt Alias"), text).unwrap();
    let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES ('Lightning Bolt', 'bolt')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
            lang, rarity, finishes, released_at, flavor_name, updated_at)
         VALUES ('bolt-alias', 'Lightning Bolt', 'tst', '1', 'en',
            'common', '[\"nonfoil\"]', '2020-01-01', 'Bolt Alias', 'now')",
        [],
    )
    .unwrap();
    (temp, paths, conn)
}

/// Run ordered operations through the command's batch-file path.
fn run_specs(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    specs: &[(&str, &str)],
    dry_run: bool,
    allow_partial: bool,
) -> i32 {
    let file = paths.root().join("update.spec");
    let text = specs
        .iter()
        .map(|(kind, spec)| format!("{kind} {spec}\n"))
        .collect::<String>();
    std::fs::write(&file, text).unwrap();
    update(
        paths,
        conn,
        &mut crate::output::Output::new(true, false, false),
        "Bolt Alias",
        &[],
        &[],
        &[],
        &[],
        Some(&file),
        allow_partial,
        dry_run,
        false,
        false,
        false,
        false,
    )
    .unwrap()
}

/// Each operation must match an existing lowercase, prefix, or alias line.
#[test]
fn saved_spellings_match_all_operations_and_keep_print_targets_exact() {
    let cases = [
        (
            "remove",
            "1 Lightning Bolt",
            "1 Lightning Bolt (M11) 148 *F*\n4 Lightning Bolt (2XM) 124\n",
        ),
        ("set", "0 Lightning Bolt", "4 Lightning Bolt (2XM) 124\n"),
        (
            "set",
            "3 Lightning Bolt",
            "3 Lightning Bolt (M11) 148 *F*\n4 Lightning Bolt (2XM) 124\n",
        ),
        (
            "add",
            "1 Lightning Bolt",
            "3 Lightning Bolt (M11) 148 *F*\n4 Lightning Bolt (2XM) 124\n",
        ),
        (
            "move",
            "1 Lightning Bolt to:sideboard",
            "1 Lightning Bolt (M11) 148 *F*\n4 Lightning Bolt (2XM) 124\n\n// sideboard\n1 Lightning Bolt (M11) 148 *F*\n",
        ),
        (
            "remove",
            "1 Lightning Bolt (2XM) 124",
            "2 Lightning Bolt (M11) 148 *F*\n3 Lightning Bolt (2XM) 124\n",
        ),
        (
            "remove",
            "1 Lightning Bolt (M11) 148 *F*",
            "1 Lightning Bolt (M11) 148 *F*\n4 Lightning Bolt (2XM) 124\n",
        ),
        (
            "set",
            "0 Lightning Bolt (M11) 148 *F*",
            "4 Lightning Bolt (2XM) 124\n",
        ),
    ];
    for saved in ["lightning bolt", "Lightning B", "Bolt Alias"] {
        let original = format!("// DECK\n2 {saved} (M11) 148 *F*\n4 Lightning Bolt (2XM) 124\n");
        let (_temp, paths, conn) = saved_deck(&original);
        for (kind, spec, expected) in cases {
            std::fs::write(paths.deck_file("Bolt Alias"), &original).unwrap();
            assert_eq!(run_specs(&paths, &conn, &[(kind, spec)], false, false), 0);
            let actual = std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap();
            assert_eq!(
                actual,
                format!("// DECK\n{expected}"),
                "{saved}: {kind} {spec}"
            );
        }
    }
}

/// A single saved alias must remain removable from a commander deck.
#[test]
fn saved_commander_lines_match_remove_and_zero_set() {
    for saved in ["lightning bolt", "Lightning B", "Bolt Alias"] {
        let original = format!("// COMMANDER\n1 Breya\n// DECK\n1 {saved}\n");
        let (_temp, paths, conn) = saved_deck(&original);
        for (kind, spec) in [("remove", "1 lightning bolt"), ("set", "0 lightning bolt")] {
            std::fs::write(paths.deck_file("Bolt Alias"), &original).unwrap();
            assert_eq!(run_specs(&paths, &conn, &[(kind, spec)], false, false), 0);
            assert_eq!(
                std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
                "// COMMANDER\n1 Breya\n\n// DECK\n"
            );
        }
    }
}

/// The singleton guard must count saved aliases after all batch operations.
#[test]
fn saved_alias_singleton_counts_use_the_post_batch_state() {
    for (saved, expected) in [
        ("1 lightning bolt", 0),
        ("1 lightning bolt\n1 Bolt Alias", 3),
    ] {
        let original = format!("// COMMANDER\n1 Breya\n// DECK\n{saved}\n");
        let (_temp, paths, conn) = saved_deck(&original);
        assert_eq!(
            run_specs(
                &paths,
                &conn,
                &[("remove", "1 Bolt Alias"), ("add", "1 Lightning B")],
                false,
                false,
            ),
            expected
        );
        let expected_text = if expected == 0 {
            "// COMMANDER\n1 Breya\n\n// DECK\n1 Lightning Bolt\n"
        } else {
            &original
        };
        assert_eq!(
            std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
            expected_text
        );
    }
}

/// Every resolved spelling must count toward the commander singleton limit.
#[test]
fn saved_spellings_and_mixed_alias_duplicates_cannot_bypass_singleton() {
    for saved in [
        "1 lightning bolt",
        "1 Lightning B",
        "1 Bolt Alias",
        "1 lightning bolt\n1 Bolt Alias",
    ] {
        let original = format!("// COMMANDER\n1 Breya\n// DECK\n{saved}\n");
        let (_temp, paths, conn) = saved_deck(&original);
        for added in [
            "1 Lightning Bolt",
            "1 lightning bolt",
            "1 Lightning B",
            "1 Bolt Alias",
        ] {
            for dry_run in [false, true] {
                assert_eq!(
                    run_specs(&paths, &conn, &[("add", added)], dry_run, false),
                    3
                );
                assert_eq!(
                    std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
                    original
                );
            }
        }
    }
}

/// Resolving aliases must not merge lines or discard any saved copies.
#[test]
fn mixed_alias_duplicates_keep_line_quantities_and_order() {
    let original = "// DECK\n2 lightning bolt\n3 Bolt Alias\n4 Lightning B\n";
    let (_temp, paths, conn) = saved_deck(original);
    for (specs, expected) in [
        (
            vec![("remove", "1 Lightning Bolt")],
            "// DECK\n1 Lightning Bolt\n3 Lightning Bolt\n4 Lightning Bolt\n",
        ),
        (
            vec![("set", "0 Lightning Bolt")],
            "// DECK\n3 Lightning Bolt\n4 Lightning Bolt\n",
        ),
        (
            vec![("add", "1 Lightning Bolt")],
            "// DECK\n3 Lightning Bolt\n3 Lightning Bolt\n4 Lightning Bolt\n",
        ),
    ] {
        std::fs::write(paths.deck_file("Bolt Alias"), original).unwrap();
        assert_eq!(run_specs(&paths, &conn, &specs, false, false), 0);
        assert_eq!(
            std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
            expected
        );
    }
}

/// Stale lines remain removable, but they cannot supply unknown new cards.
#[test]
fn stale_saved_lines_can_be_reduced_or_deleted_but_not_added() {
    let original = "// DECK\n3 Stale Unknown\n2 lightning bolt\n";
    let (_temp, paths, conn) = saved_deck(original);
    for (specs, expected) in [
        (
            vec![("remove", "1 Stale Unknown")],
            "// DECK\n2 Stale Unknown\n2 Lightning Bolt\n",
        ),
        (
            vec![("set", "0 Stale Unknown")],
            "// DECK\n2 Lightning Bolt\n",
        ),
        (
            vec![("move", "1 Stale Unknown to:sideboard")],
            "// DECK\n2 Stale Unknown\n2 Lightning Bolt\n\n// sideboard\n1 Stale Unknown\n",
        ),
    ] {
        std::fs::write(paths.deck_file("Bolt Alias"), original).unwrap();
        assert_eq!(run_specs(&paths, &conn, &specs, false, false), 0);
        assert_eq!(
            std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
            expected
        );
    }
    for kind in ["add", "set"] {
        std::fs::write(paths.deck_file("Bolt Alias"), original).unwrap();
        assert_eq!(
            run_specs(&paths, &conn, &[(kind, "4 Stale Unknown")], false, false),
            3
        );
        assert_eq!(
            std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
            original
        );
    }
}

/// Name resolution must not commit previews or failed strict batches.
#[test]
fn saved_name_resolution_keeps_batch_and_preview_writes_atomic() {
    let original = "// DECK\n2 lightning bolt\n";
    let (_temp, paths, conn) = saved_deck(original);
    for (specs, dry_run, expected) in [
        (vec![("remove", "1 Lightning Bolt")], true, 0),
        (vec![("set", "0 Lightning Bolt")], true, 0),
        (
            vec![("add", "1 Bolt Alias"), ("remove", "1 Missing")],
            false,
            3,
        ),
        (
            vec![
                ("remove", "1 Lightning Bolt"),
                ("add", "1 Unknown New Card"),
            ],
            false,
            3,
        ),
    ] {
        assert_eq!(run_specs(&paths, &conn, &specs, dry_run, false), expected);
        assert_eq!(
            std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
            original
        );
    }
    assert_eq!(
        run_specs(
            &paths,
            &conn,
            &[("remove", "1 Lightning Bolt"), ("remove", "1 Missing")],
            false,
            true
        ),
        1
    );
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
        "// DECK\n1 Lightning Bolt\n"
    );
}

/// Card resolution must leave deck names, labels, primers, and binders alone.
#[test]
fn saved_card_resolution_keeps_user_labels_and_primer_text() {
    let original = "// DECK\n2 lightning bolt\n// Bolt Alias\n3 Lightning B\n";
    let (_temp, paths, conn) = saved_deck(original);
    let primer = super::super::store::primer_file(&paths, "Bolt Alias");
    std::fs::write(&primer, "# Bolt Alias\nKeep lightning bolt as written.\n").unwrap();
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity)
         VALUES ('Bolt Alias', 'lightning bolt', 'binder', 7)",
        [],
    )
    .unwrap();
    assert_eq!(
        run_specs(
            &paths,
            &conn,
            &[("remove", "1 Lightning Bolt")],
            false,
            false
        ),
        0
    );
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
        "// DECK\n1 Lightning Bolt\n\n// Bolt Alias\n3 Lightning Bolt\n"
    );
    assert_eq!(
        std::fs::read_to_string(&primer).unwrap(),
        "# Bolt Alias\nKeep lightning bolt as written.\n"
    );
    assert!(!paths.deck_file("Lightning Bolt").exists());
    assert_eq!(std::fs::read_dir(paths.decks_dir()).unwrap().count(), 2);
    let binder: (String, String, i64) = conn
        .query_row("SELECT name, binder, quantity FROM collection", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(
        binder,
        ("Bolt Alias".to_string(), "lightning bolt".to_string(), 7)
    );
}

/// An ambiguous saved prefix must stay available for explicit removal.
#[test]
fn ambiguous_saved_lines_stay_removable() {
    let (_temp, paths, conn) = saved_deck("// DECK\n2 Lightning B\n");
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES ('Lightning Blast', 'blast')",
        [],
    )
    .unwrap();
    assert_eq!(
        run_specs(&paths, &conn, &[("remove", "1 Lightning B")], false, false),
        0
    );
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
        "// DECK\n1 Lightning B\n"
    );
    assert_eq!(
        run_specs(&paths, &conn, &[("add", "1 Lightning B")], false, false),
        3
    );
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
        "// DECK\n1 Lightning B\n"
    );
    assert_eq!(
        run_specs(&paths, &conn, &[("set", "0 Lightning B")], false, false),
        0
    );
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Bolt Alias")).unwrap(),
        "// DECK\n"
    );
}
