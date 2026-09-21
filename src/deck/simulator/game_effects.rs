// Effect execution and the tap-budget pass for the goldfish game loop,
// split from game.rs to keep files small.

use super::game::{Activation, BODY_POWER, GameState, InPlay, Pool, card_of};
use super::game_mana::{
    add_yield, add_yield_turns, effective_min_cost, pay_cost, payable, pips_ok,
};
use super::model::{Effect, Role, SimDeck, Trigger};

/// Execute one ability effect against the game state. Pure helper for the
/// trigger paths; the upkeep/combat paths call it too. `mill_opp` routes
/// Mill pips to the opponent census ("target player mills") or self.
pub(super) fn apply_effect(
    deck: &SimDeck,
    effect: &Effect,
    st: &mut GameState,
    turn: u32,
    mill_opp: bool,
) {
    apply_effect_at(deck, effect, st, turn, mill_opp, None)
}

/// The full variant: `source` is the battlefield card index whose
/// effect resolves (Treasure flags read from that card).
pub(crate) fn apply_effect_at(
    deck: &SimDeck,
    effect: &Effect,
    st: &mut GameState,
    turn: u32,
    mill_opp: bool,
    source: Option<usize>,
) {
    // Treasure banking requires the token effect's own card to create
    // Treasures ("create a Treasure token" on the same card). A deck-wide
    // blanket would convert unrelated token effects into pips.
    let treasure_source = source
        .and_then(|i| deck.cards.get(i))
        .map(|c| c.treasures_on_token)
        .unwrap_or(false);
    match effect {
        Effect::Draw(n) => {
            for _ in 0..*n {
                if let Some(i) = st.library.pop() {
                    st.hand.push(i);
                    st.seen += 1;
                    st.awareness_cards += 1;
                }
            }
        }
        Effect::Tutor | Effect::ExtraLand => {
            if let Some(i) = st.library.pop() {
                st.hand.push(i);
                st.seen += 1;
            }
        }
        Effect::Mill(n) => {
            for _ in 0..*n {
                if let Some(i) = st.library.pop() {
                    st.graveyard_seen.entry(i).or_insert(turn);
                    st.graveyard.push(i);
                    st.seen += 1;
                    st.awareness_cards += 1;
                    if mill_opp {
                        st.milled_opp += 1;
                    } else {
                        st.milled_self += 1;
                    }
                }
            }
        }
        Effect::Scry(n) => {
            // Awareness only; no draw credit. Activated surveil keeps
            // the cards (rare); on-cast surveil routes in the cast path.
            st.awareness_cards += *n;
        }
        Effect::Drain(n) => {
            // Three opponents in Commander: a player-targeted drain
            // resolves once, an "each opponent" drain triples. Both
            // read as N×3 life off the table (best case: all resolve).
            // Constructed tables are one opponent: ×1.
            let mult = if deck.format == super::model::Format::Commander {
                3
            } else {
                1
            };
            st.drained += *n * mult;
        }
        Effect::ExtraTurn => {
            st.extra_turns_queued += 1;
        }
        Effect::ReturnFromGraveyard { to_hand, count } => {
            for _ in 0..*count {
                let Some(i) = st.graveyard.pop() else {
                    break;
                };
                if *to_hand {
                    st.hand.push(i);
                    st.seen += 1;
                } else if let Some(card) = deck.cards.get(i)
                    && card.is_creature
                {
                    // Returns as a body once; the card leaves the log.
                    st.battlefield_seen.entry(i).or_insert(turn);
                    st.battlefield.push(InPlay {
                        card: i,
                        tapped: false,
                        sick: true,
                        counters: 0,
                        animated: false,
                        crewed: false,
                        entered_turn: turn as usize,
                        saga_step: 0,
                        fired: false,
                        blink_pending: false,
                        loyalty: 0,
                        equipped: false,
                        equip_host: None,
                        is_commander: false,
                        commander_slot: 0,
                    });
                } else {
                    st.hand.push(i);
                    st.seen += 1;
                }
            }
        }
        Effect::Wheel => {
            for i in st.hand.drain(..) {
                st.graveyard_seen.entry(i).or_insert(turn);
                st.graveyard.push(i);
            }
            for _ in 0..7 {
                if let Some(i) = st.library.pop() {
                    st.hand.push(i);
                    st.seen += 1;
                }
            }
        }
        // A wheel resolving mid-cast: skip one hand index (the cast
        // spell itself, whose removal is deferred) so it is not
        // double-zoned, then draw seven.
        Effect::WheelSkip(skip) => {
            let hand = std::mem::take(&mut st.hand);
            for i in hand {
                if i == *skip {
                    st.hand.push(i);
                    continue;
                }
                st.graveyard_seen.entry(i).or_insert(turn);
                st.graveyard.push(i);
            }
            for _ in 0..7 {
                if let Some(i) = st.library.pop() {
                    st.hand.push(i);
                    st.seen += 1;
                }
            }
        }
        Effect::Loot(n) => {
            for _ in 0..*n {
                if let Some(i) = st.library.pop() {
                    st.hand.push(i);
                    st.seen += 1;
                }
                // Discard the oldest hand card into the graveyard.
                if !st.hand.is_empty() {
                    let discarded = st.hand.remove(0);
                    st.graveyard_seen.entry(discarded).or_insert(turn);
                    st.graveyard.push(discarded);
                }
            }
        }
        Effect::Tokens(n) => {
            // Treasure-creating cards bank Treasure pips instead of
            // bodies (one banked any-color pip per Treasure,
            // sacrificed to use). Documented in assumptions.
            if treasure_source {
                st.treasure_bank += n;
                return;
            }
            // Token bodies join as small station/crew fuel. The count
            // creates that many bodies (capped so go-wide boards stay
            // bounded).
            for _ in 0..(*n).clamp(1, 8) {
                st.battlefield.push(InPlay {
                    card: usize::MAX - 1,
                    tapped: false,
                    sick: true,
                    counters: 0,
                    animated: false,
                    crewed: false,
                    entered_turn: turn as usize,
                    saga_step: 0,
                    fired: false,
                    blink_pending: false,
                    loyalty: 0,
                    equipped: false,
                    equip_host: None,
                    is_commander: false,
                    commander_slot: 0,
                });
            }
        }
        _ => {}
    }
}

