//! Command output checks against isolated, synthetic stores. No network is used.

use clap::Parser;
use seizethemana::paths::{Paths, Status};
use std::process::{Command, Output};

/// Seed enough local data to render cards and collection summaries without a model.
fn seeded_store() -> (tempfile::TempDir, Paths) {
    let temp = tempfile::tempdir().expect("temporary store");
    let paths = Paths::new(temp.path().join("a long data directory with spaces"));
    paths.ensure_dirs().expect("store directories");
    std::fs::create_dir_all(paths.decks_dir()).expect("deck directory");
    let conn = seizethemana::db::open(&paths.db()).expect("database");
    conn.execute(
        "INSERT INTO cards (name, oracle_id, mana_cost, type_line, oracle_text, legalities, rarity, set_code, collector_number, released_at)
         VALUES (?1, 'synthetic', '{1}{U}', 'Creature — Test', 'Draw a card.\nSource wording stays exact.', '{\"vintage\":\"restricted\",\"modern\":\"legal\"}', 'uncommon', 'tst', '1', '2020-01-01')",
        ["Jötun 漢字 e\u{301} // A long synthetic card name"],
    ).expect("synthetic card");
    conn.execute(
        "INSERT INTO collection (name, set_code, collector_number, binder, binder_type, quantity)
         VALUES (?1, 'tst', '1', 'Long binder name', 'binder', 2)",
        ["Jötun 漢字 e\u{301} // A long synthetic card name"],
    )
    .expect("synthetic holding");
    let mut status = Status::empty();
    status.setup_complete = true;
    status.write(&paths.status_file()).expect("setup marker");
    (temp, paths)
}

/// Run the real command with both streams redirected and refreshes disabled.
fn run(paths: &Paths, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stm"))
        .arg("--data-dir")
        .arg(paths.root())
        .args(["--offline", "--no-color"])
        .args(args)
        .env("TERM", "dumb")
        .output()
        .expect("run stm")
}

/// Decode output while proving that redirected streams contain no terminal escapes.
fn text(bytes: &[u8]) -> String {
    assert!(
        !bytes.contains(&0x1b),
        "redirected output contains an escape"
    );
    String::from_utf8(bytes.to_vec()).expect("UTF-8 output")
}

