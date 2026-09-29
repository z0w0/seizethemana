//! Mana payment for the goldfish game loop: pool building, pip matching,
//! and cost payment split from game.rs to keep files small.

use super::game::{ManaPool, Permanent, card_of};
use super::model::{ManaYield, Role, Scale, SimDeck, SpendRestriction};

/// Distinct colors among permanents on the battlefield (printed card
/// colors). Powers `ColorsPresent` scaling (Faeburrow Elder).
fn colors_present(deck: &SimDeck, battlefield: &[Permanent]) -> u32 {
    let mut found = [false; 5];
    for p in battlefield {
        for (i, has) in card_of(deck, p).colors.iter().enumerate() {
            if *has {
                found[i] = true;
            }
        }
    }
    found.iter().filter(|c| **c).count() as u32
}

/// Add one tap's yield to the pool without battlefield scaling.
pub(super) fn add_yield(y: &ManaYield, pool: &mut ManaPool) {
    add_yield_turns_unscaled(y, pool, u32::MAX);
}

/// Turn-aware variant without battlefield scaling.
pub(super) fn add_yield_turns_unscaled(y: &ManaYield, pool: &mut ManaPool, turn: u32) {
    // No battlefield context: every scale resolves to zero. The hot
    // tap-budget path builds no
    // throwaway deck for this.
    if y.opponent_any {
        if turn >= 2 {
            pool.colorless += y.any_pips;
        }
        return;
    }
    let any_pips = y.any_pips;
    if let Some(restriction) = y.restriction {
        // Restricted buckets pay their own cast class; each pip still
        // picks any color the source could produce.
        let pips = any_pips
            + y.fixed.iter().map(|amount| u32::from(*amount)).sum::<u32>()
            + u32::from(y.choice.iter().any(|available| *available));
        if pips > 0 {
            match restriction {
                SpendRestriction::Creature => pool.creature_only += pips,
                SpendRestriction::Legendary => pool.legendary_only += pips,
                SpendRestriction::Artifact => pool.artifact_only += pips,
                SpendRestriction::InstantSorcery => pool.instant_sorcery_only += pips,
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
        let pips = u32::from(*p);
        pool.fixed[i] += pips;
        if y.cannot_pay_generic {
            pool.fixed_no_generic[i] += pips;
        }
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

/// Turn- and battlefield-aware variant. Opponent-dependent mana is generic-only
/// from turn two, a conservative approximation without an opponent battlefield.
pub(super) fn add_yield_turns(
    deck: &SimDeck,
    y: &ManaYield,
    pool: &mut ManaPool,
    turn: u32,
    battlefield: &[Permanent],
) {
    if y.opponent_any {
        if turn >= 2 {
            pool.colorless += y.any_pips;
        }
        return;
    }
    let any_pips = y.any_pips;
    let scale_pips = match y.scaling {
        Some(Scale::ColorsPresent) => colors_present(deck, battlefield),
        Some(Scale::PerChargeCounter) => 0, // resolved at activation
        None => 0,
    };
    if let Some(restriction) = y.restriction {
        // Restricted buckets pay their own cast class; each pip still
        // picks any color the source could produce.
        let pips = any_pips
            + scale_pips
            + y.fixed.iter().map(|amount| u32::from(*amount)).sum::<u32>()
            + u32::from(y.choice.iter().any(|available| *available));
        if pips > 0 {
            match restriction {
                SpendRestriction::Creature => pool.creature_only += pips,
                SpendRestriction::Legendary => pool.legendary_only += pips,
                SpendRestriction::Artifact => pool.artifact_only += pips,
                SpendRestriction::InstantSorcery => pool.instant_sorcery_only += pips,
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
        let pips = u32::from(*p);
        pool.fixed[i] += pips;
        if y.cannot_pay_generic {
            pool.fixed_no_generic[i] += pips;
        }
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
pub(super) fn payable(cost: &super::model::Cost, pool: &ManaPool) -> bool {
    // Phyrexian pips pay with 2 life each, not mana, so the pool owes
    // only the mana part.
    pool.total() >= cost.total() - phyrexian_life_charge(cost) / 2
}

/// True when every monocolor pip is covered. The flexible pool is ONE
/// shared resource: a pip is covered by a fixed pip of that color or a
/// flexible pip, and the check requires the total unmet pips across all
/// colors (counted after fixed pips of each color) to fit inside the
/// flexible pool, with colorless covering only generic/flex costs —
/// never a monocolor pip. This mirrors how `pay_cost` spends: each
/// flexible source is consumed at most once.
pub(super) fn pips_ok(cost: &super::model::Cost, pool: &ManaPool) -> bool {
    // Generic + flex pips and colorless needs draw from flexible and
    // colorless mana first (see `pay_cost`), so monocolor pips compete
    // for the flexible pool only when generic needs are small. The
    // conservative sound check: the pip shortfall across colors must
    // fit in the flexible pool, and total cost must fit in the pool.
    // Phyrexian pips ride outside this check: the best-case agent pays
    // them with 2 life each (see `phyrexian_life_charge`).
    let pip_shortfall: u32 = cost
        .pips
        .iter()
        .enumerate()
        .map(|(i, need)| u32::from(*need).saturating_sub(pool.fixed[i]))
        .sum();
    let flex = pool.flexible;
    if pip_shortfall > flex {
        return false;
    }
    // Generic and flex pips: paid from remaining flexible, then
    // colorless, then spare fixed pips (mirroring `pay_cost`'s order).
    let mut generic_left = cost.generic + cost.hybrid_pips;
    let flexible_left = flex.saturating_sub(pip_shortfall);
    let from_flex = generic_left.min(flexible_left);
    generic_left -= from_flex;
    generic_left = generic_left.saturating_sub(pool.colorless);
    // Spare fixed pips over-pay colors legally.
    let mut spare = 0u32;
    for i in 0..5 {
        spare += pool.fixed[i]
            .saturating_sub(u32::from(cost.pips[i]))
            .saturating_sub(pool.fixed_no_generic[i]);
    }
    generic_left.saturating_sub(spare) == 0
}

/// Mana a non-restricted cast can reach: the general pool only (each
/// restricted bucket is its own budget).
pub(super) fn usable_for_noncreature(pool: &ManaPool) -> u32 {
    pool.fixed
        .iter()
        .zip(pool.fixed_no_generic)
        .map(|(fixed, no_generic)| fixed.saturating_sub(no_generic))
        .sum::<u32>()
        + pool.flexible
        + pool.colorless
}

/// The spend-restriction classes of a card's cast. A card can belong to
/// several classes: an artifact creature may be paid with creature-only
/// mana (Secluded Courtyard) and artifact-only mana alike. Creature and
/// legendary classes gate on the card's type line (Secluded Courtyard,
/// Plaza of Heroes); the artifact class skips lands; the
/// instant/sorcery class reads the interaction flag's type-line gate
/// (flash creatures are not instant casts).
pub(super) fn cast_restrictions(card: &super::model::SimCard) -> Vec<SpendRestriction> {
    let mut classes = Vec::new();
    if card.flags.is_interaction {
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

/// The mana a cast of these classes may legally spend: the general pool
/// plus every restricted bucket whose class the card belongs to.
pub(super) fn usable_for_classes(pool: &ManaPool, classes: &[SpendRestriction]) -> u32 {
    let mut usable = usable_for_noncreature(pool);
    for class in classes {
        usable += bucket_of(pool, *class);
    }
    usable
}

/// The restricted bucket for a cast class.
pub(super) fn bucket_of(pool: &ManaPool, restriction: SpendRestriction) -> u32 {
    match restriction {
        SpendRestriction::Creature => pool.creature_only,
        SpendRestriction::Legendary => pool.legendary_only,
        SpendRestriction::Artifact => pool.artifact_only,
        SpendRestriction::InstantSorcery => pool.instant_sorcery_only,
    }
}

/// Pay a restricted cast's cost: every class bucket the card belongs to
/// spends first (each bucket's mana is one color of the source's
/// choice, so it can pay generic, flex pips, and monocolor pips), then
/// the general pool pays the rest. Any unused bucket stays for a later
/// cast of the same class. Phyrexian pips pay with life, not mana, so
/// the buckets only owe the mana part of the cost.
pub(super) fn pay_restricted_cost(
    cost: &super::model::Cost,
    pool: &mut ManaPool,
    classes: &[SpendRestriction],
) {
    let mana_total = cost.total() - phyrexian_life_charge(cost) / 2;
    let mut paid = 0u32;
    for class in classes {
        if paid >= mana_total {
            break;
        }
        let room = mana_total - paid;
        let spent = bucket_of(pool, *class).min(room);
        spend_bucket(pool, *class, spent);
        paid += spent;
    }
    let mut rest = *cost;
    rest.generic = rest.generic.saturating_sub(paid);
    paid = paid.saturating_sub(cost.generic);
    rest.hybrid_pips = rest.hybrid_pips.saturating_sub(paid);
    paid = paid.saturating_sub(cost.hybrid_pips);
    for pip in rest.pips.iter_mut() {
        if paid == 0 {
            break;
        }
        let spent = u32::from(*pip).min(paid);
        *pip -= spent as u8;
        paid -= spent;
    }
    pay_cost(&rest, pool);
}

/// Drain one class's bucket by `spent` mana.
fn spend_bucket(pool: &mut ManaPool, class: SpendRestriction, spent: u32) {
    match class {
        SpendRestriction::Creature => pool.creature_only -= spent,
        SpendRestriction::Legendary => pool.legendary_only -= spent,
        SpendRestriction::Artifact => pool.artifact_only -= spent,
        SpendRestriction::InstantSorcery => pool.instant_sorcery_only -= spent,
    }
}

/// The life a phyrexian-pip cost charges when the best-case agent pays
/// it with life instead of mana (2 life per `{X/P}` pip, CR 107.4f).
pub(super) fn phyrexian_life_charge(cost: &super::model::Cost) -> u32 {
    2 * cost.phyrexian.iter().map(|p| u32::from(*p)).sum::<u32>()
}

/// Pay a cost from the pool. Pips come from fixed sources first, then
/// flexible sources. Generic comes from flexible, then colorless, then
/// spare fixed pips (over-paying colors is legal).
pub(super) fn pay_cost(cost: &super::model::Cost, pool: &mut ManaPool) {
    let mut flexible = pool.flexible;
    for (i, need) in cost.pips.iter().enumerate() {
        let mut need = u32::from(*need);
        let unrestricted = pool.fixed[i].saturating_sub(pool.fixed_no_generic[i]);
        let from_fixed = need.min(unrestricted);
        pool.fixed[i] -= from_fixed;
        need -= from_fixed;
        let from_no_generic = need.min(pool.fixed_no_generic[i]);
        pool.fixed[i] -= from_no_generic;
        pool.fixed_no_generic[i] -= from_no_generic;
        need -= from_no_generic;
        let from_flex = need.min(flexible);
        consume_flexible(pool, from_flex);
        flexible -= from_flex;
    }
    let mut remaining = cost.hybrid_pips + cost.generic;
    let from_flex = remaining.min(flexible);
    consume_flexible(pool, from_flex);
    remaining -= from_flex;
    let from_colorless = pool.colorless.min(remaining);
    pool.colorless -= from_colorless;
    remaining -= from_colorless;
    for i in 0..5 {
        if remaining == 0 {
            break;
        }
        let spare = pool.fixed[i]
            .saturating_sub(pool.fixed_no_generic[i])
            .min(remaining);
        pool.fixed[i] -= spare;
        remaining -= spare;
    }
}

/// Consume `n` mana from flexible sources.
pub(super) fn consume_flexible(pool: &mut ManaPool, n: u32) {
    pool.flexible -= pool.flexible.min(n);
}

/// The effective minimum cost for a card right now. Board-discount
/// cards (improvise, affinity) cut the printed generic by one per
/// artifact on the battlefield, never past it: a mid-game board casts
/// big improvise spells a few turns early, while one early rock cannot
/// pay for everything.
pub(super) fn effective_min_cost(
    deck: &SimDeck,
    card: &super::model::SimCard,
    battlefield: &[Permanent],
) -> super::model::Cost {
    if !card.battlefield_discount {
        return card.min_cost;
    }
    // Affinity for artifacts / improvise (CR 702.41a, 702.126a): the
    // cost drops one generic per artifact, capped at the printed
    // generic. Only artifacts on the battlefield count (creatures and
    // enchantments do not); pips never change.
    let artifacts = battlefield
        .iter()
        .filter(|p| p.card.deck_idx().is_some() && card_of(deck, p).role != Role::Land)
        .filter(|p| card_of(deck, p).is_artifact)
        .count() as u32;
    let mut eff = card.cost;
    eff.generic = eff.generic.saturating_sub(artifacts);
    eff
}
