//! Versioned random streams keyed by master seed, game ID, and purpose.
//!
//! Version 1 hashes the fixed prefix, little-endian version, master seed, game
//! ID, and one-byte domain with SHA-256, then uses the digest as a ChaCha8 key.
//! Deck contents, worker count, and batch size do not enter the seed. The shuffle
//! helper uses descending Fisher-Yates with rejection-sampled 64-bit indexes.

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sha2::{Digest, Sha256};

/// Seed derivation and sampling contract for simulator random streams.
pub const RNG_VERSION: u32 = 1;

/// Separate purposes must not consume another purpose's random stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RngDomain {
    /// Initial shuffle and all opening-hand mulligans.
    Opening = 1,
    /// Searches, shuffles, and random library effects after the opener.
    Library = 2,
    /// Random choices made by a pilot policy, when a policy uses randomness.
    Pilot = 3,
    /// External opponent activity, independent of the tested deck.
    Opponent = 4,
}

/// Recreate one game's stream without consuming any other game's random state.
pub fn game_rng(master_seed: u64, game_id: u64, domain: RngDomain) -> ChaCha8Rng {
    let mut hash = Sha256::new();
    hash.update(b"stm.simulator.rng\0");
    hash.update(RNG_VERSION.to_le_bytes());
    hash.update(master_seed.to_le_bytes());
    hash.update(game_id.to_le_bytes());
    hash.update([domain as u8]);
    ChaCha8Rng::from_seed(hash.finalize().into())
}

/// Shuffle with a fixed sampling algorithm that does not depend on `usize` width.
pub fn shuffle<T>(values: &mut [T], rng: &mut ChaCha8Rng) {
    for index in (1..values.len()).rev() {
        let upper = u64::try_from(index + 1).expect("slice length fits u64");
        let threshold = upper.wrapping_neg() % upper;
        let sampled = loop {
            let value = rng.next_u64();
            if value >= threshold {
                break value % upper;
            }
        };
        values.swap(index, sampled as usize);
    }
}

#[cfg(test)]
#[path = "tests/rng_tests.rs"]
mod rng_tests;
