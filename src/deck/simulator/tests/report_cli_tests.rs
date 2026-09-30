//! Tests for the report CLI entry point.
use super::*;

fn setup_one_land_deck() -> (tempfile::TempDir, crate::paths::Paths, rusqlite::Connection) {
    let temp = tempfile::tempdir().expect("temporary store");
    let data_dir = temp.path().join("data");
    let paths = crate::paths::Paths::resolve(Some(&data_dir)).expect("resolve paths");
    std::fs::create_dir_all(paths.decks_dir()).expect("create deck directory");
    std::fs::write(paths.deck_file("One land"), "// DECK\n1 Island\n").expect("write deck file");
    let mut status = crate::paths::Status::empty();
    status.setup_complete = true;
    status.write(&paths.status_file()).expect("write status");

    let conn = crate::db::open(&paths.db()).expect("open database");
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
            color_identity, keywords, oracle_text, rarity, legalities, set_code,
            collector_number, scryfall_id, released_at)
         VALUES ('Island', 'island', '', 0, 'Basic Land — Island', '[]', '[]',
            '[]', '', 'common', '{\"commander\":\"legal\"}', 'tst', '1',
            'island-print', '2020-01-01')",
        [],
    )
    .expect("insert Island");
    (temp, paths, conn)
}

fn simulate_one_land(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    baseline: Option<&std::path::Path>,
) -> i32 {
    let mut output = crate::output::Output::new(true, false, false);
    simulate(
        paths,
        conn,
        &mut output,
        "One land",
        8,
        Some(4),
        Some(17),
        None,
        baseline,
        false,
        Vec::new(),
        None,
        None,
        true,
    )
    .expect("simulate deck")
}

#[test]
fn simulate_json_exit_codes_use_findings_and_typed_baselines() {
    let (temp, paths, conn) = setup_one_land_deck();
    let deck = crate::deck::store::load_deck(&paths, "One land")
        .expect("load deck")
        .1;
    let cards = crate::deck::stats::lookup_names(&conn, &deck).expect("load cards");
    let mut baseline = sim_report_for(&deck, &cards, "One land", 8, Some(4), 17, None)
        .expect("build complete report");
    assert!(
        baseline
            .findings
            .iter()
            .any(|finding| finding.kind == "insufficient_land_drops")
    );
    let baseline_path = temp.path().join("baseline.json");
    std::fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).expect("serialize baseline"),
    )
    .expect("write baseline");

    assert_eq!(
        simulate_one_land(&paths, &conn, None),
        crate::cli::codes::ERROR,
        "normal simulation fails when it reports a finding"
    );
    assert_eq!(
        simulate_one_land(&paths, &conn, Some(&baseline_path)),
        crate::cli::codes::OK,
        "an existing baseline finding does not fail the comparison"
    );

    baseline.findings.clear();
    std::fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).expect("serialize baseline"),
    )
    .expect("write baseline");
    assert_eq!(
        simulate_one_land(&paths, &conn, Some(&baseline_path)),
        crate::cli::codes::ERROR,
        "a newly introduced finding fails the comparison"
    );
}
