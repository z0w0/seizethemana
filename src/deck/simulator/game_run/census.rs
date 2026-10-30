//! Track per-turn game metrics and assemble the final game log.

use super::super::game::{GameLog, GameState, HAND_LIMIT, is_creature_permanent};
use super::super::game_mana::{cast_cost, cast_restrictions, payment_plan, usable_for_classes};
use super::super::model::{CardIdx, Role, SimDeck};
use std::collections::HashMap;

/// Per-game census arrays the turn loop fills and the log carries.
#[derive(Clone)]
pub(in crate::deck::simulator) struct TurnCensus {
    /// Land drops made each turn (0-3).
    pub(in crate::deck::simulator) land_drops: Vec<u8>,
    /// Turns whose end-step state was observed before any terminal loss.
    pub(in crate::deck::simulator) turn_observed: Vec<bool>,
    /// Phase-level observations for each requested turn.
    pub(in crate::deck::simulator) turn_observations: Vec<super::super::game::TurnObservation>,
    /// Spendable mana at the first main phase of each turn.
    pub(in crate::deck::simulator) mana_available: Vec<f64>,
    /// Mana spent on casts each turn.
    pub(in crate::deck::simulator) mana_spent: Vec<f64>,
    /// Spendable mana left after main-phase actions.
    pub(in crate::deck::simulator) mana_remaining: Vec<f64>,
    /// Cumulative cards seen (drawn) by end of each turn.
    pub(in crate::deck::simulator) cards_seen: Vec<u32>,
    /// First turn the board could pay each card's cost, per deck index.
    pub(in crate::deck::simulator) mana_ready: Vec<Option<u32>>,
    /// First turn each role was seen in hand.
    pub(in crate::deck::simulator) first_seen: HashMap<Role, u32>,
    /// First turn a creature was seen in hand.
    pub(in crate::deck::simulator) first_creature: Option<u32>,
    /// Colors a cast was blocked for (had total mana, missing pips).
    pub(in crate::deck::simulator) blocked_colors: [bool; 5],
    /// First turn the commander was castable; None when never.
    pub(in crate::deck::simulator) commander_castable: Option<u32>,
    /// First turn the commander spacecraft was animated (station online).
    pub(in crate::deck::simulator) station_online: Option<u32>,
    /// First turn the companion was fetched to hand.
    pub(in crate::deck::simulator) companion_online: Option<u32>,
    /// Creature permanents (including animated vehicles) per turn.
    pub(in crate::deck::simulator) creatures: Vec<u32>,
    /// Repeatable card sources available each turn.
    pub(in crate::deck::simulator) repeatable_sources_online: Vec<u32>,
    /// Graveyard size at the end of each turn.
    pub(in crate::deck::simulator) graveyard_size: Vec<u32>,
    /// First turn each card index was seen in hand (combo assembly).
    /// (card index, color index) pairs pip-blocked this game, one row per
    /// blocked cast attempt.
    pub(in crate::deck::simulator) pip_blocks: Vec<(CardIdx, usize)>,
    /// Total attacking power on the board at the combat phase of each turn.
    pub(in crate::deck::simulator) attack_power: Vec<u32>,
    /// Potential combat damage by commander slot and turn.
    pub(in crate::deck::simulator) commander_combat_damage: Vec<Vec<u32>>,
    /// Attacking bodies each turn (denominator for the evasion census).
    pub(in crate::deck::simulator) attackers_turn: Vec<u32>,
    /// Attacking bodies with evasion (trample/flying/menace) each turn.
    pub(in crate::deck::simulator) evasive_turn: Vec<u32>,
    /// Library size at the end of each turn (deck-out proximity).
    pub(in crate::deck::simulator) library_size: Vec<u32>,
    /// Lands seen (hand + battlefield + graveyard) by end of each turn.
    pub(in crate::deck::simulator) lands_seen: Vec<u32>,
    /// Cards self-milled (own-library mill + surveil) by end of turn.
    pub(in crate::deck::simulator) self_milled: Vec<u32>,
    /// Cards milled toward opponents by end of turn.
    pub(in crate::deck::simulator) opp_milled: Vec<u32>,
    /// Cards evaluated (drawn + milled + scried/surveiled) per turn,
    /// cumulative fraction of the library.
    pub(in crate::deck::simulator) awareness: Vec<f64>,
    /// Opponent life lost to life-loss effects by end of each turn.
    pub(in crate::deck::simulator) opponent_life_loss: Vec<u32>,
    /// Player damage (combat + combat-damage triggers) per turn, cumulative.
    pub(in crate::deck::simulator) player_damage: Vec<u32>,
    /// Extra turns taken by end of each turn (0 or 1 per slot).
    pub(in crate::deck::simulator) extra_turns: Vec<u32>,
    /// First turn a win-threshold engine could fire (enough counters).
    pub(in crate::deck::simulator) win_threshold_turn: Option<u32>,
    /// First turn a planeswalker ultimate became affordable.
    pub(in crate::deck::simulator) ultimate_online: Option<u32>,
    /// Ready-to-fire interaction (in hand + affordable) per turn.
    pub(in crate::deck::simulator) interaction_ready: Vec<bool>,
    /// Spare mana while interaction was ready, per turn.
    pub(in crate::deck::simulator) interaction_mana_held: Vec<f64>,
}

