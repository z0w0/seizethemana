//! Activation selection and resolution for the goldfish game loop:
//! the spend-leftover pass, activation ranking, power-up cost, and the
//! sacrifice outlets. Split from `game_effects` to keep files small.

use super::super::game::{
    Activation, CardRef, GameState, ManaPool, Permanent, card_of, new_token_perm, take_uid,
};
use super::super::game_mana::{add_yield_turns, pay_cost, payable, phyrexian_life_charge, pips_ok};
use super::super::model::{
    CardIdx, Cost, SimAbility, SimActivation, SimActivationRestriction, SimDeck, SimEffect,
    SimTrigger,
};
use super::{
    amass, apply_effect_at, connive, draw_one, empower_jace, explore, move_to_graveyard,
    resolve_proliferate, ring_tempts,
};

/// Read the payment bundle from a runtime activated ability.
fn activation_data(ability: &SimAbility) -> &SimActivation {
    ability
        .activation
        .as_ref()
        .expect("activated runtime ability has typed costs")
}

/// The spend-leftover-mana pass: repeatedly fire the cheapest unlocked
/// activation (draw engines, mana engines, walkers, sacrifice outlets).
/// Each firing taps the source or spends loyalty; sacrifice outlets feed
/// death triggers from the surviving board. Loyalty activations bypass
/// the mana pool (they spend loyalty); drain activations resolve at
/// their scope's table multiplier. An untapped non-tapping activation
/// whose yield covers its own cost repeats — the pass caps it at
/// [`MAX_LOOP_PASSES`] firings and flags the census as a suspected
/// infinite engine.
pub(in crate::deck::simulator) fn spend_leftover(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let mut activations_this_turn: u32 = 0;
    let mut produced_mana = false;
    let mut repeatable_mana_activations = std::collections::HashMap::new();
    while activations_this_turn < super::super::model::MAX_LOOP_PASSES
        && let Some(a) = pick_best_activation(deck, st, pool, turn)
    {
        let mana_before = pool.total();
        if repeatable_mana_component(deck, st, &a) {
            let key = (
                a.uid,
                a.ability_cost.total(),
                activation_data(&a.ability).taps_source(),
                a.ability
                    .effect_sequence()
                    .first()
                    .map(std::mem::discriminant),
            );
            *repeatable_mana_activations.entry(key).or_insert(0u32) += 1;
        }
        resolve_activation(deck, st, pool, turn, &a);
        produced_mana |= pool.total() > mana_before;
        activations_this_turn += 1;
    }
    let repeated_mana_action = repeatable_mana_activations
        .values()
        .any(|activations| *activations > 1);
    if activations_this_turn == super::super::model::MAX_LOOP_PASSES
        && produced_mana
        && repeated_mana_action
    {
        // Require a repeated mana or self-untap activation as well as net
        // mana growth. A large batch of one-shot mana abilities is finite.
        st.infinite_mana_suspected = true;
        super::super::game::milestone_for_turn(st, turn).positive_mana_loop = true;
    }
}

