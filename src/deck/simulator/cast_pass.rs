//! The cast pass and land-drop helpers for the goldfish game loop, split
//! from game_run.rs to keep files small. Pure apart from the game state
//! mutations they drive.

use super::game::{GameState, ManaPool, card_of, new_perm_with, take_uid};
use super::game_effects::{
    CardZone, move_to_graveyard, prepare_spell_targets, spell_targets_available,
};
use super::game_mana::{adjust_cast_cost, pay_cost, payable, phyrexian_life_charge, pips_ok};
use super::model::{
    CardIdx, Role, SearchCardType, SearchDestination, SearchSpec, SimDeck, SimEffect,
};

/// Mandatory and optional costs chosen before spell resolution.
mod cast_costs;
/// One affordable spell's resolution (cost payment, spell data, storm).
#[path = "cast_pass/cast_resolve.rs"]
pub(in crate::deck::simulator) mod cast_resolve;
/// Spell selection and resolution: the sweep that spends the pool on casts.
mod cast_sweep;
/// Cast and spell-effect battlefield entries.
mod entry;
/// Face-down casting and turning face up (morph, megamorph, disguise).
mod face_down;
/// Shared resource selection for spell mana, convoke, and delve.
mod payment;
/// Read-only cast resources shared with mana-source planning.
pub(in crate::deck::simulator) use payment::cast_payment_resources;
/// Trigger and entry settlement for one cast.
mod settlement;
/// Ordered spell effects, including copied spell resolutions.
mod spell_effects;
/// Legal-work probes and single graveyard replay attempts.
mod work;

pub(in crate::deck::simulator) use settlement::fire_cast_triggers;

pub(in crate::deck::simulator) use cast_resolve::{
    CastPayment, resolve_cast, upkeep_trigger_registration,
};
pub(super) use entry::{entry_is_tapped, entry_tapped, fire_entry_triggers, new_entry_permanent};
pub(super) use face_down::{resolve_face_down, turn_face_up};

/// Play one land from the hand, preferring untapped lands, and fire its ETBs.
/// Activated abilities wait for the normal activation pass.
pub(super) fn play_land(deck: &SimDeck, st: &mut GameState, turn: u32) -> bool {
    // Land/spell MDFCs play their land face only when the hand holds no
    // other land to play this turn; otherwise they stay as spells.
    let mdfc_fallback = |st: &GameState| {
        st.hand
            .iter()
            .position(|idx| deck[*idx].is_mdfc_spell)
            .filter(|_| !st.hand.iter().any(|i| deck[*i].role == Role::Land))
    };
    let land_pos = st
        .hand
        .iter()
        .position(|idx| deck[*idx].role == Role::Land && !entry_is_tapped(deck, st, *idx, true))
        .or_else(|| st.hand.iter().position(|idx| deck[*idx].role == Role::Land))
        .or_else(|| mdfc_fallback(st));
    let Some(pos) = land_pos else {
        return false;
    };
    super::resolution::begin_resolution(st);
    let idx = st.hand.remove(pos);
    let tapped_in = entry_tapped(deck, st, idx, false, true);
    st.battlefield_seen.entry(idx).or_insert(turn);
    let uid = take_uid(st);
    let mut permanent = new_perm_with(uid, deck, idx, turn, tapped_in);
    permanent.land_face = true;
    if deck[idx].is_mdfc_spell {
        permanent.summoning_sick = false;
        permanent.counters = Default::default();
    }
    st.battlefield.push(permanent);
    let played_pos = st.battlefield.len() - 1;
    fire_entry_triggers(deck, st, played_pos, turn);
    super::resolution::end_resolution(deck, st, None);
    true
}

