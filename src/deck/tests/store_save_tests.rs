use super::*;

/// Build paths without creating the decks directory.
fn save_paths() -> (tempfile::TempDir, crate::paths::Paths) {
    let temp = tempfile::tempdir().expect("temporary data directory");
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    (temp, paths)
}

/// The largest allowed entry must remain readable after saving.
#[test]
fn saves_maximum_entry_quantity() {
    let (_temp, paths) = save_paths();
    let deck = Deck::parse("// DECK\n9999 Island\n").expect("valid deck");

    save_deck(&paths, "Saved", &deck).expect("save deck");

    let (_, saved) = load_deck(&paths, "Saved").expect("read saved deck");
    assert_eq!(saved, deck);
    assert_eq!(
        std::fs::read_dir(paths.decks_dir())
            .expect("deck files")
            .count(),
        1
    );
}

/// Temporary file names must not make valid deck names exceed the file name limit.
#[test]
fn saves_decks_with_long_names() {
    let (_temp, paths) = save_paths();
    let deck = Deck::parse("1 Island").expect("valid deck");
    let name = "A".repeat(240);

    save_deck(&paths, &name, &deck).expect("save long deck name");

    assert_eq!(load_deck(&paths, &name).expect("saved deck").1, deck);
}

/// Invalid final entries must not alter an existing deck or leave temporary files.
#[test]
fn rejects_invalid_final_quantities_without_changing_the_file() {
    let (_temp, paths) = save_paths();
    let original = "// DECK\n2 Island\n";
    let mut deck = Deck::parse(original).expect("valid deck");
    std::fs::create_dir_all(paths.decks_dir()).expect("decks directory");
    std::fs::write(paths.deck_file("Saved"), original).expect("original deck");

    for quantity in [i64::MIN, -1, 0, 10_000, i64::MAX] {
        deck.sections[0].1[0].quantity = quantity;
        assert!(save_deck(&paths, "Saved", &deck).is_err(), "{quantity}");
        assert_eq!(
            std::fs::read_to_string(paths.deck_file("Saved")).expect("unchanged deck"),
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

/// Quantity errors must not create a deck or its parent directory.
#[test]
fn invalid_save_does_not_create_files() {
    let (_temp, paths) = save_paths();
    let mut deck = Deck::parse("1 Island").expect("valid deck");
    deck.sections[0].1[0].quantity = 10_000;

    assert!(save_deck(&paths, "Saved", &deck).is_err());
    assert!(!paths.decks_dir().exists());
}

/// Saving must not overwrite the old shared temporary path.
#[test]
fn leaves_existing_temporary_files_untouched() {
    let (_temp, paths) = save_paths();
    let deck = Deck::parse("// DECK\n2 Island\n").expect("valid deck");
    std::fs::create_dir_all(paths.decks_dir()).expect("decks directory");
    let legacy_temp = paths.deck_file("Saved").with_extension("txt.tmp");
    std::fs::write(&legacy_temp, "another writer").expect("existing temporary file");

    save_deck(&paths, "Saved", &deck).expect("save deck");

    assert_eq!(
        std::fs::read_to_string(legacy_temp).expect("existing temporary file"),
        "another writer"
    );
    assert_eq!(load_deck(&paths, "Saved").expect("saved deck").1, deck);
    assert_eq!(
        std::fs::read_dir(paths.decks_dir())
            .expect("deck files")
            .count(),
        2
    );
}

/// A failed rename must remove only this save's temporary file.
#[test]
fn removes_temporary_file_after_failed_rename() {
    let (_temp, paths) = save_paths();
    let deck = Deck::parse("1 Island").expect("valid deck");
    let destination = paths.deck_file("Saved");
    std::fs::create_dir_all(&destination).expect("blocked destination");
    let marker = destination.join("keep");
    std::fs::write(&marker, "unchanged").expect("destination contents");

    assert!(save_deck(&paths, "Saved", &deck).is_err());

    assert_eq!(
        std::fs::read_to_string(marker).expect("marker"),
        "unchanged"
    );
    assert_eq!(
        std::fs::read_dir(paths.decks_dir())
            .expect("deck files")
            .count(),
        1
    );
}

/// Concurrent saves must each publish a complete deck without sharing temporary files.
#[test]
fn concurrent_saves_keep_files_complete() {
    let (_temp, paths) = save_paths();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        let mut writers = Vec::new();
        for writer in 0..8 {
            let paths = &paths;
            let barrier = barrier.clone();
            writers.push(scope.spawn(move || {
                let deck = Deck::parse(&format!("// DECK\n{} Island\n", 9000 + writer))
                    .expect("valid deck");
                barrier.wait();
                for _ in 0..8 {
                    save_deck(paths, "Shared", &deck).expect("independent atomic save");
                    let (_, saved) = load_deck(paths, "Shared").expect("complete saved deck");
                    assert!((9000..9008).contains(&saved.total()));
                    assert_eq!(saved.entries().count(), 1);
                }
            }));
        }
        for writer in writers {
            writer.join().expect("writer completed");
        }
    });
    assert_eq!(
        std::fs::read_dir(paths.decks_dir())
            .expect("deck files")
            .count(),
        1
    );
}
