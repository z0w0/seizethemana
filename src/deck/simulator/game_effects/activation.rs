//! Activation selection and resolution for the goldfish game loop:
//! the spend-leftover pass, activation ranking, power-up cost, and the
//! sacrifice outlets.

use super::super::game::{Activation, CardRef, GameState, ManaPool, Permanent, card_of};
use super::super::game_mana::{
    pay_cost, payable, payment_plan, phyrexian_life_charge, pips_ok, usable_for_noncreature,
};
use super::super::game_run::mana::{grants_active, try_granted_mana};
#[cfg(test)]
use super::super::model::CardIdx;
use super::super::model::{
    Cost, SimAbility, SimActivation, SimActivationRestriction, SimDeck, SimEffect, SimTrigger,
};
use super::super::resolution::{begin_resolution, end_resolution};
use super::{
    amass, apply_effect_at, apply_prepared_spell_effect_at, connive, draw_one, empower_jace,
    explore, move_to_graveyard, prepare_effect_targets, resolve_proliferate, ring_tempts,
};

/// Read the payment bundle from a runtime activated ability.
fn activation_data(ability: &SimAbility) -> &SimActivation {
    ability
        .activation
        .as_ref()
        .expect("activated runtime ability has typed costs")
}

/// Use supported granted taps before choosing native activations.
/// Native abilities keep their own costs and selection rules.
/// Each committed activation consumes one slot in the existing action limit.
pub(in crate::deck::simulator) fn spend_leftover(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let mut activations_this_turn: u32 = 0;
    let mut growing_mana_cycle = false;
    let mut mana_cycle_samples = std::collections::HashMap::new();
    while st.completion.is_none() && activations_this_turn < super::super::model::MAX_LOOP_PASSES {
        if activate_next_granted_mana(deck, st, pool, turn) {
            activations_this_turn += 1;
            continue;
        }
        let Some(a) = pick_best_activation(deck, st, pool, turn) else {
            break;
        };
        let repeatable = repeatable_mana_component(deck, st, &a);
        resolve_activation(deck, st, pool, turn, &a);
        if repeatable {
            let key = (a.uid, a.ability.id, a.ability_cost.total());
            let readiness = st
                .battlefield
                .iter()
                .map(|source| (source.uid, source.tapped, source.counters))
                .collect::<Vec<_>>();
            if let Some((previous_mana, previous_readiness)) = mana_cycle_samples.get(&key) {
                growing_mana_cycle |=
                    readiness == *previous_readiness && pool.total() > *previous_mana;
            }
            mana_cycle_samples.insert(key, (pool.total(), readiness));
        }
        activations_this_turn += 1;
    }
    let remaining = st.completion.is_none()
        && activations_this_turn == super::super::model::MAX_LOOP_PASSES
        && (pick_best_activation(deck, st, pool, turn).is_some()
            || activate_next_granted_mana(deck, &mut st.clone(), &mut pool.clone(), turn));
    if remaining && growing_mana_cycle {
        // Compare committed repeats with the same source readiness, so finite
        // taps and counter payments cannot supply the cycle's apparent growth.
        st.infinite_mana_suspected = true;
        super::super::game::milestone_for_turn(st, turn).positive_mana_loop = true;
    }
    if remaining {
        super::super::resolution::mark_incomplete(
            st,
            super::super::game::IncompleteReason::ActivationPass {
                configured_limit: super::super::model::MAX_LOOP_PASSES,
                remaining_work: 1,
            },
        );
    }
}

/// Use one granted tap before native modes can apply their unrelated costs or restrictions.
pub(in crate::deck::simulator) fn activate_next_granted_mana(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let grants = grants_active(deck, st);
    if !grants.lands && !grants.creatures {
        return false;
    }
    for position in 0..st.battlefield.len() {
        let source = st.battlefield[position].clone();
        if try_granted_mana(deck, st, &source, pool, turn) == Some(true) {
            return true;
        }
    }
    false
}