impl TurnCensus {
    /// One array slot per scheduled turn, keyed by the deck's card count.
    #[cfg(test)]
    pub(in crate::deck::simulator) fn new(turns: usize, deck_len: usize) -> Self {
        Self::with_commander_count(turns, deck_len, 0)
    }

    /// Allocate combat slots for every commander in the compiled deck.
    pub(in crate::deck::simulator) fn with_commander_count(
        turns: usize,
        deck_len: usize,
        commander_count: usize,
    ) -> Self {
        Self {
            land_drops: vec![0u8; turns],
            turn_observed: vec![false; turns],
            turn_observations: vec![super::super::game::TurnObservation::default(); turns],
            mana_available: vec![0.0f64; turns],
            mana_spent: vec![0.0f64; turns],
            mana_remaining: vec![0.0f64; turns],
            cards_seen: vec![0u32; turns],
            mana_ready: vec![None; deck_len],
            first_seen: HashMap::new(),
            first_creature: None,
            blocked_colors: [false; 5],
            commander_castable: None,
            station_online: None,
            companion_online: None,
            creatures: vec![0u32; turns],
            repeatable_sources_online: vec![0u32; turns],
            graveyard_size: vec![0u32; turns],
            pip_blocks: Vec::new(),
            attack_power: vec![0u32; turns],
            commander_combat_damage: vec![vec![0; turns]; commander_count],
            attackers_turn: vec![0u32; turns],
            evasive_turn: vec![0u32; turns],
            library_size: vec![0u32; turns],
            lands_seen: vec![0u32; turns],
            self_milled: vec![0u32; turns],
            opp_milled: vec![0u32; turns],
            awareness: vec![0.0f64; turns],
            opponent_life_loss: vec![0u32; turns],
            player_damage: vec![0u32; turns],
            extra_turns: vec![0u32; turns],
            win_threshold_turn: None,
            ultimate_online: None,
            interaction_ready: vec![false; turns],
            interaction_mana_held: vec![0.0f64; turns],
        }
    }
}

/// Record role sightings from the hand (role-access census).
pub(super) fn record_hand_sightings(
    deck: &SimDeck,
    st: &GameState,
    census: &mut TurnCensus,
    turn: usize,
) {
    let sightings = st
        .hand
        .iter()
        .map(|index| (index.index(), turn as u32))
        .chain(
            st.card_first_seen
                .iter()
                .map(|(index, seen_turn)| (*index, *seen_turn)),
        );
    for (index, seen_turn) in sightings {
        let card = &deck.cards[index];
        census
            .first_seen
            .entry(card.role)
            .and_modify(|first| *first = (*first).min(seen_turn))
            .or_insert(seen_turn);
        if card.is_creature {
            census.first_creature = Some(
                census
                    .first_creature
                    .map_or(seen_turn, |first| first.min(seen_turn)),
            );
        }
    }
}

