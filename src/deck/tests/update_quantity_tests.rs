use super::*;

/// Save a basic-land deck and seed its name for command validation.
fn quantity_deck(text: &str) -> (tempfile::TempDir, crate::paths::Paths, rusqlite::Connection) {
    let temp = tempfile::tempdir().expect("temporary data directory");
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).expect("decks directory");
    std::fs::write(paths.deck_file("Quantity"), text).expect("original deck");
    let conn = crate::db::open(std::path::Path::new(":memory:")).expect("card database");
    conn.execute(
        "INSERT INTO cards (name, oracle_id, type_line) VALUES ('Island', 'island', 'Basic Land')",
        [],
    )
    .expect("basic land");
    (temp, paths, conn)
}

/// Run ordered operations through the same file path as a CLI update.
fn update_quantities(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    specs: &str,
    dry_run: bool,
) -> anyhow::Result<i32> {
    let source = paths.root().join("quantity.spec");
    std::fs::write(&source, specs).expect("operation file");
    update(
        paths,
        conn,
        &mut crate::output::Output::new(true, false, false),
        "Quantity",
        &[],
        &[],
        &[],
        &[],
        Some(&source),
        false,
        dry_run,
        false,
        false,
        false,
        true,
    )
}

/// Adds and moves must not save or preview an unreadable final quantity.
#[test]
fn rejects_oversized_update_results_without_changing_the_file() {
    let cases = [
        ("// DECK\n9000 Island\n", "add 1000 Island\n"),
        (
            "// DECK\n1000 Island\n\n// SIDEBOARD\n9000 Island\n",
            "move 1000 Island to:sideboard\n",
        ),
    ];
    for (original, specs) in cases {
        for dry_run in [false, true] {
            let (_temp, paths, conn) = quantity_deck(original);
            let error = update_quantities(&paths, &conn, specs, dry_run)
                .expect_err("oversized final quantity");
            assert!(format!("{error:#}").contains("9999"));
            assert_eq!(
                std::fs::read_to_string(paths.deck_file("Quantity")).expect("unchanged deck"),
                original
            );
            assert_eq!(
                std::fs::read_dir(paths.decks_dir())
                    .expect("deck files")
                    .count(),
                1
            );
        }
    }
}

/// Validation must read the final batch result, not an intermediate quantity.
#[test]
fn accepts_batches_that_finish_with_valid_quantities() {
    for specs in [
        "add 1000 Island\nremove 1000 Island\n",
        "add 1000 Island\nset 9999 Island\n",
    ] {
        let (_temp, paths, conn) = quantity_deck("// DECK\n9000 Island\n");
        assert_eq!(
            update_quantities(&paths, &conn, specs, false).expect("valid final quantity"),
            crate::cli::codes::OK
        );
        let (_, deck) = load_deck(&paths, "Quantity").expect("readable saved deck");
        let expected = if specs.contains("set 9999") {
            9999
        } else {
            9000
        };
        assert_eq!(deck.total(), expected);
    }
}

/// A zero Set deletes the entry instead of leaving an invalid final entry.
#[test]
fn zero_set_still_deletes_entries() {
    for dry_run in [false, true] {
        let original = "// DECK\n9000 Island\n";
        let (_temp, paths, conn) = quantity_deck(original);
        assert_eq!(
            update_quantities(&paths, &conn, "set 0 Island\n", dry_run)
                .expect("zero Set remains valid"),
            crate::cli::codes::OK
        );
        let text = std::fs::read_to_string(paths.deck_file("Quantity")).expect("saved deck");
        let deck = Deck::parse(&text).expect("readable saved deck");
        if dry_run {
            assert_eq!(text, original);
        } else {
            assert_eq!(deck.total(), 0);
            assert_eq!(deck.entries().count(), 0);
        }
    }
}
