// The per-game turn loop for the goldfish simulator, split from game.rs
// to keep files small. Pure apart from the passed RNG. `run_game` owns
// the game scope; each pipeline step is one helper in this file.

use super::cast_phase::{cast_phase, play_land};
use super::deal::{Opener, deal_opener};
use super::game::{
    GameLog, GameState, InPlay, Pool, card_of, fire_on_enter, fire_triggers, new_perm_with,
    register_loyalty_token_engines, take_uid,
};
use super::game_commander::{
    CommanderProfile, commander_sentinel_slot, commander_sentinel_uid, commander_upkeep_effects,
    is_commander_sentinel,
};
use super::game_effects::{apply_effect_at, spend_leftover, tap_budget};
use super::game_mana::{
    add_yield, add_yield_turns, cast_restriction, effective_min_cost, pay_cost,
    pay_restricted_cost, payable, pips_ok, usable_for_noncreature,
};
use super::model::{AbilityTiming, CardIdx, Effect, Role, Scale, SimDeck};
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

mod census;
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
        #[cfg(test)]
        alternate_casts: Vec::new(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        drained: 0,
        life_gained: 0,
        flashback_permissions: std::collections::HashSet::new(),
        replay_casts: 0,
        milestones_by_turn: HashMap::new(),
        life_paid: 0,
        life_funded_draws: 0,
        life: match deck.format {
            super::model::Format::Commander => 40,
            super::model::Format::Constructed => 20,
        },
        is_monarch: false,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
    };
    apply_leylines(deck, &mut st);

    let mut census = TurnCensus::new(turns, deck.cards.len());
    let commander = CommanderProfile::new(deck);
    // Draw engines in play: (permanent uid, draws per turn). The uid
    // resolves to a battlefield position at fire time, so removals
    // (sacrifice outlets, saga completion) never alias another card.
    let mut engines: Vec<(u32, u32)> = Vec::new();
    let mut pending_extra_turns = 0u32;
    for turn in 1..=turns {
        let is_extra_turn = pending_extra_turns > 0;
        pending_extra_turns = pending_extra_turns.saturating_sub(1);
        run_turn(
            deck,
            &mut st,
            &mut census,
            &commander,
            &mut engines,
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
    engines: &mut Vec<(u32, u32)>,
    turn: usize,
    is_extra_turn: bool,
) {
    // 1 UNTAP: everything untaps; sickness clears; once-per-turn resets.
    // Crew animations expire (Vehicles stop being bodies at end of
    // turn). Blink flags re-fire the host's OnEnter triggers once
    // (Skyskipper Duo, Conjurer's Closet-style flickers).
    st.flashback_permissions.clear();
    expire_crew(st);
    fire_blink_refires(deck, st, turn as u32);
    for perm in st.battlefield.iter_mut() {
        perm.tapped = false;
        perm.sick = false;
        perm.fired = false;
    }
    st.prowess_casts = 0;

    // 2 UPKEEP: engines fire; win checks run. Saga chapters advance in
    // the precombat main phase (phase 5, CR 714.3c).
    run_upkeep(deck, st, engines, turn);
    check_win_thresholds(deck, st, census, turn);
    check_ultimates(deck, st, census, turn);

    // 3 DRAW.
    let draws_on_turn = deck.rules.shape == super::model::Format::Commander || turn > 1;
    if draws_on_turn {
        super::game_effects::draw_one(deck, st, turn as u32);
    }
    fire_triggers(deck, st, AbilityTiming::PrecombatMainPhase, turn as u32);
    census::record_hand_sightings(deck, st, census, turn);

    // 4 LAND.
    play_land_drops(deck, st, census, turn);

    // 5 POOL, commander cast, casts, 6 ACTIVATE, 7 TAP BUDGET.
    // Saga lore counters are added in the precombat main phase, after
    // the draw step (CR 714.3c); the entry chapter fired at cast time.
    run_sagas(deck, st, turn);
    let mut pool = build_pool(deck, st, turn as u32);
    commander_phase(deck, st, &mut pool, census, commander, turn, engines, true);
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
        cast_phase(
            deck,
            st,
            &mut pool,
            turn,
            &mut census.mana_spent,
            engines,
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
        commander_phase(deck, st, &mut pool, census, commander, turn, engines, false);
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
    // 9 COMBAT.
    census::record_combat(deck, st, census, turn);
    // Engine count for the turn (commander + cast-phase registrations).
    census::record_engine_count(census, turn, engines);
    // 10 END: beginning-of-end-step triggers, then hand-limit discard.
    // Monarch: one extra card at the beginning of the Monarch's end
    // step (CR 725.2), from the turn after acquisition.
    if draws_on_turn && st.is_monarch {
        super::game_effects::draw_one(deck, st, turn as u32);
    }
    fire_triggers(deck, st, AbilityTiming::OnEndStep, turn as u32);
    census::end_step_discard(deck, st, turn);
    census.extra_turns[turn - 1] = u32::from(is_extra_turn);
}

/// Play a land drawn or made accessible during the main phase when a land
/// drop remains. Newly cast extra-land engines can add another legal drop.
fn play_late_land(deck: &SimDeck, st: &mut GameState, census: &mut TurnCensus, turn: usize) {
    let extra = st.battlefield.iter().any(|perm| {
        perm.card
            .deck_idx()
            .is_some_and(|idx| deck[idx].extra_land_drops)
    });
    let limit = 1 + usize::from(extra);
    while usize::from(census.land_drops[turn - 1]) < limit
        && super::cast_phase::play_land(deck, st, turn as u32)
    {
        census.land_drops[turn - 1] += 1;
    }
}

/// Tap a permanent for its parsed mana and resolve player-controlled
/// bonuses that trigger from a nonland mana tap.
pub(super) fn add_nonland_mana(
    deck: &SimDeck,
    st: &GameState,
    source: &InPlay,
    yield_: &super::model::TapYield,
    pool: &mut Pool,
    turn: u32,
) -> bool {
    let mana_before = pool.total();
    add_yield_turns(deck, yield_, pool, turn, &st.battlefield);
    let produced = pool.total() > mana_before;
    if !produced {
        return false;
    }
    if card_of(deck, source).role == Role::Land {
        return true;
    }
    let bonus_triggers = st
        .battlefield
        .iter()
        .filter(|permanent| card_of(deck, permanent).bonus_mana_on_nonland_tap)
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

/// Re-fire the OnEnter triggers of every blink-armed permanent once
/// (the deferred firing never re-arms: one re-fire per entry).
fn fire_blink_refires(deck: &SimDeck, st: &mut GameState, turn: u32) {
    let blink_positions: Vec<usize> = st
        .battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| p.blink_pending && p.card.deck_idx().is_some())
        .map(|(i, _)| i)
        .collect();
    for i in blink_positions {
        st.battlefield[i].blink_pending = false;
        fire_on_enter(deck, st, i, turn, true);
    }
}

/// Upkeep engines fire; entries whose permanent left the battlefield
/// drop out through the live-uid set (dead entries never fire again).
/// The turn loop untapped the board before upkeep, so tap state does
/// not filter here.
fn run_upkeep(deck: &SimDeck, st: &mut GameState, engines: &mut Vec<(u32, u32)>, turn: usize) {
    let live_uids: std::collections::HashSet<u32> = st.battlefield.iter().map(|p| p.uid).collect();
    engines.retain(|(uid, _)| is_commander_sentinel(*uid) || live_uids.contains(uid));
    let live: Vec<u32> = engines.iter().map(|(uid, _)| *uid).collect();
    for uid in live {
        fire_upkeep_engine(deck, st, uid, turn);
    }
}

/// One engine's upkeep firing: the commander sentinel reads its own
/// commander's abilities; a battlefield engine reads its host card's.
fn fire_upkeep_engine(deck: &SimDeck, st: &mut GameState, uid: u32, turn: usize) {
    let is_cmd = is_commander_sentinel(uid);
    let host_idx: Option<CardIdx> = if is_cmd {
        None
    } else {
        st.battlefield
            .iter()
            .find(|p| p.uid == uid)
            .and_then(|p| p.card.deck_idx())
    };
    if !is_cmd && host_idx.is_none() {
        return;
    }
    let upkeep_effects: Vec<Effect> = if is_cmd {
        // One sentinel per cast commander; the firing reads only that
        // commander's own abilities, so two partners each fire once per
        // turn instead of every upkeep effect firing per sentinel.
        commander_upkeep_effects(deck, commander_sentinel_slot(uid))
    } else {
        deck[host_idx.unwrap()]
            .abilities()
            .filter(|a| a.trigger == AbilityTiming::OnUpkeep)
            .map(|a| a.effect.clone())
            .collect()
    };
    for effect in &upkeep_effects {
        match effect {
            Effect::Draw(n) => run_scaling_draw(deck, st, host_idx, *n, turn as u32),
            Effect::Mill(_)
            | Effect::ReturnFromGraveyard { .. }
            | Effect::Drain(_)
            | Effect::Tokens(_)
            | Effect::Wheel
            | Effect::Loot(_) => {
                // Mill direction comes from the firing card itself. The
                // commander override applies only when the sentinel is
                // the firing source; a battlefield host keeps its own
                // flag (a self-mill engine must not reroute because the
                // commander mills opponents).
                let mill_opp = if is_cmd {
                    let slot = commander_sentinel_slot(uid);
                    deck.commanders.get(slot).is_some_and(|c| c.mills_opponent)
                } else {
                    deck[host_idx.unwrap()].mills_opponent
                };
                apply_effect_at(deck, effect, st, turn as u32, mill_opp, None);
            }
            _ => {}
        }
    }
}

/// Scaling draw engines ("draw a card for each enchantment you control")
/// draw the matching permanent count, capped at 8; plain engines draw N.
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
                super::model::DrawMatch::Lands => card.role == Role::Land,
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
/// `run_turn` calls this at the start of phase 5.
pub(super) fn run_sagas(deck: &SimDeck, st: &mut GameState, turn: usize) {
    let saga_firings: Vec<(usize, Effect, bool)> = st
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
            let effect = card
                .saga
                .chapters
                .get(step)
                .cloned()
                .unwrap_or(Effect::None);
            Some((pos, effect, card.mills_opponent))
        })
        .collect();
    for (pos, effect, mill_opp) in saga_firings {
        if let Some(perm) = st.battlefield.get_mut(pos) {
            perm.saga_step += 1;
        }
        if !matches!(effect, Effect::None) {
            apply_effect_at(deck, &effect, st, turn as u32, mill_opp, None);
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
        let card = card_of(deck, perm);
        for ability in card.abilities() {
            if let Effect::WinThreshold { counters } = ability.effect
                && perm.counters >= counters
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
        for ability in card_of(deck, perm).abilities() {
            if ability.loyalty_cost >= 6
                && perm.loyalty >= ability.loyalty_cost
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
/// class) play a second land the same turn.
fn play_land_drops(deck: &SimDeck, st: &mut GameState, census: &mut TurnCensus, turn: usize) {
    if play_land(deck, st, turn as u32) {
        census.land_drops[turn - 1] = 1;
        let extra_lands = st.battlefield.iter().any(|p| {
            p.card
                .deck_idx()
                .is_some_and(|idx| deck[idx].extra_land_drops)
        });
        if extra_lands && play_land(deck, st, turn as u32) {
            census.land_drops[turn - 1] += 1;
        }
    }
}

/// Build the turn's spendable pool from untapped permanents: taps,
/// verge gates, banked-mana releases, counter-scaled taps, static
/// grants, and the Treasure bank.
pub(super) fn build_pool(deck: &SimDeck, st: &mut GameState, turn: u32) -> Pool {
    let mut pool = Pool::default();
    release_banked_mana(deck, st, &mut pool, turn);
    let uids: Vec<u32> = st
        .battlefield
        .iter()
        .map(|permanent| permanent.uid)
        .collect();
    for uid in uids {
        let Some(pos) = st
            .battlefield
            .iter()
            .position(|permanent| permanent.uid == uid)
        else {
            continue;
        };
        let perm = st.battlefield[pos].clone();
        if perm.tapped {
            continue;
        }
        let card = card_of(deck, &perm);
        if !mana_condition_met(deck, st, card) {
            continue;
        }
        // Land and noncreature mana sources are reserved when their mana
        // joins the pool. This prevents the same permanent from stationing
        // or crewing after it paid for a cast.
        if card.is_creature && perm.card.deck_idx().is_some() {
            continue;
        }
        let Some(y) = &card.tap else {
            continue;
        };
        if let Some(Scale::PerChargeCounter) = y.scaling {
            // Counter-scaled taps resolve at the tap: one any-color pip
            // per charge counter on the source (Astral Cornucopia
            // class).
            let yield_ = super::model::TapYield {
                any_pips: perm.counters,
                ..super::model::TapYield::default()
            };
            add_nonland_mana(deck, st, &perm, &yield_, &mut pool, turn);
            st.battlefield[pos].tapped = true;
            continue;
        }
        if card.gate_types.is_empty() {
            if !add_nonland_mana(deck, st, &perm, y, &mut pool, turn) {
                continue;
            }
            if card.sacrifices_for_mana {
                st.battlefield.remove(pos);
                let Some(idx) = perm.card.deck_idx() else {
                    continue;
                };
                super::game_effects::move_to_graveyard(
                    deck,
                    st,
                    idx,
                    turn,
                    super::game_effects::CardZone::Battlefield,
                );
            } else {
                st.battlefield[pos].tapped = true;
            }
            continue;
        }
        add_gated_yield(deck, st, &perm, y, &mut pool, turn);
        if card.sacrifices_for_mana {
            st.battlefield.remove(pos);
            let Some(idx) = perm.card.deck_idx() else {
                continue;
            };
            super::game_effects::move_to_graveyard(
                deck,
                st,
                idx,
                turn,
                super::game_effects::CardZone::Battlefield,
            );
        } else {
            st.battlefield[pos].tapped = true;
        }
    }
    add_static_grants(deck, st, &mut pool);
    // Treasure bank: spend up to the full bank as flexible pips (the
    // player would sacrifice them when needed; best-case the whole bank
    // converts this turn). Treasures are consumed on use, so the bank
    // empties here — new Treasure this turn restocks it for next turn.
    if st.treasure_bank > 0 {
        pool.flexible += st.treasure_bank;
        st.treasure_bank = 0;
    }
    pool
}

/// Tap legal creature mana sources only when the current pool cannot pay
/// the cheapest spell still in hand. Tapped dorks cannot also station or crew.
pub(super) fn tap_dorks_for_mana(deck: &SimDeck, st: &mut GameState, pool: &mut Pool, turn: u32) {
    let need = st
        .hand
        .iter()
        .filter(|idx| deck[**idx].role != Role::Land)
        .map(|idx| effective_min_cost(deck, &deck[*idx], &st.battlefield).total())
        .min();
    let Some(need) = need else { return };
    for pos in 0..st.battlefield.len() {
        if pool.total() >= need {
            break;
        }
        let perm = &st.battlefield[pos];
        let card = card_of(deck, perm);
        if perm.tapped
            || perm.sick
            || perm.card.deck_idx().is_some()
            || !card.is_creature
            || !mana_condition_met(deck, st, card)
        {
            continue;
        }
        let Some(yield_) = card.tap.clone() else {
            continue;
        };
        if !add_nonland_mana(deck, st, perm, &yield_, pool, turn) {
            continue;
        }
        st.battlefield[pos].tapped = true;
    }
}

/// Activate a noncreature rock after it enters, then revisit the cast
/// choices. Its mana source is tapped exactly once for the turn.
pub(super) fn tap_new_rocks(deck: &SimDeck, st: &mut GameState, pool: &mut Pool, turn: u32) {
    let uids: Vec<u32> = st
        .battlefield
        .iter()
        .filter(|permanent| permanent.entered_turn == turn as usize)
        .map(|permanent| permanent.uid)
        .collect();
    for uid in uids {
        let Some(pos) = st
            .battlefield
            .iter()
            .position(|permanent| permanent.uid == uid)
        else {
            continue;
        };
        let perm = st.battlefield[pos].clone();
        let card = card_of(deck, &perm);
        if perm.tapped
            || perm.entered_turn != turn as usize
            || card.is_creature
            || !mana_condition_met(deck, st, card)
        {
            continue;
        }
        let Some(yield_) = card.tap.clone() else {
            continue;
        };
        if !add_nonland_mana(deck, st, &perm, &yield_, pool, turn) {
            continue;
        }
        if card.sacrifices_for_mana {
            st.battlefield.remove(pos);
            let Some(idx) = perm.card.deck_idx() else {
                continue;
            };
            super::game_effects::move_to_graveyard(
                deck,
                st,
                idx,
                turn,
                super::game_effects::CardZone::Battlefield,
            );
        } else {
            st.battlefield[pos].tapped = true;
        }
    }
}

/// Check player-controlled conditions that gate a card's mana ability.
fn mana_condition_met(deck: &SimDeck, st: &GameState, card: &super::model::SimCard) -> bool {
    !card.requires_metalcraft
        || st
            .battlefield
            .iter()
            .filter(|permanent| card_of(deck, permanent).is_artifact)
            .count()
            >= 3
}

/// Banked-mana engines release in the first main phase (Coalition Relic):
/// counters × N mana joins the pool, counters clear.
fn release_banked_mana(deck: &SimDeck, st: &mut GameState, pool: &mut Pool, turn: u32) {
    let releases: Vec<(u32, super::model::TapYield, u32)> = st
        .battlefield
        .iter()
        .filter_map(|perm| {
            let card = card_of(deck, perm);
            let release = card
                .station_tiers
                .iter()
                .flat_map(|t| t.abilities.iter())
                .find(|a| {
                    a.trigger == AbilityTiming::PrecombatMainPhase
                        && matches!(a.effect, Effect::ManaPerCounter(_))
                })?;
            let y = match &release.effect {
                Effect::ManaPerCounter(y) => y.clone(),
                _ => return None,
            };
            Some((perm.uid, y, perm.counters))
        })
        .collect();
    for (uid, y, counters) in releases {
        for _ in 0..counters {
            add_yield_turns(deck, &y, pool, turn, &st.battlefield);
        }
        // The contributing permanent clears its own bank; a second
        // copy of the card keeps its counters for its own upkeep.
        if let Some(perm) = st.battlefield.iter_mut().find(|p| p.uid == uid) {
            perm.counters = 0;
        }
    }
}

/// Verge-gate lands: the gated tap mode unlocks when another land of a
/// gated type is in play; locked lands yield only the ungated first
/// mode (the first fixed pip, or the first choice color).
fn add_gated_yield(
    deck: &SimDeck,
    st: &GameState,
    perm: &InPlay,
    y: &super::model::TapYield,
    pool: &mut Pool,
    turn: u32,
) {
    let gates_open = card_of(deck, perm).gate_types.iter().any(|want| {
        let Some(type_index) = ["Plains", "Island", "Swamp", "Mountain", "Forest"]
            .iter()
            .position(|kind| kind == want)
        else {
            return false;
        };
        st.battlefield.iter().any(|p| {
            p.card != perm.card
                && card_of(deck, p).role == Role::Land
                && card_of(deck, p).land_types[type_index]
        })
    });
    if gates_open {
        add_yield_turns(deck, y, pool, turn, &st.battlefield);
        return;
    }
    let ungated_color = y
        .fixed
        .iter()
        .position(|p| *p > 0)
        .or_else(|| y.choice.iter().position(|c| *c));
    let Some(ci) = ungated_color else {
        if y.colorless > 0 {
            pool.colorless += 1;
        }
        return;
    };
    let mut ungated = super::model::TapYield {
        fixed: [0; 5],
        choice: [false; 5],
        any_pips: 0,
        opponent_any: false,
        scaling: None,
        colorless: 0,
        alternatives: false,
        restriction: None,
    };
    ungated.fixed[ci] = 1;
    add_yield(&ungated, pool);
}

/// Static mana grants (Enduring Vitality, Chromatic Lantern): each
/// matching permanent adds one flexible pip per turn, capped at two
/// pips per grant. The grant source itself must be on the battlefield.
fn add_static_grants(deck: &SimDeck, st: &GameState, pool: &mut Pool) {
    for grantor_idx in 0..st.battlefield.len() {
        let grant = {
            let perm = &st.battlefield[grantor_idx];
            card_of(deck, perm).grant
        };
        let Some(grant) = grant else {
            continue;
        };
        let want_creatures = matches!(grant, super::model::Grant::Creatures);
        let matches = st
            .battlefield
            .iter()
            .filter(|p| {
                let card = card_of(deck, p);
                // Creatures grant empowers creatures; lands grant
                // empowers lands. Commanders are never granted mana
                // by their own static engine here.
                if want_creatures {
                    card.is_creature && p.card.deck_idx().is_some()
                } else {
                    card.role == Role::Land && p.card.deck_idx().is_some()
                }
            })
            .count() as u32;
        pool.flexible += matches.min(2);
    }
}

/// Commander cast: full pip check against the general pool, cost
/// deducted, joins the board. Partner decks cast EACH payable commander
/// (each once per game); an unpayable commander is skipped and the next
/// one still gets its try. The log's cast turn is the first one.
#[allow(clippy::too_many_arguments)]
fn commander_phase(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    census: &mut TurnCensus,
    commander: &CommanderProfile,
    turn: usize,
    engines: &mut Vec<(u32, u32)>,
    record_readiness: bool,
) {
    // Snapshot the pool first: the mana-readiness check below must see
    // the pre-cast pool, or a same-turn commander cast pushes every
    // other card's readiness a turn later.
    let pool_before_commander = pool.clone();
    for (cmd_i, cmd) in deck.commanders.iter().enumerate() {
        if st
            .battlefield
            .iter()
            .any(|p| matches!(p.card, super::game::CardRef::Commander { slot } if slot == cmd_i))
        {
            continue;
        }
        // Gate on the general pool only: `pay_cost` never spends the
        // restricted buckets (creature/legendary/artifact/instant-
        // sorcery mana pays only its own cast class), so counting them
        // here would cast commanders with mana never deducted.
        let restriction = cast_restriction(cmd);
        let mana_ok = restriction.map_or_else(
            || usable_for_noncreature(pool) >= cmd.cost.total() && pips_ok(&cmd.cost, pool),
            |class| {
                let pip_total =
                    cmd.cost.pips.iter().map(|p| u32::from(*p)).sum::<u32>() + cmd.cost.flex_pips;
                pool.usable_for(class) >= cmd.cost.total()
                    && (pips_ok(&cmd.cost, pool)
                        || pool.usable_for(class) - usable_for_noncreature(pool) + pool.flexible
                            >= pip_total)
            },
        );
        if !mana_ok {
            // Skip this commander; the next partner still gets its try
            // this turn.
            continue;
        }
        // Deduct from the general pool only, mirroring the gate.
        if let Some(class) = restriction {
            pay_restricted_cost(&cmd.cost, pool, class);
        } else {
            pay_cost(&cmd.cost, pool);
        }
        census.mana_spent[turn - 1] += cmd.cost.total() as f64;
        if census.commander_castable.is_none() {
            census.commander_castable = Some(turn as u32);
        }
        // The commander is a permanent with no library entry.
        let cmd_uid = take_uid(st);
        st.battlefield.push(super::game::new_commander_perm(
            cmd_uid,
            cmd_i,
            cmd.starting_loyalty.unwrap_or(0),
            turn,
        ));
        if commander.station_at.is_none() {
            // Not a station card: online the moment it is cast.
            census.station_online = Some(turn as u32);
        }
        if commander.engine_draws[cmd_i].is_some_and(|n| n > 0) || commander.engine_other[cmd_i] {
            // Sentinel uid encodes the commander slot: the upkeep loop
            // fires that commander's abilities only.
            engines.push((
                commander_sentinel_uid(cmd_i),
                commander.engine_draws[cmd_i].unwrap_or(0),
            ));
        }
        // Planeswalker +1 token engines register at first cast: a
        // loyalty-gain activation that creates tokens is a repeatable
        // once-per-turn engine (Liliana-class token fuel).
        let cmd_pw_pos = st.battlefield.len() - 1;
        register_loyalty_token_engines(deck, st, cmd_pw_pos, engines);
        // The commander's ETB triggers fire (IGS creates station fuel
        // tokens for each multicolored permanent).
        let cmd_pos = st.battlefield.len() - 1;
        fire_on_enter(deck, st, cmd_pos, turn as u32, false);
    }

    // Mana available is recorded from the pre-cast snapshot: the
    // cast/activation passes spend from the same pool and `mana_spent`
    // re-adds those costs, so recording post-payment availability would
    // double-subtract in the unused-mana metric.
    if record_readiness {
        census.mana_available[turn - 1] = pool_before_commander.total() as f64;
    }

    // Mana-readiness: the first turn the board could pay each card's
    // cost, independent of drawing it (the castability curve). The
    // check uses the pre-cast pool: a same-turn commander cast must not
    // push every other card's readiness a turn later.
    if record_readiness {
        for (idx, card) in deck.cards.iter().enumerate() {
            if card.role != Role::Land && census.mana_ready[idx].is_none() {
                let eff = effective_min_cost(deck, card, &st.battlefield);
                if payable(&eff, &pool_before_commander) && pips_ok(&eff, &pool_before_commander) {
                    census.mana_ready[idx] = Some(turn as u32);
                }
            }
        }
    }
}

/// 8 THRESHOLD: station tiers unlock (permanent for animate tiers).
fn unlock_thresholds(deck: &SimDeck, st: &mut GameState, census: &mut TurnCensus, turn: usize) {
    for perm in st.battlefield.iter_mut() {
        let card = card_of(deck, perm);
        if card.is_station_card
            && let Some(at) = card.animate_at()
            && !perm.animated
            && perm.counters >= at
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