/// The tap-budget pass: mana only while casting still needs mana, then
/// station, then crew, then equip (suit up a body so its buff counts in
/// combat).
pub(super) fn tap_budget(
    deck: &SimDeck,
    battlefield: &mut [InPlay],
    pool: &mut Pool,
    hand: &[usize],
) {
    // Remaining demand: the cheapest uncast spell still in hand.
    let cheapest: Option<u32> = hand
        .iter()
        .filter(|i| deck.cards[**i].role != Role::Land)
        .map(|i| effective_min_cost(deck, &deck.cards[*i], battlefield).total())
        .min();

    // Creatures and crewed vehicles able to tap this turn.
    let tappable: Vec<usize> = battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            !p.tapped
                && !p.sick
                && !p.is_commander
                && !card_of(deck, p).is_station_card
                && (card_of(deck, p).is_creature || p.animated)
        })
        .map(|(i, _)| i)
        .collect();

    // 7a Mana taps: only while the cheapest spell still needs mana.
    if let Some(need) = cheapest {
        for &bi in &tappable {
            if pool.total() >= need {
                break;
            }
            let card = card_of(deck, &battlefield[bi]);
            if let Some(y) = &card.tap {
                add_yield(y, pool);
                battlefield[bi].tapped = true;
            }
        }
    }

    // 7b Station: tap remaining bodies into the highest-threshold unfilled
    // spacecraft/planet, one at a time, counters = body power.
    for &bi in &tappable {
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
                        .is_some_and(|at| p.counters < at)
            })
            .max_by_key(|(_, p)| card_of(deck, p).animate_at().unwrap_or(0))
            .map(|(i, _)| i);
        if let Some(ti) = target {
            battlefield[ti].counters += body_power(&battlefield[bi], deck);
            battlefield[bi].tapped = true;
        }
    }

    // 7c Crew: tap bodies into untapped Vehicles (total power ≥ crew N).
    for vi in 0..battlefield.len() {
        if battlefield[vi].tapped
            || battlefield[vi].sick
            || card_of(deck, &battlefield[vi]).crew.is_none()
        {
            continue;
        }
        let crew_n = card_of(deck, &battlefield[vi]).crew.unwrap_or(1);
        let bodies_needed = crew_n.div_ceil(BODY_POWER);
        let mut tapped_bodies = 0;
        let mut power = 0u32;
        for (i, perm) in battlefield.iter_mut().enumerate() {
            if tapped_bodies >= bodies_needed {
                break;
            }
            if i != vi
                && !perm.tapped
                && !perm.sick
                && !perm.is_commander
                && (card_of(deck, perm).is_creature || perm.animated)
            {
                perm.tapped = true;
                tapped_bodies += 1;
                power += body_power(perm, deck);
            }
        }
        if power >= crew_n {
            battlefield[vi].crewed = true;
            battlefield[vi].sick = false;
            // OnCrewed triggers fire now (tokens join next turn's bodies).
        }
    }

    // 7d Equip: pay the equip cost once per Equipment while spare mana
    // covers it. The gear suits up its best body (highest printed power,
    // untapped and unsick so it attacks); the buff joins that host's
    // attack only.
    for ei in 0..battlefield.len() {
        if battlefield[ei].tapped || battlefield[ei].equipped {
            continue;
        }
        let eq = card_of(deck, &battlefield[ei]).equipment;
        let Some(eq) = eq else {
            continue;
        };
        if eq.cost == 0 {
            continue;
        }
        // Host: the strongest untapped, unsick non-commander body.
        let host = (0..battlefield.len())
            .filter(|hi| {
                *hi != ei
                    && !battlefield[*hi].is_commander
                    && !battlefield[*hi].tapped
                    && !battlefield[*hi].sick
                    && card_of(deck, &battlefield[*hi]).is_creature
            })
            .max_by_key(|hi| body_power(&battlefield[*hi], deck));
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
            battlefield[ei].equip_host = Some(hi);
            battlefield[ei].tapped = eq.cost > 0 && card_of(deck, &battlefield[ei]).tap.is_none();
            battlefield[ei].fired = true;
        }
    }
}

