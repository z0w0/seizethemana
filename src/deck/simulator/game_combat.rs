//! Potential unblocked combat output, separate from the opponent life ledger.

use super::game::{CardRef, GameState, Permanent, card_of};
use super::model::{Keyword, SimAbility, SimDeck, SimEffect, SimTrigger};

/// Triggered combat ability captured before its event resolves.
struct CombatTrigger {
    source_uid: u32,
    source_card: Option<super::model::CardIdx>,
    mills_opponent: bool,
    ability: SimAbility,
}

/// One attacker's contribution in one combat damage step.
struct CombatHit {
    uid: u32,
    card: CardRef,
    keywords: super::model::KeywordSet,
    power: u32,
}

/// Combat observations for one scheduled player turn.
pub(super) struct CombatOutcome {
    /// Potential unblocked combat damage, without noncombat effects.
    pub(super) power: u32,
    /// Attacking bodies, or ready bodies when no seats remain active.
    pub(super) attackers: u32,
    /// Contributing bodies with flying, menace, or trample.
    pub(super) evasive: u32,
    /// Each commander slot's own potential combat contribution.
    pub(super) commander_power: Vec<u32>,
}

/// Measure ready bodies without applying combat potential to opponent life.
/// With no active seats, measure capacity only and do not perform attack actions.
pub(super) fn combat_phase(
    deck: &SimDeck,
    st: &mut GameState,
    turn: usize,
    _land_drops: &[u8],
) -> CombatOutcome {
    let active_defender = st.opponents.seats.iter().any(|seat| seat.active);
    let mut attackers: Vec<u32> = st
        .battlefield
        .iter()
        .filter(|perm| can_attack(deck, st, perm, turn))
        .map(|perm| perm.uid)
        .collect();
    if active_defender {
        for uid in &attackers {
            let position = st
                .battlefield
                .iter()
                .position(|perm| perm.uid == *uid)
                .expect("declared attackers remain on the unchanged battlefield");
            let keywords = super::game::effective_keywords(deck, st, &st.battlefield[position]);
            if !keywords.contains(Keyword::Vigilance) {
                st.battlefield[position].tapped = true;
            }
        }
        st.attacked_this_turn = !attackers.is_empty();
        let listeners = combat_triggers(deck, st);
        for uid in attackers.clone() {
            fire_combat_triggers(deck, st, &listeners, SimTrigger::Attacks, uid, turn);
            if st.completion.is_some() {
                break;
            }
            if st.ring_bearer == Some(uid) && st.ring_tempts >= 2 {
                super::game_effects::apply_effect_at(
                    deck,
                    &SimEffect::DrawThenDiscard(1),
                    st,
                    turn as u32,
                    false,
                    None,
                );
            }
        }
        if !attackers.is_empty() && st.completion.is_none() {
            fire_combat_triggers(deck, st, &listeners, SimTrigger::PlayerAttacks, 0, turn);
        }
        for perm in &st.battlefield {
            if perm.attacking_this_turn && !attackers.contains(&perm.uid) {
                attackers.push(perm.uid);
            }
        }
    }
    let mut outcome = CombatOutcome {
        power: 0,
        attackers: 0,
        evasive: 0,
        commander_power: vec![0; deck.commanders.len()],
    };
    if st.completion.is_some() {
        return outcome;
    }
    let combatants: Vec<_> = attackers
        .into_iter()
        .filter_map(|uid| {
            let perm = st.battlefield.iter().find(|perm| perm.uid == uid)?;
            if !super::game::is_creature_permanent(deck, perm) {
                return None;
            }
            let keywords = super::game::effective_keywords(deck, st, perm);
            outcome.attackers += 1;
            outcome.evasive += u32::from(
                keywords.contains(Keyword::Trample)
                    || keywords.contains(Keyword::Flying)
                    || keywords.contains(Keyword::Menace),
            );
            Some((
                uid,
                keywords.contains(Keyword::FirstStrike) || keywords.contains(Keyword::DoubleStrike),
            ))
        })
        .collect();
    for first_strike in [true, false] {
        let hits = damage_step(deck, st, &combatants, first_strike);
        for hit in &hits {
            outcome.power = outcome.power.saturating_add(hit.power);
            if let CardRef::Commander { slot } = hit.card {
                outcome.commander_power[slot] =
                    outcome.commander_power[slot].saturating_add(hit.power);
            }
        }
        if active_defender {
            resolve_damage_step_effects(deck, st, &hits, turn);
        }
        if st.completion.is_some() {
            break;
        }
    }
    outcome
}

/// Measure simultaneous damage before settling rewards, and recompute power for the next step.
fn damage_step(
    deck: &SimDeck,
    st: &GameState,
    combatants: &[(u32, bool)],
    first_strike: bool,
) -> Vec<CombatHit> {
    combatants
        .iter()
        .filter_map(|(uid, struck_first)| {
            let perm = st.battlefield.iter().find(|perm| perm.uid == *uid)?;
            if !super::game::is_creature_permanent(deck, perm) {
                return None;
            }
            let keywords = super::game::effective_keywords(deck, st, perm);
            if first_strike && !*struck_first
                || !first_strike && *struck_first && !keywords.contains(Keyword::DoubleStrike)
            {
                return None;
            }
            Some(CombatHit {
                uid: *uid,
                card: perm.card,
                keywords,
                power: combat_power(deck, st, perm),
            })
        })
        .collect()
}

