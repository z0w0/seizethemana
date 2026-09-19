// The per-game turn loop for the goldfish simulator, split from game.rs
// to keep files small. Pure apart from the passed RNG.

use super::game::{
    GameLog, GameState, HAND_LIMIT, InPlay, OPENING_HAND, Pool, card_of, fetches_land_text,
    fire_on_enter, land_types, new_perm,
};
use super::game_effects::{apply_effect, spend_leftover, tap_budget};
use super::game_mana::{
    add_yield, add_yield_turns, add_yield_turns_empty_board, effective_min_cost, pay_cost,
    pay_creature_cost, payable, pips_ok, usable_for_creature, usable_for_noncreature,
};
use super::model::{Effect, Role, SimDeck, Trigger};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

pub fn run_game(deck: &SimDeck, rng: &mut ChaCha8Rng, turns: u32) -> GameLog {
    let turns = turns.max(1) as usize;
    let mut library: Vec<usize> = (0..deck.cards.len()).collect();
    for i in (1..library.len()).rev() {
        let j = rng.random_range(0..=i);
        library.swap(i, j);
    }
    // Draw the opening hand from the bottom of the shuffled vec.
    let take = |library: &mut Vec<usize>, n: usize| -> Vec<usize> {
        let n = n.min(library.len());
        library.split_off(library.len() - n)
    };
    let mut hand = take(&mut library, OPENING_HAND);

    // Mulligan policy comes from the format rules: commander family
    // redraws once outside its land band; constructed plays London
    // mulligans (redraw, then bottom the same count at random).
    let count_lands = |hand: &[usize], deck: &SimDeck| -> u8 {
        hand.iter()
            .filter(|i| deck.cards[**i].role == Role::Land)
            .count() as u8
    };
    let mut opener_lands = count_lands(&hand, deck);
    let mut mulliganed = false;
    match deck.rules.mulligan {
        super::format::MulliganPolicy::FreeRedraw {
            land_band: (lo, hi),
        } => {
            if !(lo..=hi).contains(&opener_lands) {
                mulliganed = true;
                hand = take(&mut library, OPENING_HAND);
                opener_lands = count_lands(&hand, deck);
            }
        }
        super::format::MulliganPolicy::London { ship_lands } => {
            // London mulligans are bounded by hand size: each mulligan
            // draws one fewer card, so a hand smaller than the ship
            // threshold cannot improve and the loop must stop.
            let mut bottomed = 0usize;
            while opener_lands < ship_lands && hand.len() > ship_lands as usize {
                mulliganed = true;
                bottomed += 1;
                hand = take(&mut library, OPENING_HAND - bottomed);
                opener_lands = count_lands(&hand, deck);
            }
            // Bottom one random card per mulligan taken (no keep choice).
            for _ in 0..bottomed {
                if hand.is_empty() {
                    break;
                }
                let idx = rng.random_range(0..hand.len());
                let card = hand.swap_remove(idx);
                library.insert(0, card);
            }
        }
    }

    // Leyline-style openers begin the game on the battlefield.
    let mut st = GameState {
        battlefield: Vec::new(),
        library,
        hand,
        seen: 0,
        graveyard: Vec::new(),
        battlefield_seen: HashMap::new(),
        graveyard_seen: HashMap::new(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        drained: 0,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
    };
    st.hand.retain(|&idx| {
        if deck.cards[idx].opens_in_play {
            st.battlefield_seen.entry(idx).or_insert(0);
            st.battlefield.push(new_perm(deck, idx, 0, false));
            false
        } else {
            true
        }
    });
    st.seen = st.hand.len() as u32;

    // Draw engines in play: (battlefield position, draws per turn).
    let mut engines: Vec<(usize, u32)> = Vec::new();
    let mut land_drops = vec![0u8; turns];
    let mut mana_available = vec![0.0f64; turns];
    let mut mana_spent = vec![0.0f64; turns];
    let mut cards_seen = vec![0u32; turns];
    let mut mana_ready: Vec<Option<u32>> = vec![None; deck.cards.len()];
    let mut first_seen: HashMap<Role, u32> = HashMap::new();
    let mut first_creature: Option<u32> = None;
    let mut blocked_colors = [false; 5];
    let mut commander_castable: Option<u32> = None;
    let mut commander_cast_turn: Option<u32> = None;
    let mut station_online: Option<u32> = None;
    let mut bodies = vec![0u32; turns];
    let mut engines_online = vec![0u32; turns];
    let mut graveyard_size = vec![0u32; turns];
    let mut card_first_seen: HashMap<usize, u32> = HashMap::new();
    let mut pip_blocks: Vec<(usize, usize)> = Vec::new();
    let mut attack_power = vec![0u32; turns];
    let mut attackers_turn = vec![0u32; turns];
    let mut evasive_turn = vec![0u32; turns];
    let mut library_size = vec![0u32; turns];
    let mut self_milled = vec![0u32; turns];
    let mut opp_milled = vec![0u32; turns];
    let mut awareness = vec![0.0f64; turns];
    let mut drain_total = vec![0u32; turns];
    let mut extra_turns = vec![0u32; turns];
    let mut win_threshold_turn: Option<u32> = None;
    let mut ultimate_online: Option<u32> = None;
    let mut interaction_ready = vec![false; turns];
    let mut interaction_mana_held = vec![0.0f64; turns];
    // Extra-turn queue: the loop replays these indices after the main
    // schedule (one replay pass per queued turn, capped by remaining).
    let mut pending_extra_turns: u32 = 0;

    // The commander's synthetic engine tier (upkeep/end-step draws only)
    // comes from deck construction. Attack-gated draws live in real tiers
    // and fire through the combat path once the spacecraft animates.
    let commander_engine_draws = deck.commanders.first().map(|cmd| {
        cmd.station_tiers
            .iter()
            .filter(|t| t.at == 0)
            .flat_map(|t| t.abilities.iter())
            .filter(|a| a.trigger == Trigger::OnUpkeep)
            .filter_map(|a| match a.effect {
                Effect::Draw(n) => Some(n),
                _ => None,
            })
            .sum::<u32>()
    });
    let commander_station_at = deck.commanders.first().and_then(|cmd| cmd.animate_at());

    for turn in 1..=turns {
        // 1 UNTAP: everything untaps; sickness clears; once-per-turn resets.
        // Blink flags re-fire the host's OnEnter triggers once (Skyskipper
        // Duo, Conjurer's Closet-style flickers).
        let blink_positions: Vec<usize> = st
            .battlefield
            .iter()
            .enumerate()
            .filter(|(_, p)| p.blink_pending && p.card < usize::MAX - 1)
            .map(|(i, _)| i)
            .collect();
        for i in blink_positions {
            st.battlefield[i].blink_pending = false;
            fire_on_enter(deck, &mut st, i, turn as u32);
        }
        for perm in st.battlefield.iter_mut() {
            perm.tapped = false;
            perm.sick = false;
            perm.fired = false;
        }
        st.prowess_casts = 0;

        // 2 UPKEEP: engines fire; saga chapters advance (one card each).
        // Draw engines fire their amount; mill/recursion engines run
        // through the effect executor (one firing per turn, fixed delay).
        // Win-threshold engines check their counter stock here (Darksteel
        // Reactor class); planeswalker ultimates flag online when
        // loyalty reaches the minus cost.
        let engine_positions: Vec<usize> = engines
            .iter()
            .filter(|(pos, _)| {
                *pos == usize::MAX || st.battlefield.get(*pos).is_some_and(|p| !p.tapped)
            })
            .map(|(pos, _)| *pos)
            .collect();
        for pos in engine_positions {
            let (host_card, is_cmd) = if pos == usize::MAX {
                (usize::MAX, true)
            } else {
                match st.battlefield.get(pos) {
                    Some(p) => (p.card, false),
                    None => continue,
                }
            };
            let upkeep_effects: Vec<Effect> = if is_cmd {
                deck.commanders
                    .iter()
                    .flat_map(|c| c.abilities().cloned().collect::<Vec<_>>())
                    .filter(|a| a.trigger == Trigger::OnUpkeep)
                    .map(|a| a.effect)
                    .collect()
            } else {
                deck.cards
                    .get(host_card)
                    .map(|c| {
                        c.abilities()
                            .filter(|a| a.trigger == Trigger::OnUpkeep)
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| a.effect)
                    .collect()
            };
            for effect in &upkeep_effects {
                match effect {
                    Effect::Draw(n) => {
                        for _ in 0..*n {
                            if let Some(i) = st.library.pop() {
                                st.hand.push(i);
                                st.seen += 1;
                            }
                        }
                    }
                    Effect::Mill(_) | Effect::ReturnFromGraveyard { .. } => {
                        let mill_opp = deck.cards.get(host_card).is_some_and(|c| c.mills_opponent)
                            || deck.commanders.first().is_some_and(|c| c.mills_opponent);
                        apply_effect(deck, effect, &mut st, turn as u32, mill_opp);
                    }
                    _ => {}
                }
            }
        }
        for perm in st.battlefield.iter_mut() {
            let card = card_of(deck, perm);
            if card.is_saga
                && perm.entered_turn > 0
                && turn > perm.entered_turn
                && perm.saga_step < 3
            {
                perm.saga_step += 1;
                if perm.saga_step <= 2 {
                    // Chapters I–II produce one card of effect per turn
                    // (loot or ramp); chapter III is the payoff.
                    if let Some(i) = st.library.pop() {
                        st.hand.push(i);
                        st.seen += 1;
                    }
                }
            }
        }

        // Win-threshold engines: enough counters at upkeep wins.
        for perm in &st.battlefield {
            let card = card_of(deck, perm);
            for ability in card.abilities() {
                if let Effect::WinThreshold { counters } = ability.effect
                    && perm.counters >= counters
                    && win_threshold_turn.is_none()
                {
                    win_threshold_turn = Some(turn as u32);
                }
            }
        }
        // Planeswalker ultimates: online when loyalty covers the minus
        // cost (the sim does not resolve the ultimate).
        for perm in &st.battlefield {
            for ability in card_of(deck, perm).abilities() {
                if ability.loyalty_cost >= 6
                    && perm.loyalty >= ability.loyalty_cost
                    && ultimate_online.is_none()
                {
                    ultimate_online = Some(turn as u32);
                }
            }
        }

        // 3 DRAW.
        if let Some(i) = st.library.pop() {
            st.hand.push(i);
            st.seen += 1;
            st.awareness_cards += 1;
        }

        // Record role sightings from the hand.
        for i in &st.hand {
            let card = &deck.cards[*i];
            first_seen.entry(card.role).or_insert(turn as u32);
            if card.is_creature && first_creature.is_none() {
                first_creature = Some(turn as u32);
            }
            card_first_seen.entry(*i).or_insert(turn as u32);
        }

        // 4 LAND: play an untapped land when one is in hand (the
        // best-case agent keeps tapped lands for later); any land plays
        // otherwise.
        if play_land(deck, &mut st, turn as u32) {
            land_drops[turn - 1] = 1;
        }

        // Build the turn's spendable pool from untapped permanents. Verge
        // gates apply: the gated tap mode unlocks only when another land
        // of the matching type is in play.
        let mut pool = Pool::default();
        // Banked-mana engines release at the upkeep (Coalition Relic):
        // counters × N mana joins the pool, counters clear.
        let releases: Vec<(usize, super::model::TapYield, u32)> = st
            .battlefield
            .iter()
            .filter_map(|perm| {
                let card = card_of(deck, perm);
                let release = card
                    .station_tiers
                    .iter()
                    .flat_map(|t| t.abilities.iter())
                    .find(|a| {
                        a.trigger == Trigger::OnUpkeep
                            && matches!(a.effect, Effect::ManaPerCounter(_))
                    })?;
                let y = match &release.effect {
                    Effect::ManaPerCounter(y) => y.clone(),
                    _ => return None,
                };
                Some((perm.card, y, perm.counters))
            })
            .collect();
        for (card_idx, y, counters) in releases {
            for _ in 0..counters {
                add_yield_turns(deck, &y, &mut pool, turn as u32, &st.battlefield);
            }
            if let Some(perm) = st
                .battlefield
                .iter_mut()
                .find(|p| p.card == card_idx && p.counters > 0)
            {
                perm.counters = 0;
            }
        }
        for perm in &st.battlefield {
            if perm.tapped {
                continue;
            }
            let card = card_of(deck, perm);
            let Some(y) = &card.tap else {
                continue;
            };
            if card.gate_types.is_empty() {
                add_yield_turns(deck, y, &mut pool, turn as u32, &st.battlefield);
                continue;
            }
            // Gate check: does the board hold another land of a gated type?
            // The ungated primary mode is the first fixed color; the gated
            // modes are the choice colors.
            let gates_open = card.gate_types.iter().any(|want| {
                st.battlefield.iter().any(|p| {
                    p.card != perm.card
                        && card_of(deck, p).role == Role::Land
                        && land_types(&card_of(deck, p).name).contains(want)
                })
            });
            if gates_open {
                add_yield_turns(deck, y, &mut pool, turn as u32, &st.battlefield);
            } else {
                // Locked: only the ungated first mode produces. The ungated
                // color is the first fixed pip, or the first choice color
                // when the merge turned both modes into choices.
                let ungated_color = y
                    .fixed
                    .iter()
                    .position(|p| *p > 0)
                    .or_else(|| y.choice.iter().position(|c| *c));
                let Some(ci) = ungated_color else {
                    if y.colorless > 0 {
                        pool.colorless += 1;
                    }
                    continue;
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
                add_yield(&ungated, &mut pool);
            }
        }
        // Static mana grants (Enduring Vitality, Chromatic Lantern):
        // each matching permanent adds one flexible pip per turn, capped
        // at two pips per grant. The grant source itself must be on the
        // battlefield.
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
                        card.is_creature && !p.is_commander
                    } else {
                        card.role == Role::Land && !p.is_commander
                    }
                })
                .count() as u32;
            pool.flexible += matches.min(2);
        }
        // Treasure bank: spend up to the full bank as flexible pips
        // (the player would sacrifice them when needed; best-case the
        // whole bank converts this turn).
        if st.treasure_bank > 0 {
            pool.flexible += st.treasure_bank;
            st.treasure_bank = 0;
        }

        // Commander cast: full pip check, cost deducted, joins the board.
        // The cost records in mana_spent once (this +=), not twice.
        // Partner decks cast each commander separately; the log's cast turn
        // is the first one (the command-zone availability timing).
        for (cmd_i, cmd) in deck.commanders.iter().enumerate() {
            if commander_cast_turn.is_some() && cmd_i > 0 {
                // Only the first commander's timing feeds the log curve.
                break;
            }
            if st
                .battlefield
                .iter()
                .any(|p| p.is_commander && card_of(deck, p).name == cmd.name)
            {
                continue;
            }
            if !payable(&cmd.cost, &pool) || !pips_ok(&cmd.cost, &pool) {
                break;
            }
            pay_cost(&cmd.cost, &mut pool);
            mana_spent[turn - 1] += cmd.cost.total() as f64;
            if commander_cast_turn.is_none() {
                commander_cast_turn = Some(turn as u32);
                commander_castable = commander_cast_turn;
            }
            // The commander is a permanent with no library entry.
            st.battlefield.push(InPlay {
                card: usize::MAX,
                tapped: false,
                sick: false,
                counters: 0,
                animated: false,
                crewed: false,
                entered_turn: turn,
                saga_step: 0,
                fired: false,
                blink_pending: false,
                loyalty: cmd.starting_loyalty.unwrap_or(0),
                is_commander: true,
                commander_slot: cmd_i,
            });
            if commander_station_at.is_none() {
                // Not a station card: online the moment it is cast.
                station_online = Some(turn as u32);
            }
            if commander_engine_draws.is_some_and(|n| n > 0) {
                engines.push((usize::MAX, commander_engine_draws.unwrap_or(0)));
            }
            // The commander's ETB triggers fire (IGS creates station
            // fuel tokens for each multicolored permanent).
            let cmd_pos = st.battlefield.len() - 1;
            fire_on_enter(deck, &mut st, cmd_pos, turn as u32);
        }

        mana_available[turn - 1] = pool.total() as f64;

        // Mana-readiness: the first turn the board could pay each card's
        // cost, independent of drawing it (the castability curve).
        for (idx, card) in deck.cards.iter().enumerate() {
            if card.role != Role::Land && mana_ready[idx].is_none() {
                let eff = effective_min_cost(deck, card, &st.battlefield);
                if payable(&eff, &pool) && pips_ok(&eff, &pool) {
                    mana_ready[idx] = Some(turn as u32);
                }
            }
        }

        cast_phase(
            deck,
            &mut st,
            &mut pool,
            turn,
            &mut mana_spent,
            &mut engines,
            &mut pip_blocks,
            &mut blocked_colors,
        );
        // 6 ACTIVATE: spend leftover mana on unlocked activations
        // (draw engines, mana engines, walkers). Cheapest first, one per
        // permanent. Sacrifice outlets run the same pass (they tap).
        spend_leftover(deck, &mut st, &mut pool, turn as u32);

        // 7 TAP BUDGET: mana only while casting still needs it; then
        // station; then crew.
        tap_budget(deck, &mut st.battlefield, &mut pool, &st.hand);

        // 7b INTERACTION READINESS (measured, not forced): was instant-
        // speed interaction in hand while spare mana covered its cost?
        // The cheapest answer in hand decides; the goldfish never spends
        // it. Capacity, not events.
        if turn <= turns {
            let cheapest = st
                .hand
                .iter()
                .filter_map(|i| {
                    let c = &deck.cards[*i];
                    (c.is_interaction && c.is_instant_speed)
                        .then(|| c.min_cost.total().max(c.cost.total()))
                })
                .min();
            match cheapest {
                Some(cheapest) if pool.total() >= cheapest => {
                    interaction_ready[turn - 1] = true;
                    interaction_mana_held[turn - 1] = f64::from(pool.total() - cheapest);
                }
                _ => {}
            }
        }

        // 8 THRESHOLD: station tiers unlock (permanent for animate tiers).
        for perm in st.battlefield.iter_mut() {
            let card = card_of(deck, perm);
            if card.is_station_card
                && let Some(at) = card.animate_at()
                && !perm.animated
                && perm.counters >= at
            {
                perm.animated = true;
                if perm.is_commander && station_online.is_none() {
                    station_online = Some(turn as u32);
                }
            }
        }

        // 9 COMBAT: bodies attack; attack triggers fire (see game_combat).
        let combat = super::game_combat::combat_phase(deck, &mut st, turn, turns, &land_drops);
        attack_power[turn - 1] = combat.power;
        attackers_turn[turn - 1] = combat.attackers;
        evasive_turn[turn - 1] = combat.evasive;
        let token_bodies = combat.token_bodies;
        cards_seen[turn - 1] = st.seen;
        graveyard_size[turn - 1] = st.graveyard.len() as u32;
        library_size[turn - 1] = st.library.len() as u32;
        self_milled[turn - 1] = st.milled_self;
        opp_milled[turn - 1] = st.milled_opp;
        awareness[turn - 1] = f64::from(st.awareness_cards) / (deck.cards.len() as f64).max(1.0);
        drain_total[turn - 1] = st.drained;
        extra_turns[turn - 1] = st.extra_turns_queued;
        bodies[turn - 1] = st
            .battlefield
            .iter()
            .filter(|p| card_of(deck, p).is_creature || p.animated)
            .count() as u32
            + token_bodies.min(4);
        engines_online[turn - 1] = engines.len() as u32;

        // 10 END: hand-limit discard.
        while st.hand.len() > HAND_LIMIT {
            let discarded = st.hand.remove(st.hand.len() - 1);
            st.graveyard_seen
                .entry(discarded)
                .or_insert(turns.max(1) as u32);
            st.graveyard.push(discarded);
        }
        // 11 EXTRA TURNS: queued extra turns each give one land drop and
        // one draw (the fixed-value replay the goldfish credits). The
        // queue drains while schedule turns remain.
        while pending_extra_turns > 0 && turn as u32 + pending_extra_turns <= turns as u32 {
            pending_extra_turns -= 1;
            if let Some(i) = st.library.pop() {
                st.hand.push(i);
                st.seen += 1;
                st.awareness_cards += 1;
            }
            let land_pos = st
                .hand
                .iter()
                .position(|idx| deck.cards[*idx].role == Role::Land);
            if let Some(pos) = land_pos {
                let idx = st.hand.remove(pos);
                st.battlefield_seen.entry(idx).or_insert(turn as u32);
                st.battlefield.push(new_perm(deck, idx, turn as u32, false));
            }
        }
        if pending_extra_turns == 0 && st.extra_turns_queued > 0 {
            pending_extra_turns = st.extra_turns_queued;
            st.extra_turns_queued = 0;
        }
    }

    let lands_by_4: u8 = land_drops[..4.min(turns)].iter().sum();
    GameLog {
        land_drops,
        mana_available,
        mana_spent,
        cards_seen,
        commander_castable,
        mana_ready,
        first_seen,
        first_creature,
        blocked_colors,
        opener_lands,
        mulliganed,
        lands_by_4,
        station_online,
        bodies,
        engines_online,
        pip_blocks,
        graveyard_size,
        card_first_seen,
        attack_power,
        attackers: attackers_turn,
        evasive: evasive_turn,
        library_size,
        self_milled,
        opp_milled,
        awareness,
        drain_total,
        extra_turns,
        win_threshold_turn,
        ultimate_online,
        interaction_ready,
        interaction_mana_held,
        card_first_battlefield: st.battlefield_seen,
        card_first_graveyard: st.graveyard_seen,
    }
}

