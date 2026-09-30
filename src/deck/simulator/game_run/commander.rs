//! The commander cast phase for the goldfish turn loop. Split from
//! `game_run` to keep files small.

use super::super::game::{
    GameState, ManaPool, fire_on_enter, register_loyalty_token_source, take_uid,
};
use super::super::game_commander::CommanderProfile;
use super::super::game_mana::{
    bucket_of, cast_restrictions, effective_min_cost, pay_cost, pay_restricted_cost, payable,
    phyrexian_life_charge, pips_ok, usable_for_classes, usable_for_noncreature,
};
use super::super::model::{Role, SimDeck};
use super::TurnCensus;

/// Commander cast: full pip check against the general pool, cost
/// deducted, joins the battlefield. Partner decks cast EACH payable commander
/// (each once per game); an unpayable commander is skipped and the next
/// one still gets its try. The log's cast turn is the first one.
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
    for (cmd_i, cmd) in deck.commanders.iter().enumerate() {
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
        let charge = phyrexian_life_charge(&cmd.cost);
        let mana_total = cmd.cost.total() - charge / 2;
        let classes = cast_restrictions(cmd);
        let mana_ok = if classes.is_empty() {
            usable_for_noncreature(pool) >= mana_total && pips_ok(&cmd.cost, pool)
        } else {
            let pip_total =
                cmd.cost.pips.iter().map(|p| u32::from(*p)).sum::<u32>() + cmd.cost.hybrid_pips;
            let bucket = classes.iter().map(|c| bucket_of(pool, *c)).sum::<u32>();
            usable_for_classes(pool, &classes) >= mana_total
                && (pips_ok(&cmd.cost, pool) || bucket + pool.flexible >= pip_total)
        };
        if !mana_ok || st.life < charge as i32 {
            // Skip this commander; the next partner still gets its try
            // this turn.
            continue;
        }
        // Deduct mirroring the gate.
        st.life -= charge as i32;
        st.life_paid += charge;
        if classes.is_empty() {
            pay_cost(&cmd.cost, pool);
        } else {
            pay_restricted_cost(&cmd.cost, pool, &classes);
        }
        census.mana_spent[turn - 1] += (cmd.cost.total() - charge / 2) as f64;
        if census.commander_castable.is_none() {
            census.commander_castable = Some(turn as u32);
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
            if card.role != Role::Land && census.mana_ready[idx].is_none() {
                let eff = effective_min_cost(deck, card, &st.battlefield);
                if payable(&eff, &pool_before_commander) && pips_ok(&eff, &pool_before_commander) {
                    census.mana_ready[idx] = Some(turn as u32);
                }
            }
        }
    }
}
