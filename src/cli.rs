//! Command-line surface: clap argument and subcommand definitions for `stm`.
//! The binary dispatches on these; command logic lives in the feature modules.

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

/// Deck zone `deck cuts --make-room-for` ranks swaps for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum MakeRoomFor {
    /// Rank maindeck cuts for each sideboard card
    Sideboard,
    /// Rank maindeck cuts for each maybeboard card
    Maybeboard,
}

/// Global options and commands for card search, collections, and decklists.
#[derive(Parser, Debug)]
#[command(
    name = "stm",
    version,
    about = "Search Magic cards, track your collection, and build decklists",
    styles = help_styles(),
    after_help = format!("Examples:\n  stm setup\n  stm query \"draw cards\" --color-identity UG\n  stm card \"Sol Ring\"\n  stm collection import collection.csv\n  stm deck list\n\nUse 'stm <command> --help' for options and examples.\nUse --json where supported for scripts and agents. Results go to stdout;\nprogress and errors go to stderr.\n\n{EXIT_CODE_HELP}")
)]
pub struct Cli {
    /// Data directory (default: ~/.seizethemana)
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        help_heading = "Global options"
    )]
    pub data_dir: Option<std::path::PathBuf>,

    /// Turn off color (also set by NO_COLOR)
    #[arg(long, global = true, help_heading = "Global options")]
    pub no_color: bool,

    /// Show progress detail on stderr
    #[arg(long, global = true, help_heading = "Global options")]
    pub verbose: bool,

    /// Skip automatic refreshes and deck URL imports. Setup, sync, and missing
    /// model files can still require downloads
    #[arg(long, global = true, help_heading = "Global options")]
    pub offline: bool,

    /// Top-level subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Documented exit codes, shown in `stm --help`.
pub const EXIT_CODE_HELP: &str = "\
Exit codes:
  0  success
  1  runtime error, legality violations, or simulation findings
     (baseline comparisons: new findings only)
  2  usage error
  3  no results or card not found";

/// Use the terminal accent in help without changing error and warning roles.
fn help_styles() -> clap::builder::Styles {
    use clap::builder::styling::{AnsiColor, RgbColor};
    let accent = if let Some((red, green, blue)) = crate::output::accent_rgb() {
        RgbColor(red, green, blue).on_default().bold()
    } else {
        AnsiColor::Magenta.on_default().bold()
    };
    clap::builder::Styles::styled()
        .header(accent)
        .literal(accent)
}