/// The cast pass: cheapest castable spells first, pip-aware. Updates the
/// pool, ETB triggers, and the mana-ready curve. Returns nothing; the
/// caller owns every mutated binding.
// The 11 parameters are the game state the cast phase needs in full;
// a parameter struct would just be read back out field by field.
#[allow(clippy::too_many_arguments)]
pub(super) fn cast_pass(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    mana_spent: &mut [f64],
    repeatable_sources: &mut Vec<(u32, u32)>,
    pip_blocks: &mut Vec<(CardIdx, usize)>,
    blocked_colors: &mut [bool; 5],
) {
    let preparing_graveyard_exchange = st.hand.iter().any(|index| deck[*index].has_cascade)
        && st
            .library
            .iter()
            .any(|index| deck[*index].spell_data.graveyard_creature_exchange)
        && st.hand.iter().any(|index| {
            deck[*index].is_creature
                && (deck[*index].spell_data.cycling_cost.is_some()
                    || deck[*index].spell_data.cycling_life > 0)
        });
    if preparing_graveyard_exchange {
        cycle_unusable_cards(deck, st, pool, turn as u32);
    }
    mana_spent[turn - 1] += f64::from(work::cast_hand_pass(
        deck,
        st,
        pool,
        turn,
        repeatable_sources,
        pip_blocks,
        blocked_colors,
    ));
    if st.completion.is_some() {
        return;
    }
    // Face-down bodies turn up while the pool covers their morph or
    // disguise cost (CR 702.37e). The cast pass leaves spare mana in the
    // pool, so the flip resolves here.
    turn_face_up(deck, st, pool);
    if st.completion.is_some() {
        return;
    }
    cast_graveyard_spells(deck, st, pool, turn, mana_spent, repeatable_sources);
    if st.completion.is_some() {
        return;
    }
    if cycle_unusable_cards(deck, st, pool, turn as u32) {
        if st.completion.is_some() {
            return;
        }
        cast_pass(
            deck,
            st,
            pool,
            turn,
            mana_spent,
            repeatable_sources,
            pip_blocks,
            blocked_colors,
        );
    }
}

/// Attempt one physical hand card through the pilot's payment and settlement path.
/// Rejection may record missing-color telemetry, but cannot pay costs or select a shuffle.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(in crate::deck::simulator) fn try_cast_from_hand(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    index: CardIdx,
    repeatable_sources: &mut Vec<(u32, u32)>,
    pip_blocks: &mut Vec<(CardIdx, usize)>,
    blocked_colors: &mut [bool; 5],
) -> Option<u32> {
    if st.completion.is_some() || deck[index].role == Role::Land {
        return None;
    }
    let mut spent = 0;
    cast_sweep::cast_pass(
        deck,
        st,
        pool,
        turn,
        &mut vec![index],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut spent,
        repeatable_sources,
        pip_blocks,
        blocked_colors,
    )
    .then_some(spent)
}

/// Keep reactive answers in hand during an empty-board goldfish. Direct
/// player damage and life loss remain castable as win conditions.
fn goldfish_castable(card: &super::model::SimCard) -> bool {
    card.role != Role::Land
        && (!(card.flags.is_interaction && card.role == Role::Removal)
            || card.spell_data.damage_on_resolve > 0
            || card.spell_data.life_loss_on_resolve > 0
            || card.mills_opponent)
}

/// Check the nonmana resources required by a spell's additional costs.
pub(in crate::deck::simulator) fn additional_costs_payable(
    deck: &SimDeck,
    st: &GameState,
    index: CardIdx,
) -> bool {
    let costs = &deck[index].spell_data;
    let creatures = st
        .battlefield
        .iter()
        .filter(|permanent| super::game::is_creature_permanent(deck, permanent))
        .count();
    let discards = st.hand.iter().filter(|held| **held != index).count();
    !costs.unsupported_additional_cost
        && spell_targets_available(deck, st, &deck[index])
        && creatures >= costs.additional_cost_creatures as usize
        && discards >= costs.additional_cost_discards as usize
        && st.life >= costs.additional_cost_life as i32
}

/// Cast legal graveyard instances through active flashback or escape permissions.
pub(super) fn cast_graveyard_spells(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    mana_spent: &mut [f64],
    repeatable_sources: &mut Vec<(u32, u32)>,
) {
    for _ in 0..super::model::MAX_LOOP_PASSES {
        if st.completion.is_some() {
            return;
        }
        if !work::cast_one_from_graveyard(deck, st, pool, turn, mana_spent, repeatable_sources) {
            return;
        }
    }
    if st.completion.is_none() && work::has_graveyard_cast(deck, st, pool) {
        super::resolution::mark_incomplete(
            st,
            super::game::IncompleteReason::GraveyardCastPass {
                configured_limit: super::model::MAX_LOOP_PASSES,
                remaining_work: 1,
            },
        );
    }
}

