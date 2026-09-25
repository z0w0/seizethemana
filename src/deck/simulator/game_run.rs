// The per-game turn loop for the goldfish simulator, split from game.rs
// to keep files small. Pure apart from the passed RNG. `run_game` owns
// the game scope; each pipeline step is one helper in this file.

use super::cast_phase::{cast_phase, play_land};
use super::deal::{Opener, deal_opener};
use super::game::{
    GameLog, GameState, HAND_LIMIT, InPlay, Pool, card_of, fire_on_enter, fire_on_enter_opts,
    land_types, new_perm_with, register_loyalty_token_engines, take_uid,
};
use super::game_commander::{
    CommanderProfile, commander_sentinel_slot, commander_sentinel_uid, commander_upkeep_effects,
    is_commander_sentinel,
};
use super::game_effects::{apply_effect, spend_leftover, tap_budget};
use super::game_extra_turns::replay_extra_turns;
use super::game_mana::{
    add_yield, add_yield_turns, effective_min_cost, pay_cost, payable, pips_ok,
    usable_for_noncreature,
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
    // Extra-turn queue: the loop replays these after the main schedule
    // (one replay pass per queued turn, capped by remaining turns).
    let mut pending_extra_turns: u32 = 0;

    for turn in 1..=turns {
        run_turn(
            deck,
            &mut st,
            &mut census,
            &commander,
            &mut engines,
            turn,
            &mut pending_extra_turns,
        );
    }

    finish_log(st, census, opener_lands, mulliganed, turns)
}

/// Play one scheduled turn through the full pipeline.
#[allow(clippy::too_many_arguments)]
fn run_turn(
    deck: &SimDeck,
    st: &mut GameState,
    census: &mut TurnCensus,
    commander: &CommanderProfile,
    engines: &mut Vec<(u32, u32)>,
    turn: usize,
    pending_extra_turns: &mut u32,
) {
    // 1 UNTAP: everything untaps; sickness clears; once-per-turn resets.
    // Crew animations expire (Vehicles stop being bodies at end of
    // turn). Blink flags re-fire the host's OnEnter triggers once
    // (Skyskipper Duo, Conjurer's Closet-style flickers).
    expire_crew(st);
    fire_blink_refires(deck, st, turn as u32);
    for perm in st.battlefield.iter_mut() {
        perm.tapped = false;
        perm.sick = false;
        perm.fired = false;
    }
    st.prowess_casts = 0;

    // 2 UPKEEP: engines fire; saga chapters advance (one card each).
    run_upkeep(deck, st, engines, turn);
    run_sagas(deck, st, turn);
    check_win_thresholds(deck, st, census, turn);
    check_ultimates(deck, st, census, turn);

    // 3 DRAW.
    if let Some(i) = st.library.pop() {
        st.hand.push(i);
        st.seen += 1;
        st.awareness_cards += 1;
    }
    // Monarch: one extra card per turn from the turn after acquisition
    // (the Monarch draws at their upkeep).
    if st.is_monarch
        && let Some(i) = st.library.pop()
    {
        st.hand.push(i);
        st.seen += 1;
        st.awareness_cards += 1;
    }
    record_hand_sightings(deck, st, census, turn);

    // 4 LAND.
    play_land_drops(deck, st, census, turn);

    // 5 POOL, commander cast, casts, 6 ACTIVATE, 7 TAP BUDGET.
    let mut pool = build_pool(deck, st, turn as u32);
    commander_phase(deck, st, &mut pool, census, commander, turn, engines);
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
    spend_leftover(deck, st, &mut pool, turn as u32);
    tap_budget(deck, &mut st.battlefield, &mut pool, &st.hand);
    record_interaction_readiness(deck, st, &pool, census, turn);

    // 8 THRESHOLD: station tiers unlock (permanent for animate tiers).
    unlock_thresholds(deck, st, census, turn);
    // 9 COMBAT.
    record_combat(deck, st, census, turn);
    // Engine count for the turn (commander + cast-phase registrations).
    record_engine_count(census, turn, engines);
    // 10 END: hand-limit discard.
    end_step_discard(st, turn);
    // 11 EXTRA TURNS: queued extra turns replay a land drop, a draw, and
    // upkeep engines once (not full turns).
    replay_extra_turns(deck, st, census, engines, turn, pending_extra_turns);
    if *pending_extra_turns == 0 && st.extra_turns_queued > 0 {
        *pending_extra_turns = st.extra_turns_queued;
        st.extra_turns_queued = 0;
    }
}