/// Card subcommands: full detail, and tag-overlap neighbors.
#[derive(Subcommand, Debug)]
pub enum CardCommand {
    /// Show card details by name
    Show {
        /// Card name (exact, case-insensitive, or unique prefix)
        name: String,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Find cards with similar tags and rules text
    Similar {
        /// Card name (exact, case-insensitive, or unique prefix)
        name: String,
        /// Maximum results (1-100)
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        /// Only show cards held in binders, not copies assigned to decks
        #[arg(long)]
        owned: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// List Commander Spellbook combos that include a card
    Combos {
        /// Card name (exact, case-insensitive, or unique prefix)
        name: String,
        /// Only combos legal in this format, e.g. commander, modern
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Maximum results (1-100)
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
}

/// Top-level stm subcommands: setup, sync, card, collection, and deck.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Download Scryfall bulk data and build the searchable card index
    Setup {
        /// Delete the database and indexes, then rebuild. Collection records are
        /// lost; deck files remain (including primers). Back up the data directory first
        #[arg(long)]
        force: bool,
    },

    /// Refresh card data, prices, tags, and combos
    Sync {
        /// Download and apply fresh source data even if the store is current
        #[arg(long)]
        force: bool,
    },

    /// Show card details, similar cards, and combos
    #[command(
        after_help = "Examples:\n  stm card \"Sol Ring\"\n  stm card similar \"Sol Ring\" --owned\n  stm card combos \"Sol Ring\" --json\n\nQuote card names that contain spaces. Names must match a full name or a\nunique prefix; matching ignores case."
    )]
    Card {
        /// Card subcommand; a card name alone also shows details
        #[command(subcommand)]
        command: Option<CardCommand>,

        /// Card name; same as the show subcommand
        name: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Search by words and meaning, or list cards by filters and name
    #[command(
        after_help = "Examples:\n  stm query \"draw cards\" --color-identity UG --cmc '<=3'\n  stm query --type Elf --format commander\n  stm query \"mana ramp\" --max-price 5 --json\n\nCombine filters to narrow results. Quote comparisons such as '<=3' so the\nshell does not treat them as redirects. Color filters include colorless cards."
    )]
    Query {
        /// Search text; omit to list alphabetically (JSON score is 0)
        query: Option<String>,
        /// Structured filters applied to the query.
        #[command(flatten)]
        filters: CardFilters,
        /// Maximum results (1-100)
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        /// Keep only cards priced at or under this cap in US dollars (USD);
        /// unpriced cards are excluded
        #[arg(long = "max-price", value_name = "USD", value_parser = parse_max_price)]
        max_price: Option<f64>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Import owned cards, show collection stats, or search your collection
    #[command(
        after_help = "Examples:\n  stm collection\n  stm collection import collection.csv\n  stm collection query --type Elf\n  stm collection conflicts\n\nWithout a subcommand, show collection stats. Import replaces collection\nrecords unless --add is set. Saved decklists are separate from owned cards."
    )]
    Collection {
        /// Emit JSON for collection stats (without a subcommand)
        #[arg(long)]
        json: bool,
        /// Collection subcommand; none shows collection stats.
        #[command(subcommand)]
        command: Option<CollectionCommand>,
    },

    /// Create, edit, check, and share decklists
    #[command(
        after_help = "Examples:\n  stm deck list\n  stm deck create Froggy\n  stm deck import Froggy deck.txt\n  stm deck show Froggy\n  stm deck update Froggy --add '1 Sol Ring' --dry-run\n\nWithout a subcommand, list decks. 'stm deck <name>' shows a deck. Decklists\nrecord intended contents; editing them does not change collection ownership.\nRun commands that edit the same deck one at a time."
    )]
    Deck {
        /// Emit JSON for the deck list or a named deck
        #[arg(long)]
        json: bool,
        /// Deck subcommand; none shows the deck list.
        #[command(subcommand)]
        command: Option<DeckCommand>,
        /// Deck name for the bare `stm deck <name>` form
        name: Option<String>,
    },
}

