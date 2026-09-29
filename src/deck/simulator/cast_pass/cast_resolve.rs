//! Cast resolution for one affordable spell: cost payment (convoke,
//! delve, phyrexian), spell data, ETB counters, X conversion, storm copies,
//! and offspring. Split from `cast_pass` to keep files small.

use super::super::game::{
    GameState, ManaPool, card_of, new_perm_with, register_loyalty_token_source, take_uid,
};
use super::super::game_effects::{
    CardZone, apply_effect_at, draw_one, mill_library_card, move_to_graveyard,
};
use super::super::game_mana::{
    add_yield_turns_unscaled, bucket_of, cast_restrictions, pay_cost, pay_restricted_cost,
    phyrexian_life_charge, pips_ok, usable_for_noncreature,
};
use super::super::model::{CardIdx, Cost, Role, SimDeck, SimEffect, SimTrigger};
use super::{resolve_reveal_rule, select_alternative_cost_cards};

/// Resolve one affordable cast: pay the cost, push the permanent, and
/// fire every on-cast rider (kicker, additional costs, ETB counters,
/// rituals, draws, mills, scry, wheels, X conversion, life_loss, tokens,
/// cascade). Pure aside from the game state it mutates.
#[allow(clippy::too_many_arguments)]
pub(in crate::deck::simulator) fn resolve_cast(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    idx: CardIdx,
    eff: &Cost,
    cast_ids: &mut Vec<CardIdx>,
    cast_ets: &mut Vec<(u32, CardIdx)>,
    spent_total: &mut u32,
    repeatable_sources: &mut Vec<(u32, u32)>,
    resolve_cascade: bool,
) {
    let card = &deck[idx];
    let alternative_cards = select_alternative_cost_cards(deck, st, idx);
    if alternative_cards.is_empty() {
        // Phyrexian pips pay with 2 life each (CR 107.4f): the
        // best-case agent preserves mana.
        let charge = phyrexian_life_charge(eff);
        st.life -= charge as i32;
        st.life_paid += charge;
        // Convoke (CR 702.51) and delve (CR 702.66) are payment options
        // that reduce what the mana pool owes. Both consume real
        // resources first: bodies tap, graveyard cards exile.
        let owed = pay_convoke_and_delve(deck, st, card, eff);
        let classes = cast_restrictions(card);
        if classes.is_empty() {
            pay_cost(&owed, pool);
        } else {
            pay_restricted_cost(&owed, pool, &classes);
        }
        *spent_total += eff.total() - charge / 2;
    }
    cast_ids.push(idx);
    if let Some(pos) = st.hand.iter().position(|held| *held == idx) {
        st.hand.remove(pos);
    }
    // Kicker: an optional extra cost paid from leftover mana. Best
    // case the goldfish kicks when the pool covers it (the colored
    // pips are paid from fixed and flexible sources like any cost).
    // Phyrexian kicker pips pay with 2 life each (CR 107.4f), so the
    // pool owes only their mana part. The rider bumps the drain/damage
    // amount (the modeled kicker payoff).
    let kicker_charge = card
        .spell_data
        .kicker
        .as_ref()
        .map_or(0, phyrexian_life_charge);
    let kicked = if let Some(k) = card.spell_data.kicker
        && usable_for_noncreature(pool) >= k.total() - kicker_charge / 2
        && pips_ok(&k, pool)
        && st.life > kicker_charge as i32
    {
        pay_cost(&k, pool);
        st.life -= kicker_charge as i32;
        st.life_paid += kicker_charge;
        *spent_total += k.total() - kicker_charge / 2;
        true
    } else {
        false
    };
    // The permanent pushed below; battlefield scans skip it by uid
    // (its per-cast engine already fired for the casts so far).
    let cast_perm_uid = super::super::game::take_uid(st);
    // Additional discard, sacrifice, and life costs passed the cast gate
    // before mana was paid. The cast card already left the hand, so wheels
    // cannot discard the resolving spell.
    for _ in 0..card.spell_data.additional_cost_discards {
        if let Some(pos) = st.hand.iter().position(|i| *i != idx) {
            let discarded = st.hand.remove(pos);
            move_to_graveyard(deck, st, discarded, turn as u32, CardZone::Hand);
        }
    }
    let mut alternative_life = 0;
    for exiled in alternative_cards {
        if let Some(position) = st.hand.iter().position(|held| *held == exiled) {
            st.hand.remove(position);
            st.exile.push(exiled);
            alternative_life += deck[exiled].mana_value;
        }
    }
    if alternative_life > 0
        && card.spell_data.alternative_cast_cost.is_some_and(|cost| {
            cost.payoff
                == super::super::model::AlternativeCostPayoff::GainLifeEqualToExiledManaValue
        })
    {
        st.life += alternative_life as i32;
        st.life_gained += alternative_life;
    }
    let searched_sacrifice = if card.spell_data.search_after_sacrifice {
        st.battlefield
            .iter()
            .filter(|perm| perm.card.deck_idx().is_some() && card_of(deck, perm).is_creature)
            .find_map(|perm| {
                let mana_value = card_of(deck, perm).mana_value;
                st.library
                    .iter()
                    .map(|index| &deck[*index])
                    .any(|candidate| {
                        candidate.is_creature && candidate.mana_value == mana_value + 1
                    })
                    .then_some((perm.uid, mana_value))
            })
    } else {
        None
    };
    for _ in 0..card.spell_data.additional_cost_creatures {
        if let Some((uid, _)) = searched_sacrifice {
            super::super::game_effects::resolve_sacrifice_uid(deck, st, turn as u32, uid);
        } else {
            super::super::game_effects::resolve_deck_creature_sacrifice(
                deck,
                st,
                turn as u32,
                u32::MAX,
            );
        }
    }
    if let Some((_, sacrificed_mv)) = searched_sacrifice
        && let Some(pos) = st.library.iter().rposition(|candidate| {
            deck[*candidate].is_creature && deck[*candidate].mana_value == sacrificed_mv + 1
        })
    {
        let target = st.library.remove(pos);
        let target_card = &deck[target];
        let uid = super::super::game::take_uid(st);
        st.battlefield_seen.entry(target).or_insert(turn as u32);
        let mut entry = new_perm_with(uid, deck, target, turn as u32, false);
        entry.summoning_sick = !target_card.flags.has_haste;
        entry.counters = target_card.enter_counters.fixed_with_plus1(1);
        st.battlefield.push(entry);
        cast_ets.push((uid, target));
    }
    if card.spell_data.additional_cost_life > 0 {
        // Life the goldfish pays itself is not damage dealt; keep it
        // out of the lethal census.
        st.life_paid += card.spell_data.additional_cost_life;
        st.life -= card.spell_data.additional_cost_life as i32;
        st.life_funded_draws += card.spell_data.draws_on_cast;
        super::super::game::milestone_for_turn(st, turn as u32).life_funded_draws +=
            card.spell_data.draws_on_cast;
    }
    if card.spell_data.grants_flashback {
        st.flashback_permissions.extend(
            st.graveyard
                .iter()
                .copied()
                .filter(|index| deck[*index].is_instant_or_sorcery),
        );
    }
    if card.spell_data.graveyard_creature_exchange {
        let mut returned = Vec::new();
        let mut remaining = Vec::new();
        for index in st.graveyard.drain(..) {
            if deck[index].is_creature {
                st.exile.push(index);
                returned.push(index);
            } else {
                remaining.push(index);
            }
        }
        st.graveyard = remaining;
        // Death triggers on the surviving board can create new creature
        // tokens each round, so the exchange can never drain the board
        // by itself. The shared pass cap keeps the cast phase finite.
        for _ in 0..super::super::model::MAX_LOOP_PASSES {
            if !super::super::game_effects::has_sacrificable_creature(deck, st) {
                break;
            }
            super::super::game_effects::resolve_creature_sacrifice(deck, st, turn as u32, u32::MAX);
        }
        for index in returned {
            let returned_card = &deck[index];
            st.battlefield_seen.entry(index).or_insert(turn as u32);
            let uid = super::super::game::take_uid(st);
            let mut entry = new_perm_with(uid, deck, index, turn as u32, false);
            entry.summoning_sick = !returned_card.flags.has_haste;
            entry.counters = returned_card.enter_counters.fixed();
            st.battlefield.push(entry);
            cast_ets.push((uid, index));
        }
    }
    // Producers join the battlefield: rocks tap at once, creatures
    // from next turn (summoning sickness). Vehicles and spacecraft
    // join as artifacts. "Enters with X charge counters" cards
    // convert the cast's leftover pool into counters (Astral
    // Cornucopia class).
    let entry_counters = if card.enter_counters.is_x() {
        let x = convert_pool_to_x(card, pool);
        *spent_total += x;
        card.enter_counters.with_paid_x(x)
    } else {
        card.enter_counters.fixed()
    };
    if !card.is_instant_or_sorcery {
        st.battlefield_seen.entry(idx).or_insert(turn as u32);
        let mut entry = new_perm_with(cast_perm_uid, deck, idx, turn as u32, false);
        entry.summoning_sick =
            (card.is_creature || card.keyword_abilities.living_metal) && !card.flags.has_haste;
        entry.counters = entry_counters;
        st.battlefield.push(entry);
        let pw_pos = st.battlefield.len() - 1;
        register_loyalty_token_source(deck, st, pw_pos, repeatable_sources);
        cast_ets.push((cast_perm_uid, idx));
    }
    // Offspring (CR 702.175): when the permanent enters and the
    // offspring cost was paid, create a 1/1 token copy. The goldfish
    // pays the extra cost from the leftover pool when it can afford it.
    if let Some(offspring) = card.keyword_abilities.offspring
        && !card.is_instant_or_sorcery
        && usable_for_noncreature(pool) >= offspring.total()
        && pips_ok(&offspring, pool)
    {
        pay_cost(&offspring, pool);
        *spent_total += offspring.total();
        let uid = super::super::game::take_uid(st);
        let mut copy = super::super::game::new_token_perm(uid, turn as u32);
        // The copy is 1/1, so a plain token body stands in for it.
        copy.counters = super::super::model::Counters::default();
        st.battlefield.push(copy);
    }
    // Teamwork (CR 702.194): an optional additional cost; paying taps
    // bodies with total power N. The goldfish pays it when the bodies
    // are free and the rider (second mode) is modeled as part of the
    // spell's effects.
    if let Some(teamwork) = card.keyword_abilities.teamwork {
        let mut tapped_power = 0u32;
        for perm in st.battlefield.iter_mut() {
            if tapped_power >= teamwork {
                break;
            }
            if !perm.tapped && perm.card.deck_idx().is_some() {
                let power = card_of(deck, perm)
                    .printed_power
                    .unwrap_or(super::super::game::TOKEN_CREATURE_POWER);
                if super::super::game::is_creature_permanent(deck, perm) {
                    perm.tapped = true;
                    tapped_power += power;
                }
            }
        }
    }
    // Planeswalker +1 token engines register at first cast: a
    // loyalty-gain activation that creates tokens is a repeatable
    // once-per-turn engine (Liliana-class token fuel).
    // One-shot mana (rituals) joins this turn's pool only.
    if let Some(y) = &card.spell_data.mana_on_cast {
        add_yield_turns_unscaled(y, pool, turn as u32);
    }
    // One-shot draws on cast (cantrips, Divination).
    for _ in 0..card.spell_data.draws_on_cast {
        draw_one(deck, st, turn as u32);
    }
    // One-shot mill on cast (plain "mill N" spells).
    for _ in 0..card.spell_data.mills_on_enter {
        mill_library_card(deck, st, turn as u32, card.mills_opponent);
    }
    // One-shot energy on cast.
    st.player_counters.energy += card.spell_data.energy_on_cast;
    // Keyword-action spell data resolve with the spell (CR 701): amass,
    // empower Jace, the Ring tempts you, explore, connive.
    if card.spell_data.amass_on_cast > 0 {
        super::super::game_effects::amass(deck, st, turn as u32, card.spell_data.amass_on_cast);
    }
    if card.spell_data.empower_jace_on_cast > 0 {
        super::super::game_effects::empower_jace(
            deck,
            st,
            turn as u32,
            card.spell_data.empower_jace_on_cast,
        );
    }
    if card.spell_data.tempts_ring_on_cast {
        super::super::game_effects::ring_tempts(deck, st, turn as u32);
    }
    for _ in 0..card.spell_data.explores_on_cast {
        super::super::game_effects::explore(deck, st, turn as u32);
    }
    if card.spell_data.connives_on_cast > 0 {
        super::super::game_effects::connive(
            deck,
            st,
            turn as u32,
            card.spell_data.connives_on_cast,
        );
    }
    // Scry/surveil on cast: awareness only; surveil mills the
    // scry'd cards to the graveyard.
    st.awareness_cards += card.spell_data.scry_on_cast;
    for _ in 0..card.spell_data.surveils_on_cast {
        mill_library_card(deck, st, turn as u32, false);
    }
    // Schedule the next turn slot as an extra turn.
    if card.spell_data.extra_turns_on_cast {
        st.extra_turns_queued += 1;
    }
    st.extra_land_drops_this_turn += card.spell_data.extra_land_drops_on_cast;
    // One-shot drain spells (burn at a player, "each opponent
    // loses N life"). The drain's scope sets the table multiplier
    // (CR 119.3); target-player burn resolves ×1, each-opponent burn
    // ×opponents, each-player burn ×opponents plus the goldfish.
    // Creature-target burn never got here (Removal). A paid kicker
    // bumps the drain amount.
    if card.spell_data.life_loss_on_resolve > 0 {
        let rider = card.spell_data.life_loss_on_resolve
            + u32::from(kicked) * card.spell_data.life_loss_on_resolve.max(1);
        st.opponent_life_lost += rider
            * card
                .spell_data
                .life_loss_scope
                .table_multiplier(deck.format);
        if card.spell_data.life_loss_scope == super::super::model::LifeLossScope::EachPlayer {
            st.life -= rider as i32;
        }
    }
    if card.spell_data.damage_on_resolve > 0 {
        let damage = card.spell_data.damage_on_resolve
            + u32::from(kicked) * card.spell_data.damage_on_resolve.max(1);
        record_player_damage(st, deck.format, damage, card.spell_data.damage_scope);
    }
    if card.spell_data.life_gain_on_cast > 0 {
        st.life += card.spell_data.life_gain_on_cast as i32;
        st.life_gained += card.spell_data.life_gain_on_cast;
    }
    if let Some(rule) = card.spell_data.reveal_rule {
        resolve_reveal_rule(deck, st, rule, turn as u32);
    }
    // One-shot token spells ("Create four 1/1 Soldier creature
    // tokens"): the cast resolves the creation.
    if card.spell_data.tokens_on_cast > 0 {
        apply_effect_at(
            deck,
            &SimEffect::Tokens(card.spell_data.tokens_on_cast.min(8)),
            st,
            turn as u32,
            false,
            Some(idx),
        );
    }
    if card.spell_data.treasures_on_cast > 0 {
        apply_effect_at(
            deck,
            &SimEffect::Treasures(card.spell_data.treasures_on_cast.min(8)),
            st,
            turn as u32,
            false,
            Some(idx),
        );
    }
    // One-shot wheel spells ("each player discards, then draws").
    // The just-cast wheel is still in hand (removal is deferred), so
    // the skip variant keeps it out of the graveyard log.
    if card.spell_data.discard_hand_then_draw_seven_on_cast {
        super::super::game_effects::resolve_discard_hand_then_draw_seven(
            deck,
            st,
            super::super::game_effects::CastCardToSkip { skip: idx },
            turn as u32,
        );
    }
    // X-cost spells pay the leftover pool as X and scale the effect
    // (best case: X = everything floatable). The generic {X} already
    // paid 1; the rest of the pool converts. Counters cards skip
    // this branch: the entry-counter block above already converted
    // the pool to counters and recorded the spend. The X value is
    // capped at 8 for token counts; the recorded spend still shows
    // the full floatable pool, so X-heavy decks read as near-zero
    // unused mana by design.
    if let Some(class) = card.spell_data.x_class
        && class != super::super::model::XClass::Counters
    {
        let x = convert_pool_to_x(card, pool).max(1);
        // Spent accounting is single-counted: the entered X counters
        // ARE the paid X; `spent_total` records it once here.
        *spent_total += x;
        match class {
            super::super::model::XClass::Drain => {
                st.opponent_life_lost += x * card
                    .spell_data
                    .life_loss_scope
                    .table_multiplier(deck.format);
                if card.spell_data.life_loss_scope == super::super::model::LifeLossScope::EachPlayer
                {
                    st.life -= x as i32;
                }
            }
            super::super::model::XClass::Draw => {
                for _ in 0..x {
                    draw_one(deck, st, turn as u32);
                }
            }
            super::super::model::XClass::Mill => {
                let mill_opp = card.mills_opponent;
                for _ in 0..x {
                    mill_library_card(deck, st, turn as u32, mill_opp);
                }
            }
            super::super::model::XClass::Tokens => {
                apply_effect_at(
                    deck,
                    &SimEffect::Tokens(x.min(8)),
                    st,
                    turn as u32,
                    false,
                    Some(idx),
                );
            }
            super::super::model::XClass::Treasures => {
                apply_effect_at(
                    deck,
                    &SimEffect::Treasures(x.min(8)),
                    st,
                    turn as u32,
                    false,
                    Some(idx),
                );
            }
            super::super::model::XClass::RevealPermanents => {
                // Best case the top X cards all become permanents on
                // the battlefield (capped at 8). They enter through a
                // reveal effect, not a cast resolution: their
                // Enters triggers do not fire (the ETB pass below
                // excludes non-cast entries).
                for _ in 0..x.min(8) {
                    if let Some(i) = st.library.pop() {
                        st.seen += 1;
                        st.awareness_cards += 1;
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
    // Storm (CR 702.40a): the spell is copied once for each other spell
    // cast before it this turn. The copies resolve the same modeled
    // spell data (draw, life_loss, tokens) that this cast does; the original's
    // cast already counted its own cast above, so the copy count reads
    // `spells_cast_this_turn` before this spell is counted.
    if card.keyword_abilities.storm {
        let copies = st.spells_cast_this_turn;
        for _ in 0..copies {
            for _ in 0..card.spell_data.draws_on_cast {
                draw_one(deck, st, turn as u32);
            }
            if card.spell_data.life_loss_on_resolve > 0 {
                st.opponent_life_lost += card.spell_data.life_loss_on_resolve
                    * card
                        .spell_data
                        .life_loss_scope
                        .table_multiplier(deck.format);
            }
            if card.spell_data.damage_on_resolve > 0 {
                record_player_damage(
                    st,
                    deck.format,
                    card.spell_data.damage_on_resolve,
                    card.spell_data.damage_scope,
                );
            }
            if card.spell_data.tokens_on_cast > 0 {
                apply_effect_at(
                    deck,
                    &SimEffect::Tokens(card.spell_data.tokens_on_cast.min(8)),
                    st,
                    turn as u32,
                    false,
                    Some(idx),
                );
            }
            if card.spell_data.treasures_on_cast > 0 {
                apply_effect_at(
                    deck,
                    &SimEffect::Treasures(card.spell_data.treasures_on_cast.min(8)),
                    st,
                    turn as u32,
                    false,
                    Some(idx),
                );
            }
        }
    }
    st.spells_cast_this_turn += 1;
    // Cast-count engines: "add {N} for each spell cast this turn"
    // joins the pool now (Vivi-class mana engines, best case). The
    // just-cast host fires for spells before it; hosts already on
    // the battlefield fire for every spell cast this turn.
    if let Some(y) = &card.spell_data.mana_per_cast {
        for _ in 0..st.prowess_casts {
            add_yield_turns_unscaled(y, pool, turn as u32);
        }
    }
    for p in st.battlefield.iter() {
        if p.uid == cast_perm_uid {
            continue;
        }
        if let Some(y) = &card_of(deck, p).spell_data.mana_per_cast {
            add_yield_turns_unscaled(y, pool, turn as u32);
        }
    }
    // SpellCast engines fire per spell cast. Draw engines draw;
    // loot fills the graveyard; per-cast drains resolve at the
    // family multiplier. Snapshot the uids: effects fired in the loop
    // (loot) can push or pull from the board.
    let trigger_source_uids: Vec<u32> = st.battlefield.iter().map(|p| p.uid).collect();
    for uid in trigger_source_uids {
        let Some(perm) = st.battlefield.iter().find(|p| p.uid == uid) else {
            continue;
        };
        let perm = perm.clone();
        for ability in super::super::game::permanent_abilities(deck, &perm) {
            if ability.trigger != SimTrigger::SpellCast {
                continue;
            }
            match ability.effect_sequence().first() {
                Some(SimEffect::Draw(n)) => {
                    for _ in 0..*n {
                        draw_one(deck, st, turn as u32);
                    }
                }
                Some(SimEffect::DrawThenDiscard(n)) => {
                    apply_effect_at(
                        deck,
                        &SimEffect::DrawThenDiscard(*n),
                        st,
                        turn as u32,
                        false,
                        None,
                    );
                }
                Some(SimEffect::LoseLife { amount, scope }) => {
                    st.opponent_life_lost += amount * scope.table_multiplier(deck.format);
                    if *scope == super::super::model::LifeLossScope::EachPlayer {
                        st.life -= *amount as i32;
                    }
                }
                Some(SimEffect::Damage { amount, scope }) => {
                    st.damage_dealt_this_turn += *amount * scope.table_multiplier(deck.format);
                    if *scope == super::super::model::LifeLossScope::EachPlayer {
                        st.life -= *amount as i32;
                    }
                }
                Some(SimEffect::Energy(n)) => {
                    st.player_counters.energy += n;
                }
                Some(SimEffect::Proliferate) => {
                    super::super::game_effects::resolve_proliferate(st);
                }
                _ => {}
            }
        }
    }
    // Counter injection targets the highest-threshold unfilled
    // station permanent (Drill Too Deep).
    if card.spell_data.counters_on_cast > 0
        && let Some(perm) = st
            .battlefield
            .iter_mut()
            .filter(|p| card_of(deck, p).is_station_card)
            .max_by_key(|p| card_of(deck, p).animate_at().unwrap_or(0))
    {
        perm.counters.charge += card.spell_data.counters_on_cast;
    }
    // Cascade: reveal in library order and free-cast the first nonland
    // card with lower printed mana value. No cascade chaining. The free
    // cast counts fully: ETB triggers fire, per-cast engines fire,
    // and the card leaves the library into the seen census.
    if resolve_cascade && card.has_cascade {
        let mut exposed = Vec::new();
        let mut hit = None;
        while let Some(index) = st.library.pop() {
            st.seen += 1;
            st.awareness_cards += 1;
            let candidate = &deck[index];
            if candidate.role != Role::Land && candidate.mana_value < card.mana_value {
                hit = Some(index);
                break;
            }
            exposed.push(index);
        }
        // The exposed order came from the seeded library shuffle (the
        // first miss was revealed first, closest to the hit). Reinsert
        // in reverse so the first-revealed card ends up deepest and the
        // reveal order survives at the library bottom.
        for index in exposed.iter().rev() {
            st.library.insert(0, *index);
        }
        if let Some(free_idx) = hit {
            if !deck[free_idx].is_instant_or_sorcery {
                super::super::game::milestone_for_turn(st, turn as u32)
                    .free_cast_permanents_entered += 1;
            }
            resolve_cast(
                deck,
                st,
                pool,
                turn,
                free_idx,
                &Cost::default(),
                cast_ids,
                cast_ets,
                spent_total,
                repeatable_sources,
                false,
            );
        }
    }
}

/// Record damage to the modeled player table and apply damage to the
/// simulated player for "each player" effects.
fn record_player_damage(
    st: &mut GameState,
    format: super::super::model::Format,
    amount: u32,
    scope: super::super::model::LifeLossScope,
) {
    st.damage_dealt_this_turn += amount * scope.table_multiplier(format);
    if scope == super::super::model::LifeLossScope::EachPlayer {
        st.life -= amount as i32;
    }
}

/// Pay convoke and delve, returning the cost the mana pool still owes.
///
/// Convoke (CR 702.51): each untapped creature taps to pay {1} of the
/// total cost, or a monocolor pip matching one of its printed colors.
/// Delve (CR 702.66): each graveyard card exiles to pay {1} generic.
/// Both are payment options; the remaining cost goes to the pool.
fn pay_convoke_and_delve(
    deck: &SimDeck,
    st: &mut GameState,
    card: &super::super::model::SimCard,
    eff: &Cost,
) -> Cost {
    let mut owed = *eff;
    // Delve first: it can only pay generic.
    if card.keyword_abilities.delve {
        let mut exiles = owed.generic.min(st.graveyard.len() as u32);
        while exiles > 0 {
            let Some(exiled) = st.graveyard.pop() else {
                break;
            };
            st.exile.push(exiled);
            owed.generic -= 1;
            exiles -= 1;
        }
    }
    if !card.keyword_abilities.convoke {
        return owed;
    }
    // Convoke: tap untapped creatures, each covering one colored pip of a
    // color it prints, else {1} generic.
    let mut creatures: Vec<usize> = st
        .battlefield
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            !p.tapped
                && p.card.deck_idx().is_some()
                && super::super::game::is_creature_permanent(deck, p)
        })
        .map(|(i, _)| i)
        .collect();
    // Prefer colored coverage so the pool keeps its pips.
    while !creatures.is_empty() {
        let Some(&bi) = creatures.first() else {
            break;
        };
        let colors = card_of(deck, &st.battlefield[bi]).colors;
        let color_pip = (0..5).find(|i| owed.pips[*i] > 0 && colors[*i]);
        if let Some(i) = color_pip {
            owed.pips[i] -= 1;
            st.battlefield[bi].tapped = true;
            creatures.remove(0);
        } else if owed.generic > 0 {
            owed.generic -= 1;
            st.battlefield[bi].tapped = true;
            creatures.remove(0);
        } else {
            break;
        }
    }
    owed
}

/// The X value an X-cost spell pays: the general pool plus every
/// restricted bucket whose class the spell belongs to (that mana can
/// legally fund the cast). The conversion then drains the whole pool:
/// the member buckets are spent into X, the unrelated buckets are
/// discarded, and nothing counts twice.
pub(in crate::deck::simulator) fn convert_pool_to_x(
    card: &super::super::model::SimCard,
    pool: &mut ManaPool,
) -> u32 {
    let x = {
        let classes = cast_restrictions(card);
        let general = usable_for_noncreature(pool);
        let buckets: u32 = classes.iter().map(|c| bucket_of(pool, *c)).sum();
        general + buckets
    };
    pool.colorless = 0;
    pool.flexible = 0;
    pool.fixed = [0; 5];
    pool.creature_only = 0;
    pool.legendary_only = 0;
    pool.artifact_only = 0;
    pool.instant_sorcery_only = 0;
    x
}

/// Return an upkeep trigger's draw amount when it has executable effects.
pub(in crate::deck::simulator) fn upkeep_trigger_registration(
    ability: &super::super::model::SimAbility,
) -> Option<u32> {
    if !ability.kind.is_triggered()
        || ability.trigger != SimTrigger::Upkeep
        || ability.effect_sequence().is_empty()
    {
        return None;
    }
    Some(
        ability
            .effect_sequence()
            .iter()
            .filter_map(|effect| match effect {
                SimEffect::Draw(amount) => Some(*amount),
                _ => None,
            })
            .sum(),
    )
}
