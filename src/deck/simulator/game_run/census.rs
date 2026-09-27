//! Track per-turn game metrics and assemble the final game log.

use super::super::game::{GameLog, GameState, HAND_LIMIT, card_of};
use super::super::model::{CardIdx, Role, SimDeck};
use std::collections::HashMap;

/// Per-game census arrays the turn loop fills and the log carries.
pub(in crate::deck::simulator) struct TurnCensus {
    pub(in crate::deck::simulator) land_drops: Vec<u8>,
    pub(in crate::deck::simulator) mana_available: Vec<f64>,
    pub(in crate::deck::simulator) mana_spent: Vec<f64>,
    pub(in crate::deck::simulator) cards_seen: Vec<u32>,
    pub(in crate::deck::simulator) mana_ready: Vec<Option<u32>>,
    pub(in crate::deck::simulator) first_seen: HashMap<Role, u32>,
    pub(in crate::deck::simulator) first_creature: Option<u32>,
    pub(in crate::deck::simulator) blocked_colors: [bool; 5],
    pub(in crate::deck::simulator) commander_castable: Option<u32>,
    pub(in crate::deck::simulator) station_online: Option<u32>,
    pub(in crate::deck::simulator) bodies: Vec<u32>,
    pub(in crate::deck::simulator) engines_online: Vec<u32>,
    pub(in crate::deck::simulator) graveyard_size: Vec<u32>,
    pub(in crate::deck::simulator) card_first_seen: HashMap<usize, u32>,
    pub(in crate::deck::simulator) pip_blocks: Vec<(CardIdx, usize)>,
    pub(in crate::deck::simulator) attack_power: Vec<u32>,
    pub(in crate::deck::simulator) attackers_turn: Vec<u32>,
    pub(in crate::deck::simulator) evasive_turn: Vec<u32>,
    pub(in crate::deck::simulator) library_size: Vec<u32>,
    pub(in crate::deck::simulator) lands_seen: Vec<u32>,
    pub(in crate::deck::simulator) self_milled: Vec<u32>,
    pub(in crate::deck::simulator) opp_milled: Vec<u32>,
    pub(in crate::deck::simulator) awareness: Vec<f64>,
    pub(in crate::deck::simulator) drain_total: Vec<u32>,
    pub(in crate::deck::simulator) player_damage: Vec<u32>,
    pub(in crate::deck::simulator) extra_turns: Vec<u32>,
    pub(in crate::deck::simulator) win_threshold_turn: Option<u32>,
    pub(in crate::deck::simulator) ultimate_online: Option<u32>,
    pub(in crate::deck::simulator) interaction_ready: Vec<bool>,
    pub(in crate::deck::simulator) interaction_mana_held: Vec<f64>,
}