/// Collection subcommands: import, stats, valuation, and card lookups.
#[derive(Subcommand, Debug)]
// `Import` carries a `PathBuf` plus filter struct, far larger than the
// other variants' fields; the box is not worth the indirection.
#[allow(clippy::large_enum_variant)]
pub enum CollectionCommand {
    /// Import a ManaBox CSV (replaces collection records unless --add is set)
    Import {
        /// Path to ManaBox collection CSV
        file: std::path::PathBuf,
        /// Add to the existing collection instead of replacing it
        #[arg(long)]
        add: bool,
    },
    /// Find cards needed by more decklists than your collection can supply
    Conflicts {
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Search owned cards, or list owned cards by filters and name
    #[command(
        after_help = "Examples:\n  stm collection query --type Elf\n  stm collection query \"draw cards\" --binder Trade --json\n  stm collection query --binder Trade --deck Froggy\n\nBy default, search binder copies only. --binder narrows that pool. --deck\nadds copies assigned to the named deck; it does not remove binder copies."
    )]
    Query {
        /// Search text; omit to list alphabetically (JSON score is 0)
        query: Option<String>,
        /// Structured filters applied to the query.
        #[command(flatten)]
        filters: CardFilters,
        /// Search this binder instead of all binders (repeatable)
        #[arg(long = "binder", value_name = "NAME")]
        binder: Vec<String>,
        /// Also include copies assigned to this deck (repeatable)
        #[arg(long = "deck", value_name = "NAME")]
        deck: Vec<String>,
        /// Maximum results (1-100)
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Find surplus binder copies after deck and reserve needs
    #[command(
        after_help = "Examples:\n  stm collection sell\n  stm collection sell --target 50 --details\n  stm collection sell --output csv > sell.csv\n\nThis command recommends copies to sell; it does not remove ownership records.\nPrices are USD market snapshots, not guaranteed sale proceeds. Review warnings\nand saved decklists before selling. --limit affects display only, not exports."
    )]
    Sell {
        /// Export the full sell list to stdout in ManaBox TXT or CSV format
        #[arg(long, value_enum, value_name = "FORMAT", conflicts_with_all = ["json", "details", "review"])]
        output: Option<crate::collection_sell::SellOutput>,
        /// Exclude a binder from the deckbuilding pool (repeat for multiple binders)
        #[arg(long, value_name = "BINDER")]
        exclude_binder: Vec<String>,
        /// Show printing allocations and demand evidence
        #[arg(long)]
        details: bool,
        /// Also show cards with personal hold reasons
        #[arg(long)]
        review: bool,
        /// Show excess bulk copies below $1 instead of singles
        #[arg(long, conflicts_with = "target")]
        bulk: bool,
        /// Estimate bulk proceeds at this USD per 1,000 copies
        #[arg(long, value_name = "USD", requires = "bulk", value_parser = parse_positive_usd)]
        bulk_rate: Option<f64>,
        /// Only sale printings priced at or above this USD (singles stay at least $1)
        #[arg(long = "min-price", value_name = "USD", value_parser = parse_max_price)]
        min_price: Option<f64>,
        /// Only sale printings priced at or below this USD
        #[arg(long = "max-price", value_name = "USD", value_parser = parse_max_price)]
        max_price: Option<f64>,
        /// Only this rarity: common, uncommon, rare, or mythic
        #[arg(long, value_name = "RARITY")]
        rarity: Option<String>,
        /// Scope combo evidence and report legality for this format
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Low-demand Commander rank boundary (default 15000, must exceed 5000)
        #[arg(long = "rank-floor", value_name = "N", value_parser = parse_rank_floor)]
        rank_floor: Option<i64>,
        /// Pick ranked singles toward this market-value target, before selling costs
        #[arg(long, value_name = "USD", value_parser = parse_positive_usd)]
        target: Option<f64>,
        /// Maximum displayed results per section (1-500; exports are complete)
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=500))]
        limit: u32,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
}

/// Deck subcommands: build, edit, evaluate, and share deck files.
#[derive(Subcommand, Debug)]
pub enum DeckCommand {
    /// Create an empty decklist and primer
    Create {
        /// New deck name.
        name: String,
    },

