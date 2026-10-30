use super::*;

/// Exact names use the binary name index rather than a case-insensitive scan.
#[test]
fn exact_import_lookup_uses_name_index() {
    let dir = tempfile::tempdir().expect("collection store");
    let conn = crate::db::open(&dir.path().join("cards.db")).expect("card database");
    let sql = format!(
        "EXPLAIN QUERY PLAN SELECT {} FROM cards WHERE name = ?1",
        crate::db::CARD_COLUMNS
    );
    let plan: String = conn
        .query_row(&sql, ["Bolt"], |row| row.get(3))
        .expect("exact lookup plan");
    assert!(plan.contains("SEARCH cards USING INDEX"), "{plan}");
    assert!(!plan.contains("SCAN"), "{plan}");
}

/// Exact and case-varied real names beat tokens; token prefixes stay excluded.
#[test]
fn import_preserves_exact_case_and_token_prefix_rules() {
    let dir = tempfile::tempdir().expect("collection store");
    let paths = crate::paths::Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().expect("store directories");
    let mut conn = crate::db::open(&paths.db()).expect("card database");
    conn.execute_batch(
        "INSERT INTO cards (name, oracle_id) VALUES ('Bolt', 'bolt'), ('Food Chain', 'chain');
         INSERT INTO token_names (name) VALUES ('Bolt'), ('bOlT'), ('Food'), ('food');",
    )
    .expect("real cards and duplicate token names");
    let mut status = crate::paths::Status::empty();
    status.setup_complete = true;
    status.write(&paths.status_file()).expect("setup status");
    let csv = dir.path().join("collection.csv");
    std::fs::write(
        &csv,
        "Binder Name,Binder Type,Name,Set code,Collector number,Foil,Quantity\n\
         Cards,binder,Bolt,TST,1,normal,1\n\
         Cards,binder,bOlT,TST,1,normal,2\n\
         Cards,binder,Food,TST,1,normal,7\n\
         Cards,binder,food,TST,1,normal,8\n\
         Cards,binder,Food Ch,TST,1,normal,4\n",
    )
    .expect("collection CSV");
    let code = import(
        &paths,
        &mut conn,
        &mut crate::output::Output::new(true, false, false),
        &csv,
        false,
    )
    .expect("collection import");
    assert_eq!(code, crate::cli::codes::OK);
    let rows: Vec<(String, i64)> = conn
        .prepare("SELECT name, quantity FROM collection ORDER BY name")
        .expect("collection query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("collection rows")
        .collect::<Result<_, _>>()
        .expect("stored ownership");
    assert_eq!(
        rows,
        [("Bolt".to_string(), 3), ("Food Chain".to_string(), 4)]
    );
}
