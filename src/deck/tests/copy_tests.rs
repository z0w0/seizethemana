use super::*;

fn setup() -> (tempfile::TempDir, crate::paths::Paths, rusqlite::Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    (tmp, paths, conn)
}

#[test]
fn copy_duplicates_decklist_and_primer() {
    let (_tmp, paths, _conn) = setup();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Orig"), "// DECK\n1 Bolt\n").unwrap();
    std::fs::write(primer_file(&paths, "Orig"), "# Orig\nWin fast.\n").unwrap();
    let mut out = crate::output::Output::new(true, false, false);

    let code = copy(&paths, &mut out, "Orig", "Improved", false).unwrap();

    assert_eq!(code, crate::cli::codes::OK);
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Improved")).unwrap(),
        "// DECK\n1 Bolt\n"
    );
    assert_eq!(
        std::fs::read_to_string(primer_file(&paths, "Improved")).unwrap(),
        "# Orig\nWin fast.\n"
    );
    // The original is untouched.
    assert!(paths.deck_file("Orig").exists());
}

#[test]
fn copy_refuses_existing_destination_without_force() {
    let (_tmp, paths, _conn) = setup();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Orig"), "// DECK\n1 Bolt\n").unwrap();
    std::fs::write(paths.deck_file("Dest"), "// DECK\n1 Fog\n").unwrap();
    let mut out = crate::output::Output::new(true, false, false);

    let code = copy(&paths, &mut out, "Orig", "Dest", false).unwrap();
    assert_eq!(code, crate::cli::codes::ERROR);
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Dest")).unwrap(),
        "// DECK\n1 Fog\n"
    );

    let code = copy(&paths, &mut out, "Orig", "Dest", true).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Dest")).unwrap(),
        "// DECK\n1 Bolt\n"
    );
}

#[test]
fn copy_missing_source_errors() {
    let (_tmp, paths, _conn) = setup();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    assert!(copy(&paths, &mut out, "Ghost", "New", false).is_err());
}
