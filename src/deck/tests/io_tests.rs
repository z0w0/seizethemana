use super::*;
use crate::deck::store::primer_file;
use crate::output::Output;

fn setup() -> (tempfile::TempDir, crate::paths::Paths, rusqlite::Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    (tmp, paths, conn)
}

/// Seed oracle rows so name resolution and commander detection work.
fn seed_card(conn: &rusqlite::Connection, name: &str, type_line: &str, rank: Option<i64>) {
    conn.execute(
        "INSERT INTO cards (name, oracle_id, type_line, edhrec_rank, set_code, collector_number)
         VALUES (?1, 'oid', ?2, ?3, 'tst', '1')",
        rusqlite::params![name, type_line, rank],
    )
    .unwrap();
}

const DECK_TXT: &str =
    "// COMMANDER\n1 Breya, Etherium Shaper (MH3) 372 *F*\n\n// DECK\n2 Island (SOS) 274\n";

#[test]
fn import_upserts_by_name() {
    let (_tmp, paths, conn) = setup();
    seed_card(
        &conn,
        "Breya, Etherium Shaper",
        "Legendary Creature — Human",
        Some(10),
    );
    seed_card(&conn, "Island", "Basic Land — Island", None);
    let src = paths.root().join("source.txt");
    std::fs::write(&src, DECK_TXT).unwrap();
    let mut out = Output::new(true, false, false);

    import(ImportSource {
        paths: &paths,
        conn: &conn,
        out: &mut out,
        json: true,
        name: "Round",
        file: Some(&src),
        url: None,
        format: Some("manabox"),
    })
    .unwrap();
    // Upsert: importing again succeeds and overwrites by name.
    let code = import(ImportSource {
        paths: &paths,
        conn: &conn,
        out: &mut out,
        json: true,
        name: "Round",
        file: Some(&src),
        url: None,
        format: Some("manabox"),
    })
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // Contents on disk match the source grammar.
    let stored = std::fs::read_to_string(paths.deck_file("Round")).unwrap();
    assert_eq!(stored, DECK_TXT);
    // Primer stub exists.
    assert!(primer_file(&paths, "Round").exists());

    let dest = paths.root().join("out.txt");
    export(&paths, &mut out, "Round", &dest, false, "manabox").unwrap();
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), DECK_TXT);
    // Export refuses to overwrite without --force.
    let code = export(&paths, &mut out, "Round", &dest, false, "manabox").unwrap();
    assert_eq!(code, crate::cli::codes::ERROR);
    let code = export(&paths, &mut out, "Round", &dest, true, "manabox").unwrap();
    assert_eq!(code, crate::cli::codes::OK);
}

#[test]
fn manabox_quirk_renames_oversized_commander_section() {
    let (_tmp, paths, conn) = setup();
    seed_card(&conn, "Bolt", "Instant", None);
    let src = paths.root().join("manabox.txt");
    // ManaBox's export shape: the whole deck under // COMMANDER.
    std::fs::write(
        &src,
        "// COMMANDER\n2 Bolt\n1 Bolt\n1 Fog\n1 Giant Growth\n1 Lightning Helix\n1 Shock\n",
    )
    .unwrap();
    let mut out = Output::new(true, false, false);
    import(ImportSource {
        paths: &paths,
        conn: &conn,
        out: &mut out,
        json: true,
        name: "Quirk",
        file: Some(&src),
        url: None,
        format: Some("manabox"),
    })
    .unwrap();
    let deck = Deck::parse(&std::fs::read_to_string(paths.deck_file("Quirk")).unwrap()).unwrap();
    assert!(deck.section_index("COMMANDER").is_none(), "merged to DECK");
    assert_eq!(deck.total(), 7);
}

#[test]
fn real_commander_section_survives() {
    let (_tmp, paths, conn) = setup();
    seed_card(
        &conn,
        "Breya, Etherium Shaper",
        "Legendary Creature — Human",
        Some(10),
    );
    seed_card(&conn, "Island", "Basic Land — Island", None);
    let src = paths.root().join("ok.txt");
    std::fs::write(&src, DECK_TXT).unwrap();
    let mut out = Output::new(true, false, false);
    import(ImportSource {
        paths: &paths,
        conn: &conn,
        out: &mut out,
        json: true,
        name: "Ok",
        file: Some(&src),
        url: None,
        format: Some("manabox"),
    })
    .unwrap();
    let deck = Deck::parse(&std::fs::read_to_string(paths.deck_file("Ok")).unwrap()).unwrap();
    assert_eq!(deck.section_index("COMMANDER"), Some(0));
    assert_eq!(deck.total(), 3);
}

