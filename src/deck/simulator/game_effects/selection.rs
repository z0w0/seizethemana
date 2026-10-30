//! Deterministic selection from public zones and explicitly exposed cards.

use super::{CardIdx, CardZone, GameState, SimDeck, draw_one, move_to_graveyard};
use crate::deck::simulator::game::is_land_permanent;
use crate::deck::simulator::resolution::{begin_resolution, end_resolution};

/// Rank one visible candidate; smaller keys prefer needed lands and low-cost spells.
///
/// The mana budget is the battlefield land count plus one. This is a selection
/// estimate, not a cast-permission check. The function never reads the library.
pub(in crate::deck::simulator) fn preference(
    deck: &SimDeck,
    st: &GameState,
    index: CardIdx,
) -> (u8, u32) {
    let card = &deck[index];
    let battlefield_lands = st
        .battlefield
        .iter()
        .filter(|perm| is_land_permanent(perm))
        .count();
    let hand_lands = st.hand.iter().filter(|index| deck[**index].is_land).count();
    if card.is_land {
        return (
            if battlefield_lands + hand_lands < 5 {
                0
            } else {
                3
            },
            0,
        );
    }
    let cost = card.min_cost.total();
    let affordable = cost <= battlefield_lands as u32 + 1;
    (if affordable { 1 } else { 2 }, cost)
}

/// Keep useful exposed cards on top and put the rest on the bottom.
pub(super) fn scry(deck: &SimDeck, st: &mut GameState, count: u32, _turn: u32) {
    if selection_stopped(st) {
        return;
    }
    let start = st.library.len().saturating_sub(count as usize);
    let exposed = st.library.split_off(start);
    st.awareness_cards += exposed.len() as u32;
    let (mut kept, rejected): (Vec<_>, Vec<_>) = exposed
        .into_iter()
        .rev()
        .partition(|index| preference(deck, st, *index).0 < 2);
    kept.sort_by_key(|index| preference(deck, st, *index));
    st.library.splice(0..0, rejected.into_iter().rev());
    st.library.extend(kept.into_iter().rev());
}

/// Move rejected exposed cards to the graveyard without reordering kept cards.
pub(super) fn surveil(deck: &SimDeck, st: &mut GameState, count: u32, turn: u32) {
    if selection_stopped(st) {
        return;
    }
    let start = st.library.len().saturating_sub(count as usize);
    st.awareness_cards += (st.library.len() - start) as u32;
    let rejected: Vec<_> = (start..st.library.len())
        .rev()
        .filter(|position| preference(deck, st, st.library[*position]).0 >= 2)
        .collect();
    begin_resolution(st);
    for position in rejected {
        let index = st.library.remove(position);
        st.seen += 1;
        st.milled_self += 1;
        move_to_graveyard(deck, st, index, turn, CardZone::Library);
    }
    end_resolution(deck, st, None);
}

/// Finish all draw actions, including dredge, before choosing any discards.
/// A failed draw does not skip later instructions before resolution ends.
pub(super) fn draw_discard(deck: &SimDeck, st: &mut GameState, draw: u32, discard: u32, turn: u32) {
    if selection_stopped(st) {
        return;
    }
    begin_resolution(st);
    for _ in 0..draw {
        draw_one(deck, st, turn);
        if selection_stopped(st) {
            break;
        }
    }
    if !selection_stopped(st) {
        for _ in 0..(discard as usize).min(st.hand.len()) {
            let position = worst_hand_position(deck, st)
                .expect("the bounded discard count requires a nonempty hand");
            let index = st.hand.remove(position);
            move_to_graveyard(deck, st, index, turn, CardZone::Hand);
        }
    }
    end_resolution(deck, st, None);
}

/// Reorder only the exposed top cards, or shuffle when none meet the policy.
pub(super) fn look_at_top(
    deck: &SimDeck,
    st: &mut GameState,
    count: u32,
    may_shuffle: bool,
    _turn: u32,
) {
    if selection_stopped(st) {
        return;
    }
    let start = st.library.len().saturating_sub(count as usize);
    let mut exposed = st.library.split_off(start);
    st.awareness_cards += exposed.len() as u32;
    let shuffle = may_shuffle
        && !exposed.is_empty()
        && exposed
            .iter()
            .all(|index| preference(deck, st, *index).0 >= 2);
    if shuffle {
        st.library.extend(exposed);
        crate::deck::simulator::rng::shuffle(&mut st.library, &mut st.library_rng);
    } else {
        exposed.reverse();
        exposed.sort_by_key(|index| preference(deck, st, *index));
        st.library.extend(exposed.into_iter().rev());
    }
}

/// Return the least useful current hand cards without a discard or draw event.
pub(super) fn put_hand_on_top(deck: &SimDeck, st: &mut GameState, count: u32, _turn: u32) {
    if selection_stopped(st) {
        return;
    }
    for _ in 0..(count as usize).min(st.hand.len()) {
        let position = worst_hand_position(deck, st)
            .expect("the bounded return count requires a nonempty hand");
        let index = st.hand.remove(position);
        st.library.push(index);
    }
}

/// Resolve equal scores by hand position, keeping the oldest tied card.
fn worst_hand_position(deck: &SimDeck, st: &GameState) -> Option<usize> {
    st.hand
        .iter()
        .enumerate()
        .max_by_key(|(_, index)| preference(deck, st, **index))
        .map(|(position, _)| position)
}

/// Stop after the game ends, not while a failed draw is waiting for the loss check.
fn selection_stopped(st: &GameState) -> bool {
    st.completion.is_some()
}

/// Selection checks keep rules, policy, physical zones, and counters separate.
#[cfg(test)]
#[path = "tests/selection_tests.rs"]
mod selection_tests;