/// 7b INTERACTION READINESS (measured, not forced): was an instant-speed
/// answer in hand and payable from the remaining general and instant/sorcery
/// mana? Test each answer because the cheapest one may not have payable pips.
pub(in crate::deck::simulator) fn record_interaction_readiness(
    deck: &SimDeck,
    st: &GameState,
    pool: &super::super::game::ManaPool,
    census: &mut TurnCensus,
    turn: usize,
) {
    let payable_answer = st
        .hand
        .iter()
        .filter(|i| {
            let c = &deck[**i];
            c.flags.is_interaction && c.flags.is_instant_speed
        })
        .filter_map(|index| {
            let cost = cast_cost(deck, st, &deck[*index], deck[*index].min_cost, Some(*index));
            let classes = cast_restrictions(&deck[*index]);
            let payment = payment_plan(&cost, pool, &classes)?;
            Some((cost, classes, payment))
        })
        .min_by_key(|(cost, _, _)| cost.total());
    let Some((_, classes, payment)) = payable_answer else {
        return;
    };
    let mut remaining = pool.clone();
    assert!(
        payment.commit(&mut remaining),
        "readiness uses an unchanged pool snapshot"
    );
    census.interaction_ready[turn - 1] = true;
    census.interaction_mana_held[turn - 1] = f64::from(usable_for_classes(&remaining, &classes));
}

/// Interaction capacity uses the same live generic costs as a normal cast.
#[cfg(test)]
#[path = "tests/interaction_cost_tests.rs"]
mod interaction_cost_tests;

/// 9 COMBAT: run the combat phase and record the per-turn census.
pub(super) fn record_combat(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    turn: usize,
) {
    let combat = super::super::game_combat::combat_phase(deck, st, turn, &census.land_drops);
    census.attack_power[turn - 1] = combat.power;
    for (slot, damage) in combat.commander_power.into_iter().enumerate() {
        if let Some(row) = census.commander_combat_damage.get_mut(slot) {
            row[turn - 1] = damage;
        }
    }
    census.attackers_turn[turn - 1] = combat.attackers;
    census.evasive_turn[turn - 1] = combat.evasive;
}

/// Record end-of-turn zones and cumulative output after end-step effects.
pub(super) fn record_end_turn(
    deck: &SimDeck,
    st: &GameState,
    census: &mut TurnCensus,
    turn: usize,
) {
    census.turn_observed[turn - 1] = true;
    census.turn_observations[turn - 1].end_turn = true;
    census.cards_seen[turn - 1] = st.seen;
    census.graveyard_size[turn - 1] = st.graveyard.len() as u32;
    census.library_size[turn - 1] = st.library.len() as u32;
    // Lands seen so far: hand + battlefield + graveyard. The flood
    // metric reads this (a land drawn and never dropped still floods; a
    // drop made is the wrong lens). The hypergeometric expectation
    // counts lands among everything seen, so the graveyard must be
    // included here too or discarded lands would hide real flood.
    census.lands_seen[turn - 1] = (st
        .hand
        .iter()
        .filter(|i| deck[**i].role == Role::Land)
        .count()
        + st.battlefield
            .iter()
            // Only deck cards can be lands.
            .filter(|p| p.card.deck_idx().is_some() && super::super::game::is_land_permanent(p))
            .count()
        + st.graveyard
            .iter()
            .filter(|i| deck[**i].role == Role::Land)
            .count()) as u32;
    census.self_milled[turn - 1] = st.milled_self;
    census.opp_milled[turn - 1] = st.milled_opp;
    census.awareness[turn - 1] =
        f64::from(st.awareness_cards) / (deck.library_len() as f64).max(1.0);
    census.opponent_life_loss[turn - 1] = st.opponent_life_lost;
    census.player_damage[turn - 1] = if turn > 1 {
        census.player_damage[turn - 2]
    } else {
        0
    } + census.attack_power[turn - 1]
        + st.damage_dealt_this_turn;
    census.creatures[turn - 1] = st
        .battlefield
        .iter()
        .filter(|p| is_creature_permanent(deck, p))
        .count() as u32;
}

