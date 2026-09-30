//! SimEffect execution and the tap-budget pass for the goldfish game loop,
//! split from game.rs to keep files small.

/// Keyword-action and counter effects (amass, the Ring, empower Jace,
/// explore, connive, mobilize, proliferate).
#[path = "game_effects/keyword_actions.rs"]
mod keyword_actions;

pub(in crate::deck::simulator) use keyword_actions::{
    amass, connive, empower_jace, explore, mobilize, resolve_proliferate, ring_tempts,
};

/// Activation selection and resolution (spend-leftover, sacrifice outlets).
#[path = "game_effects/activation.rs"]
pub(in crate::deck::simulator) mod activation;

pub(in crate::deck::simulator) use activation::{
    enforce_legend_rule, fire_death_triggers, has_sacrificable_creature,
    resolve_creature_sacrifice, resolve_deck_creature_sacrifice, resolve_sacrifice_uid,
    spend_leftover,
};

use super::game::{
    CardRef, GameState, ManaPool, Permanent, TOKEN_CREATURE_POWER, card_of, new_perm_with,
    new_token_perm, take_uid,
};
use super::game_mana::{add_yield, effective_min_cost, pay_cost, payable};
use super::model::{
    CardIdx, LibraryGraveyardTrigger, Role, SearchCardType, SearchDestination, SearchSpec, SimDeck,
    SimEffect, SimTrigger,
};

/// Resolve one draw, applying the best available dredge replacement first.
pub(super) fn draw_one(deck: &SimDeck, st: &mut GameState, turn: u32) -> bool {
    let dredger = st
        .graveyard
        .iter()
        .enumerate()
        .filter_map(|(position, index)| {
            let amount = deck[*index].dredge?;
            (st.library.len() >= amount as usize).then_some((position, *index, amount))
        })
        .max_by_key(|(_, _, amount)| *amount);
    if let Some((position, index, amount)) = dredger {
        super::game::milestone_for_turn(st, turn).dredge_uses += 1;
        st.graveyard.remove(position);
        for _ in 0..amount {
            mill_library_card(deck, st, turn, false);
        }
        st.hand.push(index);
        return true;
    }
    let Some(index) = st.library.pop() else {
        return false;
    };
    st.hand.push(index);
    st.seen += 1;
    st.awareness_cards += 1;
    true
}

/// Move one library card to a graveyard and resolve its library-mill trigger.
///
/// Opponent mills touch no player zone: the opponent has no board in
/// the goldfish, so the move stays a census bump (`milled_opp`) and
/// the player's library and graveyard are untouched.
pub(super) fn mill_library_card(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    mill_opponent: bool,
) -> bool {
    if mill_opponent {
        st.milled_opp += 1;
        return true;
    }
    let Some(index) = st.library.pop() else {
        return false;
    };
    st.seen += 1;
    st.awareness_cards += 1;
    st.milled_self += 1;
    move_to_graveyard(deck, st, index, turn, CardZone::Library);
    true
}

/// Zone a card leaves when it moves into the graveyard.
#[derive(Clone, Copy)]
pub(super) enum CardZone {
    /// The player's library.
    Library,
    /// A player's hand.
    Hand,
    /// The battlefield.
    Battlefield,
    /// The stack after a spell resolves.
    Stack,
}

/// Record a zone move into the graveyard and resolve library-only triggers.
pub(super) fn move_to_graveyard(
    deck: &SimDeck,
    st: &mut GameState,
    index: CardIdx,
    turn: u32,
    source: CardZone,
) {
    st.graveyard_seen.entry(index).or_insert(turn);
    st.graveyard.push(index);
    if matches!(source, CardZone::Library) {
        resolve_library_graveyard_trigger(deck, st, index, turn);
    }
}

/// Resolve the supported Oracle trigger for a library-to-graveyard move.
fn resolve_library_graveyard_trigger(
    deck: &SimDeck,
    st: &mut GameState,
    index: CardIdx,
    turn: u32,
) {
    match deck[index].library_graveyard_trigger {
        Some(LibraryGraveyardTrigger::ReturnToBattlefield) => {
            st.graveyard.retain(|card| *card != index);
            st.battlefield_seen.entry(index).or_insert(turn);
            let uid = take_uid(st);
            let sim = &deck[index];
            st.battlefield
                .push(new_perm_with(uid, deck, index, turn, false));
            let _ = sim;
        }
        Some(LibraryGraveyardTrigger::DrainAndGain(amount)) => {
            st.graveyard.retain(|card| *card != index);
            st.exile.push(index);
            st.opponent_life_lost += amount * deck.format.life_loss_mult();
            st.life_gained += amount;
            st.life += amount as i32;
        }
        None => {}
    }
}