/// Per-game census arrays the turn loop fills and the log carries.
pub(super) struct TurnCensus {
    pub(super) land_drops: Vec<u8>,
    pub(super) mana_available: Vec<f64>,
    pub(super) mana_spent: Vec<f64>,
    pub(super) cards_seen: Vec<u32>,
    pub(super) mana_ready: Vec<Option<u32>>,
    pub(super) first_seen: HashMap<Role, u32>,
    pub(super) first_creature: Option<u32>,
    pub(super) blocked_colors: [bool; 5],
    pub(super) commander_castable: Option<u32>,
    pub(super) station_online: Option<u32>,
    pub(super) bodies: Vec<u32>,
    pub(super) engines_online: Vec<u32>,
    pub(super) graveyard_size: Vec<u32>,
    pub(super) card_first_seen: HashMap<usize, u32>,
    pub(super) pip_blocks: Vec<(usize, usize)>,
    pub(super) attack_power: Vec<u32>,
    pub(super) attackers_turn: Vec<u32>,
    pub(super) evasive_turn: Vec<u32>,
    pub(super) library_size: Vec<u32>,
    pub(super) lands_seen: Vec<u32>,
    pub(super) self_milled: Vec<u32>,
    pub(super) opp_milled: Vec<u32>,
    pub(super) awareness: Vec<f64>,
    pub(super) drain_total: Vec<u32>,
    pub(super) player_damage: Vec<u32>,
    pub(super) extra_turns: Vec<u32>,
    pub(super) win_threshold_turn: Option<u32>,
    pub(super) ultimate_online: Option<u32>,
    pub(super) interaction_ready: Vec<bool>,
    pub(super) interaction_mana_held: Vec<f64>,
}

impl TurnCensus {
    /// One array slot per scheduled turn, keyed by the deck's card count.
    pub(super) fn new(turns: usize, deck_len: usize) -> Self {
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

/// Leyline-style openers begin the game on the battlefield; the moved
/// cards count as seen (they left the hand zone).
fn apply_leylines(deck: &SimDeck, st: &mut GameState) {
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
        let uid = take_uid(st);
        st.battlefield.push(new_perm_with(uid, deck, idx, 0, false));
    }
    st.seen = (st.hand.len() + st.battlefield.len()) as u32;
}

/// Crew animations revert at the turn boundary: a crewed Vehicle stops
/// being a body (and stops attacking) once the turn closes.
pub(crate) fn expire_crew(st: &mut GameState) {
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
        .filter(|(_, p)| p.blink_pending && p.card < usize::MAX - 1)
        .map(|(i, _)| i)
        .collect();
    for i in blink_positions {
        st.battlefield[i].blink_pending = false;
        fire_on_enter_opts(deck, st, i, turn, true);
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
            None => return,
        }
    };
    let upkeep_effects: Vec<Effect> = if is_cmd {
        // One sentinel per cast commander; the firing reads only that
        // commander's own abilities, so two partners each fire once per
        // turn instead of every upkeep effect firing per sentinel.
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
            Effect::Draw(n) => run_scaling_draw(deck, st, host_card, *n),
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
                    deck.cards.get(host_card).is_some_and(|c| c.mills_opponent)
                };
                apply_effect(deck, effect, st, turn as u32, mill_opp);
            }
            _ => {}
        }
    }
}

/// Scaling draw engines ("draw a card for each enchantment you control")
/// draw the matching permanent count, capped at 8; plain engines draw N.
pub(super) fn run_scaling_draw(deck: &SimDeck, st: &mut GameState, host_card: usize, base: u32) {
    let scaled = deck
        .cards
        .get(host_card)
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
                .filter(|p| p.card < usize::MAX - 1 && matches(&deck.cards[p.card]))
                .count()
                .min(8) as u32
        })
        .filter(|c| *c > 0)
        .unwrap_or(base);
    for _ in 0..scaled {
        if let Some(i) = st.library.pop() {
            st.hand.push(i);
            st.seen += 1;
            st.awareness_cards += 1;
        }
    }
}

