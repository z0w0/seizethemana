//! The cast pass and land-drop helpers for the goldfish game loop, split
//! from game_run.rs to keep files small. Pure apart from the game state
//! mutations they drive.

use super::game::{
    GameState, ManaPool, card_of, fire_on_enter, fire_triggers, new_perm_with, take_uid,
};
use super::game_effects::{CardZone, draw_one, move_to_graveyard};
use super::game_mana::{effective_min_cost, pay_cost, payable, phyrexian_life_charge, pips_ok};
use super::model::{CardIdx, Role, SimDeck, SimTrigger};

/// One affordable spell's resolution (cost payment, spell data, storm).
#[path = "cast_pass/cast_resolve.rs"]
pub(in crate::deck::simulator) mod cast_resolve;
/// Spell selection and resolution: the sweep that spends the pool on casts.
mod cast_sweep;
/// Face-down casting and turning face up (morph, megamorph, disguise).
mod face_down;

pub(in crate::deck::simulator) use cast_resolve::{resolve_cast, upkeep_trigger_registration};
pub(super) use face_down::{resolve_face_down, turn_face_up};

/// Play one land from the hand (untapped first), fetch a search land,
/// and fire its ETB triggers. Returns true when a land was played.
pub(super) fn play_land(deck: &SimDeck, st: &mut GameState, turn: u32) -> bool {
    // Land/spell MDFCs play their land face only when the hand holds no
    // other land to play this turn; otherwise they stay as spells.
    let mdfc_fallback = |st: &GameState| {
        st.hand
            .iter()
            .position(|idx| deck[*idx].is_mdfc_spell)
            .filter(|_| !st.hand.iter().any(|i| deck[*i].role == Role::Land))
    };
    let land_pos = st
        .hand
        .iter()
        .position(|idx| deck[*idx].role == Role::Land && !deck[*idx].enters_tapped)
        .or_else(|| st.hand.iter().position(|idx| deck[*idx].role == Role::Land))
        .or_else(|| mdfc_fallback(st));
    let Some(pos) = land_pos else {
        return false;
    };
    let idx = st.hand.remove(pos);
    let card = &deck[idx];
    let life_cost = card.life_to_untap;
    let can_pay_life = life_cost > 0 && st.life > life_cost as i32;
    if can_pay_life {
        st.life -= life_cost as i32;
        st.life_paid += life_cost;
    }
    let tapped_in = card.enters_tapped || life_cost > 0 && !can_pay_life;
    st.battlefield_seen.entry(idx).or_insert(turn);
    let uid = take_uid(st);
    st.battlefield
        .push(new_perm_with(uid, deck, idx, turn, tapped_in));
    // ETB triggers fire for the played land; capture its slot before a
    // fetch adds a second permanent behind it.
    let played_pos = st.battlefield.len() - 1;
    if card.is_fetch_land {
        fire_on_enter(deck, st, played_pos, turn, false);
        fire_triggers(deck, st, SimTrigger::LandEnters, turn);
        // Fetch lands pay one life and sacrifice themselves before searching.
        // The parsed Oracle search provides the target type restriction.
        let fetch_life_cost = card.fetch_life_cost;
        if st.life <= fetch_life_cost as i32 {
            return true;
        }
        st.life -= fetch_life_cost as i32;
        st.life_paid += fetch_life_cost;
        let target_types = &card.fetch_target_types;
        if let Some(i) = st.library.iter().position(|c| {
            let candidate = &deck[*c];
            candidate.role == Role::Land
                && !candidate.is_fetch_land
                && (!card.fetch_basic_only || candidate.is_basic_land)
                && candidate
                    .land_types
                    .iter()
                    .enumerate()
                    .any(|(index, has_type)| {
                        *has_type && target_types.contains(&super::model::BasicLandType::ALL[index])
                    })
        }) {
            let fetched = st.library.remove(i);
            st.battlefield_seen.entry(fetched).or_insert(turn);
            let fuid = take_uid(st);
            let fetched_card = &deck[fetched];
            let fetched_life_cost = fetched_card.life_to_untap;
            let fetched_can_pay_life = fetched_life_cost > 0 && st.life > fetched_life_cost as i32;
            if fetched_can_pay_life {
                st.life -= fetched_life_cost as i32;
                st.life_paid += fetched_life_cost;
            }
            let fetched_tapped = card.fetch_enters_tapped
                || fetched_card.enters_tapped
                || fetched_life_cost > 0 && !fetched_can_pay_life;
            st.battlefield
                .push(new_perm_with(fuid, deck, fetched, turn, fetched_tapped));
            fire_on_enter(deck, st, st.battlefield.len() - 1, turn, false);
            fire_triggers(deck, st, SimTrigger::LandEnters, turn);
        }
        st.battlefield.remove(played_pos);
        move_to_graveyard(deck, st, idx, turn, CardZone::Battlefield);
        return true;
    }
    fire_on_enter(deck, st, played_pos, turn, false);
    fire_triggers(deck, st, SimTrigger::LandEnters, turn);
    true
}

