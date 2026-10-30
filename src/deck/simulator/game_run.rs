//! The per-game turn loop for the goldfish simulator.
//! `run_game` owns game state and seeded random streams. Phase helpers perform
//! the supported turn actions without database, network, or terminal I/O.

use super::cast_pass::{cast_pass, play_land};
use super::deal::{Opener, deal_opener};
use super::game::{
    GameLog, GameState, ManaPool, Permanent, card_of, fire_on_enter, fire_triggers, new_perm_with,
    take_uid,
};
use super::game_commander::CommanderProfile;
use super::game_effects::{spend_leftover, tap_budget};
use super::game_mana::add_yield_turns;
use super::model::{CardIdx, Scale, SimDeck, SimEffect, SimTrigger};
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

/// Per-turn census and end-of-game log assembly.
pub(in crate::deck::simulator) mod census;
/// The commander cast phase.
#[path = "game_run/commander.rs"]
mod commander;
/// Mana-pool building (taps, gates, banked mana, grants).
#[path = "game_run/mana.rs"]
pub(in crate::deck::simulator) mod mana;
/// Plot payments and later free casts from exile.
mod plot;
use plot::{cast_plotted_cards, plot_unaffordable_cards};

pub(in crate::deck::simulator) use commander::commander_phase;
pub(in crate::deck::simulator) use mana::{
    build_pool, fire_tapped_for_mana_triggers, grants_active, tap_dorks_for_mana, tap_new_rocks,
};
#[path = "game_run/sagas.rs"]
mod sagas;
pub(super) use sagas::run_sagas;
/// Upkeep effect scopes and scaled draw rewards.
mod upkeep;
#[cfg(test)]
pub(super) use upkeep::run_scaling_draw;
/// Side-effect-free evidence that a main-phase work cap left a legal action.
mod work;
use upkeep::run_upkeep;
use work::remaining_main_work;

/// 8 THRESHOLD: station tiers unlock (permanent for animate tiers).
fn unlock_thresholds(deck: &SimDeck, st: &mut GameState, census: &mut TurnCensus, turn: usize) {
    for perm in st.battlefield.iter_mut() {
        let card = card_of(deck, perm);
        if card.is_station_card
            && let Some(at) = card.animate_at()
            && !perm.animated
            && perm.counters.charge >= at
        {
            perm.animated = true;
            if matches!(perm.card, super::game::CardRef::Commander { .. })
                && census.station_online.is_none()
            {
                census.station_online = Some(turn as u32);
            }
        }
    }
}

pub(super) use census::TurnCensus;

/// Play one goldfish game and return its log. Pure apart from the
/// passed RNG: same deck + same seed = same game. The log carries the
/// per-turn census the aggregate/report layers read.
#[cfg(test)]
pub fn run_game(deck: &SimDeck, rng: &mut ChaCha8Rng, turns: u32) -> GameLog {
    let library_seed = rand::RngCore::next_u64(&mut rng.clone());
    run_game_with_config(
        deck,
        rng,
        turns,
        None,
        super::rng::game_rng(library_seed, 0, super::rng::RngDomain::Library),
    )
}

