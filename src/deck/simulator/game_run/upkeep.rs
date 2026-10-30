//! Supported upkeep effects resolve as complete abilities before player-loss checks.

use super::*;
use crate::deck::simulator::{game, game_effects, model, resolution};

/// Discover live upkeep sources and execute each once in stable order.
pub(super) fn run_upkeep(
    deck: &SimDeck,
    st: &mut GameState,
    repeatable_sources: &mut Vec<(u32, u32)>,
    turn: usize,
) {
    if st.completion.is_some() {
        return;
    }
    let live_uids: std::collections::HashSet<u32> = st.battlefield.iter().map(|p| p.uid).collect();
    repeatable_sources.retain(|(uid, _)| live_uids.contains(uid));
    for source in &st.battlefield {
        if repeatable_sources.iter().any(|(uid, _)| *uid == source.uid) {
            continue;
        }
        if let Some(draws) = game::permanent_abilities(deck, source)
            .filter_map(super::super::cast_pass::upkeep_trigger_registration)
            .reduce(u32::saturating_add)
        {
            repeatable_sources.push((source.uid, draws));
        }
    }
    let mut seen_uids = std::collections::HashSet::new();
    let live: Vec<u32> = repeatable_sources
        .iter()
        .filter_map(|(uid, _)| seen_uids.insert(*uid).then_some(*uid))
        .collect();
    let mut captured = Vec::new();
    for uid in live {
        let Some(source) = st.battlefield.iter().find(|perm| perm.uid == uid).cloned() else {
            continue;
        };
        for ability in game::permanent_abilities(deck, &source)
            .filter(|ability| ability.kind.is_triggered() && ability.trigger == SimTrigger::Upkeep)
        {
            let key = (uid, ability.id);
            if ability.once_per_turn && st.triggered_this_turn.contains(&key)
                || ability
                    .condition
                    .is_some_and(|condition| !game::condition_met(deck, st, &condition))
            {
                continue;
            }
            let targets = game_effects::prepare_effect_targets(deck, st, ability.effect_sequence());
            if ability.once_per_turn {
                st.triggered_this_turn.insert(key);
            }
            captured.push((source.clone(), ability.clone(), targets));
        }
    }
    for (source, ability, targets) in captured {
        if st.completion.is_some() {
            return;
        }
        let Some(mut targets) = targets else { continue };
        if !targets.any_legal_target(st)
            || ability
                .condition
                .is_some_and(|condition| !game::condition_met(deck, st, &condition))
        {
            continue;
        }
        resolution::begin_resolution(st);
        for effect in ability.effect_sequence() {
            if st.completion.is_some() {
                break;
            }
            resolve_upkeep_effect(deck, st, &source, effect, &mut targets, turn as u32);
        }
        resolution::end_resolution(deck, st, None);
    }
}

/// Apply captured source effects without substituting another permanent with the same card.
fn resolve_upkeep_effect(
    deck: &SimDeck,
    st: &mut GameState,
    source: &Permanent,
    effect: &SimEffect,
    targets: &mut game_effects::PreparedSpellTargets,
    turn: u32,
) {
    match effect {
        SimEffect::Draw(count) => run_scaling_draw(deck, st, source.card.deck_idx(), *count, turn),
        SimEffect::Counters(count) => {
            if let Some(perm) = st
                .battlefield
                .iter_mut()
                .find(|perm| perm.uid == source.uid)
            {
                perm.counters.charge += count;
            }
        }
        SimEffect::BurdenLifeLoss => {
            let counters = st
                .battlefield
                .iter()
                .find(|perm| perm.uid == source.uid)
                .map_or(source.counters, |perm| perm.counters);
            st.life -= counters.burden as i32;
            st.life_paid += counters.burden;
        }
        SimEffect::AddBurdenCounter => {
            if let Some(perm) = st
                .battlefield
                .iter_mut()
                .find(|perm| perm.uid == source.uid)
            {
                perm.counters.burden += 1;
                let draws = perm.counters.burden;
                for _ in 0..draws {
                    game_effects::draw_one(deck, st, turn);
                    if st.failed_draw {
                        break;
                    }
                }
            }
        }
        _ => game_effects::apply_prepared_spell_effect_at(
            deck,
            effect,
            st,
            turn,
            card_of(deck, source).mills_opponent,
            source.card.deck_idx(),
            targets,
        ),
    }
}

/// Draw the supported permanent count without replacing a zero count or truncating at eight.
pub(in crate::deck::simulator) fn run_scaling_draw(
    deck: &SimDeck,
    st: &mut GameState,
    host_idx: Option<CardIdx>,
    base: u32,
    turn: u32,
) {
    let scaled = host_idx
        .and_then(|idx| deck.cards.get(idx.index()))
        .and_then(|card| card.draws_per_matching)
        .map(|kind| {
            let matches = |card: &model::SimCard| match kind {
                model::DrawMatch::Enchantments => card.is_enchantment,
                model::DrawMatch::Artifacts => card.is_artifact,
                model::DrawMatch::Lands => card.is_land,
                model::DrawMatch::Creatures => card.is_creature,
            };
            st.battlefield
                .iter()
                .filter(|perm| perm.card.deck_idx().is_some_and(|idx| matches(&deck[idx])))
                .count() as u32
        })
        .unwrap_or(base);
    resolution::begin_resolution(st);
    for _ in 0..scaled {
        game_effects::draw_one(deck, st, turn);
        if st.failed_draw || st.completion.is_some() {
            break;
        }
    }
    resolution::end_resolution(deck, st, None);
}
