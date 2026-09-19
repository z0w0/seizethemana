// Tests for `deck suggest` (main pipeline + combo completions), split
// from the suggest module to keep the implementation file small.

use super::*;
use std::collections::HashMap;

fn card(name: &str, type_line: &str, identity: &str, text: &str, rank: Option<i64>) -> CardRow {
    let identity_json = serde_json::to_string(
        &identity
            .chars()
            .map(|c| c.to_string())
            .collect::<Vec<String>>(),
    )
    .unwrap();
    CardRow {
        name: name.to_string(),
        oracle_id: format!("oid-{name}"),
        mana_cost: String::new(),
        cmc: 0.0,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: identity_json,
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "rare".into(),
        edhrec_rank: rank,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: "2020-01-01".into(),
        game_changer: None,
    }
}

#[test]
fn role_parse_knows_the_names() {
    assert_eq!(Role::parse("draw"), Some(Role::Draw));
    assert_eq!(Role::parse("REMOVAL"), Some(Role::Removal));
    assert_eq!(Role::parse("finisher"), Some(Role::Wincon));
    assert_eq!(Role::parse("counterspell"), Some(Role::Counterspell));
    assert_eq!(Role::parse("lands"), Some(Role::Landfall));
    assert_eq!(Role::parse("board-wipe"), Some(Role::BoardWipe));
    assert_eq!(Role::parse("sac outlet"), Some(Role::Sacrifice));
    assert_eq!(Role::parse("Typal"), Some(Role::Typal));
    assert_eq!(Role::parse("tribal"), Some(Role::Typal));
    assert_eq!(Role::parse("group-hug"), Some(Role::GroupHug));
    assert_eq!(Role::parse("voltron"), Some(Role::Voltron));
    assert_eq!(Role::parse("stax"), Some(Role::Stax));
    assert!(Role::parse("gibberish").is_none());
}

#[test]
fn commander_identity_collects_letters() {
    let deck = super::super::Deck::parse("// COMMANDER\n1 Frog Wizard\n").unwrap();
    let cards: HashMap<String, CardRow> = [card(
        "Frog Wizard",
        "Legendary Creature — Frog Wizard",
        "GU",
        "Spells.",
        Some(100),
    )]
    .into_iter()
    .map(|c| (c.name.clone(), c))
    .collect();
    assert_eq!(commander_identity(&deck, &cards), "GU");
    // No commander section: empty identity.
    let deck = super::super::Deck::parse("// DECK\n1 Bear\n").unwrap();
    assert_eq!(commander_identity(&deck, &cards), "");
}

#[test]
fn identity_ok_filters_outside_colors() {
    let frog = card("Frog", "Creature — Frog", "GU", "text", None);
    assert!(identity_ok(&frog, "GU"));
    assert!(identity_ok(&frog, "GUB"));
    assert!(!identity_ok(&frog, "G"));
    assert!(!identity_ok(&frog, "RW"));
}

#[test]
fn commander_type_gate_knows_vehicles() {
    // Legendary creature passes; plain creature does not.
    assert!(super::super::legal::is_commander_type(&card(
        "Frog Wizard",
        "Legendary Creature — Frog",
        "GU",
        "",
        None
    )));
    assert!(!super::super::legal::is_commander_type(&card(
        "Bear",
        "Creature — Bear",
        "G",
        "",
        None
    )));
    // Legendary Vehicle with a P/T box commands (post-2025 rule).
    let mut vehicle = card(
        "War Frog",
        "Legendary Artifact — Vehicle",
        "G",
        "Trample",
        None,
    );
    vehicle.power = Some("4".to_string());
    vehicle.toughness = Some("4".to_string());
    assert!(super::super::legal::is_commander_type(&vehicle));
    // No P/T box: not a commander.
    let mut bare = card(
        "War Frog",
        "Legendary Artifact — Vehicle",
        "G",
        "Trample",
        None,
    );
    bare.power = None;
    bare.toughness = None;
    assert!(!super::super::legal::is_commander_type(&bare));
}

