// Mana payment for the goldfish game loop: pool building, pip matching,
// and cost payment split from game.rs to keep files small.

use super::game::{InPlay, Pool, card_of};
use super::model::{Restriction, Role, Scale, SimDeck, TapYield};

/// Distinct colors among permanents on the battlefield (printed card
/// colors). Powers `ColorsPresent` scaling (Faeburrow Elder).
fn colors_present(deck: &SimDeck, board: &[InPlay]) -> u32 {
    let mut found = [false; 5];
    for p in board {
        for (i, has) in card_of(deck, p).colors.iter().enumerate() {
            if *has {
                found[i] = true;
            }
        }
    }
    found.iter().filter(|c| **c).count() as u32
}

/// Add one tap's yield to the pool (no turn/board context: best-case
/// turn, empty board for scaling).
pub(super) fn add_yield(y: &TapYield, pool: &mut Pool) {
    add_yield_turns_empty_board(y, pool, u32::MAX);
}

/// Turn-aware variant with an empty board (no deck context: scaling
/// resolves to nothing).
pub(super) fn add_yield_turns_empty_board(y: &TapYield, pool: &mut Pool, turn: u32) {
    let any_pips = if y.opponent_any {
        if turn < 2 { 0 } else { y.any_pips }
    } else {
        y.any_pips
    };
    if let Some(restriction) = y.restriction {
        let pips =
            any_pips + u32::from(y.choice.iter().any(|c| *c) || y.fixed.iter().any(|p| *p > 0));
        if pips > 0 {
            match restriction {
                Restriction::Creature => pool.creature_only += pips,
                _ => pool.flexible += pips,
            }
        } else {
            pool.colorless += y.colorless;
        }
        return;
    }
    if y.alternatives {
        let reachable = any_pips > 0 || y.choice.iter().any(|c| *c);
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
    if any_pips > 0 || choice_colors > 1 {
        pool.flexible += any_pips.max(1);
    } else if choice_colors == 1
        && let Some(i) = y.choice.iter().position(|c| *c)
    {
        pool.fixed[i] += 1;
    }
    pool.colorless += y.colorless;
}

/// Turn- and board-aware variant. `opponent` any-color sources (Fellwar
/// Stone) read as best-case from turn 2 (an opponent has lands by then)
/// and nothing on turn 1; the sim is best-case everywhere else. Scaling
/// yields resolve against the battlefield.
pub(super) fn add_yield_turns(
    deck: &SimDeck,
    y: &TapYield,
    pool: &mut Pool,
    turn: u32,
    board: &[InPlay],
) {
    let any_pips = if y.opponent_any {
        if turn < 2 { 0 } else { y.any_pips }
    } else {
        y.any_pips
    };
    let scale_pips = match y.scaling {
        Some(Scale::ColorsPresent) => colors_present(deck, board),
        Some(Scale::PerChargeCounter) => 0, // resolved at activation
        None => 0,
    };
    if let Some(restriction) = y.restriction {
        // Restricted buckets pay their own cast class; each pip still
        // picks any color the source could produce. The sim honors only
        // the creature split in payment; other restrictions land in the
        // flexible pool (their casts go through the normal pool).
        let pips = any_pips
            + scale_pips
            + u32::from(y.choice.iter().any(|c| *c) || y.fixed.iter().any(|p| *p > 0));
        if pips > 0 {
            match restriction {
                Restriction::Creature => pool.creature_only += pips,
                _ => pool.flexible += pips,
            }
        } else {
            pool.colorless += y.colorless;
        }
        return;
    }
    if y.alternatives {
        let reachable = any_pips > 0 || y.choice.iter().any(|c| *c);
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
    if any_pips > 0 || choice_colors > 1 {
        pool.flexible += any_pips.max(1);
    } else if choice_colors == 1
        && let Some(i) = y.choice.iter().position(|c| *c)
    {
        pool.fixed[i] += 1;
    }
    pool.flexible += scale_pips;
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
