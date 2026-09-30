//! Choose and resolve spells that the current mana pool can pay for.

use super::super::game::{GameState, ManaPool, card_of};
use super::super::game_mana::{
    bucket_of, cast_restrictions, effective_min_cost, payable, phyrexian_life_charge, pips_ok,
    usable_for_classes, usable_for_noncreature,
};
use super::super::model::{CardIdx, Cost, SimDeck};
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
    let mut cast_any = false;
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
        if card.is_instant_or_sorcery && !card.has_mana_cost {
            idx += 1;
            continue;
        }
        if !select_alternative_cost_cards(deck, st, card_idx).is_empty() {
            cast_any = true;
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
                CastPayment::Free,
            );
            queue.remove(idx);
            continue;
        }
        let available_creatures = st
            .battlefield
            .iter()
            .filter(|p| p.card.deck_idx().is_some() && card_of(deck, p).is_creature)
            .count();
        let available_discards = st.hand.iter().filter(|i| **i != card_idx).count();
        if available_creatures < card.spell_data.additional_cost_creatures as usize
            || available_discards < card.spell_data.additional_cost_discards as usize
        {
            idx += 1;
            continue;
        }
        let eff = effective_min_cost(deck, card, &st.battlefield);
        // Convoke (CR 702.51) and delve (CR 702.66) add payment options
        // beyond the mana pool: convoke taps untapped bodies for {1}
        // each (a matching-color body can pay a colored pip), delve
        // exiles graveyard cards for {1} generic each. The cast gate
        // counts the available payments; resolution consumes them.
        let convoke = convoke_payment(deck, st, card, &eff);
        let delve = delve_payment(st, card, &eff);
        let extra_payment = convoke.total + delve;
        // Life costs pay from one total: phyrexian pips at 2 life each
        // (CR 107.4f) plus any "pay N life" additional cost. CR 119.4
        // requires the player to cover the whole amount, so a cast short
        // of the combined life cost stays put.
        let life_cost = card.spell_data.additional_cost_life + phyrexian_life_charge(&eff);
        if st.life < life_cost as i32 {
            idx += 1;
            continue;
        }
        // Spend-restricted mana pays only its cast class: the general
        // pool plus the matching bucket counts toward the cast; other
        // restricted buckets do not. The bucket covers the card's pips
        // like any other source (each restricted pip is one mana of the
        // source's chosen color, so the bucket must cover every pip,
        // once).
        // Phyrexian pips pay with life, so the pool owes only the mana
        // part of the cost.
        let mana_total = eff.total() - phyrexian_life_charge(&eff) / 2;
        let mana_total = mana_total.saturating_sub(extra_payment);
        let classes = cast_restrictions(card);
        let (cost_ok, pip_ok) = if classes.is_empty() {
            (
                usable_for_noncreature(pool) + extra_payment >= mana_total,
                pips_ok(&eff, pool) || convoke.can_cover_pips,
            )
        } else {
            // Mixed pips: the cast passes when the matching buckets,
            // on top of the general pool, can legally cover it. A
            // bucket pays any pip (its mana is one color of the
            // source's choice), so bucket + general pips must cover the
            // pip total; the general pool alone covering pips_ok also
            // passes.
            let pip_total: u32 =
                eff.pips.iter().map(|p| u32::from(*p)).sum::<u32>() + eff.hybrid_pips;
            let bucket = classes.iter().map(|c| bucket_of(pool, *c)).sum::<u32>();
            (
                usable_for_classes(pool, &classes) + extra_payment >= mana_total,
                pips_ok(&eff, pool)
                    || convoke.can_cover_pips
                    || (bucket > 0 && bucket + general_coverable(&eff, pool) >= pip_total),
            )
        };
        if !cost_ok || !pip_ok {
            // Morph/disguise fallback (CR 702.37/702.168): when the
            // face-up cost is unaffordable but {3} is, cast the card
            // face down as a 2/2 body. It may be turned up later.
            if card.morph_cost.is_some() {
                let face_down = Cost {
                    generic: 3,
                    ..Cost::default()
                };
                if payable(&face_down, pool) && pips_ok(&face_down, pool) {
                    cast_any = true;
                    resolve_face_down(deck, st, pool, turn, card_idx, cast_ids, spent_total);
                    queue.remove(idx);
                    continue;
                }
            }
            // Colors a cast was blocked for: enough total, missing pips
            // (read from the effective cost; improvise and affinity
            // change the generic part only, so printed and effective
            // pips match, but the total must use the same lens as the
            // cast gate).
            if payable(&eff, pool) && !pips_ok(&eff, pool) {
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
        cast_any = true;
        resolve_cast(
            deck,
            st,
            pool,
            turn,
            card_idx,
            &eff,
            cast_ids,
            cast_ets,
            spent_total,
            repeatable_sources,
            true,
            CastPayment::Mana,
        );
        queue.remove(idx);
    }
    cast_any
}

