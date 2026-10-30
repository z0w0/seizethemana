//! Payment-aware choices for complete, one-mana land abilities.
//!
//! The planner checks one visible commander or hand spell before floating mana.
//! It does not reserve that spell, plan later draws, or retap a floated source.
//! Painless colorless mana cannot later switch to a painful colored mode.
//! Ready one-mana creatures and actual cast resources share physical source slots.

use crate::deck::simulator::game::{
    GameState, ManaPool, Permanent, card_of, is_creature_permanent, is_land_permanent,
    permanent_abilities,
};
use crate::deck::simulator::game_mana::{
    cast_restrictions, effective_cast_cost, payment_plan_with_cast_sources,
    payment_plan_with_source_costs, phyrexian_life_charge,
};
use crate::deck::simulator::model::{
    CardIdx, Cost, LifeLossScope, ManaYield, Role, SimAbility, SimAbilityKind, SimActivationCost,
    SimDeck, SimEffect, SpendRestriction,
};

/// One complete free-tap mode and its self-directed damage.
pub(super) struct SimpleMode {
    ability_id: Option<usize>,
    types: u8,
    damage: u32,
}

/// Mutually exclusive modes attached to one physical land.
pub(super) struct SimpleSource {
    permanent: Permanent,
    modes: Vec<SimpleMode>,
}

/// A native ability with only equal-cost, equal-effect output choices retained.
pub(super) struct SelectedMode {
    /// Native ability ID, or no ID for a separate granted mana ability.
    pub(super) ability_id: Option<usize>,
    output: ManaYield,
    damage: u32,
}

/// A complete visible cast whose mana and life costs can guide source choices.
struct Target {
    card: Option<CardIdx>,
    cost: Cost,
    classes: Vec<SpendRestriction>,
    life: u32,
}

/// Keep only lands whose native mana modes all fit the bounded choice model.
pub(super) fn simple_source(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
) -> Option<SimpleSource> {
    if source.tapped
        || source.face_down
        || !is_land_permanent(source)
        || is_creature_permanent(deck, source)
        || !super::mana_condition_met(deck, st, card_of(deck, source))
    {
        return None;
    }
    let modes = source_modes(deck, st, source)?;
    if modes.len() < 2 && !modes.iter().any(|mode| mode.damage > 0) {
        return None;
    }
    Some(SimpleSource {
        permanent: source.clone(),
        modes,
    })
}

/// Retain complete native modes whose individual conditions are currently met.
fn source_modes(deck: &SimDeck, st: &GameState, source: &Permanent) -> Option<Vec<SimpleMode>> {
    let abilities: Vec<_> = permanent_abilities(deck, source)
        .filter(|ability| ability.kind == SimAbilityKind::ManaActivated)
        .collect();
    let modes: Vec<_> = abilities
        .iter()
        .map(|ability| simple_mode(deck, st, ability))
        .collect::<Option<_>>()?;
    Some(
        modes
            .into_iter()
            .zip(abilities)
            .filter(|(_, ability)| {
                ability.condition.is_none_or(|condition| {
                    crate::deck::simulator::game::condition_met(deck, st, &condition)
                })
            })
            .map(|(mode, _)| mode)
            .collect(),
    )
}

/// Include ready one-mana creatures without reserving bodies that the payment does not use.
fn ready_creature_source(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    turn: u32,
) -> Option<SimpleSource> {
    if source.tapped
        || !is_creature_permanent(deck, source)
        || (source.summoning_sick || source.entered_turn >= turn as usize)
            && !crate::deck::simulator::game::effective_keywords(deck, st, source)
                .contains(crate::deck::simulator::model::Keyword::Haste)
    {
        return None;
    }
    let modes = if super::grants_active(deck, st).creatures {
        vec![SimpleMode {
            ability_id: None,
            types: 31,
            damage: 0,
        }]
    } else {
        if source.face_down || !super::mana_condition_met(deck, st, card_of(deck, source)) {
            return None;
        }
        source_modes(deck, st, source)?
    };
    if modes.is_empty() || modes.iter().any(|mode| mode.damage > 0) {
        return None;
    }
    Some(SimpleSource {
        permanent: source.clone(),
        modes,
    })
}

