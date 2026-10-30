//! Shared preparation for spell mana, convoke, improvise, and delve payments.

use super::super::game::{GameState, ManaPool, card_of, is_creature_permanent};
use super::super::game_mana::{
    ManaPayment, artifact_cost_reduction, cast_cost, cast_restrictions,
    payment_plan_with_virtual_sources, spell_cost_reduction,
};
use super::super::model::{CardIdx, Cost, SimCard, SimDeck};

/// Optional additions chosen before one complete casting-cost payment.
#[path = "cost_choice.rs"]
mod cost_choice;
pub(super) use cost_choice::{CastCostChoice, choose_cast_cost};

/// Complete spell mana payment, including chosen tap and delve resources.
pub(super) struct CastManaPayment {
    mana: ManaPayment,
    tapped: Vec<u32>,
    delve: Vec<CardIdx>,
}

/// Current cast resources, shared with mana-mode planning without paying them.
pub(in crate::deck::simulator) struct CastPaymentResources {
    /// Mana still owed after selecting supported delve cards.
    pub(in crate::deck::simulator) owed: Cost,
    /// One generic-or-colored virtual tap choice per physical permanent UID.
    pub(in crate::deck::simulator) tap_sources: Vec<(u32, u8)>,
    delve: Vec<CardIdx>,
}

impl CastManaPayment {
    /// Commit all prepared resources before the spell changes game state.
    pub(super) fn commit(self, st: &mut GameState, pool: &mut ManaPool) -> bool {
        if self
            .tapped
            .iter()
            .any(|uid| !st.battlefield.iter().any(|p| p.uid == *uid && !p.tapped))
            || self.delve.iter().any(|index| !st.graveyard.contains(index))
        {
            return false;
        }
        if !self.mana.commit(pool) {
            return false;
        }
        for uid in self.tapped {
            let permanent = st
                .battlefield
                .iter_mut()
                .find(|p| p.uid == uid)
                .expect("prepared cast resource remains on the battlefield");
            permanent.tapped = true;
        }
        for index in self.delve {
            let position = st
                .graveyard
                .iter()
                .position(|held| *held == index)
                .expect("prepared delve card remains in the graveyard");
            st.graveyard.remove(position);
            st.exile.push(index);
        }
        true
    }
}

/// Choose cast resources without changing state or consuming RNG.
pub(super) fn prepare_cast_payment(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    index: CardIdx,
    cost: &Cost,
) -> Option<CastManaPayment> {
    let resources = cast_payment_resources(deck, st, index, cost);
    let sources: Vec<_> = resources
        .tap_sources
        .iter()
        .map(|(_, colors)| *colors)
        .collect();
    let (mana, used) = payment_plan_with_virtual_sources(
        &resources.owed,
        pool,
        &cast_restrictions(&deck[index]),
        &sources,
    )?;
    Some(CastManaPayment {
        mana,
        tapped: used
            .into_iter()
            .map(|source| resources.tap_sources[source].0)
            .collect(),
        delve: resources.delve,
    })
}

/// Describe the exact delve and virtual-tap resources used by cast preparation.
pub(in crate::deck::simulator) fn cast_payment_resources(
    deck: &SimDeck,
    st: &GameState,
    index: CardIdx,
    cost: &Cost,
) -> CastPaymentResources {
    let card = &deck[index];
    let mut owed = *cost;
    let delve: Vec<_> = st
        .graveyard
        .iter()
        .rev()
        .copied()
        .filter(|candidate| *candidate != index)
        .take(if card.keyword_abilities.delve {
            owed.generic as usize
        } else {
            0
        })
        .collect();
    owed.generic -= delve.len() as u32;
    CastPaymentResources {
        owed,
        tap_sources: virtual_tap_sources(deck, st, card),
        delve,
    }
}

/// Assign one shared resource to each permanent eligible for convoke or improvise.
fn virtual_tap_sources(deck: &SimDeck, st: &GameState, card: &SimCard) -> Vec<(u32, u8)> {
    if !card.keyword_abilities.convoke && !card.keyword_abilities.improvise {
        return Vec::new();
    }
    st.battlefield
        .iter()
        .filter(|permanent| !permanent.tapped)
        .filter_map(|permanent| {
            let convoke = card.keyword_abilities.convoke && is_creature_permanent(deck, permanent);
            let improvise = card.keyword_abilities.improvise
                && !permanent.face_down
                && card_of(deck, permanent).is_artifact;
            (convoke || improvise).then(|| {
                let colors = if convoke && !permanent.face_down {
                    card_of(deck, permanent)
                        .colors
                        .iter()
                        .enumerate()
                        .fold(0, |mask, (color, present)| {
                            mask | (u8::from(*present) << color)
                        })
                } else {
                    0
                };
                (permanent.uid, colors)
            })
        })
        .collect()
}

/// Choose X in the raw selected cost before applying battlefield cost reductions.
pub(in crate::deck::simulator) fn choose_x_cost(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    index: CardIdx,
    cost: Cost,
) -> Option<(u32, Cost)> {
    let card = &deck[index];
    let chosen_cost = |x| Some(cast_cost(deck, st, card, cost.with_x(x)?, Some(index)));
    if cost.x_symbols == 0 {
        let cost = chosen_cost(0)?;
        prepare_cast_payment(deck, st, pool, index, &cost)?;
        return Some((0, cost));
    }
    let available = u64::from(pool.total())
        + st.battlefield.len() as u64
        + st.graveyard.len() as u64
        + u64::from(artifact_cost_reduction(deck, card, &st.battlefield))
        + u64::from(spell_cost_reduction(deck, card, &st.battlefield));
    let mut high = (available / u64::from(cost.x_symbols)).min(u64::from(u32::MAX)) as u32;
    let mut low = 0;
    prepare_cast_payment(deck, st, pool, index, &chosen_cost(0)?)?;
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if chosen_cost(middle)
            .is_some_and(|cost| prepare_cast_payment(deck, st, pool, index, &cost).is_some())
        {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    Some((low, chosen_cost(low)?))
}

/// Check X payment through complete casts, including repeated symbols and zero.
#[cfg(test)]
#[path = "../tests/x_payment_tests.rs"]
mod x_payment_tests;
