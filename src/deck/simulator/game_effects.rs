// Effect execution and the tap-budget pass for the goldfish game loop,
// split from game.rs to keep files small.

use super::game::{
    Activation, BODY_POWER, CardRef, GameState, Permanent, Pool, card_of, new_perm_with,
    new_token_perm, take_uid,
};
use super::game_mana::{
    add_yield, add_yield_turns, effective_min_cost, pay_cost, payable, pips_ok,
};
use super::model::{
    Ability, AbilityTiming, CardIdx, Effect, LibraryGraveyardTrigger, Role, SearchCardType,
    SearchDestination, SearchSpec, SimDeck,
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
            st.drained += amount * deck.format.drain_mult();
            st.life_gained += amount;
            st.life += amount as i32;
        }
        None => {}
    }
}

/// The full variant: `source` is the battlefield card whose
/// effect resolves (Treasure flags read from that card).
pub(crate) fn apply_effect_at(
    deck: &SimDeck,
    effect: &Effect,
    st: &mut GameState,
    turn: u32,
    mill_opp: bool,
    source: Option<CardIdx>,
) {
    // Treasure banking requires the token effect's own card to create
    // Treasures ("create a Treasure token" on the same card). A deck-wide
    // blanket would convert unrelated token effects into pips.
    let treasure_source = source
        .and_then(|i| deck.cards.get(i.index()))
        .map(|c| c.treasures_on_token)
        .unwrap_or(false);
    match effect {
        Effect::Draw(n) => {
            for _ in 0..*n {
                draw_one(deck, st, turn);
            }
        }
        Effect::Search(spec) => search_library(deck, st, *spec, turn),
        Effect::ExtraLand => {
            draw_one(deck, st, turn);
        }
        Effect::Blink => {}
        Effect::Monarch => {
            st.is_monarch = true;
        }
        Effect::Mill(n) => {
            for _ in 0..*n {
                mill_library_card(deck, st, turn, mill_opp);
            }
        }
        Effect::Scry(n) => {
            st.awareness_cards += *n;
        }
        Effect::Surveil(n) => {
            for _ in 0..*n {
                mill_library_card(deck, st, turn, false);
            }
        }
        Effect::Drain(n) => {
            // Three opponents in Commander: a player-targeted drain
            // resolves once, an "each opponent" drain triples. Both
            // read as N×3 life off the table (best case: all resolve).
            // Constructed tables are one opponent: ×1.
            let mult = deck.format.drain_mult();
            st.drained += *n * mult;
        }
        Effect::GainLife(n) => {
            st.life += *n as i32;
            st.life_gained += *n;
        }
        Effect::ExtraTurn => {
            st.extra_turns_queued += 1;
        }
        Effect::ReturnFromGraveyard { to_hand, count } => {
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
        Effect::Wheel => {
            let discarded = std::mem::take(&mut st.hand);
            for i in discarded {
                move_to_graveyard(deck, st, i, turn, CardZone::Hand);
            }
            for _ in 0..7 {
                draw_one(deck, st, turn);
            }
        }
        // A wheel resolving mid-cast: skip one hand card (the cast
        // spell itself, whose removal is deferred) so it is not
        // double-zoned, then draw seven.
        Effect::WheelSkip(skip) => {
            let hand = std::mem::take(&mut st.hand);
            for i in hand {
                if i == *skip {
                    st.hand.push(i);
                    continue;
                }
                move_to_graveyard(deck, st, i, turn, CardZone::Hand);
            }
            for _ in 0..7 {
                draw_one(deck, st, turn);
            }
        }
        Effect::Loot(n) => {
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
        Effect::Tokens(n) => {
            // Treasure-creating cards bank Treasure pips instead of
            // bodies (one banked any-color pip per Treasure,
            // sacrificed to use). Documented in assumptions.
            if treasure_source {
                st.treasure_bank += n;
                return;
            }
            // Token bodies join as small station/crew fuel. A count of
            // 0 creates nothing (a caller that cannot parse the amount
            // must not turn it into one body); the cap bounds go-wide
            // boards.
            for _ in 0..(*n).min(8) {
                let uid = take_uid(st);
                st.battlefield.push(new_token_perm(uid, turn));
            }
        }
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
        SearchCardType::Land => card.role == Role::Land,
        SearchCardType::BasicLand => card.role == Role::Land && card.is_basic_land,
        SearchCardType::Artifact => card.is_artifact,
        SearchCardType::Enchantment => card.is_enchantment,
        SearchCardType::ArtifactOrEnchantment => card.is_artifact || card.is_enchantment,
        SearchCardType::InstantSorcery => card.is_instant_or_sorcery,
        SearchCardType::Planeswalker => card.starting_loyalty.is_some(),
        SearchCardType::Permanent => !card.is_instant_or_sorcery,
    }) && (!spec.non_human || !card.is_human)
        && spec
            .color
            .is_none_or(|color| "WUBRG".find(color).is_some_and(|index| card.colors[index]))
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
            if deck[index].role == Role::Land {
                super::game::fire_triggers(deck, st, AbilityTiming::OnLandfall, turn);
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
    pool: &mut Pool,
    hand: &[CardIdx],
) {
    // Remaining demand: the cheapest uncast spell still in hand.
    let cheapest: Option<u32> = hand
        .iter()
        .filter(|i| deck[**i].role != Role::Land)
        .map(|i| effective_min_cost(deck, &deck[*i], battlefield).total())
        .min();

    // Creatures and crewed vehicles able to tap this turn.
    let tappable: Vec<usize> = battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            !p.tapped
                && !p.sick
                && p.card.deck_idx().is_some()
                && !card_of(deck, p).is_station_card
                && (card_of(deck, p).is_creature || p.animated)
        })
        .map(|(i, _)| i)
        .collect();

    // 7a Mana taps: only while the cheapest spell still needs mana.
    if let Some(need) = cheapest {
        for &bi in &tappable {
            if pool.total() >= need {
                break;
            }
            let card = card_of(deck, &battlefield[bi]);
            if let Some(y) = &card.tap {
                add_yield(y, pool);
                battlefield[bi].tapped = true;
            }
        }
    }

    // 7b Station: tap remaining bodies into the highest-threshold unfilled
    // spacecraft/planet, one at a time, counters = body power.
    for &bi in &tappable {
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
                        .is_some_and(|at| p.counters < at)
            })
            .max_by_key(|(_, p)| card_of(deck, p).animate_at().unwrap_or(0))
            .map(|(i, _)| i);
        if let Some(ti) = target {
            battlefield[ti].counters += body_power(&battlefield[bi], deck);
            battlefield[bi].tapped = true;
        }
    }

    // 7c Crew: tap bodies into untapped Vehicles (total power ≥ crew N).
    for vi in 0..battlefield.len() {
        if battlefield[vi].tapped
            || battlefield[vi].sick
            || card_of(deck, &battlefield[vi]).crew.is_none()
        {
            continue;
        }
        let crew_n = card_of(deck, &battlefield[vi]).crew.unwrap_or(1);
        let bodies_needed = crew_n.div_ceil(BODY_POWER);
        // Check total power first: bodies only tap when the crew
        // succeeds (a failed crew leaves them ready for mana/station).
        // Candidates come strongest first: the crew succeeds whenever
        // the board's best bodies can cover the cost.
        let mut candidates: Vec<usize> = (0..battlefield.len())
            .filter(|i| {
                *i != vi
                    && !battlefield[*i].tapped
                    && !battlefield[*i].sick
                    && battlefield[*i].card.deck_idx().is_some()
                    && (card_of(deck, &battlefield[*i]).is_creature || battlefield[*i].animated)
            })
            .collect();
        candidates.sort_by_key(|i| std::cmp::Reverse(body_power(&battlefield[*i], deck)));
        candidates.truncate(bodies_needed as usize);
        let power: u32 = candidates
            .iter()
            .map(|i| body_power(&battlefield[*i], deck))
            .sum();
        if power >= crew_n {
            for i in candidates {
                battlefield[i].tapped = true;
            }
            battlefield[vi].crewed = true;
            battlefield[vi].sick = false;
            // OnCrewed triggers fire now (tokens join next turn's bodies).
        }
    }

    // 7d Equip: pay the equip cost once per Equipment while spare mana
    // covers it. The gear suits up its best body (highest printed power,
    // untapped and unsick so it attacks); the buff joins that host's
    // attack only.
    for ei in 0..battlefield.len() {
        if battlefield[ei].tapped || battlefield[ei].equipped {
            continue;
        }
        let eq = card_of(deck, &battlefield[ei]).equipment;
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
                    && !battlefield[*hi].sick
                    && card_of(deck, &battlefield[*hi]).is_creature
            })
            .max_by_key(|hi| body_power(&battlefield[*hi], deck));
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
pub(super) fn body_power(perm: &Permanent, deck: &SimDeck) -> u32 {
    match perm.card {
        CardRef::Deck(idx) => deck[idx].printed_power.unwrap_or(BODY_POWER),
        CardRef::Commander { .. } | CardRef::Token => BODY_POWER,
    }
}

