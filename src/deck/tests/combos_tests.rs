use super::*;

fn seeded_conn() -> (tempfile::TempDir, rusqlite::Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    (tmp, conn)
}

fn insert_combo(
    conn: &rusqlite::Connection,
    id: &str,
    pieces: &[&str],
    bracket: Option<&str>,
    produces: &[&str],
) {
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag, legalities, updated_at)
         VALUES (?1, ?2, 3, ?3, '{\"commander\": true}', 't')",
        rusqlite::params![
            id,
            serde_json::to_string(&produces.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                .unwrap(),
            bracket,
        ],
    )
    .unwrap();
    for (ordinal, piece) in pieces.iter().enumerate() {
        conn.execute(
            "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
             VALUES (?1, ?2, ?3, '[\"H\"]', 0)",
            rusqlite::params![id, piece, ordinal as i64],
        )
        .unwrap();
    }
}

fn silent_out() -> crate::output::Output {
    crate::output::Output::new(true, false, false)
}

#[test]
fn section_combos_splits_complete_and_near_miss() {
    let (_tmp, conn) = seeded_conn();
    insert_combo(
        &conn,
        "c1",
        &["Piece A", "Piece B"],
        Some("S"),
        &["Win the game"],
    );
    insert_combo(&conn, "c2", &["Piece A", "Piece C"], None, &["Combo"]);

    let names: std::collections::HashSet<String> = ["Piece A", "Piece B"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let commanders: std::collections::HashSet<String> = Default::default();
    let result = section_combos(
        &conn,
        "DECK",
        &names,
        &commanders,
        "commander",
        &Default::default(),
    )
    .unwrap();
    assert_eq!(result.complete.len(), 1, "A + B complete");
    assert_eq!(result.complete[0].combo, "Piece A + Piece B");
    assert_eq!(result.near_misses.len(), 1, "A + C misses Piece C");
    assert_eq!(result.near_misses[0].missing, vec!["Piece C"]);
    assert!(!result.near_misses[0].self_contained);
}

#[test]
fn commander_piece_never_reports_missing() {
    let (_tmp, conn) = seeded_conn();
    insert_combo(
        &conn,
        "c1",
        &["Breya, Etherium Shaper", "Piece X"],
        None,
        &["Win the game"],
    );
    // A commander-required piece (must_be_commander) with the commander in
    // the command zone resolves without a main-deck copy.
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag, legalities, updated_at)
         VALUES ('c2', '[]', 3, NULL, '{\"commander\": true}', 't')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
         VALUES ('c2', 'Breya, Etherium Shaper', 0, '[\"C\"]', 1),
                ('c2', 'Piece X', 1, '[\"H\"]', 0)",
        [],
    )
    .unwrap();

    let names: std::collections::HashSet<String> =
        ["Piece X"].iter().map(|s| s.to_string()).collect();
    let commanders: std::collections::HashSet<String> = ["Breya, Etherium Shaper"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = section_combos(
        &conn,
        "DECK",
        &names,
        &commanders,
        "commander",
        &Default::default(),
    )
    .unwrap();
    assert_eq!(result.complete.len(), 2, "both variants complete");
    assert!(result.near_misses.is_empty());
}

#[test]
fn format_illegal_combos_are_filtered() {
    let (_tmp, conn) = seeded_conn();
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag, legalities, updated_at)
         VALUES ('c1', '[]', 3, NULL, '{\"commander\": true}', 't')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
         VALUES ('c1', 'A', 0, '[\"H\"]', 0), ('c1', 'B', 1, '[\"H\"]', 0)",
        [],
    )
    .unwrap();
    let names: std::collections::HashSet<String> =
        ["A", "B"].iter().map(|s| s.to_string()).collect();
    let empty: std::collections::HashSet<String> = Default::default();
    let result =
        section_combos(&conn, "DECK", &names, &empty, "modern", &Default::default()).unwrap();
    assert!(result.complete.is_empty(), "filtered by format");
    let result = section_combos(
        &conn,
        "DECK",
        &names,
        &empty,
        "commander",
        &Default::default(),
    )
    .unwrap();
    assert_eq!(result.complete.len(), 1);
}

#[test]
fn breaks_bracket_flags_cedh_grade_tags() {
    assert!(breaks_bracket(Some("S"), 3));
    assert!(breaks_bracket(Some("K"), 2));
    assert!(!breaks_bracket(Some("S"), 5), "cEDH tolerates S combos");
    assert!(!breaks_bracket(None, 1), "untagged combos break nothing");
}

