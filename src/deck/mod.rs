// Deck commands: `create`, `list`, `show`, `update`, `import`, `export`,
// `primer`, `buylist`, `legal`, `simulate`.
//
// Submodules:
// - `grammar`: the ManaBox txt parser/serializer (pure, no I/O)
// - `store`: deck files on disk + `create`/`list`/`delete` + ownership lookups
// - `store_show`: the `deck show` overview, JSON view, and per-card table
// - `bracket`: the bracket checklist (Game Changers, bracket signals)
// - `cuts`: `deck cuts` candidates
// - `diff`: `deck diff`
// - `land_colors`: per-land producible colors (suggest/cuts ranking)
// - `mana`: mana-base suggestion lines
// - `mana_audit`: Karsten colored-source census (`deck mana`)
// - `ownership`: collection/deck copy accounting
// - `role`: card role taxonomy
// - `suggest`: `deck suggest` (main + commander paths)
// - `suggest_combo`: `deck suggest` combo completions
// - `update`: update-op parsing/math + `deck update`
// - `io`: `import`/`export`/`primer` file plumbing
// - `buylist`: `deck buylist` missing-copies math + store CSV renderers
// - `legal`: format/bracket legality checks
// - `stats`: deck overview stats (curve, ramp, colors, types)
// - `simulator`: the goldfish Monte Carlo simulation (`deck simulate`),
//   split into model/parse/deck/game/aggregate/report submodules

pub mod bracket;
pub mod buylist;
pub mod combos;
pub mod cuts;
pub mod diff;
pub mod grammar;
pub mod io;
pub mod land_colors;
pub mod legal;
pub mod mana;
pub mod mana_audit;
pub mod ownership;
pub mod role;
pub mod simulator;
pub mod stats;
pub mod store;
pub mod store_show;
pub mod suggest;
pub mod suggest_combo;
pub mod update;

// Command entry points, re-exported flat for `deck::<cmd>` dispatch.
pub use buylist::buylist;
pub use combos::combos;
pub use cuts::cuts;
pub use diff::diff;
pub use io::{export, import, primer};
pub use mana::{mana, mana_audit_for};
pub use store::{create, delete, list};
pub use store_show::show;
pub use update::update;

/// Re-export the grammar types (the module's public surface).
pub use grammar::Deck;
