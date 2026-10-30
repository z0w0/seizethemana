//! Choose and resolve spells that the current mana pool can pay for.

use super::super::game::{GameState, ManaPool};
use super::super::game_effects::prepare_spell_targets;
use super::super::game_mana::{
    cast_cost, cast_restrictions, payable, phyrexian_life_charge, usable_for_classes,
};
use super::super::model::{CardIdx, Cost, SimDeck};
use super::payment::prepare_cast_payment;
use super::{CastPayment, resolve_cast, resolve_face_down, select_alternative_cost_cards};

/// One affordability sweep over the remaining queue. Casts deduct from
/// the pool and add mana (rituals), so a card skipped as unaffordable
/// here can pay off in a later sweep; each cast removes its index from
/// the queue. Returns true when at least one card was cast.
#[allow(clippy::too_many_arguments)]
pub(super) fn cast_pass(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    queue: &mut Vec<CardIdx>,
    cast_ids: &mut Vec<CardIdx>,
    cast_ets: &mut Vec<(u32, CardIdx)>,
    spent_total: &mut u32,
    repeatable_sources: &mut Vec<(u32, u32)>,
    pip_blocks: &mut Vec<(CardIdx, usize)>,
    blocked_colors: &mut [bool; 5],
) -> bool {
    let mut idx = 0usize;
    while idx < queue.len() {
        let card_idx = queue[idx];
        // Mid-cast hand changes (wheels, loots) shift or shrink the
        // hand: skip when the cast's card already left the hand.
        if !st.hand.contains(&card_idx) {
            queue.remove(idx);
            continue;
        }
        let card = &deck[card_idx];
        if !card.has_mana_cost {
            idx += 1;
            continue;
        }
        if !super::additional_costs_payable(deck, st, card_idx) {
            idx += 1;
            continue;
        }
        let Some(targets) = prepare_spell_targets(deck, st, card) else {
            idx += 1;
            continue;
        };
        let alternate_cards = select_alternative_cost_cards(deck, st, card_idx);
        if !alternate_cards.is_empty()
            && st.hand.len() - 1 - alternate_cards.len()
                >= card.spell_data.additional_cost_discards as usize
            && prepare_cast_payment(
                deck,
                st,
                pool,
                card_idx,
                &cast_cost(deck, st, card, Cost::default(), Some(card_idx)),
            )
            .is_some()
        {
            resolve_cast(
                deck,
                st,
                pool,
                turn,
                card_idx,
                &Cost::default(),
                cast_ids,
                cast_ets,
                spent_total,
                repeatable_sources,
                true,
                CastPayment::Alternative,
                targets,
            );
            queue.remove(idx);
            return true;
        }
        let eff = cast_cost(deck, st, card, card.min_cost, Some(card_idx));
        // Life costs pay from one total: phyrexian pips at 2 life each
        // (CR 107.4f) plus any "pay N life" additional cost. CR 119.4
        // requires the player to cover the whole amount, so a cast short
        // of the combined life cost stays put.
        let life_cost = card.spell_data.additional_cost_life + phyrexian_life_charge(&eff);
        if st.life < life_cost as i32 {
            idx += 1;
            continue;
        }
        if prepare_cast_payment(deck, st, pool, card_idx, &eff).is_none() {
            // Morph/disguise fallback (CR 702.37/702.168): when the
            // face-up cost is unaffordable but {3} is, cast the card
            // face down as a 2/2 body. It may be turned up later.
            if card.morph_cost.is_some() {
                let face_down = Cost {
                    generic: 3,
                    ..Cost::default()
                };
                if payable(&face_down, pool) {
                    resolve_face_down(deck, st, pool, turn, card_idx, cast_ids, spent_total);
                    queue.remove(idx);
                    return true;
                }
            }
            // Colors a cast was blocked for: enough total, missing pips
            // (read from the effective cost; improvise and affinity
            // change the generic part only, so printed and effective
            // pips match, but the total must use the same lens as the
            // cast gate).
            if usable_for_classes(pool, &cast_restrictions(card))
                >= eff.total() - phyrexian_life_charge(&eff) / 2
            {
                let flexible = pool.flexible;
                for (ci, need) in card.cost.pips.iter().enumerate() {
                    if *need > 0 && pool.fixed[ci] + flexible < u32::from(*need) {
                        blocked_colors[ci] = true;
                        pip_blocks.push((card_idx, ci));
                    }
                }
            }
            idx += 1;
            continue;
        }
        resolve_cast(
            deck,
            st,
            pool,
            turn,
            card_idx,
            &card.min_cost,
            cast_ids,
            cast_ets,
            spent_total,
            repeatable_sources,
            true,
            CastPayment::Mana,
            targets,
        );
        queue.remove(idx);
        return true;
    }
    false
}