/// Reject extra costs and effects rather than treating them as free mana choices.
fn simple_mode(deck: &SimDeck, st: &GameState, ability: &SimAbility) -> Option<SimpleMode> {
    let activation = ability.activation.as_ref()?;
    if activation.costs != [SimActivationCost::TapSource]
        || !activation.restrictions.is_empty()
        || ability.resolution_payment.is_some()
        || ability.opponent_tax.is_some()
    {
        return None;
    }
    let (yield_, damage) = match ability.effect_sequence() {
        [SimEffect::Mana(yield_)] => (yield_, 0),
        [
            SimEffect::Mana(yield_),
            SimEffect::Damage {
                amount,
                scope: LifeLossScope::SelfOnly,
            },
        ] => (yield_, *amount),
        _ => return None,
    };
    if yield_.restriction.is_some()
        || yield_.cannot_pay_generic
        || yield_.scaling.is_some()
        || yield_.opponent_any
        || yield_.legendary_colors
    {
        return None;
    }
    let mut pool = ManaPool::default();
    crate::deck::simulator::game_mana::add_yield_turns(
        deck,
        yield_,
        &mut pool,
        st.current_turn,
        &st.battlefield,
    );
    if pool.total() != 1 {
        return None;
    }
    let mut types = u8::from(pool.colorless > 0) << 5;
    if pool.flexible > 0 {
        types |= 31;
    }
    for (color, amount) in pool.fixed.iter().enumerate() {
        types |= u8::from(*amount > 0) << color;
    }
    for lot in pool.lots {
        types |= lot.types;
    }
    (types != 0).then_some(SimpleMode {
        ability_id: Some(ability.id),
        types,
        damage,
    })
}

/// Choose modes only after ordinary sources and banked Treasures are visible.
pub(super) fn build_simple_sources(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) {
    let mut sources: Vec<_> = st
        .battlefield
        .iter()
        .filter_map(|source| simple_source(deck, st, source))
        .collect();
    sources.extend(
        st.battlefield
            .iter()
            .filter_map(|source| ready_creature_source(deck, st, source, turn)),
    );
    activate_simple_sources(deck, st, pool, &sources, turn, false);
}

/// Replan after each activation, using real painless production before taking damage.
pub(super) fn activate_simple_sources(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    sources: &[SimpleSource],
    turn: u32,
    nonspell: bool,
) -> bool {
    if sources.is_empty() {
        return false;
    }
    let mut activated = false;
    if !nonspell {
        for _ in 0..sources.len() {
            if st.completion.is_some() {
                return activated;
            }
            let available = available_sources(deck, st, sources, turn);
            if available.is_empty() {
                break;
            }
            let chosen = visible_targets(deck, st)
                .into_iter()
                .find_map(|target| choices_for_target(deck, st, pool, &available, &target))
                .unwrap_or_default();
            let next = chosen
                .into_iter()
                .filter_map(|(index, types)| {
                    select_mode(&available[index], Some(types))
                        .map(|mode| (&available[index].permanent, mode))
                })
                .min_by_key(|(_, mode)| mode.damage);
            let Some((source, mode)) = next else {
                break;
            };
            if !super::activate_mana_mode_for_use(deck, st, source, pool, turn, false, Some(&mode))
            {
                break;
            }
            activated = true;
        }
    }
    for source in available_sources(deck, st, sources, turn) {
        if st.completion.is_some() {
            break;
        }
        if !is_creature_permanent(deck, &source.permanent)
            && let Some(mode) = select_mode(&source, None)
        {
            activated |= super::activate_mana_mode_for_use(
                deck,
                st,
                &source.permanent,
                pool,
                turn,
                nonspell,
                Some(&mode),
            );
        }
    }
    activated
}

/// Refresh only the original physical UIDs against their current readiness and modes.
fn available_sources(
    deck: &SimDeck,
    st: &GameState,
    sources: &[SimpleSource],
    turn: u32,
) -> Vec<SimpleSource> {
    sources
        .iter()
        .filter_map(|source| {
            st.battlefield
                .iter()
                .find(|current| current.uid == source.permanent.uid)
        })
        .filter_map(|source| {
            if is_creature_permanent(deck, source) {
                ready_creature_source(deck, st, source, turn)
            } else {
                simple_source(deck, st, source)
            }
        })
        .collect()
}