/// Play one game with optional external-opponent settings and the supplied RNG.
pub fn run_game_with_config(
    deck: &SimDeck,
    rng: &mut ChaCha8Rng,
    turns: u32,
    opponents: Option<&super::opponents::OpponentConfig>,
    library_rng: ChaCha8Rng,
) -> GameLog {
    let turns = turns.max(1) as usize;
    let opener = deal_opener(deck, rng);
    let Opener {
        hand,
        library,
        lands: opener_lands,
        mulliganed,
        ..
    } = opener;

    let mut st = GameState {
        completion: None,
        failed_draw: false,
        current_turn: 1,
        current_phase: super::game::GamePhase::Upkeep,
        resolution: Default::default(),
        library_rng,
        battlefield: Vec::new(),
        library,
        hand,
        seen: 0,
        graveyard: Vec::new(),
        graveyard_generations: HashMap::new(),
        exile: Vec::new(),
        battlefield_seen: HashMap::new(),
        graveyard_seen: HashMap::new(),
        card_first_seen: HashMap::new(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        opponent_life_lost: 0,
        damage_dealt_this_turn: 0,
        life_gained: 0,
        flashback_permissions: std::collections::HashSet::new(),
        replay_casts: 0,
        milestones_by_turn: HashMap::new(),
        life_paid: 0,
        life_funded_draws: 0,
        life: deck.rules.starting_life,
        is_monarch: false,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
        player_counters: super::model::PlayerCounters::default(),
        activated_this_turn: std::collections::HashSet::new(),
        activated_once: std::collections::HashSet::new(),
        triggered_this_turn: std::collections::HashSet::new(),
        companion_fetched: false,
        ring_tempts: 0,
        extra_land_drops_this_turn: 0,
        spells_cast_this_turn: 0,
        attacked_this_turn: false,
        plotted: Vec::new(),
        ring_bearer: None,
        commander_casts: vec![0; deck.commanders.len()],
        opponents: super::opponents::OpponentTable::for_rules(deck.rules),
    };
    if let Some(config) = opponents {
        st.opponents.config = config.clone();
    }
    let starting_hand = st.hand.clone();
    for index in starting_hand {
        super::game::record_card_seen(&mut st, index, 1);
    }
    apply_leylines(deck, &mut st);

    let mut census =
        TurnCensus::with_commander_count(turns, deck.cards.len(), deck.commanders.len());
    let commander = CommanderProfile::new(deck);
    // Repeatable sources in play: (permanent uid, upkeep draw count). The uid
    // resolves to a battlefield position at fire time, so removals
    // (sacrifice outlets, saga completion) never alias another card.
    let mut repeatable_sources: Vec<(u32, u32)> = Vec::new();
    let mut pending_extra_turns = 0u32;
    for turn in 1..=turns {
        let is_extra_turn = pending_extra_turns > 0;
        pending_extra_turns = pending_extra_turns.saturating_sub(1);
        run_turn(
            deck,
            &mut st,
            &mut census,
            &commander,
            &mut repeatable_sources,
            turn,
            is_extra_turn,
        );
        if st.completion.is_some() {
            break;
        }
        pending_extra_turns = pending_extra_turns.saturating_add(st.extra_turns_queued);
        st.extra_turns_queued = 0;
        if turn < turns && pending_extra_turns == 0 {
            st.current_phase = super::game::GamePhase::OpponentRound;
            st.current_turn = turn as u32;
            let damage_before = st.damage_dealt_this_turn;
            super::opponents::run_opponent_round(deck, &mut st, turn as u32);
            census::record_opponent_round_damage(
                &mut census,
                turn,
                damage_before,
                st.damage_dealt_this_turn,
            );
            census.opponent_life_loss[turn - 1] = st.opponent_life_lost;
            if st.completion.is_some() {
                break;
            }
        }
    }

    census::finish_log(deck, st, census, opener_lands, mulliganed, turns)
}

/// Play one scheduled turn through the full pipeline.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_turn(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    commander: &CommanderProfile,
    repeatable_sources: &mut Vec<(u32, u32)>,
    turn: usize,
    is_extra_turn: bool,
) {
    if st.completion.is_some() {
        return;
    }
    st.current_turn = turn as u32;
    beginning_phase(deck, st, census, repeatable_sources, turn);
    if st.completion.is_some() {
        return;
    }
    precombat_main_phase(deck, st, census, commander, repeatable_sources, turn);
    if st.completion.is_some() {
        return;
    }
    // State-based action (CR 704.5j): only one legendary permanent of
    // each name survives before the combat census reads the board.
    super::game_effects::enforce_legend_rule(deck, st, turn as u32);
    combat_phase(deck, st, census, repeatable_sources, turn);
    if st.completion.is_some() {
        return;
    }
    postcombat_main_phase();
    ending_phase(deck, st, census, turn, is_extra_turn);
    if st.completion.is_some() {
        return;
    }
    census::record_hand_sightings(deck, st, census, turn);
    census::record_end_turn(deck, st, census, turn);
}

