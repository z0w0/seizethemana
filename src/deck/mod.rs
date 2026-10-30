//! Deck commands: `create`, `list`, `show`, `update`, `import`, `export`,
//! `primer`, `buylist`, `legal`, `simulate`.
//!
//! Submodules:
//! - `grammar`: the ManaBox txt parser/serializer (pure, no I/O)
//! - `store`: deck files on disk + `create`/`list`/`delete` + ownership lookups
//! - `store_show`: the `deck show` overview, JSON view, and per-card table
//! - `bracket`: the bracket checklist (Game Changers, bracket signals)
//! - `cuts`: `deck cuts` candidates
//! - `diff`: `deck diff`
//! - `land_colors`: per-land producible colors (suggest/cuts ranking)
//! - `mana`: mana-base suggestion lines
//! - `mana_audit`: Karsten colored-source census (`deck mana`)
//! - `ownership`: collection/deck copy accounting
//! - `role`: card role taxonomy
//! - `suggest`: `deck suggest` (main + commander paths)
//! - `suggest_combo`: `deck suggest` combo completions
//! - `update`: update-op parsing/math + `deck update`
//! - `io`: `import`/`export`/`primer` file plumbing
//! - `buylist`: `deck buylist` missing-copies math + store CSV renderers
//! - `legal`: format/bracket legality checks
//! - `stats`: deck overview stats (curve, ramp, colors, types)
//! - `simulator`: the goldfish Monte Carlo simulation (`deck simulate`),
//!   split into model/parse/deck/game/aggregate/report submodules

/// The bracket checklist for Commander decks.
pub mod bracket;
/// `deck buylist` missing-copies math and store CSV renderers.
pub mod buylist;
/// `deck combos` bracket-aware combo audit.
pub mod combos;
/// `deck cuts` guided cutting candidates.
pub mod cuts;
/// `deck diff` change instructions between two decks.
pub mod diff;
/// ManaBox deck txt parser and serializer.
pub mod grammar;
/// `deck hand` opening-hand sampling.
pub mod hand;
/// Deck file I/O: import, export, and primer handling.
pub mod io;
/// External decklist formats (Moxfield, Archidekt, Arena).
pub mod io_external;
/// Per-land producible colors for suggest and cuts ranking.
pub mod land_colors;
/// Format and Commander-bracket legality checks.
pub mod legal;
/// Deck dedupe and maintenance.
pub mod maintain;
/// `deck mana` mana-base suggestion lines.
pub mod mana;
/// Karsten colored-source census for `deck mana`.
pub mod mana_audit;
/// `deck update` operation verbs and their parsing.
pub mod ops;
/// Collection and deck copy accounting.
pub mod ownership;
/// Card role taxonomy for `deck suggest`.
pub mod role;
/// The goldfish Monte Carlo simulation for `deck simulate`.
pub mod simulator;
/// Deck overview stats: curve, ramp, colors, and types.
pub mod stats;
/// Deck file storage and read commands.
pub mod store;
/// The `deck show` overview, JSON view, and per-card table.
pub mod store_show;
/// `deck suggest` role and theme suggestions.
pub mod suggest;
/// `deck suggest` combo completions.
pub mod suggest_combo;
/// `deck update` command entry point.
pub mod update;
/// Decklist fetching from hosted deck sites.
pub mod url_fetch;

// Command entry points, re-exported flat for `deck::<cmd>` dispatch.
pub use buylist::buylist;
pub use combos::combos;
pub use cuts::cuts;
pub use diff::{diff, diff_as_update};
pub use hand::hand;
pub use io::{export, import, primer};
pub use mana::{mana, mana_audit_for};
pub use store::{copy, create, delete, list};
pub use store_show::show;
pub use update::update;

/// Re-export the grammar types (the module's public surface).
pub use grammar::Deck;
