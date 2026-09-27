//! Choose and resolve spells that the current mana pool can pay for.

use super::super::game::{GameState, Pool, card_of};
use super::super::game_mana::{
    cast_restriction, effective_min_cost, payable, pips_ok, usable_for_noncreature,
};
use super::super::model::{CardIdx, Cost, Restriction, SimDeck};
use super::{resolve_cast, select_alternative_cost_cards};

/// One affordability sweep over the remaining queue. Casts deduct from
/// the pool and add mana (rituals), so a card skipped as unaffordable
/// here can pay off in a later sweep; each cast removes its index from
/// the queue. Returns true when at least one card was cast.
#[allow(clippy::too_many_arguments)]
pub(super) fn cast_pass(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: usize,
    queue: &mut Vec<CardIdx>,
    cast_ids: &mut Vec<CardIdx>,
    cast_ets: &mut Vec<(u32, CardIdx)>,
    spent_total: &mut u32,
    engines: &mut Vec<(u32, u32)>,
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
                engines,
                true,
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
        if available_creatures < card.riders.additional_cost_bodies as usize
            || available_discards < card.riders.additional_cost_discards as usize
            || st.life <= card.riders.additional_cost_life as i32
        {
            idx += 1;
            continue;
        }
        let eff = effective_min_cost(deck, card, &st.battlefield);
        // Spend-restricted mana pays only its cast class: the general
        // pool plus the matching bucket counts toward the cast; other
        // restricted buckets do not. The bucket covers the card's pips
        // like any other source (each restricted pip is one mana of the
        // source's chosen color, so the bucket must cover every pip,
        // once).
        let restriction = cast_restriction(card);
        let (cost_ok, pip_ok) = if let Some(restriction) = restriction {
            // Mixed pips: the cast passes when the restricted bucket,
            // on top of the general pool, can legally cover it. The
            // bucket pays any pip (its mana is one color of the
            // source's choice), so bucket + general pips must cover the
            // pip total; the general pool alone covering pips_ok also
            // passes.
            let pip_total: u32 =
                eff.pips.iter().map(|p| u32::from(*p)).sum::<u32>() + eff.flex_pips;
            (
                pool.usable_for(restriction) >= eff.total(),
                pips_ok(&eff, pool)
                    || (bucket_of(pool, restriction) > 0
                        && bucket_of(pool, restriction) + general_coverable(&eff, pool)
                            >= pip_total),
            )
        } else {
            (
                usable_for_noncreature(pool) >= eff.total(),
                pips_ok(&eff, pool),
            )
        };
        if !cost_ok || !pip_ok {
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
            engines,
            true,
        );
        queue.remove(idx);
    }
    cast_any
}

/// The restricted bucket for a cast class.
fn bucket_of(pool: &Pool, restriction: Restriction) -> u32 {
    match restriction {
        Restriction::Creature => pool.creature_only,
        Restriction::Legendary => pool.legendary_only,
        Restriction::Artifact => pool.artifact_only,
        Restriction::InstantSorcery => pool.instant_sorcery_only,
    }
}

/// Monocolor pips the general pool (fixed + flexible) already covers,
/// outside the restricted bucket. The mixed-pip gate adds this to the
/// bucket: bucket mana pays any pip, so bucket + general-covered pips
/// must reach the pip total.
fn general_coverable(eff: &Cost, pool: &Pool) -> u32 {
    let mut flexible = pool.flexible;
    let mut covered = 0u32;
    for (i, need) in eff.pips.iter().enumerate() {
        let need = u32::from(*need);
        covered += need.min(pool.fixed[i]);
        flexible = flexible.saturating_sub(need.saturating_sub(pool.fixed[i]));
    }
    covered + flexible
}