/// The cast pass: cheapest castable spells first, pip-aware. Updates the
/// pool, ETB triggers, and the mana-ready curve. Returns nothing; the
/// caller owns every mutated binding.
// The 11 parameters are the game state the cast phase needs in full;
// a parameter struct would just be read back out field by field.
#[allow(clippy::too_many_arguments)]
pub(super) fn cast_pass(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    mana_spent: &mut [f64],
    repeatable_sources: &mut Vec<(u32, u32)>,
    pip_blocks: &mut Vec<(CardIdx, usize)>,
    blocked_colors: &mut [bool; 5],
) {
    let preparing_graveyard_exchange = st.hand.iter().any(|index| deck[*index].has_cascade)
        && st
            .library
            .iter()
            .any(|index| deck[*index].spell_data.graveyard_creature_exchange)
        && st.hand.iter().any(|index| {
            deck[*index].is_creature
                && (deck[*index].spell_data.cycling_cost.is_some()
                    || deck[*index].spell_data.cycling_life > 0)
        });
    if preparing_graveyard_exchange {
        cycle_unusable_cards(deck, st, pool, turn as u32);
    }
    // 5 CAST: cheapest castable spells (pip-aware). Spend-restricted
    // mana pays creature casts only.
    let mut order: Vec<CardIdx> = st
        .hand
        .iter()
        .copied()
        .filter(|idx| deck[*idx].role != Role::Land)
        .collect();
    order.sort_by_key(|idx| deck[*idx].min_cost.total());
    // Card identities of each cast: mid-cast hand changes (wheels,
    // loots) shift positions, so the cast resolves and removes by card
    // index, never by the stale position.
    let mut cast_ids: Vec<CardIdx> = Vec::new();
    // (uid, card index) of each cast's battlefield permanent: the ETB
    // pass fires only for these (lands and effect-pushes are excluded).
    let mut cast_ets: Vec<(u32, CardIdx)> = Vec::new();
    let mut spent_total = 0u32;
    // Rituals add mana mid-pass (mana_on_cast, mana_per_cast), so a
    // card unaffordable on first sight can become payable later in the
    // same pass. The pass repeats until a full sweep casts nothing.
    let mut queue: Vec<CardIdx> = order;
    loop {
        let progress = cast_sweep::cast_pass(
            deck,
            st,
            pool,
            turn,
            &mut queue,
            &mut cast_ids,
            &mut cast_ets,
            &mut spent_total,
            repeatable_sources,
            pip_blocks,
            blocked_colors,
        );
        if !progress {
            break;
        }
        // Draws and loot may add new legal spells. Rebuild from the live
        // hand after every productive sweep; already-cast identities are
        // excluded, while skipped unaffordable cards remain candidates.
        queue = st
            .hand
            .iter()
            .copied()
            .filter(|idx| deck[*idx].role != Role::Land && !cast_ids.contains(idx))
            .collect();
        queue.sort_by_key(|idx| deck[*idx].min_cost.total());
        if queue.is_empty() {
            break;
        }
    }
    mana_spent[turn - 1] += spent_total as f64;
    // Face-down bodies turn up while the pool covers their morph or
    // disguise cost (CR 702.37e). The cast pass leaves spare mana in the
    // pool, so the flip resolves here.
    turn_face_up(deck, st, pool, &mut mana_spent[turn - 1]);
    // Nonpermanent spells go to the graveyard after resolution. Remove cast
    // cards by identity because wheels and loots can change hand order.
    for idx in cast_ids.iter() {
        if let Some(pos) = st.hand.iter().position(|i| i == idx) {
            st.hand.remove(pos);
        }
        if deck[*idx].is_instant_or_sorcery {
            if deck[*idx].exile_on_resolve {
                st.exile.push(*idx);
            } else {
                move_to_graveyard(deck, st, *idx, turn as u32, CardZone::Stack);
            }
        }
    }
    // ETB triggers for cards cast this turn (they entered the board);
    // their upkeep triggers register for the next turn. Entries resolve
    // through stable uids: lands played this turn already fired via
    // `play_land` and reveal-effect pushes (tokens, returned bodies)
    // never had a cast, so both are excluded; token pushes during a
    // fire cannot shift another entry's identity.
    let cast_uids: Vec<u32> = cast_ets.iter().map(|e| e.0).collect();
    let newly_cast: Vec<(u32, CardIdx)> = st
        .battlefield
        .iter()
        .filter(|p| {
            p.entered_turn == turn && p.card.deck_idx().is_some() && cast_uids.contains(&p.uid)
        })
        .filter_map(|p| p.card.deck_idx().map(|idx| (p.uid, idx)))
        .collect();
    for (uid, card_idx) in &newly_cast {
        // Re-resolve the position at fire time: earlier fires can push
        // tokens and shift indices.
        let Some(pos) = st.battlefield.iter().position(|p| p.uid == *uid) else {
            continue;
        };
        fire_on_enter(deck, st, pos, turn as u32, false);
        // Upkeep triggers on the cast card register now, by uid. Locked
        // station tiers register nothing yet (CR 721.2a): the trigger
        // belongs to a striation that does not exist below its threshold.
        let charge = st.battlefield[pos].counters.charge;
        for ability in deck[*card_idx].unlocked_abilities(charge) {
            if let Some(draws) = upkeep_trigger_registration(ability) {
                super::game::register_repeatable_source(repeatable_sources, *uid, draws);
            }
        }
    }
    cast_graveyard_spells(deck, st, pool, turn, mana_spent, repeatable_sources);
    if cycle_unusable_cards(deck, st, pool, turn as u32) {
        cast_pass(
            deck,
            st,
            pool,
            turn,
            mana_spent,
            repeatable_sources,
            pip_blocks,
            blocked_colors,
        );
    }
}