impl TurnCensus {
    /// One array slot per scheduled turn, keyed by the deck's card count.
    pub(in crate::deck::simulator) fn new(turns: usize, deck_len: usize) -> Self {
        Self {
            land_drops: vec![0u8; turns],
            mana_available: vec![0.0f64; turns],
            mana_spent: vec![0.0f64; turns],
            cards_seen: vec![0u32; turns],
            mana_ready: vec![None; deck_len],
            first_seen: HashMap::new(),
            first_creature: None,
            blocked_colors: [false; 5],
            commander_castable: None,
            station_online: None,
            bodies: vec![0u32; turns],
            engines_online: vec![0u32; turns],
            graveyard_size: vec![0u32; turns],
            card_first_seen: HashMap::new(),
            pip_blocks: Vec::new(),
            attack_power: vec![0u32; turns],
            attackers_turn: vec![0u32; turns],
            evasive_turn: vec![0u32; turns],
            library_size: vec![0u32; turns],
            lands_seen: vec![0u32; turns],
            self_milled: vec![0u32; turns],
            opp_milled: vec![0u32; turns],
            awareness: vec![0.0f64; turns],
            drain_total: vec![0u32; turns],
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
    for i in &st.hand {
        let card = &deck[*i];
        census.first_seen.entry(card.role).or_insert(turn as u32);
        if card.is_creature && census.first_creature.is_none() {
            census.first_creature = Some(turn as u32);
        }
        census
            .card_first_seen
            .entry(i.index())
            .or_insert(turn as u32);
    }
}

/// 7b INTERACTION READINESS (measured, not forced): was instant-speed
/// interaction in hand while spare mana covered its cost? The cheapest
/// answer in hand decides; the goldfish never spends it.
pub(super) fn record_interaction_readiness(
    deck: &SimDeck,
    st: &GameState,
    pool: &super::super::game::Pool,
    census: &mut TurnCensus,
    turn: usize,
) {
    let cheapest = st
        .hand
        .iter()
        .filter_map(|i| {
            let c = &deck[*i];
            (c.is_interaction && c.is_instant_speed).then(|| c.min_cost.total())
        })
        .min();
    if let Some(cheapest) = cheapest
        && pool.total() >= cheapest
    {
        census.interaction_ready[turn - 1] = true;
        census.interaction_mana_held[turn - 1] = f64::from(pool.total() - cheapest);
    }
}

/// 9 COMBAT: run the combat phase and record the per-turn census.
pub(super) fn record_combat(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    turn: usize,
) {
    let combat = super::super::game_combat::combat_phase(deck, st, turn, &census.land_drops);
    census.attack_power[turn - 1] = combat.power;
    census.attackers_turn[turn - 1] = combat.attackers;
    census.evasive_turn[turn - 1] = combat.evasive;
    let token_bodies = combat.token_bodies;
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
            // Token permanents carry sentinel card indexes; only real
            // cards can be lands.
            .filter(|p| {
                p.card
                    .deck_idx()
                    .is_some_and(|idx| deck[idx].role == Role::Land)
            })
            .count()
        + st.graveyard
            .iter()
            .filter(|i| deck[**i].role == Role::Land)
            .count()) as u32;
    census.self_milled[turn - 1] = st.milled_self;
    census.opp_milled[turn - 1] = st.milled_opp;
    census.awareness[turn - 1] = f64::from(st.awareness_cards) / (deck.cards.len() as f64).max(1.0);
    census.drain_total[turn - 1] = st.drained;
    census.player_damage[turn - 1] = if turn > 1 {
        census.player_damage[turn - 2]
    } else {
        0
    } + combat.power;
    census.bodies[turn - 1] = st
        .battlefield
        .iter()
        .filter(|p| card_of(deck, p).is_creature || p.animated)
        .count() as u32
        + token_bodies.min(4);
}

/// Record the engine count for the turn (called after the engine list
/// settles: commander cast, cast-phase registrations).
pub(super) fn record_engine_count(census: &mut TurnCensus, turn: usize, engines: &[(u32, u32)]) {
    census.engines_online[turn - 1] = engines.len() as u32;
}

/// Build the final `GameLog` from the census and the game state.
pub(super) fn finish_log(
    st: GameState,
    census: TurnCensus,
    opener_lands: u8,
    mulliganed: bool,
    turns: usize,
) -> GameLog {
    let milestones_by_turn = (1..=turns as u32)
        .map(|turn| {
            st.milestones_by_turn
                .get(&turn)
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    let lands_by_4: u8 = census.land_drops[..4.min(turns)].iter().sum();
    // The "by turn 4" fields carry the real last-turn value when the
    // schedule is shorter than 4 turns; the aggregate only reads them
    // under its `turns >= 4` guard. Both fields follow the same rule,
    // so the short-schedule value is the last census entry for both.
    let last = 3.min(turns - 1);
    let cards_seen_by_4_value = census.cards_seen[last];
    let lands_seen_by_4_value = census.lands_seen[last];
    GameLog {
        land_drops: census.land_drops,
        mana_available: census.mana_available,
        mana_spent: census.mana_spent,
        cards_seen: census.cards_seen,
        commander_castable: census.commander_castable,
        mana_ready: census.mana_ready,
        first_seen: census.first_seen,
        first_creature: census.first_creature,
        blocked_colors: census.blocked_colors,
        opener_lands,
        mulliganed,
        lands_by_4,
        // End of turn 4: the flood window. Draw engines widen the window
        // past the nominal 11 (opener + 4 draws); the log carries the
        // real seen count so the expectation matches the measurement.
        // Short schedules (fewer than 4 turns) report 0: the flood
        // bucket only reads this field under a `turns >= 4` guard.
        lands_seen_by_11: lands_seen_by_4_value,
        cards_seen_by_4: cards_seen_by_4_value,
        station_online: census.station_online,
        bodies: census.bodies,
        engines_online: census.engines_online,
        pip_blocks: census
            .pip_blocks
            .into_iter()
            .map(|(idx, ci)| (idx.index(), ci))
            .collect(),
        graveyard_size: census.graveyard_size,
        card_first_seen: census.card_first_seen,
        replay_casts: st.replay_casts,
        life_paid: st.life_paid,
        life_funded_draws: st.life_funded_draws,
        attack_power: census.attack_power,
        attackers: census.attackers_turn,
        evasive: census.evasive_turn,
        library_size: census.library_size,
        self_milled: census.self_milled,
        opp_milled: census.opp_milled,
        awareness: census.awareness,
        drain_total: census.drain_total,
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
        #[cfg(test)]
        alternate_casts: st.alternate_casts.iter().map(|idx| idx.index()).collect(),
        infinite_mana_suspected: st.infinite_mana_suspected,
    }
}

/// 10 END: hand-limit discard from the end of the hand. The graveyard
/// entry records THIS turn (the discard turn), not the game length.
pub(super) fn end_step_discard(deck: &SimDeck, st: &mut GameState, turn: usize) {
    while st.hand.len() > HAND_LIMIT {
        let discarded = st.hand.remove(st.hand.len() - 1);
        super::super::game_effects::move_to_graveyard(
            deck,
            st,
            discarded,
            turn as u32,
            super::super::game_effects::CardZone::Hand,
        );
    }
}
