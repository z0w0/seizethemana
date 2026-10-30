//! Connect the external schedule to supported player-owned triggered abilities.

use super::OpponentEvent;
use crate::deck::simulator::{
    game::{GameState, ManaPool, card_of, condition_met, permanent_abilities},
    model::{Cost, Keyword, OpponentTax, SimDeck, SimTrigger},
};

/// Play the scheduled opponent turns between two normal player turns.
pub(in crate::deck::simulator) fn run_opponent_round(
    deck: &SimDeck,
    state: &mut GameState,
    player_turn: u32,
) {
    for seat in 0..state.opponents.seats.len() {
        state.triggered_this_turn.clear();
        if let Some(event) = state.opponents.begin_turn(seat) {
            resolve_event(deck, state, seat, event, player_turn);
        }
        if state.completion.is_some() {
            return;
        }
        if let Some(event) = state.opponents.main_action(seat) {
            resolve_event(deck, state, seat, event, player_turn);
            if state.completion.is_some() {
                return;
            }
            state.opponents.resolve_main_spell(seat);
        }
    }
}

/// Snapshot event listeners before any reward changes the player's battlefield.
fn resolve_event(
    deck: &SimDeck,
    state: &mut GameState,
    seat: usize,
    event: OpponentEvent,
    player_turn: u32,
) {
    let pending: Vec<_> = state
        .battlefield
        .iter()
        .flat_map(|source| {
            permanent_abilities(deck, source)
                .filter(|ability| matches_event(ability.trigger, event))
                .filter(|ability| {
                    ability
                        .condition
                        .is_none_or(|condition| condition_met(deck, state, &condition))
                })
                .map(|ability| {
                    (
                        source.uid,
                        source.card.deck_idx(),
                        source_power(deck, state, source),
                        ability,
                    )
                })
        })
        .collect();
    for (uid, card, last_power, ability) in pending {
        if state.completion.is_some() {
            return;
        }
        if ability.once_per_turn && !state.triggered_this_turn.insert((uid, ability.id)) {
            continue;
        }
        if ability
            .condition
            .is_some_and(|condition| !condition_met(deck, state, &condition))
        {
            continue;
        }
        let tax = ability.opponent_tax.map(|tax| match tax {
            OpponentTax::Fixed(amount) => amount,
            OpponentTax::SourcePower => state
                .battlefield
                .iter()
                .find(|source| source.uid == uid)
                .map_or(last_power, |source| source_power(deck, state, source)),
        });
        if tax.is_some_and(|amount| state.opponents.pay_tax(seat, amount)) {
            continue;
        }
        crate::deck::simulator::resolution::begin_resolution(state);
        for effect in ability.effect_sequence() {
            if state.completion.is_some() {
                break;
            }
            crate::deck::simulator::game_effects::apply_effect_at(
                deck,
                effect,
                state,
                player_turn,
                false,
                card,
            );
        }
        crate::deck::simulator::resolution::end_resolution(deck, state, None);
        state.opponents.settle_losses();
    }
}

/// Use the same external event for all engines that observe a cast or draw.
fn matches_event(trigger: SimTrigger, event: OpponentEvent) -> bool {
    match event {
        OpponentEvent::Draw => trigger == SimTrigger::OpponentDraw,
        OpponentEvent::Cast {
            noncreature,
            first_noncreature,
        } => {
            trigger == SimTrigger::OpponentSpellCast
                || noncreature && trigger == SimTrigger::OpponentNoncreatureCast
                || first_noncreature && trigger == SimTrigger::OpponentFirstNoncreatureCast
        }
    }
}

/// Retain source power for taxes whose source can leave before resolution.
fn source_power(
    deck: &SimDeck,
    state: &GameState,
    source: &crate::deck::simulator::game::Permanent,
) -> u32 {
    let mut power = i64::from(card_of(deck, source).printed_power.unwrap_or(0))
        + i64::from(source.counters.plus1)
        - i64::from(source.counters.minus1);
    for permanent in &state.battlefield {
        let card = card_of(deck, permanent);
        if let Some((bonus, _)) = card.flags.buff {
            power += i64::from(bonus);
        }
        if permanent.equipped
            && permanent.equip_host == Some(source.uid)
            && let Some(equipment) = card.flags.equipment
        {
            power += i64::from(equipment.buff.0);
        }
    }
    power.clamp(0, i64::from(u32::MAX)) as u32
}

/// Pay supported mana cumulative upkeep using real sources, or sacrifice it.
/// The pool exists only in upkeep and does not carry into the main phase.
pub(in crate::deck::simulator) fn pay_cumulative_upkeep(
    deck: &SimDeck,
    state: &mut GameState,
    turn: u32,
) {
    let sources: Vec<_> = state
        .battlefield
        .iter()
        .filter_map(|source| {
            card_of(deck, source)
                .cumulative_upkeep
                .map(|cost| (source.uid, cost))
        })
        .collect();
    let mut pool = ManaPool::default();
    for (uid, base) in sources {
        if state.completion.is_some() {
            return;
        }
        if !state.battlefield.iter().any(|source| source.uid == uid) {
            continue;
        }
        let age = state.opponents.upkeep_ages.entry(uid).or_default();
        *age += 1;
        let cost = scaled_upkeep(base, *age);
        fund_upkeep(deck, state, &mut pool, &cost, turn);
        if crate::deck::simulator::game_mana::payable(&cost, &pool) {
            crate::deck::simulator::game_mana::pay_cost(&cost, &mut pool);
        } else {
            crate::deck::simulator::game_effects::resolve_sacrifice_uid(deck, state, turn, uid);
        }
    }
}

/// Cumulative upkeep repeats the printed payment once per age counter.
fn scaled_upkeep(mut cost: Cost, age: u32) -> Cost {
    cost.generic = cost.generic.saturating_mul(age);
    for pip in &mut cost.pips {
        *pip = u8::try_from(u32::from(*pip) * age).expect("upkeep horizon fits pip storage");
    }
    cost
}

/// Activate sources only while the current upkeep payment needs more mana.
fn fund_upkeep(deck: &SimDeck, state: &mut GameState, pool: &mut ManaPool, cost: &Cost, turn: u32) {
    let sources: Vec<_> = state.battlefield.iter().map(|source| source.uid).collect();
    for uid in sources {
        if crate::deck::simulator::game_mana::payable(cost, pool) {
            return;
        }
        let Some(source) = state
            .battlefield
            .iter()
            .find(|source| source.uid == uid)
            .cloned()
        else {
            continue;
        };
        if source.tapped {
            continue;
        }
        if crate::deck::simulator::game::is_creature_permanent(deck, &source)
            && source.summoning_sick
            && !crate::deck::simulator::game::effective_keywords(deck, state, &source)
                .contains(Keyword::Haste)
        {
            continue;
        }
        crate::deck::simulator::game_run::mana::tap_for_nonspell_payment(
            deck, state, &source, pool, turn,
        );
    }
    while !crate::deck::simulator::game_mana::payable(cost, pool) && state.treasure_bank > 0 {
        state.treasure_bank -= 1;
        pool.flexible += 1;
    }
}

#[cfg(test)]
#[path = "../tests/opponent_runtime_tests.rs"]
mod opponent_runtime_tests;
