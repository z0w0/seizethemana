// Library root: every `stm` capability lives here so benches and example
// binaries can use the same internals as the CLI. The binary (`main.rs`)
// owns only argv dispatch and the process-wide offline flag.

pub mod card;
pub mod cli;
pub mod collection;
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