/// Body power for a permanent: the printed power when the card row has
/// one, else the flat token value. Crew and station math use it.
pub(super) fn body_power(perm: &InPlay, deck: &SimDeck) -> u32 {
    if perm.card < usize::MAX - 1 {
        deck.cards[perm.card].printed_power.unwrap_or(BODY_POWER)
    } else {
        BODY_POWER
    }
}

/// The spend-leftover-mana pass: repeatedly fire the cheapest unlocked
/// activation (draw engines, mana engines, walkers, sacrifice outlets).
/// Each firing taps the source or spends loyalty; sacrifice outlets feed
/// death triggers from the surviving board. Loyalty activations bypass
/// the mana pool (they spend loyalty); drain activations resolve at the
/// format's opponent multiplier. An untapped non-tapping activation
/// whose yield covers its own cost repeats — the loop caps it and flags
/// the census as a suspected infinite engine.
pub(super) fn spend_leftover(deck: &SimDeck, st: &mut GameState, pool: &mut Pool, turn: u32) {
    let drain_mult = if deck.format == super::model::Format::Commander {
        3
    } else {
        1
    };
    let mut activations_this_turn: u32 = 0;
    let mut suspect_infinite = false;
    loop {
        let mut best: Option<Activation> = None;
        for (bi, perm) in st.battlefield.iter().enumerate() {
            if perm.tapped {
                continue;
            }
            let card = card_of(deck, perm);
            // Loyalty activations gate on loyalty, not mana, so they
            // pass the tap/counter filter too (planeswalker minus and
            // plus abilities fire here).
            let unlocked = card
                .station_tiers
                .iter()
                .filter(|t| t.at == 0 || perm.counters >= t.at)
                .flat_map(|t| t.abilities.iter())
                .filter(|a| {
                    let free_mana = a.cost.total() == 0 && matches!(a.effect, Effect::Mana(_));
                    // X-sink counters ("{X}: Put X tower counters") cost
                    // mana without tapping: the leftover pool converts.
                    let x_sink = matches!(a.effect, Effect::Counters(0));
                    a.trigger == Trigger::Activated
                        && (a.taps
                            || a.uses_counters
                            || a.loyalty_cost > 0
                            || a.loyalty_gain > 0
                            || free_mana
                            || x_sink)
                });
            for ability in unlocked {
                let loyalty_affordable =
                    ability.loyalty_cost == 0 || perm.loyalty >= ability.loyalty_cost;
                let usable = matches!(
                    ability.effect,
                    Effect::Draw(_)
                        | Effect::Tutor
                        | Effect::Mana(_)
                        | Effect::Counters(_)
                        | Effect::Loot(_)
                        | Effect::Drain(_)
                ) || ability.sacrifice_bodies > 0;
                // Banked activations (Pentad Prism) consume a charge
                // counter per fire; gate on the host's counters. Free
                // untapped activations ({0}: Add ...) bypass `fired` —
                // they repeat in real Magic and feed the loop census —
                // unless the card bounds them ("Activate only once each
                // turn").
                let banked = ability.uses_counters && perm.counters > 0;
                let free_loop = !ability.taps
                    && !ability.uses_counters
                    && ability.loyalty_cost == 0
                    && ability.loyalty_gain == 0
                    && ability.cost.total() == 0
                    && !ability.once_per_turn;
                // Free untapped activations repeat every turn regardless
                // of `fired` (they have no once-per-turn cost marker);
                // the cap catches runaway loops.
                let gated = !free_loop && perm.fired;
                if gated {
                    continue;
                }
                if usable
                    && loyalty_affordable
                    && (banked
                        || ability.loyalty_cost > 0
                        || (payable(&ability.cost, pool) && pips_ok(&ability.cost, pool)))
                    && best.as_ref().is_none_or(|b| {
                        let candidate_cost = ability.cost.total() + ability.sacrifice_bodies.min(1);
                        // Free untapped candidates win cost ties: they
                        // leave the source untapped so the pass can
                        // keep looping ({0}: Add mode beats the {T}
                        // mode for loop engines).
                        candidate_cost < b.cost
                            || (candidate_cost == b.cost && b.ability.taps && !ability.taps)
                    })
                {
                    let draws = match ability.effect {
                        Effect::Draw(n) => n,
                        Effect::Tutor => 1,
                        Effect::Loot(n) => n,
                        _ => 0,
                    };
                    let mana = match &ability.effect {
                        Effect::Mana(y) => Some(y.clone()),
                        _ => None,
                    };
                    let counters = match ability.effect {
                        Effect::Counters(n) => n,
                        _ => 0,
                    };
                    let drain = match ability.effect {
                        Effect::Drain(n) => n,
                        _ => 0,
                    };
                    best = Some(Activation {
                        pos: bi,
                        ability: ability.clone(),
                        cost: ability.cost.total(),
                        draws,
                        mana_yield: mana,
                        counters,
                        drain,
                    });
                }
            }
        }
        let Some(a) = best else {
            break;
        };
        activations_this_turn += 1;
        if activations_this_turn > 24 {
            // Repeated zero-cost activations that produce mana: the
            // engine loops (Basalt Monolith class). Cap the pass.
            suspect_infinite = true;
            break;
        }
        let x_sink = matches!(a.ability.effect, Effect::Counters(0));
        let perm = &mut st.battlefield[a.pos];
        let ability = a.ability.clone();
        // Loyalty activations spend/gain loyalty; mana costs do not apply.
        if ability.loyalty_cost > 0 {
            perm.loyalty = perm.loyalty.saturating_sub(ability.loyalty_cost);
            perm.fired = true;
        } else if ability.loyalty_gain > 0 {
            perm.loyalty += ability.loyalty_gain;
            perm.fired = true;
        } else if ability.uses_counters {
            // Banked activation: one charge counter buys the pip; the
            // source stays untapped but can fire once per turn (fired
            // gates the repeat pass; the counter decrement still applies).
            perm.counters = perm.counters.saturating_sub(1);
            perm.fired = true;
        } else if !ability.taps && ability.cost.total() == 0 {
            // Free untapped activation: stays untapped, never sets
            // `fired` — the loop runs until the cap catches it. A
            // once-per-turn bound sets `fired` (no loop).
            if ability.once_per_turn {
                perm.fired = true;
            }
        } else {
            pay_cost(&ability.cost, pool);
            // Only `{T}`-cost activations tap the source; free
            // activations ({0}:) stay untapped.
            if a.ability.taps {
                perm.tapped = true;
            }
            perm.fired = true;
        }
        for _ in 0..a.draws {
            if let Some(i) = st.library.pop() {
                st.hand.push(i);
                st.seen += 1;
            }
        }
        // Drain activations resolve at the format's opponent multiplier.
        st.drained += a.drain * drain_mult;
        // Sacrifice outlets feed an untapped non-token body to the
        // cost: it leaves play and its OnDeath triggers fire.
        for _ in 0..ability.sacrifice_bodies {
            let victim = st.battlefield.iter().position(|p| {
                !p.is_commander
                    && p.card < usize::MAX - 1
                    && p.card != a.pos
                    && card_of(deck, p).is_creature
                    && !p.tapped
            });
            let Some(v) = victim else {
                break;
            };
            let victim_card = st.battlefield[v].card;
            st.battlefield.remove(v);
            st.graveyard_seen.entry(victim_card).or_insert(turn);
            st.graveyard.push(victim_card);
            // Death triggers: draw/token payoffs fire from the
            // surviving board.
            let death_effects: Vec<Effect> = st
                .battlefield
                .iter()
                .flat_map(|p| card_of(deck, p).abilities().cloned().collect::<Vec<_>>())
                .filter(|ab| ab.trigger == Trigger::OnDeath)
                .map(|ab| ab.effect)
                .collect();
            for effect in &death_effects {
                // Death-trigger mills are graveyard fuel (self).
                apply_effect(deck, effect, st, turn, false);
            }
        }
        // Mana activations feed this turn's pool; counter engines
        // (Moxite Refinery) charge their host.
        if let Some(y) = &a.mana_yield {
            add_yield_turns(deck, y, pool, turn, &st.battlefield);
        }
        if let Some(perm) = st.battlefield.get_mut(a.pos) {
            if x_sink {
                perm.counters += pool.total();
                *pool = super::game::Pool::default();
            } else {
                perm.counters += a.counters;
            }
        }
    }
    if suspect_infinite {
        st.infinite_mana_suspected = true;
    }
}
