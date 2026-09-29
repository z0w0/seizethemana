//! Keyword-action and counter effects for the goldfish game loop
//! (CR 701): amass, the Ring, empower Jace, explore, connive, mobilize,
//! and proliferate. Split from `game_effects` to keep files small.

use super::super::game::{CardRef, GameState, new_token_perm, take_uid};
use super::super::game_effects::{CardZone, draw_one, move_to_graveyard};
use super::super::model::SimDeck;

/// Amass N (CR 701.47): if the player controls no Army, create a 0/0
/// Army body; then put N +1/+1 counters on the chosen Army. The subtype
/// parameter does not change the goldfish math.
pub(in crate::deck::simulator) fn amass(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    amount: u32,
) {
    let existing = st.battlefield.iter().position(|p| p.army);
    let pos = match existing {
        Some(pos) => pos,
        None => {
            let uid = take_uid(st);
            let mut token = new_token_perm(uid, turn);
            token.army = true;
            st.battlefield.push(token);
            st.battlefield.len() - 1
        }
    };
    st.battlefield[pos].counters.plus1 += amount;
    let _ = deck;
}

/// The Ring tempts the player (CR 701.54c/d): the Ring emblem's level
/// rises, the best attacker becomes the Ring-bearer, and "Whenever the
/// Ring tempts you" triggers fire once per temptation.
pub(in crate::deck::simulator) fn ring_tempts(deck: &SimDeck, st: &mut GameState, turn: u32) {
    st.ring_tempts += 1;
    // Ring-bearer (CR 701.54a): choose a creature; best case the
    // strongest untapped body, so the level-2 attack loot and level-4
    // combat drain have the best chance to connect.
    st.ring_bearer = st
        .battlefield
        .iter()
        .filter(|p| super::super::game::is_creature_permanent(deck, p) && !p.face_down)
        .max_by_key(|p| super::super::game_effects::creature_power(p, deck))
        .map(|p| p.uid);
    super::super::game::fire_triggers(deck, st, super::super::model::SimTrigger::RingTempts, turn);
}

/// Empower Jace N (CR 701.71): create a Jace planeswalker token if none
/// exists, then add N loyalty counters to a Jace token the player
/// controls. The token has "[-1]: Surveil 1" and "[-3]: Draw a card".
pub(in crate::deck::simulator) fn empower_jace(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    amount: u32,
) {
    let existing = st.battlefield.iter().position(|p| p.jace_token);
    let pos = match existing {
        Some(pos) => pos,
        None => {
            let uid = take_uid(st);
            let mut token = new_token_perm(uid, turn);
            token.card = CardRef::Token(super::super::game::TokenKind::Jace);
            token.jace_token = true;
            st.battlefield.push(token);
            st.battlefield.len() - 1
        }
    };
    st.battlefield[pos].counters.loyalty += amount;
    let _ = deck;
}

/// Explore (CR 701.44): reveal the top card of the library. A land card
/// goes to the hand; otherwise a +1/+1 counter goes on the exploring
/// permanent. The goldfish reveals from its own library, so a nonland
/// stays on top and the counter is the net effect.
pub(in crate::deck::simulator) fn explore(deck: &SimDeck, st: &mut GameState, turn: u32) {
    let top_is_land = st
        .library
        .last()
        .is_some_and(|idx| deck[*idx].role == super::super::model::Role::Land);
    if top_is_land {
        // The land moves to hand through the normal draw path, which
        // counts it as seen.
        draw_one(deck, st, turn);
        return;
    }
    // Nonland: a +1/+1 counter joins the best body (the exploring
    // permanent is the highest-power untapped creature; best case).
    if let Some(pos) = st
        .battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| super::super::game::is_creature_permanent(deck, p))
        .max_by_key(|(_, p)| super::super::game_effects::creature_power(p, deck))
        .map(|(i, _)| i)
    {
        st.battlefield[pos].counters.plus1 += 1;
    }
}

/// Connive N (CR 701.50): draw N cards, discard N cards, then put a
/// +1/+1 counter on a conniving creature if a nonland card was
/// discarded. The goldfish treats the discards as nonland when the
/// discarded card was not a land.
pub(in crate::deck::simulator) fn connive(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    count: u32,
) {
    let mut discarded_nonland = false;
    for _ in 0..count {
        draw_one(deck, st, turn);
        // Discard the oldest card, matching the loot heuristic.
        if !st.hand.is_empty() {
            let discarded = st.hand.remove(0);
            if deck[discarded].role != super::super::model::Role::Land {
                discarded_nonland = true;
            }
            move_to_graveyard(deck, st, discarded, turn, CardZone::Hand);
        }
    }
    if discarded_nonland
        && let Some(pos) = st
            .battlefield
            .iter()
            .enumerate()
            .filter(|(_, p)| super::super::game::is_creature_permanent(deck, p))
            .max_by_key(|(_, p)| super::super::game_effects::creature_power(p, deck))
            .map(|(i, _)| i)
    {
        st.battlefield[pos].counters.plus1 += 1;
    }
}

/// Mobilize N (CR 702.181): create N 1/1 red Warrior tokens, tapped and
/// attacking; sacrifice them at the beginning of the next end step. The
/// tokens join this turn's attack (best case) and their sacrifice fires
/// death triggers.
pub(in crate::deck::simulator) fn mobilize(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    amount: u32,
) {
    let _ = deck;
    for _ in 0..amount.min(8) {
        let uid = take_uid(st);
        let mut token = new_token_perm(uid, turn);
        // Tapped and attacking: the token counts as an attacker this
        // turn even though it entered now.
        token.tapped = true;
        token.attacking_this_turn = true;
        token.sacrifice_at_end = true;
        st.battlefield.push(token);
    }
}

/// Proliferate (CR 701.34a): for each permanent and the player, add one
/// counter of each kind already present. Best case: every counter-bearing
/// permanent is chosen. Keyword counters do not stack, so they are
/// unchanged.
pub(in crate::deck::simulator) fn resolve_proliferate(st: &mut GameState) {
    for perm in &mut st.battlefield {
        if perm.counters.plus1 > 0 {
            perm.counters.plus1 += 1;
        }
        if perm.counters.minus1 > 0 {
            perm.counters.minus1 += 1;
        }
        if perm.counters.charge > 0 {
            perm.counters.charge += 1;
        }
        if perm.counters.loyalty > 0 {
            perm.counters.loyalty += 1;
        }
    }
    // Player counters proliferate too. Poison is out of scope in a
    // goldfish, so only energy moves.
    if st.player_counters.energy > 0 {
        st.player_counters.energy += 1;
    }
}
