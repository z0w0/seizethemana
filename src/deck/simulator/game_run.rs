//! The per-game turn loop for the goldfish simulator, split from game.rs
//! to keep files small. Pure apart from the passed RNG. `run_game` owns
//! the game scope; each pipeline step is one helper in this file.

use super::cast_pass::{cast_pass, play_land, upkeep_trigger_registration};
use super::deal::{Opener, deal_opener};
use super::game::{
    GameLog, GameState, ManaPool, Permanent, card_of, fire_on_enter, fire_triggers, new_perm_with,
    take_uid,
};
use super::game_commander::CommanderProfile;
use super::game_effects::{apply_effect_at, spend_leftover, tap_budget};
use super::game_mana::{add_yield_turns, pay_cost, payable, pips_ok};
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

pub(in crate::deck::simulator) use commander::commander_phase;
pub(in crate::deck::simulator) use mana::{
    build_pool, fire_tapped_for_mana_triggers, grants_active, tap_dorks_for_mana, tap_new_rocks,
};

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
pub fn run_game(deck: &SimDeck, rng: &mut ChaCha8Rng, turns: u32) -> GameLog {
    let turns = turns.max(1) as usize;
    let opener = deal_opener(deck, rng);
    let Opener {
        hand,
        library,
        lands: opener_lands,
        mulliganed,
    } = opener;

    let mut st = GameState {
        battlefield: Vec::new(),
        library,
        hand,
        seen: 0,
        graveyard: Vec::new(),
        exile: Vec::new(),
        battlefield_seen: HashMap::new(),
        graveyard_seen: HashMap::new(),
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
    };
    apply_leylines(deck, &mut st);

    let mut census = TurnCensus::new(turns, deck.cards.len());
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
        pending_extra_turns = pending_extra_turns.saturating_add(st.extra_turns_queued);
        st.extra_turns_queued = 0;
    }

    census::finish_log(st, census, opener_lands, mulliganed, turns)
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
    beginning_phase(deck, st, census, repeatable_sources, turn);
    precombat_main_phase(deck, st, census, commander, repeatable_sources, turn);
    // State-based action (CR 704.5j): only one legendary permanent of
    // each name survives before the combat census reads the board.
    super::game_effects::enforce_legend_rule(deck, st, turn as u32);
    combat_phase(deck, st, census, repeatable_sources, turn);
    postcombat_main_phase();
    ending_phase(deck, st, census, turn, is_extra_turn);
}

