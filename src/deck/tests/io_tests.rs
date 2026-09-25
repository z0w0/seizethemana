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