/// The spend-leftover-mana pass: repeatedly fire the cheapest unlocked
/// activation (draw engines, mana engines, walkers, sacrifice outlets).
/// Each firing taps the source or spends loyalty; sacrifice outlets feed
/// death triggers from the surviving board. Loyalty activations bypass
/// the mana pool (they spend loyalty); drain activations resolve at the
/// format's opponent multiplier. An untapped non-tapping activation
/// whose yield covers its own cost repeats — the pass caps it at
/// [`MAX_LOOP_PASSES`] firings and flags the census as a suspected
/// infinite engine.
pub(super) fn spend_leftover(deck: &SimDeck, st: &mut GameState, pool: &mut Pool, turn: u32) {
    let drain_mult = deck.format.drain_mult();
    let mut activations_this_turn: u32 = 0;
    let mut produced_mana = false;
    let mut repeatable_mana_activations = std::collections::HashMap::new();
    while activations_this_turn < super::model::MAX_LOOP_PASSES
        && let Some(a) = pick_best_activation(deck, st, pool)
    {
        let mana_before = pool.total();
        if repeatable_mana_component(deck, st, &a) {
            let key = (
                a.uid,
                a.cost,
                a.ability.taps,
                std::mem::discriminant(&a.ability.effect),
            );
            *repeatable_mana_activations.entry(key).or_insert(0u32) += 1;
        }
        resolve_activation(deck, st, pool, turn, drain_mult, &a);
        produced_mana |= pool.total() > mana_before;
        activations_this_turn += 1;
    }
    let repeated_mana_action = repeatable_mana_activations
        .values()
        .any(|activations| *activations > 1);
    if activations_this_turn == super::model::MAX_LOOP_PASSES
        && produced_mana
        && repeated_mana_action
    {
        // Require a repeated mana or self-untap activation as well as net
        // mana growth. A large batch of one-shot mana abilities is finite.
        st.infinite_mana_suspected = true;
        super::game::milestone_for_turn(st, turn).positive_mana_loop = true;
    }
}