/// Cast legal graveyard instances through active flashback or escape permissions.
pub(super) fn cast_graveyard_spells(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    mana_spent: &mut [f64],
    repeatable_sources: &mut Vec<(u32, u32)>,
) {
    for _ in 0..super::model::MAX_LOOP_PASSES {
        let escape_active = st
            .battlefield
            .iter()
            .any(|permanent| card_of(deck, permanent).spell_data.grants_escape);
        // Own printed flashback/escape costs (CR 702.34, 702.138) sit
        // next to the granted shapes: a self-flashback costs its printed
        // flashback cost and exiles; a self-escape costs its printed
        // escape cost plus three exiled cards.
        let mut candidates: Vec<(CardIdx, bool)> = st
            .graveyard
            .iter()
            .copied()
            .filter_map(|index| {
                let card = &deck[index];
                let flashback = st.flashback_permissions.contains(&index)
                    && card.is_instant_or_sorcery
                    || card.spell_data.own_flashback.is_some() && card.is_instant_or_sorcery;
                let escape = (escape_active || card.spell_data.own_escape.is_some())
                    && card.role != Role::Land;
                (flashback || escape).then_some((index, escape && !flashback))
            })
            .collect();
        candidates.sort_by_key(|(index, _)| deck[*index].min_cost.total());
        let mut cast_one = false;
        for (index, escape) in candidates {
            if !st.graveyard.contains(&index) {
                continue;
            }
            let fodder: Vec<CardIdx> = st
                .graveyard
                .iter()
                .copied()
                .filter(|other| *other != index)
                .take(3)
                .collect();
            if escape && fodder.len() < 3 {
                continue;
            }
            let own_costs = &deck[index].spell_data;
            let cost = if !escape {
                own_costs
                    .own_flashback
                    .unwrap_or_else(|| effective_min_cost(deck, &deck[index], &st.battlefield))
            } else {
                own_costs
                    .own_escape
                    .unwrap_or_else(|| effective_min_cost(deck, &deck[index], &st.battlefield))
            };
            // Additional life costs gate the same way hand casts gate
            // (the cast would pay the life and could drive life
            // negative).
            if st.life <= deck[index].spell_data.additional_cost_life as i32 {
                continue;
            }
            // Phyrexian pips pay with 2 life each (CR 107.4f).
            let charge = phyrexian_life_charge(&cost);
            if st.life <= charge as i32 {
                continue;
            }
            if !payable(&cost, pool) || !pips_ok(&cost, pool) {
                continue;
            }
            if escape {
                for other in fodder {
                    if let Some(position) = st.graveyard.iter().position(|held| *held == other) {
                        st.exile.push(st.graveyard.remove(position));
                    }
                }
            }
            if let Some(position) = st.graveyard.iter().position(|held| *held == index) {
                st.graveyard.remove(position);
            }
            st.flashback_permissions.remove(&index);
            st.life -= charge as i32;
            st.life_paid += charge;
            let mut replay_ids = Vec::new();
            let mut replay_ets = Vec::new();
            let mut spent = 0;
            resolve_cast(
                deck,
                st,
                pool,
                turn,
                index,
                &cost,
                &mut replay_ids,
                &mut replay_ets,
                &mut spent,
                repeatable_sources,
                true,
            );
            st.replay_casts += 1;
            super::game::milestone_for_turn(st, turn as u32).graveyard_casts += 1;
            mana_spent[turn - 1] += spent as f64;
            if deck[index].is_instant_or_sorcery
                || (deck[index].spell_data.own_flashback.is_some()
                    && !st.graveyard.contains(&index))
            {
                st.exile.push(index);
            }
            for (uid, card_index) in replay_ets {
                if let Some(position) = st.battlefield.iter().position(|perm| perm.uid == uid) {
                    fire_on_enter(deck, st, position, turn as u32, false);
                    let charge = st.battlefield[position].counters.charge;
                    for ability in deck[card_index].unlocked_abilities(charge) {
                        if let Some(draws) = upkeep_trigger_registration(ability) {
                            super::game::register_repeatable_source(repeatable_sources, uid, draws);
                        }
                    }
                }
            }
            cast_one = true;
            break;
        }
        if !cast_one {
            break;
        }
    }
}

