//! Help examples and scope notes that users need before running commands.

use super::*;

/// Read help as plain text without opening a store or running a command.
fn help(path: &[&str]) -> String {
    let mut args = vec!["stm"];
    args.extend_from_slice(path);
    args.push("--help");
    Cli::try_parse_from(args)
        .expect_err("help stops before dispatch")
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A new user can find setup, a real query, and the machine-output contract.
#[test]
fn root_help_gives_a_starting_point_and_output_contract() {
    let text = help(&[]);
    for expected in [
        "Examples: stm setup",
        "stm query \"draw cards\" --color-identity UG",
        "stm <command> --help",
        "Results go to stdout; progress and errors go to stderr",
        "Setup, sync, and missing model files can still require downloads",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
}

/// Collection location flags describe their union, not an exclusive deck search.
#[test]
fn collection_query_help_explains_binder_and_deck_scope() {
    let text = help(&["collection", "query"]);
    for expected in [
        "Filters:",
        "By default, search binder copies only",
        "--binder narrows that pool",
        "--deck adds copies assigned to the named deck",
        "it does not remove binder copies",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
}

/// Update instructions use real names and tell users how to preview a write.
#[test]
fn update_help_uses_full_card_names_and_a_safe_preview() {
    let text = help(&["deck", "update"]);
    for expected in [
        "stm deck update Froggy --add '1 Sol Ring' --dry-run",
        "Use full card names, not abbreviations",
        "repeat without --dry-run to save",
        "Collection ownership does not change",
        "this flag does not change --dry-run",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
    assert!(!text.contains("'2 Bolt'"));
}

/// A copy without a source primer leaves the destination primer in place.
#[test]
fn copy_help_explains_when_the_primer_changes() {
    let text = help(&["deck", "copy"]);
    assert!(text.contains("Replace its primer only if the source has one; otherwise keep it"));
    assert!(text.contains("Back up both first"));
}

/// Simulations describe sampling limits and result exits beside the examples.
#[test]
fn simulation_help_separates_settings_and_report_limits() {
    let text = help(&["deck", "simulate"]);
    for expected in [
        "Opponent model:",
        "Report options:",
        "drawing each spell with enough lands",
        "They do not estimate win rate",
        "an unsupported ability stays inert",
        "Baseline mode exits 1 for new findings only",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
}

/// Help examples use valid syntax without combining exclusive output formats.
#[test]
fn task_examples_parse_without_running_commands() {
    for args in [
        vec![
            "stm",
            "query",
            "draw cards",
            "--color-identity",
            "UG",
            "--cmc",
            "<=3",
        ],
        vec!["stm", "query", "--type", "Elf", "--format", "commander"],
        vec!["stm", "card", "similar", "Sol Ring", "--owned"],
        vec![
            "stm",
            "collection",
            "query",
            "--binder",
            "Trade",
            "--deck",
            "Froggy",
        ],
        vec!["stm", "collection", "sell", "--target", "50", "--details"],
        vec!["stm", "collection", "sell", "--output", "csv"],
        vec![
            "stm",
            "deck",
            "update",
            "Froggy",
            "--add",
            "1 Sol Ring",
            "--dry-run",
        ],
        vec![
            "stm",
            "deck",
            "update",
            "Froggy",
            "--move",
            "1 Sol Ring to:sideboard",
        ],
        vec![
            "stm",
            "deck",
            "update",
            "Froggy",
            "--from",
            "changes.txt",
            "--dry-run",
            "--legal",
        ],
        vec![
            "stm",
            "deck",
            "suggest",
            "Froggy",
            "frog payoff",
            "--max-price",
            "5",
        ],
        vec![
            "stm", "deck", "simulate", "Froggy", "--runs", "1000", "--seed", "42",
        ],
        vec![
            "stm",
            "deck",
            "simulate",
            "Froggy",
            "--seed",
            "42",
            "--baseline",
            "baseline.json",
        ],
        vec!["stm", "deck", "diff", "Before", "After", "--as-update"],
    ] {
        assert!(
            Cli::try_parse_from(&args).is_ok(),
            "invalid example: {args:?}"
        );
    }
    let diff = help(&["deck", "diff"]);
    assert!(diff.contains("Changes turn the first deck into the second"));
    assert!(diff.contains("Choose only one output mode"));
}

/// Clap shows fixed defaults once instead of repeating them in descriptions.
#[test]
fn result_limits_show_a_single_default() {
    for (path, default) in [
        (vec!["query"], "20"),
        (vec!["card", "similar"], "20"),
        (vec!["collection", "query"], "20"),
        (vec!["collection", "sell"], "20"),
        (vec!["deck", "suggest"], "10"),
        (vec!["deck", "hand"], "3"),
        (vec!["deck", "cuts"], "5"),
    ] {
        let text = help(&path);
        assert_eq!(
            text.matches(&format!("[default: {default}]")).count(),
            1,
            "{text}"
        );
        assert!(!text.contains(&format!("(default {default},")), "{text}");
    }
}
