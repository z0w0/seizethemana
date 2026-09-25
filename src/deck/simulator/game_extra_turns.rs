// Extra-turn replay for the goldfish game loop: queued extra turns
// replay a land drop, a draw, and one firing of each upkeep engine
// (Time Sieve class). Split from game_run.rs to keep files small.

use super::game::{GameState, new_perm_with, take_uid};
use super::game_commander::{commander_sentinel_slot, is_commander_sentinel};
use super::game_effects::apply_effect;
use super::game_run::{TurnCensus, run_scaling_draw};
use super::model::{Effect, Role, SimDeck, Trigger};

/// 11 EXTRA TURNS: queued extra turns replay one full minimal pass: a
/// land drop (recorded in `land_drops[]` like the main schedule, so the
/// total drops on one turn index stay within the deck's per-turn cap of
/// three), the draw, and upkeep engines firing once. The queue drains
/// while schedule turns remain.
pub(super) fn replay_extra_turns(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    engines: &[(u32, u32)],
    turn: usize,
    pending_extra_turns: &mut u32,
) {
    while *pending_extra_turns > 0
        && turn as u32 + *pending_extra_turns <= census.land_drops.len() as u32
    {
        *pending_extra_turns -= 1;
        if let Some(i) = st.library.pop() {
            st.hand.push(i);
            st.seen += 1;
            st.awareness_cards += 1;
        }
        if census.land_drops[turn - 1] < 3 {
            let land_pos = st
                .hand
                .iter()
                .position(|idx| deck.cards[*idx].role == Role::Land);
            if let Some(pos) = land_pos {
                let idx = st.hand.remove(pos);
                census.land_drops[turn - 1] += 1;
                st.battlefield_seen.entry(idx).or_insert(turn as u32);
                let uid = take_uid(st);
                st.battlefield
                    .push(new_perm_with(uid, deck, idx, turn as u32, false));
            }
        }
        for (uid, draws) in engines.iter().copied() {
            fire_extra_turn_engine(deck, st, uid, draws, turn);
        }
    }
}

/// One engine's firing on a replayed extra turn: the commander sentinel
/// resolves to its commander's own abilities; a battlefield engine
/// fires its host card's upkeep abilities (scaling engines scale here
/// too).
fn fire_extra_turn_engine(deck: &SimDeck, st: &mut GameState, uid: u32, draws: u32, turn: usize) {
    if is_commander_sentinel(uid) {
        // Commander sentinel: the cast commander permanents
        // (is_commander, on the battlefield) gate the firing; each cast
        // commander's own abilities fire once.
        if !st.battlefield.iter().any(|p| p.is_commander) {
            return;
        }
        let cmd = deck
            .commanders
            .get(commander_sentinel_slot(uid))
            .unwrap_or(&deck.commanders[0]);
        let effects: Vec<Effect> = cmd
            .abilities()
            .filter(|a| a.trigger == Trigger::OnUpkeep)
            .map(|a| a.effect.clone())
            .collect();
        let mill_opp = cmd.mills_opponent;
        for effect in &effects {
            if let Effect::Draw(n) = effect {
                for _ in 0..*n {
                    if let Some(i) = st.library.pop() {
                        st.hand.push(i);
                        st.seen += 1;
                        st.awareness_cards += 1;
                    }
                }
                continue;
            }
            apply_effect(deck, effect, st, turn as u32, mill_opp);
        }
        return;
    }
    let Some(p) = st.battlefield.iter().find(|p| p.uid == uid) else {
        return;
    };
    let p = p.clone();
    if draws > 0 {
        for _ in 0..draws {
            if let Some(i) = st.library.pop() {
                st.hand.push(i);
                st.seen += 1;
                st.awareness_cards += 1;
            }
        }
        return;
    }
    let host_card = p.card;
    let mill_opp = deck.cards.get(host_card).is_some_and(|c| c.mills_opponent);
    let effects: Vec<Effect> = deck
        .cards
        .get(host_card)
        .map(|c| {
            c.abilities()
                .filter(|a| a.trigger == Trigger::OnUpkeep)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.effect)
        .collect();
    for effect in &effects {
        if let Effect::Draw(n) = effect {
            run_scaling_draw(deck, st, host_card, *n);
            continue;
        }
        apply_effect(deck, effect, st, turn as u32, mill_opp);
    }
}
