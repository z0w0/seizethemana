//! Mana production and shared, atomic payment for the simulator.

use super::game::{ManaPool, Permanent, card_of};
use super::model::{Cost, ManaYield, Role, Scale, SimDeck, SpendRestriction};

/// Exact payment planning for produced mana.
mod payment;
pub(super) use payment::payment_plan_with_cast_sources;
pub(super) use payment::payment_plan_with_source_costs;
pub(super) use payment::{ManaPayment, payment_plan, payment_plan_with_virtual_sources};

/// Live flat generic reductions from exposed battlefield sources.
mod cost_reduction;
pub(in crate::deck::simulator) use cost_reduction::spell_cost_reduction;
/// State-aware additional generic casting costs.
mod cast_costs;
pub(in crate::deck::simulator) use cast_costs::{
    cast_cost, cast_cost_increase, effective_cast_cost,
};

/// All five colored mana types, in WUBRG bit order.
const COLORED: u8 = 31;
/// The colorless mana type, separate from generic-cost credit.
const COLORLESS: u8 = 32;

/// Produced mana whose allowed types and spend rules remain attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManaLot {
    /// Mana left in this lot.
    pub(super) amount: u32,
    /// Allowed WUBRG types (bits 0 through 4) and colorless (bit 5).
    pub(super) types: u8,
    /// Cast class required to spend this mana, when restricted.
    pub(super) restriction: Option<SpendRestriction>,
    /// This mana cannot pay a generic part of a cost.
    pub(super) cannot_pay_generic: bool,
    /// The entire lot must use one chosen mana type.
    pub(super) one_color: bool,
}

/// Colors among permanents eligible for a source's Oracle condition.
fn colors_present(deck: &SimDeck, battlefield: &[Permanent], legendary_only: bool) -> [bool; 5] {
    let mut found = [false; 5];
    for permanent in battlefield {
        let card = card_of(deck, permanent);
        if legendary_only && !(card.is_legendary && (card.is_creature || card.is_planeswalker)) {
            continue;
        }
        for (present, color) in found.iter_mut().zip(card.colors) {
            *present |= color;
        }
    }
    found
}

/// Add produced mana without discarding its allowed types or restrictions.
fn add_lot(pool: &mut ManaPool, amount: u32, types: u8, y: &ManaYield) {
    if amount == 0 || types == 0 {
        return;
    }
    let lot = ManaLot {
        amount,
        types,
        restriction: y.restriction,
        cannot_pay_generic: y.cannot_pay_generic,
        one_color: y.one_color && amount > 1,
    };
    if lot.restriction.is_none() && !lot.one_color {
        if types == COLORLESS && !lot.cannot_pay_generic {
            pool.colorless += amount;
            return;
        }
        if types == COLORED && !lot.cannot_pay_generic {
            pool.flexible += amount;
            return;
        }
        if types.count_ones() == 1 && types & COLORED != 0 {
            let color = types.trailing_zeros() as usize;
            pool.fixed[color] += amount;
            if lot.cannot_pay_generic {
                pool.fixed_no_generic[color] += amount;
            }
            return;
        }
    }
    if !lot.one_color
        && let Some(existing) = pool.lots.iter_mut().find(|existing| {
            existing.types == lot.types
                && existing.restriction == lot.restriction
                && existing.cannot_pay_generic == lot.cannot_pay_generic
                && !existing.one_color
        })
    {
        existing.amount += amount;
    } else {
        pool.lots.push(lot);
    }
}

/// Add one yield with resolved battlefield conditions and commander color identity.
fn add_resolved_yield(
    y: &ManaYield,
    pool: &mut ManaPool,
    turn: u32,
    commander_identity: [bool; 5],
) {
    if y.opponent_any {
        if turn >= 2 && !y.cannot_pay_generic {
            if y.restriction.is_some() {
                pool.lots.push(ManaLot {
                    amount: y.any_pips,
                    types: 0,
                    restriction: y.restriction,
                    cannot_pay_generic: false,
                    one_color: false,
                });
            } else {
                pool.generic_credit += y.any_pips;
            }
        }
        return;
    }
    let choices = y.choice.iter().enumerate().fold(0, |mask, (i, present)| {
        mask | if *present { 1 << i } else { 0 }
    });
    let choice_types = choices | if y.choice_colorless { COLORLESS } else { 0 };
    let commander_types = commander_identity
        .iter()
        .enumerate()
        .fold(0, |mask, (i, present)| {
            mask | if *present { 1 << i } else { 0 }
        });
    if y.alternatives {
        let types = choice_types
            | if y.any_pips > 0 { COLORED } else { 0 }
            | if y.commander_pips > 0 {
                commander_types
            } else {
                0
            }
            | if y.colorless > 0 { COLORLESS } else { 0 };
        add_lot(pool, 1, types, y);
        return;
    }
    for (i, amount) in y.fixed.iter().enumerate() {
        add_lot(pool, u32::from(*amount), 1 << i, y);
    }
    add_lot(pool, y.any_pips, COLORED, y);
    add_lot(pool, y.commander_pips, commander_types, y);
    add_lot(pool, u32::from(choice_types != 0), choice_types, y);
    add_lot(pool, y.colorless, COLORLESS, y);
}

/// Add one tap's yield without battlefield context.
pub(super) fn add_yield(y: &ManaYield, pool: &mut ManaPool) {
    add_yield_turns_unscaled(y, pool, u32::MAX);
}

