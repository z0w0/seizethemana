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
/// stm CLI root: global flags plus the subcommand dispatch.
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
    },

    /// List Commander Spellbook combos that include a card
    Combos {
        /// Card name (exact, case-insensitive, or unique prefix)
        name: String,
        /// Only combos legal in this format, e.g. commander, modern
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Maximum results (default 20, max 100)
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

    /// Hybrid card search: keyword (BM25) + meaning (vector) matches,
    /// fused by reciprocal rank fusion (score 0-1)
    Query {
        /// Free-text query, e.g. "sacrifice a creature to draw cards"
        query: String,
        #[command(flatten)]
        filters: CardFilters,
        /// Maximum results (default 20, max 100)
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

    /// Collection: import, stats, and owned-only search
    Collection {
        /// Emit JSON (bare `stm collection` stats view)
        #[arg(long)]
        json: bool,
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

/// Collection subcommands: import, stats, valuation, and card lookups.
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
    },
    /// Cards wanted by more decks than you own copies of
    Conflicts {
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Hybrid search restricted to cards you own (keyword + meaning legs)
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
    },
}

/// Deck subcommands: build, edit, evaluate, and share deck files.
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

    /// Static colored-source audit (Karsten requirement floors; no simulation)
    Mana {
        /// Deck name
        name: String,
        /// Pin the format (default: inferred from the deck's sections)
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
        /// RNG seed for reproducible hands (default: random)
        #[arg(long)]
        seed: Option<u64>,
        /// Number of hands to deal (default 3, max 10)
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=10))]
        count: u32,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Update a deck (add/remove/set/move quantities, per section)
    Update {
        name: String,
        /// Add copies, e.g. `2 Lightning Bolt` or `commander:1 Breya`
        #[arg(long = "add", value_name = "SPEC")]
        add: Vec<String>,
        /// Remove a line entirely, or decrement with `2 Bolt`
        #[arg(long = "remove", value_name = "SPEC")]
        remove: Vec<String>,
        /// Set an exact quantity, e.g. `4 Bolt` or `0 Breya` (0 deletes the line)
        #[arg(long = "set", value_name = "SPEC")]
        set: Vec<String>,
        /// Move copies between sections, e.g. `1 Bolt to:sideboard` or
        /// `sideboard:1 Bolt` (to: defaults to DECK)
        #[arg(long = "move", value_name = "SPEC")]
        r#move: Vec<String>,
        /// Text file of extra specs, one per line (`add 1 Name`, `remove 1
        /// Name`, `set 2 Name`, `move 1 Name to:sideboard`, or a bare spec
        /// = add; `#` comments allowed)
        #[arg(long = "from", value_name = "FILE")]
        from: Option<std::path::PathBuf>,
        /// Apply the resolvable ops and report the unresolvable ones
        /// instead of aborting the whole batch
        #[arg(long = "allow-partial")]
        allow_partial: bool,
        /// Preview the change: list diff + cost impact, nothing written
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// With --dry-run: also simulate before/after at the same seed and
        /// print the consistency delta
        #[arg(long = "sim")]
        sim: bool,
        /// With --dry-run: also run the legality checks on the post-change
        /// deck and fail the preview when the result is illegal
        #[arg(long = "legal")]
        legal: bool,
        /// Add basic lands after the ops until the deck reaches its size
        /// (100 for commander, 60 otherwise), picking a basic per the
        /// deck's color identity
        #[arg(long = "backfill-basics")]
        backfill_basics: bool,
        /// Emit JSON (dry-run preview or the update summary)
        #[arg(long)]
        json: bool,
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
        /// Structured role: draw, ramp, removal, board-wipe, counterspell,
        /// wincon, tutor, sacrifice, reanimate, recursion, token, anthem,
        /// equipment, evasion, burn, lifegain, mill, discard, stax, tax,
        /// combo, storm, blink, landfall, artifact, enchantment,
        /// planeswalker, voltron, spellslinger, typal, group-hug, ...
        #[arg(long = "role", value_name = "ROLE")]
        role: Option<String>,
        /// Find commander candidates for the deck (theme-matched, P/T-legal)
        #[arg(long = "commander")]
        commander: bool,
        /// Format the deck plays (default: inferred from the deck's
        /// sections). Filters suggestions to cards legal in that format;
        /// commander-required combos are excluded for 60-card formats.
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Bracket 1-5: filter suggestions that break the bracket (brackets
        /// 1-2 allow no Game Changers)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Budget cap: drop candidates priced above this amount in US
        /// dollars (USD); unpriced candidates are excluded before ranking
        #[arg(long = "max-price", value_name = "USD", value_parser = parse_max_price)]
        max_price: Option<f64>,
        /// Maximum results (default 10)
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=50))]
        limit: u32,
        /// Restrict candidates to cards the collection owns
        #[arg(long)]
        owned: bool,
        /// Card names or a text file of names (one per line, `#`
        /// comments allowed) never to suggest
        #[arg(long = "exclude", value_name = "NAME|FILE")]
        exclude: Vec<String>,
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
        /// Commander bracket 1-5 for the mana-base target band (default 3)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Rank a deck's incumbents by expendability (guided cutting)
    Cuts {
        name: String,
        /// Maximum cut rows (default 5)
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=50))]
        count: u32,
        /// Pair each cut with fills for this role (deficit role N)
        #[arg(long = "for", value_name = "ROLE")]
        for_role: Option<String>,
        /// Commander bracket 1-5: Game Changers over the allowance pin to
        /// the top
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        bracket: Option<u8>,
        /// Format to judge legality against (default: inferred from the
        /// deck's sections)
        #[arg(long = "format", value_name = "FMT")]
        format: Option<String>,
        /// Budget cap for `--for` fill candidates: drop candidates priced
        /// above this amount in US dollars (USD); unpriced candidates are
        /// excluded
        #[arg(long = "max-price", value_name = "USD", value_parser = parse_max_price)]
        max_price: Option<f64>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Audit a deck's Commander Spellbook combos, per section
    Combos {
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

    /// Diff two decks per section: exact change instructions
    /// (original → optimized)
    Diff {
        /// Base deck name, or a ManaBox deck txt file path
        deck_a: String,
        /// Target deck name, or a ManaBox deck txt file path
        deck_b: String,
        /// Diff exact printings (name + set + cn + foil) instead of card
        /// names (any printing fills a slot)
        #[arg(long = "exact")]
        exact: bool,
        /// Emit JSON
        #[arg(long)]
        json: bool,
        /// Emit the change-log markdown (remove/add instruction table)
        #[arg(long = "markdown")]
        markdown: bool,
        /// Emit `deck update --from` op lines (remove/set/add) instead of
        /// a diff table
        #[arg(long = "as-update")]
        as_update: bool,
    },

    /// Export a deck to a ManaBox txt file
    Export {
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

    /// Duplicate a decklist and its primer under a new name
    Copy {
        /// Source deck name
        source: String,
        /// New deck name to create
        destination: String,
        /// Overwrite the destination decklist if it exists
        #[arg(long)]
        force: bool,
    },

    /// Print or replace a deck's primer markdown
    Primer {
        name: String,
        /// Replace the primer's contents from a markdown file, or `-` for
        /// stdin
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
        return Err("--max-price must be a finite USD amount".to_string());
    }
    if v < 0.0 {
        return Err("--max-price must be zero or positive".to_string());
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
        // --offline is a global flag: it lives on the Cli, not per command.
        let cli = Cli::try_parse_from(["stm", "query", "x", "--offline"]).expect("parse");
        assert!(cli.offline);
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