/// Run untap, upkeep, and draw at the start of a scheduled turn.
fn beginning_phase(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    repeatable_sources: &mut Vec<(u32, u32)>,
    turn: usize,
) {
    st.current_phase = super::game::GamePhase::Upkeep;
    // 1 UNTAP: everything untaps; sickness clears; once-per-turn resets.
    // Crew animations expire (Vehicles stop being bodies at end of
    // turn). Blink flags re-fire the host's Enters triggers once
    // (Skyskipper Duo, Conjurer's Closet-style flickers).
    st.flashback_permissions.clear();
    expire_crew(st);
    for perm in st.battlefield.iter_mut() {
        // "Doesn't untap during your untap step" statics stay tapped
        // (Basalt Monolith class); only an untap activation clears the
        // tap. Every other permanent untaps normally.
        if !card_of(deck, perm).flags.doesnt_untap {
            perm.tapped = false;
        }
        perm.summoning_sick = false;
        perm.fired = false;
        perm.prowess_bonus = 0;
    }
    st.prowess_casts = 0;
    st.damage_dealt_this_turn = 0;
    st.spells_cast_this_turn = 0;
    st.extra_land_drops_this_turn = 0;
    st.attacked_this_turn = false;
    st.activated_this_turn.clear();
    st.triggered_this_turn.clear();
    fire_returned_triggers(deck, st, turn as u32);
    if st.completion.is_some() {
        return;
    }

    // Upkeep triggers fire and win checks run. Saga chapters advance in
    // the precombat main phase (CR 714.3c).
    super::opponents::pay_cumulative_upkeep(deck, st, turn as u32);
    if st.completion.is_some() {
        return;
    }
    run_upkeep(deck, st, repeatable_sources, turn);
    if st.completion.is_some() {
        return;
    }
    check_win_thresholds(deck, st, census, turn);
    if st.completion.is_some() {
        return;
    }
    check_ultimates(deck, st, census, turn);
    if st.completion.is_some() {
        return;
    }

    // 3 DRAW.
    st.current_phase = super::game::GamePhase::Draw;
    let draws_on_turn = deck.rules.shape == super::model::Format::Commander || turn > 1;
    if draws_on_turn && st.completion.is_none() {
        super::game_effects::draw_one(deck, st, turn as u32);
    }
    census.turn_observations[turn - 1].draw = st.completion.is_none();
}