/// Add an unscaled yield; conditions that need the battlefield produce nothing.
pub(super) fn add_yield_turns_unscaled(y: &ManaYield, pool: &mut ManaPool, turn: u32) {
    if y.legendary_colors || y.scaling == Some(Scale::PerChargeCounter) {
        return;
    }
    add_resolved_yield(y, pool, turn, [false; 5]);
}

/// Add a yield after resolving supported battlefield conditions.
pub(super) fn add_yield_turns(
    deck: &SimDeck,
    y: &ManaYield,
    pool: &mut ManaPool,
    turn: u32,
    battlefield: &[Permanent],
) {
    if y.scaling == Some(Scale::PerChargeCounter) {
        return;
    }
    let mut resolved = y.clone();
    if y.legendary_colors {
        resolved.choice = colors_present(deck, battlefield, true);
        resolved.any_pips = 0;
    }
    if y.scaling == Some(Scale::ColorsPresent) {
        for (amount, present) in
            resolved
                .fixed
                .iter_mut()
                .zip(colors_present(deck, battlefield, false))
        {
            *amount += u8::from(present);
        }
    }
    add_resolved_yield(&resolved, pool, turn, deck.commander_color_identity());
}

/// Whether the unrestricted pool can pay the entire mana part of a cost.
pub(super) fn payable(cost: &Cost, pool: &ManaPool) -> bool {
    payment_plan(cost, pool, &[]).is_some()
}

/// Whether an unrestricted payment plan covers every required pip.
pub(super) fn pips_ok(cost: &Cost, pool: &ManaPool) -> bool {
    payable(cost, pool)
}

/// Generic-cost capacity without a cast-class restriction.
pub(super) fn usable_for_noncreature(pool: &ManaPool) -> u32 {
    usable_for_classes(pool, &[])
}

/// Spend classes allowed by the spell's types.
pub(super) fn cast_restrictions(card: &super::model::SimCard) -> Vec<SpendRestriction> {
    let mut classes = Vec::new();
    if card.is_instant_or_sorcery {
        classes.push(SpendRestriction::InstantSorcery);
    }
    if card.is_artifact && card.role != Role::Land {
        classes.push(SpendRestriction::Artifact);
    }
    if card.is_creature {
        classes.push(SpendRestriction::Creature);
    }
    if card.is_legendary {
        classes.push(SpendRestriction::Legendary);
    }
    classes
}

/// Generic-cost capacity, counting each eligible lot only once.
pub(super) fn usable_for_classes(pool: &ManaPool, classes: &[SpendRestriction]) -> u32 {
    pool.fixed
        .iter()
        .zip(pool.fixed_no_generic)
        .map(|(amount, no_generic)| amount - no_generic)
        .sum::<u32>()
        + pool.flexible
        + pool.colorless
        + pool.generic_credit
        + pool.treasures
        + pool
            .lots
            .iter()
            .filter(|lot| !lot.cannot_pay_generic)
            .filter(|lot| lot.restriction.is_none_or(|class| classes.contains(&class)))
            .map(|lot| lot.amount)
            .sum::<u32>()
}

/// Pay a complete class-restricted mana cost or leave the pool unchanged.
pub(super) fn pay_restricted_cost(
    cost: &Cost,
    pool: &mut ManaPool,
    classes: &[SpendRestriction],
) -> bool {
    payment_plan(cost, pool, classes).is_some_and(|plan| plan.commit(pool))
}

/// Life paid when every Phyrexian pip is paid with two life (CR 107.4f).
pub(super) fn phyrexian_life_charge(cost: &Cost) -> u32 {
    2 * cost.phyrexian.iter().map(|p| u32::from(*p)).sum::<u32>()
}

/// Pay a complete unrestricted mana cost or leave the pool unchanged.
pub(super) fn pay_cost(cost: &Cost, pool: &mut ManaPool) -> bool {
    pay_restricted_cost(cost, pool, &[])
}

/// Apply generic battlefield reductions without changing pips or adding state costs.
pub(super) fn effective_min_cost(
    deck: &SimDeck,
    card: &super::model::SimCard,
    battlefield: &[Permanent],
) -> Cost {
    effective_cost_with_increase(deck, card, battlefield, 0)
}

/// Add generic cast costs before applying battlefield reductions.
pub(super) fn effective_cost_with_increase(
    deck: &SimDeck,
    card: &super::model::SimCard,
    battlefield: &[Permanent],
    increase: u32,
) -> Cost {
    adjust_cast_cost(deck, card, battlefield, card.min_cost, increase)
}

/// Apply increases before affinity and flat reductions to the chosen total cost.
pub(super) fn adjust_cast_cost(
    deck: &SimDeck,
    card: &super::model::SimCard,
    battlefield: &[Permanent],
    mut cost: Cost,
    increase: u32,
) -> Cost {
    cost.generic = cost.generic.saturating_add(increase);
    let reduction = artifact_cost_reduction(deck, card, battlefield)
        .saturating_add(spell_cost_reduction(deck, card, battlefield));
    cost.generic = cost.generic.saturating_sub(reduction);
    cost
}

/// Count the artifacts that reduce this spell's generic cast cost.
pub(super) fn artifact_cost_reduction(
    deck: &SimDeck,
    card: &super::model::SimCard,
    battlefield: &[Permanent],
) -> u32 {
    if !card.keyword_abilities.affinity {
        return 0;
    }
    battlefield
        .iter()
        .filter(|p| !p.face_down && card_of(deck, p).is_artifact)
        .count() as u32
}