/// Return true for a mana action that can repeat without consuming a finite
/// life, counter, body, or once-per-turn resource.
fn repeatable_mana_component(deck: &SimDeck, st: &GameState, activation: &Activation) -> bool {
    let ability = &activation.ability;
    if ability.once_per_turn
        || ability.uses_counters
        || ability.sacrifice_bodies > 0
        || ability.life_cost > 0
    {
        return false;
    }
    match &ability.effect {
        Effect::Mana(_) => ability.cost.total() == 0 && !ability.taps,
        Effect::UntapSelf => st
            .battlefield
            .get(activation.pos)
            .is_some_and(|permanent| card_of(deck, permanent).tap.is_some()),
        _ => false,
    }
}

/// Test probe wrapper for resolution.
#[cfg(test)]
pub(super) fn resolve_activation_public(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: u32,
    drain_mult: u32,
    a: &Activation,
) {
    resolve_activation(deck, st, pool, turn, drain_mult, a);
}

/// Test probe wrapper.
#[cfg(test)]
pub(super) fn pick_best_activation_public(
    deck: &SimDeck,
    st: &GameState,
    pool: &Pool,
) -> Option<Activation> {
    pick_best_activation(deck, st, pool)
}

/// Find the cheapest usable activation on the battlefield. A candidate
/// must be untapped (loyalty and banked shapes included), affordable,
/// and ungated by `fired`; free untapped candidates win cost ties so the
/// pass can keep looping.
fn pick_best_activation(deck: &SimDeck, st: &GameState, pool: &Pool) -> Option<Activation> {
    let mut best: Option<Activation> = None;
    for (bi, perm) in st.battlefield.iter().enumerate() {
        let card = card_of(deck, perm);
        // Loyalty activations gate on loyalty, not mana, so they pass
        // the tap/counter filter too (planeswalker minus and plus
        // abilities fire here).
        let unlocked = card
            .station_tiers
            .iter()
            .filter(|t| t.at == 0 || perm.counters >= t.at)
            .flat_map(|t| t.abilities.iter())
            .filter(|a| {
                let free_mana = a.cost.total() == 0 && matches!(a.effect, Effect::Mana(_));
                // X-sink counters ("{X}: Put X tower counters") cost
                // mana without tapping: the leftover pool converts.
                let x_sink = matches!(a.effect, Effect::Counters(0));
                a.trigger == AbilityTiming::Activated
                    && (a.taps
                        || a.uses_counters
                        || a.loyalty_cost > 0
                        || a.loyalty_gain > 0
                        || a.life_cost > 0
                        || a.sacrifice_bodies > 0
                        || free_mana
                        || x_sink
                        || matches!(a.effect, Effect::Search(_))
                        || matches!(a.effect, Effect::UntapSelf))
                    && (!a.taps || !perm.tapped && !perm.sick)
            });
        for ability in unlocked {
            if ability.life_cost > 0 && st.life <= ability.life_cost as i32 {
                continue;
            }

            let eligible_bodies: Vec<u32> = st
                .battlefield
                .iter()
                .filter(|candidate| {
                    candidate.uid != perm.uid
                        && candidate.card.deck_idx().is_some()
                        && card_of(deck, candidate).is_creature
                })
                .map(|candidate| candidate.uid)
                .collect();
            if eligible_bodies.len() < ability.sacrifice_bodies as usize {
                continue;
            }
            if !activation_usable(perm, ability, pool) {
                continue;
            }
            let candidate_cost = ability.cost.total();
            if best.as_ref().is_none_or(|b| {
                // Free untapped candidates win cost ties: they leave the
                // source untapped so the pass can keep looping ({0}: Add
                // mode beats the {T} mode for loop engines).
                candidate_cost < b.cost
                    || (candidate_cost == b.cost && b.ability.taps && !ability.taps)
            }) {
                let draws = match ability.effect {
                    Effect::Draw(n) => n,
                    Effect::DrawAndMinusCounter => 1,
                    Effect::Loot(n) => n,
                    _ => 0,
                };
                let search = match ability.effect {
                    Effect::Search(spec) => Some(spec),
                    _ => None,
                };
                let mana = match &ability.effect {
                    Effect::Mana(y) => Some(y.clone()),
                    _ => None,
                };
                let counters = match ability.effect {
                    Effect::Counters(n) => n,
                    _ => 0,
                };
                let drain = match ability.effect {
                    Effect::Drain(n) => n,
                    _ => 0,
                };
                best = Some(Activation {
                    pos: bi,
                    uid: perm.uid,
                    ability: ability.clone(),
                    cost: ability.cost.total(),
                    draws,
                    search,
                    mana_yield: mana,
                    counters,
                    drain,
                    sacrifice_uid: eligible_bodies.first().copied(),
                    target_uid: eligible_bodies
                        .get(ability.sacrifice_bodies as usize)
                        .copied(),
                });
            }
        }
    }
    best
}

