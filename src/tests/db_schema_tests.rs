use super::*;

/// Check the stored columns in their schema order.
fn assert_columns(conn: &Connection, table: &str, expected: &[&str]) {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
        .expect("prepare columns");
    let columns: Vec<String> = stmt
        .query_map([table], |row| row.get(0))
        .expect("query columns")
        .collect::<Result<_, _>>()
        .expect("read columns");
    assert_eq!(columns, expected, "columns in {table}");
}

/// Check the complete current table set without full-text search shadow tables.
fn assert_current_tables(conn: &Connection) {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'cards_fts_%' ORDER BY name",
        )
        .expect("prepare tables");
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("query tables")
        .collect::<Result<_, _>>()
        .expect("read tables");
    assert_eq!(
        tables,
        [
            "card_prints",
            "card_tags",
            "cards",
            "cards_fts",
            "collection",
            "combo_pieces",
            "combos",
            "embedding_revisions",
            "pending_embeddings",
            "sets",
            "tags",
            "token_names",
        ]
    );
}

/// Check the metadata and durable work columns used by card sync.
fn assert_current_columns(conn: &Connection) {
    assert_columns(
        conn,
        "cards",
        &[
            "id",
            "name",
            "oracle_id",
            "mana_cost",
            "cmc",
            "type_line",
            "colors",
            "color_identity",
            "keywords",
            "power",
            "toughness",
            "loyalty",
            "oracle_text",
            "rarity",
            "edhrec_rank",
            "legalities",
            "set_code",
            "collector_number",
            "scryfall_id",
            "released_at",
            "game_changer",
            "tags_text",
            "reserved",
            "penny_rank",
        ],
    );
    assert_columns(
        conn,
        "sets",
        &["set_code", "set_name", "set_type", "block", "franchise"],
    );
    assert_columns(
        conn,
        "card_prints",
        &[
            "scryfall_id",
            "name",
            "flavor_name",
            "set_code",
            "collector_number",
            "lang",
            "rarity",
            "finishes",
            "released_at",
            "usd",
            "usd_foil",
            "usd_etched",
            "updated_at",
            "universes_beyond",
        ],
    );
    for table in ["pending_embeddings", "embedding_revisions"] {
        assert_columns(conn, table, &["name", "revision"]);
    }
}

/// Check the version, tables, columns, indexes, and triggers of a fresh store.
fn assert_current_schema(conn: &Connection) {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 1);
    assert_current_tables(conn);
    assert_current_columns(conn);
    let expected = Connection::open_in_memory().expect("reference database");
    expected
        .execute_batch(INITIAL_SCHEMA)
        .expect("reference schema");
    assert_eq!(
        schema_definitions(conn).expect("stored schema"),
        schema_definitions(&expected).expect("reference schema definitions")
    );
}

/// In-memory databases use the same version-1 schema as persistent stores.
#[test]
fn fresh_in_memory_database_has_current_schema() {
    let conn = open(std::path::Path::new(":memory:")).expect("open in memory");
    assert_current_schema(&conn);
}

/// Reopening a file keeps collection records and durable embedding work.
#[test]
fn fresh_file_database_keeps_current_schema_and_rows_on_reopen() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let path = temp.path().join("stm.db");
    let conn = open(&path).expect("create database");
    assert_current_schema(&conn);
    conn.execute_batch(
        "INSERT INTO collection (name, binder, binder_type, quantity)
         VALUES ('Island', 'Collection', 'binder', 11);
         INSERT INTO pending_embeddings (name, revision) VALUES ('Island', 'pending');
         INSERT INTO embedding_revisions (name, revision) VALUES ('Island', 'published');",
    )
    .expect("seed stored rows");
    drop(conn);

    let conn = open(&path).expect("reopen database");
    assert_current_schema(&conn);
    let rows: (i64, String, String) = conn
        .query_row(
            "SELECT c.quantity, p.revision, e.revision FROM collection c
             JOIN pending_embeddings p ON p.name = c.name
             JOIN embedding_revisions e ON e.name = c.name",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read stored rows");
    assert_eq!(rows, (11, "pending".to_string(), "published".to_string()));
}