/// Cycle uncast cards when their cycling cost is payable, then retry casts.
fn cycle_unusable_cards(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let mut cycled = false;
    let candidates = st.hand.clone();
    for index in candidates {
        let card = &deck[index];
        if card.spell_data.cycling_cost.is_none() && card.spell_data.cycling_life == 0 {
            continue;
        }
        let cost = card
            .spell_data
            .cycling_cost
            .as_ref()
            .cloned()
            .unwrap_or_default();
        // Phyrexian pips in the cycling cost pay with 2 life each
        // (CR 107.4f), on top of any "pay N life" rider.
        let charge = phyrexian_life_charge(&cost);
        if !st.hand.contains(&index)
            || st.life <= (card.spell_data.cycling_life + charge) as i32
            || !payable(&cost, pool)
            || !pips_ok(&cost, pool)
        {
            continue;
        }
        pay_cost(&cost, pool);
        st.life -= (card.spell_data.cycling_life + charge) as i32;
        st.life_paid += card.spell_data.cycling_life + charge;
        let funded_draw = u32::from(
            card.spell_data.cycling_life > 0 && card.spell_data.landcycling_type.is_none(),
        );
        st.life_funded_draws += funded_draw;
        super::game::milestone_for_turn(st, turn).life_funded_draws += funded_draw;
        st.hand.retain(|held| *held != index);
        move_to_graveyard(deck, st, index, turn, CardZone::Hand);
        if let Some(land) = card.spell_data.landcycling_type {
            let color_index = land.index();
            let target = st.library.iter().rposition(|candidate| {
                deck[*candidate].role == Role::Land && deck[*candidate].land_types[color_index]
            });
            if let Some(pos) = target {
                st.hand.push(st.library.remove(pos));
            }
        } else {
            draw_one(deck, st, turn);
        }
        cycled = true;
    }
    cycled
}

/// Reveal cards for the supported life-payment spell until life reaches zero.
pub(in crate::deck::simulator) fn resolve_reveal_rule(
    deck: &SimDeck,
    st: &mut GameState,
    rule: super::model::RevealRule,
    turn: u32,
) {
    while st.life > 0 {
        let Some(index) = st.library.pop() else {
            break;
        };
        match rule.destination {
            super::model::RevealDestination::Hand => st.hand.push(index),
        }
        st.seen += 1;
        st.awareness_cards += 1;
        match rule.life_loss {
            super::model::RevealLifeLoss::ManaValue => {
                st.life -= deck[index].mana_value as i32;
                st.life_funded_draws += 1;
                super::game::milestone_for_turn(st, turn).life_funded_draws += 1;
            }
        }
    }
}

/// Select cards from hand that satisfy a parsed alternate casting cost.
/// A fixed-count cost needs every card; an optional exile (the life
/// payoff) takes whatever matching cards the hand holds.
pub(in crate::deck::simulator) fn select_alternative_cost_cards(
    deck: &SimDeck,
    st: &GameState,
    spell: CardIdx,
) -> Vec<CardIdx> {
    let Some(cost) = deck[spell].spell_data.alternative_cast_cost else {
        return Vec::new();
    };
    let mut candidates: Vec<CardIdx> = st
        .hand
        .iter()
        .copied()
        .filter(|index| *index != spell)
        .filter(|index| {
            cost.filter
                .color
                .is_none_or(|color| deck[*index].colors[color.index()])
        })
        .collect();
    candidates.sort_by_key(|index| std::cmp::Reverse(deck[*index].mana_value));
    if cost.payoff == super::model::AlternativeCostPayoff::GainLifeEqualToExiledManaValue {
        return candidates;
    }
    if candidates.len() < cost.count as usize {
        return Vec::new();
    }
    candidates.truncate(cost.count as usize);
    candidates
}