/// Run untap, upkeep, and draw at the start of a scheduled turn.
fn beginning_phase(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    repeatable_sources: &mut Vec<(u32, u32)>,
    turn: usize,
) {
    // 1 UNTAP: everything untaps; sickness clears; once-per-turn resets.
    // Crew animations expire (Vehicles stop being bodies at end of
    // turn). Blink flags re-fire the host's Enters triggers once
    // (Skyskipper Duo, Conjurer's Closet-style flickers).
    st.flashback_permissions.clear();
    expire_crew(st);
    fire_returned_triggers(deck, st, turn as u32);
    for perm in st.battlefield.iter_mut() {
        // "Doesn't untap during your untap step" statics stay tapped
        // (Basalt Monolith class); only an untap activation clears the
        // tap. Every other permanent untaps normally.
        if !card_of(deck, perm).flags.doesnt_untap {
            perm.tapped = false;
        }
        perm.summoning_sick = false;
        perm.fired = false;
    }
    st.prowess_casts = 0;
    st.damage_dealt_this_turn = 0;
    st.spells_cast_this_turn = 0;
    st.extra_land_drops_this_turn = 0;
    st.attacked_this_turn = false;
    st.activated_this_turn.clear();
    st.triggered_this_turn.clear();

    // Upkeep triggers fire and win checks run. Saga chapters advance in
    // the precombat main phase (CR 714.3c).
    run_upkeep(deck, st, repeatable_sources, turn);
    check_win_thresholds(deck, st, census, turn);
    check_ultimates(deck, st, census, turn);

    // 3 DRAW.
    let draws_on_turn = deck.rules.shape == super::model::Format::Commander || turn > 1;
    if draws_on_turn {
        super::game_effects::draw_one(deck, st, turn as u32);
    }
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
    // Saga lore counters are added as the precombat main phase begins
    // (CR 714.3c), before the main-phase triggers and land drops; the
    // entry chapter fired at cast time.
    run_sagas(deck, st, turn);
    fire_triggers(deck, st, SimTrigger::PrecombatMain, turn as u32);
    census::record_hand_sightings(deck, st, census, turn);

    // 4 LAND.
    play_land_drops(deck, st, census, turn);

    // 5 POOL, companion fetch, commander cast, casts, 6 ACTIVATE,
    // 7 TAP BUDGET.
    let mut pool = build_pool(deck, st, turn as u32);
    // PLOT (CR 702.170): exile a hand card with a plot cost when the
    // card is otherwise unaffordable; from a later turn it casts for
    // free. Plot spends mana from this turn's pool.
    plot_unaffordable_cards(deck, st, &mut pool, census, turn);
    // Free-cast plotted cards from a later turn than they were plotted.
    cast_plotted_cards(deck, st, turn, &mut census.mana_spent, repeatable_sources);
    fetch_companion(deck, st, &mut pool, census, turn);
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
    for _ in 0..super::model::MAX_LOOP_PASSES {
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
        tap_dorks_for_mana(deck, st, &mut pool, turn as u32);
        tap_new_rocks(deck, st, &mut pool, turn as u32);
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
        if before_mana != pool.total()
            || before_hand != st.hand
            || before_board != st.battlefield.len()
        {
            activations_done = false;
            continue;
        }
        if !activations_done {
            spend_leftover(deck, st, &mut pool, turn as u32);
            activations_done = true;
            continue;
        }
        break;
    }
    tap_budget(deck, &mut st.battlefield, &mut pool, &st.hand);
    census::record_interaction_readiness(deck, st, &pool, census, turn);

    // 8 THRESHOLD: station tiers unlock (permanent for animate tiers).
    unlock_thresholds(deck, st, census, turn);
}

/// Resolve combat and record its turn metrics.
fn combat_phase(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    repeatable_sources: &[(u32, u32)],
    turn: usize,
) {
    // 9 COMBAT.
    census::record_combat(deck, st, census, turn);
    // Engine count for the turn (commander + cast-phase registrations).
    census::record_repeatable_source_count(census, turn, repeatable_sources);
}

/// Keep the postcombat main phase explicit; the action policy has no
/// postcombat actions yet.
fn postcombat_main_phase() {}

/// Run end-step triggers and cleanup actions.
pub(super) fn ending_phase(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    turn: usize,
    is_extra_turn: bool,
) {
    // 10 END: beginning-of-end-step triggers, then hand-limit discard.
    // Monarch: one extra card at the beginning of the Monarch's end
    // step (CR 725.2), including the turn the player becomes monarch.
    if st.is_monarch {
        super::game_effects::draw_one(deck, st, turn as u32);
    }
    fire_triggers(deck, st, SimTrigger::EndStep, turn as u32);
    // Mobilize tokens are sacrificed at the beginning of the next end
    // step (CR 702.181a). The sacrifice fires death triggers like any
    // other sacrifice.
    sacrifice_end_step_tokens(deck, st, turn);
    // Saddle and crew both expire at end of turn (CR 702.171b).
    expire_saddle(st);
    census::end_step_discard(deck, st, turn);
    census.extra_turns[turn - 1] = u32::from(is_extra_turn);
}

/// Sacrifice permanents marked for the end step (mobilize's Warrior
/// tokens): remove them, move deck cards to the graveyard, and fire the
/// surviving board's death triggers. Tokens leave without a graveyard
/// entry (tokens cease to exist), but the sacrifice still triggers.
fn sacrifice_end_step_tokens(deck: &SimDeck, st: &mut GameState, turn: usize) {
    while let Some(pos) = st.battlefield.iter().position(|p| p.sacrifice_at_end) {
        let victim = st.battlefield.remove(pos);
        if let Some(idx) = victim.card.deck_idx() {
            super::game_effects::move_to_graveyard(
                deck,
                st,
                idx,
                turn as u32,
                super::game_effects::CardZone::Battlefield,
            );
        }
        super::game_effects::fire_death_triggers(deck, st, turn as u32, victim.uid);
    }
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
    census.mana_spent[turn - 1] += 3.0;
    st.companion_fetched = true;
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
    let extra = st.battlefield.iter().any(|perm| {
        perm.card
            .deck_idx()
            .is_some_and(|idx| deck[idx].flags.extra_land_drops)
    });
    let limit = 1 + u32::from(extra) + st.extra_land_drops_this_turn;
    while u32::from(census.land_drops[turn - 1]) < limit
        && super::cast_pass::play_land(deck, st, turn as u32)
    {
        census.land_drops[turn - 1] += 1;
    }
}

