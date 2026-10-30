//! Read-only evidence that one more production action can execute after a work cap.

use super::super::game_effects::activation::activate_next_granted_mana;
use super::*;

/// Probe production paths on copied state without spending real resources or random values.
pub(super) fn remaining_main_work(
    deck: &SimDeck,
    st: &GameState,
    pool: &ManaPool,
    census: &TurnCensus,
    commander: &CommanderProfile,
    repeatable_sources: &[(u32, u32)],
    turn: usize,
) -> bool {
    let mut trial = st.clone();
    let mut trial_pool = pool.clone();
    let mut trial_census = census.clone();
    let mut sources = repeatable_sources.to_vec();
    trial_pool.treasures += std::mem::take(&mut trial.treasure_bank);
    play_late_land(deck, &mut trial, &mut trial_census, turn);
    cast_pass(
        deck,
        &mut trial,
        &mut trial_pool,
        turn,
        &mut trial_census.mana_spent,
        &mut sources,
        &mut trial_census.pip_blocks,
        &mut trial_census.blocked_colors,
    );
    if trial.spells_cast_this_turn != st.spells_cast_this_turn
        || trial.hand != st.hand
        || trial.battlefield.len() != st.battlefield.len()
        || trial.completion.is_some()
    {
        return true;
    }
    tap_dorks_for_mana(deck, &mut trial, &mut trial_pool, turn as u32);
    tap_new_rocks(deck, &mut trial, &mut trial_pool, turn as u32);
    if trial_pool.total() != pool.total() || trial.completion.is_some() {
        return true;
    }
    commander_phase(
        deck,
        &mut trial,
        &mut trial_pool,
        &mut trial_census,
        commander,
        turn,
        &mut sources,
        false,
    );
    trial.spells_cast_this_turn != st.spells_cast_this_turn
        || trial.completion.is_some()
        || activate_next_granted_mana(deck, &mut trial, &mut trial_pool, turn as u32)
        || crate::deck::simulator::game_effects::activation::pick_best_activation(
            deck,
            &trial,
            &trial_pool,
            turn as u32,
        )
        .is_some()
}

/// Remaining-work probes cannot spend physical or random resources.
#[cfg(test)]
#[path = "tests/work_tests.rs"]
mod work_tests;
