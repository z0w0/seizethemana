// Mana payment for the goldfish game loop: pool building, pip matching,
// and cost payment split from game.rs to keep files small.

use super::game::{InPlay, Pool, card_of};
use super::model::{Role, SimDeck};

pub(super) fn add_yield(y: &super::model::TapYield, pool: &mut Pool) {
    if y.creature_only {
        let reachable = y.any || y.choice.iter().any(|c| *c) || y.fixed.iter().any(|p| *p > 0);
        if reachable {
            pool.creature_only += 1;
        } else if y.colorless > 0 {
            pool.creature_only += y.colorless;
        }
        return;
    }
    if y.alternatives {
        let reachable = y.any || y.choice.iter().any(|c| *c);
        if reachable {
            pool.flexible += 1;
        } else if y.colorless > 0 {
            pool.colorless += 1;
        }
        return;
    }
    for (i, p) in y.fixed.iter().enumerate() {
        pool.fixed[i] += u32::from(*p);
    }
    let choice_colors = y.choice.iter().filter(|c| **c).count();
    if y.any || choice_colors > 1 {
        pool.flexible += 1;
    } else if choice_colors == 1
        && let Some(i) = y.choice.iter().position(|c| *c)
    {
        pool.fixed[i] += 1;
    }
    pool.colorless += y.colorless;
}

/// True when the cost is payable in total from the pool.
pub(super) fn payable(cost: &super::model::Cost, pool: &Pool) -> bool {
    pool.total() >= cost.total()
}

/// True when every monocolor pip is covered by fixed pips plus flexible
/// sources (each flexible source pays any one pip).
pub(super) fn pips_ok(cost: &super::model::Cost, pool: &Pool) -> bool {
    let flex = pool.flexible;
    cost.pips
        .iter()
        .enumerate()
        .all(|(i, need)| pool.fixed[i] + flex >= u32::from(*need))
}

/// Mana a creature spell can reach: everything except the
/// creature-only bucket (which is its own budget).
pub(super) fn usable_for_noncreature(pool: &Pool) -> u32 {
    pool.fixed.iter().sum::<u32>() + pool.flexible + pool.colorless
}

/// Mana a creature spell can reach, creature-only yield included.
pub(super) fn usable_for_creature(pool: &Pool) -> u32 {
    usable_for_noncreature(pool) + pool.creature_only
}

/// Consume from the creature-only bucket first (spend it before it
/// expires), then the general pool.
pub(super) fn pay_creature_cost(cost: &super::model::Cost, pool: &mut Pool) {
    let from_restricted = pool.creature_only.min(cost.total());
    pool.creature_only -= from_restricted;
    let mut rest = cost.clone();
    rest.generic = rest.generic.saturating_sub(from_restricted);
    // Pips stay; the restricted bucket pays generic first (its mana is
    // one color of the source's choice; the pool's fixed pips cover
    // color needs either way).
    if rest.generic == 0 && rest.total() == from_restricted.min(cost.total()) {
        return;
    }
    pay_cost(&rest, pool);
}

/// Pay a cost from the pool. Pips come from fixed sources first, then
/// flexible sources. Generic comes from flexible, then colorless, then
/// spare fixed pips (over-paying colors is legal).
pub(super) fn pay_cost(cost: &super::model::Cost, pool: &mut Pool) {
    let mut flexible = pool.flexible;
    for (i, need) in cost.pips.iter().enumerate() {
        let mut need = u32::from(*need);
        let from_fixed = need.min(pool.fixed[i]);
        pool.fixed[i] -= from_fixed;
        need -= from_fixed;
        let from_flex = need.min(flexible);
        consume_flexible(pool, from_flex);
        flexible -= from_flex;
    }
    let mut remaining = cost.flex_pips + cost.generic;
    let from_flex = remaining.min(flexible);
    consume_flexible(pool, from_flex);
    remaining -= from_flex;
    for i in 0..5 {
        if remaining == 0 {
            break;
        }
        let spare = pool.fixed[i].min(remaining);
        pool.fixed[i] -= spare;
        remaining -= spare;
    }
    pool.colorless = pool.colorless.saturating_sub(remaining);
}

/// Consume `n` mana from flexible sources.
pub(super) fn consume_flexible(pool: &mut Pool, n: u32) {
    pool.flexible -= pool.flexible.min(n);
}

/// The effective minimum cost for a card right now. Board-discount cards
/// (improvise, affinity) cut generic by 2 at parse time plus one more per
/// 4 artifacts on the battlefield, never past the printed generic: a
/// mid-game board casts big improvise spells a few turns early, while one
/// early rock cannot pay for everything.
pub(super) fn effective_min_cost(
    deck: &SimDeck,
    card: &super::model::SimCard,
    battlefield: &[InPlay],
) -> super::model::Cost {
    if !card.board_discount {
        return card.min_cost.clone();
    }
    let artifacts = battlefield
        .iter()
        .filter(|p| !p.is_commander && card_of(deck, p).role != Role::Land)
        .count();
    let headroom = card.cost.generic.saturating_sub(card.min_cost.generic);
    let mut eff = card.min_cost.clone();
    eff.generic += (artifacts as u32 / 4).min(headroom);
    eff
}
