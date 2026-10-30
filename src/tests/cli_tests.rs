use super::*;
use clap::CommandFactory;

/// Simulation rejects invalid worker counts and contradictory opponent settings.
#[test]
fn simulation_execution_flags_are_validated() {
    let cli = Cli::try_parse_from([
        "stm",
        "deck",
        "simulate",
        "Lathril",
        "--threads",
        "2",
        "--batch-size",
        "7",
        "--opponents",
        "inactive",
    ])
    .expect("simulation settings parse");
    match cli.command {
        Command::Deck {
            command:
                Some(DeckCommand::Simulate {
                    threads,
                    batch_size,
                    opponents,
                    ..
                }),
            ..
        } => {
            assert_eq!(threads.map(std::num::NonZeroUsize::get), Some(2));
            assert_eq!(batch_size.get(), 7);
            assert_eq!(opponents.as_deref(), Some("inactive"));
        }
        other => panic!("expected simulation, got {other:?}"),
    }
    for invalid in ["--threads", "--batch-size"] {
        assert!(Cli::try_parse_from(["stm", "deck", "simulate", "Lathril", invalid, "0"]).is_err());
    }
    assert!(
        Cli::try_parse_from([
            "stm",
            "deck",
            "simulate",
            "Lathril",
            "--opponents",
            "inactive",
            "--opponent-scenario",
            "table.json",
        ])
        .is_err()
    );
}

/// Query parsing preserves filters, output mode, and default limits.
#[test]
fn cli_parses_valid_tree() {
    let cli = Cli::try_parse_from([
        "stm", "query", "bolt", "--type", "Instant", "--cmc", "<=3", "--json",
    ])
    .expect("should parse");
    match cli.command {
        Command::Query {
            query, json, limit, ..
        } => {
            assert_eq!(query.as_deref(), Some("bolt"));
            assert!(json);
            assert_eq!(limit, 20);
        }
        other => panic!("expected query command, got {other:?}"),
    }
}

/// Sync supports an explicit forced refresh.
#[test]
fn sync_takes_force() {
    let cli = Cli::try_parse_from(["stm", "sync"]).expect("parse");
    assert!(matches!(cli.command, Command::Sync { force: false }));
    let cli = Cli::try_parse_from(["stm", "sync", "--force"]).expect("parse");
    assert!(matches!(cli.command, Command::Sync { force: true }));
}

/// Offline reaches nested commands, and card shorthand remains valid.
#[test]
fn read_commands_take_offline() {
    let cli = Cli::try_parse_from(["stm", "query", "x", "--offline"]).expect("parse");
    assert!(cli.offline);
    let cli = Cli::try_parse_from(["stm", "card", "Bolt"]).expect("parse");
    match cli.command {
        Command::Card {
            command: None,
            name,
            ..
        } => assert_eq!(name.as_deref(), Some("Bolt")),
        other => panic!("unexpected: {other:?}"),
    }
    let cli = Cli::try_parse_from(["stm", "card", "show", "Bolt"]).expect("parse");
    match cli.command {
        Command::Card {
            command: Some(CardCommand::Show { name, .. }),
            ..
        } => assert_eq!(name, "Bolt"),
        other => panic!("unexpected: {other:?}"),
    }
    let cli = Cli::try_parse_from(["stm", "collection", "--offline", "query", "x"]).expect("parse");
    assert!(
        cli.offline,
        "the global flag reaches subcommands without a per-command copy"
    );
    assert!(matches!(
        cli.command,
        Command::Collection {
            command: Some(CollectionCommand::Query { .. }),
            ..
        }
    ));
}

