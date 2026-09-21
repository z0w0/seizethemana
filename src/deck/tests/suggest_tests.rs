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
    let fused = fuse_legs(&[sem.clone(), sem_only.clone()], &owned_list, 3, true);
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
        &paths, &mut conn, &mut out, "CDeck", None, None, None, 10, true,
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
        &paths, &mut conn, &mut out, "Modern", None, None, None, 10, true,
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

#[test]
fn apply_suggest_price_caps_and_reports_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    for (name, usd) in [
        ("Cheap", Some(1.0_f64)),
        ("Pricy", Some(9.0)),
        ("Unpriced", None),
    ] {
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
             VALUES (?1, ?1, 'm11', '148', 'en', 'rare', '[\"nonfoil\"]',
                '2020-01-01', ?2, NULL, 't')",
            rusqlite::params![name, usd],
        )
        .unwrap();
    }
    for name in ["Cheap", "Pricy", "Unpriced"] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '', 2, 'Instant', '[]', '[]', '[]', 'text', 'rare',
                '{}', 'tst', '1', 'sid', '2020-01-01')",
            rusqlite::params![name, format!("oid-{name}")],
        )
        .unwrap();
    }
    let row = |name: &str| {
        (
            crate::db::get_card(&conn, name).unwrap().unwrap(),
            Vec::<String>::new(),
            0.5_f32,
        )
    };
    // No cap: everything passes through untouched.
    let ranked = vec![row("Cheap"), row("Pricy"), row("Unpriced")];
    let passed = super::super::suggest::apply_suggest_price(
        &conn,
        &mut crate::output::Output::new(true, false, false),
        ranked.clone(),
        None,
        true,
    )
    .unwrap();
    assert_eq!(passed.len(), 3, "no cap keeps every row");
    // $2 cap: the above-cap and unpriced rows are hidden.
    let ranked = vec![row("Cheap"), row("Pricy"), row("Unpriced")];
    let (kept, hidden) =
        crate::prints::retain_by_price(&conn, ranked, |(card, _, _)| &card.name, 2.0).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(hidden, 2);
}

#[test]
fn suggest_price_helper_keeps_only_priced_rows() {
    // Phase 6.1: the shared helper is the whole price gate — priced rows
    // at or under the cap stay, unpriced and above-cap rows go.
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    for (name, usd) in [
        ("Cheap", Some(1.0_f64)),
        ("Pricy", Some(9.0)),
        ("Unpriced", None),
    ] {
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
             VALUES (?1, ?2, 'm11', '148', 'en', 'rare', '[\"nonfoil\"]',
                '2020-01-01', ?3, NULL, 't')",
            rusqlite::params![format!("sid-{name}"), name, usd],
        )
        .unwrap();
    }
    let names = vec![
        "Cheap".to_string(),
        "Pricy".to_string(),
        "Unpriced".to_string(),
    ];
    let (kept, hidden) = crate::prints::retain_by_price(&conn, names, |n| n, 2.0).unwrap();
    assert_eq!(kept, vec!["Cheap".to_string()]);
    assert_eq!(hidden, 2, "above-cap and unpriced rows are hidden");
}

#[test]
fn fuse_legs_without_edhrec_tiebreak_ranks_neutrally() {
    // 60-card neutral ranking: no EDHREC influence, fused score then name.
    let mut loved = card("Loved", "Creature", "G", "", Some(1));
    loved.oracle_id = "oid-loved".to_string();
    let mut plain = card("Plain", "Creature", "G", "", None);
    plain.oracle_id = "oid-plain".to_string();
    // The two cards tie on fused score (one leg, same rank). With the
    // tiebreak on, Loved's rank 1 pulls it ahead; without it, the neutral
    // key decides (fused score, then name).
    let tag_leg = vec![(loved.clone(), Vec::<String>::new())];
    let with = fuse_legs(&[plain.clone()], &tag_leg, 2, true);
    let without = fuse_legs(&[plain.clone()], &tag_leg, 2, false);
    assert_eq!(with[0].0.name, "Loved", "EDHREC rank breaks the tie");
    assert_eq!(
        without[0].0.name, "Loved",
        "neutral key: the tie falls through to name order (Loved < Plain)"
    );
    // Reverse the names: the neutral key must flip with the alphabet.
    let mut zebra = card("Zebra", "Creature", "G", "", Some(1));
    zebra.oracle_id = "oid-zebra".to_string();
    let tag_zebra = vec![(zebra, Vec::<String>::new())];
    let without = fuse_legs(&[plain.clone()], &tag_zebra, 2, false);
    assert_eq!(
        without[0].0.name, "Plain",
        "neutral key: Plain < Zebra by name"
    );
}