/// Plot an unaffordable hand card (CR 702.170): pay the plot cost and
/// exile it face up. From a later turn the card casts for free. Best
/// case: plot when the plot cost is payable so the card is not stranded.
fn plot_unaffordable_cards(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    census: &mut TurnCensus,
    turn: usize,
) {
    let candidates: Vec<CardIdx> = st
        .hand
        .iter()
        .copied()
        .filter(|idx| {
            let card = &deck[*idx];
            card.keyword_abilities.plot.is_some()
                && !payable(&card.min_cost, pool)
                && card
                    .keyword_abilities
                    .plot
                    .as_ref()
                    .is_some_and(|plot| payable(plot, pool) && pips_ok(plot, pool))
        })
        .collect();
    for idx in candidates {
        let Some(plot) = deck[idx].keyword_abilities.plot else {
            continue;
        };
        if !payable(&plot, pool) || !pips_ok(&plot, pool) {
            continue;
        }
        pay_cost(&plot, pool);
        census.mana_spent[turn - 1] += plot.total() as f64;
        if let Some(pos) = st.hand.iter().position(|held| *held == idx) {
            st.hand.remove(pos);
        }
        st.plotted.push((idx, turn as u32));
    }
}

/// Cast plotted cards (CR 702.170d): a card plotted on an earlier turn
/// casts from exile without paying its mana cost.
fn cast_plotted_cards(
    deck: &SimDeck,
    st: &mut GameState,
    turn: usize,
    mana_spent: &mut [f64],
    repeatable_sources: &mut Vec<(u32, u32)>,
) {
    let ready: Vec<CardIdx> = st
        .plotted
        .iter()
        .filter(|(_, plotted_turn)| (*plotted_turn as usize) < turn)
        .map(|(idx, _)| *idx)
        .collect();
    for idx in ready {
        let card = &deck[idx];
        // The free cast creates the same battlefield presence a paid
        // cast would: the card is removed from the plotted list and
        // pushed with its enter counters; no mana changes hands.
        st.plotted.retain(|(plotted, _)| *plotted != idx);
        let uid = take_uid(st);
        if card.is_instant_or_sorcery {
            st.graveyard_seen.entry(idx).or_insert(turn as u32);
            st.graveyard.push(idx);
            super::game_effects::apply_effect_at(
                deck,
                &SimEffect::Draw(card.spell_data.draws_on_cast),
                st,
                turn as u32,
                false,
                Some(idx),
            );
        } else {
            st.battlefield_seen.entry(idx).or_insert(turn as u32);
            let mut entry = new_perm_with(uid, deck, idx, turn as u32, false);
            entry.summoning_sick =
                (card.is_creature || card.keyword_abilities.living_metal) && !card.flags.has_haste;
            entry.counters = card.enter_counters.fixed();
            st.battlefield.push(entry);
            fire_on_enter(deck, st, st.battlefield.len() - 1, turn as u32, false);
            for ability in card.unlocked_abilities(0) {
                if let Some(draws) = upkeep_trigger_registration(ability) {
                    super::game::register_repeatable_source(repeatable_sources, uid, draws);
                }
            }
        }
        let _ = mana_spent;
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
    // A static grant ("creatures/lands you control have '{T}: Add one
    // mana of any color'") converts the permanent's own tap: the tap
    // yields one any-color pip instead of its printed yield (the same
    // tap, one mana).
    let grants = grants_active(deck, st);
    let card = card_of(deck, source);
    let grant_conversion = (card.is_land && grants.lands) || (card.is_creature && grants.creatures);
    let mana_before = pool.total();
    if grant_conversion {
        pool.flexible += 1;
    } else {
        // Counter-scaled taps resolve against the source's own counters
        // (Crystalline Crawler class): one any-color pip per charge
        // counter when the yield scales per counter.
        let yield_ = if matches!(yield_.scaling, Some(Scale::PerChargeCounter)) {
            super::model::ManaYield {
                any_pips: source.counters.charge,
                ..super::model::ManaYield::default()
            }
        } else {
            yield_.clone()
        };
        add_yield_turns(deck, &yield_, pool, turn, &st.battlefield);
    }
    let produced = pool.total() > mana_before;
    if !produced {
        return false;
    }
    if card_of(deck, source).is_land {
        return true;
    }
    let bonus_triggers = st
        .battlefield
        .iter()
        .filter(|permanent| card_of(deck, permanent).flags.bonus_mana_on_nonland_tap)
        .count() as u32;
    if bonus_triggers == 0 {
        return true;
    }
    let mut produced_colors = [false; 5];
    for (index, amount) in yield_.fixed.iter().enumerate() {
        produced_colors[index] |= *amount > 0;
    }
    for (index, available) in yield_.choice.iter().enumerate() {
        produced_colors[index] |= *available;
    }
    let color_count = produced_colors
        .iter()
        .filter(|available| **available)
        .count();
    if yield_.opponent_any {
        pool.colorless += bonus_triggers;
    } else if yield_.any_pips > 0 || color_count > 1 {
        pool.flexible += bonus_triggers;
    } else if let Some(color) = produced_colors.iter().position(|available| *available) {
        pool.fixed[color] += bonus_triggers;
    } else if yield_.colorless > 0 {
        pool.colorless += bonus_triggers;
    }
    true
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
    let returned_positions: Vec<usize> = st
        .battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| p.returned_trigger_pending && p.card.deck_idx().is_some())
        .map(|(i, _)| i)
        .collect();
    for i in returned_positions {
        st.battlefield[i].returned_trigger_pending = false;
        fire_on_enter(deck, st, i, turn, true);
    }
}

