//! Cast resolution for one affordable spell: cost payment (convoke,
//! delve, phyrexian), spell data, ETB counters, X conversion, storm copies,
//! and offspring.

use super::super::game::{GameState, ManaPool, card_of};
use super::super::game_effects::{
    CardZone, PreparedSpellTargets, move_to_graveyard, prepare_spell_targets,
};
use super::super::game_mana::{add_yield_turns_unscaled, cast_cost, phyrexian_life_charge};
use super::super::model::{CardIdx, Cost, Role, SimDeck, SimEffect, SimTrigger};
use super::super::resolution::{begin_resolution, end_resolution};
use super::cast_costs::{PaidCastCosts, pay_cast_costs};
use super::entry::{
    enter_cast_card, exchange_graveyard_creatures, resolve_sacrifice_search, reveal_permanents,
};
use super::settlement::{fire_cast_triggers, settle_entries};
use super::spell_effects::resolve_spell_effects;

/// How a cast was paid. An X spell cast without paying its mana cost has
/// only one legal X value, zero (CR 107.3b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::deck::simulator) enum CastPayment {
    /// The caster paid the printed mana cost.
    Mana,
    /// The printed mana cost is waived, but additional costs still apply.
    /// X is zero (CR 107.3b).
    Free,
    /// A parsed alternative cost replaces the spell's mana cost.
    Alternative,
    /// A flashback cast pays mana and exiles the spell as it leaves the stack.
    Flashback,
}

/// Pay the raw selected mana cost, then resolve the spell and its copies.
/// Targets must be selected before any cost changes game state.
#[allow(clippy::too_many_arguments)]
pub(in crate::deck::simulator) fn resolve_cast(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    idx: CardIdx,
    raw_cost: &Cost,
    cast_ids: &mut Vec<CardIdx>,
    cast_ets: &mut Vec<(u32, CardIdx)>,
    spent_total: &mut u32,
    repeatable_sources: &mut Vec<(u32, u32)>,
    resolve_cascade: bool,
    payment: CastPayment,
    targets: PreparedSpellTargets,
) {
    let card = &deck[idx];
    let selected = if matches!(payment, CastPayment::Mana | CastPayment::Flashback) {
        *raw_cost
    } else {
        Cost::default()
    };
    let choice = super::payment::choose_cast_cost(deck, st, pool, idx, selected)
        .expect("cast gate established the selected total cost is payable");
    begin_resolution(st);
    let costs = pay_cast_costs(
        deck,
        st,
        pool,
        turn as u32,
        idx,
        &choice,
        spent_total,
        payment,
    );
    cast_ids.push(idx);
    let prior_spells = st.spells_cast_this_turn;
    st.spells_cast_this_turn += 1;
    for permanent in &st.battlefield {
        if let Some(yield_) = &card_of(deck, permanent).spell_data.mana_per_cast {
            add_yield_turns_unscaled(yield_, pool, turn as u32);
        }
    }
    fire_cast_triggers(deck, st, pool, turn as u32, card);
    if !card.is_creature && card.role != Role::Land {
        super::super::game::record_noncreature_cast(deck, st);
    }
    end_resolution(deck, st, Some(pool));
    if st.completion.is_some() {
        return;
    }
    if resolve_cascade && card.has_cascade {
        resolve_cascade_cast(
            deck,
            st,
            pool,
            turn,
            card.mana_value,
            cast_ids,
            cast_ets,
            spent_total,
            repeatable_sources,
        );
        if st.completion.is_some() {
            return;
        }
    }
    SpellResolution {
        index: idx,
        turn: turn as u32,
        x: choice.x,
        payment,
        costs,
        targets,
    }
    .resolve(deck, st, pool, prior_spells, cast_ets, repeatable_sources);
}

/// Prepare a cascade hit's targets before its mandatory costs can change zones.
#[allow(clippy::too_many_arguments)]
fn resolve_cascade_cast(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    mana_value: u32,
    cast_ids: &mut Vec<CardIdx>,
    cast_ets: &mut Vec<(u32, CardIdx)>,
    spent_total: &mut u32,
    repeatable_sources: &mut Vec<(u32, u32)>,
) {
    if let Some(free_idx) = reveal_cascade(deck, st, mana_value) {
        if free_cast_payable(deck, st, pool, free_idx)
            && let Some(targets) = prepare_spell_targets(deck, st, &deck[free_idx])
        {
            if !deck[free_idx].is_instant_or_sorcery {
                super::super::game::milestone_for_turn(st, turn as u32)
                    .free_cast_permanents_entered += 1;
            }
            resolve_cast(
                deck,
                st,
                pool,
                turn,
                free_idx,
                &Cost::default(),
                cast_ids,
                cast_ets,
                spent_total,
                repeatable_sources,
                false,
                CastPayment::Free,
                targets,
            );
        } else {
            st.library.insert(0, free_idx);
        }
    }
}

/// Paid choices and pre-cost target identities shared by a spell and its copies.
struct SpellResolution {
    index: CardIdx,
    turn: u32,
    x: u32,
    payment: CastPayment,
    costs: PaidCastCosts,
    targets: PreparedSpellTargets,
}

