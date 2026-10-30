use super::*;

#[test]
fn dry_run_report_serializes_typed_optional_blocks() {
    let report = DryRunReport {
        name: "Froggy".to_string(),
        dry_run: true,
        changes: vec![],
        cost: CostImpact {
            to_buy: report::Buylist {
                items: vec![],
                total_usd: 0.0,
            },
            freed_usd: 0.0,
            net_usd: 0.0,
        },
        sim: Some(crate::deck::simulator::report_view::ReportDiff::default()),
        legal: Some(LegalVerdict {
            legal: true,
            violations: vec![],
            advisories: vec![],
        }),
    };
    let json = serde_json::to_value(report).expect("serialize typed dry-run report");
    assert_eq!(json["dry_run"], true);
    assert_eq!(json["changes"], serde_json::json!([]));
    assert!(json["cost"].is_object());
    assert!(json["sim"]["findings"].is_array());
    assert!(json["legal"]["violations"].is_array());
}

/// Dry-run through the command surface: nothing is written and the exit
/// code is clean when no sim problems appear.
#[test]
fn update_dry_run_writes_nothing() {
    let (tmp, conn) = seeded_conn();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Froggy"), "// DECK\n1 Lightning Bolt\n").unwrap();
    let mut out = silent_out();
    let before_text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &["1 Lightning Bolt".to_string()],
        &[],
        &[],
        &[],
        None,
        false,
        true,
        false,
        false,
        false,
        false,
    )
    .unwrap();
    assert_eq!(code, crate::cli::codes::OK);
    let after_text = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    assert_eq!(before_text, after_text, "dry-run must not write the deck");
}

#[test]
fn update_dry_run_sim_json_runs_the_typed_report_diff() {
    let (tmp, conn) = seeded_conn();
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(paths.deck_file("Froggy"), "// DECK\n1 Lightning Bolt\n").unwrap();
    let before = std::fs::read_to_string(paths.deck_file("Froggy")).unwrap();
    let mut out = silent_out();

    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &["1 Lightning Bolt".to_string()],
        &[],
        &[],
        &[],
        None,
        false,
        true,
        true,
        false,
        false,
        true,
    )
    .unwrap();

    assert_eq!(code, crate::cli::codes::OK);
    assert_eq!(
        std::fs::read_to_string(paths.deck_file("Froggy")).unwrap(),
        before,
        "dry-run JSON must not write the deck"
    );
}

#[test]
fn dry_run_legal_flags_violation() {
    // Adding a second copy in a commander deck trips the singleton rule
    // in the --legal verdict, and the preview exits 1.
    let (tmp, conn) = seeded_conn();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Breya', 'o1', '', 0, 'Legendary Creature — Human', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's1', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    for i in 0..10 {
        conn.execute(
            &format!(
                "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                    keywords, oracle_text, rarity, legalities, set_code, collector_number,
                    scryfall_id, released_at, game_changer, edhrec_rank)
                 VALUES ('Island{i}', 'o3-{i}', '', 0, 'Basic Land — Island', '[]', '[]', '[]', '',
                    'common', '{{\"commander\":\"legal\"}}', 'tst', '1', 's3-{i}', '2020-01-01', NULL, NULL)"
            ),
            [],
        )
        .unwrap();
    }
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    std::fs::write(
        paths.deck_file("Froggy"),
        "// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n10 Island\n",
    )
    .unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Froggy",
        &[],
        &[],
        &["2 Bolt".to_string()],
        &[],
        None,
        false,
        true,
        false,
        true,
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        code,
        crate::cli::codes::ERROR,
        "illegal post-change deck fails the preview"
    );
}

#[test]
fn dry_run_legal_uses_constructed_rules_for_60_card_decks() {
    // A 60-card-shaped deck must not be judged by commander rules in the
    // --legal verdict: singleton and exact-100 checks would reject it.
    let (tmp, conn) = seeded_conn();
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
            keywords, oracle_text, rarity, legalities, set_code, collector_number,
            scryfall_id, released_at, game_changer, edhrec_rank)
         VALUES ('Bolt', 'o2', '{1}{R}', 1, 'Instant', '[]', '[]', '[]', '',
            'common', '{\"modern\":\"legal\",\"commander\":\"legal\"}', 'tst', '1', 's2', '2020-01-01', NULL, NULL)",
        [],
    )
    .unwrap();
    for i in 0..56 {
        conn.execute(
            &format!(
                "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors, color_identity,
                    keywords, oracle_text, rarity, legalities, set_code, collector_number,
                    scryfall_id, released_at, game_changer, edhrec_rank)
                 VALUES ('Island{i}', 'o3-{i}', '', 0, 'Basic Land — Island', '[]', '[]', '[]', '',
                    'common', '{{\"modern\":\"legal\"}}', 'tst', '1', 's3-{i}', '2020-01-01', NULL, NULL)"
            ),
            [],
        )
        .unwrap();
    }
    let paths = crate::paths::Paths::resolve(Some(tmp.path().join("data").as_path())).unwrap();
    std::fs::create_dir_all(paths.decks_dir()).unwrap();
    let mut deck_text = String::from("// DECK\n3 Bolt\n");
    for i in 0..56 {
        deck_text.push_str(&format!("1 Island{i}\n"));
    }
    std::fs::write(paths.deck_file("Storm"), deck_text).unwrap();
    let mut out = silent_out();
    let code = update(
        &paths,
        &conn,
        &mut out,
        "Storm",
        &["1 Bolt".to_string()],
        &[],
        &[],
        &[],
        None,
        false,
        true,
        false,
        true,
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        code,
        crate::cli::codes::OK,
        "a 59-card constructed deck adding its 60th card passes the preview"
    );
}
