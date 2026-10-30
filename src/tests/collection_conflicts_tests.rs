use super::*;

/// Temporary deck files and collection data for conflict reports.
fn fixture() -> (tempfile::TempDir, crate::paths::Paths, Connection) {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).expect("create decks directory");
    let conn = crate::db::open(std::path::Path::new(":memory:")).expect("open database");
    (temp, paths, conn)
}

/// Register ownership without requiring a valid decklist.
fn register_deck(conn: &Connection, name: &str) {
    conn.execute(
        "INSERT INTO collection
            (name, set_code, collector_number, foil, binder, binder_type, quantity)
         VALUES ('Sol Ring', 'cmm', '396', 'normal', ?1, 'deck', 1)",
        [name],
    )
    .expect("register deck");
}

/// A malformed registered deck stays unverified even though its file exists.
#[test]
fn conflict_json_includes_malformed_and_missing_decks() {
    let (_temp, paths, conn) = fixture();
    std::fs::write(paths.deck_file("Good"), "// DECK\n1 Sol Ring\n").expect("write valid deck");
    std::fs::write(
        paths.deck_file("Broken"),
        "// DECK\n1 Sol Ring\nnot a deck entry\n",
    )
    .expect("write malformed deck");
    for name in ["Good", "Broken", "Missing"] {
        register_deck(&conn, name);
    }

    let report = conflict_report(&paths, &conn).expect("build conflict report");
    let json = serde_json::to_value(report).expect("serialize conflict report");

    assert_eq!(json["rows"], serde_json::json!([]));
    assert_eq!(
        json["unverified_decks"],
        serde_json::json!(["Broken", "Missing"])
    );
    assert!(paths.deck_file("Broken").exists());
}

/// A path that cannot be read as a deck file must not produce a clean report.
#[test]
fn conflict_json_includes_deck_path_read_failures() {
    let (_temp, paths, conn) = fixture();
    std::fs::create_dir(paths.deck_file("Unreadable")).expect("create unreadable deck path");
    register_deck(&conn, "Unreadable");

    let report = conflict_report(&paths, &conn).expect("build conflict report");
    let json = serde_json::to_value(report).expect("serialize conflict report");

    assert_eq!(json["rows"], serde_json::json!([]));
    assert_eq!(json["unverified_decks"], serde_json::json!(["Unreadable"]));
    assert!(paths.deck_file("Unreadable").exists());
}

/// Failed files stay unverified even when no collection row registers the deck.
#[test]
fn conflict_json_includes_unregistered_failed_decks() {
    let (_temp, paths, conn) = fixture();
    std::fs::write(paths.deck_file("Broken"), "not a deck entry").expect("write malformed deck");
    std::fs::create_dir(paths.deck_file("Unreadable")).expect("create unreadable deck path");

    let report = conflict_report(&paths, &conn).expect("build conflict report");
    let json = serde_json::to_value(report).expect("serialize conflict report");

    assert_eq!(json["rows"], serde_json::json!([]));
    assert_eq!(
        json["unverified_decks"],
        serde_json::json!(["Broken", "Unreadable"])
    );
}

/// Per-card demand skips failed files and contains no synthetic error cards.
#[test]
fn demanded_by_keeps_skipping_failed_deck_files() {
    let (_temp, paths, _conn) = fixture();
    std::fs::write(paths.deck_file("Good"), "// DECK\n2 Sol Ring\n").expect("write valid deck");
    std::fs::write(
        paths.deck_file("Broken"),
        "// DECK\n9 Sol Ring\nnot a deck entry\n",
    )
    .expect("write malformed deck");
    std::fs::create_dir(paths.deck_file("Unreadable")).expect("create unreadable deck path");

    let demand = deck_demand(&paths).expect("read checked deck demand");

    assert_eq!(demand.len(), 1);
    assert_eq!(demand["Sol Ring"], vec![("Good".to_string(), 2)]);
    assert_eq!(
        demanded_by(&paths, "Sol Ring").expect("read per-card demand"),
        vec![("Good".to_string(), 2)]
    );
    assert!(
        demanded_by(&paths, "Missing Card")
            .expect("look up absent card demand")
            .is_empty()
    );
}