/// Choose the minimum damage among complete payments using all current source types.
fn choices_for_target(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    sources: &[SimpleSource],
    target: &Target,
) -> Option<Vec<(usize, u8)>> {
    let mut costs: Vec<_> = sources.iter().map(source_type_costs).collect();
    let (_, chosen) = if let Some(index) = target.card {
        let resources = crate::deck::simulator::cast_pass::cast_payment_resources(
            deck,
            st,
            index,
            &target.cost,
        );
        let mut virtual_taps = vec![None; costs.len()];
        for (uid, types) in resources.tap_sources {
            if let Some(source) = sources
                .iter()
                .position(|source| source.permanent.uid == uid)
            {
                virtual_taps[source] = Some(types);
            } else {
                costs.push([None; 6]);
                virtual_taps.push(Some(types));
            }
        }
        payment_plan_with_cast_sources(
            &resources.owed,
            pool,
            &target.classes,
            &costs,
            &virtual_taps,
        )?
    } else {
        payment_plan_with_source_costs(&target.cost, pool, &target.classes, &costs)?
    };
    let damage: u64 = chosen
        .iter()
        .map(|(index, types)| {
            u64::from(
                costs[*index][types.trailing_zeros() as usize]
                    .expect("a matched source type has a supported mode"),
            )
        })
        .sum();
    let available_life = u64::try_from(st.life).ok()?;
    ((damage == 0 || available_life > damage) && available_life - damage >= u64::from(target.life))
        .then_some(chosen)
}

/// Keep the lowest native damage for each type from one physical source.
fn source_type_costs(source: &SimpleSource) -> [Option<u32>; 6] {
    let mut costs = [None; 6];
    for mode in &source.modes {
        for (color, cost) in costs.iter_mut().enumerate() {
            if mode.types & (1 << color) != 0 {
                *cost = Some(cost.map_or(mode.damage, |held: u32| held.min(mode.damage)));
            }
        }
    }
    costs
}

/// Preserve alternative colors only when every merged mode has the same damage.
fn select_mode(source: &SimpleSource, requested: Option<u8>) -> Option<SelectedMode> {
    let selected = source
        .modes
        .iter()
        .filter(|mode| requested.map_or(mode.damage == 0, |types| mode.types & types != 0))
        .min_by_key(|mode| mode.damage)?;
    let types = source
        .modes
        .iter()
        .filter(|mode| mode.damage == selected.damage)
        .fold(0, |types, mode| types | mode.types);
    let mut output = ManaYield {
        choice_colorless: types & 32 != 0,
        ..Default::default()
    };
    for (color, present) in output.choice.iter_mut().enumerate() {
        *present = types & (1 << color) != 0;
    }
    Some(SelectedMode {
        ability_id: selected.ability_id,
        output,
        damage: selected.damage,
    })
}

/// Mirror normal cast eligibility before spending life on a mana choice.
fn visible_targets(deck: &SimDeck, st: &GameState) -> Vec<Target> {
    let mut targets = Vec::new();
    for (slot, commander) in deck.commanders.iter().enumerate() {
        let costs = &commander.spell_data;
        if !commander.has_mana_cost
            || costs.unsupported_additional_cost
            || costs.additional_cost_creatures > 0
            || costs.additional_cost_discards > 0
            || costs.additional_cost_life > 0
            || !crate::deck::simulator::game_effects::spell_targets_available(deck, st, commander)
            || st.battlefield.iter().any(|permanent| {
                matches!(permanent.card, crate::deck::simulator::game::CardRef::Commander { slot: held } if held == slot)
            })
        {
            continue;
        }
        let cost = effective_cast_cost(
            deck,
            st,
            commander,
            commander.min_cost,
            st.commander_casts
                .get(slot)
                .copied()
                .unwrap_or(0)
                .saturating_mul(2),
            None,
        );
        targets.push(Target {
            card: None,
            cost,
            classes: cast_restrictions(commander),
            life: phyrexian_life_charge(&cost),
        });
    }
    let mut hand = st.hand.clone();
    hand.sort_by_key(|index| deck[*index].min_cost.total());
    for index in hand {
        let card = &deck[index];
        if card.role == Role::Land
            || !card.has_mana_cost
            || card.flags.is_interaction
                && card.role == Role::Removal
                && card.spell_data.damage_on_resolve == 0
                && card.spell_data.life_loss_on_resolve == 0
                && !card.mills_opponent
            || !crate::deck::simulator::cast_pass::additional_costs_payable(deck, st, index)
        {
            continue;
        }
        let alternate =
            crate::deck::simulator::cast_pass::select_alternative_cost_cards(deck, st, index);
        if !alternate.is_empty()
            && st.hand.len() - 1 - alternate.len()
                >= card.spell_data.additional_cost_discards as usize
        {
            let cost = effective_cast_cost(deck, st, card, Cost::default(), 0, Some(index));
            targets.push(Target {
                card: Some(index),
                cost,
                classes: cast_restrictions(card),
                life: card.spell_data.additional_cost_life + phyrexian_life_charge(&cost),
            });
        }
        let cost = effective_cast_cost(deck, st, card, card.min_cost, 0, Some(index));
        targets.push(Target {
            card: Some(index),
            cost,
            classes: cast_restrictions(card),
            life: card.spell_data.additional_cost_life + phyrexian_life_charge(&cost),
        });
    }
    targets
}

