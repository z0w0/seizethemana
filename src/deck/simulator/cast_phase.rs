// The cast pass and land-drop helpers for the goldfish game loop, split
// from game_run.rs to keep files small. Pure apart from the game state
// mutations they drive.

use super::game::{
    GameState, Pool, card_of, fire_on_enter, fire_triggers, new_perm_with,
    register_loyalty_token_engines, take_uid,
};
use super::game_effects::{
    CardZone, apply_effect_at, draw_one, mill_library_card, move_to_graveyard,
};
use super::game_mana::{
    add_yield_turns_empty_board, cast_restriction, effective_min_cost, pay_cost,
    pay_restricted_cost, payable, pips_ok, usable_for_noncreature,
};
use super::model::{AbilityTiming, CardIdx, Cost, Effect, Role, SimDeck};

mod cast_sweep;

/// The drain multiplier for this deck's format: three opponents in the
/// commander family, one in constructed.
fn drain_mult_in(deck: &SimDeck) -> u32 {
    deck.format.drain_mult()
}

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
        fire_triggers(deck, st, AbilityTiming::OnLandfall, turn);
        // Fetch lands pay one life and sacrifice themselves before searching.
        // Their target restriction comes from the fetch name, not from the
        // broad land role shared by every library entry.
        let fetch_life_cost = card.fetch_life_cost;
        if st.life <= fetch_life_cost as i32 {
            return true;
        }
        st.life -= fetch_life_cost as i32;
        st.life_paid += fetch_life_cost;
        let fetch_name = card.name.as_str();
        let target_types = if card.fetch_target_types.iter().any(|matches| *matches) {
            card.fetch_target_types
        } else {
            let mut fallback = [false; 5];
            for kind in super::game::fetch_target_pair(fetch_name) {
                if let Some(index) = ["Plains", "Island", "Swamp", "Mountain", "Forest"]
                    .iter()
                    .position(|candidate| candidate == kind)
                {
                    fallback[index] = true;
                }
            }
            fallback
        };
        if let Some(i) = st.library.iter().position(|c| {
            let candidate = &deck[*c];
            candidate.role == Role::Land
                && !candidate.is_fetch_land
                && (!card.fetch_basic_only || candidate.is_basic_land)
                && candidate
                    .land_types
                    .iter()
                    .zip(target_types)
                    .any(|(has_type, target)| *has_type && target)
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
            fire_triggers(deck, st, AbilityTiming::OnLandfall, turn);
        }
        st.battlefield.remove(played_pos);
        move_to_graveyard(deck, st, idx, turn, CardZone::Battlefield);
        return true;
    }
    fire_on_enter(deck, st, played_pos, turn, false);
    fire_triggers(deck, st, AbilityTiming::OnLandfall, turn);
    true
}