    /// List saved decklists and deck assignments without a decklist
    List {
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Show a decklist, ownership, and card stats
    Show {
        /// Deck name; omit to list saved decklists
        name: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Check colored mana sources against recommended minimums
    Mana {
        /// Deck name
        name: String,
        /// Format to check (default: inferred from the deck's sections)
        #[arg(long)]
        format: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Sample opening hands (same shuffle and mulligan rules as simulate)
    Hand {
        /// Deck name
        name: String,
        /// Random seed for reproducible hands (default: random)
        #[arg(long)]
        seed: Option<u64>,
        /// Number of hands to deal (1-10)
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=10))]
        count: u32,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Add, remove, set, or move cards in a decklist
    #[command(
        after_help = "Examples:\n  stm deck update Froggy --add '1 Sol Ring' --dry-run\n  stm deck update Froggy --remove '2 Island' --add '2 Forest'\n  stm deck update Froggy --move '1 Sol Ring to:sideboard'\n  stm deck update Froggy --from changes.txt --dry-run --legal\n\nQuote each card instruction. Use full card names, not abbreviations. Prefix\nan instruction with a section, such as 'sideboard:1 Sol Ring'. An omitted\nsection defaults to DECK; an omitted move destination also defaults to DECK.\n\nFile instructions start with add, remove, set, or move. A bare instruction\nmeans add. Lines starting with # are comments. Preview with --dry-run, then\nrepeat without --dry-run to save. Collection ownership does not change."
    )]
    Update {
        /// Deck name.
        name: String,
        /// Add copies, e.g. '2 Lightning Bolt' (repeatable)
        #[arg(long = "add", value_name = "SPEC")]
        add: Vec<String>,
        /// Remove a line by name, or remove a quantity, e.g. '2 Lightning Bolt'
        #[arg(long = "remove", value_name = "SPEC")]
        remove: Vec<String>,
        /// Set a quantity, e.g. '4 Lightning Bolt' (0 deletes the line)
        #[arg(long = "set", value_name = "SPEC")]
        set: Vec<String>,
        /// Move copies, e.g. '1 Lightning Bolt to:sideboard' (repeatable)
        #[arg(long = "move", value_name = "SPEC")]
        r#move: Vec<String>,
        /// Read more card instructions from a text file, one per line
        #[arg(long = "from", value_name = "FILE")]
        from: Option<std::path::PathBuf>,
        /// Save other edits when remove or move targets are missing (still exits 1).
        /// Invalid instructions still fail; this flag does not change --dry-run
        #[arg(long = "allow-partial")]
        allow_partial: bool,
        /// Preview card and cost changes without saving
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// With --dry-run, compare before and after simulations at the same seed
        #[arg(long = "sim")]
        sim: bool,
        /// With --dry-run, check the proposed deck's legality and fail if illegal
        #[arg(long = "legal")]
        legal: bool,
        /// Add basic lands within color identity to reach 100 cards for Commander
        /// or 60 for other formats
        #[arg(long = "backfill-basics")]
        backfill_basics: bool,
        /// Emit JSON (dry-run preview or the update summary)
        #[arg(long)]
        json: bool,
    },

    /// Merge duplicate lines per section and cap Commander singleton quantities
    Dedupe {
        /// Deck name.
        name: String,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Suggest cards by role, theme, or combo needs, with owned cards first
    #[command(
        after_help = "Examples:\n  stm deck suggest Froggy --role draw --owned\n  stm deck suggest Froggy \"frog payoff\" --max-price 5\n  stm deck suggest Froggy --json\n\nWithout a query or role, suggest cards that complete stored combos.\nCommon roles include draw, ramp, removal, board-wipe, counterspell, tutor,\nrecursion, token, and wincon. Queries describe a theme in your own words."
    )]
    Suggest {
        /// Deck name.
        name: String,
        /// Theme or card job, e.g. "frog payoff" (same as --query)
        #[arg(value_name = "QUERY")]
        positional_query: Option<String>,
        /// Theme or card job, e.g. "frog payoff"
        #[arg(long = "query", value_name = "TEXT")]
        query: Option<String>,
        /// Card job or theme, e.g. draw, ramp, removal, or landfall
        #[arg(long = "role", value_name = "ROLE")]
        role: Option<String>,
        /// Find commander candidates by theme and eligible card type
        #[arg(long = "commander")]
        commander: bool,
        /// Restrict suggestions to this format (default: inferred from sections).
        /// Exclude combos that need a commander in 60-card formats
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Bracket 1-5: filter suggestions that break the bracket (brackets
        /// 1-2 allow no Game Changers)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Maximum price per card in USD; exclude unpriced cards
        #[arg(long = "max-price", value_name = "USD", value_parser = parse_max_price)]
        max_price: Option<f64>,
        /// Maximum results (1-50)
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=50))]
        limit: u32,
        /// Restrict candidates to cards the collection owns
        #[arg(long)]
        owned: bool,
        /// Exclude a card name, file of names, or '-' for stdin (repeatable)
        #[arg(long = "exclude", value_name = "NAME|FILE")]
        exclude: Vec<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Check a deck's format legality (and Commander bracket)
    Legal {
        /// Deck name.
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

    /// Import a decklist from a file or URL, replacing any decklist with this name
    #[command(
        after_help = "Examples:\n  stm deck import Froggy deck.txt\n  stm deck import Froggy --url https://archidekt.com/decks/12345\n\nChoose a local file or an Archidekt URL. Import replaces the named decklist\nand keeps an existing primer. Back up the decklist first. Collection ownership\ndoes not change."
    )]
    Import {
        /// Deck name to create or overwrite.
        name: String,
        /// Deck text file to import
        #[arg(value_name = "FILE")]
        file: Option<std::path::PathBuf>,
        /// URL to import from (Archidekt deck URL)
        #[arg(long = "url", value_name = "URL")]
        url: Option<String>,
        /// Force the format (default: auto-detect from the text)
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
    },

    /// Simulate goldfish games to find mana and consistency problems
    #[command(
        after_help = "Examples:\n  stm deck simulate Froggy --runs 1000 --seed 42\n  stm deck simulate Froggy --seed 42 --json > baseline.json\n  stm deck simulate Froggy --seed 42 --baseline baseline.json\n\nGoldfish games test your deck without a full opposing game. They do not\nestimate win rate. The opponent model supplies limited external activity.\nReports state supported rules and limits; an unsupported ability stays inert.\n\nA report can be valid even when the command exits 1 for findings. Baseline\nmode exits 1 for new findings only. Use matching seeds, game counts, turn\nlimits, formats, and opponent settings for useful comparisons."
    )]
    Simulate {
        /// Deck name.
        name: String,
        /// Number of games to simulate (100-1000000; default 10000)
        #[arg(long, value_parser = clap::value_parser!(u32).range(100..=1_000_000))]
        runs: Option<u32>,
        /// Turns per game (1-30; default 10 for Commander, 8 for other formats)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=30))]
        turns: Option<u32>,
        /// Random seed for reproducible games (default: random)
        #[arg(long)]
        seed: Option<u64>,
        /// Worker threads (default: available parallelism)
        #[arg(long)]
        threads: Option<std::num::NonZeroUsize>,
        /// Maximum games scheduled per batch
        #[arg(long, default_value = "256")]
        batch_size: std::num::NonZeroUsize,
        /// External activity: synthetic or inactive (default: synthetic for
        /// Commander, inactive otherwise)
        #[arg(long, value_parser = ["synthetic", "inactive"], conflicts_with = "opponent_scenario", help_heading = "Opponent model")]
        opponents: Option<String>,
        /// JSON opponent settings: activity, land target, spell costs, and tax policy
        #[arg(
            long,
            value_name = "FILE",
            conflicts_with = "opponents",
            help_heading = "Opponent model"
        )]
        opponent_scenario: Option<std::path::PathBuf>,
        /// Format to simulate (default: inferred from the deck's sections)
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Compare with a saved JSON report (use the same seed and game count)
        #[arg(
            long = "baseline",
            value_name = "FILE",
            help_heading = "Report options"
        )]
        baseline: Option<std::path::PathBuf>,
        /// Estimate drawing each spell with enough lands, without mulligans or ramp
        #[arg(long = "hypgeo", help_heading = "Report options")]
        hypgeo: bool,
        /// Measure access to this pair, e.g. 'Sol Ring + Mana Vault' (repeatable)
        #[arg(long = "combo", value_name = "A + B", help_heading = "Report options")]
        combo: Vec<String>,
        /// Maximum discovered combos, including missing-piece combos (1-200; default 20)
        #[arg(long = "combo-limit", value_name = "N", value_parser = clap::value_parser!(u32).range(1..=200), help_heading = "Report options")]
        combo_limit: Option<u32>,
        /// Commander bracket 1-5 for the mana-base target band (default 3)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Rank cards to consider cutting from the maindeck
    Cuts {
        /// Deck name.
        name: String,
        /// Maximum cards to suggest cutting (1-50)
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=50))]
        count: u32,
        /// Pair cuts with suggested cards for this role
        #[arg(long = "for", value_name = "ROLE")]
        for_role: Option<String>,
        /// Commander bracket 1-5; rank excess Game Changers first
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Format to judge legality against (default: inferred from the
        /// deck's sections)
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Maximum replacement-card price in USD for --for; exclude unpriced cards
        #[arg(long = "max-price", value_name = "USD", value_parser = parse_max_price)]
        max_price: Option<f64>,
        /// Rank maindeck cuts for each card in this section; check color identity,
        /// legality, and role
        #[arg(long = "make-room-for", value_enum)]
        make_room_for: Option<MakeRoomFor>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Audit a deck's Commander Spellbook combos, per section
    Combos {
        /// Deck name.
        name: String,
        /// Format to filter combos by (default: inferred from the deck's
        /// sections)
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Commander bracket 1-5: flag combos whose bracket tag exceeds it
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Compare two decklists by section
    #[command(
        after_help = "Examples:\n  stm deck diff Before After\n  stm deck diff Before After --json\n  stm deck diff Before After --as-update > changes.txt\n  stm deck update Before --from changes.txt --dry-run\n\nChanges turn the first deck into the second. Choose only one output mode:\n--json, --markdown, or --as-update. By default, compare card names; --exact\nalso compares set, collector number, and finish."
    )]
    Diff {
        /// Base deck name, or a ManaBox deck txt file path
        deck_a: String,
        /// Target deck name, or a ManaBox deck txt file path
        deck_b: String,
        /// Compare exact printings (name, set, collector number, finish) instead of card
        /// names (any printing fills a slot)
        #[arg(long = "exact")]
        exact: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
        /// Print a Markdown table of cards to remove and add
        #[arg(long = "markdown", conflicts_with = "json")]
        markdown: bool,
        /// Print card instructions for 'deck update --from'
        #[arg(long = "as-update", conflicts_with_all = ["json", "markdown"])]
        as_update: bool,
    },

    /// Export a decklist to a text file
    Export {
        /// Deck name.
        name: String,
        /// Destination txt file
        file: std::path::PathBuf,
        /// Overwrite the destination file if it exists
        #[arg(long)]
        force: bool,
        /// Output format: manabox (default), names, moxfield, archidekt,
        /// arena
        #[arg(long, value_name = "FMT", default_value = "manabox")]
        format: String,
    },

    /// Delete a decklist and its primer (collection ownership is kept)
    Delete {
        /// Deck name.
        name: String,
    },

    /// List deck copies to buy after available owned copies fill slots
    Buylist {
        /// Deck name.
        name: String,
        /// Output format: generic (default), cardkingdom, tcgplayer
        #[arg(long, value_name = "STORE")]
        store: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Duplicate a decklist and its primer under a new name
    Copy {
        /// Source deck name
        source: String,
        /// New deck name to create
        destination: String,
        /// Overwrite the destination decklist. Replace its primer only if the
        /// source has one; otherwise keep it. Back up both first
        #[arg(long)]
        force: bool,
    },

    /// Print or replace a deck's Markdown primer
    Primer {
        /// Deck name.
        name: String,
        /// Replace the primer from a Markdown file, or '-' for stdin.
        /// Back up the current primer first
        #[arg(long, value_name = "FILE")]
        set: Option<std::path::PathBuf>,
    },
}