/// Typed mode conditions replace legacy aggregate gates for a selected simple mode.
pub(super) fn produce_mode_yield(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    yield_: &ManaYield,
    pool: &mut ManaPool,
    turn: u32,
    selected: Option<&SelectedMode>,
) -> bool {
    if let Some(selected) = selected {
        super::add_nonland_mana(deck, st, source, &selected.output, pool, turn)
    } else {
        super::add_mode_yield(deck, st, source, yield_, pool, turn)
    }
}

/// Fix each source choice so a mana-tap bonus can use only types produced.
pub(in crate::deck::simulator::game_run) fn add_chosen_source_yield(
    deck: &SimDeck,
    st: &GameState,
    output: &ManaYield,
    pool: &mut ManaPool,
) -> u8 {
    let choices = output
        .choice
        .iter()
        .enumerate()
        .fold(0u8, |mask, (i, present)| {
            mask | if *present { 1 << i } else { 0 }
        })
        | if output.choice_colorless { 32 } else { 0 };
    let mut fixed = output.clone();
    fixed.any_pips = 0;
    fixed.commander_pips = 0;
    fixed.choice = [false; 5];
    fixed.choice_colorless = false;
    let mut produced = 0;
    let commander_types = deck
        .commander_color_identity()
        .iter()
        .enumerate()
        .fold(0u8, |mask, (i, present)| {
            mask | if *present { 1 << i } else { 0 }
        });
    let choices = if output.alternatives {
        fixed.fixed = [0; 5];
        fixed.colorless = 0;
        choices
            | if output.any_pips > 0 { 31 } else { 0 }
            | if output.commander_pips > 0 {
                commander_types
            } else {
                0
            }
            | if output.colorless > 0 { 32 } else { 0 }
    } else {
        choices
    };
    let quantities = if output.alternatives {
        [(1, choices), (0, 0), (0, 0)]
    } else {
        [
            (output.any_pips, 31),
            (output.commander_pips, commander_types),
            (u32::from(choices != 0), choices),
        ]
    };
    fixed.alternatives = false;
    for (amount, allowed) in quantities {
        if amount == 0 || allowed == 0 {
            continue;
        }
        let chosen = choose_source_type(deck, st, allowed);
        pool.lots.push(crate::deck::simulator::game_mana::ManaLot {
            amount,
            types: chosen,
            restriction: output.restriction,
            cannot_pay_generic: output.cannot_pay_generic,
            one_color: false,
        });
        produced |= chosen;
    }
    for (color, amount) in fixed.fixed.iter().enumerate() {
        if *amount > 0 {
            produced |= 1 << color;
        }
    }
    if fixed.colorless > 0 {
        produced |= 32;
    }
    crate::deck::simulator::game_mana::add_yield(&fixed, pool);
    produced
}

/// Choose a legal mana type from visible hand costs with stable WUBRG ties.
fn choose_source_type(deck: &SimDeck, st: &GameState, allowed: u8) -> u8 {
    let color = (0..6)
        .filter(|color| allowed & (1 << color) != 0)
        .max_by_key(|color| {
            let demand: u32 = st
                .hand
                .iter()
                .map(|index| {
                    let cost = deck[*index].min_cost;
                    if *color == 5 {
                        u32::from(cost.colorless)
                    } else {
                        u32::from(cost.pips[*color])
                    }
                })
                .sum();
            (demand, std::cmp::Reverse(*color))
        })
        .expect("a produced choice has at least one allowed mana type");
    1 << color
}

/// Source-driven mode choices and bounded planner regressions.
#[cfg(test)]
#[path = "tests/mana_modes_tests.rs"]
mod mana_modes_tests;

/// Base payments, ready creature sources, and minimum-damage choices.
#[cfg(test)]
#[path = "tests/mana_modes_payment_tests.rs"]
mod mana_modes_payment_tests;

/// Shared physical cast resources and mandatory costs on pitch alternatives.
#[cfg(test)]
#[path = "tests/mana_modes_cast_tests.rs"]
mod mana_modes_cast_tests;

/// Replanning after actual painless production includes supported nonland mana bonuses.
#[cfg(test)]
#[path = "tests/mana_modes_bonus_tests.rs"]
mod mana_modes_bonus_tests;
