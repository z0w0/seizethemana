// Library root: every `stm` capability lives here so benches and example
// binaries can use the same internals as the CLI. The binary (`main.rs`)
// owns only argv dispatch.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set when the user passed `--offline` anywhere; network-dependent code
/// (URL imports, background refreshes) reads this instead of threading the
/// flag through every call site.
static OFFLINE: AtomicBool = AtomicBool::new(false);

/// True when `--offline` was set at startup.
pub fn offline_requested() -> bool {
    OFFLINE.load(Ordering::Relaxed)
}

/// Store the `--offline` flag (called once from the binary's dispatch).
pub fn set_offline(offline: bool) {
    OFFLINE.store(offline, Ordering::Relaxed);
}

pub mod card;
pub mod cli;
pub mod collection;
pub mod collection_conflicts;
pub mod collection_stats;
pub mod combos;
pub mod db;
pub mod deck;
pub mod embed;
pub mod output;
pub mod paths;
pub mod prints;
pub mod quality;
pub mod query;
pub mod release;
pub mod scryfall;
pub mod search;
pub mod setup;
pub mod spellbook;
pub mod sync;
pub mod tags;
pub mod universe;
