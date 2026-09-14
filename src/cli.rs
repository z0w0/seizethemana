use clap::{Args, Parser, Subcommand};

/// Exit codes used across the CLI (grep-style, documented in `--help`).
pub mod codes {
    /// Success.
    pub const OK: i32 = 0;
    /// Runtime error.
    pub const ERROR: i32 = 1;
    /// Usage error (produced by clap; ours too, for invalid deck updates).
    pub const USAGE: i32 = 2;
    /// No results matched, or a referenced card was not found.
    pub const NO_RESULTS: i32 = 3;
}

/// Seize the Mana: Scryfall-backed card search, collection, and deck tooling.
#[derive(Parser, Debug)]
#[command(
    name = "stm",
    version,
    about = "Scryfall-backed card search, collection, and deck tooling",
    after_help = EXIT_CODE_HELP
)]
pub struct Cli {
    /// Data directory (default: ~/.seizethemana)
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<std::path::PathBuf>,

    /// Disable colored output (also honors NO_COLOR and non-TTY stdout)
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Show progress detail on stderr
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Skip background data refreshes (stale-while-revalidate)
    #[arg(long, global = true)]
    pub offline: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// Documented exit codes, shown in `stm --help`.
pub const EXIT_CODE_HELP: &str = "\
Exit codes:
  0  success
  1  runtime error
  2  usage error
  3  no results / card not found";

/// Card subcommands: full detail, and tag-overlap neighbors.
#[derive(Subcommand, Debug)]
pub enum CardCommand {
    /// Show full detail for one card by name
    Show {
        /// Card name (exact, case-insensitive, or unique prefix)
        name: String,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// List cards with the most oracle-tag overlap to a card
    Similar {
        /// Card name (exact, case-insensitive, or unique prefix)
        name: String,
        /// Maximum results (default 20, max 100)
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        /// Only show cards in the collection
        #[arg(long)]
        owned: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
        /// Skip the stale-while-revalidate sync check
        #[arg(long)]
        offline: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Download Scryfall bulk data and build the searchable card index
    Setup {
        /// Rebuild even if setup already completed
        #[arg(long)]
        force: bool,
    },

    /// Refresh card data, prices, and oracle tags from the daily bulks
    Sync {
        /// Re-pull both bulks (cards + tags) and re-apply everything even when fresh
        #[arg(long)]
        force: bool,
    },

    /// Card detail and tag-overlap lookups
    Card {
        /// Card name (`stm card <name>` sugar for `show`)
        #[command(subcommand)]
        command: Option<CardCommand>,

        /// Card name for the bare `stm card <name>` form
        name: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Semantic card search over the whole oracle
    Query {
        /// Free-text query, e.g. "sacrifice a creature to draw cards"
        query: String,
        #[command(flatten)]
        filters: CardFilters,
        /// Maximum results (default 20, max 100)
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        /// Emit JSON
        #[arg(long)]
        json: bool,
        /// Skip the stale-while-revalidate sync check
        #[arg(long)]
        offline: bool,
    },

    /// Collection: import, stats, and owned-only search
    Collection {
        /// Emit JSON (bare `stm collection` stats view)
        #[arg(long)]
        json: bool,
        /// Skip the stale-while-revalidate sync check
        #[arg(long)]
        offline: bool,
        #[command(subcommand)]
        command: Option<CollectionCommand>,
    },

    /// Deck building: create, preview, update, legality, import, export
    Deck {
        /// Emit JSON (bare `stm deck` list view and `stm deck <name>` sugar)
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<DeckCommand>,
        /// Deck name for the bare `stm deck <name>` form
        name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum CollectionCommand {
    /// Import a ManaBox collection CSV export
    Import {
        /// Path to ManaBox collection CSV
        file: std::path::PathBuf,
        /// Add to the existing collection instead of replacing it
        #[arg(long)]
        add: bool,
        /// Replace the collection without confirmation
        #[arg(long)]
        force: bool,
    },
    /// Semantic search restricted to cards you own
    Query {
        /// Free-text query
        query: String,
        #[command(flatten)]
        filters: CardFilters,
        /// Only show cards in this binder (repeatable)
        #[arg(long = "binder", value_name = "NAME")]
        binder: Vec<String>,
        /// Only show cards assigned to this deck (repeatable)
        #[arg(long = "deck", value_name = "NAME")]
        deck: Vec<String>,
        /// Maximum results (default 20, max 100)
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        /// Emit JSON
        #[arg(long)]
        json: bool,
        /// Skip the stale-while-revalidate sync check
        #[arg(long)]
        offline: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum DeckCommand {
    /// Create a new empty deck
    Create { name: String },

    /// List decks
    List {
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Preview a deck (default when no subcommand is given)
    Show {
        /// Deck name; optional so `stm deck <name>` works as sugar
        name: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Update a deck (add/remove/set quantities, per section)
    Update {
        name: String,
        /// Add copies, e.g. `2 Lightning Bolt` or `commander:1 Breya`
        #[arg(long = "add", value_name = "SPEC")]
        add: Vec<String>,
        /// Remove a line entirely, or decrement with `2 Bolt`
        #[arg(long = "remove", value_name = "SPEC")]
        remove: Vec<String>,
        /// Set an exact quantity, e.g. `Bolt 4` (0 deletes the line)
        #[arg(long = "set", value_name = "SPEC")]
        set: Vec<String>,
        /// Text file of extra specs, one per line (`add 1 Name`, `remove 1
        /// Name`, `set 2 Name`, or a bare spec = add; `#` comments allowed)
        #[arg(long = "from", value_name = "FILE")]
        from: Option<std::path::PathBuf>,
    },

    /// Merge duplicate lines (same card name) into one line per section
    Dedupe {
        name: String,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Suggest cards for a deck: role fills, theme cards, or combo
    /// completions, owned first
    Suggest {
        name: String,
        /// Positional free-text query (same as --query), e.g. "frog payoff"
        #[arg(value_name = "QUERY")]
        positional_query: Option<String>,
        /// Free-text query, e.g. "frog payoff" (optional with --role)
        #[arg(long = "query", value_name = "TEXT")]
        query: Option<String>,
        /// Structured role: draw, removal, ramp, wincon, counterspell, land
        #[arg(long = "role", value_name = "ROLE")]
        role: Option<String>,
        /// Find commander candidates for the deck (theme-matched, P/T-legal)
        #[arg(long = "commander")]
        commander: bool,
        /// Bracket 1-5: filter suggestions that break the bracket (brackets
        /// 1-2 allow no Game Changers)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Maximum results (default 10)
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=50))]
        limit: u32,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Check a deck's format legality (and Commander bracket)
    Legal {
        name: String,
        /// Format to check (default: inferred from the deck's sections)
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Commander bracket 1-5 to check (commander formats only)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Import a decklist from a ManaBox deck txt export (upsert by name)
    Import {
        name: String,
        /// ManaBox deck txt file to import
        file: std::path::PathBuf,
    },

    /// Simulate goldfish games to find mana and consistency problems
    Simulate {
        name: String,
        /// Number of games to simulate (default 10000)
        #[arg(long, value_parser = clap::value_parser!(u32).range(100..=1_000_000))]
        runs: Option<u32>,
        /// Turns per game (default 10 commander / 8 constructed)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=30))]
        turns: Option<u32>,
        /// RNG seed for reproducible runs (default: random)
        #[arg(long)]
        seed: Option<u64>,
        /// Format to simulate (default: inferred from the deck's sections)
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Prior JSON report to diff against (deltas only; same seed/runs advised)
        #[arg(long = "baseline", value_name = "FILE")]
        baseline: Option<std::path::PathBuf>,
        /// Print exact hypergeometric cast-on-curve ceilings beside the
        /// simulated castability (draw-agnostic probability math)
        #[arg(long = "hypgeo")]
        hypgeo: bool,
        /// Explicit combo pair to measure ("A + B"; repeatable). Adds to
        /// or overrides any stored combos.
        #[arg(long = "combo", value_name = "A + B")]
        combo: Vec<String>,
        /// Cap on discovered-combo rows (complete + near-miss; default 20)
        #[arg(long = "combo-limit", value_name = "N", value_parser = clap::value_parser!(u32).range(1..=200))]
        combo_limit: Option<u32>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Export a deck to a ManaBox txt file
    Export {
        name: String,
        /// Destination txt file
        file: std::path::PathBuf,
        /// Overwrite the destination file if it exists
        #[arg(long)]
        force: bool,
    },

    /// Delete a decklist (ownership in the collection is kept)
    Delete { name: String },

    /// Missing deck copies as a buylist (owned copies autofill; only
    /// purchases are listed)
    Buylist {
        name: String,
        /// Output format: generic (default), cardkingdom, tcgplayer
        #[arg(long, value_name = "STORE")]
        store: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Print or replace a deck's primer markdown
    Primer {
        name: String,
        /// Replace the primer's contents from a markdown file
        #[arg(long, value_name = "FILE")]
        set: Option<std::path::PathBuf>,
    },
}

/// Structured filters shared by `query` and `collection query`.
#[derive(Args, Debug, Default)]
pub struct CardFilters {
    /// Type line substring, e.g. Creature, Instant, Legendary Land
    #[arg(long = "type", value_name = "TEXT")]
    pub type_: Option<String>,

    /// Colors the card may contain (subset of WUBRG), e.g. WU
    #[arg(long = "color", value_name = "WUBRG")]
    pub color: Option<String>,

    /// Color identity filter (subset of WUBRG), for Commander legality
    #[arg(long = "color-identity", value_name = "WUBRG")]
    pub color_identity: Option<String>,

    /// Converted mana cost comparison, e.g. <=3, =2, >5
    #[arg(long = "cmc", value_name = "OP")]
    pub cmc: Option<String>,

    /// Power comparison, e.g. >=5
    #[arg(long = "power", value_name = "OP")]
    pub power: Option<String>,

    /// Toughness comparison, e.g. <=2
    #[arg(long = "toughness", value_name = "OP")]
    pub toughness: Option<String>,

    /// Rarity: common, uncommon, rare, mythic
    #[arg(long = "rarity", value_name = "R")]
    pub rarity: Option<String>,

    /// Set code, e.g. MH3
    #[arg(long = "set", value_name = "CODE")]
    pub set: Option<String>,

    /// Keyword substring, e.g. Flying
    #[arg(long = "keyword", value_name = "TEXT")]
    pub keyword: Option<String>,

    /// Oracle text substring, e.g. "draw a card"
    #[arg(long = "oracle-text", value_name = "TEXT")]
    pub oracle_text: Option<String>,

    /// Format legality filter, e.g. commander, modern
    #[arg(long = "format", value_name = "FMT")]
    pub format: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_parses_valid_tree() {
        // Well-formed invocation must parse.
        Cli::try_parse_from([
            "stm", "query", "bolt", "--type", "Instant", "--cmc", "<=3", "--json",
        ])
        .expect("should parse");
    }

    #[test]
    fn sync_takes_force() {
        let cli = Cli::try_parse_from(["stm", "sync"]).expect("parse");
        assert!(matches!(cli.command, Command::Sync { force: false }));
        let cli = Cli::try_parse_from(["stm", "sync", "--force"]).expect("parse");
        assert!(matches!(cli.command, Command::Sync { force: true }));
    }

    #[test]
    fn read_commands_take_offline() {
        let cli = Cli::try_parse_from(["stm", "query", "x", "--offline"]).expect("parse");
        match cli.command {
            Command::Query { offline, .. } => assert!(offline),
            other => panic!("unexpected: {other:?}"),
        }
        let cli = Cli::try_parse_from(["stm", "card", "Bolt"]).expect("parse");
        match cli.command {
            Command::Card {
                command: None,
                name,
                ..
            } => {
                assert_eq!(name.as_deref(), Some("Bolt"));
            }
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
        let cli =
            Cli::try_parse_from(["stm", "collection", "--offline", "query", "x"]).expect("parse");
        match cli.command {
            Command::Collection {
                offline,
                command:
                    Some(CollectionCommand::Query {
                        offline: q_offline, ..
                    }),
                ..
            } => {
                assert!(offline);
                assert!(q_offline);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn card_similar_takes_flags() {
        let cli =
            Cli::try_parse_from(["stm", "card", "similar", "Bolt", "--owned"]).expect("parse");
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
        match cli.command {
            Command::Card {
                command:
                    Some(CardCommand::Similar {
                        limit,
                        json,
                        offline,
                        ..
                    }),
                ..
            } => {
                assert_eq!(limit, 50);
                assert!(json);
                assert!(offline);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(Cli::try_parse_from(["stm", "card", "similar", "Bolt", "--limit", "101"]).is_err());
    }

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
                command:
                    Some(DeckCommand::Update {
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

    #[test]
    fn collection_bare_is_valid() {
        let cli = Cli::try_parse_from(["stm", "collection"]).expect("parse");
        assert!(matches!(
            cli.command,
            Command::Collection { command: None, .. }
        ));
    }

    #[test]
    fn help_mentions_exit_codes() {
        // Documented contract: exit codes appear in top-level help.
        assert!(EXIT_CODE_HELP.contains("3  no results"));
    }

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
        // Bracket outside 1-5 fails to parse.
        assert!(Cli::try_parse_from(["stm", "deck", "legal", "F", "--bracket", "6"]).is_err());
    }

    #[test]
    fn cli_definition_is_unique() {
        // Guards against duplicate long flags silently breaking help output.
        Cli::command().debug_assert();
    }
}
