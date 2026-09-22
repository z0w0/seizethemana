// The per-game turn loop for the goldfish simulator, split from game.rs
// to keep files small. Pure apart from the passed RNG.

use super::cast_phase::{cast_phase, play_land};
use super::deal::{Opener, deal_opener};
use super::game::{
    GameLog, GameState, HAND_LIMIT, InPlay, Pool, card_of, fire_on_enter, fire_on_enter_opts,
    land_types, new_perm_with, register_loyalty_token_engines, take_uid,
};
use super::game_effects::{apply_effect, spend_leftover, tap_budget};
use super::game_mana::{
    add_yield, add_yield_turns, effective_min_cost, pay_cost, payable, pips_ok,
};
use super::model::{Effect, Role, Scale, SimDeck, Trigger};
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

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
        life_paid: 0,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
    };
    let mut leyline_idx: Vec<usize> = Vec::new();
    st.hand.retain(|&idx| {
        if deck.cards[idx].opens_in_play {
            leyline_idx.push(idx);
            false
        } else {
            true
        }
    });
    for idx in leyline_idx {
        st.battlefield_seen.entry(idx).or_insert(0);
        let uid = take_uid(&mut st);
        st.battlefield.push(new_perm_with(uid, deck, idx, 0, false));
    }
    // Leyline openers count as seen: they left the hand zone.
    st.seen = (st.hand.len() + st.battlefield.len()) as u32;

    // Draw engines in play: (permanent uid, draws per turn). The uid
    // resolves to a battlefield position at fire time, so removals
    // (sacrifice outlets, saga completion) never alias another card.
    let mut engines: Vec<(u32, u32)> = Vec::new();
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
    let mut lands_seen = vec![0u32; turns];
    let mut self_milled = vec![0u32; turns];
    let mut opp_milled = vec![0u32; turns];
    let mut awareness = vec![0.0f64; turns];
    let mut drain_total = vec![0u32; turns];
    let mut player_damage = vec![0u32; turns];
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
    // Per-commander: each partner registers its own upkeep engine, so a
    // partner pair never shares (or doubles) a firing.
    let commander_engine_draws: Vec<Option<u32>> = deck
        .commanders
        .iter()
        .map(|cmd| {
            Some(
                cmd.station_tiers
                    .iter()
                    .filter(|t| t.at == 0)
                    .flat_map(|t| t.abilities.iter())
                    .filter(|a| a.trigger == Trigger::OnUpkeep)
                    .filter_map(|a| match a.effect {
                        Effect::Draw(n) => Some(n),
                        _ => None,
                    })
                    .sum::<u32>(),
            )
        })
        .collect();
    // Commanders with any other OnUpkeep engine (drain, mill, tokens,
    // recursion) register a zero-draw engine: the upkeep loop runs the
    // real parsed abilities for a pushed slot.
    let commander_engine_other: Vec<bool> = deck
        .commanders
        .iter()
        .map(|cmd| {
            cmd.station_tiers
                .iter()
                .filter(|t| t.at == 0)
                .flat_map(|t| t.abilities.iter())
                .any(|a| {
                    a.trigger == Trigger::OnUpkeep
                        && matches!(
                            a.effect,
                            Effect::Mill(_)
                                | Effect::ReturnFromGraveyard { .. }
                                | Effect::Drain(_)
                                | Effect::Tokens(_)
                                | Effect::Wheel
                                | Effect::Loot(_)
                        )
                })
        })
        .collect();
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
            fire_on_enter_opts(deck, &mut st, i, turn as u32, true);
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
        // Engine entries whose permanent left the battlefield drop out
        // here (dead entries never fire again).
        let engine_uids: Vec<u32> = engines
            .iter()
            .filter(|(uid, _)| {
                is_commander_sentinel(*uid)
                    || st.battlefield.iter().any(|p| p.uid == *uid && !p.tapped)
            })
            .map(|(uid, _)| *uid)
            .collect();
        engines.retain(|(uid, _)| engine_uids.contains(uid));
        for uid in engine_uids {
            let (host_card, is_cmd) = if is_commander_sentinel(uid) {
                (usize::MAX, true)
            } else {
                match st
                    .battlefield
                    .iter()
                    .find(|p| p.uid == uid)
                    .map(|p| (p.card, false))
                {
                    Some(pair) => pair,
                    None => continue,
                }
            };
            let upkeep_effects: Vec<Effect> = if is_cmd {
                // One sentinel per cast commander; the firing reads only
                // that commander's own abilities, so two partners each
                // fire once per turn instead of every upkeep effect
                // firing per sentinel.
                commander_upkeep_effects(deck, commander_sentinel_slot(uid))
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
                        // Scaling draw engines ("draw a card for each
                        // enchantment you control") draw the matching
                        // permanent count, capped at 8. The count reads
                        // from the host card's class.
                        let host_card_ref = deck.cards.get(host_card);
                        let scaled = host_card_ref
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
                                    .filter(|p| {
                                        p.card < usize::MAX - 1 && matches(&deck.cards[p.card])
                                    })
                                    .count()
                                    .min(8) as u32
                            })
                            .filter(|c| *c > 0)
                            .unwrap_or(*n);
                        for _ in 0..scaled {
                            if let Some(i) = st.library.pop() {
                                st.hand.push(i);
                                st.seen += 1;
                                st.awareness_cards += 1;
                            }
                        }
                    }
                    Effect::Mill(_)
                    | Effect::ReturnFromGraveyard { .. }
                    | Effect::Drain(_)
                    | Effect::Tokens(_)
                    | Effect::Wheel
                    | Effect::Loot(_) => {
                        let mill_opp = deck.cards.get(host_card).is_some_and(|c| c.mills_opponent)
                            || deck.commanders.first().is_some_and(|c| c.mills_opponent);
                        apply_effect(deck, effect, &mut st, turn as u32, mill_opp);
                    }
                    _ => {}
                }
            }
        }
        // Saga chapters advance on the battlefield permanents. The
        // staging runs one chapter per turn from the turn after entry;
        // the permanent leaves the battlefield once the final chapter
        // resolved (real sagas sacrifice after the last chapter).
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
                let chapter_abilities: Vec<Effect> = card
                    .abilities()
                    .filter(|a| a.trigger == Trigger::Activated)
                    .map(|a| a.effect.clone())
                    .collect();
                let step = perm.saga_step as usize;
                let effect = chapter_abilities.get(step).cloned().unwrap_or({
                    if step + 1 < card.chapter_count() {
                        Effect::Draw(1)
                    } else {
                        Effect::None
                    }
                });
                Some((pos, effect, card.mills_opponent))
            })
            .collect();
        for (pos, effect, mill_opp) in saga_firings {
            if let Some(perm) = st.battlefield.get_mut(pos) {
                perm.saga_step += 1;
            }
            if !matches!(effect, Effect::None) {
                apply_effect(deck, &effect, &mut st, turn as u32, mill_opp);
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
            st.battlefield_seen.entry(removed.card).or_insert(0);
            st.graveyard_seen.entry(removed.card).or_insert(turn as u32);
            st.graveyard.push(removed.card);
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
        // (Upkeep draws counted in the engine loop above; the effect
        // executor's Draw arm adds awareness for applied upkeep draws.)

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
        // otherwise. "You may play an additional land" boards (Aesi,
        // Wayward Swordtooth class) play a second land the same turn.
        if play_land(deck, &mut st, turn as u32) {
            land_drops[turn - 1] = 1;
            let extra_lands = st
                .battlefield
                .iter()
                .any(|p| p.card < usize::MAX - 1 && deck.cards[p.card].extra_land_drops);
            if extra_lands && play_land(deck, &mut st, turn as u32) {
                land_drops[turn - 1] += 1;
            }
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
            if let Some(Scale::PerChargeCounter) = y.scaling {
                // Counter-scaled taps resolve at the tap: one any-color
                // pip per charge counter on the source (Astral
                // Cornucopia class).
                for _ in 0..perm.counters {
                    add_yield_turns(
                        deck,
                        &super::model::TapYield {
                            any_pips: 1,
                            ..super::model::TapYield::default()
                        },
                        &mut pool,
                        turn as u32,
                        &st.battlefield,
                    );
                }
                continue;
            }
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
        // Partner decks cast EACH payable commander (each once per game);
        // an unpayable commander is skipped and the next one still gets
        // its try. The log's cast turn is the first one (the command-zone
        // availability timing).
        // Snapshot the pool first: the mana-readiness check below must
        // see the pre-cast pool, or a same-turn commander cast pushes
        // every other card's readiness a turn later.
        let pool_before_commander = pool.clone();
        for (cmd_i, cmd) in deck.commanders.iter().enumerate() {
            if st
                .battlefield
                .iter()
                .any(|p| p.is_commander && card_of(deck, p).name == cmd.name)
            {
                continue;
            }
            if !payable(&cmd.cost, &pool) || !pips_ok(&cmd.cost, &pool) {
                // Skip this commander; the next partner still gets its
                // try this turn.
                continue;
            }
            pay_cost(&cmd.cost, &mut pool);
            mana_spent[turn - 1] += cmd.cost.total() as f64;
            if commander_cast_turn.is_none() {
                commander_cast_turn = Some(turn as u32);
                commander_castable = commander_cast_turn;
            }
            // The commander is a permanent with no library entry.
            let cmd_uid = take_uid(&mut st);
            st.battlefield.push(InPlay {
                uid: cmd_uid,
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
                equipped: false,
                equip_host: None,
                is_commander: true,
                commander_slot: cmd_i,
            });
            if commander_station_at.is_none() {
                // Not a station card: online the moment it is cast.
                station_online = Some(turn as u32);
            }
            if commander_engine_draws[cmd_i].is_some_and(|n| n > 0) || commander_engine_other[cmd_i]
            {
                // Sentinel uid encodes the commander slot: the upkeep
                // loop fires that commander's abilities only.
                engines.push((
                    commander_sentinel_uid(cmd_i),
                    commander_engine_draws[cmd_i].unwrap_or(0),
                ));
            }
            // Planeswalker +1 token engines register at first cast: a
            // loyalty-gain activation that creates tokens is a repeatable
            // once-per-turn engine (Liliana-class token fuel).
            let cmd_pw_pos = st.battlefield.len() - 1;
            register_loyalty_token_engines(deck, &st, cmd_pw_pos, &mut engines);
            // The commander's ETB triggers fire (IGS creates station
            // fuel tokens for each multicolored permanent).
            let cmd_pos = st.battlefield.len() - 1;
            fire_on_enter(deck, &mut st, cmd_pos, turn as u32);
        }

        // Mana available is recorded from the pre-cast snapshot: the
        // cast/activation passes spend from the same pool and
        // `mana_spent` re-adds those costs, so recording post-payment
        // availability would double-subtract in the unused-mana metric.
        mana_available[turn - 1] = pool_before_commander.total() as f64;

        // Mana-readiness: the first turn the board could pay each card's
        // cost, independent of drawing it (the castability curve). The
        // check uses the pre-cast pool: a same-turn commander cast must
        // not push every other card's readiness a turn later.
        for (idx, card) in deck.cards.iter().enumerate() {
            if card.role != Role::Land && mana_ready[idx].is_none() {
                let eff = effective_min_cost(deck, card, &st.battlefield);
                if payable(&eff, &pool_before_commander) && pips_ok(&eff, &pool_before_commander) {
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
        {
            let cheapest = st
                .hand
                .iter()
                .filter_map(|i| {
                    let c = &deck.cards[*i];
                    (c.is_interaction && c.is_instant_speed).then(|| c.min_cost.total())
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
        // Lands seen so far: hand + battlefield. The flood metric reads
        // this (a land drawn and never dropped still floods; a drop made
        // is the wrong lens).
        lands_seen[turn - 1] = (st
            .hand
            .iter()
            .filter(|i| deck.cards[**i].role == Role::Land)
            .count()
            + st.battlefield
                .iter()
                // Token permanents carry sentinel card indexes; only real
                // cards can be lands.
                .filter(|p| p.card < usize::MAX - 1 && deck.cards[p.card].role == Role::Land)
                .count()) as u32;
        self_milled[turn - 1] = st.milled_self;
        opp_milled[turn - 1] = st.milled_opp;
        awareness[turn - 1] = f64::from(st.awareness_cards) / (deck.cards.len() as f64).max(1.0);
        drain_total[turn - 1] = st.drained;
        player_damage[turn - 1] = if turn > 1 { player_damage[turn - 2] } else { 0 } + combat.power;
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
        // 11 EXTRA TURNS: queued extra turns replay one full minimal
        // pass: a land drop (recorded in `land_drops[]` like the main
        // schedule, capped so one turn index carries at most two extra
        // replays), the draw, and upkeep engines firing once. The
        // queue drains while schedule turns remain.
        while pending_extra_turns > 0 && turn as u32 + pending_extra_turns <= turns as u32 {
            pending_extra_turns -= 1;
            if let Some(i) = st.library.pop() {
                st.hand.push(i);
                st.seen += 1;
                st.awareness_cards += 1;
            }
            if land_drops[turn - 1] < 3 {
                let land_pos = st
                    .hand
                    .iter()
                    .position(|idx| deck.cards[*idx].role == Role::Land);
                if let Some(pos) = land_pos {
                    let idx = st.hand.remove(pos);
                    land_drops[turn - 1] += 1;
                    st.battlefield_seen.entry(idx).or_insert(turn as u32);
                    let uid = take_uid(&mut st);
                    st.battlefield
                        .push(new_perm_with(uid, deck, idx, turn as u32, false));
                }
            }
            // Upkeep engines fire on the extra turn too (a Time Sieve
            // loop keeps drawing). The commander sentinel uid resolves
            // to its commander's own abilities like the main upkeep
            // loop does.
            for (uid, draws) in engines.clone() {
                if is_commander_sentinel(uid) {
                    // Commander sentinel: the cast commander permanents
                    // (is_commander, on the battlefield) gate the firing;
                    // each cast commander's own abilities fire once.
                    if !st.battlefield.iter().any(|p| p.is_commander) {
                        continue;
                    }
                    let cmd = deck
                        .commanders
                        .get(commander_sentinel_slot(uid))
                        .unwrap_or(&deck.commanders[0]);
                    let effects: Vec<Effect> = cmd
                        .abilities()
                        .filter(|a| a.trigger == Trigger::OnUpkeep)
                        .map(|a| a.effect.clone())
                        .collect();
                    let mill_opp = cmd.mills_opponent;
                    for effect in &effects {
                        if let Effect::Draw(n) = effect {
                            for _ in 0..*n {
                                if let Some(i) = st.library.pop() {
                                    st.hand.push(i);
                                    st.seen += 1;
                                    st.awareness_cards += 1;
                                }
                            }
                            continue;
                        }
                        apply_effect(deck, effect, &mut st, turn as u32, mill_opp);
                    }
                    continue;
                }
                let Some(p) = st.battlefield.iter().find(|p| p.uid == uid) else {
                    continue;
                };
                let p = p.clone();
                if p.tapped || p.fired {
                    continue;
                }
                if draws > 0 {
                    for _ in 0..draws {
                        if let Some(i) = st.library.pop() {
                            st.hand.push(i);
                            st.seen += 1;
                            st.awareness_cards += 1;
                        }
                    }
                } else {
                    let host_card = p.card;
                    let mill_opp = deck.cards.get(host_card).is_some_and(|c| c.mills_opponent);
                    let effects: Vec<Effect> = deck
                        .cards
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
                        .collect();
                    for effect in &effects {
                        if let Effect::Draw(n) = effect {
                            // Scaling engines scale here too.
                            let scaled = deck
                                .cards
                                .get(host_card)
                                .and_then(|c| c.draws_per_matching)
                                .map(|kind| {
                                    let matches = |card: &super::model::SimCard| match kind {
                                        super::model::DrawMatch::Enchantments => {
                                            card.is_enchantment
                                        }
                                        super::model::DrawMatch::Artifacts => card.is_artifact,
                                        super::model::DrawMatch::Lands => card.role == Role::Land,
                                        super::model::DrawMatch::Creatures => card.is_creature,
                                    };
                                    st.battlefield
                                        .iter()
                                        .filter(|p| {
                                            p.card < usize::MAX - 1 && matches(&deck.cards[p.card])
                                        })
                                        .count()
                                        .min(8) as u32
                                })
                                .filter(|c| *c > 0)
                                .unwrap_or(*n);
                            for _ in 0..scaled {
                                if let Some(i) = st.library.pop() {
                                    st.hand.push(i);
                                    st.seen += 1;
                                    st.awareness_cards += 1;
                                }
                            }
                            continue;
                        }
                        apply_effect(deck, effect, &mut st, turn as u32, mill_opp);
                    }
                }
            }
        }
        if pending_extra_turns == 0 && st.extra_turns_queued > 0 {
            pending_extra_turns = st.extra_turns_queued;
            st.extra_turns_queued = 0;
        }
    }

    let lands_by_4: u8 = land_drops[..4.min(turns)].iter().sum();
    let cards_seen_by_4_value = cards_seen[3.min(turns - 1)];
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
        // End of turn 4: the flood window. Draw engines widen the window
        // past the nominal 11 (opener + 4 draws); the log carries the
        // real seen count so the expectation matches the measurement.
        lands_seen_by_11: lands_seen.get(3).copied().unwrap_or(0),
        cards_seen_by_4: cards_seen_by_4_value,
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
        player_damage,
        extra_turns,
        win_threshold_turn,
        ultimate_online,
        interaction_ready,
        interaction_mana_held,
        card_first_battlefield: st.battlefield_seen,
        card_first_graveyard: st.graveyard_seen,
        infinite_mana_suspected: st.infinite_mana_suspected,
    }
}

/// Sentinel uid for a cast commander's upkeep engine. Real card uids
/// count up from 0; sentinels count down from `u32::MAX`, so they never
/// collide with battlefield uids. The low bits carry the commander slot,
/// so each cast commander fires only its own abilities.
fn commander_sentinel_uid(slot: usize) -> u32 {
    u32::MAX - (slot as u32).min(15)
}

/// True when the uid is a commander sentinel (not a battlefield uid).
fn is_commander_sentinel(uid: u32) -> bool {
    uid > u32::MAX - 16
}

/// The commander slot encoded in a sentinel uid.
fn commander_sentinel_slot(uid: u32) -> usize {
    (u32::MAX - uid) as usize
}

/// Upkeep abilities of one commander (by deck slot), in effect form.
fn commander_upkeep_effects(deck: &SimDeck, slot: usize) -> Vec<Effect> {
    deck.commanders
        .get(slot)
        .map(|cmd| {
            cmd.abilities()
                .filter(|a| a.trigger == Trigger::OnUpkeep)
                .map(|a| a.effect.clone())
                .collect()
        })
        .unwrap_or_default()
}