/// The pending spell stays in hand while other cards are discarded.
pub(super) struct CastCardToSkip {
    /// The cast spell still in the hand, skipped by the discard pass.
    pub(super) skip: CardIdx,
}

/// Discard the hand except for the pending spell, then draw seven cards.
pub(super) fn resolve_discard_hand_then_draw_seven(
    deck: &SimDeck,
    st: &mut GameState,
    skip: CastCardToSkip,
    turn: u32,
) {
    let hand = std::mem::take(&mut st.hand);
    for i in hand {
        if i == skip.skip {
            st.hand.push(i);
            continue;
        }
        move_to_graveyard(deck, st, i, turn, CardZone::Hand);
    }
    for _ in 0..7 {
        draw_one(deck, st, turn);
    }
}

/// The full variant: `source` is the battlefield card whose
/// effect resolves (Treasure flags read from that card).
pub(crate) fn apply_effect_at(
    deck: &SimDeck,
    effect: &SimEffect,
    st: &mut GameState,
    turn: u32,
    mill_opp: bool,
    source: Option<CardIdx>,
) {
    match effect {
        SimEffect::Draw(n) => {
            for _ in 0..*n {
                draw_one(deck, st, turn);
            }
        }
        SimEffect::Search(spec) => search_library(deck, st, *spec, turn),
        SimEffect::ExtraLand => {
            // "You may play an additional land this turn": grant one
            // extra drop for the rest of the turn. The land phase spends
            // it; no card is drawn.
            st.extra_land_drops_this_turn += 1;
        }
        SimEffect::ExileThenReturnSource => {}
        SimEffect::Monarch => {
            st.is_monarch = true;
        }
        SimEffect::Mill(n) => {
            for _ in 0..*n {
                mill_library_card(deck, st, turn, mill_opp);
            }
        }
        SimEffect::Scry(n) => {
            st.awareness_cards += *n;
        }
        SimEffect::Surveil(n) => {
            for _ in 0..*n {
                mill_library_card(deck, st, turn, false);
            }
        }
        SimEffect::LoseLife { amount, scope } => {
            // The scope decides the table multiplier (CR 119.3): "each
            // opponent" hits the opponent count, "target player" hits
            // exactly one, "each player" hits the table plus the player,
            // and "you lose" costs only the player.
            st.opponent_life_lost += *amount * scope.table_multiplier(deck.format);
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
        SimEffect::GainLife(n) => {
            st.life += *n as i32;
            st.life_gained += *n;
        }
        SimEffect::ExtraTurn => {
            st.extra_turns_queued += 1;
        }
        SimEffect::ReturnFromGraveyard { to_hand, count } => {
            for _ in 0..*count {
                let Some(i) = st.graveyard.pop() else {
                    break;
                };
                if *to_hand {
                    // Cards already seen (drawn, milled, discarded) keep
                    // their seen count; a return does not re-see them.
                    st.hand.push(i);
                } else if deck[i].is_creature {
                    // Returns as a body once; the card leaves the log.
                    st.battlefield_seen.entry(i).or_insert(turn);
                    let uid = take_uid(st);
                    st.battlefield.push(new_perm_with(uid, deck, i, turn, true));
                } else {
                    st.hand.push(i);
                }
            }
        }
        SimEffect::DiscardHandThenDrawSeven => {
            let discarded = std::mem::take(&mut st.hand);
            for i in discarded {
                move_to_graveyard(deck, st, i, turn, CardZone::Hand);
            }
            for _ in 0..7 {
                draw_one(deck, st, turn);
            }
        }
        SimEffect::DrawThenDiscard(n) => {
            for _ in 0..*n {
                draw_one(deck, st, turn);
                // Discard the oldest hand card into the graveyard.
                // The front is the oldest; fresh draws sit at the back
                // and survive the loot. A pending cast's hand entry can
                // be eaten here; the cast pass resolves positions by
                // card identity, so the stale slot never casts a
                // different card.
                if !st.hand.is_empty() {
                    let discarded = st.hand.remove(0);
                    move_to_graveyard(deck, st, discarded, turn, CardZone::Hand);
                }
            }
        }
        SimEffect::Tokens(n) => {
            // Token bodies join as small station/crew fuel. A count of
            // 0 creates nothing (a caller that cannot parse the amount
            // must not turn it into one body); the cap bounds go-wide
            // boards.
            for _ in 0..(*n).min(8) {
                let uid = take_uid(st);
                st.battlefield.push(new_token_perm(uid, turn));
            }
        }
        SimEffect::Treasures(count) => {
            st.treasure_bank += count;
        }
        SimEffect::TokensEachOpponent { per_opponent } => {
            // "For each opponent, create N tokens" (CR 102.x opponent
            // count): N per opponent, so the table sees N × opponents.
            let total = per_opponent * deck.format.life_loss_mult();
            apply_effect_at(
                deck,
                &SimEffect::Tokens(total.min(8)),
                st,
                turn,
                mill_opp,
                source,
            );
        }
        SimEffect::Energy(n) => {
            st.player_counters.energy += n;
        }
        SimEffect::Amass(amount) => {
            amass(deck, st, turn, *amount);
        }
        SimEffect::RingTempts => {
            ring_tempts(deck, st, turn);
        }
        SimEffect::EmpowerJace(amount) => {
            empower_jace(deck, st, turn, *amount);
        }
        SimEffect::Explore => {
            explore(deck, st, turn);
        }
        SimEffect::Connive(count) => {
            connive(deck, st, turn, *count);
        }
        SimEffect::Mobilize(amount) => {
            mobilize(deck, st, turn, *amount);
        }
        SimEffect::Afterlife(amount) => {
            // Death-trigger tokens: plain 2/2 bodies in the goldfish body
            // model, joining the board (the caller fires this from the
            // death path, after the source left).
            for _ in 0..(*amount).min(8) {
                let uid = take_uid(st);
                st.battlefield.push(new_token_perm(uid, turn));
            }
        }
        SimEffect::AddBurdenCounter => {
            // The One Ring class: add one burden counter to the source,
            // then draw a card per counter on it.
            if let Some(perm) = source.and_then(|idx| {
                st.battlefield
                    .iter_mut()
                    .find(|p| p.card.deck_idx() == Some(idx))
            }) {
                perm.counters.burden += 1;
                let draws = perm.counters.burden;
                for _ in 0..draws.min(8) {
                    draw_one(deck, st, turn);
                }
            }
        }
        SimEffect::BurdenLifeLoss => {
            // Upkeep life loss per burden counter on the source.
            if let Some(idx) = source
                && let Some(perm) = st
                    .battlefield
                    .iter()
                    .find(|p| p.card.deck_idx() == Some(idx))
            {
                let loss = perm.counters.burden;
                st.life -= loss as i32;
                st.life_paid += loss;
            }
        }
        SimEffect::Proliferate => resolve_proliferate(st),
        _ => {}
    }
}

/// Move the first eligible library card to the requested search destination.
fn search_library(deck: &SimDeck, st: &mut GameState, spec: SearchSpec, turn: u32) {
    if let Some(limit) = spec.top_count {
        let start = st.library.len().saturating_sub(limit);
        let mut revealed = st.library.split_off(start);
        st.seen += revealed.len() as u32;
        st.awareness_cards += revealed.len() as u32;
        let found = revealed
            .iter()
            .rposition(|index| search_matches(deck, *index, &spec))
            .map(|position| revealed.remove(position));
        st.library.extend(revealed);
        if let Some(index) = found {
            place_search_result(deck, st, index, spec.destination, turn);
        }
        return;
    }
    let position = st
        .library
        .iter()
        .rposition(|index| search_matches(deck, *index, &spec));
    if spec.optional && position.is_none() {
        return;
    }
    let Some(pos) = position else { return };
    let index = st.library.remove(pos);
    st.seen += 1;
    place_search_result(deck, st, index, spec.destination, turn);
}

/// Check one card against a parsed search restriction.
fn search_matches(deck: &SimDeck, index: CardIdx, spec: &SearchSpec) -> bool {
    let card = &deck[index];
    spec.card_type.is_none_or(|kind| match kind {
        SearchCardType::Creature => card.is_creature,
        SearchCardType::Land => card.is_land,
        SearchCardType::BasicLand => card.is_land && card.is_basic_land,
        SearchCardType::Artifact => card.is_artifact,
        SearchCardType::Enchantment => card.is_enchantment,
        SearchCardType::ArtifactOrEnchantment => card.is_artifact || card.is_enchantment,
        SearchCardType::InstantSorcery => card.is_instant_or_sorcery,
        SearchCardType::Planeswalker => card.is_planeswalker,
        SearchCardType::Permanent => !card.is_instant_or_sorcery,
    }) && (!spec.non_human || !card.is_human)
        && spec.color.is_none_or(|color| card.colors[color.index()])
        && (!spec.colorless || card.colors.iter().all(|color| !color))
        && spec.mana_value.is_none_or(|value| card.mana_value == value)
        && spec
            .max_mana_value
            .is_none_or(|value| card.mana_value <= value)
        && spec
            .min_mana_value
            .is_none_or(|value| card.mana_value >= value)
}

/// Move one search result to its requested zone and fire entry effects.
fn place_search_result(
    deck: &SimDeck,
    st: &mut GameState,
    index: CardIdx,
    destination: SearchDestination,
    turn: u32,
) {
    match destination {
        SearchDestination::Hand => {
            st.hand.push(index);
        }
        SearchDestination::LibraryTop => st.library.push(index),
        SearchDestination::Exile => st.exile.push(index),
        SearchDestination::Battlefield | SearchDestination::BattlefieldTapped => {
            st.battlefield_seen.entry(index).or_insert(turn);
            let uid = take_uid(st);
            let tapped = destination == SearchDestination::BattlefieldTapped;
            st.battlefield
                .push(new_perm_with(uid, deck, index, turn, tapped));
            let position = st.battlefield.len() - 1;
            super::game::fire_on_enter(deck, st, position, turn, false);
            if deck[index].is_land {
                super::game::fire_triggers(deck, st, SimTrigger::LandEnters, turn);
            }
        }
    }
}

/// The tap-budget pass: mana only while casting still needs mana, then
/// station, then crew, then equip (suit up a body so its buff counts in
/// combat).
pub(super) fn tap_budget(
    deck: &SimDeck,
    battlefield: &mut [Permanent],
    pool: &mut ManaPool,
    hand: &[CardIdx],
) {
    // Remaining demand: the cheapest uncast spell still in hand.
    let cheapest: Option<u32> = hand
        .iter()
        .filter(|i| deck[**i].role != Role::Land)
        .map(|i| effective_min_cost(deck, &deck[*i], battlefield).total())
        .min();

    // Creatures and crewed vehicles able to tap this turn. Summoning
    // sickness blocks their own tap-symbol abilities (CR 302.6), so the
    // mana-tap list is unsick only.
    let tappable: Vec<usize> = battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            !p.tapped
                && !p.summoning_sick
                && p.card.deck_idx().is_some()
                && !card_of(deck, p).is_station_card
                && super::game::is_creature_permanent(deck, p)
        })
        .map(|(i, _)| i)
        .collect();
    // Stationing and crewing tap other creatures, not the body's own
    // tap-symbol ability, so summoning-sick bodies qualify (CR 302.6,
    // 702.122a).
    let stationable: Vec<usize> = battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            !p.tapped
                && p.card.deck_idx().is_some()
                && !card_of(deck, p).is_station_card
                && super::game::is_creature_permanent(deck, p)
        })
        .map(|(i, _)| i)
        .collect();

    // 7a Mana taps: only while the cheapest spell still needs mana.
    if let Some(need) = cheapest {
        for &bi in &tappable {
            if super::game_mana::usable_for_noncreature(pool) >= need {
                break;
            }
            let card = card_of(deck, &battlefield[bi]);
            if card
                .unlocked_abilities(battlefield[bi].counters.charge)
                .any(|ability| ability.kind == super::model::SimAbilityKind::ManaActivated)
            {
                continue;
            }
            if let Some(y) = &card.tap {
                add_yield(y, pool);
                battlefield[bi].tapped = true;
            }
        }
    }

    // 7b Station: tap remaining bodies into the highest-threshold unfilled
    // spacecraft/planet, one at a time, counters = body power.
    for &bi in &stationable {
        if battlefield[bi].tapped {
            continue;
        }
        let target = battlefield
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                *i != bi
                    && card_of(deck, p).is_station_card
                    && card_of(deck, p)
                        .animate_at()
                        .is_some_and(|at| p.counters.charge < at)
            })
            .max_by_key(|(_, p)| card_of(deck, p).animate_at().unwrap_or(0))
            .map(|(i, _)| i);
        if let Some(ti) = target {
            battlefield[ti].counters.charge += creature_power(&battlefield[bi], deck);
            battlefield[bi].tapped = true;
        }
    }

    // 7c Crew: tap bodies into untapped Vehicles (total power ≥ crew N).
    for vi in 0..battlefield.len() {
        if battlefield[vi].tapped || card_of(deck, &battlefield[vi]).crew.is_none() {
            continue;
        }
        let crew_n = card_of(deck, &battlefield[vi]).crew.unwrap_or(1);
        let creatures_needed = crew_n.div_ceil(TOKEN_CREATURE_POWER);
        // Check total power first: bodies only tap when the crew
        // succeeds (a failed crew leaves them ready for mana/station).
        // Candidates come strongest first: the crew succeeds whenever
        // the board's best bodies can cover the cost.
        let mut candidates: Vec<usize> = (0..battlefield.len())
            .filter(|i| {
                *i != vi
                    && !battlefield[*i].tapped
                    && battlefield[*i].card.deck_idx().is_some()
                    && super::game::is_creature_permanent(deck, &battlefield[*i])
            })
            .collect();
        candidates.sort_by_key(|i| std::cmp::Reverse(creature_power(&battlefield[*i], deck)));
        candidates.truncate(creatures_needed as usize);
        let power: u32 = candidates
            .iter()
            .map(|i| creature_power(&battlefield[*i], deck))
            .sum();
        if power >= crew_n {
            for i in candidates {
                battlefield[i].tapped = true;
            }
            battlefield[vi].crewed = true;
        }
    }

    // 7d Saddle: tap bodies into untapped Mounts with a saddled payoff
    // (total power ≥ saddle N, CR 702.171a). Saddling leaves the mount
    // untapped; it only switches on "while saddled" payoffs.
    for si in 0..battlefield.len() {
        if battlefield[si].saddled || battlefield[si].tapped {
            continue;
        }
        let card = card_of(deck, &battlefield[si]);
        let Some(saddle_n) = card.keyword_abilities.saddle else {
            continue;
        };
        if card.flags.saddled_buff.is_none() {
            // No modeled saddled payoff: the goldfish does not pay the
            // cost for nothing.
            continue;
        }
        let mut candidates: Vec<usize> = (0..battlefield.len())
            .filter(|i| {
                *i != si
                    && !battlefield[*i].tapped
                    && battlefield[*i].card.deck_idx().is_some()
                    && super::game::is_creature_permanent(deck, &battlefield[*i])
            })
            .collect();
        candidates.sort_by_key(|i| std::cmp::Reverse(creature_power(&battlefield[*i], deck)));
        let mut chosen = Vec::new();
        let mut power = 0u32;
        for i in candidates {
            if power >= saddle_n {
                break;
            }
            power += creature_power(&battlefield[i], deck);
            chosen.push(i);
        }
        if power >= saddle_n {
            for i in chosen {
                battlefield[i].tapped = true;
            }
            battlefield[si].saddled = true;
        }
    }

    // 7e Equip: pay the equip cost once per Equipment while spare mana
    // covers it. The gear suits up its best body (highest printed power,
    // untapped and unsick so it attacks); the buff joins that host's
    // attack only.
    for ei in 0..battlefield.len() {
        if battlefield[ei].tapped || battlefield[ei].equipped {
            continue;
        }
        let eq = card_of(deck, &battlefield[ei]).flags.equipment;
        let Some(eq) = eq else {
            continue;
        };
        if eq.cost == 0 {
            continue;
        }
        // Host: the strongest untapped, unsick non-commander body.
        let host = (0..battlefield.len())
            .filter(|hi| {
                *hi != ei
                    && battlefield[*hi].card.deck_idx().is_some()
                    && !battlefield[*hi].tapped
                    && !battlefield[*hi].summoning_sick
                    && super::game::is_creature_permanent(deck, &battlefield[*hi])
            })
            .max_by_key(|hi| creature_power(&battlefield[*hi], deck));
        let Some(hi) = host else {
            continue;
        };
        let cost = super::model::Cost {
            generic: eq.cost,
            ..super::model::Cost::default()
        };
        if payable(&cost, pool) {
            pay_cost(&cost, pool);
            battlefield[ei].equipped = true;
            battlefield[ei].equip_host = Some(battlefield[hi].uid);
            battlefield[ei].fired = true;
        }
    }
}

/// Body power for a permanent: the printed power when the card row has
/// one, else the flat token value. Crew and station math use it.
pub(super) fn creature_power(perm: &Permanent, deck: &SimDeck) -> u32 {
    match perm.card {
        CardRef::Deck(idx) => deck[idx].printed_power.unwrap_or(TOKEN_CREATURE_POWER),
        CardRef::Commander { .. } | CardRef::Token(_) => TOKEN_CREATURE_POWER,
    }
}
