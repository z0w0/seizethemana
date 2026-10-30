//! State-aware additions to a spell's selected casting cost.

use crate::deck::simulator::game::GameState;
use crate::deck::simulator::model::{CardIdx, Cost, SimCard, SimDeck};

/// Generic mana owed when the caster cannot make the spell's Behold choice.
pub(in crate::deck::simulator) fn cast_cost_increase(
    deck: &SimDeck,
    st: &GameState,
    card: &SimCard,
    excluded_hand: Option<CardIdx>,
) -> u32 {
    card.spell_data
        .behold_alternative
        .as_ref()
        .map_or(0, |alternative| {
            crate::deck::simulator::game_effects::behold::generic_surcharge(
                deck,
                st,
                alternative,
                excluded_hand,
            )
        })
}

/// Add Behold's extra mana to the selected cost before applying generic reductions.
///
/// The selected cost must already contain its chosen X value. Exclude a pending
/// hand or replay spell with `Some(index)`; command-zone casts use `None`.
pub(in crate::deck::simulator) fn cast_cost(
    deck: &SimDeck,
    st: &GameState,
    card: &SimCard,
    selected_cost: Cost,
    excluded_hand: Option<CardIdx>,
) -> Cost {
    effective_cast_cost(deck, st, card, selected_cost, 0, excluded_hand)
}

/// Add explicit increases and Behold mana before affinity and flat generic cuts.
///
/// Use commander tax as `increase` and `None` as `excluded` for a commander.
/// Other casts use zero and `Some(index)`. The raw cost must already have X fixed.
pub(in crate::deck::simulator) fn effective_cast_cost(
    deck: &SimDeck,
    st: &GameState,
    card: &SimCard,
    raw: Cost,
    increase: u32,
    excluded: Option<CardIdx>,
) -> Cost {
    super::adjust_cast_cost(
        deck,
        card,
        &st.battlefield,
        raw,
        increase.saturating_add(cast_cost_increase(deck, st, card, excluded)),
    )
}

/// Verify Behold increases through complete casts and unchanged reveal resources.
#[cfg(test)]
#[path = "tests/cast_costs_tests.rs"]
mod cast_costs_tests;

/// Check Behold increases on replay, pitch, and cascade casting routes.
#[cfg(test)]
#[path = "tests/behold_cast_routes_tests.rs"]
mod behold_cast_routes_tests;
