//! Track per-turn game metrics and assemble the final game log.

use super::super::game::{GameLog, GameState, HAND_LIMIT, is_creature_permanent};
use super::super::game_mana::usable_for_noncreature;
use super::super::model::{CardIdx, Role, SimDeck};
use std::collections::HashMap;

/// Per-game census arrays the turn loop fills and the log carries.
pub(in crate::deck::simulator) struct TurnCensus {
    /// Land drops made each turn (0-3).
    pub(in crate::deck::simulator) land_drops: Vec<u8>,
    /// Spendable mana at the first main phase of each turn.
    pub(in crate::deck::simulator) mana_available: Vec<f64>,
    /// Mana spent on casts each turn.
    pub(in crate::deck::simulator) mana_spent: Vec<f64>,
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
    pub(in crate::deck::simulator) card_first_seen: HashMap<usize, u32>,
    /// (card index, color index) pairs pip-blocked this game, one row per
    /// blocked cast attempt.
    pub(in crate::deck::simulator) pip_blocks: Vec<(CardIdx, usize)>,
    /// Total attacking power on the board at the combat phase of each turn.
    pub(in crate::deck::simulator) attack_power: Vec<u32>,
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
            companion_online: None,
            creatures: vec![0u32; turns],
            repeatable_sources_online: vec![0u32; turns],
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
/// answer in hand decides; the goldfish never spends it. Instant-speed
/// answers spend the general pool plus the instant/sorcery bucket, and
/// a bucket pip is one mana of the source's chosen color, so the
/// bucket + general-covered pips must reach the pip total (the cast
/// gate's mixed-pip rule).
pub(in crate::deck::simulator) fn record_interaction_readiness(
    deck: &SimDeck,
    st: &GameState,
    pool: &super::super::game::ManaPool,
    census: &mut TurnCensus,
    turn: usize,
) {
    let cheapest = st
        .hand
        .iter()
        .filter(|i| {
            let c = &deck[**i];
            c.flags.is_interaction && c.flags.is_instant_speed
        })
        .min_by_key(|i| deck[**i].min_cost.total());
    let Some(card_idx) = cheapest else {
        return;
    };
    let cost = &deck[*card_idx].min_cost;
    let general = usable_for_noncreature(pool);
    let bucket = pool.instant_sorcery_only;
    if general + bucket < cost.total() {
        return;
    }
    let pip_total: u32 = cost.pips.iter().map(|p| u32::from(*p)).sum::<u32>() + cost.hybrid_pips;
    // Pips the general pool covers: fixed pips in place, flexible
    // filling the rest.
    let mut flexible = pool.flexible;
    let mut covered = 0u32;
    for (i, need) in cost.pips.iter().enumerate() {
        let need = u32::from(*need);
        covered += need.min(pool.fixed[i]);
        flexible = flexible.saturating_sub(need.saturating_sub(pool.fixed[i]));
    }
    covered += flexible;
    if bucket > 0 && bucket + covered < pip_total {
        return;
    }
    census.interaction_ready[turn - 1] = true;
    census.interaction_mana_held[turn - 1] = f64::from(general + bucket - cost.total());
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
    census.awareness[turn - 1] =
        f64::from(st.awareness_cards) / (deck.library_len() as f64).max(1.0);
    census.opponent_life_loss[turn - 1] = st.opponent_life_lost;
    census.player_damage[turn - 1] = if turn > 1 {
        census.player_damage[turn - 2]
    } else {
        0
    } + combat.power
        + st.damage_dealt_this_turn;
    census.creatures[turn - 1] = st
        .battlefield
        .iter()
        .filter(|p| is_creature_permanent(deck, p))
        .count() as u32;
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
        card_first_seen: census.card_first_seen,
        replay_casts: st.replay_casts,
        life_paid: st.life_paid,
        life_gained: st.life_gained,
        life_funded_draws: st.life_funded_draws,
        attack_power: census.attack_power,
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
    }
}

/// 10 END: hand-limit discard. The best-case agent keeps the cards it
/// wants most, so it sheds the highest-mana-value cards first (CR 402.2
/// leaves the choice to the player; the sim approximates it). A board
/// permanent with "You have no maximum hand size" skips the discard
/// entirely. The graveyard entry records THIS turn (the discard turn),
/// not the game length.
pub(super) fn end_step_discard(deck: &SimDeck, st: &mut GameState, turn: usize) {
    if st
        .battlefield
        .iter()
        .any(|p| super::super::game::card_of(deck, p).flags.no_max_hand_size)
    {
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