/// Return true for a mana action that can repeat without consuming a finite
/// life, counter, creature, or once-per-turn resource.
fn repeatable_mana_component(deck: &SimDeck, st: &GameState, activation: &Activation) -> bool {
    let ability = &activation.ability;
    let costs = activation_data(ability);
    if costs.has_restriction(SimActivationRestriction::OncePerTurn)
        || costs.has_loyalty_cost()
        || costs.untaps_source()
        || costs.charge_counter_payment() > 0
        || costs.creature_sacrifices() > 0
        || costs.sacrifices_source()
        || costs.life_payment() > 0
    {
        return false;
    }
    ability.effect_sequence().iter().any(|effect| match effect {
        SimEffect::Mana(_) => costs.mana_cost().total() == 0 && !costs.taps_source(),
        SimEffect::UntapSelf => st
            .battlefield
            .get(activation.pos)
            .is_some_and(|permanent| card_of(deck, permanent).tap.is_some()),
        _ => false,
    })
}

/// Find the cheapest usable activation on the battlefield. A candidate
/// must be untapped (loyalty and banked shapes included), affordable,
/// and ungated by `fired`; free untapped candidates win cost ties so the
/// pass can keep looping.
pub(in crate::deck::simulator) fn pick_best_activation(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    turn: u32,
) -> Option<Activation> {
    let mut best: Option<Activation> = None;
    for (bi, perm) in st.battlefield.iter().enumerate() {
        if perm.face_down {
            continue;
        }
        // Loyalty activations gate on loyalty, not mana, so they pass
        // the tap/counter filter too (planeswalker minus and plus
        // abilities fire here). Station striations are gated by the
        // permanent's charge counters (CR 721.2a).
        let unlocked = super::super::game::permanent_abilities(deck, perm).filter(|a| {
            let Some(costs) = a.activation.as_ref() else {
                return false;
            };
            let mana_cost = costs.mana_cost();
            let free_mana = mana_cost.total() == 0
                && a.effect_sequence()
                    .iter()
                    .any(|effect| matches!(effect, SimEffect::Mana(_)));
            // A zero amount represents X for supported charge-counter effects.
            let x_sink = a
                .effect_sequence()
                .iter()
                .any(|effect| matches!(effect, SimEffect::Counters(0)));
            a.kind.is_activated()
                    && (costs.taps_source()
                        || costs.charge_counter_payment() > 0
                         || costs.has_loyalty_cost()
                        || costs.life_payment() > 0
                         || costs.creature_sacrifices() > 0
                         || costs.sacrifices_source()
                         || costs.has_restriction(SimActivationRestriction::OncePerGame)
                         || free_mana
                        || x_sink
                        || a.effect_sequence()
                            .iter()
                            .any(|effect| matches!(effect, SimEffect::Search(_) | SimEffect::SearchThenBeholdUntap { .. } | SimEffect::MultiSearch(_)))
                        || a.effect_sequence()
                            .iter()
                            .any(|effect| matches!(effect, SimEffect::UntapSelf)))
                    && (!costs.taps_source() || !perm.tapped)
                    && (!(costs.taps_source() || costs.untaps_source())
                        || !(super::super::game::is_creature_permanent(deck, perm)
                            && (perm.summoning_sick || perm.entered_turn >= turn as usize)
                            && !super::super::game::effective_keywords(deck, st, perm)
                                .contains(super::super::model::Keyword::Haste)))
                    // {Q} costs need the source tapped (CR 107.6).
                    && (!costs.untaps_source() || perm.tapped)
        });
        for ability in unlocked {
            if ability
                .condition
                .is_some_and(|condition| !super::super::game::condition_met(deck, st, &condition))
            {
                continue;
            }
            let costs = activation_data(ability);
            if ability.effect_sequence().iter().any(|effect| {
                matches!(effect, SimEffect::Mana(yield_) | SimEffect::ManaPerCounter(yield_)
                    if !super::super::game_run::mana::mana_yield_eligible(deck, st, perm, yield_, costs.taps_source()))
            }) {
                continue;
            }
            if costs.life_payment() > 0 && st.life < costs.life_payment() as i32 {
                continue;
            }
            // Once-per-game abilities (Exhaust, Power-up): the permanent
            // records the first firing and never activates again
            // (CR 702.177a, 702.193a).
            let ability_key = (perm.uid, ability.id);
            if costs.has_restriction(SimActivationRestriction::OncePerGame)
                && st.activated_once.contains(&ability_key)
            {
                continue;
            }
            if costs.has_restriction(SimActivationRestriction::OncePerTurn)
                && st.activated_this_turn.contains(&ability_key)
            {
                continue;
            }
            if costs.has_loyalty_cost() && perm.fired {
                continue;
            }

            let mut eligible_creatures: Vec<u32> = st
                .battlefield
                .iter()
                .filter(|candidate| {
                    candidate.uid != perm.uid
                        && super::super::game::is_creature_permanent(deck, candidate)
                })
                .map(|candidate| candidate.uid)
                .collect();
            if !costs.sacrifices_source()
                && !costs.creature_sacrifice_excludes_source()
                && costs.creature_sacrifices() as usize > eligible_creatures.len()
                && super::super::game::is_creature_permanent(deck, perm)
            {
                eligible_creatures.push(perm.uid);
            }
            if eligible_creatures.len() < costs.creature_sacrifices() as usize {
                continue;
            }
            // Power-up (CR 702.193b): while the permanent entered this
            // turn, its mana cost reduces the activation cost.
            let base_cost = power_up_cost(deck, perm, ability, turn);
            let Some((paid_x, ability_cost)) = choose_activation_x_cost(base_cost, pool) else {
                continue;
            };
            if !activation_usable(
                perm,
                ability,
                &ability_cost,
                pool,
                st.life,
                st.player_counters.energy,
            ) {
                continue;
            }
            if prepare_effect_targets(deck, st, ability.effect_sequence()).is_none() {
                continue;
            }
            let candidate_cost = ability_cost.total();
            // Loyalty abilities rank by effect category first: a draw
            // outranks tokens, life_loss, search, and surveil, so a token
            // walker's [-3]: Draw is taken before a [-1]: Surveil burns
            // the loyalty (Empower Jace, CR 701.71). Within a category
            // the cheaper ability wins, which keeps a +1 loyalty-gain
            // ability building toward an ultimate.
            let candidate_value = ability_category(ability);
            let best_value = best
                .as_ref()
                .map(|b| ability_category(&b.ability))
                .unwrap_or(0);
            if best.as_ref().is_none_or(|b| {
                // Free untapped candidates win cost ties: they leave the
                // source untapped so the pass can keep looping ({0}: Add
                // mode beats the {T} mode for loop engines).
                candidate_value > best_value
                    || (candidate_value == best_value
                        && (candidate_cost < b.ability_cost.total()
                            || (candidate_cost == b.ability_cost.total()
                                && activation_data(&b.ability).taps_source()
                                && !costs.taps_source())))
            }) {
                let draws = activation_draw_count(ability);
                best = Some(Activation {
                    pos: bi,
                    uid: perm.uid,
                    ability: ability.clone(),
                    ability_cost,
                    paid_x,
                    draws,
                    sacrifice_uid: eligible_creatures.first().copied(),
                    target_uid: eligible_creatures
                        .get(costs.creature_sacrifices() as usize)
                        .copied(),
                });
            }
        }
    }
    best
}