/// Upkeep triggers fire; entries whose permanent left the battlefield
/// drop out through the live-uid set (dead entries never fire again).
/// The turn loop untapped the board before upkeep, so tap state does
/// not filter here.
fn run_upkeep(
    deck: &SimDeck,
    st: &mut GameState,
    repeatable_sources: &mut Vec<(u32, u32)>,
    turn: usize,
) {
    let live_uids: std::collections::HashSet<u32> = st.battlefield.iter().map(|p| p.uid).collect();
    repeatable_sources.retain(|(uid, _)| live_uids.contains(uid));
    let mut seen_uids = std::collections::HashSet::new();
    let live: Vec<u32> = repeatable_sources
        .iter()
        .filter_map(|(uid, _)| seen_uids.insert(*uid).then_some(*uid))
        .collect();
    for uid in live {
        fire_upkeep_triggers_for_source(deck, st, uid, turn);
    }
}

/// Fire every supported upkeep ability owned by one live permanent.
fn fire_upkeep_triggers_for_source(deck: &SimDeck, st: &mut GameState, uid: u32, turn: usize) {
    let Some(source) = st.battlefield.iter().find(|perm| perm.uid == uid).cloned() else {
        return;
    };
    let host = card_of(deck, &source);
    let host_idx = source.card.deck_idx();
    let abilities: Vec<_> = super::game::permanent_abilities(deck, &source)
        .filter(|ability| ability.kind.is_triggered() && ability.trigger == SimTrigger::Upkeep)
        .filter(|ability| {
            ability
                .condition
                .is_none_or(|condition| super::game::condition_met(deck, st, &condition))
        })
        .cloned()
        .collect();
    for ability in abilities {
        let key = (uid, ability.id);
        if ability.once_per_turn && st.triggered_this_turn.contains(&key) {
            continue;
        }
        if ability
            .condition
            .is_some_and(|condition| !super::game::condition_met(deck, st, &condition))
        {
            continue;
        }
        for effect in ability.effect_sequence() {
            match effect {
                SimEffect::Draw(count) => run_scaling_draw(deck, st, host_idx, *count, turn as u32),
                SimEffect::Counters(count) => {
                    if let Some(perm) = st.battlefield.iter_mut().find(|perm| perm.uid == uid) {
                        perm.counters.charge += count;
                    }
                }
                _ => apply_effect_at(deck, effect, st, turn as u32, host.mills_opponent, host_idx),
            }
        }
        if ability.once_per_turn {
            st.triggered_this_turn.insert(key);
        }
    }
}