/// True when an unlocked activation can fire right now: the `fired`
/// gate, affordability, and the loyalty/counter gates.
fn activation_usable(perm: &Permanent, ability: &Ability, pool: &Pool) -> bool {
    let usable = matches!(
        ability.effect,
        Effect::Draw(_)
            | Effect::DrawAndMinusCounter
            | Effect::GainLife(_)
            | Effect::Search(_)
            | Effect::Mana(_)
            | Effect::UntapSelf
            | Effect::Counters(_)
            | Effect::Loot(_)
            | Effect::Drain(_)
            | Effect::Tokens(_)
    ) || ability.sacrifice_bodies > 0;
    // Banked activations (Pentad Prism) consume a charge counter per
    // fire; gate on the host's counters. Free untapped activations
    // ({0}: Add ...) bypass `fired` — they repeat in real Magic and
    // feed the loop census — unless the card bounds them ("Activate
    // only once each turn").
    let banked = ability.uses_counters && perm.counters > 0;

    let free_loop = ability.life_cost > 0
        || ability.sacrifice_bodies > 0
        || !ability.taps
            && !ability.uses_counters
            && ability.loyalty_cost == 0
            && ability.loyalty_gain == 0
            && ability.cost.total() == 0
            && !ability.once_per_turn;
    // Free untapped activations repeat every turn regardless of
    // `fired` (they have no once-per-turn cost marker); the cap
    // catches runaway loops.
    let gated = !free_loop && perm.fired;
    let loyalty_affordable = ability.loyalty_cost == 0 || perm.loyalty >= ability.loyalty_cost;
    usable
        && !gated
        && loyalty_affordable
        && (banked
            || (!ability.uses_counters
                && (ability.loyalty_cost > 0
                    || ability.loyalty_gain > 0
                    || (payable(&ability.cost, pool) && pips_ok(&ability.cost, pool)))))
}

