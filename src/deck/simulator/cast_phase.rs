// The cast pass and land-drop helpers for the goldfish game loop, split
// from game_run.rs to keep files small. Pure apart from the game state
// mutations they drive.

use super::game::{
    GameState, InPlay, Pool, card_of, fetches_land_text, fire_on_enter, new_perm,
    register_loyalty_token_engines,
};
use super::game_effects::{apply_effect, apply_effect_at};
use super::game_mana::{
    add_yield_turns_empty_board, effective_min_cost, pay_cost, pay_creature_cost, payable, pips_ok,
    usable_for_creature, usable_for_noncreature,
};
use super::model::{Effect, Role, SimDeck, Trigger};

/// The drain multiplier for this deck's format: three opponents in the
/// commander family, one in constructed.
fn drain_mult_in(deck: &SimDeck) -> u32 {
    if deck.format == super::model::Format::Commander {
        3
    } else {
        1
    }
}

/// Play one land from the hand (untapped first), fetch a search land,
/// and fire its ETB triggers. Returns true when a land was played.
pub(super) fn play_land(deck: &SimDeck, st: &mut GameState, turn: u32) -> bool {
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
pub(super) fn cast_phase(
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
        // Mid-cast hand changes (wheels, loots) shrink the hand; stale
        // positions from the cast order just skip.
        if pos >= st.hand.len() {
            continue;
        }
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
        // Kicker: an optional extra cost paid from leftover mana. Best
        // case the goldfish kicks when the pool covers it. The rider
        // bumps the drain/damage amount (the modeled kicker payoff).
        let kicked = if let Some(k) = card.kicker
            && pool.total() >= k
        {
            pay_cost(
                &super::model::Cost {
                    generic: k,
                    ..super::model::Cost::default()
                },
                pool,
            );
            spent_total += k;
            true
        } else {
            false
        };
        cast_positions.push(pos);
        // The permanent pushed below; battlefield scans skip it (its
        // per-cast engine already fired for the casts so far).
        let cast_perm_pos = st.battlefield.len();
        // Additional costs: the cast consumes bodies ("sacrifice a
        // creature") and life ("pay N life"). Sacrificed bodies leave
        // the battlefield and fire their death triggers next turn
        // (the removal is immediate; the log fills now).
        for _ in 0..card.additional_cost_bodies {
            let victim = st.battlefield.iter().position(|p| {
                !p.is_commander
                    && p.card < usize::MAX - 1
                    && card_of(deck, p).is_creature
                    && !p.tapped
            });
            let Some(v) = victim else {
                break;
            };
            let victim_card = st.battlefield[v].card;
            st.battlefield.remove(v);
            st.graveyard_seen.entry(victim_card).or_insert(turn as u32);
            st.graveyard.push(victim_card);
        }
        if card.additional_cost_life > 0 {
            st.drained += card.additional_cost_life;
        }
        // Producers join the battlefield: rocks tap at once, creatures
        // from next turn (summoning sickness). Vehicles and spacecraft
        // join as artifacts. "Enters with X charge counters" cards
        // convert the cast's leftover pool into counters (Astral
        // Cornucopia class).
        let entry_counters = if card.enter_counters == super::parse_land::X_ENTRY_COUNTERS {
            let x = pool.total();
            pool.colorless = 0;
            pool.flexible = 0;
            pool.fixed = [0; 5];
            pool.creature_only = 0;
            spent_total += x;
            x
        } else {
            card.enter_counters
        };
        st.battlefield_seen.entry(idx).or_insert(turn as u32);
        st.battlefield.push(InPlay {
            card: idx,
            tapped: false,
            sick: card.is_creature,
            counters: entry_counters,
            animated: false,
            crewed: false,
            entered_turn: turn,
            saga_step: 0,
            fired: false,
            blink_pending: false,
            loyalty: card.starting_loyalty.unwrap_or(0),
            equipped: false,
            equip_host: None,
            is_commander: false,
            commander_slot: 0,
        });
        // Planeswalker +1 token engines register at first cast: a
        // loyalty-gain activation that creates tokens is a repeatable
        // once-per-turn engine (Liliana-class token fuel).
        let pw_pos = st.battlefield.len() - 1;
        register_loyalty_token_engines(deck, st, pw_pos, engines);
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
        // opponents in the commander family) or ×1 constructed;
        // creature-target burn never got here (Removal). A paid kicker
        // bumps the drain amount.
        if card.drain_on_cast > 0 {
            let rider = card.drain_on_cast + u32::from(kicked) * card.drain_on_cast.max(1);
            st.drained += rider * drain_mult_in(deck);
        }
        // One-shot token spells ("Create four 1/1 Soldier creature
        // tokens"): the cast resolves the creation.
        if card.tokens_on_cast > 0 {
            apply_effect_at(
                deck,
                &Effect::Tokens(card.tokens_on_cast.min(8)),
                st,
                turn as u32,
                false,
                Some(idx),
            );
        }
        // One-shot wheel spells ("each player discards, then draws").
        // The just-cast wheel is still in hand (removal is deferred), so
        // the skip variant keeps it out of the graveyard log.
        if card.wheel_on_cast {
            apply_effect(deck, &Effect::WheelSkip(idx), st, turn as u32, false);
        }
        // X-cost spells pay the leftover pool as X and scale the effect
        // (best case: X = everything floatable). The generic {X} already
        // paid 1; the rest of the pool converts.
        if let Some(class) = card.x_class {
            let x = pool.total().max(1);
            pool.colorless = 0;
            pool.flexible = 0;
            pool.fixed = [0; 5];
            pool.creature_only = 0;
            spent_total += x;
            match class {
                super::model::XClass::Drain => {
                    st.drained += x * drain_mult_in(deck);
                }
                super::model::XClass::Draw => {
                    for _ in 0..x {
                        if let Some(i) = st.library.pop() {
                            st.hand.push(i);
                            st.seen += 1;
                            st.awareness_cards += 1;
                        }
                    }
                }
                super::model::XClass::Mill => {
                    let mill_opp = card.mills_opponent;
                    for _ in 0..x {
                        if let Some(i) = st.library.pop() {
                            st.graveyard_seen.entry(i).or_insert(turn as u32);
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
                super::model::XClass::Tokens => {
                    apply_effect_at(
                        deck,
                        &Effect::Tokens(x.min(8)),
                        st,
                        turn as u32,
                        false,
                        Some(idx),
                    );
                }
            }
        }
        // Prowess census: noncreature spells cast this turn. The same
        // counter feeds cast-count engines (storm mana, per-cast drains).
        if !card.is_creature && card.role != Role::Land {
            st.prowess_casts += 1;
        }
        // Cast-count engines: "add {N} for each spell cast this turn"
        // joins the pool now (Vivi-class mana engines, best case). The
        // just-cast host fires for spells before it; hosts already on
        // the battlefield fire for every spell cast this turn.
        if let Some(y) = &card.mana_per_cast {
            for _ in 0..st.prowess_casts {
                add_yield_turns_empty_board(y, pool, turn as u32);
            }
        }
        for (pi, p) in st.battlefield.iter().enumerate() {
            if pi == cast_perm_pos {
                // The just-cast host fired above.
                continue;
            }
            if let Some(y) = &card_of(deck, p).mana_per_cast {
                add_yield_turns_empty_board(y, pool, turn as u32);
            }
        }
        // OnCastSpell engines fire per spell cast. Draw engines draw;
        // loot fills the graveyard; per-cast drains resolve at the
        // family multiplier.
        for p in st.battlefield.clone() {
            for ability in card_of(deck, &p).abilities() {
                if ability.trigger != Trigger::OnCastSpell {
                    continue;
                }
                match &ability.effect {
                    Effect::Draw(n) => {
                        for _ in 0..*n {
                            if let Some(i) = st.library.pop() {
                                st.hand.push(i);
                                st.seen += 1;
                            }
                        }
                    }
                    Effect::Loot(n) => {
                        apply_effect(deck, &Effect::Loot(*n), st, turn as u32, false);
                    }
                    Effect::Drain(n) => {
                        st.drained += n * drain_mult_in(deck);
                    }
                    _ => {}
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
    // Mid-cast hand changes (wheels, loots) shrink the hand, so removal
    // positions clamp to the live length.
    cast_positions.sort_unstable();
    for pos in cast_positions.into_iter().rev() {
        if pos < st.hand.len() {
            st.hand.remove(pos);
        }
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
            let engine_effect = match (&ability.trigger, &ability.effect) {
                (Trigger::OnUpkeep, Effect::Draw(n)) => Some(*n),
                // Mill, recursion, drain, and token upkeep engines run
                // through the effect executor like the draw ones.
                (
                    Trigger::OnUpkeep,
                    Effect::Mill(_)
                    | Effect::ReturnFromGraveyard { .. }
                    | Effect::Drain(_)
                    | Effect::Tokens(_),
                ) => Some(0),
                _ => None,
            };
            if let Some(draws) = engine_effect {
                let live_pos = st
                    .battlefield
                    .iter()
                    .position(|p| p.card == card_idx && p.entered_turn == turn)
                    .unwrap_or(pos);
                engines.push((live_pos, draws));
            }
        }
    }
}