/// Keep an old connection open across a completed synthetic forced reset.
fn replaced_query_store() -> (tempfile::TempDir, Paths, rusqlite::Connection) {
    let temp = tempfile::tempdir().expect("reset store");
    let paths = Paths::new(temp.path().to_path_buf());
    let conn = seizethemana::db::open(&paths.db()).expect("old database");
    conn.execute("INSERT INTO cards (name, oracle_id, oracle_text) VALUES ('Card', 'card', 'needle revision one')", []).expect("old card");
    conn.execute_batch("INSERT INTO collection (name, binder, binder_type, quantity) VALUES ('Card', 'Old', 'binder', 7);
        INSERT INTO card_prints (scryfall_id, name, set_code, collector_number, lang, rarity, released_at, usd, updated_at)
        VALUES ('card', 'Card', 'tst', '1', 'en', 'common', '2020-01-01', 1, 'seed');").expect("old scope data");
    for suffix in ["", "-wal", "-shm"] {
        let path = std::path::PathBuf::from(format!("{}{suffix}", paths.db().display()));
        if path.exists() {
            std::fs::remove_file(path).expect("reset old database");
        }
    }
    let replacement = seizethemana::db::open(&paths.db()).expect("new database");
    replacement.execute("INSERT INTO cards (name, oracle_id, oracle_text) VALUES ('Card', 'card', 'needle revision two')", []).expect("new card");
    replacement.execute_batch("INSERT INTO collection (name, binder, binder_type, quantity) VALUES ('Card', 'New', 'binder', 3);
        INSERT INTO card_prints (scryfall_id, name, set_code, collector_number, lang, rarity, released_at, usd, updated_at)
        VALUES ('card', 'Card', 'tst', '1', 'en', 'common', '2020-01-01', 4, 'seed');").expect("new scope data");
    let store = seizethemana::embed::VectorStore::new();
    store
        .save_vectors(paths.root())
        .expect("empty vector index");
    let mut status = Status::empty();
    status.setup_complete = true;
    status.dim = seizethemana::embed::DIM;
    status.model = seizethemana::embed::MODEL_NAME.to_string();
    status.doc_version = seizethemana::embed::DOC_VERSION;
    status
        .write(&paths.status_file())
        .expect("new setup status");
    assert_eq!(
        seizethemana::db::load_all_cards(&conn).expect("stale connection")[0].oracle_text,
        "needle revision one"
    );
    (temp, paths, conn)
}

/// A completed forced reset must replace SQL rows for an already-open reader.
#[test]
fn ranked_query_reattaches_after_database_inode_replacement() {
    let (_temp, paths, conn) = replaced_query_store();
    let hits = seizethemana::query::run_search(
        &paths,
        &conn,
        &seizethemana::output::Output::new(true, true, false),
        "needle",
        &seizethemana::search::CardFilters::default(),
        10,
        None,
    )
    .expect("ranked query without a model");
    assert_eq!(hits[0].card.oracle_text, "needle revision two");
}

/// CLI price and ownership filters must use the same replacement as ranked rows.
#[test]
fn ranked_cli_scopes_reattach_after_database_inode_replacement() {
    let (_temp, paths, mut conn) = replaced_query_store();
    let mut out = seizethemana::output::Output::new(true, true, false);
    let filters = seizethemana::cli::CardFilters::default();
    let code = seizethemana::query::run_query(
        &paths,
        &mut conn,
        &mut out,
        "needle",
        &filters,
        Some(2.0),
        10,
        true,
    )
    .expect("new price filter");
    assert_eq!(
        code,
        seizethemana::cli::codes::NO_RESULTS,
        "the replacement price is above the cap"
    );
    for (binder, expected) in [
        ("New", seizethemana::cli::codes::OK),
        ("Old", seizethemana::cli::codes::NO_RESULTS),
    ] {
        let code = seizethemana::collection::run_query(
            &paths,
            &mut conn,
            &mut out,
            "needle",
            &filters,
            &[binder.to_string()],
            &[],
            10,
            true,
        )
        .expect("new ownership scope");
        assert_eq!(code, expected, "binder {binder}");
    }
}

/// Card prose distinguishes unknown prices and restricted formats from ordinary legality.
#[test]
fn seeded_card_text_and_json_keep_source_data() {
    let (_temp, paths) = seeded_store();
    let name = "Jötun 漢字 e\u{301} // A long synthetic card name";
    let human = run(&paths, &["card", name]);
    assert!(human.status.success(), "{}", text(&human.stderr));
    assert!(human.stderr.is_empty());
    let output = text(&human.stdout);
    for expected in [
        name,
        "Price: unpriced",
        "vintage (restricted)",
        "Source wording stays exact.",
    ] {
        assert!(output.contains(expected), "missing {expected:?}: {output}");
    }
    let json = run(&paths, &["card", name, "--json"]);
    assert!(json.status.success());
    assert!(json.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("card JSON");
    assert_eq!(value["name"], name);
    assert_eq!(
        value["oracle_text"],
        "Draw a card.\nSource wording stays exact."
    );
    assert_eq!(value["legalities"]["vintage"], "restricted");
    assert!(value["price"].is_null());
}

/// Collection totals state their price scope, rather than treating unpriced cards as free.
#[test]
fn seeded_collection_summary_states_price_scope() {
    let (_temp, paths) = seeded_store();
    let result = run(&paths, &["collection"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(result.stderr.is_empty());
    let output = text(&result.stdout);
    assert!(output.contains("Collection\nCard names: 1 · Copies: 2 · Foil or etched: 0"));
    let prose = output.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(prose.contains("Market value uses stored prices for English printings."));
    assert!(prose.contains("The total excludes unpriced copies and selling costs."));
    assert!(output.contains("Long binder name"));
}

/// A collection held only in decks is not an empty collection.
#[test]
fn deck_only_collection_search_explains_the_binder_default() {
    let (_temp, paths) = seeded_store();
    let conn = seizethemana::db::open(&paths.db()).expect("database");
    conn.execute(
        "UPDATE collection SET binder_type = 'deck', binder = 'Assigned'",
        [],
    )
    .expect("deck-only holdings");
    let result = run(&paths, &["collection", "query"]);
    assert_eq!(result.status.code(), Some(3));
    assert!(result.stdout.is_empty());
    let diagnostic = text(&result.stderr);
    assert!(diagnostic.contains("no binder cards found"));
    assert!(diagnostic.contains("--deck <name>"));
    assert!(!diagnostic.contains("collection is empty"));
    let included = run(
        &paths,
        &["collection", "query", "--deck", "Assigned", "--json"],
    );
    assert!(included.status.success());
    assert!(included.stderr.is_empty());
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&included.stdout).expect("scoped JSON");
    assert_eq!(rows[0]["owned"], 2);
}

/// Empty states and missing cards preserve exit codes and stream separation.
#[test]
fn empty_decks_and_unknown_card_wording_snapshot() {
    let (_temp, paths) = seeded_store();
    let empty = run(&paths, &["deck", "list"]);
    assert_eq!(empty.status.code(), Some(3));
    assert!(empty.stdout.is_empty());
    assert_eq!(
        text(&empty.stderr),
        "error: no saved decklists or deck assignments found\nhint: create one: stm deck create <name>, or import: stm deck import <name> <file>\n"
    );
    let unknown = run(&paths, &["card", "Missing", "--json"]);
    assert_eq!(unknown.status.code(), Some(3));
    assert!(unknown.stdout.is_empty());
    assert_eq!(
        text(&unknown.stderr),
        "error: no card named \"Missing\"\nhint: use the full card name or a unique prefix; case does not matter\n"
    );
    let json = run(&paths, &["deck", "list", "--json"]);
    assert!(json.status.success());
    assert_eq!(text(&json.stdout), "[]\n");
    assert!(json.stderr.is_empty());
}

/// Status remains on stderr and retains the full path, including spaces.
#[test]
fn create_status_preserves_the_full_path() {
    let (_temp, paths) = seeded_store();
    let result = run(&paths, &["deck", "create", "Seeded deck"]);
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    let status = text(&result.stderr);
    assert!(status.starts_with("     Created deck \"Seeded deck\" ("));
    assert!(status.contains(&paths.deck_file("Seeded deck").display().to_string()));
    assert!(status.contains("primer"));
}

/// Help describes command-result exits and the files changed by deck writes.
#[test]
fn help_explains_finding_exits_and_deck_file_changes() {
    let (_temp, paths) = seeded_store();
    for (args, expected) in [
        (
            vec!["--help"],
            "runtime error, legality violations, or simulation findings",
        ),
        (
            vec!["deck", "delete", "--help"],
            "Delete a decklist and its primer (collection ownership is kept)",
        ),
        (
            vec!["deck", "copy", "--help"],
            "Replace its primer only if the source has one; otherwise keep it",
        ),
        (
            vec!["deck", "show", "--help"],
            "Deck name; omit to list saved decklists",
        ),
    ] {
        let result = run(&paths, &args);
        assert!(result.status.success(), "{}", text(&result.stderr));
        assert!(result.stderr.is_empty());
        let help = text(&result.stdout)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(help.contains(expected), "{help}");
    }
}

/// Conflicting diff modes fail during parsing instead of writing the wrong format.
#[test]
fn diff_output_modes_are_exclusive() {
    let (_temp, paths) = seeded_store();
    for pair in [
        ["--json", "--as-update"],
        ["--as-update", "--json"],
        ["--markdown", "--as-update"],
        ["--as-update", "--markdown"],
        ["--json", "--markdown"],
        ["--markdown", "--json"],
    ] {
        let result = run(
            &paths,
            &["deck", "diff", "Base", "Target", pair[0], pair[1]],
        );
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        assert!(text(&result.stderr).contains("cannot be used with"));
    }
    for mode in ["--json", "--markdown", "--as-update"] {
        assert!(
            seizethemana::cli::Cli::try_parse_from(
                ["stm", "deck", "diff", "Base", "Target", mode,]
            )
            .is_ok()
        );
    }
}

/// Ranked search can show human progress without a model or machine-stream leaks.
#[test]
fn ranked_query_progress_uses_only_human_stderr() {
    let (_temp, paths, _old_connection) = replaced_query_store();
    let human = run(&paths, &["query", "needle"]);
    assert!(human.status.success(), "{}", text(&human.stderr));
    assert!(text(&human.stderr).contains("Searching cards (preparing text search)"));
    assert!(text(&human.stdout).contains("Card"));
    assert!(!text(&human.stdout).contains("Searching"));
    let json = run(&paths, &["query", "needle", "--json"]);
    assert!(json.status.success(), "{}", text(&json.stderr));
    assert!(json.stderr.is_empty());
    let hits: Vec<serde_json::Value> =
        serde_json::from_slice(&json.stdout).expect("ranked query JSON");
    assert_eq!(hits[0]["name"], "Card");
    assert_eq!(
        std::fs::read_dir(paths.models_dir())
            .expect("model directory")
            .count(),
        0
    );
}

/// Owned ranked search follows the same progress and JSON stream policy.
#[test]
fn collection_query_progress_stays_out_of_results() {
    let (_temp, paths, _old_connection) = replaced_query_store();
    let human = run(&paths, &["collection", "query", "needle"]);
    assert!(human.status.success(), "{}", text(&human.stderr));
    assert!(text(&human.stderr).contains("Searching owned cards"));
    assert!(text(&human.stdout).contains("Card"));
    assert!(!text(&human.stdout).contains("Searching"));
    let json = run(&paths, &["collection", "query", "needle", "--json"]);
    assert!(json.status.success(), "{}", text(&json.stderr));
    assert!(json.stderr.is_empty());
    let hits: Vec<serde_json::Value> =
        serde_json::from_slice(&json.stdout).expect("owned query JSON");
    assert_eq!(hits[0]["name"], "Card");
    assert_eq!(hits[0]["owned"], 3);
}

/// Simulation output keeps seeded reports stable across partial batch boundaries.
#[test]
fn simulation_progress_preserves_seeded_reports_and_json_streams() {
    let (_temp, paths) = seeded_store();
    let conn = seizethemana::db::open(&paths.db()).expect("simulation database");
    conn.execute(
        "INSERT INTO cards (name, oracle_id, type_line, oracle_text, color_identity)
         VALUES ('Island', 'island', 'Basic Land — Island', '{T}: Add {U}.', '[\"U\"]')",
        [],
    )
    .expect("synthetic basic land");
    std::fs::write(paths.deck_file("Simulation"), "// DECK\n60 Island\n").expect("simulation deck");
    let args = [
        "deck",
        "simulate",
        "Simulation",
        "--runs",
        "100",
        "--turns",
        "1",
        "--seed",
        "42",
        "--threads",
        "1",
    ];
    let human = run(&paths, &args);
    assert!(
        matches!(human.status.code(), Some(0..=1)),
        "{}",
        text(&human.stderr)
    );
    assert!(text(&human.stderr).contains("Simulating 100 games"));
    assert!(text(&human.stderr).contains("seed 42"));
    assert!(!text(&human.stdout).contains("Simulating"));
    let mut reports = Vec::new();
    for batch_size in ["7", "13"] {
        let mut json_args = args.to_vec();
        json_args.extend(["--batch-size", batch_size, "--json"]);
        let json = run(&paths, &json_args);
        assert_eq!(json.status.code(), human.status.code());
        assert!(json.stderr.is_empty(), "{}", text(&json.stderr));
        reports.push(
            serde_json::from_slice::<serde_json::Value>(&json.stdout).expect("simulation JSON"),
        );
    }
    assert_eq!(reports[0], reports[1]);
}

/// Seed database filters with no setup status, model files, or usable vectors.
fn filter_store() -> (tempfile::TempDir, Paths) {
    let temp = tempfile::tempdir().expect("filter store");
    let paths = Paths::new(temp.path().to_path_buf());
    let conn = seizethemana::db::open(&paths.db()).expect("filter database");
    for (name, type_line, colors, legality, price) in [
        (
            "alpha Elf",
            "Creature — Elf Druid",
            "[\"G\"]",
            "legal",
            Some(2.0),
        ),
        (
            "Beta Elf",
            "Creature — Elf Warrior",
            "[\"G\"]",
            "legal",
            Some(4.0),
        ),
        (
            "Delta Elf",
            "Creature — Elf",
            "[\"G\"]",
            "banned",
            Some(1.0),
        ),
        ("Red Elf", "Creature — Elf", "[\"R\"]", "legal", Some(1.0)),
        (
            "Gamma Wizard",
            "Creature — Wizard",
            "[\"G\"]",
            "legal",
            Some(1.0),
        ),
        ("Unpriced Elf", "Creature — Elf", "[\"G\"]", "legal", None),
        ("Deck Elf", "Creature — Elf", "[\"G\"]", "legal", Some(9.0)),
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id, type_line, colors, color_identity, cmc, rarity, set_code, legalities)
             VALUES (?1, ?1, ?2, ?3, ?3, 2, 'common', 'original', ?4)",
            rusqlite::params![name, type_line, colors, format!("{{\"modern\":\"{legality}\"}}")],
        ).expect("filter card");
        conn.execute(
            "INSERT INTO card_prints (scryfall_id, name, set_code, collector_number, lang, rarity, released_at, usd, updated_at)
             VALUES (?1, ?1, 'alt', '1', 'en', 'common', '2020-01-01', ?2, 'seed')",
            rusqlite::params![name, price],
        ).expect("alternate set printing");
    }
    for (name, binder, kind, quantity) in [
        ("alpha Elf", "Play", "binder", 2),
        ("alpha Elf", "Wanted", "deck", 3),
        ("alpha Elf", "Unrequested", "deck", 9),
        ("Beta Elf", "Other", "binder", 4),
        ("Deck Elf", "Wanted", "deck", 1),
        ("Unpriced Elf", "Play", "binder", 1),
    ] {
        conn.execute("INSERT INTO collection (name, set_code, collector_number, binder, binder_type, quantity) VALUES (?1, 'alt', '1', ?2, ?3, ?4)", rusqlite::params![name, binder, kind, quantity]).expect("scoped holding");
    }
    std::fs::write(paths.vectors_file(), b"not a vector index").expect("unusable index");
    (temp, paths)
}

/// Leave refresh enabled to prove database-only commands never start a sync.
fn filter_command(paths: &Paths, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stm"))
        .arg("--data-dir")
        .arg(paths.root())
        .arg("--no-color")
        .args(args)
        .env("TERM", "dumb")
        .output()
        .expect("filter command")
}

/// Read a successful database-only JSON result with no status or progress output.
fn filter_rows(paths: &Paths, args: &[&str]) -> Vec<serde_json::Value> {
    let result = filter_command(paths, args);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(result.stderr.is_empty(), "{}", text(&result.stderr));
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&result.stdout).expect("query JSON");
    assert!(rows.iter().all(|row| row["score"].as_f64() == Some(0.0)));
    assert!(
        !paths.status_file().exists(),
        "a listing must not create setup state"
    );
    assert!(
        !paths.root().join("store.lock").exists(),
        "a listing must skip the vector snapshot lock"
    );
    assert_eq!(
        std::fs::read_dir(paths.models_dir())
            .expect("empty model directory")
            .count(),
        0
    );
    rows
}

/// Subtype, stored set, colors, format, mana value, and price compose before limit.
#[test]
fn filter_only_global_query_uses_all_filters_without_an_index() {
    let (_temp, paths) = filter_store();
    let args = [
        "query",
        "--type",
        "Elf",
        "--set",
        "ORIGINAL",
        "--color",
        "G",
        "--color-identity",
        "G",
        "--format",
        "modern",
        "--cmc",
        "<=2",
        "--max-price",
        "5",
        "--json",
    ];
    let rows = filter_rows(&paths, &args);
    assert_eq!(
        rows.iter()
            .map(|row| row["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        ["alpha Elf", "Beta Elf"]
    );
    assert_eq!(
        rows[0]["set"], "original",
        "the card's source printing stays unchanged"
    );
    let limited = filter_rows(
        &paths,
        &[
            "query", "--type", "Elf", "--set", "original", "--limit", "1", "--json",
        ],
    );
    assert_eq!(limited[0]["name"], "alpha Elf");
    assert_eq!(limited.len(), 1);
    let human = filter_command(&paths, &["query", "  ", "--type", "Elf", "--limit", "1"]);
    assert!(human.status.success());
    assert!(human.stderr.is_empty());
    assert!(!text(&human.stdout).contains("(0.000)"));
    assert!(text(&human.stdout).contains("alpha Elf"));
}

/// Owned lists keep the existing binder pool plus explicitly requested deck rows.
#[test]
fn filter_only_collection_query_preserves_scope_counts_and_locations() {
    let (_temp, paths) = filter_store();
    let rows = filter_rows(
        &paths,
        &[
            "collection",
            "query",
            "--type",
            "Elf",
            "--set",
            "original",
            "--json",
        ],
    );
    assert_eq!(
        rows.iter()
            .map(|row| row["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        ["alpha Elf", "Beta Elf", "Unpriced Elf"]
    );
    assert_eq!(rows[0]["owned"], 2);
    let scoped = filter_rows(
        &paths,
        &[
            "collection",
            "query",
            "--binder",
            "Play",
            "--deck",
            "Wanted",
            "--type",
            "Elf",
            "--json",
        ],
    );
    assert_eq!(
        scoped
            .iter()
            .map(|row| row["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        ["alpha Elf", "Deck Elf", "Unpriced Elf"]
    );
    assert_eq!(scoped[0]["owned"], 5);
    assert_eq!(
        scoped[0]["locations"].as_array().expect("locations").len(),
        2
    );
    assert_eq!(scoped[1]["owned"], 1);
    let limited = filter_rows(&paths, &["collection", "query", "--limit", "1", "--json"]);
    assert_eq!(limited[0]["name"], "alpha Elf");
    let blank = filter_rows(&paths, &["collection", "query", "  ", "--json"]);
    assert_eq!(blank.len(), 3);
}

/// Bare listings retain the default limit and empty JSON remains a valid array.
#[test]
fn filter_only_bare_defaults_and_empty_json_are_deterministic() {
    let (_temp, paths) = filter_store();
    let conn = seizethemana::db::open(&paths.db()).expect("database");
    for index in 0..25 {
        let name = format!("Listed {index:02}");
        conn.execute(
            "INSERT INTO cards (name, oracle_id) VALUES (?1, ?1)",
            [&name],
        )
        .expect("extra card");
        conn.execute("INSERT INTO collection (name, binder, binder_type, quantity) VALUES (?1, 'Other', 'binder', 1)", [&name]).expect("extra holding");
    }
    for prefix in [vec!["query"], vec!["collection", "query"]] {
        let mut args = prefix.clone();
        args.push("--json");
        let rows = filter_rows(&paths, &args);
        assert_eq!(rows.len(), 20);
        let names: Vec<_> = rows
            .iter()
            .map(|row| row["name"].as_str().expect("name").to_ascii_lowercase())
            .collect();
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        let mut empty = prefix;
        empty.extend(["--type", "Impossible type", "--json"]);
        let result = filter_command(&paths, &empty);
        assert_eq!(result.status.code(), Some(3));
        assert_eq!(text(&result.stdout), "[]\n");
        assert!(result.stderr.is_empty());
    }
    let empty_scope = filter_command(
        &paths,
        &["collection", "query", "--binder", "Missing", "--json"],
    );
    assert_eq!(empty_scope.status.code(), Some(3));
    assert_eq!(text(&empty_scope.stdout), "[]\n");
    assert!(empty_scope.stderr.is_empty());
}

/// Nonblank text keeps the ranked path instead of silently listing by name.
#[test]
fn text_queries_still_require_the_ranked_search_index() {
    let (_temp, paths) = filter_store();
    for args in [
        vec!["query", "Elf", "--json"],
        vec!["collection", "query", "Elf", "--json"],
    ] {
        let result = run(&paths, &args);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert!(text(&result.stderr).contains("card index not built yet"));
    }
}

/// Nonblank queries retain vector/word ranking instead of alphabetical order.
#[test]
fn ranked_text_search_keeps_scores_and_primary_print_set_filters() {
    let (_temp, paths) = filter_store();
    let conn = seizethemana::db::open(&paths.db()).expect("database");
    conn.execute(
        "UPDATE cards SET edhrec_rank = 1 WHERE name = 'Beta Elf'",
        [],
    )
    .expect("rank tie breaker");
    let cards = seizethemana::db::load_all_cards(&conn).expect("cards");
    let mut vectors = seizethemana::embed::VectorStore::new();
    for card in &cards {
        let mut vector = vec![0.0; seizethemana::embed::DIM];
        vector[usize::from(card.name != "Beta Elf")] = 1.0;
        vectors.push(&card.name, vector).expect("synthetic vector");
    }
    let mut query = vec![0.0; seizethemana::embed::DIM];
    query[0] = 1.0;
    let filters = seizethemana::search::CardFilters {
        type_: Some("Elf".to_string()),
        set: Some("ORIGINAL".to_string()),
        ..Default::default()
    };
    let results = seizethemana::query::search_with_vectors(
        &conn, &cards, &vectors, &query, "Beta", &filters, 2, None,
    )
    .expect("ranked search");
    assert_eq!(results.vector[0].card.name, "Beta Elf");
    assert_eq!(results.vector[0].score, 1.0);
    assert_eq!(results.fts[0].card.name, "Beta Elf");
    assert_eq!(results.hybrid[0].card.name, "Beta Elf");
    assert!(results.hybrid[0].score > 0.0);
    assert_eq!(results.hybrid.len(), 2);
    let other_print = seizethemana::search::CardFilters {
        set: Some("ALT".to_string()),
        ..filters
    };
    let excluded = seizethemana::query::search_with_vectors(
        &conn,
        &cards,
        &vectors,
        &query,
        "Beta",
        &other_print,
        2,
        None,
    )
    .expect("primary-set restriction");
    assert!(excluded.fts.is_empty());
    assert!(excluded.vector.is_empty());
    assert!(excluded.hybrid.is_empty());
}

/// Alternate and owned printing records do not change a card's stored set filter.
#[test]
fn filter_only_set_uses_primary_card_data_and_name_based_owned_scope() {
    let (_temp, paths) = filter_store();
    for args in [
        vec!["query", "--set", "ALT", "--json"],
        vec!["collection", "query", "--set", "ALT", "--json"],
    ] {
        let empty = filter_command(&paths, &args);
        assert_eq!(empty.status.code(), Some(3));
        assert_eq!(text(&empty.stdout), "[]\n");
        assert!(empty.stderr.is_empty());
    }
    let conn = seizethemana::db::open(&paths.db()).expect("database");
    conn.execute("INSERT INTO cards (name, oracle_id, type_line, set_code) VALUES ('Catalog ALT', 'catalog-alt', 'Creature — Elf', 'alt')", []).expect("unowned primary-set card");
    let global = filter_rows(&paths, &["query", "--set", "ALT", "--json"]);
    assert_eq!(global.len(), 1);
    assert_eq!(global[0]["name"], "Catalog ALT");
    let owned = filter_rows(
        &paths,
        &[
            "collection",
            "query",
            "--set",
            "ORIGINAL",
            "--binder",
            "Play",
            "--json",
        ],
    );
    assert_eq!(
        owned
            .iter()
            .map(|row| row["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        ["alpha Elf", "Unpriced Elf"]
    );
    assert_eq!(
        owned[0]["owned"], 2,
        "owned ALT copies still qualify by card name"
    );
    let unowned = filter_command(&paths, &["collection", "query", "--set", "ALT", "--json"]);
    assert_eq!(unowned.status.code(), Some(3));
    assert_eq!(text(&unowned.stdout), "[]\n");
    assert!(unowned.stderr.is_empty());
}

/// Unicode lowercase order uses exact stored names as ties without normalization.
#[test]
fn filter_only_order_keeps_unicode_names_and_case_ties() {
    let temp = tempfile::tempdir().expect("Unicode store");
    let paths = Paths::new(temp.path().to_path_buf());
    let conn = seizethemana::db::open(&paths.db()).expect("database");
    for name in [
        "åland",
        "Zulu",
        "alpha",
        "ALPHA",
        "Élan",
        "élan",
        "e\u{301}lan",
        "İbis",
        "ibis",
        "漢字",
        "长名字",
        "Straße",
        "STRASSE",
        "BETA",
        "beta",
        "Åland",
    ] {
        conn.execute(
            "INSERT INTO cards (name, oracle_id) VALUES (?1, ?1)",
            [name],
        )
        .expect("distinct card identity");
        conn.execute("INSERT INTO collection (name, binder, binder_type, quantity) VALUES (?1, 'Names', 'binder', 1)", [name]).expect("owned name");
    }
    let expected = [
        "ALPHA",
        "alpha",
        "BETA",
        "beta",
        "e\u{301}lan",
        "ibis",
        "İbis",
        "STRASSE",
        "Straße",
        "Zulu",
        "Åland",
        "åland",
        "Élan",
        "élan",
        "漢字",
        "长名字",
    ];
    for args in [
        vec!["query", "--limit", "100", "--json"],
        vec!["collection", "query", "--limit", "100", "--json"],
    ] {
        let rows = filter_rows(&paths, &args);
        let names = rows
            .iter()
            .map(|row| row["name"].as_str().expect("name"))
            .collect::<Vec<_>>();
        assert_eq!(names, expected);
        assert_eq!(
            names.iter().collect::<std::collections::HashSet<_>>().len(),
            expected.len()
        );
    }
}

/// Similar-card ownership is binder-only, even when deck assignments also exist.
#[test]
fn similar_owned_scope_excludes_deck_only_cards() {
    let (_temp, paths) = filter_store();
    let conn = seizethemana::db::open(&paths.db()).expect("database");
    for name in ["Gamma Wizard", "alpha Elf", "Deck Elf"] {
        conn.execute(
            "INSERT INTO card_tags (oracle_id, tag_id) VALUES (?1, 'shared')",
            [name],
        )
        .expect("shared tag");
    }
    conn.execute("INSERT INTO tags (id, slug, label, use_count) VALUES ('shared', 'shared', 'shared role', 3)", []).expect("tag label");
    let mut status = Status::empty();
    status.setup_complete = true;
    status.write(&paths.status_file()).expect("setup marker");
    let result = run(
        &paths,
        &["card", "similar", "Gamma Wizard", "--owned", "--json"],
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(result.stderr.is_empty());
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&result.stdout).expect("similar JSON");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "alpha Elf");
    let human = run(&paths, &["card", "similar", "Gamma Wizard", "--owned"]);
    assert!(human.status.success());
    assert!(
        text(&human.stdout)
            .contains("owned scope: binder copies only; deck-assigned copies are excluded")
    );
    conn.execute("DELETE FROM collection WHERE binder_type = 'binder'", [])
        .expect("remove temporary binder holdings");
    let empty = run(&paths, &["card", "similar", "Gamma Wizard", "--owned"]);
    assert_eq!(empty.status.code(), Some(3));
    assert!(text(&empty.stderr).contains("no owned binder cards found"));
    assert!(text(&empty.stderr).contains("deck-assigned copies are excluded"));
}
