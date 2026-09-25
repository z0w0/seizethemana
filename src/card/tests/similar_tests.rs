use super::super::similar::*;
use rusqlite::Connection;

fn insert_card(conn: &Connection, name: &str, oracle_id: &str) {
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES (?1, ?2)",
        rusqlite::params![name, oracle_id],
    )
    .unwrap();
}

#[test]
fn rank_similar_orders_by_shared_tags() {
    let tmp = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&tmp.path().join("t.db")).unwrap();
    // Seed: Bolt (oid-bolt) tagged t1, t2, t3.
    // Other cards: Fire (t1,t2 => 2 shared), Ice (t1 => 1), Boltless (none).
    insert_card(&conn, "Bolt", "oid-bolt");
    insert_card(&conn, "Fire", "oid-fire");
    insert_card(&conn, "Ice", "oid-ice");
    insert_card(&conn, "Boltless", "oid-none");
    for oid in ["oid-bolt", "oid-fire", "oid-ice"] {
        conn.execute(
            "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id) VALUES (?1, 't1')",
            [oid],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id) VALUES ('oid-bolt', 't2')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id) VALUES ('oid-fire', 't2')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO card_tags (oracle_id, tag_id) VALUES ('oid-bolt', 't3')",
        [],
    )
    .unwrap();
    for label in [("t1", "burn"), ("t2", "damage"), ("t3", "instant speed")] {
        conn.execute(
            "INSERT OR IGNORE INTO tags (id, slug, label, use_count) VALUES (?1, ?1, ?2, 1)",
            rusqlite::params![label.0, label.1],
        )
        .unwrap();
    }

    let hits = rank_similar(&conn, "oid-bolt", 10, None).unwrap();
    assert_eq!(hits.len(), 2, "Boltless shares nothing");
    assert_eq!(hits[0].card.name, "Fire");
    assert_eq!(hits[0].shared_count, 2);
    assert_eq!(hits[0].shared_tags, vec!["burn", "damage"]);
    assert_eq!(hits[1].card.name, "Ice");
    assert_eq!(hits[1].shared_count, 1);

    // The owned filter restricts results.
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type) VALUES ('Ice', 'Collect', 'binder')",
        [],
    )
    .unwrap();
    let owned = crate::collection::owned_names_all(&conn).unwrap();
    let hits = rank_similar(&conn, "oid-bolt", 10, Some(&owned)).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].card.name, "Ice");

    // Over-select: with a tiny limit and mostly-unowned matches, the
    // owned window must still fill from rows past the SQL cut.
    conn.execute(
        "INSERT INTO collection (name, binder, binder_type) VALUES ('Waste', 'Collect', 'binder')",
        [],
    )
    .unwrap();
    // Waste shares one tag with Bolt and ranks low (high EDHREC rank).
    insert_card(&conn, "Waste", "oid-waste");
    conn.execute(
        "INSERT INTO card_tags (oracle_id, tag_id) VALUES ('oid-waste', 't1')",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE cards SET edhrec_rank = 20000 WHERE name = 'Waste'",
        [],
    )
    .unwrap();
    let owned = crate::collection::owned_names_all(&conn).unwrap();
    let hits = rank_similar(&conn, "oid-bolt", 1, Some(&owned)).unwrap();
    // Fire and Ice are unowned; Waste is owned and must not be cut by
    // the SQL LIMIT 1 before the filter.
    assert_eq!(hits.len(), 1, "owned window filled past the SQL cut");
    assert_eq!(hits[0].card.name, "Waste");
}