/// Add damage from the opponent round after the player's turn census.
pub(in crate::deck::simulator) fn record_opponent_round_damage(
    census: &mut TurnCensus,
    turn: usize,
    damage_before: u32,
    damage_after: u32,
) {
    let opponent_damage = damage_after.saturating_sub(damage_before);
    census.player_damage[turn - 1] = census.player_damage[turn - 1].saturating_add(opponent_damage);
}

/// Record the engine count for the turn (called after the engine list
/// settles: commander cast, cast-phase registrations).
pub(super) fn record_repeatable_source_count(
    census: &mut TurnCensus,
    turn: usize,
    sources: &[(u32, u32)],
) {
    census.repeatable_sources_online[turn - 1] = sources.len() as u32;
}

/// Retain known land access when a loss prevents the fixed-window census.
fn fixed_window_lands_seen(
    deck: &SimDeck,
    st: &GameState,
    census: &TurnCensus,
    last: usize,
) -> u32 {
    if census.turn_observations[last].end_turn {
        return census.lands_seen[last];
    }
    let by = last as u32 + 1;
    let lands_seen = deck
        .cards
        .iter()
        .enumerate()
        .filter(|(index, card)| {
            card.role == Role::Land
                && (st
                    .card_first_seen
                    .get(index)
                    .is_some_and(|turn| *turn <= by)
                    || st
                        .battlefield_seen
                        .get(&CardIdx(*index as u32))
                        .is_some_and(|turn| *turn <= by)
                    || st
                        .graveyard_seen
                        .get(&CardIdx(*index as u32))
                        .is_some_and(|turn| *turn <= by))
        })
        .count() as u32;
    let observed_lands = census.lands_seen[..=last]
        .iter()
        .copied()
        .max()
        .expect("the fixed window contains at least one turn");
    lands_seen.max(observed_lands)
}

