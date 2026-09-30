//! Mana-pool building for the goldfish turn loop: taps, gates, banked
//! mana, counter-scaled taps, grants, and the Treasure bank. Split from
//! `game_run` to keep files small.

use super::super::game::{GameState, ManaPool, Permanent, card_of};
use super::super::game_mana::{
    add_yield, add_yield_turns, effective_min_cost, pay_cost, payable, pips_ok,
};
use super::super::model::{Role, Scale, SimAbilityKind, SimDeck, SimEffect, SimTrigger};
use super::add_nonland_mana;

/// Build the turn's spendable pool from untapped permanents: taps,
/// verge gates, banked-mana releases, counter-scaled taps, static
/// grants, and the Treasure bank.
pub(in crate::deck::simulator) fn build_pool(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
) -> ManaPool {
    let mut pool = ManaPool::default();
    release_banked_mana(deck, st, &mut pool, turn);
    let uids: Vec<u32> = st
        .battlefield
        .iter()
        .map(|permanent| permanent.uid)
        .collect();
    for uid in uids {
        let Some(pos) = st
            .battlefield
            .iter()
            .position(|permanent| permanent.uid == uid)
        else {
            continue;
        };
        let perm = st.battlefield[pos].clone();
        if perm.tapped {
            continue;
        }
        let card = card_of(deck, &perm);
        if !mana_condition_met(deck, st, card) {
            continue;
        }
        // Land and noncreature mana sources are reserved when their mana
        // joins the pool. This prevents the same permanent from stationing
        // or crewing after it paid for a cast.
        if card.is_creature && perm.card.deck_idx().is_some() {
            continue;
        }
        let has_mana_mode = card
            .unlocked_abilities(perm.counters.charge)
            .any(|ability| ability.kind == SimAbilityKind::ManaActivated);
        if has_mana_mode {
            activate_mana_mode(deck, st, &perm, &mut pool, turn);
            continue;
        }
        let Some(y) = &card.tap else {
            continue;
        };
        if let Some(Scale::PerChargeCounter) = y.scaling {
            // Counter-scaled taps resolve at the tap: one any-color pip
            // per charge counter on the source (Astral Cornucopia
            // class).
            let yield_ = super::super::model::ManaYield {
                any_pips: perm.counters.charge,
                ..super::super::model::ManaYield::default()
            };
            add_nonland_mana(deck, st, &perm, &yield_, &mut pool, turn);
            st.battlefield[pos].tapped = true;
            if !card.is_land {
                fire_tapped_for_mana_triggers(deck, st, &mut pool, turn);
            }
            continue;
        }
        if card.gate_types.is_empty() {
            if !add_nonland_mana(deck, st, &perm, y, &mut pool, turn) {
                continue;
            }
            st.battlefield[pos].tapped = true;
            if !card.is_land {
                fire_tapped_for_mana_triggers(deck, st, &mut pool, turn);
            }
            continue;
        }
        add_gated_yield(deck, st, &perm, y, &mut pool, turn);
        st.battlefield[pos].tapped = true;
    }
    // Treasure bank: spend up to the full bank as flexible pips (the
    // player would sacrifice them when needed; best-case the whole bank
    // converts this turn). Treasures are consumed on use, so the bank
    // empties here — new Treasure this turn restocks it for next turn.
    if st.treasure_bank > 0 {
        pool.flexible += st.treasure_bank;
        st.treasure_bank = 0;
    }
    pool
}

