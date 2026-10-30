//! Commander casting and mana-readiness observations for the goldfish turn loop.

use super::super::cast_pass::cast_resolve::spell_mana_ready;
use super::super::game::{
    GameState, ManaPool, fire_on_enter, register_loyalty_token_source, take_uid,
};
use super::super::game_commander::CommanderProfile;
use super::super::game_effects::spell_targets_available;
use super::super::game_mana::{
    cast_restrictions, effective_cast_cost, payment_plan, phyrexian_life_charge,
};
use super::super::model::{Role, SimDeck};
use super::TurnCensus;

/// Commander cast: full pip check against the general pool, cost
/// deducted, joins the battlefield. Each commander pays its own recast tax.
/// An unpayable commander is skipped so a partner can still be cast.
#[allow(clippy::too_many_arguments)]
pub(in crate::deck::simulator) fn commander_phase(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    census: &mut TurnCensus,
    commander: &CommanderProfile,
    turn: usize,
    repeatable_sources: &mut Vec<(u32, u32)>,
    record_readiness: bool,
) {
    // Snapshot the pool first: the mana-readiness check below must see
    // the pre-cast pool, or a same-turn commander cast pushes every
    // other card's readiness a turn later.
    let pool_before_commander = pool.clone();
    st.commander_casts.resize(deck.commanders.len(), 0);
    for (cmd_i, cmd) in deck.commanders.iter().enumerate() {
        if st.completion.is_some() {
            return;
        }
        if !cmd.has_mana_cost
            || cmd.spell_data.unsupported_additional_cost
            || !spell_targets_available(deck, st, cmd)
            // Command-zone casts do not yet share the additional-cost planner.
            || cmd.spell_data.additional_cost_creatures > 0
            || cmd.spell_data.additional_cost_discards > 0
            || cmd.spell_data.additional_cost_life > 0
        {
            continue;
        }
        if st.battlefield.iter().any(
            |p| matches!(p.card, super::super::game::CardRef::Commander { slot } if slot == cmd_i),
        ) {
            continue;
        }
        // Gate on the general pool plus every matching restricted
        // bucket: creature/legendary/artifact/instant-sorcery mana pays
        // only its own cast class, so unrelated buckets must not count
        // (or the cast would spend mana never deducted). Phyrexian pips
        // pay with 2 life each (CR 107.4f), so the pool owes only the
        // mana part and life must cover the charge.
        let cost = effective_cast_cost(
            deck,
            st,
            cmd,
            cmd.min_cost,
            st.commander_casts[cmd_i].saturating_mul(2),
            None,
        );
        let charge = phyrexian_life_charge(&cost);
        let mana_total = cost.total() - charge / 2;
        let classes = cast_restrictions(cmd);
        if st.life < charge as i32 {
            // Skip this commander; the next partner still gets its try
            // this turn.
            continue;
        }
        let Some(payment) = payment_plan(&cost, pool, &classes) else {
            continue;
        };
        census.commander_castable.get_or_insert(turn as u32);
        assert!(
            payment.commit(pool),
            "commander payment commits against the prepared pool"
        );
        super::super::resolution::begin_resolution(st);
        st.life -= charge as i32;
        st.life_paid += charge;
        census.mana_spent[turn - 1] += mana_total as f64;
        st.commander_casts[cmd_i] += 1;
        st.spells_cast_this_turn += 1;
        super::super::resolution::end_resolution(deck, st, Some(pool));
        if st.completion.is_some() {
            return;
        }
        super::super::resolution::begin_resolution(st);
        super::super::cast_pass::fire_cast_triggers(deck, st, pool, turn as u32, cmd);
        if !cmd.is_creature {
            super::super::game::record_noncreature_cast(deck, st);
        }
        super::super::resolution::end_resolution(deck, st, Some(pool));
        if st.completion.is_some() {
            return;
        }
        // The commander is a permanent with no library entry.
        let cmd_uid = take_uid(st);
        st.battlefield.push(super::super::game::new_commander_perm(
            cmd_uid,
            cmd_i,
            cmd.starting_loyalty.unwrap_or(0),
            turn,
            cmd.is_creature,
            cmd.flags.has_haste,
        ));
        if commander.station_at.is_none() {
            // Not a station card: online the moment it is cast.
            census.station_online = Some(turn as u32);
        }
        if commander.upkeep_draw_amounts[cmd_i].is_some_and(|n| n > 0)
            || commander.has_other_upkeep_effect[cmd_i]
        {
            super::super::game::register_repeatable_source(
                repeatable_sources,
                cmd_uid,
                commander.upkeep_draw_amounts[cmd_i].unwrap_or(0),
            );
        }
        // Planeswalker +1 token engines register at first cast: a
        // loyalty-gain activation that creates tokens is a repeatable
        // once-per-turn engine (Liliana-class token fuel).
        let cmd_pw_pos = st.battlefield.len() - 1;
        register_loyalty_token_source(deck, st, cmd_pw_pos, repeatable_sources);
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
            if card.role != Role::Land
                && census.mana_ready[idx].is_none()
                && spell_mana_ready(
                    deck,
                    st,
                    &pool_before_commander,
                    super::super::model::CardIdx(idx as u32),
                )
            {
                census.mana_ready[idx] = Some(turn as u32);
            }
        }
    }
}

/// Commander cost and free plotted-cast regressions.
#[cfg(test)]
#[path = "tests/commander_cast_tests.rs"]
mod commander_cast_tests;