/// Return true for a mana action that can repeat without consuming a finite
/// life, counter, creature, or once-per-turn resource.
fn repeatable_mana_component(deck: &SimDeck, st: &GameState, activation: &Activation) -> bool {
    let ability = &activation.ability;
    let costs = activation_data(ability);
    if costs.has_restriction(SimActivationRestriction::OncePerTurn)
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
        let card = card_of(deck, perm);
        if perm.face_down {
            continue;
        }
        // Loyalty activations gate on loyalty, not mana, so they pass
        // the tap/counter filter too (planeswalker minus and plus
        // abilities fire here). Station striations are gated by the
        // permanent's charge counters (CR 721.2a).
        let unlocked = card.unlocked_abilities(perm.counters.charge).filter(|a| {
            let Some(costs) = a.activation.as_ref() else {
                return false;
            };
            let mana_cost = costs.mana_cost();
            let loyalty = costs.loyalty_change();
            let free_mana = mana_cost.total() == 0
                && a.effect_sequence()
                    .iter()
                    .any(|effect| matches!(effect, SimEffect::Mana(_)));
            // X-sink counters ("{X}: Put X tower counters") cost
            // mana without tapping: the leftover pool converts.
            let x_sink = a
                .effect_sequence()
                .iter()
                .any(|effect| matches!(effect, SimEffect::Counters(0)));
            a.kind.is_activated()
                    && (costs.taps_source()
                        || costs.charge_counter_payment() > 0
                        || loyalty != 0
                        || costs.life_payment() > 0
                        || costs.creature_sacrifices() > 0
                        || costs.sacrifices_source()
                        || free_mana
                        || x_sink
                        || a.effect_sequence()
                            .iter()
                            .any(|effect| matches!(effect, SimEffect::Search(_)))
                        || a.effect_sequence()
                            .iter()
                            .any(|effect| matches!(effect, SimEffect::UntapSelf)))
                    && (!costs.taps_source()
                        || !perm.tapped
                            && !(super::super::game::is_creature_permanent(deck, perm)
                                && (perm.summoning_sick || perm.entered_turn >= turn as usize)
                                && !super::super::game::effective_keywords(deck, st, perm)
                                    .contains(super::super::model::Keyword::Haste)))
                    // {Q} costs need the source tapped (CR 107.6).
                    && (!costs.untaps_source() || perm.tapped)
        });
        for ability in unlocked {
            let costs = activation_data(ability);
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
            if costs.loyalty_change() != 0 && perm.fired {
                continue;
            }

            let mut eligible_creatures: Vec<u32> = st
                .battlefield
                .iter()
                .filter(|candidate| {
                    candidate.uid != perm.uid && card_of(deck, candidate).is_creature
                })
                .map(|candidate| candidate.uid)
                .collect();
            if !costs.sacrifices_source()
                && costs.creature_sacrifices() as usize > eligible_creatures.len()
                && card.is_creature
            {
                eligible_creatures.push(perm.uid);
            }
            if eligible_creatures.len() < costs.creature_sacrifices() as usize {
                continue;
            }
            // Power-up (CR 702.193b): while the permanent entered this
            // turn, its mana cost reduces the activation cost.
            let ability_cost = power_up_cost(deck, perm, ability, turn);
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
        matches!(
            effect,
            SimEffect::Draw(_)
                | SimEffect::DrawAndMinusCounter
                | SimEffect::GainLife(_)
                | SimEffect::Search(_)
                | SimEffect::Mana(_)
                | SimEffect::UntapSelf
                | SimEffect::Counters(_)
                | SimEffect::DrawThenDiscard(_)
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
            SimEffect::Draw(_) | SimEffect::DrawAndMinusCounter | SimEffect::DrawThenDiscard(_) => {
                4
            }
            SimEffect::Tokens(_) | SimEffect::Treasures(_) => 3,
            SimEffect::LoseLife { .. } | SimEffect::Damage { .. } => 2,
            SimEffect::Search(_) | SimEffect::GainLife(_) => 1,
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
    let costs = activation_data(ability);
    let source_card = st
        .battlefield
        .iter()
        .find(|perm| perm.uid == a.uid)
        .and_then(|perm| perm.card.deck_idx());
    let x_sink = ability
        .effect_sequence()
        .iter()
        .any(|effect| matches!(effect, SimEffect::Counters(0)));
    // Power-up / exhaust: the activation is once per game, so record it
    // regardless of which payment branch below runs (CR 702.177a).
    let ability_key = (a.uid, ability.id);
    if costs.has_restriction(SimActivationRestriction::OncePerGame) {
        st.activated_once.insert(ability_key);
    }
    if costs.has_restriction(SimActivationRestriction::OncePerTurn) {
        st.activated_this_turn.insert(ability_key);
    }
    pay_cost(&a.ability_cost, pool);
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
    if costs.loyalty_change() != 0 || costs.charge_counter_payment() > 0 {
        perm.fired = true;
    }
    if costs.taps_source() {
        perm.tapped = true;
    }
    if costs.untaps_source() {
        perm.tapped = false;
    }
    if costs.life_payment() > 0 {
        st.life_funded_draws += a.draws;
        super::super::game::milestone_for_turn(st, turn).life_funded_draws += a.draws;
    }
    if costs.creature_sacrifices() > 0
        && let Some(uid) = a.sacrifice_uid
    {
        resolve_sacrifice_uid(deck, st, turn, uid);
    }
    if costs.sacrifices_source() {
        resolve_sacrifice_uid(deck, st, turn, a.uid);
    }
    for effect in ability.effect_sequence() {
        resolve_activation_effect(deck, st, pool, turn, a, source_card, effect);
    }
    if ability.kind == super::super::model::SimAbilityKind::ManaActivated
        && costs.taps_source()
        && source_card.is_some_and(|index| !deck[index].is_land)
    {
        super::super::game_run::fire_tapped_for_mana_triggers(deck, st, pool, turn);
    }
    // The sacrifice loop may have removed battlefield entries and
    // shifted positions, so locate the permanent by uid (every other
    // lookup in this file does) instead of the recorded index.
    if x_sink && let Some(perm) = st.battlefield.iter_mut().find(|p| p.uid == a.uid) {
        // X-sink: the printed {X} already paid 1 through `pay_cost`
        // above. The rest of the pool is the player-chosen extra X
        // (paying everything in is legal), so the paid X = 1 +
        // leftover: the counters match the spend and the pool ends
        // at zero with nothing swallowed.
        let extra = pool.total();
        perm.counters.charge += 1 + extra;
        *pool = super::super::game::ManaPool::default();
    }
}

/// Resolve one activation effect while preserving its position in the
/// ability's Oracle resolution order.
fn resolve_activation_effect(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
    activation: &Activation,
    source_card: Option<CardIdx>,
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
            st.opponent_life_lost += amount * scope.table_multiplier(deck.format);
            if scope.charges_player() {
                st.life -= *amount as i32;
            }
        }
        SimEffect::Damage { amount, scope } => {
            st.damage_dealt_this_turn += amount * scope.table_multiplier(deck.format);
            if scope.charges_player() {
                st.life -= *amount as i32;
            }
        }
        SimEffect::Mana(yield_) => {
            add_yield_turns(deck, yield_, pool, turn, &st.battlefield);
        }
        SimEffect::Counters(amount) => {
            if let Some(source) = st
                .battlefield
                .iter_mut()
                .find(|permanent| permanent.uid == activation.uid)
            {
                source.counters.charge += amount;
            }
        }
        SimEffect::UntapSelf => {
            if let Some(permanent) = st
                .battlefield
                .iter_mut()
                .find(|permanent| permanent.uid == activation.uid)
            {
                permanent.tapped = false;
                let source = permanent.clone();
                if let Some(yield_) = card_of(deck, &source).tap.clone() {
                    super::super::game_run::add_nonland_mana(
                        deck, st, &source, &yield_, pool, turn,
                    );
                    if let Some(permanent) = st
                        .battlefield
                        .iter_mut()
                        .find(|permanent| permanent.uid == activation.uid)
                    {
                        permanent.tapped = true;
                    }
                }
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
        SimEffect::Afterlife(amount) => {
            for _ in 0..(*amount).min(8) {
                let uid = take_uid(st);
                st.battlefield.push(new_token_perm(uid, turn));
            }
        }
        _ => apply_effect_at(deck, effect, st, turn, false, source_card),
    }
}

/// Consume one creature for a sacrifice cost and fire the surviving
/// board's death triggers (aristocrats outlets).
pub(in crate::deck::simulator) fn resolve_deck_creature_sacrifice(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    source_uid: u32,
) {
    let victim = st.battlefield.iter().position(|p| {
        p.card.deck_idx().is_some()
            && p.uid != source_uid
            && card_of(deck, p).is_creature
            && !p.tapped
    });
    resolve_sacrifice_at(deck, st, turn, victim);
}

/// Sacrifice any creature, including a creature token, for a cost.
/// Tokens count as creatures, so an exchange or
/// sacrifice cost can consume them.
pub(in crate::deck::simulator) fn resolve_creature_sacrifice(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    source_uid: u32,
) {
    let victim = st
        .battlefield
        .iter()
        .position(|p| !p.tapped && p.uid != source_uid && card_of(deck, p).is_creature);
    resolve_sacrifice_at(deck, st, turn, victim);
}

/// True when an untapped creature permanent can pay a sacrifice cost.
pub(in crate::deck::simulator) fn has_sacrificable_creature(
    deck: &SimDeck,
    st: &GameState,
) -> bool {
    st.battlefield
        .iter()
        .any(|p| !p.tapped && card_of(deck, p).is_creature)
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

/// Fire the dying permanent's own Dies triggers (afterlife, "when
/// this dies" payoffs) from its last-known card.
fn fire_victim_death_triggers(deck: &SimDeck, st: &mut GameState, turn: u32, victim: &Permanent) {
    if victim.card.deck_idx().is_none() {
        return;
    }
    let abilities: Vec<SimAbility> = card_of(deck, victim)
        .unlocked_abilities(victim.counters.charge)
        .filter(|ability| ability.kind.is_triggered() && ability.trigger == SimTrigger::Dies)
        .filter(|ability| {
            super::super::game::trigger_subject_matches(
                ability.event_subject,
                victim.uid,
                victim.uid,
            )
        })
        .cloned()
        .collect();
    for ability in abilities {
        let ability_key = (victim.uid, ability.id);
        if ability.once_per_turn && st.triggered_this_turn.contains(&ability_key) {
            continue;
        }
        if ability
            .condition
            .is_some_and(|condition| !super::super::game::condition_met(deck, st, &condition))
        {
            continue;
        }
        for effect in ability.effect_sequence() {
            apply_effect_at(deck, effect, st, turn, false, victim.card.deck_idx());
        }
        if ability.once_per_turn {
            st.triggered_this_turn.insert(ability_key);
        }
    }
}

/// Fire the surviving board's Dies triggers. Station striations only
/// exist once unlocked (CR 721.2a). Death-trigger mills are graveyard
/// fuel (self), and the effects resolve from the board as it stands.
pub(in crate::deck::simulator) fn fire_death_triggers(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    event_uid: u32,
) {
    let death_triggers: Vec<(u32, SimAbility)> = st
        .battlefield
        .iter()
        .flat_map(|perm| {
            super::super::game::permanent_abilities(deck, perm)
                .cloned()
                .map(move |ability| (perm.uid, ability))
        })
        .filter_map(|(source_uid, ability)| {
            if !ability.kind.is_triggered() || ability.trigger != SimTrigger::Dies {
                return None;
            }
            // Survivors can see another permanent die or any permanent
            // die, but a this-permanent trigger belongs to the victim.
            (ability.event_subject != super::super::model::SimEventSubject::This
                && super::super::game::trigger_subject_matches(
                    ability.event_subject,
                    source_uid,
                    event_uid,
                ))
            .then_some((source_uid, ability))
        })
        .collect();
    for (source_uid, ability) in death_triggers {
        let ability_key = (source_uid, ability.id);
        if ability.once_per_turn && st.triggered_this_turn.contains(&ability_key) {
            continue;
        }
        if ability
            .condition
            .is_some_and(|condition| !super::super::game::condition_met(deck, st, &condition))
        {
            continue;
        }
        let source = st
            .battlefield
            .iter()
            .find(|perm| perm.uid == source_uid)
            .and_then(|perm| perm.card.deck_idx());
        let mills_opponent = st
            .battlefield
            .iter()
            .find(|perm| perm.uid == source_uid)
            .is_some_and(|perm| card_of(deck, perm).mills_opponent);
        for effect in ability.effect_sequence() {
            apply_effect_at(deck, effect, st, turn, mills_opponent, source);
        }
        if ability.once_per_turn {
            st.triggered_this_turn.insert(ability_key);
        }
    }
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
    let victim_perm = st.battlefield[v].clone();
    let victim_card = victim_perm.card;
    st.battlefield.remove(v);
    if let Some(idx) = victim_card.deck_idx() {
        move_to_graveyard(deck, st, idx, turn, super::CardZone::Battlefield);
    }
    // The dying permanent's own leaves-the-battlefield triggers fire
    // first (CR 603.6c uses last-known information), then the surviving
    // board's.
    fire_victim_death_triggers(deck, st, turn, &victim_perm);
    fire_death_triggers(deck, st, turn, victim_perm.uid);
    if let Some(idx) = victim_card.deck_idx()
        && deck[idx].has_undying
        && victim_perm.counters.plus1 == 0
    {
        st.graveyard.retain(|index| *index != idx);
        st.battlefield_seen.entry(idx).or_insert(turn);
        let uid = take_uid(st);
        let mut returned = super::super::game::new_perm_with(uid, deck, idx, turn, false);
        returned.counters.plus1 = 1;
        st.battlefield.push(returned);
    }
}
