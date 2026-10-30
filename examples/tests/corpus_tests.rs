use super::*;

/// Provide complete stored facts without a database or model.
fn card() -> db::CardRow {
    db::CardRow {
        name: "Test Card".into(),
        oracle_id: "test-oracle".into(),
        mana_cost: "{1}{U}".into(),
        cmc: 2.0,
        type_line: "Creature".into(),
        colors: r#"["U"]"#.into(),
        color_identity: r#"["U"]"#.into(),
        keywords: r#"["Flying"]"#.into(),
        power: Some("1".into()),
        toughness: Some("2".into()),
        loyalty: None,
        oracle_text: "Draw a card.".into(),
        rarity: "common".into(),
        edhrec_rank: Some(100),
        penny_rank: Some(200),
        reserved: Some(false),
        legalities: r#"{"modern":"legal"}"#.into(),
        set_code: "tst".into(),
        collector_number: "1".into(),
        scryfall_id: "test-print".into(),
        released_at: "2020-01-01".into(),
        game_changer: Some(false),
    }
}

/// Every CardRow field must affect report identity, including non-document facts.
#[test]
fn corpus_identity_covers_every_stored_card_field() {
    let original = card();
    let tags = tags::TagIndex::default();
    let expected = fingerprint_corpus(std::slice::from_ref(&original), &tags).unwrap();
    let changes: &[fn(&mut db::CardRow)] = &[
        |card| card.name.push('!'),
        |card| card.oracle_id.push('!'),
        |card| card.mana_cost.push('!'),
        |card| card.cmc += 1.0,
        |card| card.type_line.push('!'),
        |card| card.colors = r#"["R"]"#.into(),
        |card| card.color_identity = r#"["R"]"#.into(),
        |card| card.keywords = r#"["Haste"]"#.into(),
        |card| card.power = Some("3".into()),
        |card| card.toughness = Some("4".into()),
        |card| card.loyalty = Some("5".into()),
        |card| card.oracle_text.push('!'),
        |card| card.rarity = "rare".into(),
        |card| card.edhrec_rank = Some(101),
        |card| card.penny_rank = Some(201),
        |card| card.reserved = Some(true),
        |card| card.legalities = r#"{"modern":"banned"}"#.into(),
        |card| card.set_code.push('!'),
        |card| card.collector_number.push('!'),
        |card| card.scryfall_id.push('!'),
        |card| card.released_at.push('!'),
        |card| card.game_changer = Some(true),
    ];
    for (index, change) in changes.iter().enumerate() {
        let mut changed = original.clone();
        change(&mut changed);
        assert_ne!(
            fingerprint_corpus(&[changed], &tags).unwrap(),
            expected,
            "stored field change {index} did not change the corpus identity"
        );
    }
    assert_eq!(fingerprint_corpus(&[original], &tags).unwrap(), expected);
}

/// Ordered rows and document text are distinct parts of corpus evidence.
#[test]
fn corpus_identity_covers_row_order_and_production_documents() {
    let first = card();
    let mut second = first.clone();
    second.name = "Second Card".into();
    let tags = tags::TagIndex::default();
    assert_ne!(
        fingerprint_corpus(&[first.clone(), second.clone()], &tags).unwrap(),
        fingerprint_corpus(&[second, first.clone()], &tags).unwrap()
    );
    let untagged_fingerprint = fingerprint_corpus(std::slice::from_ref(&first), &tags).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let conn = db::open(&directory.path().join("tags.db")).unwrap();
    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('draw', 'draw', 'draw', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO card_tags (oracle_id, tag_id, weight) VALUES (?1, 'draw', '')",
        [&first.oracle_id],
    )
    .unwrap();
    let changed_tags = tags::TagIndex::load(&conn).unwrap();
    assert_ne!(
        fingerprint_corpus(&[first], &changed_tags).unwrap(),
        untagged_fingerprint
    );
}
