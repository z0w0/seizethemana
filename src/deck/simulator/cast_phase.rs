// The cast pass and land-drop helpers for the goldfish game loop, split
// from game_run.rs to keep files small. Pure apart from the game state
// mutations they drive.

use super::game::{
    GameState, InPlay, Pool, card_of, fetches_land_text, fire_on_enter, new_perm_with,
    register_loyalty_token_engines, take_uid,
};
use super::game_effects::{apply_effect, apply_effect_at};
use super::game_mana::{
    add_yield_turns_empty_board, cast_restriction, effective_min_cost, pay_cost,
    pay_restricted_cost, payable, pips_ok, usable_for_noncreature,
};
use super::model::{Effect, Restriction, Role, SimDeck, Trigger};

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
    // Land/spell MDFCs play their land face only when the hand holds no
    // other land to play this turn; otherwise they stay as spells.
    let mdfc_fallback = |st: &GameState| {
        st.hand
            .iter()
            .position(|idx| deck.cards[*idx].is_mdfc_spell)
            .filter(|_| !st.hand.iter().any(|i| deck.cards[*i].role == Role::Land))
    };
    let land_pos = st
        .hand
        .iter()
        .position(|idx| deck.cards[*idx].role == Role::Land && !deck.cards[*idx].enters_tapped)
        .or_else(|| {
            st.hand
                .iter()
                .position(|idx| deck.cards[*idx].role == Role::Land)
        })
        .or_else(|| mdfc_fallback(st));
    let Some(pos) = land_pos else {
        return false;
    };
    let idx = st.hand.remove(pos);
    let card = &deck.cards[idx];
    let tapped_in = card.enters_tapped;
    st.battlefield_seen.entry(idx).or_insert(turn);
    let uid = take_uid(st);
    st.battlefield
        .push(new_perm_with(uid, deck, idx, turn, tapped_in));
    if fetches_land_text(card) {
        // Search up a land from the library (enters tapped).
        if let Some(i) = st
            .library
            .iter()
            .position(|c| deck.cards[*c].role == Role::Land && !fetches_land_text(&deck.cards[*c]))
        {
            let fetched = st.library.remove(i);
            st.battlefield_seen.entry(fetched).or_insert(turn);
            let fuid = take_uid(st);
            st.battlefield
                .push(new_perm_with(fuid, deck, fetched, turn, true));
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
// The 11 parameters are the game state the cast phase needs in full;
// a parameter struct would just be read back out field by field.
#[allow(clippy::too_many_arguments)]
pub(super) fn cast_phase(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut Pool,
    turn: usize,
    mana_spent: &mut [f64],
    engines: &mut Vec<(u32, u32)>,
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
    // (uid, card index) of each cast's battlefield permanent: the ETB
    // pass fires only for these (lands and effect-pushes are excluded).
    let mut cast_ets: Vec<(u32, usize)> = Vec::new();
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
        // Spend-restricted mana pays only its cast class: the general
        // pool plus the matching bucket counts toward the cast; other
        // restricted buckets do not. The bucket covers the card's pips
        // like any other source (each restricted pip is one mana of the
        // source's chosen color, so the bucket must cover every pip,
        // once).
        let restriction = cast_restriction(card);
        let (cost_ok, pip_ok) = if let Some(restriction) = restriction {
            let bucket = match restriction {
                Restriction::Creature => pool.creature_only,
                Restriction::Legendary => pool.legendary_only,
                Restriction::Artifact => pool.artifact_only,
                Restriction::InstantSorcery => pool.instant_sorcery_only,
            };
            let pip_total: u32 =
                eff.pips.iter().map(|p| u32::from(*p)).sum::<u32>() + eff.flex_pips;
            (
                pool.usable_for(restriction) >= eff.total(),
                pips_ok(&eff, pool) || (bucket > 0 && bucket >= pip_total),
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
        if let Some(restriction) = cast_restriction(card) {
            pay_restricted_cost(&eff, pool, restriction);
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
        // The permanent pushed below; battlefield scans skip it by uid
        // (its per-cast engine already fired for the casts so far).
        let cast_perm_uid = super::game::take_uid(st);
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
            // Life the goldfish pays itself is not damage dealt; keep it
            // out of the lethal census.
            st.life_paid += card.additional_cost_life;
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
            uid: cast_perm_uid,
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
        cast_ets.push((cast_perm_uid, idx));
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
        // paid 1; the rest of the pool converts. Counters cards skip
        // this branch: the entry-counter block above already converted
        // the pool to counters and recorded the spend.
        if let Some(class) = card.x_class
            && class != super::model::XClass::Counters
        {
            let x = pool.total().max(1);
            pool.colorless = 0;
            pool.flexible = 0;
            pool.fixed = [0; 5];
            pool.creature_only = 0;
            // Spent accounting is single-counted: the entered X counters
            // ARE the paid X; `spent_total` records it once here.
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
                super::model::XClass::RevealPermanents => {
                    // Best case the top X cards all become permanents on
                    // the battlefield (capped at 8). They enter through a
                    // reveal effect, not a cast resolution: their
                    // OnEnter triggers do not fire (the ETB pass below
                    // excludes non-cast entries).
                    for _ in 0..x.min(8) {
                        if let Some(i) = st.library.pop() {
                            st.battlefield_seen.entry(i).or_insert(turn as u32);
                            let uid = take_uid(st);
                            st.battlefield
                                .push(new_perm_with(uid, deck, i, turn as u32, false));
                        }
                    }
                }
                _ => {}
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
        for p in st.battlefield.iter() {
            if p.uid == cast_perm_uid {
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
        // Cascade: one free cast of the cheapest cheaper castable card
        // from the library. Single level, no cascade chaining. The free
        // cast counts fully: ETB triggers fire, per-cast engines fire,
        // and the card leaves the library into the seen census.
        if card.has_cascade
            && let Some(cascade_pos) = st
                .library
                .iter()
                .enumerate()
                .filter(|(_, i)| {
                    let free = &deck.cards[**i];
                    free.role != Role::Land
                        && free.min_cost.total() < card.min_cost.total()
                        && !free.has_cascade
                })
                // The cheapest match wins; a tie keeps the deeper
                // library position (last found).
                .max_by_key(|(pos, i)| (std::cmp::Reverse(deck.cards[**i].min_cost.total()), *pos))
                .map(|(pos, _)| pos)
        {
            let free_idx = st.library.remove(cascade_pos);
            let free_card = &deck.cards[free_idx];
            st.seen += 1;
            st.awareness_cards += 1;
            if free_card.is_creature {
                st.battlefield_seen.entry(free_idx).or_insert(turn as u32);
                let uid = take_uid(st);
                st.battlefield.push(InPlay {
                    uid,
                    card: free_idx,
                    tapped: false,
                    sick: true,
                    counters: free_card.enter_counters,
                    animated: false,
                    crewed: false,
                    entered_turn: turn,
                    saga_step: 0,
                    fired: false,
                    blink_pending: false,
                    loyalty: free_card.starting_loyalty.unwrap_or(0),
                    equipped: false,
                    equip_host: None,
                    is_commander: false,
                    commander_slot: 0,
                });
            }
            // Per-cast engines fire for the free cast (the cheapest path
            // applies the same credit the real cast would).
            if let Some(y) = &free_card.mana_per_cast {
                for _ in 0..st.prowess_casts {
                    add_yield_turns_empty_board(y, pool, turn as u32);
                }
            }
            if let Some(y) = &free_card.mana_on_cast {
                add_yield_turns_empty_board(y, pool, turn as u32);
            }
            if free_card.drain_on_cast > 0 {
                st.drained += free_card.drain_on_cast * drain_mult_in(deck);
            }
            for _ in 0..free_card.draws_on_cast {
                if let Some(i) = st.library.pop() {
                    st.hand.push(i);
                    st.seen += 1;
                    st.awareness_cards += 1;
                }
            }
            if free_card.tokens_on_cast > 0 {
                apply_effect_at(
                    deck,
                    &Effect::Tokens(free_card.tokens_on_cast.min(8)),
                    st,
                    turn as u32,
                    false,
                    Some(free_idx),
                );
            }
            st.prowess_casts += 1;
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
    // their upkeep engines register for the next turn. Entries resolve
    // through stable uids: lands played this turn already fired via
    // `play_land` and reveal-effect pushes (tokens, returned bodies)
    // never had a cast, so both are excluded; token pushes during a
    // fire cannot shift another entry's identity.
    let cast_uids: Vec<u32> = cast_ets.iter().map(|e| e.0).collect();
    let newly_cast: Vec<(usize, u32, usize)> = st
        .battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.entered_turn == turn
                && !p.is_commander
                && p.card < usize::MAX - 1
                && cast_uids.contains(&p.uid)
        })
        .map(|(pos, p)| (pos, p.uid, p.card))
        .collect();
    for (_, uid, card_idx) in &newly_cast {
        // Re-resolve the position at fire time: earlier fires can push
        // tokens and shift indices.
        let Some(pos) = st.battlefield.iter().position(|p| p.uid == *uid) else {
            continue;
        };
        fire_on_enter(deck, st, pos, turn as u32);
        // Upkeep engines on the cast card register now, by uid.
        for ability in deck.cards[*card_idx].abilities() {
            if let Some(draws) = engine_effect_or_draws(&ability.trigger, &ability.effect) {
                engines.push((*uid, draws));
            }
        }
    }
}

/// Upkeep-engine draws for an ability: the amount for Draw engines, 0
/// for the executor-run shapes, None when not an upkeep engine.
fn engine_effect_or_draws(trigger: &Trigger, effect: &Effect) -> Option<u32> {
    match (trigger, effect) {
        (Trigger::OnUpkeep, Effect::Draw(n)) => Some(*n),
        (
            Trigger::OnUpkeep,
            Effect::Mill(_)
            | Effect::ReturnFromGraveyard { .. }
            | Effect::Drain(_)
            | Effect::Tokens(_),
        ) => Some(0),
        _ => None,
    }
}