/// Resolve lifelink and captured damage events only when the damage step had a live defender.
fn resolve_damage_step_effects(
    deck: &SimDeck,
    st: &mut GameState,
    hits: &[CombatHit],
    turn: usize,
) {
    if !st.opponents.seats.iter().any(|seat| seat.active) {
        return;
    }
    for hit in hits {
        if hit.keywords.contains(Keyword::Lifelink) {
            st.life += hit.power as i32;
            st.life_gained += hit.power;
        }
    }
    let listeners = combat_triggers(deck, st);
    for hit in hits.iter().filter(|hit| hit.power > 0) {
        if st.ring_bearer == Some(hit.uid) && st.ring_tempts >= 4 {
            st.opponent_life_lost += st
                .opponents
                .lose_life(3, super::model::LifeLossScope::EachOpponent);
        }
        fire_combat_triggers(
            deck,
            st,
            &listeners,
            SimTrigger::CombatDamage,
            hit.uid,
            turn,
        );
        if st.completion.is_some() {
            return;
        }
    }
}

/// Apply supported creature readiness rules before declaring attackers.
fn can_attack(deck: &SimDeck, st: &GameState, perm: &Permanent, turn: usize) -> bool {
    if !super::game::is_creature_permanent(deck, perm) {
        return false;
    }
    perm.attacking_this_turn
        || !perm.tapped
            && (!perm.summoning_sick && perm.entered_turn < turn
                || super::game::effective_keywords(deck, st, perm).contains(Keyword::Haste))
}

/// Combine current counters and supported combat bonuses before clamping power.
fn combat_power(deck: &SimDeck, st: &GameState, perm: &Permanent) -> u32 {
    let power = super::game_effects::current_creature_power_signed(perm, deck, &st.battlefield);
    power.clamp(0, i64::from(u32::MAX)) as u32
}

/// Resolve one event's supported triggers with complete effect boundaries.
fn fire_combat_triggers(
    deck: &SimDeck,
    st: &mut GameState,
    listeners: &[CombatTrigger],
    trigger: SimTrigger,
    event_uid: u32,
    turn: usize,
) {
    for listener in listeners {
        if st.completion.is_some() {
            return;
        }
        let ability = &listener.ability;
        if ability.trigger != trigger
            || !super::game::trigger_subject_matches(
                ability.event_subject,
                listener.source_uid,
                event_uid,
            )
        {
            continue;
        }
        let matches = st
            .battlefield
            .iter()
            .find(|perm| perm.uid == event_uid)
            .map_or_else(
                || {
                    trigger == SimTrigger::PlayerAttacks
                        && ability.event_filter.card_types.is_empty()
                        && ability.event_filter.subtypes.is_empty()
                        && !ability.event_filter.noncreature
                        && !ability.event_filter.nontoken
                },
                |perm| {
                    super::game::event_filter_matches(
                        &ability.event_filter,
                        if perm.face_down {
                            super::game::token_creature_card()
                        } else {
                            card_of(deck, perm)
                        },
                        matches!(perm.card, CardRef::Token(_)),
                    )
                },
            );
        let key = (listener.source_uid, ability.id);
        let limited = ability.once_per_turn || ability.event_filter.first_each_turn;
        if !matches
            || limited && st.triggered_this_turn.contains(&key)
            || ability.resolution_payment.is_some()
            || ability
                .condition
                .is_some_and(|condition| !super::game::condition_met(deck, st, &condition))
        {
            continue;
        }
        if limited {
            st.triggered_this_turn.insert(key);
        }
        super::resolution::begin_resolution(st);
        for effect in ability.effect_sequence() {
            if st.completion.is_some() {
                break;
            }
            match effect {
                SimEffect::Counters(amount) => {
                    if let Some(source) = st
                        .battlefield
                        .iter_mut()
                        .find(|source| source.uid == listener.source_uid)
                    {
                        source.counters.charge += amount;
                    }
                }
                _ => super::game_effects::apply_effect_at(
                    deck,
                    effect,
                    st,
                    turn as u32,
                    listener.mills_opponent,
                    listener.source_card,
                ),
            }
        }
        super::resolution::end_resolution(deck, st, None);
        st.opponents.settle_losses();
    }
}

/// Capture combat listeners by physical source identity.
fn combat_triggers(deck: &SimDeck, st: &GameState) -> Vec<CombatTrigger> {
    st.battlefield
        .iter()
        .flat_map(|source| {
            super::game::permanent_abilities(deck, source)
                .filter(|ability| {
                    ability.kind.is_triggered()
                        && matches!(
                            ability.trigger,
                            SimTrigger::Attacks
                                | SimTrigger::PlayerAttacks
                                | SimTrigger::CombatDamage
                        )
                })
                .map(|ability| CombatTrigger {
                    source_uid: source.uid,
                    source_card: source.card.deck_idx(),
                    mills_opponent: card_of(deck, source).mills_opponent,
                    ability: ability.clone(),
                })
        })
        .collect()
}

/// Combat identity, timing, and capacity-only regression scenarios.
#[cfg(test)]
#[path = "tests/combat_contract_tests.rs"]
mod combat_contract_tests;
