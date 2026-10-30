use super::*;

/// A broken combo store is an error, not a valid empty data set.
#[test]
fn combo_store_errors_are_not_missing_data() {
    let conn = rusqlite::Connection::open_in_memory().expect("open test database");
    assert!(store_has_combos(&conn).is_err());
    conn.execute("CREATE TABLE combos (id TEXT)", [])
        .expect("create combo table");
    assert!(!store_has_combos(&conn).expect("query empty table"));
    conn.execute("INSERT INTO combos VALUES ('one')", [])
        .expect("insert combo");
    assert!(store_has_combos(&conn).expect("query populated table"));
}
