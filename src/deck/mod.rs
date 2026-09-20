// Deck commands: `create`, `list`, `show`, `update`, `import`, `export`,
// `primer`, `buylist`, `legal`, `simulate`.
//
// Submodules:
// - `grammar`: the ManaBox txt parser/serializer (pure, no I/O)
// - `store`: deck files on disk + `create`/`list`/`show` + ownership lookups
// - `update`: update-op parsing/math + `deck update`
// - `io`: `import`/`export`/`primer` file plumbing
// - `buylist`: `deck buylist` missing-copies math + store CSV renderers
// - `legal`: format/bracket legality checks
// - `stats`: deck overview stats (curve, ramp, colors, types)
// - `simulator`: the goldfish Monte Carlo simulation (`deck simulate`),
//   split into model/parse/deck/game/aggregate/report submodules

pub mod buylist;
pub mod combos;
pub mod cuts;
pub mod diff;
pub mod grammar;
pub mod io;
pub mod legal;
pub mod ownership;
pub mod role;
pub mod simulator;
pub mod stats;
pub mod store;
pub mod suggest;
pub mod suggest_combo;
pub mod update;

// Command entry points, re-exported flat for `deck::<cmd>` dispatch.
pub use buylist::buylist;
pub use combos::combos;
pub use cuts::cuts;
pub use diff::diff;
pub use io::{export, import, primer};
pub use store::{create, delete, list, show};
pub use update::update;

/// Re-export the grammar types (the module's public surface).
pub use grammar::Deck;