/// Run Saga, land, cast, activation, station, and tap-budget work in the
/// precombat main phase.
#[allow(clippy::too_many_arguments)]
fn precombat_main_phase(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    commander: &CommanderProfile,
    repeatable_sources: &mut Vec<(u32, u32)>,
    turn: usize,
) {
    st.current_phase = super::game::GamePhase::Main;
    // Saga lore counters are added as the precombat main phase begins
    // (CR 714.3c), before the main-phase triggers and land drops; the
    // entry chapter fired at cast time.
    run_sagas(deck, st, turn);
    if st.completion.is_some() {
        return;
    }
    fire_triggers(deck, st, SimTrigger::PrecombatMain, turn as u32);
    if st.completion.is_some() {
        return;
    }
    census::record_hand_sightings(deck, st, census, turn);

    // 4 LAND.
    play_land_drops(deck, st, census, turn);
    if st.completion.is_some() {
        return;
    }

    // 5 POOL, companion fetch, commander cast, casts, 6 ACTIVATE,
    // 7 TAP BUDGET.
    let mut pool = build_pool(deck, st, turn as u32);
    if st.completion.is_some() {
        return;
    }
    // PLOT (CR 702.170): exile a hand card with a plot cost when the
    // card is otherwise unaffordable; from a later turn it casts for
    // free. Plot spends mana from this turn's pool.
    plot_unaffordable_cards(deck, st, &mut pool, turn);
    if st.completion.is_some() {
        return;
    }
    // Free-cast plotted cards from a later turn than they were plotted.
    cast_plotted_cards(
        deck,
        st,
        &mut pool,
        turn,
        &mut census.mana_spent,
        repeatable_sources,
    );
    if st.completion.is_some() {
        return;
    }
    fetch_companion(deck, st, &mut pool, census, turn);
    if st.completion.is_some() {
        return;
    }
    commander_phase(
        deck,
        st,
        &mut pool,
        census,
        commander,
        turn,
        repeatable_sources,
        true,
    );
    // Revisit casts after draw and mana actions. Cheapest legal casts run
    // first; newly cast rocks activate before the next cast sweep. The
    // shared pass cap protects the turn loop from unsupported repeatable
    // effects.
    let mut activations_done = false;
    let mut settled = false;
    let mut repeated_mana_passes = 0u32;
    for _ in 0..super::model::MAX_LOOP_PASSES {
        if st.completion.is_some() {
            return;
        }
        pool.treasures += std::mem::take(&mut st.treasure_bank);
        let before_mana = pool.total();
        let before_hand = st.hand.clone();
        let before_board = st.battlefield.len();
        play_late_land(deck, st, census, turn);
        cast_pass(
            deck,
            st,
            &mut pool,
            turn,
            &mut census.mana_spent,
            repeatable_sources,
            &mut census.pip_blocks,
            &mut census.blocked_colors,
        );
        if st.completion.is_some() {
            return;
        }
        tap_dorks_for_mana(deck, st, &mut pool, turn as u32);
        if st.completion.is_some() {
            return;
        }
        tap_new_rocks(deck, st, &mut pool, turn as u32);
        if st.completion.is_some() {
            return;
        }
        if pool.total() > before_mana
            && before_hand == st.hand
            && before_board == st.battlefield.len()
        {
            repeated_mana_passes += 1;
        }
        if before_mana != pool.total()
            || before_hand != st.hand
            || before_board != st.battlefield.len()
        {
            activations_done = false;
            continue;
        }
        commander_phase(
            deck,
            st,
            &mut pool,
            census,
            commander,
            turn,
            repeatable_sources,
            false,
        );
        if st.completion.is_some() {
            return;
        }
        if before_mana != pool.total()
            || before_hand != st.hand
            || before_board != st.battlefield.len()
        {
            activations_done = false;
            continue;
        }
        if !activations_done {
            spend_leftover(deck, st, &mut pool, turn as u32);
            if st.completion.is_some() {
                return;
            }
            activations_done = true;
            continue;
        }
        settled = true;
        break;
    }
    if !settled
        && st.completion.is_none()
        && remaining_main_work(deck, st, &pool, census, commander, repeatable_sources, turn)
    {
        if repeated_mana_passes > 1 {
            st.infinite_mana_suspected = true;
            super::game::milestone_for_turn(st, turn as u32).positive_mana_loop = true;
        }
        super::resolution::mark_incomplete(
            st,
            super::game::IncompleteReason::TurnAction {
                configured_limit: super::model::MAX_LOOP_PASSES,
                remaining_work: 1,
            },
        );
        return;
    }
    tap_budget(deck, &mut st.battlefield, &mut pool, &st.hand);
    census::record_interaction_readiness(deck, st, &pool, census, turn);
    census.mana_remaining[turn - 1] = pool.mana_total() as f64;
    st.treasure_bank += pool.treasures;

    // 8 THRESHOLD: station tiers unlock (permanent for animate tiers).
    unlock_thresholds(deck, st, census, turn);
    census.turn_observations[turn - 1].main = st.completion.is_none();
}

