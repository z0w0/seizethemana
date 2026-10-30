//! Opening-hand dealing shared by the simulator and `stm deck hand`.
//! Shuffle the library, draw the opener, and apply the format's mulligan policy
//! using the caller's random stream.

use super::game::OPENING_HAND;
use super::model::CardIdx;
use super::model::{Role, SimDeck};
use rand_chacha::ChaCha8Rng;

/// The opening hand after the format's mulligan policy ran.
#[derive(Debug)]
pub struct Opener {
    /// The kept hand (indexes into `SimDeck.cards`).
    pub hand: Vec<CardIdx>,
    /// The shuffled library with the kept hand removed (draw order state
    /// for the rest of the game).
    pub library: Vec<CardIdx>,
    /// Land count of the kept hand (MDFC spell faces count land-able).
    pub lands: u8,
    /// True when the opener was redrawn under the format's policy.
    pub mulliganed: bool,
    /// Redraws chosen by the pilot, including a free multiplayer redraw.
    pub redraws: u8,
    /// Cards put on the bottom after keeping, in selection order.
    pub bottomed: Vec<CardIdx>,
}

/// Draw n cards off the bottom of the shuffled library.
pub(super) fn take_n(library: &mut Vec<CardIdx>, n: usize) -> Vec<CardIdx> {
    let n = n.min(library.len());
    library.split_off(library.len() - n)
}

/// Return a rejected hand to the deck and shuffle the full library again.
pub(super) fn reshuffle_hand(
    library: &mut Vec<CardIdx>,
    hand: &mut Vec<CardIdx>,
    rng: &mut ChaCha8Rng,
) {
    library.append(hand);
    shuffle_library(library, rng);
}

/// Shuffle card indexes in place using the game's seeded random stream.
fn shuffle_library(library: &mut [CardIdx], rng: &mut ChaCha8Rng) {
    super::rng::shuffle(library, rng);
}

/// Position of the card to bottom from a redrawn 7-card hand: a land at
/// 4+ lands (shed the flood), a spell below that (a land/spell MDFC
/// counts as land-able and is not bottomed for its spell face).
pub(super) fn bottom_position(deck: &SimDeck, hand: &[CardIdx], opener_lands: u8) -> usize {
    if opener_lands >= 4 {
        hand.iter()
            .position(|i| deck[*i].role == Role::Land)
            .unwrap_or(0)
    } else {
        hand.iter()
            .position(|i| deck[*i].role != Role::Land && !deck[*i].is_mdfc_spell)
            .unwrap_or(0)
    }
}

/// Land-able count of a hand: lands plus MDFCs (a land/spell MDFC
/// plays as a land when nothing else is available, so the mulligan
/// policy counts it land-able).
pub fn count_lands_in(deck: &SimDeck, hand: &[CardIdx]) -> u8 {
    hand.iter()
        .filter(|i| deck[**i].role == Role::Land || deck[**i].is_mdfc_spell)
        .count() as u8
}

/// Shuffle the library and deal the opening hand under the deck's
/// mulligan policy. The sim calls this at the top of every game; `deck
/// hand` calls it directly, so seed N here deals the same opener as the
/// sim's game #1.
pub fn deal_opener(deck: &SimDeck, rng: &mut ChaCha8Rng) -> Opener {
    let card_count = u32::try_from(deck.cards.len()).expect("deck size fits u32");
    // The companion starts outside the game (CR 702.139a), so it never
    // enters the shuffle.
    let mut library: Vec<CardIdx> = (0..card_count)
        .map(CardIdx)
        .filter(|idx| deck.companion != Some(*idx))
        .collect();
    shuffle_library(&mut library, rng);
    // Draw the opening hand from the bottom of the shuffled vec.
    let mut hand = take_n(&mut library, OPENING_HAND);

    // Mulligan policy comes from the format rules: Commander redraws once
    // outside its land band; constructed keeps a land-band hand or follows
    // London mulligans, bottoming one card per mulligan taken.
    let mut opener_lands = count_lands_in(deck, &hand);
    let mut mulliganed = false;
    let mut redraws = 0;
    let mut bottomed = Vec::new();
    match deck.rules.mulligan {
        super::format::MulliganPolicy::FreeRedraw {
            land_band: (lo, hi),
        } => {
            if !(lo..=hi).contains(&opener_lands) {
                mulliganed = true;
                redraws = 1;
                reshuffle_hand(&mut library, &mut hand, rng);
                hand = take_n(&mut library, OPENING_HAND);
                opener_lands = count_lands_in(deck, &hand);
            }
        }
        super::format::MulliganPolicy::London => {
            let mut mulligan_count = 0usize;
            while hand.len() == OPENING_HAND
                && !(2..=5).contains(&opener_lands)
                && mulligan_count < OPENING_HAND - 1
            {
                mulliganed = true;
                reshuffle_hand(&mut library, &mut hand, rng);
                hand = take_n(&mut library, OPENING_HAND);
                mulligan_count += 1;
                redraws += 1;
                opener_lands = count_lands_in(deck, &hand);
            }
            for _ in 0..mulligan_count.min(hand.len()) {
                let pos = bottom_position(deck, &hand, opener_lands);
                let card = hand.remove(pos);
                bottomed.push(card);
                library.insert(0, card);
                opener_lands = count_lands_in(deck, &hand);
            }
        }
    }
    Opener {
        hand,
        library,
        lands: opener_lands,
        mulliganed,
        redraws,
        bottomed,
    }
}

#[cfg(test)]
#[path = "tests/deal_tests.rs"]
mod deal_tests;