/// Similar-card search accepts ownership and output flags with bounded limits.
#[test]
fn card_similar_takes_flags() {
    let cli = Cli::try_parse_from(["stm", "card", "similar", "Bolt", "--owned"]).expect("parse");
    match cli.command {
        Command::Card {
            command:
                Some(CardCommand::Similar {
                    name, limit, owned, ..
                }),
            ..
        } => {
            assert_eq!(name, "Bolt");
            assert_eq!(limit, 20);
            assert!(owned);
        }
        other => panic!("unexpected: {other:?}"),
    }
    let cli = Cli::try_parse_from([
        "stm",
        "card",
        "similar",
        "Bolt",
        "--limit",
        "50",
        "--json",
        "--offline",
    ])
    .expect("parse");
    assert!(cli.offline, "--offline is global");
    match cli.command {
        Command::Card {
            command: Some(CardCommand::Similar { limit, json, .. }),
            ..
        } => {
            assert_eq!(limit, 50);
            assert!(json);
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(Cli::try_parse_from(["stm", "card", "similar", "Bolt", "--limit", "101"]).is_err());
}

/// Query limits reject empty and excessive result requests.
#[test]
fn query_limit_is_bounded() {
    let cli = Cli::try_parse_from(["stm", "query", "bolt", "--limit", "100"]).expect("parse");
    match cli.command {
        Command::Query { limit, .. } => assert_eq!(limit, 100),
        other => panic!("unexpected command: {other:?}"),
    }
    assert!(Cli::try_parse_from(["stm", "query", "bolt", "--limit", "101"]).is_err());
    assert!(Cli::try_parse_from(["stm", "query", "bolt", "--limit", "0"]).is_err());
}

/// Repeated deck operations retain their separate update instructions.
#[test]
fn deck_update_takes_repeated_ops() {
    let cli = Cli::try_parse_from([
        "stm",
        "deck",
        "update",
        "Burn",
        "--add",
        "2 Bolt",
        "--add",
        "commander:1 Breya",
        "--remove",
        "Sparky",
        "--set",
        "Bolt 4",
    ])
    .expect("parse");
    match cli.command {
        Command::Deck {
            json: _,
            command: Some(DeckCommand::Update {
                add, remove, set, ..
            }),
            ..
        } => {
            assert_eq!(add.len(), 2);
            assert_eq!(remove.len(), 1);
            assert_eq!(set.len(), 1);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// A bare collection command selects collection statistics.
#[test]
fn collection_bare_is_valid() {
    let cli = Cli::try_parse_from(["stm", "collection"]).expect("parse");
    assert!(matches!(
        cli.command,
        Command::Collection { command: None, .. }
    ));
}

/// Help documents the no-results exit code.
#[test]
fn help_mentions_exit_codes() {
    assert!(EXIT_CODE_HELP.contains("3  no results"));
}

/// Deck legality accepts format and bracket output options and rejects invalid brackets.
#[test]
fn deck_legal_takes_format_bracket_json() {
    let cli = Cli::try_parse_from(["stm", "deck", "legal", "Froggy"]).expect("parse");
    match cli.command {
        Command::Deck {
            command: Some(DeckCommand::Legal { format, .. }),
            ..
        } => assert_eq!(format, None),
        other => panic!("unexpected: {other:?}"),
    }
    let cli = Cli::try_parse_from([
        "stm",
        "deck",
        "legal",
        "Froggy",
        "--format",
        "commander",
        "--bracket",
        "3",
        "--json",
    ])
    .expect("parse");
    match cli.command {
        Command::Deck {
            command:
                Some(DeckCommand::Legal {
                    format,
                    bracket,
                    json,
                    ..
                }),
            ..
        } => {
            assert_eq!(format.as_deref(), Some("commander"));
            assert_eq!(bracket, Some(3));
            assert!(json);
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(Cli::try_parse_from(["stm", "deck", "legal", "F", "--bracket", "6"]).is_err());
}

/// Duplicate CLI options must not break command help.
#[test]
fn cli_definition_is_unique() {
    Cli::command().debug_assert();
}

/// Both query commands accept no text, filters alone, and an explicit blank text.
#[test]
fn query_text_is_optional_for_global_and_owned_listings() {
    for prefix in [vec!["stm", "query"], vec!["stm", "collection", "query"]] {
        for suffix in [
            vec![],
            vec!["--type", "Elf", "--set", "FDN"],
            vec!["  ", "--json"],
        ] {
            let mut args = prefix.clone();
            args.extend(suffix);
            let cli = Cli::try_parse_from(args).expect("optional query parses");
            match cli.command {
                Command::Query { query, limit, .. }
                | Command::Collection {
                    command: Some(CollectionCommand::Query { query, limit, .. }),
                    ..
                } => {
                    assert!(query.as_deref().is_none_or(|text| text.trim().is_empty()));
                    assert_eq!(limit, 20);
                }
                other => panic!("expected query, got {other:?}"),
            }
        }
    }
}

/// Root help starts with the task and keeps the public exit-code contract.
#[test]
fn root_help_wording_snapshot() {
    let help = Cli::try_parse_from(["stm", "--help"]).expect_err("help exits before dispatch");
    assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
    let text = help.to_string();
    assert!(text.starts_with("Search Magic cards, track your collection, and build decklists\n"));
    assert!(text.contains("Usage: stm [OPTIONS] <COMMAND>"));
    assert!(text.contains("Skip automatic refreshes and deck URL imports"));
    assert!(text.contains(EXIT_CODE_HELP));
}

/// Forced setup help states data loss and retained files beside the flag.
#[test]
fn setup_force_help_states_the_reset_cost() {
    let text = Cli::try_parse_from(["stm", "setup", "--help"])
        .expect_err("help")
        .to_string();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for expected in [
        "Delete the database and indexes, then rebuild",
        "Collection records are lost; deck files remain",
        "Back up the data directory first",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
}

/// The similar-card ownership flag describes the binder-only membership scope.
#[test]
fn similar_owned_help_states_binder_scope() {
    let text = Cli::try_parse_from(["stm", "card", "similar", "--help"])
        .expect_err("help")
        .to_string();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(text.contains("Only show cards held in binders, not copies assigned to decks"));
}

/// Render short and long help for every command without a data store.
#[test]
fn every_command_has_plain_short_and_long_help() {
    let mut paths = vec![vec!["stm".to_string()]];
    help_paths(&Cli::command(), vec!["stm".to_string()], &mut paths);
    assert_eq!(
        paths.len(),
        33,
        "update the inventory if the command tree changes"
    );
    for path in paths {
        for flag in ["-h", "--help"] {
            let mut args = path.clone();
            args.push(flag.to_string());
            let error = Cli::try_parse_from(args).expect_err("help must not dispatch");
            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
            let text = error.to_string();
            assert!(text.contains("Usage:"), "{path:?} {flag}");
            assert!(!text.contains('\u{1b}'), "plain help: {path:?} {flag}");
            let (local, global) = text
                .split_once("Global options:")
                .expect("global flags have their own section");
            for name in ["--data-dir", "--no-color", "--verbose", "--offline"] {
                assert!(!local.contains(name), "misplaced {name}: {path:?} {flag}");
                assert!(global.contains(name), "missing {name}: {path:?} {flag}");
            }
        }
    }
}

/// Collect public command paths without Clap's generated help command.
fn help_paths(command: &clap::Command, prefix: Vec<String>, paths: &mut Vec<Vec<String>>) {
    for child in command
        .get_subcommands()
        .filter(|child| child.get_name() != "help")
    {
        let mut path = prefix.clone();
        path.push(child.get_name().to_string());
        paths.push(path.clone());
        help_paths(child, path, paths);
    }
}
