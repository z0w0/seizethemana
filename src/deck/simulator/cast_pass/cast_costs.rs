//! Pay the supported costs of a chosen spell before cast triggers resolve.

use super::super::game::{GameState, ManaPool, card_of, is_creature_permanent};
use super::super::game_effects::{CardZone, move_to_graveyard, resolve_sacrifice_batch};
use super::super::game_mana::phyrexian_life_charge;
use super::super::model::{AlternativeCostPayoff, CardIdx, SimDeck};
use super::payment::{CastCostChoice, prepare_cast_payment};
use super::{CastPayment, select_alternative_cost_cards};

/// Cost choices needed when the spell resolves.
pub(super) struct PaidCastCosts {
    pub(super) kicked: bool,
    pub(super) offspring: bool,
    pub(super) sacrifice_mana_value: Option<u32>,
    pub(super) alternative_life_gain: u32,
}

/// Pay mandatory and selected optional costs without resolving the spell.
///
/// The caller must first check cast legality and all required payments against
/// the same game state and pool. This function commits costs without repeating
/// the full legality check.
///
/// # Panics
/// Panics if required mana or selected hand resources are no longer available.
#[allow(clippy::too_many_arguments)]
pub(super) fn pay_cast_costs(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
    index: CardIdx,
    choice: &CastCostChoice,
    spent: &mut u32,
    payment: CastPayment,
) -> PaidCastCosts {
    let card = &deck[index];
    let prepared = prepare_cast_payment(deck, st, pool, index, &choice.cost)
        .expect("cast gate prepared the complete mana payment");
    let before = pool.total();
    assert!(
        prepared.commit(st, pool),
        "cast payment uses unchanged resources"
    );
    *spent += before - pool.total();
    let life = phyrexian_life_charge(&choice.cost);
    st.life -= life as i32;
    st.life_paid += life;
    if let Some(position) = st.hand.iter().position(|held| *held == index) {
        st.hand.remove(position);
    }
    let life = card.spell_data.additional_cost_life;
    st.life -= life as i32;
    st.life_paid += life;
    pay_teamwork(deck, st, card.keyword_abilities.teamwork);
    let alternative_life_gain = pay_hand_costs(deck, st, index, turn, payment);
    let sacrifices: Vec<_> = st
        .battlefield
        .iter()
        .filter(|permanent| is_creature_permanent(deck, permanent))
        .take(card.spell_data.additional_cost_creatures as usize)
        .map(|permanent| (permanent.uid, card_of(deck, permanent).mana_value))
        .collect();
    let sacrifice_mana_value = if card.spell_data.search_after_sacrifice.is_some() {
        sacrifices.first().map(|(_, mana_value)| *mana_value)
    } else {
        None
    };
    let uids: Vec<_> = sacrifices.into_iter().map(|(uid, _)| uid).collect();
    resolve_sacrifice_batch(deck, st, turn, &uids);
    PaidCastCosts {
        kicked: choice.kicked,
        offspring: choice.offspring,
        sacrifice_mana_value,
        alternative_life_gain,
    }
}

/// Reserve alternate-cost cards so additional discards cannot spend them too.
fn pay_hand_costs(
    deck: &SimDeck,
    st: &mut GameState,
    index: CardIdx,
    turn: u32,
    payment: CastPayment,
) -> u32 {
    let card = &deck[index];
    let alternate = if payment == CastPayment::Alternative {
        select_alternative_cost_cards(deck, st, index)
    } else {
        Vec::new()
    };
    for _ in 0..card.spell_data.additional_cost_discards {
        let position = st
            .hand
            .iter()
            .position(|held| !alternate.contains(held))
            .expect("cast gate reserved a distinct card for each discard");
        let discarded = st.hand.remove(position);
        move_to_graveyard(deck, st, discarded, turn, CardZone::Hand);
    }
    let mut total = 0;
    for exiled in alternate {
        let position = st
            .hand
            .iter()
            .position(|held| *held == exiled)
            .expect("cast gate reserved the alternate-cost card");
        st.hand.remove(position);
        st.exile.push(exiled);
        total += deck[exiled].mana_value;
    }
    if card
        .spell_data
        .alternative_cast_cost
        .is_some_and(|cost| cost.payoff == AlternativeCostPayoff::GainLifeEqualToExiledManaValue)
    {
        total
    } else {
        0
    }
}

/// Tap a complete optional teamwork payment before the spell enters play.
fn pay_teamwork(deck: &SimDeck, st: &mut GameState, required: Option<u32>) {
    let Some(required) = required else { return };
    let mut selected = Vec::new();
    let mut power = 0u32;
    for permanent in &st.battlefield {
        if power >= required {
            break;
        }
        if !permanent.tapped && is_creature_permanent(deck, permanent) {
            power = power.saturating_add(super::super::game_effects::current_creature_power(
                permanent,
                deck,
                &st.battlefield,
            ));
            selected.push(permanent.uid);
        }
    }
    if power < required {
        return;
    }
    for uid in selected {
        let permanent = st
            .battlefield
            .iter_mut()
            .find(|permanent| permanent.uid == uid)
            .expect("teamwork creature remains on the battlefield during payment");
        permanent.tapped = true;
    }
}