/// Use one complete mana mode from the permanent, preserving that mode's
/// own costs and output instead of merging costs across the card.
pub(in crate::deck::simulator) fn activate_mana_mode(
    deck: &SimDeck,
    st: &mut GameState,
    source: &Permanent,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let card = card_of(deck, source);
    let candidate = card
        .unlocked_abilities(source.counters.charge)
        .filter(|ability| ability.kind == SimAbilityKind::ManaActivated)
        .filter_map(|ability| {
            let activation = ability.activation.as_ref()?;
            let ability_key = (source.uid, ability.id);
            if activation
                .has_restriction(super::super::model::SimActivationRestriction::OncePerTurn)
                && st.activated_this_turn.contains(&ability_key)
                || activation
                    .has_restriction(super::super::model::SimActivationRestriction::OncePerGame)
                    && st.activated_once.contains(&ability_key)
            {
                return None;
            }
            let yield_ = ability
                .effect_sequence()
                .iter()
                .find_map(|effect| match effect {
                    SimEffect::Mana(yield_) | SimEffect::ManaPerCounter(yield_) => {
                        Some(yield_.clone())
                    }
                    _ => None,
                })?;
            if yield_.fixed.iter().all(|amount| *amount == 0)
                && yield_.choice.iter().all(|available| !available)
                && yield_.any_pips == 0
                && yield_.colorless == 0
                && !(yield_.scaling == Some(Scale::PerChargeCounter) && source.counters.charge > 0)
            {
                return None;
            }
            let mana_cost = activation.mana_cost();
            if !payable(&mana_cost, pool) || !pips_ok(&mana_cost, pool) {
                return None;
            }
            if activation.taps_source()
                && (source.tapped
                    || (super::super::game::is_creature_permanent(deck, source)
                        && (source.summoning_sick || source.entered_turn >= turn as usize)
                        && !super::super::game::effective_keywords(deck, st, source)
                            .contains(super::super::model::Keyword::Haste)))
            {
                return None;
            }
            if activation.untaps_source() && !source.tapped {
                return None;
            }
            if activation.sacrifices_source() && source.card.deck_idx().is_none() {
                return None;
            }
            if activation.creature_sacrifices() > 0
                && !super::super::game_effects::has_sacrificable_creature(deck, st)
            {
                return None;
            }
            if activation.life_payment() > 0 && st.life < activation.life_payment() as i32
                || activation.energy_payment() > st.player_counters.energy
                || activation.charge_counter_payment() > source.counters.charge
            {
                return None;
            }
            if !mana_mode_has_target(deck, st, yield_.restriction) {
                return None;
            }
            let output = yield_
                .fixed
                .iter()
                .map(|amount| u32::from(*amount))
                .sum::<u32>()
                + yield_.colorless
                + yield_.any_pips
                + u32::from(yield_.choice.iter().any(|choice| *choice))
                + if yield_.scaling == Some(Scale::PerChargeCounter) {
                    source.counters.charge
                } else {
                    0
                };
            let score = output.saturating_sub(mana_cost.total());
            Some((score, ability.clone(), activation.clone(), yield_))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .max_by_key(|candidate| candidate.0);
    let Some((_, ability, activation, yield_)) = candidate else {
        return false;
    };
    let mut trial_pool = pool.clone();
    if !add_mode_yield(deck, st, source, &yield_, &mut trial_pool, turn) {
        return false;
    }

    pay_cost(&activation.mana_cost(), pool);
    st.life -= activation.life_payment() as i32;
    st.life_paid += activation.life_payment();
    st.player_counters.energy -= activation.energy_payment();
    if activation.charge_counter_payment() > 0
        && let Some(permanent) = st
            .battlefield
            .iter_mut()
            .find(|item| item.uid == source.uid)
    {
        permanent.counters.charge -= activation.charge_counter_payment();
    }
    if activation.taps_source()
        && let Some(permanent) = st
            .battlefield
            .iter_mut()
            .find(|item| item.uid == source.uid)
    {
        permanent.tapped = true;
    }
    if activation.untaps_source()
        && let Some(permanent) = st
            .battlefield
            .iter_mut()
            .find(|item| item.uid == source.uid)
    {
        permanent.tapped = false;
    }
    if activation.sacrifices_source() {
        super::super::game_effects::resolve_sacrifice_uid(deck, st, turn, source.uid);
    }
    if activation.creature_sacrifices() > 0 {
        super::super::game_effects::resolve_creature_sacrifice(deck, st, turn, source.uid);
    }
    for effect in ability.effect_sequence() {
        match effect {
            SimEffect::Mana(yield_) => {
                add_mode_yield(deck, st, source, yield_, pool, turn);
            }
            SimEffect::ManaPerCounter(yield_) => {
                for _ in 0..activation.charge_counter_payment() {
                    add_mode_yield(deck, st, source, yield_, pool, turn);
                }
            }
            other => super::super::game_effects::apply_effect_at(
                deck,
                other,
                st,
                turn,
                false,
                source.card.deck_idx(),
            ),
        }
    }
    if activation.taps_source() && !card.is_land {
        fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
    let ability_key = (source.uid, ability.id);
    if activation.has_restriction(super::super::model::SimActivationRestriction::OncePerTurn) {
        st.activated_this_turn.insert(ability_key);
    }
    if activation.has_restriction(super::super::model::SimActivationRestriction::OncePerGame) {
        st.activated_once.insert(ability_key);
    }
    true
}

/// True when at least one held spell can use the mode's restricted mana.
fn mana_mode_has_target(
    deck: &SimDeck,
    st: &GameState,
    restriction: Option<super::super::model::SpendRestriction>,
) -> bool {
    let Some(restriction) = restriction else {
        return true;
    };
    st.hand.iter().any(|index| {
        let card = &deck[*index];
        match restriction {
            super::super::model::SpendRestriction::Creature => card.is_creature,
            super::super::model::SpendRestriction::Legendary => card.is_legendary,
            super::super::model::SpendRestriction::Artifact => card.is_artifact,
            super::super::model::SpendRestriction::InstantSorcery => card.is_instant_or_sorcery,
        }
    })
}

/// Resolve one mana mode through its land gate or ordinary mana source.
fn add_mode_yield(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    yield_: &super::super::model::ManaYield,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let before = pool.total();
    if card_of(deck, source).gate_types.is_empty() {
        super::super::game_run::add_nonland_mana(deck, st, source, yield_, pool, turn);
    } else {
        add_gated_yield(deck, st, source, yield_, pool, turn);
    }
    pool.total() > before
}

/// Resolve triggered mana abilities caused by a nonland mana tap.
pub(in crate::deck::simulator) fn fire_tapped_for_mana_triggers(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let triggers: Vec<_> = st
        .battlefield
        .iter()
        .flat_map(|source| {
            super::super::game::permanent_abilities(deck, source)
                .filter(|ability| {
                    ability.kind == SimAbilityKind::ManaTriggered
                        && ability.trigger == SimTrigger::TappedForMana
                        && ability.condition.is_none_or(|condition| {
                            super::super::game::condition_met(deck, st, &condition)
                        })
                })
                .map(|ability| {
                    (
                        source.uid,
                        source.card.deck_idx(),
                        card_of(deck, source).mills_opponent,
                        ability.clone(),
                    )
                })
        })
        .collect();
    for (uid, source_card, mills_opponent, ability) in triggers {
        let key = (uid, ability.id);
        if ability.once_per_turn && st.triggered_this_turn.contains(&key)
            || ability
                .condition
                .is_some_and(|condition| !super::super::game::condition_met(deck, st, &condition))
        {
            continue;
        }
        for effect in ability.effect_sequence() {
            match effect {
                // The source-tap path already copies the tapped source's
                // mana type for this parsed trigger.
                SimEffect::Mana(_)
                    if source_card
                        .is_some_and(|index| deck[index].flags.bonus_mana_on_nonland_tap) => {}
                SimEffect::Mana(yield_) => {
                    add_yield_turns(deck, yield_, pool, turn, &st.battlefield);
                }
                SimEffect::ManaPerCounter(yield_) => {
                    let count = st
                        .battlefield
                        .iter()
                        .find(|permanent| permanent.uid == uid)
                        .map_or(0, |permanent| permanent.counters.charge);
                    for _ in 0..count {
                        add_yield_turns(deck, yield_, pool, turn, &st.battlefield);
                    }
                }
                other => super::super::game_effects::apply_effect_at(
                    deck,
                    other,
                    st,
                    turn,
                    mills_opponent,
                    source_card,
                ),
            }
        }
        if ability.once_per_turn {
            st.triggered_this_turn.insert(key);
        }
    }
}

/// Tap legal creature mana sources only when the current pool cannot pay
/// the cheapest spell still in hand. Tapped dorks cannot also station or crew.
pub(in crate::deck::simulator) fn tap_dorks_for_mana(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let need = st
        .hand
        .iter()
        .filter(|idx| deck[**idx].role != Role::Land)
        .map(|idx| effective_min_cost(deck, &deck[*idx], &st.battlefield).total())
        .min();
    let Some(need) = need else { return };
    for pos in 0..st.battlefield.len() {
        if super::super::game_mana::usable_for_noncreature(pool) >= need {
            break;
        }
        let perm = st.battlefield[pos].clone();
        let card = card_of(deck, &perm);
        if perm.tapped
            || (perm.summoning_sick || perm.entered_turn >= turn as usize)
                && super::super::game::is_creature_permanent(deck, &perm)
                && !super::super::game::effective_keywords(deck, st, &perm)
                    .contains(super::super::model::Keyword::Haste)
            || !super::super::game::is_creature_permanent(deck, &perm)
            || !mana_condition_met(deck, st, card)
        {
            continue;
        }
        let Some(yield_) = card.tap.clone() else {
            continue;
        };
        let has_mana_mode = card
            .unlocked_abilities(perm.counters.charge)
            .any(|ability| ability.kind == SimAbilityKind::ManaActivated);
        if has_mana_mode {
            activate_mana_mode(deck, st, &perm, pool, turn);
            continue;
        }
        if !add_nonland_mana(deck, st, &perm, &yield_, pool, turn) {
            continue;
        }
        st.battlefield[pos].tapped = true;
        fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
}

/// Activate a noncreature rock after it enters, then revisit the cast
/// choices. Its mana source is tapped exactly once for the turn.
pub(in crate::deck::simulator) fn tap_new_rocks(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let uids: Vec<u32> = st
        .battlefield
        .iter()
        .filter(|permanent| permanent.entered_turn == turn as usize)
        .map(|permanent| permanent.uid)
        .collect();
    for uid in uids {
        let Some(pos) = st
            .battlefield
            .iter()
            .position(|permanent| permanent.uid == uid)
        else {
            continue;
        };
        let perm = st.battlefield[pos].clone();
        let card = card_of(deck, &perm);
        if perm.tapped
            || perm.entered_turn != turn as usize
            || card.is_creature
            || !mana_condition_met(deck, st, card)
        {
            continue;
        }
        let Some(yield_) = card.tap.clone() else {
            continue;
        };
        let has_mana_mode = card
            .unlocked_abilities(perm.counters.charge)
            .any(|ability| ability.kind == SimAbilityKind::ManaActivated);
        if has_mana_mode {
            activate_mana_mode(deck, st, &perm, pool, turn);
            continue;
        }
        if !add_nonland_mana(deck, st, &perm, &yield_, pool, turn) {
            continue;
        }
        st.battlefield[pos].tapped = true;
        fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
}

/// Check player-controlled conditions that gate a card's mana ability.
fn mana_condition_met(deck: &SimDeck, st: &GameState, card: &super::super::model::SimCard) -> bool {
    !card.flags.requires_metalcraft
        || st
            .battlefield
            .iter()
            .filter(|permanent| card_of(deck, permanent).is_artifact)
            .count()
            >= 3
}

/// Banked-mana engines release in the first main phase (Coalition Relic):
/// counters × N mana joins the pool, counters clear.
fn release_banked_mana(deck: &SimDeck, st: &mut GameState, pool: &mut ManaPool, turn: u32) {
    let releases: Vec<(u32, super::super::model::ManaYield, u32)> = st
        .battlefield
        .iter()
        .filter_map(|perm| {
            let card = card_of(deck, perm);
            let release = card.unlocked_abilities(perm.counters.charge).find(|a| {
                a.trigger == SimTrigger::PrecombatMain
                    && matches!(
                        a.effect_sequence().first(),
                        Some(SimEffect::ManaPerCounter(_))
                    )
            })?;
            let y = match release.effect_sequence().first()? {
                SimEffect::ManaPerCounter(y) => y.clone(),
                _ => return None,
            };
            Some((perm.uid, y, perm.counters.charge))
        })
        .collect();
    for (uid, y, counters) in releases {
        for _ in 0..counters {
            add_yield_turns(deck, &y, pool, turn, &st.battlefield);
        }
        // The contributing permanent clears its own bank; a second
        // copy of the card keeps its counters for its own upkeep.
        if let Some(perm) = st.battlefield.iter_mut().find(|p| p.uid == uid) {
            perm.counters.charge = 0;
        }
    }
}

/// Verge-gate lands: the gated tap mode unlocks when another land of a
/// gated type is in play; locked lands yield only the ungated first
/// mode (the first fixed pip, or the first choice color).
fn add_gated_yield(
    deck: &SimDeck,
    st: &GameState,
    perm: &Permanent,
    y: &super::super::model::ManaYield,
    pool: &mut ManaPool,
    turn: u32,
) {
    let gates_open = card_of(deck, perm).gate_types.iter().any(|want| {
        st.battlefield.iter().any(|p| {
            p.uid != perm.uid
                && card_of(deck, p).role == Role::Land
                && card_of(deck, p).land_types[want.index()]
        })
    });
    if gates_open {
        add_yield_turns(deck, y, pool, turn, &st.battlefield);
        return;
    }
    let ungated_color = y
        .fixed
        .iter()
        .position(|p| *p > 0)
        .or_else(|| y.choice.iter().position(|c| *c));
    let Some(ci) = ungated_color else {
        if y.colorless > 0 {
            pool.colorless += 1;
        }
        return;
    };
    let mut ungated = super::super::model::ManaYield {
        fixed: [0; 5],
        choice: [false; 5],
        any_pips: 0,
        opponent_any: false,
        cannot_pay_generic: false,
        scaling: None,
        colorless: 0,
        alternatives: false,
        restriction: None,
    };
    ungated.fixed[ci] = 1;
    add_yield(&ungated, pool);
}

/// The static mana grants live on the battlefield (Enduring Vitality,
/// Chromatic Lantern): each grant converts every matching permanent's
/// tap to one any-color pip.
pub(in crate::deck::simulator) fn grants_active(
    deck: &SimDeck,
    st: &GameState,
) -> super::super::model::Grants {
    let mut grants = super::super::model::Grants::default();
    for perm in &st.battlefield {
        match card_of(deck, perm).flags.grant {
            Some(super::super::model::Grant::Creatures) => grants.creatures = true,
            Some(super::super::model::Grant::Lands) => grants.lands = true,
            None => {}
        }
    }
    grants
}