/// Parse `--max-price`: reject negative caps at parse time (a negative
/// budget hides everything; the user meant a smaller positive cap).
fn parse_max_price(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|_| format!("`{s}` is not a USD amount"))?;
    if !v.is_finite() {
        return Err("price must be a finite USD amount".to_string());
    }
    if v < 0.0 {
        return Err("price must be zero or positive".to_string());
    }
    Ok(v)
}

/// Keep the low-demand boundary above the useful-spare boundary.
fn parse_rank_floor(s: &str) -> Result<i64, String> {
    let v: i64 = s
        .parse()
        .map_err(|_| format!("`{s}` is not an integer rank"))?;
    if v <= crate::collection_sell::PLAYED_CUTOFF {
        return Err(format!(
            "--rank-floor must be above {} (the played cutoff)",
            crate::collection_sell::PLAYED_CUTOFF
        ));
    }
    Ok(v)
}

/// Parse `--target`: require a positive USD amount so the fund never comes
/// back silently empty.
fn parse_positive_usd(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|_| format!("`{s}` is not a USD amount"))?;
    if !v.is_finite() || v <= 0.0 {
        return Err("amount must be a positive, finite USD value".to_string());
    }
    Ok(v)
}

/// The active subcommand's `--json` flag, or false when the subcommand has
/// none. Lets the renderer suppress status lines before dispatch.
impl Cli {
    /// Whether the active subcommand set `--json`.
    pub fn json_flag(&self) -> bool {
        match &self.command {
            Command::Setup { .. } => false,
            Command::Sync { .. } => false,
            Command::Card { command, json, .. } => {
                if let Some(sub) = command {
                    *match sub {
                        CardCommand::Show { json, .. }
                        | CardCommand::Similar { json, .. }
                        | CardCommand::Combos { json, .. } => json,
                    }
                } else {
                    *json
                }
            }
            Command::Query { json, .. } => *json,
            Command::Collection { json, command, .. } => match command {
                None => *json,
                Some(c) => match c {
                    CollectionCommand::Conflicts { json } => *json,
                    CollectionCommand::Import { .. } => false,
                    CollectionCommand::Query { json, .. } => *json,
                    CollectionCommand::Sell { json, .. } => *json,
                },
            },
            Command::Deck { json, command, .. } => deck_json_flag(json, command),
        }
    }
}