/// Cycle uncast cards when their cycling cost is payable, then retry casts.
fn cycle_unusable_cards(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let mut cycled = false;
    let candidates = st.hand.clone();
    for index in candidates {
        if st.completion.is_some() {
            break;
        }
        let card = &deck[index];
        if card.spell_data.cycling_cost.is_none() && card.spell_data.cycling_life == 0 {
            continue;
        }
        let cost = card
            .spell_data
            .cycling_cost
            .as_ref()
            .cloned()
            .unwrap_or_default();
        // Phyrexian pips in the cycling cost pay with 2 life each
        // (CR 107.4f), on top of any "pay N life" rider.
        let charge = phyrexian_life_charge(&cost);
        if !st.hand.contains(&index)
            || st.life < (card.spell_data.cycling_life + charge) as i32
            || !payable(&cost, pool)
            || !pips_ok(&cost, pool)
        {
            continue;
        }
        super::resolution::begin_resolution(st);
        pay_cost(&cost, pool);
        st.life -= (card.spell_data.cycling_life + charge) as i32;
        st.life_paid += card.spell_data.cycling_life + charge;
        st.hand.retain(|held| *held != index);
        move_to_graveyard(deck, st, index, turn, CardZone::Hand);
        super::resolution::end_resolution(deck, st, Some(pool));
        if st.completion.is_some() {
            return true;
        }
        super::resolution::begin_resolution(st);
        if let Some(land) = card.spell_data.landcycling_type {
            let mut land_types = [false; 5];
            land_types[land.index()] = true;
            super::game_effects::apply_effect_at(
                deck,
                &SimEffect::Search(SearchSpec {
                    card_type: Some(SearchCardType::Land),
                    land_types,
                    destination: SearchDestination::Hand,
                    shuffle: true,
                    ..SearchSpec::default()
                }),
                st,
                turn,
                false,
                Some(index),
            );
        } else if super::game_effects::draw_one_outcome(deck, st, turn)
            == super::game_effects::DrawOutcome::Drawn
            && card.spell_data.cycling_life + charge > 0
        {
            st.life_funded_draws += 1;
            super::game::milestone_for_turn(st, turn).life_funded_draws += 1;
        }
        super::resolution::end_resolution(deck, st, None);
        cycled = true;
        if st.completion.is_some() {
            break;
        }
    }
    cycled
}

/// Reveal cards for the supported life-payment spell until life is
/// exhausted. CR 119.4: a player can pay life only up to their total, so a
/// reveal whose mana value exceeds remaining life stops the loop before it
/// would drive life below zero.
pub(in crate::deck::simulator) fn resolve_reveal_rule(
    deck: &SimDeck,
    st: &mut GameState,
    rule: super::model::RevealRule,
    turn: u32,
) {
    let largest_known_cost = deck
        .library_cards()
        .map(|card| card.mana_value)
        .max()
        .unwrap_or(0);
    let mut first_reveal = true;
    while first_reveal || i64::from(st.life) > i64::from(largest_known_cost) {
        let Some(index) = st.library.pop() else { break };
        first_reveal = false;
        match rule.destination {
            super::model::RevealDestination::Hand => {
                super::game::record_card_seen(st, index, turn);
                st.hand.push(index);
            }
        }
        st.seen += 1;
        st.awareness_cards += 1;
        match rule.life_loss {
            super::model::RevealLifeLoss::ManaValue => {
                st.life -= deck[index].mana_value as i32;
                st.life_funded_draws += 1;
                super::game::milestone_for_turn(st, turn).life_funded_draws += 1;
            }
        }
    }
}

/// Select cards from hand that satisfy a parsed alternate casting cost.
/// A fixed-count cost needs exactly that many matching cards.
pub(in crate::deck::simulator) fn select_alternative_cost_cards(
    deck: &SimDeck,
    st: &GameState,
    spell: CardIdx,
) -> Vec<CardIdx> {
    let Some(cost) = deck[spell].spell_data.alternative_cast_cost else {
        return Vec::new();
    };
    let mut candidates: Vec<CardIdx> = st
        .hand
        .iter()
        .copied()
        .filter(|index| *index != spell)
        .filter(|index| {
            cost.filter
                .color
                .is_none_or(|color| deck[*index].colors[color.index()])
        })
        .collect();
    candidates.sort_by_key(|index| std::cmp::Reverse(deck[*index].mana_value));
    if candidates.len() < cost.count as usize {
        return Vec::new();
    }
    candidates.truncate(cost.count as usize);
    candidates
}

#[cfg(test)]
#[path = "tests/cast_settlement_tests.rs"]
mod cast_settlement_tests;

/// Exact cast-resource and alternate-cost regressions.
#[cfg(test)]
#[path = "tests/cast_payment_regression_tests.rs"]
mod cast_payment_regression_tests;

/// Fetch lands use ordinary activation costs and committed search effects.
#[cfg(test)]
#[path = "tests/fetch_activation_tests.rs"]
mod fetch_activation_tests;

/// Cast budgets, retained cost restrictions, and shared power-payment regressions.
#[cfg(test)]
#[path = "tests/cast_limit_tests.rs"]
mod cast_limit_tests;
