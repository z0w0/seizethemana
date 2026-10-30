//! Runtime effects and the tap-budget pass for the goldfish game loop.

/// Nonconsuming Behold choices and linked search results.
pub(in crate::deck::simulator) mod behold;
/// Keyword-action and counter effects (amass, the Ring, empower Jace,
/// explore, connive, mobilize, proliferate).
#[path = "game_effects/keyword_actions.rs"]
mod keyword_actions;
/// Complete library searches and shared result placement.
mod search;
/// Choices over exposed cards and the current hand.
mod selection;
#[cfg(test)]
use search::search_matches;

pub(in crate::deck::simulator) use keyword_actions::{
    amass, connive, empower_jace, explore, mobilize, resolve_proliferate, ring_tempts,
};

/// Activation selection and resolution (spend-leftover, sacrifice outlets).
#[path = "game_effects/activation.rs"]
pub(in crate::deck::simulator) mod activation;

pub(in crate::deck::simulator) use activation::{
    enforce_legend_rule, resolve_sacrifice_batch, resolve_sacrifice_uid, spend_leftover,
};

use super::game::{
    CardRef, GameState, ManaPool, Permanent, TOKEN_CREATURE_POWER, card_of,
    graveyard_entry_matches, graveyard_generation, new_token_perm, record_card_seen, take_uid,
};
use super::game_mana::{add_yield, effective_min_cost, pay_cost, payable};
use super::model::{CardIdx, Role, SimCard, SimDeck, SimEffect};
#[cfg(test)]
use super::model::{SearchCardType, SearchDestination, SearchSpec};
use super::resolution::{begin_resolution, end_resolution};

/// Graveyard targets selected before a spell pays any costs.
#[derive(Clone, Default)]
pub(super) struct PreparedSpellTargets {
    graveyard_returns: std::collections::VecDeque<Vec<GraveyardTarget>>,
}

/// One physical card during one specific stay in the graveyard.
#[derive(Clone, Copy)]
struct GraveyardTarget {
    index: CardIdx,
    generation: u32,
}

impl PreparedSpellTargets {
    /// Resolve nontargeted effects or effects with at least one surviving target.
    pub(super) fn any_legal_target(&self, st: &GameState) -> bool {
        self.graveyard_returns.iter().all(Vec::is_empty)
            || self
                .graveyard_returns
                .iter()
                .flatten()
                .any(|target| graveyard_entry_matches(st, target.index, target.generation))
    }
}

/// Check required spell targets without treating a permanent's ETB as a cost.
pub(super) fn spell_targets_available(deck: &SimDeck, st: &GameState, card: &SimCard) -> bool {
    prepare_spell_targets(deck, st, card).is_some()
}

/// Select supported graveyard targets from the pre-cost zones.
pub(super) fn prepare_spell_targets(
    deck: &SimDeck,
    st: &GameState,
    card: &SimCard,
) -> Option<PreparedSpellTargets> {
    prepare_effect_targets(deck, st, &card.spell_data.effects)
}

/// Select required targets for a whole spell or triggered ability at one boundary.
pub(super) fn prepare_effect_targets(
    deck: &SimDeck,
    st: &GameState,
    effects: &[SimEffect],
) -> Option<PreparedSpellTargets> {
    let mut targets = PreparedSpellTargets::default();
    for effect in effects {
        if let SimEffect::ReturnFromGraveyard { to_hand, count } = effect {
            let selected = select_graveyard_returns(deck, st, *to_hand, *count);
            if selected.len() < *count as usize {
                return None;
            }
            targets.graveyard_returns.push_back(selected);
        }
    }
    Some(targets)
}

/// Apply a spell effect without selecting a new graveyard target after costs.
pub(super) fn apply_prepared_spell_effect_at(
    deck: &SimDeck,
    effect: &SimEffect,
    st: &mut GameState,
    turn: u32,
    mill_opp: bool,
    source: Option<CardIdx>,
    targets: &mut PreparedSpellTargets,
) {
    if let SimEffect::ReturnFromGraveyard { to_hand, .. } = effect {
        let selected = targets
            .graveyard_returns
            .pop_front()
            .expect("each spell return effect has prepared targets");
        begin_resolution(st);
        return_graveyard_cards(deck, st, turn, *to_hand, &selected);
        end_resolution(deck, st, None);
    } else {
        apply_effect_at(deck, effect, st, turn, mill_opp, source);
    }
}