/// `--json` for deck subcommands; a subcommand's flag wins, the sugar views
/// fall back to the parent flag.
fn deck_json_flag(json: &bool, command: &Option<DeckCommand>) -> bool {
    match command {
        None => *json,
        Some(c) => match c {
            DeckCommand::Create { .. }
            | DeckCommand::Import { .. }
            | DeckCommand::Export { .. }
            | DeckCommand::Delete { .. }
            | DeckCommand::Copy { .. }
            | DeckCommand::Primer { .. } => false,
            DeckCommand::List { json }
            | DeckCommand::Show { json, .. }
            | DeckCommand::Mana { json, .. }
            | DeckCommand::Hand { json, .. }
            | DeckCommand::Update { json, .. }
            | DeckCommand::Dedupe { json, .. }
            | DeckCommand::Suggest { json, .. }
            | DeckCommand::Legal { json, .. }
            | DeckCommand::Simulate { json, .. }
            | DeckCommand::Cuts { json, .. }
            | DeckCommand::Combos { json, .. }
            | DeckCommand::Diff { json, .. }
            | DeckCommand::Buylist { json, .. } => *json,
        },
    }
}

/// Structured filters shared by `query` and `collection query`.
#[derive(Args, Debug, Default)]
pub struct CardFilters {
    /// Type line contains this text, e.g. Creature, Instant, or 'Legendary Land'
    #[arg(long = "type", value_name = "TEXT", help_heading = "Filters")]
    pub type_: Option<String>,