/// Resolve combat and record its turn metrics.
fn combat_phase(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    repeatable_sources: &[(u32, u32)],
    turn: usize,
) {
    st.current_phase = super::game::GamePhase::Combat;
    // 9 COMBAT.
    census::record_combat(deck, st, census, turn);
    census.turn_observations[turn - 1].combat = st.completion.is_none();
    // Engine count for the turn (commander + cast-phase registrations).
    census::record_repeatable_source_count(census, turn, repeatable_sources);
}

/// Keep the postcombat main phase explicit; the action policy has no
/// postcombat actions yet.
fn postcombat_main_phase() {}

#[cfg(test)]
#[path = "tests/game_run_runtime_tests.rs"]
mod game_run_runtime_tests;

/// Source-cost and last-known-characteristic controls.
#[cfg(test)]
#[path = "tests/mana_source_tests.rs"]
mod mana_source_tests;

/// Run end-step triggers and cleanup actions.
pub(super) fn ending_phase(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    turn: usize,
    is_extra_turn: bool,
) {
    st.current_phase = super::game::GamePhase::EndStep;
    // 10 END: beginning-of-end-step triggers, then hand-limit discard.
    // Monarch: one extra card at the beginning of the Monarch's end
    // step (CR 725.2), including the turn the player becomes monarch.
    if st.is_monarch {
        super::game_effects::draw_one(deck, st, turn as u32);
    }
    if st.completion.is_some() {
        return;
    }
    fire_triggers(deck, st, SimTrigger::EndStep, turn as u32);
    if st.completion.is_some() {
        return;
    }
    // Mobilize tokens are sacrificed at the beginning of the next end
    // step (CR 702.181a). The sacrifice fires death triggers like any
    // other sacrifice.
    sacrifice_end_step_tokens(deck, st, turn);
    if st.completion.is_some() {
        return;
    }
    // Saddle and crew both expire at end of turn (CR 702.171b).
    expire_saddle(st);
    census::end_step_discard(deck, st, turn);
    if st.completion.is_none() {
        for permanent in &mut st.battlefield {
            permanent.prowess_bonus = 0;
        }
    }
    census.extra_turns[turn - 1] = u32::from(is_extra_turn);
    census.turn_observations[turn - 1].end_step = st.completion.is_none();
}

/// Sacrifice permanents marked for the end step (mobilize's Warrior
/// tokens): remove them, move deck cards to the graveyard, and fire the
/// surviving board's death triggers. Tokens leave without a graveyard
/// entry (tokens cease to exist), but the sacrifice still triggers.
fn sacrifice_end_step_tokens(deck: &SimDeck, st: &mut GameState, turn: usize) {
    let victims: Vec<_> = st
        .battlefield
        .iter()
        .filter(|permanent| permanent.sacrifice_at_end)
        .map(|permanent| permanent.uid)
        .collect();
    super::game_effects::resolve_sacrifice_batch(deck, st, turn as u32, &victims);
}

/// Saddled is a until-end-of-turn designation (CR 702.171b).
fn expire_saddle(st: &mut GameState) {
    for perm in st.battlefield.iter_mut() {
        perm.saddled = false;
    }
}

/// Fetch the companion (CR 702.139a, 116.2g): once per game, in a main
/// phase, pay {3} to put the companion into the hand. Best case: as soon
/// as {3} is available. The condition is assumed met, documented in the
/// output assumptions.
fn fetch_companion(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    census: &mut TurnCensus,
    turn: usize,
) {
    if st.companion_fetched {
        return;
    }
    let Some(companion) = deck.companion else {
        return;
    };
    let cost = super::model::Cost {
        generic: 3,
        ..super::model::Cost::default()
    };
    if !super::game_mana::payable(&cost, pool) {
        return;
    }
    super::game_mana::pay_cost(&cost, pool);
    st.companion_fetched = true;
    super::game::record_card_seen(st, companion, turn as u32);
    st.hand.push(companion);
    census.companion_online = census.companion_online.or(Some(turn as u32));
}