/// Select the newest eligible graveyard entries before paying targeted-return costs.
fn select_graveyard_returns(
    deck: &SimDeck,
    st: &GameState,
    to_hand: bool,
    count: u32,
) -> Vec<GraveyardTarget> {
    st.graveyard
        .iter()
        .rev()
        .copied()
        .filter(|index| to_hand || deck[*index].is_creature)
        .take(count as usize)
        .map(|index| GraveyardTarget {
            index,
            generation: graveyard_generation(st, index),
        })
        .collect()
}

/// Move selected legal cards without substituting another type or destination.
fn return_graveyard_cards(
    deck: &SimDeck,
    st: &mut GameState,
    turn: u32,
    to_hand: bool,
    selected: &[GraveyardTarget],
) {
    for target in selected {
        let index = target.index;
        if !graveyard_entry_matches(st, index, target.generation)
            || !to_hand && !deck[index].is_creature
        {
            continue;
        }
        let position = st
            .graveyard
            .iter()
            .position(|held| *held == index)
            .expect("a legal graveyard target remains in its recorded zone");
        st.graveyard.remove(position);
        if to_hand {
            record_card_seen(st, index, turn);
            st.hand.push(index);
        } else {
            st.battlefield_seen.entry(index).or_insert(turn);
            let entry = super::cast_pass::new_entry_permanent(deck, st, index, turn, false);
            st.battlefield.push(entry);
            super::cast_pass::fire_entry_triggers(deck, st, st.battlefield.len() - 1, turn);
        }
    }
}

/// Distinguish an actual draw from a completed dredge replacement or an unavailable draw.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DrawOutcome {
    /// A library card was drawn into hand.
    Drawn,
    /// Dredge replaced the draw with milling and a graveyard return.
    Replaced,
    /// The game had ended or the library could not supply a draw.
    NotDrawn,
}

/// Complete one draw action, including its supported dredge replacement.
pub(super) fn draw_one(deck: &SimDeck, st: &mut GameState, turn: u32) -> bool {
    draw_one_outcome(deck, st, turn) != DrawOutcome::NotDrawn
}

/// Resolve a draw while retaining whether an actual card was drawn.
pub(super) fn draw_one_outcome(deck: &SimDeck, st: &mut GameState, turn: u32) -> DrawOutcome {
    if st.completion.is_some() {
        return DrawOutcome::NotDrawn;
    }
    begin_resolution(st);
    let drawn = draw_one_inner(deck, st, turn);
    end_resolution(deck, st, None);
    drawn
}

/// Complete a draw or its full dredge replacement before settling triggers.
fn draw_one_inner(deck: &SimDeck, st: &mut GameState, turn: u32) -> DrawOutcome {
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
        record_card_seen(st, index, turn);
        st.hand.push(index);
        return DrawOutcome::Replaced;
    }
    let Some(index) = st.library.pop() else {
        st.failed_draw = true;
        return DrawOutcome::NotDrawn;
    };
    record_card_seen(st, index, turn);
    st.hand.push(index);
    st.seen += 1;
    st.awareness_cards += 1;
    DrawOutcome::Drawn
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
    if st.completion.is_some() {
        return;
    }
    begin_resolution(st);
    st.graveyard_seen.entry(index).or_insert(turn);
    *st.graveyard_generations.entry(index).or_default() += 1;
    st.graveyard.push(index);
    if matches!(source, CardZone::Library) {
        super::resolution::queue_library_graveyard(deck, st, index, turn);
    }
    end_resolution(deck, st, None);
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
    if st.completion.is_some() {
        return;
    }
    begin_resolution(st);
    apply_effect_inner(deck, effect, st, turn, mill_opp, source);
    end_resolution(deck, st, None);
}