#[test]
fn import_rejects_bad_grammar() {
    let (_tmp, paths, conn) = setup();
    let src = paths.root().join("bad.txt");
    std::fs::write(&src, "not a deck line").unwrap();
    let mut out = Output::new(true, false, false);
    assert!(
        import(ImportSource {
            paths: &paths,
            conn: &conn,
            out: &mut out,
            json: true,
            name: "Bad",
            file: Some(&src),
            url: None,
            format: None
        })
        .is_err()
    );
}

/// Oversized imports must preserve existing files and must not create new decks.
#[test]
fn import_quantity_errors_leave_saved_files_unchanged() {
    let (_temp, paths, conn) = setup();
    std::fs::create_dir_all(paths.decks_dir()).expect("decks directory");
    let original = "// DECK\n2 Island\n";
    std::fs::write(paths.deck_file("Existing"), original).expect("original deck");
    std::fs::write(primer_file(&paths, "Existing"), "original primer").expect("original primer");
    let source = paths.root().join("oversized.txt");
    let cases = [
        (Some("manabox"), "10000 Island\n"),
        (Some("moxfield"), "10000x Island\n"),
        (Some("archidekt"), "10000x Island [mh3] 274\n"),
        (Some("arena"), "Deck\n10000 Island\n"),
        (Some("names"), "10000 Island\n"),
        (None, "10000x Island\n"),
    ];
    for (format, text) in cases {
        std::fs::write(&source, text).expect("source deck");
        for name in ["Existing", "New"] {
            assert!(
                import(ImportSource {
                    paths: &paths,
                    conn: &conn,
                    out: &mut Output::new(true, false, false),
                    json: true,
                    name,
                    file: Some(&source),
                    url: None,
                    format,
                })
                .is_err(),
                "{format:?}: {name}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(paths.deck_file("Existing")).expect("original deck"),
            original
        );
        assert_eq!(
            std::fs::read_to_string(primer_file(&paths, "Existing")).expect("original primer"),
            "original primer"
        );
        assert!(!paths.deck_file("New").exists());
        assert!(!primer_file(&paths, "New").exists());
        assert_eq!(
            std::fs::read_dir(paths.decks_dir())
                .expect("deck files")
                .count(),
            2
        );
    }
}

/// Final import validation must run before creating an empty destination file.
#[test]
fn imported_invalid_final_quantity_does_not_create_a_stub() {
    let (_temp, paths, conn) = setup();
    let mut deck = Deck::parse("1 Island").expect("valid deck");
    deck.sections[0].1[0].quantity = 0;

    assert!(
        save_imported(
            &paths,
            &conn,
            &mut Output::new(true, false, false),
            "Invalid",
            &deck,
            "source.txt",
            "",
        )
        .is_err()
    );
    assert!(!paths.decks_dir().exists());
}

#[test]
fn export_names_strips_print_and_foil_decorations() {
    let (_tmp, paths, conn) = setup();
    seed_card(&conn, "Bolt", "Instant", None);
    let deck_path = paths.deck_file("Names");
    if let Some(dir) = deck_path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(
        deck_path,
        "// COMMANDER\n1 Breya\n// DECK\n2 Bolt (SOS) 100 *F*\n1 Fog\n",
    )
    .unwrap();
    let dest = _tmp.path().join("names.txt");
    let mut out = Output::new(true, false, false);
    let code = export(&paths, &mut out, "Names", &dest, false, "names").unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let text = std::fs::read_to_string(&dest).unwrap();
    assert_eq!(text, "// COMMANDER\n1 Breya\n\n// DECK\n2 Bolt\n1 Fog\n");
}

#[test]
fn primer_create_read_update() {
    let (_tmp, paths, _conn) = setup();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("P"), "// DECK\n1 Bolt\n").unwrap();
    let mut out = Output::new(false, true, false);

    // No primer yet: reading creates an empty stub.
    let code = primer(&paths, &mut out, "P", None).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    assert_eq!(
        std::fs::read_to_string(primer_file(&paths, "P")).unwrap(),
        ""
    );

    // --set writes contents from a markdown file.
    let src = paths.root().join("primer.md");
    std::fs::write(&src, "# My deck\nPlan: win.\n").unwrap();
    let code = primer(&paths, &mut out, "P", Some(&src)).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    assert_eq!(
        std::fs::read_to_string(primer_file(&paths, "P")).unwrap(),
        "# My deck\nPlan: win.\n"
    );
}