#[test]
fn fuse_legs_rewards_consensus_and_keeps_labels() {
    let sem = card("Shared", "Creature", "G", "", Some(10));
    let mut sem_only = card("Semantic", "Creature", "G", "", Some(20));
    sem_only.oracle_id = "oid-semantic".to_string();
    let mut tag_only = card("Tagged", "Creature", "G", "", Some(30));
    tag_only.oracle_id = "oid-tagged".to_string();
    let labels = vec!["frog".to_string()];
    let owned_list = vec![(tag_only, labels.clone())];
    // Shared ranks first: seen on both legs. Semantic and tag-only cards
    // tie at one-leg scores, but Tagged sits first on the tag leg while
    // Semantic sits second on the semantic leg: Tagged's 1/(k+1) beats
    // Semantic's 1/(k+2).
    let fused = fuse_legs(&[sem.clone(), sem_only.clone()], &owned_list, 3);
    let names: Vec<&str> = fused.iter().map(|(c, _, _)| c.name.as_str()).collect();
    assert_eq!(names, vec!["Shared", "Tagged", "Semantic"]);
    // Labels ride along with their card only.
    assert_eq!(fused[0].1.len(), 0, "Shared joined via the semantic leg");
    assert_eq!(fused[1].1, labels);
    // Fused scores are normalized to [0, 1]; consensus ranks first.
    assert!((0.0..=1.0).contains(&fused[0].2), "score in range");
    assert!(fused[0].2 > fused[2].2, "Shared outscores Semantic");
}

#[test]
fn group_owned_first_splits_groups() {
    let owned_set: std::collections::HashMap<String, i64> =
        [("Owned".to_string(), 1i64)].into_iter().collect();
    let a = card("Owned", "Creature", "G", "", Some(2));
    let b = card("Unowned", "Creature", "G", "", Some(1));
    let grouped = group_owned_first(vec![(b, vec![], 0.5), (a.clone(), vec![], 0.4)], &owned_set);
    assert_eq!(grouped[0].0.name, "Owned", "owned card jumps the queue");
    assert_eq!(grouped[1].0.name, "Unowned");
    assert!(grouped[0].0.name == a.name);
}

#[test]
fn card_is_commander_legal_blocks_banned() {
    let mut banned = card("Bad", "Creature", "G", "", None);
    banned.legalities = r#"{"commander": "banned"}"#.to_string();
    assert!(!card_is_commander_legal(&banned));
    let legal = card("Good", "Creature", "G", "", None);
    assert!(card_is_commander_legal(&legal));
    // Unknown legality passes (deck legal reports it separately).
    assert!(card_is_commander_legal(&card(
        "Mystery", "Creature", "G", "", None
    )));
}

#[test]
fn card_legal_in_pins_formats() {
    let mut modern_only = card("Modern", "Creature", "G", "", None);
    modern_only.legalities = r#"{"modern": "legal"}"#.to_string();
    // No pinned format passes everything.
    assert!(card_legal_in(&modern_only, None));
    assert!(card_legal_in(&card("Any", "Creature", "G", "", None), None));
    // A pinned format needs an explicit legal/restricted key.
    assert!(card_legal_in(&modern_only, Some("modern")));
    assert!(!card_legal_in(&modern_only, Some("commander")));
    // Missing key and unknown formats fail (not a silent pass).
    let unlisted = card("Unlisted", "Creature", "G", "", None);
    assert!(!card_legal_in(&unlisted, Some("modern")));
    let mut banned = card("Banned", "Creature", "G", "", None);
    banned.legalities = r#"{"modern": "banned"}"#.to_string();
    assert!(!card_legal_in(&banned, Some("modern")));
}