/// Play a land made accessible during the main phase when the turn's land
/// play allowance has not been used. Cast effects can add to that allowance.
pub(super) fn play_late_land(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    turn: usize,
) {
    let limit = land_play_allowance(deck, st);
    while u32::from(census.land_drops[turn - 1]) < limit
        && super::cast_pass::play_land(deck, st, turn as u32)
    {
        census.land_drops[turn - 1] += 1;
    }
}

/// Tap a permanent for its parsed mana and resolve player-controlled
/// bonuses that trigger from a nonland mana tap.
pub(super) fn add_nonland_mana(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    yield_: &super::model::ManaYield,
    pool: &mut ManaPool,
    turn: u32,
) -> bool {
    let land = super::game::is_land_permanent(source);
    let output = resolved_source_yield(deck, st, source, yield_, false);
    let bonus_triggers = if land {
        0
    } else {
        st.battlefield
            .iter()
            .filter(|permanent| {
                let card = card_of(deck, permanent);
                !permanent.face_down
                    && !(permanent.land_face && card.is_mdfc_spell)
                    && card.flags.bonus_mana_on_nonland_tap
            })
            .count() as u32
    };
    let mana_before = pool.total();
    let produced_types = if bonus_triggers > 0 && !output.opponent_any {
        mana::add_chosen_source_yield(deck, st, &output, pool)
    } else {
        add_yield_turns(deck, &output, pool, turn, &st.battlefield);
        0
    };
    if pool.total() == mana_before {
        return false;
    }
    if bonus_triggers == 0 {
        return true;
    }
    if output.opponent_any {
        pool.generic_credit += bonus_triggers;
        return true;
    }
    pool.lots.push(super::game_mana::ManaLot {
        amount: bonus_triggers,
        types: produced_types,
        restriction: None,
        cannot_pay_generic: false,
        one_color: false,
    });
    true
}

/// Choose the plain granted output or resolve the native mode's counters and colors.
fn resolved_source_yield(
    deck: &SimDeck,
    st: &GameState,
    source: &Permanent,
    yield_: &super::model::ManaYield,
    use_granted_mode: bool,
) -> super::model::ManaYield {
    if use_granted_mode {
        return super::model::ManaYield {
            any_pips: 1,
            ..Default::default()
        };
    }
    let mut output = yield_.clone();
    if output.scaling == Some(Scale::PerChargeCounter) {
        output.any_pips = source.counters.charge;
        output.scaling = None;
    }
    if output.legendary_colors {
        output.choice = [false; 5];
        for permanent in &st.battlefield {
            if permanent.face_down {
                continue;
            }
            let card = card_of(deck, permanent);
            if card.is_legendary && (card.is_creature || card.is_planeswalker) {
                for (choice, color) in output.choice.iter_mut().zip(card.colors) {
                    *choice |= color;
                }
            }
        }
        output.any_pips = 0;
        output.legendary_colors = false;
    }
    if output.scaling == Some(Scale::ColorsPresent) {
        let mut colors = [false; 5];
        for permanent in st
            .battlefield
            .iter()
            .filter(|permanent| !permanent.face_down)
        {
            for (present, color) in colors.iter_mut().zip(card_of(deck, permanent).colors) {
                *present |= color;
            }
        }
        for (amount, present) in output.fixed.iter_mut().zip(colors) {
            *amount += u8::from(present);
        }
        output.scaling = None;
    }
    output
}

/// Leyline-style openers begin the game on the battlefield; the moved
/// cards count as seen (they left the hand zone).
fn apply_leylines(deck: &SimDeck, st: &mut GameState) {
    let mut leyline_idx: Vec<CardIdx> = Vec::new();
    st.hand.retain(|idx| {
        if deck[*idx].opens_in_play {
            leyline_idx.push(*idx);
            false
        } else {
            true
        }
    });
    for idx in leyline_idx {
        st.battlefield_seen.entry(idx).or_insert(0);
        let uid = take_uid(st);
        st.battlefield.push(new_perm_with(uid, deck, idx, 0, false));
    }
    st.seen = (st.hand.len() + st.battlefield.len()) as u32;
}