/// Apply one chosen activation: pay the cost (mana, loyalty, or a
/// charge counter), tap the source, resolve the effect, and run the
/// sacrifice-outlet death triggers.
fn resolve_activation(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: u32,
    drain_mult: u32,
    a: &Activation,
) {
    let ability = &a.ability;
    let x_sink = matches!(ability.effect, Effect::Counters(0));
    let perm = &mut st.battlefield[a.pos];
    // Loyalty activations spend/gain loyalty; mana costs do not apply.
    if ability.loyalty_cost > 0 {
        perm.loyalty = perm.loyalty.saturating_sub(ability.loyalty_cost);
        perm.fired = true;
    } else if ability.loyalty_gain > 0 {
        perm.loyalty += ability.loyalty_gain;
        perm.fired = true;
    } else if ability.uses_counters {
        // Banked activation: one charge counter buys the pip; the
        // source stays untapped but can fire once per turn (fired
        // gates the repeat pass; the counter decrement still applies).
        perm.counters = perm.counters.saturating_sub(1);
        perm.fired = true;
    } else if !ability.taps && ability.cost.total() == 0 {
        // Free untapped activation: stays untapped, never sets `fired`
        // — the loop runs until the cap catches it. A once-per-turn
        // bound sets `fired` (no loop).
        if ability.once_per_turn {
            perm.fired = true;
        }
    } else {
        pay_cost(&ability.cost, pool);
        // Only `{T}`-cost activations tap the source; free activations
        // ({0}:) stay untapped.
        if a.ability.taps {
            perm.tapped = true;
        }
        if ability.taps || ability.once_per_turn {
            perm.fired = true;
        }
    }
    if ability.life_cost > 0 {
        st.life -= ability.life_cost as i32;
        st.life_paid += ability.life_cost;
        st.life_funded_draws += a.draws;
        super::game::milestone_for_turn(st, turn).life_funded_draws += a.draws;
    }
    if ability.sacrifice_bodies > 0
        && let Some(uid) = a.sacrifice_uid
    {
        resolve_sacrifice_uid(deck, st, turn, uid);
    }
    for _ in 0..a.draws {
        draw_one(deck, st, turn);
    }
    if matches!(ability.effect, Effect::DrawAndMinusCounter)
        && let Some(target_uid) = a.target_uid
    {
        apply_minus_counter(deck, st, turn, target_uid);
    }
    if let Effect::GainLife(amount) = ability.effect {
        st.life += amount as i32;
        st.life_gained += amount;
    }
    if let Some(spec) = a.search {
        apply_effect_at(deck, &Effect::Search(spec), st, turn, false, None);
    }
    // Drain activations resolve at the format's opponent multiplier.
    st.drained += a.drain * drain_mult;
    // Mana activations feed this turn's pool; counter engines
    // (Moxite Refinery) charge their host.
    if let Some(y) = &a.mana_yield {
        add_yield_turns(deck, y, pool, turn, &st.battlefield);
    }
    if matches!(ability.effect, Effect::UntapSelf)
        && let Some(perm) = st.battlefield.iter_mut().find(|perm| perm.uid == a.uid)
    {
        perm.tapped = false;
        let source = perm.clone();
        let yield_ = card_of(deck, &source).tap.clone();
        if let Some(yield_) = yield_ {
            super::game_run::add_nonland_mana(deck, st, &source, &yield_, pool, turn);
            if let Some(perm) = st.battlefield.iter_mut().find(|perm| perm.uid == a.uid) {
                perm.tapped = true;
            }
        }
    }
    // The sacrifice loop may have removed battlefield entries and
    // shifted positions, so locate the permanent by uid (every other
    // lookup in this file does) instead of the recorded index.
    if let Some(perm) = st.battlefield.iter_mut().find(|p| p.uid == a.uid) {
        if x_sink {
            // X-sink: the printed {X} already paid 1 through `pay_cost`
            // above. The rest of the pool is the player-chosen extra X
            // (paying everything in is legal), so the paid X = 1 +
            // leftover: the counters match the spend and the pool ends
            // at zero with nothing swallowed.
            let extra = pool.total();
            perm.counters += 1 + extra;
            *pool = super::game::Pool::default();
        } else {
            perm.counters += a.counters;
        }
    }
}

