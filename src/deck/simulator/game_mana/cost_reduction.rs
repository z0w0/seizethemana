//! Live battlefield contributions to flat generic spell-cost reductions.

use crate::deck::simulator::game::{Permanent, card_of};
use crate::deck::simulator::model::{SimCard, SimDeck};

/// Sum matching exposed sources; the caller reduces only the total cost's generic mana.
pub(in crate::deck::simulator) fn spell_cost_reduction(
    deck: &SimDeck,
    card: &SimCard,
    battlefield: &[Permanent],
) -> u32 {
    battlefield
        .iter()
        .filter(|permanent| !permanent.face_down)
        .map(|permanent| (permanent, card_of(deck, permanent)))
        .filter(|(permanent, source)| {
            !source.is_mdfc_spell
                || permanent.land_face == source.card_types.iter().any(|kind| kind == "land")
        })
        .flat_map(|(_, source)| &source.cost_reductions)
        .filter(|reduction| reduction.filter.matches(card))
        .fold(0u32, |total, reduction| {
            total.saturating_add(reduction.amount)
        })
}

/// Check source lifetime, face state, and casting-cost integration.
#[cfg(test)]
#[path = "tests/cost_reduction_tests.rs"]
mod cost_reduction_tests;

/// Exercise total casting costs through the production cast pass.
#[cfg(test)]
#[path = "tests/cost_reduction_cast_tests.rs"]
mod cost_reduction_cast_tests;
