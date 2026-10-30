//! Optional casting costs selected before one complete mana payment.

use super::{choose_x_cost, prepare_cast_payment};
use crate::deck::simulator::game::{GameState, ManaPool};
use crate::deck::simulator::game_mana::{cast_cost, phyrexian_life_charge};
use crate::deck::simulator::model::{CardIdx, Cost, SimCard, SimDeck};

/// The chosen X value and optional costs included in one final casting cost.
pub(in crate::deck::simulator::cast_pass) struct CastCostChoice {
    /// Shared X value chosen before optional costs.
    pub(in crate::deck::simulator::cast_pass) x: u32,
    /// Total mana cost after all additions and generic reductions.
    pub(in crate::deck::simulator::cast_pass) cost: Cost,
    /// The total includes the card's kicker cost.
    pub(in crate::deck::simulator::cast_pass) kicked: bool,
    /// The total includes the card's offspring cost.
    pub(in crate::deck::simulator::cast_pass) offspring: bool,
}

/// Maximize X first, then try kicker and offspring against the complete raw cost.
pub(in crate::deck::simulator::cast_pass) fn choose_cast_cost(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    index: CardIdx,
    raw_cost: Cost,
) -> Option<CastCostChoice> {
    let card = &deck[index];
    let (x, cost) = choose_x_cost(deck, st, pool, index, raw_cost)?;
    if !life_payable(card, st, &cost) {
        return None;
    }
    let mut choice = CastCostChoice {
        x,
        cost,
        kicked: false,
        offspring: false,
    };
    let mut raw_total = raw_cost.with_x(x)?;
    if let Some(kicker) = card.spell_data.kicker
        && let Some((raw, total)) = optional_total(deck, st, pool, index, raw_total, kicker)
    {
        raw_total = raw;
        choice.cost = total;
        choice.kicked = true;
    }
    if !card.is_instant_or_sorcery
        && let Some(offspring) = card.keyword_abilities.offspring
        && let Some((_, total)) = optional_total(deck, st, pool, index, raw_total, offspring)
    {
        choice.cost = total;
        choice.offspring = true;
    }
    Some(choice)
}

/// Test one optional addition without paying resources or losing prior generic cuts.
fn optional_total(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    index: CardIdx,
    raw: Cost,
    extra: Cost,
) -> Option<(Cost, Cost)> {
    let raw = combine_costs(raw, extra)?;
    let card = &deck[index];
    let total = cast_cost(deck, st, card, raw, Some(index));
    if !life_payable(card, st, &total) {
        return None;
    }
    prepare_cast_payment(deck, st, pool, index, &total)?;
    Some((raw, total))
}

/// Reserve one life budget for mandatory life and every Phyrexian pip in the total.
fn life_payable(card: &SimCard, st: &GameState, cost: &Cost) -> bool {
    let life =
        i64::from(card.spell_data.additional_cost_life) + i64::from(phyrexian_life_charge(cost));
    i64::from(st.life) >= life
}

/// Combine fixed optional mana symbols without dropping unsupported or variable costs.
fn combine_costs(mut base: Cost, extra: Cost) -> Option<Cost> {
    if extra.unsupported || extra.x_symbols != 0 {
        return None;
    }
    base.generic = base.generic.checked_add(extra.generic)?;
    add_symbols(&mut base.pips, extra.pips)?;
    add_symbols(&mut base.hybrid, extra.hybrid)?;
    add_symbols(&mut base.twobrid, extra.twobrid)?;
    add_symbols(&mut base.phyrexian, extra.phyrexian)?;
    base.snow = base.snow.checked_add(extra.snow)?;
    base.colorless = base.colorless.checked_add(extra.colorless)?;
    Some(base)
}

/// Keep every mana symbol count exact when adding a fixed optional cost.
fn add_symbols<const N: usize>(base: &mut [u8; N], extra: [u8; N]) -> Option<()> {
    for (base, extra) in base.iter_mut().zip(extra) {
        *base = base.checked_add(extra)?;
    }
    Some(())
}

/// Optional choices use one total and the same payment gate as ordinary casts.
#[cfg(test)]
#[path = "tests/cost_choice_tests.rs"]
mod cost_choice_tests;