/// Scaling draw triggers ("draw a card for each enchantment you control")
/// draw the matching permanent count, capped at 8; plain triggers draw N.
pub(super) fn run_scaling_draw(
    deck: &SimDeck,
    st: &mut GameState,
    host_idx: Option<CardIdx>,
    base: u32,
    turn: u32,
) {
    let scaled = host_idx
        .and_then(|idx| deck.cards.get(idx.index()))
        .and_then(|c| c.draws_per_matching)
        .map(|kind| {
            let matches = |card: &super::model::SimCard| match kind {
                super::model::DrawMatch::Enchantments => card.is_enchantment,
                super::model::DrawMatch::Artifacts => card.is_artifact,
                super::model::DrawMatch::Lands => card.is_land,
                super::model::DrawMatch::Creatures => card.is_creature,
            };
            st.battlefield
                .iter()
                .filter(|p| p.card.deck_idx().is_some_and(|idx| matches(&deck[idx])))
                .count()
                .min(8) as u32
        })
        .filter(|c| *c > 0)
        .unwrap_or(base);
    for _ in 0..scaled {
        super::game_effects::draw_one(deck, st, turn);
    }
}

/// Saga chapters advance one per turn after their entry chapter. The
/// sim adds lore counters in the precombat main phase (CR 714.3c);
/// `run_turn` calls this as the precombat main phase begins.
pub(super) fn run_sagas(deck: &SimDeck, st: &mut GameState, turn: usize) {
    let saga_firings: Vec<(usize, Vec<SimEffect>, bool, Option<CardIdx>)> = st
        .battlefield
        .iter()
        .enumerate()
        .filter_map(|(pos, perm)| {
            let card = card_of(deck, perm);
            if !card.is_saga
                || perm.entered_turn == 0
                || turn <= perm.entered_turn
                || perm.saga_step as usize >= card.chapter_count()
            {
                return None;
            }
            let step = perm.saga_step as usize;
            let effects = card.saga.chapters.get(step).cloned().unwrap_or_default();
            Some((pos, effects, card.mills_opponent, perm.card.deck_idx()))
        })
        .collect();
    for (pos, effects, mill_opp, source) in saga_firings {
        if let Some(perm) = st.battlefield.get_mut(pos) {
            perm.saga_step += 1;
        }
        for effect in &effects {
            if !matches!(effect, SimEffect::None) {
                apply_effect_at(deck, effect, st, turn as u32, mill_opp, source);
            }
        }
    }
    // Sagas that resolved their final chapter leave the battlefield
    // (they sacrifice in real Magic; their death triggers fire).
    let finished_sagas: Vec<usize> = st
        .battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            let card = card_of(deck, p);
            card.is_saga && p.saga_step as usize >= card.chapter_count() && p.entered_turn > 0
        })
        .map(|(i, _)| i)
        .collect();
    for pos in finished_sagas.into_iter().rev() {
        let removed = st.battlefield.remove(pos);
        let Some(idx) = removed.card.deck_idx() else {
            continue;
        };
        st.battlefield_seen.entry(idx).or_insert(0);
        super::game_effects::move_to_graveyard(
            deck,
            st,
            idx,
            turn as u32,
            super::game_effects::CardZone::Battlefield,
        );
    }
}

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
    let battlefield_extra = st.battlefield.iter().any(|p| {
        p.card
            .deck_idx()
            .is_some_and(|idx| deck[idx].flags.extra_land_drops)
    });
    let limit = 1 + u32::from(battlefield_extra) + st.extra_land_drops_this_turn;
    while u32::from(census.land_drops[turn - 1]) < limit && play_land(deck, st, turn as u32) {
        census.land_drops[turn - 1] += 1;
    }
}