/// Play one land from the hand (untapped first), fetch a search land,
/// and fire its ETB triggers. Returns true when a land was played.
fn play_land(deck: &SimDeck, st: &mut GameState, turn: u32) -> bool {
    let land_pos = st
        .hand
        .iter()
        .position(|idx| deck.cards[*idx].role == Role::Land && !deck.cards[*idx].enters_tapped)
        .or_else(|| {
            st.hand
                .iter()
                .position(|idx| deck.cards[*idx].role == Role::Land)
        });
    let Some(pos) = land_pos else {
        return false;
    };
    let idx = st.hand.remove(pos);
    let card = &deck.cards[idx];
    let tapped_in = card.enters_tapped;
    st.battlefield_seen.entry(idx).or_insert(turn);
    st.battlefield.push(new_perm(deck, idx, turn, tapped_in));
    if fetches_land_text(card) {
        // Search up a land from the library (enters tapped).
        if let Some(i) = st
            .library
            .iter()
            .position(|c| deck.cards[*c].role == Role::Land && !fetches_land_text(&deck.cards[*c]))
        {
            let fetched = st.library.remove(i);
            st.battlefield_seen.entry(fetched).or_insert(turn);
            st.battlefield.push(new_perm(deck, fetched, turn, true));
        }
    }
    // ETB triggers for the new land (ExtraLand-style ramp lands).
    let new_pos = st.battlefield.len() - 1;
    fire_on_enter(deck, st, new_pos, turn);
    true
}