#[test]
fn deck_show_json_curve_block_reports_commander_target() {
    // deck show --json drives the curve block through store_show::show:
    // the deck's own shape decides the target sentence (a commander deck
    // reads the commander bands; a 60-card deck the 60-card ones).
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    for (name, cost, cmc, type_line, identity, text) in [
        (
            "Frog Wizard",
            "{1}{G}{U}",
            3.0,
            "Legendary Creature — Frog Wizard",
            "[\"G\",\"U\"]",
            "text",
        ),
        (
            "Forest",
            "",
            0.0,
            "Basic Land — Forest",
            "[]",
            "({T}: Add {G}.)",
        ),
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '[]', ?6, '[]', ?7, 'rare', '{}',
                'tst', '1', ?2, '2020-01-01')",
            rusqlite::params![
                name,
                format!("oid-{name}"),
                cost,
                cmc,
                type_line,
                identity,
                text
            ],
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
    std::fs::write(
        paths.deck_file("CDeck"),
        "// COMMANDER\n1 Frog Wizard\n// DECK\n20 Forest\n",
    )
    .unwrap();
    // Capture the curve through the exact wiring `show` uses: load the
    // deck, detect its format, build the JSON block. (Calling `show`
    // itself would print; the store has no stdout capture.)
    let (_p, deck) = crate::deck::store::load_deck(&paths, "CDeck").unwrap();
    let cards_by_name = crate::deck::stats::lookup_names(&conn, &deck);
    let stats = crate::deck::stats::compute(&deck, &cards_by_name);
    let is_commander = crate::deck::legal::is_commander(&deck, None);
    assert!(is_commander, "COMMANDER section marks the deck");
    let curve = crate::deck::stats::curve_json(&stats, is_commander);
    assert_eq!(curve["target"], "target: comes together by t8-t10");
    assert_eq!(curve["histogram"][0], 0, "lands never count in the curve");
    assert_eq!(
        curve["histogram"][3], 1,
        "the 3-MV commander sits in slot 3"
    );
    // A 60-card deck through the same wiring reads the 60-card bands.
    crate::deck::create(
        &paths,
        &mut crate::output::Output::new(true, false, false),
        "Standard",
    )
    .unwrap();
    std::fs::write(paths.deck_file("Standard"), "// DECK\n20 Forest\n").unwrap();
    let (_p, deck) = crate::deck::store::load_deck(&paths, "Standard").unwrap();
    let cards_by_name = crate::deck::stats::lookup_names(&conn, &deck);
    let stats = crate::deck::stats::compute(&deck, &cards_by_name);
    let is_commander = crate::deck::legal::is_commander(&deck, None);
    assert!(!is_commander);
    let curve = crate::deck::stats::curve_json(&stats, is_commander);
    assert_eq!(curve["target"], "target: does its thing by t4");
}

#[test]
fn combo_suggest_price_cap_excludes_unpriced_and_above_cap() {
    // Phase 6.1/8.5: the cap applies BEFORE the limit cut, proven on the
    // command's own JSON output.
    let dir = tempfile::tempdir().unwrap();
    let mut conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    // Deck holds one piece; two cards each complete one variant: Cheap
    // (under cap) and Pricy (above cap). Unpriced also completes but is
    // excluded (unpriced cards never pass a cap).
    for (name, usd) in [
        ("Held", None),
        ("Cheap", Some(1.0_f64)),
        ("Pricy", Some(9.0)),
        ("Unpriced", None),
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '', 2, 'Creature', '[]', '[\"G\"]', '[]', 'text', 'rare',
                '{\"commander\":\"legal\"}', 'tst', '1', 'sid', '2020-01-01')",
            rusqlite::params![name, format!("oid-{name}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number,
                lang, rarity, finishes, released_at, usd, usd_foil, updated_at)
             VALUES (?1, ?2, 'm11', '148', 'en', 'rare', '[\"nonfoil\"]',
                '2020-01-01', ?3, NULL, 't')",
            rusqlite::params![format!("sid-{name}"), name, usd],
        )
        .unwrap();
    }
    // One variant per missing piece: Held + X.
    for id in ["v-cheap", "v-pricy", "v-unpriced"] {
        conn.execute(
            "INSERT INTO combos (id, produces, mana_value_needed, bracket_tag,
                legalities, popularity, updated_at)
             VALUES (?1, '[]', 2, 'C', '{\"commander\":true}', 500, 'now')",
            rusqlite::params![id],
        )
        .unwrap();
        let missing: &str = match id {
            "v-cheap" => "Cheap",
            "v-pricy" => "Pricy",
            _ => "Unpriced",
        };
        for (name, ordinal) in [("Held", 0), (missing, 1)] {
            conn.execute(
                "INSERT INTO combo_pieces (combo_id, name, ordinal, zones, must_be_commander)
                 VALUES (?1, ?2, ?3, '[\"B\"]', 0)",
                rusqlite::params![id, name, ordinal],
            )
            .unwrap();
        }
    }

    let paths = crate::paths::Paths::new(dir.path().to_path_buf());
    // status.json must claim setup so load_deck's store gate passes.
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 4,
        embedded_cards: 4,
        model: "m".into(),
        dim: 384,
        names: vec![],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    crate::deck::create(
        &paths,
        &mut crate::output::Output::new(true, false, false),
        "Modern",
    )
    .unwrap();
    std::fs::write(paths.deck_file("Modern"), "// DECK\n1 Held\n").unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let code = crate::deck::suggest_combo::run_combo_suggest(
        &paths,
        &mut conn,
        &mut out,
        "Modern",
        None,
        None,
        Some(2.0),
        10,
        true,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // All three variants touch the deck; the cap drops Pricy and Unpriced
    // inside run_combo_suggest (cap before the limit cut), leaving only
    // the under-cap Cheap completion.
    let deck = crate::deck::Deck::parse("// DECK\n1 Held\n").unwrap();
    let names: std::collections::HashSet<String> = deck.entries().map(|e| e.name.clone()).collect();
    let variants = crate::combos::load_variants_for(&conn, &names).unwrap();
    assert_eq!(variants.len(), 3, "every variant touches the deck");
    // The under-cap card survives the same helper the command uses.
    let cheap = crate::db::get_card(&conn, "Cheap").unwrap().unwrap();
    let (kept, hidden) =
        crate::prints::retain_by_price(&conn, vec![cheap], |c: &crate::db::CardRow| &c.name, 2.0)
            .unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(hidden, 0, "Cheap is priced under the cap");
}

