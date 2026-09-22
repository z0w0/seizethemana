// Tests for owned-search location scoping.

use super::*;
use crate::collection_conflicts::locations_for;

fn conn() -> (tempfile::TempDir, Connection) {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    (tmp, conn)
}

fn row(conn: &Connection, binder: &str, kind: &str, name: &str, qty: i64) {
    conn.execute(
        "INSERT INTO collection (name, set_code, collector_number, foil, binder, binder_type, quantity)
         VALUES (?1, 'tst', '1', 'normal', ?2, ?3, ?4)",
        rusqlite::params![name, binder, kind, qty],
    )
    .unwrap();
}

#[test]
fn deck_cards_hidden_by_default_deck_filter_opts_in() {
    let (_tmp, conn) = conn();
    // Aetherize: in a binder AND assigned to Froggy (two rows).
    row(&conn, "Uncommons+", "binder", "Aetherize", 1);
    row(&conn, "Froggy", "deck", "Aetherize", 1);
    // Channeler: deck-assigned only.
    row(&conn, "Froggy", "deck", "Channeler", 1);
    // Gloom: binder only.
    row(&conn, "Uncommons+", "binder", "Gloom", 1);
    // Fog: in a binder the user did not name.
    row(&conn, "Commons", "binder", "Fog", 1);

    // Default (no filters): binder rows only. Deck-assigned cards are
    // spoken for; the pool is what is available.
    let bare = owned_names(&conn, &[], &[]).unwrap();
    assert!(bare.contains("Aetherize"), "binder copy still counts");
    assert!(bare.contains("Gloom"));
    assert!(!bare.contains("Channeler"), "deck-only card is hidden");
    // Locations mirror the scope: no deck rows in a bare search.
    let locs = locations_for(&conn, "Aetherize", &[], &[]).unwrap();
    assert_eq!(
        locs,
        vec![("Uncommons+".to_string(), "binder".to_string(), 1)]
    );

    // --deck opts in to that deck's rows (plus all binder rows).
    let with_deck = owned_names(&conn, &[], &["Froggy".into()]).unwrap();
    assert!(with_deck.contains("Channeler"));
    assert!(with_deck.contains("Aetherize"));
    // Locations show the deck row too.
    let locs = locations_for(&conn, "Channeler", &[], &["Froggy".into()]).unwrap();
    assert_eq!(locs, vec![("Froggy".to_string(), "deck".to_string(), 1)]);

    // --binder narrows the binder pool; named decks still union in.
    let scoped = owned_names(&conn, &["Uncommons+".into()], &["Froggy".into()]).unwrap();
    assert!(scoped.contains("Aetherize"));
    assert!(scoped.contains("Channeler"));
    assert!(!scoped.contains("Fog"), "un-named binder is out");
    let locs = locations_for(
        &conn,
        "Aetherize",
        &["Uncommons+".into()],
        &["Froggy".into()],
    )
    .unwrap();
    assert_eq!(
        locs,
        vec![
            ("Uncommons+".to_string(), "binder".to_string(), 1),
            ("Froggy".to_string(), "deck".to_string(), 1),
        ]
    );

    // Decks the user did not name contribute nothing.
    row(&conn, "Other", "deck", "Sneaky", 1);
    let scoped = owned_names(&conn, &[], &["Froggy".into()]).unwrap();
    assert!(!scoped.contains("Sneaky"));
}