/// The cast pass: cheapest castable spells first, pip-aware. Updates the
/// pool, ETB triggers, and the mana-ready curve. Returns nothing; the
/// caller owns every mutated binding.
#[allow(clippy::too_many_arguments)]
fn cast_phase(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: usize,
    mana_spent: &mut [f64],
    engines: &mut Vec<(usize, u32)>,
    pip_blocks: &mut Vec<(usize, usize)>,
    blocked_colors: &mut [bool; 5],
) {
    // 5 CAST: cheapest castable spells (pip-aware). Spend-restricted
    // mana pays creature casts only.
    let mut order: Vec<usize> = (0..st.hand.len())
        .filter(|p| deck.cards[st.hand[*p]].role != Role::Land)
        .collect();
    order.sort_by_key(|p| deck.cards[st.hand[*p]].min_cost.total());
    let mut cast_positions = Vec::new();
    let mut spent_total = 0u32;
    for pos in order {
        let idx = st.hand[pos];
        let card = &deck.cards[idx];
        let eff = effective_min_cost(deck, card, &st.battlefield);
        // Spend-restricted mana pays creature casts only.
        let (cost_ok, pip_ok) = if card.is_creature {
            (
                usable_for_creature(pool) >= eff.total(),
                pips_ok(&eff, pool) || pool.creature_only > 0,
            )
        } else {
            (
                usable_for_noncreature(pool) >= eff.total(),
                pips_ok(&eff, pool),
            )
        };
        if !cost_ok || !pip_ok {
            // Colors a cast was blocked for: enough total, missing pips.
            if payable(&card.cost, pool) && !pips_ok(&card.cost, pool) {
                let flexible = pool.flexible;
                for (ci, need) in card.cost.pips.iter().enumerate() {
                    if *need > 0 && pool.fixed[ci] + flexible < u32::from(*need) {
                        blocked_colors[ci] = true;
                        pip_blocks.push((idx, ci));
                    }
                }
            }
            continue;
        }
        if card.is_creature {
            pay_creature_cost(&eff, pool);
        } else {
            pay_cost(&eff, pool);
        }
        spent_total += eff.total();
        cast_positions.push(pos);
        // Producers join the battlefield: rocks tap at once, creatures
        // from next turn (summoning sickness). Vehicles and spacecraft
        // join as artifacts.
        st.battlefield_seen.entry(idx).or_insert(turn as u32);
        st.battlefield.push(InPlay {
            card: idx,
            tapped: false,
            sick: card.is_creature,
            counters: card.enter_counters,
            animated: false,
            crewed: false,
            entered_turn: turn,
            saga_step: 0,
            fired: false,
            blink_pending: false,
            loyalty: card.starting_loyalty.unwrap_or(0),
            is_commander: false,
            commander_slot: 0,
        });
        // One-shot mana (rituals) joins this turn's pool only.
        if let Some(y) = &card.mana_on_cast {
            add_yield_turns_empty_board(y, pool, turn as u32);
        }
        // One-shot draws on cast (cantrips, Divination).
        for _ in 0..card.draws_on_cast {
            if let Some(i) = st.library.pop() {
                st.hand.push(i);
                st.seen += 1;
                st.awareness_cards += 1;
            }
        }
        // One-shot mill on cast (plain "mill N" spells).
        for _ in 0..card.mills_on_enter {
            if let Some(i) = st.library.pop() {
                st.graveyard_seen.entry(i).or_insert(turn as u32);
                st.graveyard.push(i);
                st.seen += 1;
                st.awareness_cards += 1;
                if card.mills_opponent {
                    st.milled_opp += 1;
                } else {
                    st.milled_self += 1;
                }
            }
        }
        // Scry/surveil on cast: awareness only; surveil mills the
        // scry'd cards to the graveyard.
        if card.scry_on_cast > 0 {
            st.awareness_cards += card.scry_on_cast;
            if card.surveils {
                for _ in 0..card.scry_on_cast {
                    if let Some(i) = st.library.pop() {
                        st.graveyard_seen.entry(i).or_insert(turn as u32);
                        st.graveyard.push(i);
                        st.milled_self += 1;
                    }
                }
            }
        }
        // One-shot extra turns queue for replay after this turn.
        if card.extra_turns_on_cast {
            st.extra_turns_queued += 1;
        }
        // One-shot drain spells (burn at a player, "each opponent
        // loses N life"). Player-targeted damage resolves ×3 (three
        // opponents); creature-target burn never got here (Removal).
        if card.drain_on_cast > 0 {
            st.drained += card.drain_on_cast * 3;
        }
        // Prowess census: noncreature spells cast this turn.
        if !card.is_creature && card.role != Role::Land {
            st.prowess_casts += 1;
        }
        // OnCastSpell engines fire per spell cast.
        for p in st.battlefield.clone() {
            for ability in card_of(deck, &p).abilities() {
                if let (Trigger::OnCastSpell, Effect::Draw(n)) = (ability.trigger, &ability.effect)
                {
                    for _ in 0..*n {
                        if let Some(i) = st.library.pop() {
                            st.hand.push(i);
                            st.seen += 1;
                        }
                    }
                }
            }
        }
        // Counter injection targets the highest-threshold unfilled
        // station permanent (Drill Too Deep).
        if card.counters_on_cast > 0
            && let Some(perm) = st
                .battlefield
                .iter_mut()
                .filter(|p| card_of(deck, p).is_station_card)
                .max_by_key(|p| card_of(deck, p).animate_at().unwrap_or(0))
        {
            perm.counters += card.counters_on_cast;
        }
    }
    mana_spent[turn - 1] += spent_total as f64;
    // Remove cast cards highest position first (cast order is by cost).
    cast_positions.sort_unstable();
    for pos in cast_positions.into_iter().rev() {
        st.hand.remove(pos);
    }
    // ETB triggers for cards cast this turn (they entered the board);
    // their upkeep engines register for the next turn. Positions are
    // collected directly so two copies of the same name each fire.
    let newly_cast: Vec<usize> = st
        .battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| p.entered_turn == turn && !p.is_commander && p.card < usize::MAX - 1)
        .map(|(pos, _)| pos)
        .collect();
    for pos in newly_cast {
        fire_on_enter(deck, st, pos, turn as u32);
        // Upkeep engines on the cast card register now. `pos` may have
        // shifted from token pushes; re-resolve by card identity.
        let card_idx = st.battlefield[pos].card;
        for ability in deck.cards[card_idx].abilities() {
            match (ability.trigger, &ability.effect) {
                (Trigger::OnUpkeep, Effect::Draw(n)) => {
                    let live_pos = st
                        .battlefield
                        .iter()
                        .position(|p| p.card == card_idx && p.entered_turn == turn)
                        .unwrap_or(pos);
                    engines.push((live_pos, *n));
                }
                (Trigger::OnUpkeep, Effect::Mill(_))
                | (Trigger::OnUpkeep, Effect::ReturnFromGraveyard { .. }) => {
                    let live_pos = st
                        .battlefield
                        .iter()
                        .position(|p| p.card == card_idx && p.entered_turn == turn)
                        .unwrap_or(pos);
                    engines.push((live_pos, 0));
                }
                _ => {}
            }
        }
    }
}