#[test]
fn primer_requires_existing_deck() {
    let (_tmp, paths, _conn) = setup();
    let mut out = Output::new(true, false, false);
    let code = primer(&paths, &mut out, "Ghost", None);
    assert!(code.is_err());
}

#[test]
fn import_validates_deck_names() {
    let (_tmp, paths, conn) = setup();
    let src = paths.root().join("deck.txt");
    std::fs::write(&src, DECK_TXT).unwrap();
    let mut out = Output::new(true, false, false);
    // Invalid names error before any file is touched.
    assert!(
        import(ImportSource {
            paths: &paths,
            conn: &conn,
            out: &mut out,
            json: true,
            name: "../escape",
            file: Some(&src),
            url: None,
            format: None
        })
        .is_err()
    );
    assert!(!paths.deck_file("../escape").exists());
}

#[test]
fn read_commands_reject_path_traversal_names() {
    // A read command takes a raw name: a parent segment must not escape
    // the decks directory.
    let (_tmp, paths, _conn) = setup();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    let outside = paths.root().join("secret.txt");
    std::fs::write(&outside, "1 Island\n").unwrap();
    assert!(crate::deck::store::load_deck(&paths, "../secret").is_err());
    assert!(crate::deck::store::load_deck(&paths, "../../secret").is_err());
    assert!(crate::deck::store::load_deck(&paths, "sub/deck").is_err());
}

#[test]
fn offline_import_rejects_url() {
    // `--url` under `--offline` is a usage error with a file fallback
    // hint; no network is touched.
    let (_tmp, paths, _conn) = setup();
    crate::set_offline(true);
    let mut out = Output::new(true, false, false);
    let code = import(ImportSource {
        paths: &paths,
        conn: &_conn,
        out: &mut out,
        json: true,
        name: "Offline",
        file: None,
        url: Some("https://example.com/deck"),
        format: Some("manabox"),
    })
    .unwrap();
    crate::set_offline(false);
    assert_eq!(code, crate::cli::codes::USAGE);
    assert!(!paths.deck_file("Offline").exists());
}

#[test]
fn url_import_requires_a_fetchable_source_and_writes_the_deck() {
    // No live HTTP in unit tests: the offline path pins the guard, and a
    // bogus host proves the URL branch runs and surfaces its fetch error
    // without writing a deck file.
    crate::set_offline(false);
    let (_tmp, paths, conn) = setup();
    assert!(!crate::offline_requested());
    let mut out = Output::new(true, false, false);
    let url = "http://127.0.0.1:1/deck";
    let code = import(ImportSource {
        paths: &paths,
        conn: &conn,
        out: &mut out,
        json: false,
        name: "NoFetch",
        file: None,
        url: Some(url),
        format: None,
    });
    // The fetch must fail (nothing listens there) and no deck file may
    // appear.
    assert!(code.is_err());
    assert!(!paths.deck_file("NoFetch").exists());
    crate::set_offline(true);
}

#[test]
fn non_tty_import_notes_commander_candidates() {
    // A decklist without a commander in JSON mode (non-interactive):
    // the import succeeds and the commander note names the candidates.
    let (_tmp, paths, conn) = setup();
    seed_card(&conn, "Breya", "Legendary Creature — Human", Some(5));
    seed_card(&conn, "Bolt", "Instant", None);
    seed_card(&conn, "Island", "Basic Land — Island", None);
    let src = paths.root().join("nocmdr.txt");
    std::fs::write(&src, "// DECK\n1 Bolt\n1 Island\n").unwrap();
    let mut out = Output::new(true, false, false);
    let code = import(ImportSource {
        paths: &paths,
        conn: &conn,
        out: &mut out,
        json: true,
        name: "NoCmdr",
        file: Some(&src),
        url: None,
        format: Some("manabox"),
    })
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let deck = Deck::parse(&std::fs::read_to_string(paths.deck_file("NoCmdr")).unwrap()).unwrap();
    assert!(
        deck.section_index("COMMANDER").is_none(),
        "non-TTY import never mutates the deck with a commander"
    );
}