/// Monocolor pips the general pool (fixed + flexible) already covers,
/// outside the restricted bucket. The mixed-pip gate adds this to the
/// bucket: bucket mana pays any pip, so bucket + general-covered pips
/// must reach the pip total.
fn general_coverable(eff: &Cost, pool: &ManaPool) -> u32 {
    let mut flexible = pool.flexible;
    let mut covered = 0u32;
    for (i, need) in eff.pips.iter().enumerate() {
        let need = u32::from(*need);
        covered += need.min(pool.fixed[i]);
        flexible = flexible.saturating_sub(need.saturating_sub(pool.fixed[i]));
    }
    covered + flexible
}

/// Convoke payment (CR 702.51): every untapped creature can be tapped to
/// pay {1} of the spell's cost, or one matching-colored pip. The helper
/// reports how much generic the creatures can cover and whether they
/// plus the pool can cover the colored pips.
pub(super) struct ConvokePayment {
    /// Generic mana the convokable creatures can pay ({1} each).
    pub(super) total: u32,
    /// True when the creatures, tapped for their colors, can cover every
    /// monocolor pip of the cost.
    pub(super) can_cover_pips: bool,
}

/// Count convoke-eligible creatures and their pip coverage.
fn convoke_payment(
    deck: &SimDeck,
    st: &super::super::game::GameState,
    card: &super::super::model::SimCard,
    eff: &Cost,
) -> ConvokePayment {
    if !card.keyword_abilities.convoke {
        return ConvokePayment {
            total: 0,
            can_cover_pips: false,
        };
    }
    let creatures: Vec<&super::super::game::Permanent> = st
        .battlefield
        .iter()
        .filter(|p| {
            !p.tapped
                && p.card.deck_idx().is_some()
                && super::super::game::is_creature_permanent(deck, p)
        })
        .collect();
    // One creature pays one mana of its printed colors; one with the
    // needed color covers that color's pip one-for-one.
    let mut remaining_pips = eff.pips;
    for creature in &creatures {
        if remaining_pips.iter().all(|p| *p == 0) {
            break;
        }
        let colors = super::super::game::card_of(deck, creature).colors;
        if let Some(i) = (0..5).find(|i| remaining_pips[*i] > 0 && colors[*i]) {
            remaining_pips[i] -= 1;
        }
    }
    ConvokePayment {
        total: (creatures.len() as u32).min(eff.total()),
        can_cover_pips: remaining_pips.iter().all(|p| *p == 0),
    }
}

/// Delve payment (CR 702.66): every card in the graveyard can be exiled
/// to pay {1} generic mana.
fn delve_payment(
    st: &super::super::game::GameState,
    card: &super::super::model::SimCard,
    eff: &Cost,
) -> u32 {
    if !card.keyword_abilities.delve {
        return 0;
    }
    (st.graveyard.len() as u32).min(eff.generic)
}