/// The cast pass: cheapest castable spells first, pip-aware. Updates the
/// pool, ETB triggers, and the mana-ready curve. Returns nothing; the
/// caller owns every mutated binding.
// The 11 parameters are the game state the cast phase needs in full;
// a parameter struct would just be read back out field by field.
#[allow(clippy::too_many_arguments)]
pub(super) fn cast_phase(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: usize,
    mana_spent: &mut [f64],
    engines: &mut Vec<(u32, u32)>,
    pip_blocks: &mut Vec<(CardIdx, usize)>,
    blocked_colors: &mut [bool; 5],
) {
    let preparing_graveyard_exchange = st.hand.iter().any(|index| deck[*index].has_cascade)
        && st
            .library
            .iter()
            .any(|index| deck[*index].riders.graveyard_creature_exchange)
        && st.hand.iter().any(|index| {
            deck[*index].is_creature
                && (deck[*index].riders.cycling_cost.is_some()
                    || deck[*index].riders.cycling_life > 0)
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
            engines,
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
    // their upkeep engines register for the next turn. Entries resolve
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
        .map(|p| (p.uid, p.card.deck_idx().unwrap()))
        .collect();
    for (uid, card_idx) in &newly_cast {
        // Re-resolve the position at fire time: earlier fires can push
        // tokens and shift indices.
        let Some(pos) = st.battlefield.iter().position(|p| p.uid == *uid) else {
            continue;
        };
        fire_on_enter(deck, st, pos, turn as u32, false);
        // Upkeep engines on the cast card register now, by uid.
        for ability in deck[*card_idx].abilities() {
            if let Some(draws) = engine_effect_or_draws(&ability.trigger, &ability.effect) {
                engines.push((*uid, draws));
            }
        }
    }
    cast_graveyard_spells(deck, st, pool, turn, mana_spent, engines);
    if cycle_unusable_cards(deck, st, pool, turn as u32) {
        cast_phase(
            deck,
            st,
            pool,
            turn,
            mana_spent,
            engines,
            pip_blocks,
            blocked_colors,
        );
    }
}

/// Cast legal graveyard instances through active flashback or escape permissions.
fn cast_graveyard_spells(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: usize,
    mana_spent: &mut [f64],
    engines: &mut Vec<(u32, u32)>,
) {
    for _ in 0..super::model::MAX_LOOP_PASSES {
        let escape_active = st
            .battlefield
            .iter()
            .any(|permanent| card_of(deck, permanent).riders.grants_escape);
        let mut candidates: Vec<(CardIdx, bool)> = st
            .graveyard
            .iter()
            .copied()
            .filter_map(|index| {
                let card = &deck[index];
                let flashback =
                    st.flashback_permissions.contains(&index) && card.is_instant_or_sorcery;
                let escape = escape_active && card.role != Role::Land;
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
            let cost = effective_min_cost(deck, &deck[index], &st.battlefield);
            // Additional life costs gate the same way hand casts gate
            // (the cast would pay the life and could drive life
            // negative).
            if st.life <= deck[index].riders.additional_cost_life as i32 {
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
                engines,
                true,
            );
            st.replay_casts += 1;
            super::game::milestone_for_turn(st, turn as u32).graveyard_casts += 1;
            mana_spent[turn - 1] += spent as f64;
            if deck[index].is_instant_or_sorcery {
                st.exile.push(index);
            }
            for (uid, card_index) in replay_ets {
                if let Some(position) = st.battlefield.iter().position(|perm| perm.uid == uid) {
                    fire_on_enter(deck, st, position, turn as u32, false);
                    for ability in deck[card_index].abilities() {
                        if let Some(draws) =
                            engine_effect_or_draws(&ability.trigger, &ability.effect)
                        {
                            engines.push((uid, draws));
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
fn cycle_unusable_cards(deck: &SimDeck, st: &mut GameState, pool: &mut Pool, turn: u32) -> bool {
    let mut cycled = false;
    let candidates = st.hand.clone();
    for index in candidates {
        let card = &deck[index];
        if card.riders.cycling_cost.is_none() && card.riders.cycling_life == 0 {
            continue;
        }
        let cost = card
            .riders
            .cycling_cost
            .as_ref()
            .cloned()
            .unwrap_or_default();
        if !st.hand.contains(&index)
            || st.life <= card.riders.cycling_life as i32
            || !payable(&cost, pool)
            || !pips_ok(&cost, pool)
        {
            continue;
        }
        pay_cost(&cost, pool);
        st.life -= card.riders.cycling_life as i32;
        st.life_paid += card.riders.cycling_life;
        let funded_draw =
            u32::from(card.riders.cycling_life > 0 && card.riders.landcycling_type.is_none());
        st.life_funded_draws += funded_draw;
        super::game::milestone_for_turn(st, turn).life_funded_draws += funded_draw;
        st.hand.retain(|held| *held != index);
        move_to_graveyard(deck, st, index, turn, CardZone::Hand);
        if let Some(color) = card.riders.landcycling_type {
            let color_index = "WUBRG".find(color);
            let target = color_index.and_then(|color_index| {
                st.library.iter().rposition(|candidate| {
                    deck[*candidate].role == Role::Land && deck[*candidate].land_types[color_index]
                })
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
fn resolve_reveal_rule(
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
fn select_alternative_cost_cards(deck: &SimDeck, st: &GameState, spell: CardIdx) -> Vec<CardIdx> {
    let Some(cost) = deck[spell].riders.alternative_cast_cost else {
        return Vec::new();
    };
    let mut candidates: Vec<CardIdx> = st
        .hand
        .iter()
        .copied()
        .filter(|index| *index != spell)
        .filter(|index| {
            cost.filter.color.is_none_or(|color| {
                "WUBRG"
                    .find(color)
                    .is_some_and(|position| deck[*index].colors[position])
            })
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

/// Resolve one affordable cast: pay the cost, push the permanent, and
/// fire every on-cast rider (kicker, additional costs, ETB counters,
/// rituals, draws, mills, scry, wheels, X conversion, drain, tokens,
/// cascade). Pure aside from the game state it mutates.
#[allow(clippy::too_many_arguments)]
fn resolve_cast(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: usize,
    idx: CardIdx,
    eff: &Cost,
    cast_ids: &mut Vec<CardIdx>,
    cast_ets: &mut Vec<(u32, CardIdx)>,
    spent_total: &mut u32,
    engines: &mut Vec<(u32, u32)>,
    resolve_cascade: bool,
) {
    let card = &deck[idx];
    let alternative_cards = select_alternative_cost_cards(deck, st, idx);
    if alternative_cards.is_empty() {
        if let Some(restriction) = cast_restriction(card) {
            pay_restricted_cost(eff, pool, restriction);
        } else {
            pay_cost(eff, pool);
        }
        *spent_total += eff.total();
    } else {
        #[cfg(test)]
        st.alternate_casts.push(idx);
    }
    cast_ids.push(idx);
    if let Some(pos) = st.hand.iter().position(|held| *held == idx) {
        st.hand.remove(pos);
    }
    // Kicker: an optional extra cost paid from leftover mana. Best
    // case the goldfish kicks when the pool covers it (the colored
    // pips are paid from fixed and flexible sources like any cost).
    // The rider bumps the drain/damage amount (the modeled kicker
    // payoff).
    let kicked = if let Some(k) = card.riders.kicker.clone()
        && usable_for_noncreature(pool) >= k.total()
        && pips_ok(&k, pool)
    {
        pay_cost(&k, pool);
        *spent_total += k.total();
        true
    } else {
        false
    };
    // The permanent pushed below; battlefield scans skip it by uid
    // (its per-cast engine already fired for the casts so far).
    let cast_perm_uid = super::game::take_uid(st);
    // Additional discard, sacrifice, and life costs passed the cast gate
    // before mana was paid. The cast card already left the hand, so wheels
    // cannot discard the resolving spell.
    for _ in 0..card.riders.additional_cost_discards {
        if let Some(pos) = st.hand.iter().position(|i| *i != idx) {
            let discarded = st.hand.remove(pos);
            move_to_graveyard(deck, st, discarded, turn as u32, CardZone::Hand);
        }
    }
    let mut alternative_life = 0;
    for exiled in alternative_cards {
        if let Some(position) = st.hand.iter().position(|held| *held == exiled) {
            st.hand.remove(position);
            st.exile.push(exiled);
            alternative_life += deck[exiled].mana_value;
        }
    }
    if alternative_life > 0
        && card.riders.alternative_cast_cost.is_some_and(|cost| {
            cost.payoff == super::model::AlternativeCostPayoff::GainLifeEqualToExiledManaValue
        })
    {
        st.life += alternative_life as i32;
        st.life_gained += alternative_life;
    }
    let searched_sacrifice = if card.riders.search_after_sacrifice {
        st.battlefield
            .iter()
            .filter(|perm| perm.card.deck_idx().is_some() && card_of(deck, perm).is_creature)
            .find_map(|perm| {
                let mana_value = card_of(deck, perm).mana_value;
                st.library
                    .iter()
                    .map(|index| &deck[*index])
                    .any(|candidate| {
                        candidate.is_creature && candidate.mana_value == mana_value + 1
                    })
                    .then_some((perm.uid, mana_value))
            })
    } else {
        None
    };
    for _ in 0..card.riders.additional_cost_bodies {
        if let Some((uid, _)) = searched_sacrifice {
            super::game_effects::resolve_sacrifice_uid(deck, st, turn as u32, uid);
        } else {
            super::game_effects::resolve_sacrifice(deck, st, turn as u32, u32::MAX);
        }
    }
    if let Some((_, sacrificed_mv)) = searched_sacrifice
        && let Some(pos) = st.library.iter().rposition(|candidate| {
            deck[*candidate].is_creature && deck[*candidate].mana_value == sacrificed_mv + 1
        })
    {
        let target = st.library.remove(pos);
        let target_card = &deck[target];
        let uid = super::game::take_uid(st);
        st.battlefield_seen.entry(target).or_insert(turn as u32);
        let mut entry = new_perm_with(uid, deck, target, turn as u32, false);
        entry.sick = !target_card.has_haste;
        entry.counters = if target_card.enter_counters == super::parse_land::X_ENTRY_COUNTERS {
            1
        } else {
            target_card.enter_counters + 1
        };
        st.battlefield.push(entry);
        cast_ets.push((uid, target));
    }
    if card.riders.additional_cost_life > 0 {
        // Life the goldfish pays itself is not damage dealt; keep it
        // out of the lethal census.
        st.life_paid += card.riders.additional_cost_life;
        st.life -= card.riders.additional_cost_life as i32;
        st.life_funded_draws += card.riders.draws_on_cast;
        super::game::milestone_for_turn(st, turn as u32).life_funded_draws +=
            card.riders.draws_on_cast;
    }
    if card.riders.grants_flashback {
        st.flashback_permissions.extend(
            st.graveyard
                .iter()
                .copied()
                .filter(|index| deck[*index].is_instant_or_sorcery),
        );
    }
    if card.riders.graveyard_creature_exchange {
        let mut returned = Vec::new();
        let mut remaining = Vec::new();
        for index in st.graveyard.drain(..) {
            if deck[index].is_creature {
                st.exile.push(index);
                returned.push(index);
            } else {
                remaining.push(index);
            }
        }
        st.graveyard = remaining;
        // Death triggers on the surviving board can create new creature
        // tokens each round, so the exchange can never drain the board
        // by itself. The shared pass cap keeps the cast phase finite.
        for _ in 0..super::model::MAX_LOOP_PASSES {
            if !super::game_effects::has_sacrifice_body(deck, st) {
                break;
            }
            super::game_effects::resolve_sacrifice_body(deck, st, turn as u32, u32::MAX);
        }
        for index in returned {
            let returned_card = &deck[index];
            st.battlefield_seen.entry(index).or_insert(turn as u32);
            let uid = super::game::take_uid(st);
            let mut entry = new_perm_with(uid, deck, index, turn as u32, false);
            entry.sick = !returned_card.has_haste;
            entry.counters = if returned_card.enter_counters == super::parse_land::X_ENTRY_COUNTERS
            {
                0
            } else {
                returned_card.enter_counters
            };
            st.battlefield.push(entry);
            cast_ets.push((uid, index));
        }
    }
    // Producers join the battlefield: rocks tap at once, creatures
    // from next turn (summoning sickness). Vehicles and spacecraft
    // join as artifacts. "Enters with X charge counters" cards
    // convert the cast's leftover pool into counters (Astral
    // Cornucopia class).
    let entry_counters = if card.enter_counters == super::parse_land::X_ENTRY_COUNTERS {
        let x = pool.total();
        pool.colorless = 0;
        pool.flexible = 0;
        pool.fixed = [0; 5];
        wipe_restricted_buckets(pool);
        *spent_total += x;
        x
    } else {
        card.enter_counters
    };
    if !card.is_instant_or_sorcery {
        st.battlefield_seen.entry(idx).or_insert(turn as u32);
        let mut entry = new_perm_with(cast_perm_uid, deck, idx, turn as u32, false);
        entry.sick = card.is_creature && !card.has_haste;
        entry.counters = entry_counters;
        st.battlefield.push(entry);
        let pw_pos = st.battlefield.len() - 1;
        register_loyalty_token_engines(deck, st, pw_pos, engines);
        cast_ets.push((cast_perm_uid, idx));
    }
    // Planeswalker +1 token engines register at first cast: a
    // loyalty-gain activation that creates tokens is a repeatable
    // once-per-turn engine (Liliana-class token fuel).
    // One-shot mana (rituals) joins this turn's pool only.
    if let Some(y) = &card.riders.mana_on_cast {
        add_yield_turns_empty_board(y, pool, turn as u32);
    }
    // One-shot draws on cast (cantrips, Divination).
    for _ in 0..card.riders.draws_on_cast {
        draw_one(deck, st, turn as u32);
    }
    // One-shot mill on cast (plain "mill N" spells).
    for _ in 0..card.riders.mills_on_enter {
        mill_library_card(deck, st, turn as u32, card.mills_opponent);
    }
    // Scry/surveil on cast: awareness only; surveil mills the
    // scry'd cards to the graveyard.
    st.awareness_cards += card.riders.scry_on_cast;
    for _ in 0..card.riders.surveils_on_cast {
        mill_library_card(deck, st, turn as u32, false);
    }
    // Schedule the next turn slot as an extra turn.
    if card.riders.extra_turns_on_cast {
        st.extra_turns_queued += 1;
    }
    // One-shot drain spells (burn at a player, "each opponent
    // loses N life"). Player-targeted damage resolves ×3 (three
    // opponents in the commander family) or ×1 constructed;
    // creature-target burn never got here (Removal). A paid kicker
    // bumps the drain amount.
    if card.riders.drain_on_cast > 0 {
        let rider =
            card.riders.drain_on_cast + u32::from(kicked) * card.riders.drain_on_cast.max(1);
        st.drained += rider * drain_mult_in(deck);
    }
    if card.riders.life_gain_on_cast > 0 {
        st.life += card.riders.life_gain_on_cast as i32;
        st.life_gained += card.riders.life_gain_on_cast;
    }
    if let Some(rule) = card.riders.reveal_rule {
        resolve_reveal_rule(deck, st, rule, turn as u32);
    }
    // One-shot token spells ("Create four 1/1 Soldier creature
    // tokens"): the cast resolves the creation.
    if card.riders.tokens_on_cast > 0 {
        apply_effect_at(
            deck,
            &Effect::Tokens(card.riders.tokens_on_cast.min(8)),
            st,
            turn as u32,
            false,
            Some(idx),
        );
    }
    // One-shot wheel spells ("each player discards, then draws").
    // The just-cast wheel is still in hand (removal is deferred), so
    // the skip variant keeps it out of the graveyard log.
    if card.riders.wheel_on_cast {
        apply_effect_at(deck, &Effect::WheelSkip(idx), st, turn as u32, false, None);
    }
    // X-cost spells pay the leftover pool as X and scale the effect
    // (best case: X = everything floatable). The generic {X} already
    // paid 1; the rest of the pool converts. Counters cards skip
    // this branch: the entry-counter block above already converted
    // the pool to counters and recorded the spend. The X value is
    // capped at 8 for token counts; the recorded spend still shows
    // the full floatable pool, so X-heavy decks read as near-zero
    // unused mana by design.
    if let Some(class) = card.riders.x_class
        && class != super::model::XClass::Counters
    {
        let x = pool.total().max(1);
        pool.colorless = 0;
        pool.flexible = 0;
        pool.fixed = [0; 5];
        wipe_restricted_buckets(pool);
        // Spent accounting is single-counted: the entered X counters
        // ARE the paid X; `spent_total` records it once here.
        *spent_total += x;
        match class {
            super::model::XClass::Drain => {
                st.drained += x * drain_mult_in(deck);
            }
            super::model::XClass::Draw => {
                for _ in 0..x {
                    draw_one(deck, st, turn as u32);
                }
            }
            super::model::XClass::Mill => {
                let mill_opp = card.mills_opponent;
                for _ in 0..x {
                    mill_library_card(deck, st, turn as u32, mill_opp);
                }
            }
            super::model::XClass::Tokens => {
                apply_effect_at(
                    deck,
                    &Effect::Tokens(x.min(8)),
                    st,
                    turn as u32,
                    false,
                    Some(idx),
                );
            }
            super::model::XClass::RevealPermanents => {
                // Best case the top X cards all become permanents on
                // the battlefield (capped at 8). They enter through a
                // reveal effect, not a cast resolution: their
                // OnEnter triggers do not fire (the ETB pass below
                // excludes non-cast entries).
                for _ in 0..x.min(8) {
                    if let Some(i) = st.library.pop() {
                        st.seen += 1;
                        st.awareness_cards += 1;
                        st.battlefield_seen.entry(i).or_insert(turn as u32);
                        let uid = take_uid(st);
                        st.battlefield
                            .push(new_perm_with(uid, deck, i, turn as u32, false));
                    }
                }
            }
            _ => {}
        }
    }
    // Prowess census: noncreature spells cast this turn. The same
    // counter feeds cast-count engines (storm mana, per-cast drains).
    if !card.is_creature && card.role != Role::Land {
        st.prowess_casts += 1;
    }
    // Cast-count engines: "add {N} for each spell cast this turn"
    // joins the pool now (Vivi-class mana engines, best case). The
    // just-cast host fires for spells before it; hosts already on
    // the battlefield fire for every spell cast this turn.
    if let Some(y) = &card.riders.mana_per_cast {
        for _ in 0..st.prowess_casts {
            add_yield_turns_empty_board(y, pool, turn as u32);
        }
    }
    for p in st.battlefield.iter() {
        if p.uid == cast_perm_uid {
            continue;
        }
        if let Some(y) = &card_of(deck, p).riders.mana_per_cast {
            add_yield_turns_empty_board(y, pool, turn as u32);
        }
    }
    // OnCastSpell engines fire per spell cast. Draw engines draw;
    // loot fills the graveyard; per-cast drains resolve at the
    // family multiplier. Snapshot the uids: effects fired in the loop
    // (loot) can push or pull from the board.
    let engine_uids: Vec<u32> = st.battlefield.iter().map(|p| p.uid).collect();
    for uid in engine_uids {
        let Some(perm) = st.battlefield.iter().find(|p| p.uid == uid) else {
            continue;
        };
        let perm = perm.clone();
        for ability in card_of(deck, &perm).abilities() {
            if ability.trigger != AbilityTiming::OnCastSpell {
                continue;
            }
            match &ability.effect {
                Effect::Draw(n) => {
                    for _ in 0..*n {
                        draw_one(deck, st, turn as u32);
                    }
                }
                Effect::Loot(n) => {
                    apply_effect_at(deck, &Effect::Loot(*n), st, turn as u32, false, None);
                }
                Effect::Drain(n) => {
                    st.drained += n * drain_mult_in(deck);
                }
                _ => {}
            }
        }
    }
    // Counter injection targets the highest-threshold unfilled
    // station permanent (Drill Too Deep).
    if card.riders.counters_on_cast > 0
        && let Some(perm) = st
            .battlefield
            .iter_mut()
            .filter(|p| card_of(deck, p).is_station_card)
            .max_by_key(|p| card_of(deck, p).animate_at().unwrap_or(0))
    {
        perm.counters += card.riders.counters_on_cast;
    }
    // Cascade: reveal in library order and free-cast the first nonland
    // card with lower printed mana value. No cascade chaining. The free
    // cast counts fully: ETB triggers fire, per-cast engines fire,
    // and the card leaves the library into the seen census.
    if resolve_cascade && card.has_cascade {
        let mut exposed = Vec::new();
        let mut hit = None;
        while let Some(index) = st.library.pop() {
            st.seen += 1;
            st.awareness_cards += 1;
            let candidate = &deck[index];
            if candidate.role != Role::Land && candidate.mana_value < card.mana_value {
                hit = Some(index);
                break;
            }
            exposed.push(index);
        }
        // The exposed order came from the seeded library shuffle (the
        // first miss was revealed first, closest to the hit). Reinsert
        // in reverse so the first-revealed card ends up deepest and the
        // reveal order survives at the library bottom.
        for index in exposed.iter().rev() {
            st.library.insert(0, *index);
        }
        if let Some(free_idx) = hit {
            if !deck[free_idx].is_instant_or_sorcery {
                super::game::milestone_for_turn(st, turn as u32).free_cast_permanents_entered += 1;
            }
            resolve_cast(
                deck,
                st,
                pool,
                turn,
                free_idx,
                &Cost::default(),
                cast_ids,
                cast_ets,
                spent_total,
                engines,
                false,
            );
        }
    }
}

/// Zero every restricted bucket (an X-cost or counters spell spends the
/// whole pool; restricted mana cannot outlive the conversion).
pub(super) fn wipe_restricted_buckets(pool: &mut Pool) {
    pool.creature_only = 0;
    pool.legendary_only = 0;
    pool.artifact_only = 0;
    pool.instant_sorcery_only = 0;
}

/// The upkeep-engine payload of an ability: the draw count for
/// executor-run draw engines, zero for the executor-run shapes
/// (mill, reanimation, drain, tokens), None when not an upkeep engine.
fn engine_effect_or_draws(trigger: &AbilityTiming, effect: &Effect) -> Option<u32> {
    match (trigger, effect) {
        (AbilityTiming::OnUpkeep, Effect::Draw(n)) => Some(*n),
        (
            AbilityTiming::OnUpkeep,
            Effect::Mill(_)
            | Effect::ReturnFromGraveyard { .. }
            | Effect::Drain(_)
            | Effect::Tokens(_),
        ) => Some(0),
        _ => None,
    }
}

/// Probe entry for the graveyard-cast pass (test only).
#[cfg(test)]
pub(crate) fn cast_graveyard_spells_probe(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: usize,
    mana_spent: &mut [f64],
    engines: &mut Vec<(u32, u32)>,
) {
    cast_graveyard_spells(deck, st, pool, turn, mana_spent, engines);
}
