//! Regression tests for game-loop fixes: restricted-mana creature
//! casts, crew expiry, blink vs land-search vs Monarch separation,
//! graveyard timing, X-cost restricted-bucket wipes, fetch land pairs,
//! and awareness accounting.

use super::deck_test_support::card;
use super::game::run_game;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::*;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Build a deck from explicit card rows (count pairs).
fn deck_from(rows: &[(CardRow, usize)]) -> SimDeck {
    let mut cards = Vec::new();
    for (row, count) in rows {
        for _ in 0..*count {
            cards.push(parse_sim_card(row));
        }
    }
    SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    }
}

/// Board regression scenarios.
#[path = "game_fix_board_tests.rs"]
mod board_tests;

/// Zone trigger regression scenarios.
#[path = "game_fix_zone_trigger_tests.rs"]
mod zone_trigger_tests;

/// Payment regression scenarios.
#[path = "game_fix_payment_tests.rs"]
mod payment_tests;
