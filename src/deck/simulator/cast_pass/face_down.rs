//! Face-down casting and turning face up (CR 702.37/702.168). A card
//! with morph, megamorph, or disguise may be cast as a 2/2 face-down
//! body for {3}; it turns up later for its morph cost.

use super::super::game::{GameState, ManaPool, card_of, new_perm_with, take_uid};
use super::super::game_mana::{pay_cost, payable, pips_ok};
use super::super::model::{CardIdx, Cost, SimDeck};

/// Cast a card face down (morph or disguise, CR 702.37/702.168): pay
/// {3}, put a 2/2 face-down body on the battlefield. The permanent has
/// no name, text, or abilities until it is turned face up, so it fires
/// no enter triggers and registers no engines. The {3} counts as mana
/// spent this turn.
pub(in crate::deck::simulator) fn resolve_face_down(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    idx: CardIdx,
    cast_ids: &mut Vec<CardIdx>,
    spent_total: &mut u32,
) {
    let cost = Cost {
        generic: 3,
        ..Cost::default()
    };
    pay_cost(&cost, pool);
    *spent_total += cost.total();
    cast_ids.push(idx);
    if let Some(pos) = st.hand.iter().position(|held| *held == idx) {
        st.hand.remove(pos);
    }
    let uid = take_uid(st);
    let mut entry = new_perm_with(uid, deck, idx, turn as u32, false);
    entry.face_down = true;
    entry.summoning_sick = true;
    st.battlefield_seen.entry(idx).or_insert(turn as u32);
    st.battlefield.push(entry);
}

/// Turn face-down permanents face up (CR 702.37e/702.168d) while the
/// mana pool covers the morph or disguise cost. Turning up does not
/// re-fire enter triggers. Best case: as soon as the cost is payable.
pub(in crate::deck::simulator) fn turn_face_up(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    mana_spent: &mut f64,
) {
    let mut flipped = Vec::new();
    for (pos, perm) in st.battlefield.iter().enumerate() {
        if !perm.face_down {
            continue;
        }
        let Some(cost) = card_of(deck, perm).morph_cost else {
            continue;
        };
        if payable(&cost, pool) && pips_ok(&cost, pool) {
            flipped.push((pos, cost));
        }
    }
    for (pos, cost) in flipped {
        pay_cost(&cost, pool);
        *mana_spent += cost.total() as f64;
        if let Some(perm) = st.battlefield.get_mut(pos) {
            perm.face_down = false;
        }
    }
}