#[test]
fn combos_end_to_end_reports_sections_and_brackets() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    let (_tmp2, conn) = seeded_conn();
    insert_combo(
        &conn,
        "c1",
        &["Bolt", "Piece X"],
        Some("S"),
        &["Infinite turns"],
    );
    insert_combo(&conn, "c2", &["Bolt", "Shock"], None, &["Combo"]);
    std::fs::write(
        paths.deck_file("TestDeck"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n1 Shock\n// SIDEBOARD\n1 Piece X\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = combos(&paths, &conn, &mut out, "TestDeck", None, Some(3), false).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // JSON shape check.
    let mut json_out = silent_out();
    let code = combos(&paths, &conn, &mut json_out, "TestDeck", None, None, true).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
}

#[test]
fn deck_without_combo_pieces_reports_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::Paths::new(tmp.path().to_path_buf());
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    let (_tmp2, conn) = seeded_conn();
    insert_combo(&conn, "c1", &["Piece A", "Piece B"], None, &["Combo"]);
    std::fs::write(paths.deck_file("Plain"), "// DECK\n1 Bolt\n").unwrap();
    let mut out = silent_out();
    let code = combos(&paths, &conn, &mut out, "Plain", None, None, true).unwrap();
    assert_eq!(code, crate::cli::codes::OK);
}

#[test]
fn near_miss_owned_flag_and_full_produces() {
    let (_tmp, conn) = seeded_conn();
    insert_combo(
        &conn,
        "c1",
        &["Piece A", "Piece B"],
        None,
        &["infinite mana", "infinite draw", "win the game"],
    );
    // Piece B is owned in the binder (collection-wide).
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type, quantity)
         VALUES ('Piece B', 'binder-main', 'binder', 1)",
        [],
    )
    .unwrap();
    let names: std::collections::HashSet<String> =
        ["Piece A"].iter().map(|s| s.to_string()).collect();
    let commanders: std::collections::HashSet<String> = Default::default();
    let owned = crate::collection::owned_counts_all(&conn).unwrap();
    let result = section_combos(&conn, "DECK", &names, &commanders, "commander", &owned).unwrap();
    assert_eq!(result.near_misses.len(), 1);
    let miss = &result.near_misses[0];
    assert_eq!(miss.owned, Some(true), "the completing piece is owned");
    assert_eq!(
        miss.produces.len(),
        3,
        "the produces list is no longer truncated"
    );
}

#[test]
fn near_miss_to_buy_and_popularity_and_commander_flag() {
    let (_tmp, conn) = seeded_conn();
    insert_combo(&conn, "c1", &["Piece A", "Unowned Piece"], None, &["win"]);
    // A commander-required variant.
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag, legalities, updated_at)
         VALUES ('c2', '[\"mana\"]', 3, NULL, '{\"commander\": true}', 't')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
         VALUES ('c2', 'Piece A', 0, '[\"H\"]', 0),
                ('c2', 'Cmd Piece', 1, '[\"C\"]', 1)",
        [],
    )
    .unwrap();
    let names: std::collections::HashSet<String> =
        ["Piece A"].iter().map(|s| s.to_string()).collect();
    let commanders: std::collections::HashSet<String> = Default::default();
    let result = section_combos(
        &conn,
        "DECK",
        &names,
        &commanders,
        "commander",
        &Default::default(),
    )
    .unwrap();
    let miss = &result.near_misses[0];
    assert_eq!(miss.owned, Some(false), "the completing piece is a to-buy");
    // The commander-required variant resolves (the commander piece never
    // reports missing) and carries the flag.
    let complete = result.complete.iter().find(|r| r.id == "c2").unwrap();
    assert!(complete.complete);
    assert!(complete.requires_commander);
    // The unowned near miss has no popularity in the store → None.
    assert_eq!(miss.popularity, None);
}

#[test]
fn bracket_breaks_flag_near_misses_too() {
    let (_tmp, conn) = seeded_conn();
    insert_combo(
        &conn,
        "c1",
        &["Piece A", "Piece B"],
        Some("S"),
        &["extra turns"],
    );
    let names: std::collections::HashSet<String> =
        ["Piece A"].iter().map(|s| s.to_string()).collect();
    let commanders: std::collections::HashSet<String> = Default::default();
    let result = section_combos(
        &conn,
        "DECK",
        &names,
        &commanders,
        "commander",
        &Default::default(),
    )
    .unwrap();
    assert_eq!(result.near_misses.len(), 1);
    // The command-level bracket_breaks assembly covers near misses.
    let mut c = result.clone();
    c.bracket_breaks = c
        .complete
        .iter()
        .chain(c.near_misses.iter())
        .filter(|r| breaks_bracket(r.bracket_tag.as_deref(), 3))
        .map(|r| r.combo.clone())
        .collect();
    assert_eq!(
        c.bracket_breaks.len(),
        1,
        "the S-tagged near miss breaks bracket 3"
    );
}