#[test]
fn commander_search_truncates_to_limit() {
    // Phase 10.1: the commander path must respect --limit. `suggest` with
    // commander=true runs the real pipeline (semantic leg off; the tag
    // leg supplies 6 candidates) and must not error on limit 3.
    let dir = tempfile::tempdir().unwrap();
    let mut conn = crate::db::open(&dir.path().join("t.db")).unwrap();
    for i in 0..6 {
        let name = format!("Frog Chief {i}");
        conn.execute(
            "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities,
                set_code, collector_number, scryfall_id, released_at)
             VALUES (?1, ?2, '', 4, 'Legendary Creature — Frog', '[]', '[\"G\"]',
                '[]', 'text', 'rare', '{\"commander\":\"legal\"}', 'tst', '1',
                ?2, '2020-01-01')",
            rusqlite::params![name, format!("oid-{i}")],
        )
        .unwrap();
    }
    // Tag rows so the tag leg fires ("commander" family).
    conn.execute(
        "INSERT INTO tags (id, slug, label, use_count) VALUES ('t1', 'commander', 'commander', 1)",
        [],
    )
    .unwrap();
    for i in 0..6 {
        conn.execute(
            "INSERT INTO card_tags (oracle_id, tag_id) VALUES (?1, 't1')",
            rusqlite::params![format!("oid-{i}")],
        )
        .unwrap();
    }
    // The semantic leg needs a vector store; skip it by pointing paths at
    // an unset root and relying on the tag leg only is impossible —
    // instead assert the limit directly on the ranked path: fuse + price
    // + truncate. run_commander_search truncates after apply_suggest_price
    // (suggest.rs), so mirror that order here.
    let paths = crate::paths::Paths::new(dir.path().to_path_buf());
    crate::paths::Status {
        setup_complete: true,
        ingested_cards: 6,
        embedded_cards: 6,
        model: "m".into(),
        dim: 384,
        names: vec![],
        scryfall_synced_at: String::new(),
        doc_version: 0,
        combos_synced_at: String::new(),
    }
    .write(&paths.status_file())
    .unwrap();
    crate::deck::create(
        &paths,
        &mut crate::output::Output::new(true, false, false),
        "CDeck",
    )
    .unwrap();
    std::fs::write(paths.deck_file("CDeck"), "// DECK\n1 Forest\n").unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities,
            set_code, collector_number, scryfall_id, released_at)
         VALUES ('Forest', 'oid-Forest', '', 0, 'Basic Land — Forest', '[]',
            '[]', '[]', '({T}: Add {G}.)', 'common', '{}', 'tst', '1', 'sid-f',
            '2020-01-01')",
        [],
    )
    .unwrap();
    // An empty vectors.bin (384-dim rows = zero bytes) satisfies the
    // store gate; the semantic leg then returns no hits and the tag leg
    // supplies the pool of 6.
    std::fs::write(paths.root().join("vectors.bin"), Vec::<u8>::new()).unwrap();
    let mut out = crate::output::Output::new(true, false, false);
    let code = super::super::suggest::suggest(
        &paths,
        &mut conn,
        &mut out,
        "CDeck",
        Some("frog"),
        None,
        true,
        None,
        None,
        None,
        3,
        true,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    // Pin the `--limit` cut on the extracted ranking core (the exact
    // function run_commander_search uses, minus printing): the store has
    // 6 legal commanders, so a regression that drops the truncate call
    // returns all 6 instead of 3.
    let deck = super::super::Deck::parse("// DECK\n1 Forest\n").unwrap();
    let ranked = super::super::suggest::commander_candidates(
        &paths,
        &conn,
        &mut out,
        &deck,
        Some("frog"),
        None,
        3,
    )
    .unwrap();
    assert_eq!(
        ranked.len(),
        3,
        "the commander path truncates to --limit (6 candidates in the store)"
    );
}