    /// Allow only these colors, e.g. WU (also includes colorless cards)
    #[arg(long = "color", value_name = "WUBRG", help_heading = "Filters")]
    pub color: Option<String>,

    /// Card identity must fit within these colors, e.g. WU (includes colorless)
    #[arg(
        long = "color-identity",
        value_name = "WUBRG",
        help_heading = "Filters"
    )]
    pub color_identity: Option<String>,

    /// Mana value comparison, e.g. '<=3', '=2', or '>5'
    #[arg(long = "cmc", value_name = "OP", help_heading = "Filters")]
    pub cmc: Option<String>,

    /// Power comparison, e.g. '>=5'
    #[arg(long = "power", value_name = "OP", help_heading = "Filters")]
    pub power: Option<String>,

    /// Toughness comparison, e.g. '<=2'
    #[arg(long = "toughness", value_name = "OP", help_heading = "Filters")]
    pub toughness: Option<String>,

    /// Rarity: common, uncommon, rare, mythic
    #[arg(long = "rarity", value_name = "R", help_heading = "Filters")]
    pub rarity: Option<String>,

    /// Set code of the stored card record, e.g. MH3 (not every available printing)
    #[arg(long = "set", value_name = "CODE", help_heading = "Filters")]
    pub set: Option<String>,

    /// Keyword contains this text, e.g. Flying
    #[arg(long = "keyword", value_name = "TEXT", help_heading = "Filters")]
    pub keyword: Option<String>,

    /// Rules text contains this text, e.g. 'draw a card'
    #[arg(long = "oracle-text", value_name = "TEXT", help_heading = "Filters")]
    pub oracle_text: Option<String>,

    /// Legal in this format, e.g. commander or modern
    #[arg(long = "format", value_name = "FMT", help_heading = "Filters")]
    pub format: Option<String>,
}

#[cfg(test)]
#[path = "tests/cli_tests.rs"]
mod cli_tests;

/// Help layout, examples, and command safety guidance.
#[cfg(test)]
#[path = "tests/cli_help_tests.rs"]
mod cli_help_tests;
