use super::*;
use clap::CommandFactory;

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
            assert_eq!(query, "bolt");
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