/// Crew animations revert at the turn boundary: a crewed Vehicle stops
/// being a body (and stops attacking) once the turn closes.
pub(super) fn expire_crew(st: &mut GameState) {
    for perm in st.battlefield.iter_mut() {
        perm.crewed = false;
    }
}

/// Re-fire the Enters triggers of each returned permanent once
/// (the deferred firing never re-arms: one re-fire per entry).
fn fire_returned_triggers(deck: &SimDeck, st: &mut GameState, turn: u32) {
    let returned_uids: Vec<u32> = st
        .battlefield
        .iter()
        .filter(|p| p.returned_trigger_pending)
        .map(|p| p.uid)
        .collect();
    for uid in returned_uids {
        if st.completion.is_some() {
            return;
        }
        let Some(i) = st.battlefield.iter().position(|perm| perm.uid == uid) else {
            continue;
        };
        st.battlefield[i].returned_trigger_pending = false;
        fire_on_enter(deck, st, i, turn, true);
    }
}

/// Saga chapters advance one per turn after their entry chapter. The
/// sim adds lore counters in the precombat main phase (CR 714.3c);
/// `run_turn` calls this as the precombat main phase begins.
/// Win-threshold engines: enough counters at upkeep wins.
fn check_win_thresholds(deck: &SimDeck, st: &GameState, census: &mut TurnCensus, turn: usize) {
    for perm in &st.battlefield {
        for ability in super::game::permanent_abilities(deck, perm) {
            if let Some(SimEffect::WinThreshold { counters }) = ability.effect_sequence().first()
                && perm.counters.charge >= *counters
                && census.win_threshold_turn.is_none()
            {
                census.win_threshold_turn = Some(turn as u32);
            }
        }
    }
}

/// Planeswalker ultimates: online when loyalty covers the minus cost
/// (the sim does not resolve the ultimate).
fn check_ultimates(deck: &SimDeck, st: &GameState, census: &mut TurnCensus, turn: usize) {
    for perm in &st.battlefield {
        for ability in super::game::permanent_abilities(deck, perm) {
            let loyalty_cost = ability
                .activation
                .as_ref()
                .map(|activation| activation.loyalty_change().unsigned_abs())
                .unwrap_or(0);
            if loyalty_cost >= 6
                && perm.counters.loyalty >= loyalty_cost
                && census.ultimate_online.is_none()
            {
                census.ultimate_online = Some(turn as u32);
            }
        }
    }
}

/// 4 LAND: play an untapped land when one is in hand (the best-case
/// agent keeps tapped lands for later); any land plays otherwise.
/// "You may play an additional land" boards (Aesi, Wayward Swordtooth
/// class) play a second land the same turn; one-shot effects ("You may
/// play an additional land this turn" — Explore class) grant extra
/// drops through `extra_land_drops_this_turn`.
fn play_land_drops(deck: &SimDeck, st: &mut GameState, census: &mut TurnCensus, turn: usize) {
    if !play_land(deck, st, turn as u32) {
        return;
    }
    census.land_drops[turn - 1] = 1;
    if st.completion.is_some() {
        return;
    }
    let limit = land_play_allowance(deck, st);
    while st.completion.is_none()
        && u32::from(census.land_drops[turn - 1]) < limit
        && play_land(deck, st, turn as u32)
    {
        census.land_drops[turn - 1] += 1;
    }
}

/// Combine independent land-play rules while their sources remain on the board.
fn land_play_allowance(deck: &SimDeck, st: &GameState) -> u32 {
    1 + st.extra_land_drops_this_turn
        + st.battlefield
            .iter()
            .filter(|permanent| !permanent.face_down)
            .map(|permanent| card_of(deck, permanent).flags.extra_land_drops)
            .sum::<u32>()
}