/// Apply one complete effect before its events can resolve other abilities.
fn apply_effect_inner(
    deck: &SimDeck,
    effect: &SimEffect,
    st: &mut GameState,
    turn: u32,
    mill_opp: bool,
    source: Option<CardIdx>,
) {
    match effect {
        SimEffect::Behold { spec, effects } => {
            if behold::available(deck, st, spec, None) {
                for effect in effects {
                    apply_effect_inner(deck, effect, st, turn, mill_opp, source);
                }
            }
        }
        SimEffect::SearchThenBeholdUntap {
            search: spec,
            spec: behold_spec,
        } => {
            let uid = search::search_one(deck, st, *spec, turn);
            behold::untap_search_result(deck, st, behold_spec, uid);
        }
        SimEffect::Draw(n) => {
            for _ in 0..*n {
                draw_one(deck, st, turn);
            }
        }
        SimEffect::Search(spec) => {
            let _ = search::search_one(deck, st, *spec, turn);
        }
        SimEffect::MultiSearch(spec) => search::search_many(deck, st, spec, turn),
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
        SimEffect::Scry(n) => selection::scry(deck, st, *n, turn),
        SimEffect::Surveil(n) => selection::surveil(deck, st, *n, turn),
        SimEffect::LookAtTop { count, may_shuffle } => {
            selection::look_at_top(deck, st, *count, *may_shuffle, turn);
        }
        SimEffect::PutHandOnTop(count) => selection::put_hand_on_top(deck, st, *count, turn),
        SimEffect::LoseLife { amount, scope } => {
            // The scope decides the table multiplier (CR 119.3): "each
            // opponent" hits the opponent count, "target player" hits
            // exactly one, "each player" hits the table plus the player,
            // and "you lose" costs only the player.
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
        SimEffect::GainLife(n) => {
            st.life += *n as i32;
            st.life_gained += *n;
        }
        SimEffect::ExtraTurn => {
            st.extra_turns_queued += 1;
        }
        SimEffect::ReturnFromGraveyard { to_hand, count } => {
            let selected = select_graveyard_returns(deck, st, *to_hand, *count);
            return_graveyard_cards(deck, st, turn, *to_hand, &selected);
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
        SimEffect::DrawThenDiscard(n) => selection::draw_discard(deck, st, *n, *n, turn),
        SimEffect::DrawDiscard { draw, discard } => {
            selection::draw_discard(deck, st, *draw, *discard, turn);
        }
        SimEffect::Tokens(n) => {
            // Token bodies join as small station/crew fuel. A count of
            // 0 creates nothing (a caller that cannot parse the amount
            // must not turn it into one body); the cap bounds go-wide
            // boards.
            for _ in 0..(*n).min(8) {
                let uid = take_uid(st);
                st.battlefield.push(new_token_perm(uid, turn));
                super::game::fire_on_enter(deck, st, st.battlefield.len() - 1, turn, false);
            }
            record_effect_limit(st, *n, 8);
        }
        SimEffect::Treasures(count) => {
            st.treasure_bank += count;
        }
        SimEffect::TokensEachOpponent { per_opponent } => {
            // "For each opponent, create N tokens" (CR 102.x opponent
            // count): N per opponent, so the table sees N × opponents.
            let total = per_opponent * deck.format.life_loss_mult();
            apply_effect_at(deck, &SimEffect::Tokens(total), st, turn, mill_opp, source);
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
            // Afterlife creates 1/1 flying Spirits after the source leaves.
            for _ in 0..(*amount).min(8) {
                let uid = take_uid(st);
                let mut token = new_token_perm(uid, turn);
                token.card = CardRef::Token(super::game::TokenKind::Spirit);
                st.battlefield.push(token);
                super::game::fire_on_enter(deck, st, st.battlefield.len() - 1, turn, false);
            }
            record_effect_limit(st, *amount, 8);
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
                for _ in 0..draws {
                    draw_one(deck, st, turn);
                    if st.failed_draw {
                        break;
                    }
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

/// Record real unfinished object creation, without flagging exact-limit effects.
pub(super) fn record_effect_limit(st: &mut GameState, requested: u32, limit: u32) {
    if requested > limit {
        super::resolution::mark_incomplete(
            st,
            super::game::IncompleteReason::EffectWork {
                configured_limit: limit,
                remaining_work: requested - limit,
            },
        );
    }
}

/// Verify search filters against both matching and nonmatching library cards.
#[cfg(test)]
#[path = "game_effects/tests/search_tests.rs"]
mod search_tests;

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
            battlefield[ti].counters.charge +=
                current_creature_power(&battlefield[bi], deck, battlefield);
            battlefield[bi].tapped = true;
        }
    }

    // 7c Crew: tap bodies into untapped Vehicles (total power ≥ crew N).
    for vi in 0..battlefield.len() {
        if battlefield[vi].tapped || card_of(deck, &battlefield[vi]).crew.is_none() {
            continue;
        }
        let crew_n = card_of(deck, &battlefield[vi]).crew.unwrap_or(1);
        // Check total power first: bodies only tap when the crew
        // succeeds (a failed crew leaves them ready for mana/station).
        // Candidates come strongest first: the crew succeeds whenever
        // the board's best bodies can cover the cost.
        let mut candidates: Vec<usize> = (0..battlefield.len())
            .filter(|i| {
                *i != vi
                    && !battlefield[*i].tapped
                    && super::game::is_creature_permanent(deck, &battlefield[*i])
            })
            .collect();
        candidates.sort_by_key(|i| {
            std::cmp::Reverse(current_creature_power(&battlefield[*i], deck, battlefield))
        });
        let mut power = 0u32;
        let mut chosen = Vec::new();
        for index in candidates {
            if power >= crew_n {
                break;
            }
            power = power.saturating_add(current_creature_power(
                &battlefield[index],
                deck,
                battlefield,
            ));
            chosen.push(index);
        }
        if power >= crew_n {
            for i in chosen {
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
                    && super::game::is_creature_permanent(deck, &battlefield[*i])
            })
            .collect();
        candidates.sort_by_key(|i| {
            std::cmp::Reverse(current_creature_power(&battlefield[*i], deck, battlefield))
        });
        let mut chosen = Vec::new();
        let mut power = 0u32;
        for i in candidates {
            if power >= saddle_n {
                break;
            }
            power =
                power.saturating_add(current_creature_power(&battlefield[i], deck, battlefield));
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
        // Equipment can attach to a commander or a token as well as a library card.
        let host = (0..battlefield.len())
            .filter(|hi| {
                *hi != ei
                    && !battlefield[*hi].tapped
                    && (!battlefield[*hi].summoning_sick
                        || card_of(deck, &battlefield[*hi]).flags.has_haste)
                    && super::game::is_creature_permanent(deck, &battlefield[*hi])
            })
            .max_by_key(|hi| current_creature_power(&battlefield[*hi], deck, battlefield));
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

/// Base power and counters for focused counter-model tests.
#[cfg(test)]
pub(super) fn creature_power(perm: &Permanent, deck: &SimDeck) -> u32 {
    creature_power_signed(perm, deck).clamp(0, i64::from(u32::MAX)) as u32
}

/// Keep negative power until callers finish applying supported bonuses.
pub(super) fn creature_power_signed(perm: &Permanent, deck: &SimDeck) -> i64 {
    let card = card_of(deck, perm);
    let base = if perm.face_down {
        TOKEN_CREATURE_POWER
    } else if perm.army {
        0
    } else {
        card.printed_power.unwrap_or(TOKEN_CREATURE_POWER)
    };
    i64::from(base) + i64::from(perm.counters.plus1) - i64::from(perm.counters.minus1)
        + i64::from(perm.prowess_bonus)
}

/// Current power for crew, station, and other power-based choices.
pub(super) fn current_creature_power(
    perm: &Permanent,
    deck: &SimDeck,
    battlefield: &[Permanent],
) -> u32 {
    current_creature_power_signed(perm, deck, battlefield).clamp(0, i64::from(u32::MAX)) as u32
}

/// Apply supported board and equipment bonuses without losing negative base power.
pub(super) fn current_creature_power_signed(
    perm: &Permanent,
    deck: &SimDeck,
    battlefield: &[Permanent],
) -> i64 {
    let mut power = creature_power_signed(perm, deck);
    for source in battlefield.iter().filter(|source| !source.face_down) {
        let card = card_of(deck, source);
        if source.land_face && card.is_mdfc_spell {
            continue;
        }
        power += i64::from(card.flags.buff.map_or(0, |buff| buff.0));
        if source.equipped && source.equip_host == Some(perm.uid) {
            power += i64::from(card.flags.equipment.map_or(0, |equipment| equipment.buff.0));
        }
    }
    if perm.saddled && !perm.face_down {
        power += i64::from(
            card_of(deck, perm)
                .flags
                .saddled_buff
                .map_or(0, |buff| buff.0),
        );
    }
    power
}
