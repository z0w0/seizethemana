use super::*;
use std::collections::HashSet;

/// Seed an owned match below 80 unowned matches without any tag rows.
fn owned_search_store(text: &str) -> (tempfile::TempDir, crate::paths::Paths, Connection) {
    let dir = tempfile::tempdir().expect("suggestion store");
    let paths = crate::paths::Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().expect("store directories");
    let conn = crate::db::open(&paths.db()).expect("card database");
    for index in 0..=80 {
        let name = format!("Candidate {index:03}");
        conn.execute(
            "INSERT INTO cards (name, oracle_id, type_line, color_identity,
                                oracle_text, edhrec_rank, legalities)
             VALUES (?1, ?1, 'Legendary Creature', '[\"G\"]', ?2, ?3,
                     '{\"commander\":\"legal\",\"modern\":\"legal\"}')",
            rusqlite::params![name, text, index + 1],
        )
        .expect("candidate card");
    }
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity)
         VALUES ('Candidate 080', 'Cards', 'binder', 1)",
        [],
    )
    .expect("owned candidate");
    let mut status = crate::paths::Status::empty();
    status.setup_complete = true;
    status.ingested_cards = 81;
    status.dim = crate::embed::DIM;
    status.write(&paths.status_file()).expect("setup status");
    crate::embed::VectorStore::new()
        .save_vectors(paths.root())
        .expect("empty vector index for full-text search");
    (dir, paths, conn)
}

/// Prove that the fixture's owned card falls outside the old retrieval window.
fn assert_owned_below_global_window(paths: &crate::paths::Paths, conn: &Connection) {
    let hits = crate::query::run_search(
        paths,
        conn,
        &crate::output::Output::new(true, false, false),
        "retrievalneedle",
        &crate::search::CardFilters::default(),
        60,
        None,
    )
    .expect("unrestricted retrieval");
    assert_eq!(hits.len(), 60);
    assert!(hits.iter().all(|hit| hit.card.name != "Candidate 080"));
    let tags: i64 = conn
        .query_row("SELECT COUNT(*) FROM card_tags", [], |row| row.get(0))
        .expect("tag count");
    assert_eq!(tags, 0, "tags must not rescue the owned semantic match");
}

/// Role suggestions retrieve owned semantic matches beyond the global window.
#[test]
fn owned_role_search_retrieves_match_below_global_window() {
    let (_dir, paths, conn) = owned_search_store("retrievalneedle");
    assert_owned_below_global_window(&paths, &conn);
    let owned = crate::collection::owned_names_all(&conn).expect("owned names");
    let ranked = gather_hits(
        &paths,
        &conn,
        &mut crate::output::Output::new(true, false, false),
        "G",
        true,
        None,
        Some(Role::Draw),
        Some("retrievalneedle"),
        None,
        1,
        None,
        true,
        0,
        &[],
        Some(&owned),
    )
    .expect("owned role suggestions");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].0.name, "Candidate 080");
}

/// Commander suggestions retrieve owned matches without a supporting tag leg.
#[test]
fn owned_commander_search_retrieves_match_below_global_window() {
    let (_dir, paths, conn) = owned_search_store("retrievalneedle");
    assert_owned_below_global_window(&paths, &conn);
    let owned = crate::collection::owned_names_all(&conn).expect("owned names");
    let deck = super::super::Deck::parse("// DECK\n1 Candidate 000\n").expect("deck");
    let ranked = commander_candidates(
        &paths,
        &conn,
        &mut crate::output::Output::new(true, false, false),
        &deck,
        Some("retrievalneedle"),
        None,
        None,
        None,
        1,
        &[],
        Some(&owned),
        true,
    )
    .expect("owned commander suggestions");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].0.name, "Candidate 080");
}

/// Keyword retrieval counts only owned matches toward its row limit.
#[test]
fn owned_keyword_search_retrieves_match_below_global_window() {
    let (_dir, paths, conn) = owned_search_store("Draw a card.");
    let global =
        keyword_hits(&conn, Role::Draw, "G", true, 60, None).expect("unrestricted keyword matches");
    assert_eq!(global.len(), 60);
    assert!(global.iter().all(|card| card.name != "Candidate 080"));
    let owned = crate::collection::owned_names_all(&conn).expect("owned names");
    let ranked = gather_hits(
        &paths,
        &conn,
        &mut crate::output::Output::new(true, false, false),
        "G",
        true,
        None,
        Some(Role::Draw),
        Some("absentneedle"),
        None,
        1,
        None,
        true,
        0,
        &[],
        Some(&owned),
    )
    .expect("owned keyword suggestions");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].0.name, "Candidate 080");
}

/// Unowned tag matches cannot fill the fusion window before the ownership filter.
#[test]
fn owned_tag_search_filters_before_fusion() {
    let (_dir, paths, conn) = owned_search_store("retrievalneedle");
    conn.execute_batch(
        "INSERT INTO tags (id, slug, label) VALUES ('draw', 'draw', 'draw');
         INSERT INTO card_tags (oracle_id, tag_id) SELECT oracle_id, 'draw' FROM cards;",
    )
    .expect("draw labels");
    let owned: HashSet<String> = ["Candidate 080".to_string()].into();
    let ranked = gather_hits(
        &paths,
        &conn,
        &mut crate::output::Output::new(true, false, false),
        "G",
        true,
        None,
        Some(Role::Draw),
        Some("absentneedle"),
        None,
        1,
        None,
        true,
        0,
        &[],
        Some(&owned),
    )
    .expect("owned tag suggestions");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].0.name, "Candidate 080");
    assert_eq!(ranked[0].1, ["draw"]);
}
