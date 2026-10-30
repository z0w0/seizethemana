use super::*;

/// A setup store with one held combo piece and a saved deck.
fn combo_store() -> (tempfile::TempDir, crate::paths::Paths, Connection) {
    let temp = tempfile::tempdir().expect("create suggestion store");
    let paths = crate::paths::Paths::new(temp.path().to_path_buf());
    paths.ensure_dirs().expect("create store directories");
    std::fs::create_dir_all(paths.decks_dir()).expect("create decks directory");
    std::fs::write(paths.deck_file("Deck"), "// DECK\n1 Held Piece\n").expect("write deck");
    let conn = crate::db::open(&paths.db()).expect("open card database");
    conn.execute(
        "INSERT INTO cards (name, oracle_id, cmc, legalities)
         VALUES ('Held Piece', 'held', 1.0, '{\"commander\":\"legal\",\"modern\":\"legal\"}')",
        [],
    )
    .expect("insert held piece");
    let mut status = crate::paths::Status::empty();
    status.setup_complete = true;
    status
        .write(&paths.status_file())
        .expect("write setup status");
    (temp, paths, conn)
}

/// Add a valid card that completes one variant with the deck's held piece.
fn insert_completion(conn: &Connection, name: &str, popularity: i64) {
    conn.execute(
        "INSERT INTO cards (name, oracle_id, cmc, legalities)
         VALUES (?1, ?1, 2.0, '{\"commander\":\"legal\",\"modern\":\"legal\"}')",
        [name],
    )
    .expect("insert completion card");
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, legalities, popularity, updated_at)
         VALUES (?1, '[]', 2, '{\"commander\":true,\"modern\":true}', ?2, 'fixture')",
        rusqlite::params![name, popularity],
    )
    .expect("insert combo variant");
    conn.execute(
        "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
         VALUES (?1, 'Held Piece', 0, '[\"H\"]', 0),
                (?1, ?1, 1, '[\"H\"]', 0)",
        [name],
    )
    .expect("insert combo pieces");
}

/// Use the real suggestion entry point without a query, role, or commander search.
fn run_default(
    paths: &crate::paths::Paths,
    conn: &mut Connection,
    owned_only: bool,
    exclude: &[String],
) -> anyhow::Result<i32> {
    super::super::suggest::suggest(
        paths,
        conn,
        &mut crate::output::Output::new(true, false, false),
        "Deck",
        None,
        None,
        false,
        None,
        None,
        None,
        1,
        owned_only,
        exclude,
        true,
    )
}

/// The default command must not suggest an unowned piece under `--owned`.
#[test]
fn default_combo_path_respects_owned_only() {
    let (_temp, paths, mut conn) = combo_store();
    insert_completion(&conn, "Unowned Piece", 1);

    assert_eq!(
        run_default(&paths, &mut conn, false, &[]).expect("unrestricted suggestions"),
        crate::cli::codes::OK
    );
    assert_eq!(
        run_default(&paths, &mut conn, true, &[]).expect("owned suggestions"),
        crate::cli::codes::NO_RESULTS
    );
}

/// Owned cards below the global limit survive, using the normal ownership scope.
#[test]
fn owned_default_combo_path_filters_before_the_limit() {
    let (_temp, paths, mut conn) = combo_store();
    for index in 0..80 {
        insert_completion(&conn, &format!("Unowned Piece {index:03}"), 100 + index);
    }
    insert_completion(&conn, "Owned Piece", 1);
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity)
         VALUES ('Owned Piece', 'Cards', 'binder', 1)",
        [],
    )
    .expect("insert owned piece");
    let excluded_owned = ["Owned Piece".to_string()];

    for binder_type in ["binder", "deck"] {
        conn.execute("UPDATE collection SET binder_type = ?1", [binder_type])
            .expect("set ownership scope");
        assert_eq!(
            run_default(&paths, &mut conn, false, &excluded_owned)
                .expect("unrestricted suggestions without the owned piece"),
            crate::cli::codes::OK,
            "unowned candidates remain eligible without --owned"
        );
        assert_eq!(
            run_default(&paths, &mut conn, true, &[]).expect("owned suggestions below the limit"),
            crate::cli::codes::OK,
            "the owned {binder_type} card ranks below all 80 unowned candidates"
        );
        assert_eq!(
            run_default(&paths, &mut conn, true, &excluded_owned)
                .expect("owned suggestions with the owned piece excluded"),
            crate::cli::codes::NO_RESULTS,
            "unowned candidates must not replace the excluded {binder_type} card"
        );
    }
}

/// A missing combo table is a database failure, not an empty combo store.
#[test]
fn default_combo_path_propagates_missing_combo_table() {
    let (_temp, paths, mut conn) = combo_store();
    conn.execute("DROP TABLE combos", [])
        .expect("remove combo table");

    let error = run_default(&paths, &mut conn, false, &[]).expect_err("missing table must fail");

    assert_eq!(error.to_string(), "counting combo variants");
    assert!(format!("{error:#}").contains("no such table: combos"));
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
}

/// Invalid candidate metadata must fail the command instead of hiding the card.
#[test]
fn default_combo_path_propagates_completion_conversion_errors() {
    let (_temp, paths, mut conn) = combo_store();
    insert_completion(&conn, "Broken Piece", 1);
    conn.execute(
        "UPDATE cards SET cmc = 'invalid' WHERE name = 'Broken Piece'",
        [],
    )
    .expect("corrupt completion metadata");

    let error = run_default(&paths, &mut conn, false, &[]).expect_err("invalid CMC must fail");

    assert_eq!(
        error.to_string(),
        "reading combo completion card \"Broken Piece\""
    );
    assert!(matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::InvalidColumnType(_, column, rusqlite::types::Type::Text))
            if column == "cmc"
    ));
}

/// A genuinely missing card row still skips that completion without an error.
#[test]
fn default_combo_path_skips_missing_completion_metadata() {
    let (_temp, paths, mut conn) = combo_store();
    insert_completion(&conn, "Missing Piece", 1);
    conn.execute("DELETE FROM cards WHERE name = 'Missing Piece'", [])
        .expect("remove completion metadata");

    assert_eq!(
        run_default(&paths, &mut conn, false, &[]).expect("skip missing completion"),
        crate::cli::codes::NO_RESULTS
    );
}
