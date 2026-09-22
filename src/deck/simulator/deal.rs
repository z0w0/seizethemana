// Opening-hand dealing for the goldfish simulator, split from
// `game_run.rs` to serve both the sim and `stm deck hand`: shuffle the
// library, draw the opener, apply the format's mulligan policy. Pure
// apart from the passed RNG.

use super::game::OPENING_HAND;
use super::model::{Role, SimDeck};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// The opening hand after the format's mulligan policy ran.
#[derive(Debug)]
pub struct Opener {
    /// Card indices into `SimDeck.cards` (the kept hand).
    pub hand: Vec<usize>,
    /// The shuffled library with the kept hand removed (draw order state
    /// for the rest of the game).
    pub library: Vec<usize>,
    /// Land count of the kept hand (MDFC spell faces count land-able).
    pub lands: u8,
    /// True when the opener was redrawn under the format's policy.
    pub mulliganed: bool,
}

/// Draw n cards off the bottom of the shuffled library.
pub(super) fn take_n(library: &mut Vec<usize>, n: usize) -> Vec<usize> {
    let n = n.min(library.len());
    library.split_off(library.len() - n)
}

/// Position of the card to bottom from a redrawn 7-card hand: a land at
/// 4+ lands (shed the flood), a spell below that (a land/spell MDFC
/// counts as land-able and is not bottomed for its spell face).
pub(super) fn bottom_position(deck: &SimDeck, hand: &[usize], opener_lands: u8) -> usize {
    if opener_lands >= 4 {
        hand.iter()
            .position(|i| deck.cards[*i].role == Role::Land)
            .unwrap_or(0)
    } else {
        hand.iter()
            .position(|i| deck.cards[*i].role != Role::Land && !deck.cards[*i].is_mdfc_spell)
            .unwrap_or(0)
    }
}

/// Land-able count of a hand: lands plus MDFCs (a land/spell MDFC
/// plays as a land when nothing else is available, so the mulligan
/// policy counts it land-able).
pub fn count_lands_in(deck: &SimDeck, hand: &[usize]) -> u8 {
    hand.iter()
        .filter(|i| deck.cards[**i].role == Role::Land || deck.cards[**i].is_mdfc_spell)
        .count() as u8
}

/// Karsten's London mulligan for a 7-card opener outside the 2-5 land
/// keep band: redraw a fresh 7, then bottom one card toward 3 lands (a
/// land at 4+ lands, a spell below that). Only the redrawn hand bottoms;
/// kept hands stay at 7 cards. Returns the 6-card hand.
pub(super) fn london_mulligan(
    deck: &SimDeck,
    library: &mut Vec<usize>,
    rng: &mut ChaCha8Rng,
) -> Vec<usize> {
    let mut hand = take_n(library, OPENING_HAND);
    let opener_lands = count_lands_in(deck, &hand);
    let pos = bottom_position(deck, &hand, opener_lands);
    let card = hand.remove(pos);
    let at = rng.random_range(0..=library.len());
    library.insert(at, card);
    hand
}

/// Shuffle the library and deal the opening hand under the deck's
/// mulligan policy. The sim calls this at the top of every game; `deck
/// hand` calls it directly, so seed N here deals the same opener as the
/// sim's game #1.
pub fn deal_opener(deck: &SimDeck, rng: &mut ChaCha8Rng) -> Opener {
    let mut library: Vec<usize> = (0..deck.cards.len()).collect();
    for i in (1..library.len()).rev() {
        let j = rng.random_range(0..=i);
        library.swap(i, j);
    }
    // Draw the opening hand from the bottom of the shuffled vec.
    let mut hand = take_n(&mut library, OPENING_HAND);

    // Mulligan policy comes from the format rules: commander family
    // redraws once outside its land band; constructed plays London
    // mulligans (redraw, then bottom one chosen card, keeping six).
    let mut opener_lands = count_lands_in(deck, &hand);
    let mut mulliganed = false;
    match deck.rules.mulligan {
        super::format::MulliganPolicy::FreeRedraw {
            land_band: (lo, hi),
        } => {
            if !(lo..=hi).contains(&opener_lands) {
                mulliganed = true;
                // Bottom the shipped hand into the library (the London
                // pattern): every mulliganed game keeps the full 99-card
                // library instead of silently playing 7 cards short.
                library.splice(0..0, hand.drain(..));
                hand = take_n(&mut library, OPENING_HAND);
                opener_lands = count_lands_in(deck, &hand);
            }
        }
        super::format::MulliganPolicy::London => {
            let redraw = !(2..=5).contains(&opener_lands) && hand.len() >= OPENING_HAND;
            if redraw {
                mulliganed = true;
                // Bottom the shipped hand first (the same card-preservation
                // rule the FreeRedraw policy follows); the London redraw
                // then bottoms one chosen card from the fresh hand.
                library.splice(0..0, hand.drain(..));
                hand = london_mulligan(deck, &mut library, rng);
                // The mulligan policy reads the redrawn hand's count;
                // the reported opener is the kept (bottomed) hand.
                opener_lands = count_lands_in(deck, &hand);
            }
        }
    }
    Opener {
        hand,
        library,
        lands: opener_lands,
        mulliganed,
    }
}