/// Consume one creature body for a sacrifice cost and fire the surviving
/// board's death triggers (aristocrats outlets).
pub(super) fn resolve_sacrifice(deck: &SimDeck, st: &mut GameState, turn: u32, source_uid: u32) {
    let victim = st.battlefield.iter().position(|p| {
        p.card.deck_idx().is_some()
            && p.uid != source_uid
            && card_of(deck, p).is_creature
            && !p.tapped
    });
    resolve_sacrifice_at(deck, st, turn, victim);
}

/// Sacrifice any creature body: deck bodies first, then token bodies.
/// Tokens count as creatures here (2/2 bodies), so an exchange or
/// sacrifice cost can consume them.
pub(super) fn resolve_sacrifice_body(
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

/// True when the board holds any sacrificial creature body (deck card
/// or token), untapped.
pub(super) fn has_sacrifice_body(deck: &SimDeck, st: &GameState) -> bool {
    st.battlefield
        .iter()
        .any(|p| !p.tapped && card_of(deck, p).is_creature)
}

/// Sacrifice one selected creature and fire the surviving board's death triggers.
pub(super) fn resolve_sacrifice_uid(deck: &SimDeck, st: &mut GameState, turn: u32, uid: u32) {
    let victim = st.battlefield.iter().position(|p| p.uid == uid);
    resolve_sacrifice_at(deck, st, turn, victim);
}

/// Remove the selected battlefield position and resolve death triggers.
fn resolve_sacrifice_at(deck: &SimDeck, st: &mut GameState, turn: u32, victim: Option<usize>) {
    let Some(v) = victim else {
        return;
    };
    let victim_perm = st.battlefield[v].clone();
    let victim_card = victim_perm.card;
    st.battlefield.remove(v);
    if let Some(idx) = victim_card.deck_idx() {
        move_to_graveyard(deck, st, idx, turn, CardZone::Battlefield);
    }
    // Death triggers: draw/token payoffs fire from the surviving board.
    let death_effects: Vec<Effect> = st
        .battlefield
        .iter()
        .flat_map(|p| card_of(deck, p).abilities().cloned().collect::<Vec<_>>())
        .filter(|ab| ab.trigger == AbilityTiming::OnDeath)
        .map(|ab| ab.effect)
        .collect();
    for effect in &death_effects {
        // Death-trigger mills are graveyard fuel (self).
        apply_effect_at(deck, effect, st, turn, false, None);
    }
    if let Some(idx) = victim_card.deck_idx()
        && deck[idx].has_undying
        && victim_perm.counters == 0
    {
        st.graveyard.retain(|index| *index != idx);
        st.battlefield_seen.entry(idx).or_insert(turn);
        let uid = take_uid(st);
        let mut returned = super::game::new_perm_with(uid, deck, idx, turn, false);
        returned.counters = 1;
        st.battlefield.push(returned);
    }
}

/// Resolve the chosen creature's -1/-1 counter and any resulting death.
fn apply_minus_counter(deck: &SimDeck, st: &mut GameState, turn: u32, target_uid: u32) {
    let Some(position) = st
        .battlefield
        .iter()
        .position(|perm| perm.uid == target_uid)
    else {
        return;
    };
    let target = &mut st.battlefield[position];
    if target.counters > 0 {
        target.counters -= 1;
        return;
    }
    if card_of(deck, target)
        .printed_toughness
        .is_some_and(|toughness| toughness <= 1)
    {
        resolve_sacrifice_uid(deck, st, turn, target_uid);
    }
}