impl SpellResolution {
    /// Settle each copy and its triggers before resolving the next stack object.
    fn resolve(
        self,
        deck: &SimDeck,
        st: &mut GameState,
        pool: &mut ManaPool,
        prior_spells: u32,
        entries: &mut Vec<(u32, CardIdx)>,
        repeatable_sources: &mut Vec<(u32, u32)>,
    ) {
        let card = &deck[self.index];
        let resolutions = 1 + if card.keyword_abilities.storm {
            prior_spells
        } else {
            0
        };
        for resolution in 0..resolutions {
            let original = resolution + 1 == resolutions;
            begin_resolution(st);
            if self.targets.any_legal_target(st) {
                self.apply(deck, st, pool, entries, repeatable_sources, original);
            }
            if original && card.is_instant_or_sorcery {
                if card.exile_on_resolve || self.payment == CastPayment::Flashback {
                    st.exile.push(self.index);
                } else {
                    move_to_graveyard(deck, st, self.index, self.turn, CardZone::Stack);
                }
            }
            settle_entries(deck, st, self.turn, entries, repeatable_sources);
            end_resolution(deck, st, Some(pool));
            if st.completion.is_some() {
                return;
            }
            st.opponents.settle_losses();
        }
    }

    /// Resolve legal spell effects and linked riders inside one resolution scope.
    fn apply(
        &self,
        deck: &SimDeck,
        st: &mut GameState,
        pool: &mut ManaPool,
        entries: &mut Vec<(u32, CardIdx)>,
        repeatable_sources: &mut Vec<(u32, u32)>,
        original: bool,
    ) {
        let card = &deck[self.index];
        st.life += self.costs.alternative_life_gain as i32;
        st.life_gained += self.costs.alternative_life_gain;
        resolve_sacrifice_search(
            deck,
            st,
            self.turn,
            self.costs.sacrifice_mana_value,
            card.spell_data.search_after_sacrifice,
            entries,
        );
        if card.spell_data.grants_flashback {
            let permissions: Vec<_> = st
                .graveyard
                .iter()
                .copied()
                .filter(|index| deck[*index].is_instant_or_sorcery)
                .map(|index| (index, super::super::game::graveyard_generation(st, index)))
                .collect();
            st.flashback_permissions.extend(permissions);
        }
        if card.spell_data.graveyard_creature_exchange {
            exchange_graveyard_creatures(deck, st, self.turn, entries);
        }
        if original {
            enter_cast_card(
                deck,
                st,
                self.index,
                self.turn,
                self.x,
                self.costs.offspring,
                entries,
                repeatable_sources,
            );
        }
        let seen_before = st.seen;
        resolve_spell_effects(
            deck,
            st,
            pool,
            self.index,
            self.turn,
            self.x,
            self.costs.kicked,
            self.targets.clone(),
        );
        if card.spell_data.additional_cost_life > 0 {
            let draws = st
                .seen
                .saturating_sub(seen_before)
                .min(card.spell_data.draws_on_cast);
            st.life_funded_draws += draws;
            super::super::game::milestone_for_turn(st, self.turn).life_funded_draws += draws;
        }
        if card.spell_data.x_class == Some(super::super::model::XClass::RevealPermanents) {
            reveal_permanents(deck, st, self.turn, self.x, entries);
        }
    }
}

/// Check a physical card's minimum mana payment with its supported cast resources.
pub(in crate::deck::simulator) fn spell_mana_ready(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    index: CardIdx,
) -> bool {
    let card = &deck[index];
    let cost = cast_cost(deck, st, card, card.min_cost, Some(index));
    card.has_mana_cost
        && !card.spell_data.unsupported_additional_cost
        && st.life >= phyrexian_life_charge(&cost) as i32
        && super::payment::prepare_cast_payment(deck, st, pool, index, &cost).is_some()
}

/// Check mana and other mandatory costs when the printed mana cost is waived.
pub(in crate::deck::simulator) fn free_cast_payable(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    index: CardIdx,
) -> bool {
    super::additional_costs_payable(deck, st, index)
        && super::payment::choose_x_cost(deck, st, pool, index, Cost::default()).is_some()
}

/// Reveal the first lower-value nonland and put misses on the library bottom.
fn reveal_cascade(deck: &SimDeck, st: &mut GameState, mana_value: u32) -> Option<CardIdx> {
    let mut exposed = Vec::new();
    let mut hit = None;
    while let Some(index) = st.library.pop() {
        st.seen += 1;
        st.awareness_cards += 1;
        let candidate = &deck[index];
        if candidate.role != Role::Land && candidate.mana_value < mana_value {
            hit = Some(index);
            break;
        }
        exposed.push(index);
    }
    for index in exposed.into_iter().rev() {
        st.library.insert(0, index);
    }
    hit
}

/// Return an upkeep trigger's draw amount when it has executable effects.
pub(in crate::deck::simulator) fn upkeep_trigger_registration(
    ability: &super::super::model::SimAbility,
) -> Option<u32> {
    if !ability.kind.is_triggered()
        || ability.trigger != SimTrigger::Upkeep
        || ability.effect_sequence().is_empty()
    {
        return None;
    }
    Some(
        ability
            .effect_sequence()
            .iter()
            .filter_map(|effect| match effect {
                SimEffect::Draw(amount) => Some(*amount),
                _ => None,
            })
            .sum(),
    )
}

/// X reductions, independent copies, and simultaneous revealed entries.
#[cfg(test)]
#[path = "tests/cast_resolution_tests.rs"]
mod cast_resolution_tests;
