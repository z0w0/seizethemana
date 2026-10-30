//! Settle cast triggers and battlefield entries before the next normal action.

use super::super::game::{GameState, ManaPool, permanent_abilities};
use super::super::model::{CardIdx, SimCard, SimDeck, SimTrigger};
use super::super::resolution::{enqueue_trigger, prepare_trigger, settle_pending};

/// Resolve cast listeners from the battlefield before the spell resolves.
pub(in crate::deck::simulator) fn fire_cast_triggers(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
    spell: &SimCard,
) {
    let state = &*st;
    let listeners: Vec<_> = state
        .battlefield
        .iter()
        .flat_map(|source| {
            permanent_abilities(deck, source)
                .filter(|ability| {
                    ability.kind.is_triggered() && ability.trigger == SimTrigger::SpellCast
                })
                .filter(|ability| {
                    super::super::game::event_filter_matches(&ability.event_filter, spell, false)
                })
                .filter(|ability| {
                    ability.condition.is_none_or(|condition| {
                        super::super::game::condition_met(deck, state, &condition)
                    })
                })
                .filter_map(move |ability| prepare_trigger(deck, state, source, ability, turn))
        })
        .collect();
    for trigger in listeners {
        enqueue_trigger(st, trigger);
    }
    settle_pending(deck, st, Some(pool));
}

/// Fire entry triggers and register upkeep engines for resolved entries.
pub(super) fn settle_entries(
    deck: &SimDeck,
    st: &mut GameState,
    _turn: u32,
    entries: &mut Vec<(u32, CardIdx)>,
    repeatable_sources: &mut Vec<(u32, u32)>,
) {
    for (uid, card_idx) in std::mem::take(entries) {
        let Some(position) = st
            .battlefield
            .iter()
            .position(|permanent| permanent.uid == uid)
        else {
            continue;
        };
        let permanent = &st.battlefield[position];
        for ability in deck[card_idx].unlocked_abilities(permanent.counters.charge) {
            if let Some(draws) = super::upkeep_trigger_registration(ability) {
                super::super::game::register_repeatable_source(repeatable_sources, uid, draws);
            }
        }
    }
}