/// Choose the largest X payable by mana allowed for a nonspell activation.
fn choose_activation_x_cost(cost: Cost, pool: &ManaPool) -> Option<(u32, Cost)> {
    if cost.x_symbols == 0 {
        return Some((0, cost));
    }
    payment_plan(&cost.with_x(0)?, pool, &[])?;
    let mut low = 0;
    let mut high = usable_for_noncreature(pool) / u32::from(cost.x_symbols);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if cost
            .with_x(middle)
            .is_some_and(|cost| payment_plan(&cost, pool, &[]).is_some())
        {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    Some((low, cost.with_x(low)?))
}

/// True when an unlocked activation can fire right now: the `fired`
/// gate, affordability, the loyalty/counter gates, and life covering
/// any phyrexian pips (the resolution would charge them).
fn activation_usable(
    perm: &Permanent,
    ability: &SimAbility,
    cost: &Cost,
    pool: &ManaPool,
    life: i32,
    energy: u32,
) -> bool {
    let costs = activation_data(ability);
    let usable = ability.effect_sequence().iter().any(|effect| {
        if matches!(effect, SimEffect::UntapSelf) {
            return perm.tapped;
        }
        matches!(
            effect,
            SimEffect::Draw(_)
                | SimEffect::DrawAndMinusCounter
                | SimEffect::GainLife(_)
                | SimEffect::Search(_)
                | SimEffect::SearchThenBeholdUntap { .. }
                | SimEffect::MultiSearch(_)
                | SimEffect::ReturnFromGraveyard { .. }
                | SimEffect::Mana(_)
                | SimEffect::Counters(_)
                | SimEffect::DrawThenDiscard(_)
                | SimEffect::DrawDiscard { .. }
                | SimEffect::LoseLife { .. }
                | SimEffect::Damage { .. }
                | SimEffect::Tokens(_)
                | SimEffect::Treasures(_)
                | SimEffect::Proliferate
                | SimEffect::AddBurdenCounter
        )
    }) || costs.creature_sacrifices() > 0
        || costs.sacrifices_source();
    // Banked activations (Pentad Prism) consume a charge counter per
    // fire; gate on the host's counters. Free untapped activations
    // ({0}: Add ...) bypass `fired` — they repeat in real Magic and
    // feed the loop census — unless the card bounds them ("Activate
    // only once each turn").
    let banked = costs.charge_counter_payment() > 0
        && perm.counters.charge >= costs.charge_counter_payment();

    let loyalty_change = costs.loyalty_change();
    let loyalty_affordable =
        loyalty_change >= 0 || perm.counters.loyalty >= loyalty_change.unsigned_abs();
    let energy_affordable = energy >= costs.energy_payment();
    let life_affordable = life >= (costs.life_payment() + phyrexian_life_charge(cost)) as i32;
    let mana_affordable = payable(cost, pool) && pips_ok(cost, pool);
    usable
        && loyalty_affordable
        && energy_affordable
        && life_affordable
        && (costs.charge_counter_payment() == 0 || banked)
        && mana_affordable
}

/// The effect category of an activation: a draw outranks tokens, life_loss,
/// search, and surveil. Loyalty abilities select by category so a
/// walker's draw is not starved by a cheaper surveil, while a +1
/// loyalty-gain ability still wins its category and builds loyalty.
fn ability_category(ability: &SimAbility) -> u32 {
    ability
        .effect_sequence()
        .iter()
        .map(|effect| match effect {
            SimEffect::Draw(_)
            | SimEffect::DrawAndMinusCounter
            | SimEffect::DrawThenDiscard(_)
            | SimEffect::DrawDiscard { .. } => 4,
            SimEffect::Tokens(_) | SimEffect::Treasures(_) => 3,
            SimEffect::LoseLife { .. } | SimEffect::Damage { .. } => 2,
            SimEffect::Search(_)
            | SimEffect::SearchThenBeholdUntap { .. }
            | SimEffect::MultiSearch(_)
            | SimEffect::GainLife(_) => 1,
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

/// Count cards drawn across every supported effect in an activation.
fn activation_draw_count(ability: &SimAbility) -> u32 {
    ability
        .effect_sequence()
        .iter()
        .map(|effect| match effect {
            SimEffect::Draw(amount) | SimEffect::DrawThenDiscard(amount) => *amount,
            SimEffect::DrawDiscard { draw, .. } => *draw,
            SimEffect::DrawAndMinusCounter => 1,
            _ => 0,
        })
        .sum()
}

/// Power-up cost (CR 702.193b): generic in the permanent's mana cost
/// reduces generic; colored/colorless reduce mana of the same type, and
/// any excess reduces generic. Applied only while the permanent entered
/// this turn.
fn power_up_cost(deck: &SimDeck, perm: &Permanent, ability: &SimAbility, turn: u32) -> Cost {
    let costs = activation_data(ability);
    let mana_cost = costs.mana_cost();
    if !costs.has_restriction(SimActivationRestriction::PowerUp) {
        return mana_cost;
    }
    // The discount applies only while the permanent entered this turn
    // (CR 702.193b); entry turn and the live turn share one counter.
    if perm.entered_turn != turn as usize {
        return mana_cost;
    }
    let permanent_cost = card_of(deck, perm).cost;
    let mut cost = mana_cost;
    // Generic reduction first (CR 702.193b).
    let generic_cut = permanent_cost.generic.min(cost.generic);
    cost.generic -= generic_cut;
    // Colored symbols reduce matching pips, then generic.
    let mut leftover = permanent_cost.generic - generic_cut;
    for i in 0..5 {
        let need = u32::from(permanent_cost.pips[i]);
        let cut = need.min(u32::from(cost.pips[i]));
        cost.pips[i] -= cut as u8;
        leftover += need - cut;
    }
    cost.generic = cost.generic.saturating_sub(leftover);
    cost
}

/// Apply one chosen activation: pay the cost (mana, loyalty, or a
/// charge counter), tap the source, resolve the effect, and run the
/// sacrifice-outlet death triggers.
pub(in crate::deck::simulator) fn resolve_activation(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
    a: &Activation,
) {
    let ability = &a.ability;
    if ability
        .condition
        .is_some_and(|condition| !super::super::game::condition_met(deck, st, &condition))
    {
        return;
    }
    let Some(source_pos) = st.battlefield.iter().position(|perm| perm.uid == a.uid) else {
        return;
    };
    let source = &st.battlefield[source_pos];
    let costs = activation_data(ability);
    if source_pos != a.pos
        || costs.taps_source() && source.tapped
        || costs.untaps_source() && !source.tapped
        || (costs.taps_source() || costs.untaps_source())
            && super::super::game::is_creature_permanent(deck, source)
            && (source.summoning_sick || source.entered_turn >= turn as usize)
            && !super::super::game::effective_keywords(deck, st, source)
                .contains(super::super::model::Keyword::Haste)
        || costs.has_restriction(SimActivationRestriction::OncePerGame)
            && st.activated_once.contains(&(a.uid, ability.id))
        || costs.has_restriction(SimActivationRestriction::OncePerTurn)
            && st.activated_this_turn.contains(&(a.uid, ability.id))
        || costs.has_loyalty_cost() && source.fired
        || ability.effect_sequence().iter().any(|effect| {
            matches!(effect, SimEffect::Mana(yield_) | SimEffect::ManaPerCounter(yield_)
                if !super::super::game_run::mana::mana_yield_eligible(deck, st, source, yield_, costs.taps_source()))
        })
        || !activation_usable(
            source,
            ability,
            &a.ability_cost,
            pool,
            st.life,
            st.player_counters.energy,
        )
    {
        return;
    }
    let Some(mut targets) = prepare_effect_targets(deck, st, ability.effect_sequence()) else {
        return;
    };
    let source = source.clone();
    let Some(sacrifice_uids) = prepare_activation_sacrifices(deck, st, a, costs) else {
        return;
    };
    let mut paid_pool = pool.clone();
    if !pay_cost(&a.ability_cost, &mut paid_pool) {
        return;
    }
    begin_resolution(st);
    *pool = paid_pool;
    let source_card = source.card.deck_idx();
    // Power-up / exhaust: the activation is once per game, so record it
    // regardless of which payment branch below runs (CR 702.177a).
    let ability_key = (a.uid, ability.id);
    if costs.has_restriction(SimActivationRestriction::OncePerGame) {
        st.activated_once.insert(ability_key);
    }
    if costs.has_restriction(SimActivationRestriction::OncePerTurn) {
        st.activated_this_turn.insert(ability_key);
    }
    let phyrexian_life = phyrexian_life_charge(&a.ability_cost);
    st.life -= (phyrexian_life + costs.life_payment()) as i32;
    st.life_paid += phyrexian_life + costs.life_payment();
    st.player_counters.energy -= costs.energy_payment();
    let perm = &mut st.battlefield[a.pos];
    perm.counters.loyalty = if costs.loyalty_change() < 0 {
        perm.counters
            .loyalty
            .saturating_sub(costs.loyalty_change().unsigned_abs())
    } else {
        perm.counters.loyalty + costs.loyalty_change() as u32
    };
    perm.counters.charge = perm
        .counters
        .charge
        .saturating_sub(costs.charge_counter_payment());
    if costs.has_loyalty_cost() || costs.charge_counter_payment() > 0 {
        perm.fired = true;
    }
    if costs.taps_source() {
        perm.tapped = true;
    }
    if costs.untaps_source() {
        perm.tapped = false;
    }
    if costs.creature_sacrifices() > 0 || costs.sacrifices_source() {
        resolve_sacrifice_batch(deck, st, turn, &sacrifice_uids);
    }
    if ability.kind != super::super::model::SimAbilityKind::ManaActivated {
        end_resolution(deck, st, Some(pool));
        if st.completion.is_some() {
            return;
        }
        begin_resolution(st);
    }
    let seen_before = st.seen;
    if targets.any_legal_target(st) {
        for effect in ability.effect_sequence() {
            if st.completion.is_some() {
                break;
            }
            if matches!(effect, SimEffect::ReturnFromGraveyard { .. }) {
                apply_prepared_spell_effect_at(
                    deck,
                    effect,
                    st,
                    turn,
                    false,
                    source_card,
                    &mut targets,
                );
            } else {
                resolve_activation_effect(deck, st, pool, turn, a, &source, effect);
            }
        }
    }
    if costs.life_payment() > 0 {
        let draws = st.seen.saturating_sub(seen_before).min(a.draws);
        st.life_funded_draws += draws;
        super::super::game::milestone_for_turn(st, turn).life_funded_draws += draws;
    }
    if ability.kind == super::super::model::SimAbilityKind::ManaActivated
        && costs.taps_source()
        && source_card.is_some_and(|index| !deck[index].is_land)
    {
        super::super::game_run::fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
    end_resolution(deck, st, Some(pool));
    st.opponents.settle_losses();
}

/// Prepare distinct battlefield identities for an activation's sacrifice costs.
fn prepare_activation_sacrifices(
    deck: &SimDeck,
    st: &GameState,
    activation: &Activation,
    costs: &SimActivation,
) -> Option<Vec<u32>> {
    let source_exists = st
        .battlefield
        .iter()
        .any(|permanent| permanent.uid == activation.uid);
    if costs.sacrifices_source() && !source_exists {
        return None;
    }
    let count = costs.creature_sacrifices() as usize;
    let exclude_source = costs.sacrifices_source() || costs.creature_sacrifice_excludes_source();
    let mut victims = Vec::with_capacity(count + usize::from(costs.sacrifices_source()));
    if count > 0 {
        if let Some(uid) = activation.sacrifice_uid
            && st.battlefield.iter().any(|permanent| {
                permanent.uid == uid
                    && super::super::game::is_creature_permanent(deck, permanent)
                    && (!exclude_source || uid != activation.uid)
            })
        {
            victims.push(uid);
        }
        for permanent in &st.battlefield {
            if victims.len() == count {
                break;
            }
            if super::super::game::is_creature_permanent(deck, permanent)
                && (!exclude_source || permanent.uid != activation.uid)
                && !victims.contains(&permanent.uid)
            {
                victims.push(permanent.uid);
            }
        }
        if victims.len() != count {
            return None;
        }
    }
    if costs.sacrifices_source() {
        victims.push(activation.uid);
    }
    Some(victims)
}

/// Resolve one activation effect while preserving its position in the
/// ability's Oracle resolution order.
fn resolve_activation_effect(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
    activation: &Activation,
    source: &Permanent,
    effect: &SimEffect,
) {
    match effect {
        SimEffect::Draw(count) => {
            for _ in 0..*count {
                draw_one(deck, st, turn);
            }
        }
        SimEffect::DrawAndMinusCounter => {
            draw_one(deck, st, turn);
            if let Some(target_uid) = activation.target_uid {
                apply_minus_counter(deck, st, turn, target_uid);
            }
        }
        SimEffect::GainLife(amount) => {
            st.life += *amount as i32;
            st.life_gained += amount;
        }
        SimEffect::LoseLife { amount, scope } => {
            st.opponent_life_lost += st.opponents.lose_life(*amount, *scope);
            if scope.charges_player() {
                st.life -= *amount as i32;
            }
        }
        SimEffect::Damage { amount, scope } => {
            st.damage_dealt_this_turn += st.opponents.lose_life(*amount, *scope);
            if scope.charges_player() {
                st.life -= *amount as i32;
            }
        }
        SimEffect::Mana(yield_) => {
            if activation.ability.kind == super::super::model::SimAbilityKind::ManaActivated
                && activation_data(&activation.ability).taps_source()
            {
                super::super::game_run::mana::add_mode_yield(deck, st, source, yield_, pool, turn);
            } else {
                super::super::game_run::mana::add_activation_yield(deck, st, yield_, pool, turn);
            }
        }
        SimEffect::Counters(amount) => {
            if let Some(source) = st
                .battlefield
                .iter_mut()
                .find(|permanent| permanent.uid == activation.uid)
            {
                source.counters.charge += if *amount == 0 {
                    activation.paid_x
                } else {
                    *amount
                };
            }
        }
        SimEffect::UntapSelf => {
            if let Some(permanent) = st
                .battlefield
                .iter_mut()
                .find(|permanent| permanent.uid == activation.uid)
            {
                permanent.tapped = false;
            }
        }
        SimEffect::Energy(amount) => st.player_counters.energy += amount,
        SimEffect::Proliferate => resolve_proliferate(st),
        SimEffect::Treasures(count) => st.treasure_bank += count,
        SimEffect::Amass(amount) => amass(deck, st, turn, *amount),
        SimEffect::Explore => explore(deck, st, turn),
        SimEffect::Connive(count) => connive(deck, st, turn, *count),
        SimEffect::RingTempts => ring_tempts(deck, st, turn),
        SimEffect::EmpowerJace(amount) => empower_jace(deck, st, turn, *amount),
        _ => apply_effect_at(deck, effect, st, turn, false, source.card.deck_idx()),
    }
}

/// Sacrifice one selected creature and fire the surviving board's death triggers.
pub(in crate::deck::simulator) fn resolve_sacrifice_uid(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    uid: u32,
) {
    let victim = st.battlefield.iter().position(|p| p.uid == uid);
    resolve_sacrifice_at(deck, st, turn, victim);
}

/// Resolve a last-known source's triggers for one simultaneous death event.
fn fire_source_death_triggers(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    victim: &Permanent,
    source: &Permanent,
) {
    let event_card = if victim.face_down {
        super::super::game::token_creature_card()
    } else {
        card_of(deck, victim)
    };
    let triggers: Vec<_> = super::super::game::permanent_abilities(deck, source)
        .filter(|ability| ability.kind.is_triggered() && ability.trigger == SimTrigger::Dies)
        .filter(|ability| {
            super::super::game::trigger_subject_matches(
                ability.event_subject,
                source.uid,
                victim.uid,
            )
        })
        .filter(|ability| {
            super::super::game::event_filter_matches(
                &ability.event_filter,
                event_card,
                matches!(victim.card, CardRef::Token(_)),
            )
        })
        .filter_map(|ability| {
            super::super::resolution::prepare_trigger(deck, st, source, ability, turn)
        })
        .collect();
    for trigger in triggers {
        super::super::resolution::enqueue_trigger(st, trigger);
    }
}

/// Sacrifice one simultaneous batch and resolve triggers from its prior board.
pub(in crate::deck::simulator) fn resolve_sacrifice_batch(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    uids: &[u32],
) {
    if uids.is_empty() {
        return;
    }
    begin_resolution(st);
    let sources: Vec<_> = st
        .battlefield
        .iter()
        .filter(|source| {
            super::super::game::permanent_abilities(deck, source)
                .any(|ability| ability.kind.is_triggered() && ability.trigger == SimTrigger::Dies)
        })
        .cloned()
        .collect();
    let victims: Vec<_> = st
        .battlefield
        .iter()
        .filter(|p| uids.contains(&p.uid))
        .cloned()
        .collect();
    st.battlefield.retain(|p| !uids.contains(&p.uid));
    for victim in &victims {
        if let Some(index) = victim.card.deck_idx() {
            move_to_graveyard(deck, st, index, turn, super::CardZone::Battlefield);
        }
    }
    for victim in &victims {
        for source in &sources {
            fire_source_death_triggers(deck, st, turn, victim, source);
        }
    }
    for victim in &victims {
        super::super::resolution::queue_undying(deck, st, victim, turn);
    }
    end_resolution(deck, st, None);
}

/// Resolve the chosen creature's -1/-1 counter and any resulting death.
pub(in crate::deck::simulator) fn apply_minus_counter(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    target_uid: u32,
) {
    let Some(position) = st
        .battlefield
        .iter()
        .position(|perm| perm.uid == target_uid)
    else {
        return;
    };
    let target = &mut st.battlefield[position];
    // CR 122.3: +1/+1 and -1/-1 counters annihilate in pairs as a
    // state-based action, so an existing +1/+1 counter absorbs the new
    // -1/-1 counter.
    if target.counters.plus1 > 0 {
        target.counters.plus1 -= 1;
        return;
    }
    target.counters.minus1 += 1;
    let toughness = card_of(deck, target)
        .printed_toughness
        .unwrap_or(super::super::game::TOKEN_CREATURE_TOUGHNESS);
    if target.counters.minus1 >= toughness {
        resolve_sacrifice_uid(deck, st, turn, target_uid);
    }
}

/// Enforce the legend rule (CR 704.5j): the player may control only one
/// legendary permanent of each name. The goldfish keeps the copy that
/// entered first (it may hold counters or equipment) and puts later
/// duplicates into the graveyard, firing their death triggers. Tokens are
/// skipped: the sim does not create legendary-copy tokens.
pub(in crate::deck::simulator) fn enforce_legend_rule(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
) {
    let mut seen: Vec<String> = Vec::new();
    let mut duplicates: Vec<usize> = Vec::new();
    for (position, perm) in st.battlefield.iter().enumerate() {
        if perm.card.deck_idx().is_none() && !matches!(perm.card, CardRef::Commander { .. }) {
            continue;
        }
        let card = card_of(deck, perm);
        if !card.is_legendary {
            continue;
        }
        if seen.iter().any(|name| name == &card.name) {
            duplicates.push(position);
        } else {
            seen.push(card.name.clone());
        }
    }
    // Remove from the back so earlier positions stay valid.
    for position in duplicates.into_iter().rev() {
        resolve_sacrifice_at(deck, st, turn, Some(position));
    }
}

/// Remove the selected battlefield position and resolve death triggers.
pub(in crate::deck::simulator) fn resolve_sacrifice_at(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    victim: Option<usize>,
) {
    let Some(v) = victim else {
        return;
    };
    let uid = st.battlefield[v].uid;
    resolve_sacrifice_batch(deck, st, turn, &[uid]);
}

#[cfg(test)]
#[path = "tests/activation_tests.rs"]
mod activation_tests;

#[cfg(test)]
#[path = "tests/sacrifice_tests.rs"]
mod sacrifice_tests;
