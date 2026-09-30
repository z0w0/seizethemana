//! Library root: every `stm` capability lives here so benches and example
//! binaries can use the same internals as the CLI. The binary (`main.rs`)
//! owns only argv dispatch.

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

/// Card detail and per-card lookups.
pub mod card;
/// Command-line surface: clap argument and subcommand definitions.
pub mod cli;
/// Collection import and owned-only search.
pub mod collection;
/// Cross-deck conflict report.
pub mod collection_conflicts;
/// Dead-money sell suggestions for binder cards.
pub mod collection_sell;
/// Whole-collection stats.
pub mod collection_stats;
/// Shared Spellbook combo reads.
pub mod combos;
/// SQLite layer: connection, migrations, and the shared card row.
pub mod db;
/// Deck building, legality, and goldfish simulation.
pub mod deck;
/// Local text embedding and the flat vector store.
pub mod embed;
/// Terminal output and styling.
pub mod output;
/// On-disk data locations and the `status.json` state file.
pub mod paths;
/// Per-printing price storage and lookups.
pub mod prints;
/// Hybrid card search: keyword plus meaning legs.
pub mod query;
/// Release-date helpers shared by ingest and display.
pub mod release;
/// Scryfall bulk download and ingest.
pub mod scryfall;
/// Structured card filters and matching.
pub mod search;
/// `stm setup`: download bulk data and build the index.
pub mod setup;
/// Commander Spellbook combo ingest and parsing.
pub mod spellbook;
/// `stm sync`: refresh card data, prices, and tags.
pub mod sync;
/// Scryfall oracle tags: parsing, ingest, and lookup.
pub mod tags;
/// Universes Beyond and franchise mapping.
pub mod universe;
