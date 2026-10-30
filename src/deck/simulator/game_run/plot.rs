//! Plot payments and free casts on later turns.

use super::super::cast_pass::cast_resolve::free_cast_payable;
use super::super::cast_pass::{CastPayment, resolve_cast};
use super::super::game::{GameState, ManaPool};
use super::super::game_effects::prepare_spell_targets;
use super::super::game_mana::{cast_cost, pay_cost, payable, pips_ok};
use super::super::model::{CardIdx, Cost, SimDeck};

/// Plot an unaffordable hand card for a free cast on a later turn (CR 702.170).
pub(super) fn plot_unaffordable_cards(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
) {
    if st.completion.is_some() {
        return;
    }
    let candidates: Vec<CardIdx> = st
        .hand
        .iter()
        .copied()
        .filter(|idx| {
            let card = &deck[*idx];
            card.keyword_abilities.plot.is_some()
                && !payable(&cast_cost(deck, st, card, card.min_cost, Some(*idx)), pool)
                && card
                    .keyword_abilities
                    .plot
                    .as_ref()
                    .is_some_and(|plot| payable(plot, pool) && pips_ok(plot, pool))
        })
        .collect();
    for idx in candidates {
        let Some(plot) = deck[idx].keyword_abilities.plot else {
            continue;
        };
        if !payable(&plot, pool) || !pips_ok(&plot, pool) {
            continue;
        }
        pay_cost(&plot, pool);
        if let Some(pos) = st.hand.iter().position(|held| *held == idx) {
            st.hand.remove(pos);
        }
        st.plotted.push((idx, turn as u32));
    }
}

/// Cast earlier plotted cards until one finishes the game (CR 702.170d).
pub(super) fn cast_plotted_cards(
    deck: &SimDeck,
    st: &mut GameState,
    pool: &mut ManaPool,
    turn: usize,
    mana_spent: &mut [f64],
    repeatable_sources: &mut Vec<(u32, u32)>,
) {
    let ready: Vec<CardIdx> = st
        .plotted
        .iter()
        .filter(|(_, plotted_turn)| (*plotted_turn as usize) < turn)
        .map(|(idx, _)| *idx)
        .collect();
    for idx in ready {
        if st.completion.is_some() {
            return;
        }
        if !free_cast_payable(deck, st, pool, idx) {
            continue;
        }
        let Some(targets) = prepare_spell_targets(deck, st, &deck[idx]) else {
            continue;
        };
        st.plotted.retain(|(plotted, _)| *plotted != idx);
        let mut spent = 0;
        resolve_cast(
            deck,
            st,
            pool,
            turn,
            idx,
            &Cost::default(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut spent,
            repeatable_sources,
            true,
            CastPayment::Free,
            targets,
        );
        mana_spent[turn - 1] += f64::from(spent);
    }
}

/// Plot boundaries and native mana-tap bonuses through production actions.
#[cfg(test)]
#[path = "../tests/plot_mana_boundary_tests.rs"]
mod plot_mana_boundary_tests;

/// Additional Behold mana remains payable on a later free plotted cast.
#[cfg(test)]
#[path = "tests/behold_plot_tests.rs"]
mod behold_plot_tests;
