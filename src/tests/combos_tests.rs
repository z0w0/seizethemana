use super::*;
use std::collections::HashSet;

fn open_store() -> Connection {
    // A fresh file per test run: a static name keyed off the process id
    // keeps parallel nextest workers from sharing a database.
    let path = std::env::temp_dir().join(format!("stm-combos-test-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    crate::db::open(&path).unwrap()
}

fn seed_variant(conn: &Connection, id: &str, popularity: i64, legalities: &str) {
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag, legalities,
            popularity, updated_at)
         VALUES (?1, '[\"Win the game\"]', 3, 'R', ?2, ?3, 'now')",
        rusqlite::params![id, legalities, popularity],
    )
    .unwrap();
}

fn seed_piece(conn: &Connection, combo_id: &str, name: &str, ordinal: i64, cmdr: bool) {
    conn.execute(
        "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
         VALUES (?1, ?2, ?3, '[\"H\"]', ?4)",
        rusqlite::params![combo_id, name, ordinal, cmdr as i64],
    )
    .unwrap();
}

fn names(list: &[&str]) -> HashSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn load_variants_for_joins_pieces_by_name() {
    let conn = open_store();
    seed_variant(&conn, "c1", 100, r#"{"commander":true,"modern":false}"#);
    seed_piece(&conn, "c1", "Demonic Consultation", 0, false);
    seed_piece(&conn, "c1", "Thassa's Oracle", 1, false);
    // A variant no deck name touches; must not come back.
    seed_variant(&conn, "c2", 10, r#"{"commander":true,"modern":true}"#);
    seed_piece(&conn, "c2", "Unrelated", 0, false);

    let hits = load_variants_for(&conn, &names(&["Thassa's Oracle"])).unwrap();
    assert_eq!(hits.len(), 1);
    let (variant, pieces) = &hits[0];
    assert_eq!(variant.id, "c1");
    assert_eq!(variant.produces, vec!["Win the game"]);
    assert_eq!(variant.popularity, Some(100));
    assert_eq!(pieces.len(), 2);
    assert_eq!(pieces[0].name, "Demonic Consultation");

    // Face-splits and multi-name joins return the same single variant.
    let hits = load_variants_for(&conn, &names(&["Demonic Consultation", "Unrelated"])).unwrap();
    assert_eq!(hits.len(), 2, "sorted by id, both variants");
    assert_eq!(hits[0].0.id, "c1");
    assert_eq!(hits[1].0.id, "c2");
}

#[test]
fn load_variants_for_is_empty_on_no_matches() {
    let conn = open_store();
    seed_variant(&conn, "c1", 1, "{}");
    seed_piece(&conn, "c1", "A", 0, false);
    assert!(
        load_variants_for(&conn, &names(&["Zilch"]))
            .unwrap()
            .is_empty()
    );
    assert!(
        load_variants_for(&conn, &HashSet::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn load_variants_for_spreads_large_name_sets() {
    let conn = open_store();
    seed_variant(&conn, "c1", 1, r#"{"commander":true}"#);
    // 600 distinct piece names: two candidate chunks must still find it.
    let all: Vec<String> = (0..600).map(|i| format!("Card {i}")).collect();
    for (i, name) in all.iter().enumerate() {
        seed_piece(&conn, "c1", name, i as i64, false);
    }
    let set: HashSet<String> = all.into_iter().collect();
    let hits = load_variants_for(&conn, &set).unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn requires_commander_reads_piece_flag() {
    let plain = ComboPieceRow {
        name: "A".into(),
        ordinal: 0,
        zones: vec!["H".into()],
        must_be_commander: false,
    };
    let cmdr = ComboPieceRow {
        name: "B".into(),
        ordinal: 1,
        zones: vec!["C".into()],
        must_be_commander: true,
    };
    assert!(!requires_commander(std::slice::from_ref(&plain)));
    assert!(requires_commander(&[plain, cmdr]));
}

fn variant(legalities: &str) -> ComboVariant {
    ComboVariant {
        id: "x".into(),
        produces: vec![],
        mana_value_needed: 2,
        bracket_tag: None,
        popularity: None,
        legalities: serde_json::from_str(legalities).unwrap(),
    }
}

#[test]
fn variant_legal_in_requires_exact_key() {
    let v = variant(r#"{"modern":true,"commander":false}"#);
    assert!(variant_legal_in(&v, "modern"));
    assert!(!variant_legal_in(&v, "commander"));
    assert!(!variant_legal_in(&v, "standard"), "unknown keys fail");
}

#[test]
fn filter_for_format_drops_commander_required_variants() {
    let conn = open_store();
    // Free combo, legal in modern and commander.
    seed_variant(&conn, "free", 3, r#"{"commander":true,"modern":true}"#);
    seed_piece(&conn, "free", "A", 0, false);
    seed_piece(&conn, "free", "B", 1, false);
    // Commander-only combo (piece must be commander).
    seed_variant(
        &conn,
        "cmdr-only",
        1,
        r#"{"commander":true,"modern":false}"#,
    );
    seed_piece(&conn, "cmdr-only", "A", 0, false);
    seed_piece(&conn, "cmdr-only", "C", 1, true);
    // Modern-legal per the map but requires a commander piece: the flag
    // wins, the combo cannot fire in a 60-card format.
    seed_variant(
        &conn,
        "cmdr-flagged",
        2,
        r#"{"commander":true,"modern":true}"#,
    );
    seed_piece(&conn, "cmdr-flagged", "A", 0, false);
    seed_piece(&conn, "cmdr-flagged", "D", 1, true);

    let all = load_variants_for(&conn, &names(&["A", "B", "C", "D"])).unwrap();
    assert_eq!(all.len(), 3);

    let modern = filter_for_format(all.clone(), "modern");
    assert_eq!(
        modern
            .iter()
            .map(|(v, _)| v.id.as_str())
            .collect::<Vec<_>>(),
        vec!["free"],
        "commander-required variants are dropped for modern"
    );

    let commander = filter_for_format(all, "commander");
    assert_eq!(
        commander
            .iter()
            .map(|(v, _)| v.id.as_str())
            .collect::<Vec<_>>(),
        vec!["cmdr-flagged", "cmdr-only", "free"],
        "commander keeps everything the legality map allows, sorted by id"
    );
}