/// Build the final `GameLog` from the census and the game state.
pub(in crate::deck::simulator) fn finish_log(
    deck: &SimDeck,
    st: GameState,
    mut census: TurnCensus,
    opener_lands: u8,
    mulliganed: bool,
    turns: usize,
) -> GameLog {
    record_hand_sightings(deck, &st, &mut census, st.current_turn as usize);
    if st.completion.is_some() && st.current_turn > 0 {
        let slot = st.current_turn as usize - 1;
        if slot < turns && !census.turn_observations[slot].end_turn {
            census.opponent_life_loss[slot] = st.opponent_life_lost;
            let prior = if slot > 0 {
                census.player_damage[slot - 1]
            } else {
                0
            };
            census.player_damage[slot] = prior
                .saturating_add(census.attack_power[slot])
                .saturating_add(st.damage_dealt_this_turn);
        }
    }
    let milestones_by_turn = (1..=turns as u32)
        .map(|turn| {
            st.milestones_by_turn
                .get(&turn)
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    let lands_by_4: u8 = census.land_drops[..4.min(turns)].iter().sum();
    let last = 3.min(turns - 1);
    let cards_seen_by_4_value =
        if !census.turn_observations[last].end_turn && st.current_turn <= last as u32 + 1 {
            st.seen
        } else {
            census.cards_seen[last]
        };
    let lands_seen_by_4_value = fixed_window_lands_seen(deck, &st, &census, last);
    GameLog {
        completion: st
            .completion
            .unwrap_or(super::super::game::GameCompletion::HorizonReached),
        turns_observed: census
            .turn_observed
            .iter()
            .filter(|observed| **observed)
            .count() as u32,
        turn_observations: census.turn_observations,
        land_drops: census.land_drops,
        mana_available: census.mana_available,
        mana_spent: census.mana_spent,
        mana_remaining: census.mana_remaining,
        cards_seen: census.cards_seen,
        commander_castable: census.commander_castable,
        mana_ready: census.mana_ready,
        first_seen: census.first_seen,
        first_creature: census.first_creature,
        blocked_colors: census.blocked_colors,
        opener_lands,
        mulliganed,
        lands_by_4,
        lands_seen_by_turn_4: lands_seen_by_4_value,
        cards_seen_by_4: cards_seen_by_4_value,
        station_online: census.station_online,
        companion_online: census.companion_online,
        creatures: census.creatures,
        repeatable_sources_online: census.repeatable_sources_online,
        pip_blocks: census
            .pip_blocks
            .into_iter()
            .map(|(idx, ci)| (idx.index(), ci))
            .collect(),
        graveyard_size: census.graveyard_size,
        card_first_seen: st.card_first_seen,
        replay_casts: st.replay_casts,
        life_paid: st.life_paid,
        life_gained: st.life_gained,
        life_funded_draws: st.life_funded_draws,
        attack_power: census.attack_power,
        commander_combat_damage: census.commander_combat_damage,
        attackers: census.attackers_turn,
        evasive: census.evasive_turn,
        library_size: census.library_size,
        self_milled: census.self_milled,
        opp_milled: census.opp_milled,
        awareness: census.awareness,
        opponent_life_loss: census.opponent_life_loss,
        player_damage: census.player_damage,
        extra_turns: census.extra_turns,
        milestones_by_turn,
        win_threshold_turn: census.win_threshold_turn,
        ultimate_online: census.ultimate_online,
        interaction_ready: census.interaction_ready,
        interaction_mana_held: census.interaction_mana_held,
        card_first_battlefield: st
            .battlefield_seen
            .into_iter()
            .map(|(idx, turn)| (idx.index(), turn))
            .collect(),
        card_first_graveyard: st
            .graveyard_seen
            .into_iter()
            .map(|(idx, turn)| (idx.index(), turn))
            .collect(),
        infinite_mana_suspected: st.infinite_mana_suspected,
        opponent_activity: st.opponents.activity(),
    }
}

/// Whole-ability loss boundaries and terminal card-access reconciliation.
#[cfg(test)]
#[path = "tests/census_boundary_tests.rs"]
mod census_boundary_tests;

/// Discard excess hand cards using the highest-mana-value-first policy.
/// A visible front-face "no maximum hand size" static rule skips this discard.
/// Lasting hand-size spell effects need player state and remain unsupported.
/// Graveyard entry records the discard turn, not the game length.
pub(super) fn end_step_discard(deck: &SimDeck, st: &mut GameState, turn: usize) {
    if st.battlefield.iter().any(|permanent| {
        let card = super::super::game::card_of(deck, permanent);
        let front_is_land = card.card_types.iter().any(|kind| kind == "land");
        !permanent.face_down
            && (!card.is_mdfc_spell || permanent.land_face == front_is_land)
            && card.flags.no_max_hand_size
    }) {
        return;
    }
    while st.hand.len() > HAND_LIMIT {
        let worst = st
            .hand
            .iter()
            .enumerate()
            .max_by_key(|(_, idx)| deck[**idx].mana_value)
            .map(|(pos, _)| pos)
            .unwrap_or(st.hand.len() - 1);
        let discarded = st.hand.remove(worst);
        super::super::game_effects::move_to_graveyard(
            deck,
            st,
            discarded,
            turn as u32,
            super::super::game_effects::CardZone::Hand,
        );
    }
}