/// Saga chapters advance one per turn from the turn after entry (the
/// one-turn delay mirrors the real "I" chapter landing a turn late);
/// the permanent leaves once the final chapter resolved.
fn run_sagas(deck: &SimDeck, st: &mut GameState, turn: usize) {
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
            apply_effect(deck, &effect, st, turn as u32, mill_opp);
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

/// Record role sightings from the hand (role-access census).
fn record_hand_sightings(deck: &SimDeck, st: &GameState, census: &mut TurnCensus, turn: usize) {
    for i in &st.hand {
        let card = &deck.cards[*i];
        census.first_seen.entry(card.role).or_insert(turn as u32);
        if card.is_creature && census.first_creature.is_none() {
            census.first_creature = Some(turn as u32);
        }
        census.card_first_seen.entry(*i).or_insert(turn as u32);
    }
}

/// 4 LAND: play an untapped land when one is in hand (the best-case
/// agent keeps tapped lands for later); any land plays otherwise.
/// "You may play an additional land" boards (Aesi, Wayward Swordtooth
/// class) play a second land the same turn.
fn play_land_drops(deck: &SimDeck, st: &mut GameState, census: &mut TurnCensus, turn: usize) {
    if play_land(deck, st, turn as u32) {
        census.land_drops[turn - 1] = 1;
        let extra_lands = st
            .battlefield
            .iter()
            .any(|p| p.card < usize::MAX - 1 && deck.cards[p.card].extra_land_drops);
        if extra_lands && play_land(deck, st, turn as u32) {
            census.land_drops[turn - 1] += 1;
        }
    }
}

/// Build the turn's spendable pool from untapped permanents: taps,
/// verge gates, banked-mana releases, counter-scaled taps, static
/// grants, and the Treasure bank.
fn build_pool(deck: &SimDeck, st: &mut GameState, turn: u32) -> Pool {
    let mut pool = Pool::default();
    release_banked_mana(deck, st, &mut pool, turn);
    for perm in &st.battlefield {
        if perm.tapped {
            continue;
        }
        let card = card_of(deck, perm);
        let Some(y) = &card.tap else {
            continue;
        };
        if let Some(Scale::PerChargeCounter) = y.scaling {
            // Counter-scaled taps resolve at the tap: one any-color pip
            // per charge counter on the source (Astral Cornucopia
            // class).
            for _ in 0..perm.counters {
                add_yield_turns(
                    deck,
                    &super::model::TapYield {
                        any_pips: 1,
                        ..super::model::TapYield::default()
                    },
                    &mut pool,
                    turn,
                    &st.battlefield,
                );
            }
            continue;
        }
        if card.gate_types.is_empty() {
            add_yield_turns(deck, y, &mut pool, turn, &st.battlefield);
            continue;
        }
        add_gated_yield(deck, st, perm, y, &mut pool, turn);
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

/// Banked-mana engines release at the upkeep (Coalition Relic):
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
                    a.trigger == Trigger::OnUpkeep && matches!(a.effect, Effect::ManaPerCounter(_))
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
        st.battlefield.iter().any(|p| {
            p.card != perm.card
                && card_of(deck, p).role == Role::Land
                && land_types(&card_of(deck, p).name).contains(want)
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
                    card.is_creature && !p.is_commander
                } else {
                    card.role == Role::Land && !p.is_commander
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
) {
    // Snapshot the pool first: the mana-readiness check below must see
    // the pre-cast pool, or a same-turn commander cast pushes every
    // other card's readiness a turn later.
    let pool_before_commander = pool.clone();
    for (cmd_i, cmd) in deck.commanders.iter().enumerate() {
        if st
            .battlefield
            .iter()
            .any(|p| p.is_commander && card_of(deck, p).name == cmd.name)
        {
            continue;
        }
        // Gate on the general pool only: `pay_cost` never spends the
        // restricted buckets (creature/legendary/artifact/instant-
        // sorcery mana pays only its own cast class), so counting them
        // here would cast commanders with mana never deducted.
        if usable_for_noncreature(pool) < cmd.cost.total() || !pips_ok(&cmd.cost, pool) {
            // Skip this commander; the next partner still gets its try
            // this turn.
            continue;
        }
        // Deduct from the general pool only, mirroring the gate.
        pay_cost(&cmd.cost, pool);
        census.mana_spent[turn - 1] += cmd.cost.total() as f64;
        if census.commander_castable.is_none() {
            census.commander_castable = Some(turn as u32);
        }
        // The commander is a permanent with no library entry.
        let cmd_uid = take_uid(st);
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
        fire_on_enter(deck, st, cmd_pos, turn as u32);
    }

    // Mana available is recorded from the pre-cast snapshot: the
    // cast/activation passes spend from the same pool and `mana_spent`
    // re-adds those costs, so recording post-payment availability would
    // double-subtract in the unused-mana metric.
    census.mana_available[turn - 1] = pool_before_commander.total() as f64;

    // Mana-readiness: the first turn the board could pay each card's
    // cost, independent of drawing it (the castability curve). The
    // check uses the pre-cast pool: a same-turn commander cast must not
    // push every other card's readiness a turn later.
    for (idx, card) in deck.cards.iter().enumerate() {
        if card.role != Role::Land && census.mana_ready[idx].is_none() {
            let eff = effective_min_cost(deck, card, &st.battlefield);
            if payable(&eff, &pool_before_commander) && pips_ok(&eff, &pool_before_commander) {
                census.mana_ready[idx] = Some(turn as u32);
            }
        }
    }
}

/// 7b INTERACTION READINESS (measured, not forced): was instant-speed
/// interaction in hand while spare mana covered its cost? The cheapest
/// answer in hand decides; the goldfish never spends it.
fn record_interaction_readiness(
    deck: &SimDeck,
    st: &GameState,
    pool: &Pool,
    census: &mut TurnCensus,
    turn: usize,
) {
    let cheapest = st
        .hand
        .iter()
        .filter_map(|i| {
            let c = &deck.cards[*i];
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
            if perm.is_commander && census.station_online.is_none() {
                census.station_online = Some(turn as u32);
            }
        }
    }
}

/// 9 COMBAT: run the combat phase and record the per-turn census.
fn record_combat(deck: &SimDeck, st: &mut GameState, census: &mut TurnCensus, turn: usize) {
    let combat = super::game_combat::combat_phase(
        deck,
        st,
        turn,
        census.attackers_turn.len(),
        &census.land_drops,
    );
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
        .filter(|i| deck.cards[**i].role == Role::Land)
        .count()
        + st.battlefield
            .iter()
            // Token permanents carry sentinel card indexes; only real
            // cards can be lands.
            .filter(|p| p.card < usize::MAX - 1 && deck.cards[p.card].role == Role::Land)
            .count()
        + st.graveyard
            .iter()
            .filter(|i| deck.cards[**i].role == Role::Land)
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
    census.extra_turns[turn - 1] = st.extra_turns_queued;
    census.bodies[turn - 1] = st
        .battlefield
        .iter()
        .filter(|p| card_of(deck, p).is_creature || p.animated)
        .count() as u32
        + token_bodies.min(4);
}

/// Record the engine count for the turn (called after the engine list
/// settles: commander cast, cast-phase registrations).
fn record_engine_count(census: &mut TurnCensus, turn: usize, engines: &[(u32, u32)]) {
    census.engines_online[turn - 1] = engines.len() as u32;
}

/// 10 END: hand-limit discard from the end of the hand. The graveyard
/// entry records THIS turn (the discard turn), not the game length.
fn end_step_discard(st: &mut GameState, turn: usize) {
    while st.hand.len() > HAND_LIMIT {
        let discarded = st.hand.remove(st.hand.len() - 1);
        st.graveyard_seen.entry(discarded).or_insert(turn as u32);
        st.graveyard.push(discarded);
    }
}

/// Build the final `GameLog` from the census and the game state.
fn finish_log(
    st: GameState,
    census: TurnCensus,
    opener_lands: u8,
    mulliganed: bool,
    turns: usize,
) -> GameLog {
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
        pip_blocks: census.pip_blocks,
        graveyard_size: census.graveyard_size,
        card_first_seen: census.card_first_seen,
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
        win_threshold_turn: census.win_threshold_turn,
        ultimate_online: census.ultimate_online,
        interaction_ready: census.interaction_ready,
        interaction_mana_held: census.interaction_mana_held,
        card_first_battlefield: st.battlefield_seen,
        card_first_graveyard: st.graveyard_seen,
        infinite_mana_suspected: st.infinite_mana_suspected,
    }
}