/// A version number cannot hide missing tables, columns, indexes, or triggers.
#[test]
fn open_rejects_incomplete_version_one_schema_without_repairing_it() {
    for mutation in [
        "DROP TABLE pending_embeddings",
        "ALTER TABLE cards DROP COLUMN reserved",
        "ALTER TABLE sets DROP COLUMN franchise",
        "ALTER TABLE card_prints DROP COLUMN universes_beyond",
        "DROP INDEX idx_cards_type",
        "DROP TRIGGER cards_fts_ai",
    ] {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("stm.db");
        let conn = open(&path).expect("create database");
        conn.execute_batch(mutation).expect("change schema");
        let before = schema_definitions(&conn).expect("schema before open");
        drop(conn);

        let error = open(&path).expect_err("incompatible schema must fail");
        assert!(
            format!("{error:#}").contains("database schema does not match version 1"),
            "{mutation}: {error:#}"
        );
        let conn = Connection::open(&path).expect("inspect database");
        assert_eq!(
            schema_definitions(&conn).expect("schema after open"),
            before
        );
    }
}

/// Extra write constraints must not pass as the current schema or change rows.
#[test]
fn open_rejects_extra_write_constraints_without_changing_schema_or_rows() {
    for mutation in [
        "CREATE UNIQUE INDEX unexpected_oracle_key ON cards(oracle_id)",
        "CREATE TRIGGER unexpected_card_insert AFTER INSERT ON cards
         BEGIN UPDATE collection SET quantity = 0; END;",
        "CREATE UNIQUE INDEX sqlitex_unexpected_oracle_key ON cards(oracle_id)",
        "CREATE TRIGGER sqlitex_unexpected_card_insert AFTER INSERT ON cards
         BEGIN UPDATE collection SET quantity = 0; END;",
    ] {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("stm.db");
        let conn = open(&path).expect("create database");
        conn.execute_batch(
            "INSERT INTO cards (name, oracle_id) VALUES ('Island', 'island-oracle');
             INSERT INTO collection (name, binder, binder_type, quantity)
             VALUES ('Island', 'Collection', 'binder', 11);",
        )
        .expect("seed stored rows");
        conn.execute_batch(mutation).expect("add write constraint");
        let before = schema_definitions(&conn).expect("schema before open");
        drop(conn);

        let error = open(&path).expect_err("extra write constraint must fail");
        assert!(
            format!("{error:#}").contains("database schema does not match version 1"),
            "{mutation}: {error:#}"
        );
        let conn = Connection::open(&path).expect("inspect database");
        assert_eq!(
            schema_definitions(&conn).expect("schema after open"),
            before
        );
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("stored version");
        assert_eq!(version, 1);
        let rows: (String, i64, String) = conn
            .query_row(
                "SELECT c.name, c.quantity, cards.oracle_id FROM collection c
                 JOIN cards ON cards.name = c.name",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("stored rows");
        assert_eq!(
            rows,
            ("Island".to_string(), 11, "island-oracle".to_string())
        );
    }
}

/// Nonempty databases with an unknown version remain untouched.
#[test]
fn open_rejects_unsupported_schema_versions_without_changing_rows() {
    for version in [0, 2] {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("stm.db");
        let conn = open(&path).expect("create database");
        conn.execute(
            "INSERT INTO collection (name, binder, binder_type, quantity)
             VALUES ('Island', 'Collection', 'binder', 11)",
            [],
        )
        .expect("seed collection");
        conn.pragma_update(None, "user_version", version)
            .expect("change version");
        drop(conn);

        let error = open(&path).expect_err("unsupported version must fail");
        assert!(
            format!("{error:#}")
                .contains(&format!("unsupported database schema version {version}"))
        );
        let conn = Connection::open(&path).expect("inspect database");
        let stored_version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("stored version");
        assert_eq!(stored_version, version);
        let quantity: i64 = conn
            .query_row("SELECT quantity FROM collection", [], |row| row.get(0))
            .expect("stored collection");
        assert_eq!(quantity, 11);
    }
}