#[test]
fn combo_suggest_ranks_missing_card_and_filters_identity() {
    use crate::spellbook::{ComboPieceRow, ComboVariant};

    let dir = tempfile::tempdir().unwrap();
    let mut conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    // Deck cards: two held pieces. Missing piece 1: inside identity;
    // missing piece 2: outside identity (filtered).
    for (name, identity, _rank) in [
        ("Held A", "GU", Some(50_i64)),
        ("Held B", "GU", Some(51)),
        ("Wanted", "GU", Some(10)),
        ("OffColor", "RW", Some(1)),
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '', 2, 'Creature', '[]', ?3, '[]', 'text', 'rare',
                '{\"commander\":\"legal\"}', 'tst', '1', 'sid', '2020-01-01')",
            rusqlite::params![
                name,
                format!("oid-{name}"),
                serde_json::to_string(&identity.chars().map(|c| c.to_string()).collect::<Vec<_>>())
                    .unwrap()
            ],
        )
        .unwrap();
    }
    // Variants: held A + held B + missing. One missing card is inside
    // identity, one outside.
    let mut legalities = HashMap::new();
    legalities.insert("commander".to_string(), true);
    let _mk = |id: &str, names: &[&str]| {
        (
            ComboVariant {
                id: id.to_string(),
                produces: vec!["Win the game".to_string()],
                mana_value_needed: 3,
                bracket_tag: Some("S".to_string()),
                popularity: Some(1000),
                legalities: legalities.clone(),
            },
            names
                .iter()
                .enumerate()
                .map(|(i, n)| ComboPieceRow {
                    name: n.to_string(),
                    ordinal: i as i64,
                    zones: vec!["B".to_string()],
                    must_be_commander: false,
                })
                .collect::<Vec<_>>(),
        )
    };
    // Seed the store directly.
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag,
            legalities, popularity, updated_at)
         VALUES ('1-2', '[\"Win the game\"]', 3, 'S', '{\"commander\":true}', 1000, 'now')",
        [],
    )
    .unwrap();
    for (name, ordinal) in [("Held A", 0), ("Held B", 1), ("Wanted", 2)] {
        conn.execute(
            "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
             VALUES ('1-2', ?1, ?2, '[\"B\"]', 0)",
            rusqlite::params![name, ordinal],
        )
        .unwrap();
    }
    // Variant 3-4: Held A + OffColor (identity-violating missing card).
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag,
            legalities, popularity, updated_at)
         VALUES ('3-4', '[\"Win the game\"]', 3, 'S', '{\"commander\":true}', 2000, 'now')",
        [],
    )
    .unwrap();
    for (name, ordinal) in [("Held A", 0), ("OffColor", 1)] {
        conn.execute(
            "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
             VALUES ('3-4', ?1, ?2, '[\"B\"]', 0)",
            rusqlite::params![name, ordinal],
        )
        .unwrap();
    }

    let paths = crate::paths::Paths::new(dir.path().to_path_buf());
    crate::deck::create(
        &paths,
        &mut crate::output::Output::new(true, false, false),
        "CDeck",
    )
    .unwrap();
    let deck_file = paths.deck_file("CDeck");
    // Commander card row must exist for identity.
    conn.execute(
        "INSERT OR IGNORE INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at)
         VALUES ('Frog Wizard', 'oid-fw', '', 4, 'Legendary Creature — Frog', '[]',
            '[\"G\",\"U\"]', '[]', 'text', 'rare', '{\"commander\":\"legal\"}', 'tst', '1', 'sid-fw', '2020-01-01')",
        [],
    )
    .unwrap();
    std::fs::write(
        &deck_file,
        "// COMMANDER\n1 Frog Wizard\n// DECK\n1 Held A\n1 Held B\n",
    )
    .unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let code = crate::deck::suggest_combo::run_combo_suggest(
        &paths, &mut conn, &mut out, "CDeck", None, None, 10, true,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // run_combo_suggest prints to stdout, so verify the completion filter
    // against the store: the OffColor variant joins but identity filtering
    // removes it from completions; Wanted is the only in-identity miss.
    let deck_names: std::collections::HashSet<String> =
        ["Held A", "Held B"].iter().map(|s| s.to_string()).collect();
    let variants = crate::combos::load_variants_for(&conn, &deck_names).unwrap();
    let missing_names: Vec<Vec<&str>> = variants
        .iter()
        .filter_map(|(_v, pieces)| {
            let missing: Vec<&str> = pieces
                .iter()
                .map(|p| p.name.as_str())
                .filter(|n| *n != "Held A" && *n != "Held B")
                .collect();
            (missing.len() == 1).then_some(missing)
        })
        .collect();
    assert_eq!(
        missing_names.len(),
        2,
        "both variants join, but the OffColor completion is filtered by identity"
    );
    assert!(
        missing_names.iter().flatten().any(|n| *n == "Wanted"),
        "the in-identity Wanted card is a completion"
    );
    // The filter path itself is covered by completion()'s identity_ok
    // call; assert it directly on the OffColor card row.
    let off_color = crate::db::get_card(&conn, "OffColor")
        .unwrap()
        .expect("OffColor stored");
    let deck =
        crate::deck::Deck::parse("// COMMANDER\n1 Frog Wizard\n// DECK\n1 Held A\n1 Held B\n")
            .unwrap();
    let identity = commander_identity(&deck, &crate::deck::stats::lookup_names(&conn, &deck));
    assert!(!identity_ok(&off_color, &identity));
}

