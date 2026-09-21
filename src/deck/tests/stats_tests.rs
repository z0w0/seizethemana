// Tests for deck stats: buckets, ramp roles, aggregates.

use super::*;

fn card(name: &str, cmc: f64, type_line: &str, identity: &str, text: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: String::new(),
        cmc,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: identity.into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

fn deck_of(entries: &[(&str, i64)]) -> Deck {
    let mut deck = Deck::default();
    for (name, qty) in entries {
        deck.section_entries_mut("DECK")
            .push(crate::deck::grammar::DeckEntry {
                quantity: *qty,
                name: name.to_string(),
                set_code: None,
                collector_number: None,
                foil: false,
            });
    }
    deck
}

#[test]
fn cmc_bucket_classifies() {
    assert_eq!(cmc_bucket(0.0, false), Some("0".into()));
    assert_eq!(cmc_bucket(6.0, false), Some("6".into()));
    // Fractional CMC truncates into its floor bucket (6.5 is "6").
    assert_eq!(cmc_bucket(6.5, false), Some("6".into()));
    assert_eq!(cmc_bucket(9.0, false), Some("7+".into()));
    assert_eq!(cmc_bucket(2.0, true), None); // lands stay off the curve
}

#[test]
fn ramp_roles_split_lands_rocks_dorks() {
    let land = card("Plains", 0.0, "Basic Land — Plains", "[]", "");
    let rock = card("Sol Ring", 1.0, "Artifact", "[]", "{T}: Add {C}{C}.");
    let dork = card(
        "Llanowar Elves",
        1.0,
        "Creature — Elf",
        "[]",
        "{T}: Add {G}.",
    );
    assert!(is_land(&land));
    assert!(is_rock(&rock));
    assert!(is_dork(&dork));
    assert!(!is_rock(&dork));
    assert!(!is_dork(&rock));
    // A plain creature is neither.
    let plain = card("Bear", 2.0, "Creature — Bear", "[]", " vanilla ");
    assert!(!is_dork(&plain) && !is_rock(&plain));
    // Prose mana production ("Add one mana of any color") counts.
    let birds = card(
        "Birds",
        1.0,
        "Creature — Bird",
        "[]",
        "Flying\n{T}: Add one mana of any color.",
    );
    assert!(is_dork(&birds));
}

#[test]
fn compute_aggregates_curve_ramp_colors_types() {
    let cards: HashMap<String, CardRow> = [
        card("Plains", 0.0, "Basic Land — Plains", "[]", ""),
        card("Sol Ring", 1.0, "Artifact", "[]", "{T}: Add {C}{C}."),
        card("Elves", 1.0, "Creature — Elf", r#"["G"]"#, "{T}: Add {G}."),
        card("Bolt", 1.0, "Instant", r#"["R"]"#, "Deal 3."),
        card(
            "Dragon",
            7.0,
            "Creature — Dragon",
            r#"["R","G"]"#,
            "Flying.",
        ),
    ]
    .into_iter()
    .map(|c| (c.name.clone(), c))
    .collect();
    let deck = deck_of(&[
        ("Plains", 10),
        ("Sol Ring", 1),
        ("Elves", 2),
        ("Bolt", 4),
        ("Dragon", 1),
        ("Unknown Card", 2),
    ]);
    let stats = compute(&deck, &cards);
    assert_eq!(stats.total, 20);
    // Lands excluded from curve/avg.
    let curve_count: i64 = stats.curve.iter().map(|b| b.count).sum();
    assert_eq!(curve_count, 8);
    assert!((stats.avg_cmc - 1.75).abs() < 1e-9); // (1*1+1*2+1*4+7*1)/8
    assert_eq!(stats.ramp, (10, 1, 2, 0));
    let colors: Vec<(&str, i64)> = stats
        .colors
        .iter()
        .map(|b| (b.label.as_str(), b.count))
        .collect();
    assert_eq!(colors, vec![("G", 3), ("R", 5)]);
    // Types: most copies first, capped.
    assert!(stats.types[0].count >= stats.types.last().unwrap().count);
    assert!(stats.types.len() <= 6);
}

#[test]
fn compute_empty_deck() {
    let stats = compute(&Deck::default(), &HashMap::new());
    assert_eq!(stats.total, 0);
    assert_eq!(stats.avg_cmc, 0.0);
    assert!(stats.curve.is_empty());
}

#[test]
fn curve_json_histogram_matches_fixture() {
    use crate::deck::grammar::{Deck, DeckEntry};
    let mut cards: HashMap<String, CardRow> = HashMap::new();
    let land = card("Island", 0.0, "Basic Land — Island", "", "");
    cards.insert("Island".into(), land);
    for (name, cmc) in [
        ("One", 1.0),
        ("Two", 2.0),
        ("TwoB", 2.0),
        ("Six", 6.0),
        ("Seven", 8.0),
    ] {
        let mut c = card(name, cmc, "Creature", "", "");
        c.mana_cost = format!("{{{}}}", cmc as i64);
        cards.insert(name.into(), c);
    }
    let deck = Deck {
        sections: vec![(
            "DECK".into(),
            vec![
                DeckEntry {
                    quantity: 10,
                    name: "Island".into(),
                    set_code: None,
                    collector_number: None,
                    foil: false,
                },
                DeckEntry {
                    quantity: 3,
                    name: "One".into(),
                    set_code: None,
                    collector_number: None,
                    foil: false,
                },
                DeckEntry {
                    quantity: 5,
                    name: "Two".into(),
                    set_code: None,
                    collector_number: None,
                    foil: false,
                },
                DeckEntry {
                    quantity: 4,
                    name: "TwoB".into(),
                    set_code: None,
                    collector_number: None,
                    foil: false,
                },
                DeckEntry {
                    quantity: 2,
                    name: "Six".into(),
                    set_code: None,
                    collector_number: None,
                    foil: false,
                },
                DeckEntry {
                    quantity: 1,
                    name: "Seven".into(),
                    set_code: None,
                    collector_number: None,
                    foil: false,
                },
            ],
        )],
    };
    let stats = compute(&deck, &cards);
    let v = curve_json(&stats, false);
    // Histogram indexed MV 0..6+: MV1 in slot 1, MV2 in slot 2, MV6 and
    // the MV8 copy share slot 6.
    assert_eq!(v["histogram"][0], 0, "no MV-0 cards in the fixture");
    assert_eq!(v["histogram"][1], 3);
    assert_eq!(v["histogram"][2], 9);
    assert_eq!(
        v["histogram"][6], 3,
        "MV6 (2 copies) + MV8 (1) in the last slot"
    );
    let avg = v["avg_mv"].as_f64().unwrap();
    assert!((avg - (3.0 * 1.0 + 9.0 * 2.0 + 2.0 * 6.0 + 8.0) / 15.0).abs() < 0.05);
}

#[test]
fn curve_target_is_archetype_aware() {
    assert_eq!(curve_target(true, 1.5), "target: comes together by t4-t6");
    assert_eq!(curve_target(true, 2.5), "target: comes together by t6-t8");
    assert_eq!(curve_target(true, 3.5), "target: comes together by t8-t10");
    assert_eq!(curve_target(false, 1.5), "target: does its thing by t4");
    assert_eq!(curve_target(false, 2.5), "target: does its thing by t4-t6");
    assert_eq!(curve_target(false, 3.5), "target: does its thing by t6");
}