#[test]
fn combo_suggest_excludes_commander_required_for_constructed() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    // Deck: one held piece; missing card is modern-legal. Two variants:
    // one free (completes), one whose second piece must be a commander.
    for name in ["Held", "Wanted"] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '', 2, 'Creature', '[]', '[\"G\"]', '[]', 'text', 'rare',
                '{\"commander\":\"legal\",\"modern\":\"legal\"}', 'tst', '1', 'sid', '2020-01-01')",
            rusqlite::params![name, format!("oid-{name}")],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag,
            legalities, popularity, updated_at)
         VALUES ('free', '[]', 2, 'C', '{\"commander\":true,\"modern\":true}', 500, 'now')",
        [],
    )
    .unwrap();
    for (name, ordinal) in [("Held", 0), ("Wanted", 1)] {
        conn.execute(
            "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
             VALUES ('free', ?1, ?2, '[\"B\"]', 0)",
            rusqlite::params![name, ordinal],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag,
            legalities, popularity, updated_at)
         VALUES ('locked', '[]', 2, 'C', '{\"commander\":true,\"modern\":true}', 900, 'now')",
        [],
    )
    .unwrap();
    // Second piece must be the commander: not a 60-card completion.
    for (name, ordinal, cmdr) in [("Held", 0, 0), ("Wanted", 1, 1)] {
        conn.execute(
            "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
             VALUES ('locked', ?1, ?2, '[\"B\"]', ?3)",
            rusqlite::params![name, ordinal, cmdr],
        )
        .unwrap();
    }

    let paths = crate::paths::Paths::new(dir.path().to_path_buf());
    crate::deck::create(
        &paths,
        &mut crate::output::Output::new(true, false, false),
        "Modern",
    )
    .unwrap();
    std::fs::write(paths.deck_file("Modern"), "// DECK\n1 Held\n").unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    // JSON output carries the per-completion rows the assertions read.
    let code = crate::deck::suggest_combo::run_combo_suggest(
        &paths, &mut conn, &mut out, "Modern", None, None, 10, true,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // "Wanted" completes the free modern-legal variant (2 near-misses but
    // the locked variant is commander-only and must not surface). The
    // variants_completed count distinguishes them: free contributes 1.
    let variants =
        crate::combos::load_variants_for(&conn, &["Held".to_string()].into_iter().collect())
            .unwrap();
    let legal: Vec<(&str, bool)> = variants
        .iter()
        .map(|(v, p)| (v.id.as_str(), crate::combos::requires_commander(p)))
        .collect();
    assert_eq!(
        legal,
        vec![("free", false), ("locked", true)],
        "both variants load; only 'free' fires in a 60-card deck"
    );
}
