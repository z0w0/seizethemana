//! Pinned native mana actions keep tap, sickness, color, and source-use rules.

use super::{attempt_index, renamed};
use crate::db::CardRow;
use crate::deck::simulator::card_test_support::main_phase_state as state;
use crate::deck::simulator::game::{ManaPool, new_perm_with};
use crate::deck::simulator::game_run::mana::activate_mana_mode;
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::scenario_test_support::{deck, row};

/// Sol Ring pays a generic spell, never a colored pip, and cannot tap twice.
pub(in crate::deck::simulator) fn assert_sol_ring(source: &CardRow) {
    assert_native_source(source, "{2}", "{G}", 2);
}

/// Llanowar Elves pays green, not white, and obeys creature tap restrictions.
pub(in crate::deck::simulator) fn assert_llanowar(source: &CardRow) {
    assert_native_source(source, "{G}", "{W}", 1);
}

/// Birds of Paradise pays a colored pip but cannot produce explicit colorless mana.
pub(in crate::deck::simulator) fn assert_birds(source: &CardRow) {
    assert_native_source(source, "{U}", "{C}", 1);
}

/// Delighted Halfling's colorless mode runs while its unsupported second mode stays inert.
pub(in crate::deck::simulator) fn assert_halfling_colorless(source: &CardRow) {
    crate::deck::simulator::card_ast_tests::assert_halfling_modes(source);
    assert_native_source(source, "{C}", "{G}", 1);
    for source in renamed(source) {
        let cards = deck(&[
            source.clone(),
            row("Colored legendary spell", "{G}", "Legendary Creature", ""),
        ]);
        let mut game = state(vec![1], vec![]);
        let mut permanent = new_perm_with(7, &cards, CardIdx(0), 0, false);
        permanent.summoning_sick = false;
        game.battlefield.push(permanent.clone());
        let mut pool = ManaPool::default();
        assert!(activate_mana_mode(
            &cards, &mut game, &permanent, &mut pool, 1
        ));
        assert_eq!(pool.total(), 1);
        assert_eq!(pool.colorless, 1);
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
            None
        );
        assert_eq!(pool.total(), 1);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.life, 20);
    }
}

/// Exercise legal activation, a wrong payment class, repeated use, and source-state misses.
fn assert_native_source(source: &CardRow, payable: &str, rejected: &str, amount: u32) {
    for source in renamed(source) {
        let cards = deck(&[
            source.clone(),
            row("Payable spell", payable, "Sorcery", ""),
            row("Wrong payment class", rejected, "Sorcery", ""),
        ]);
        let mut game = state(vec![1, 2], vec![]);
        let mut permanent = new_perm_with(7, &cards, CardIdx(0), 0, false);
        permanent.summoning_sick = false;
        game.battlefield.push(permanent.clone());
        let mut pool = ManaPool::default();
        assert!(activate_mana_mode(
            &cards, &mut game, &permanent, &mut pool, 1
        ));
        assert_eq!(pool.total(), amount);
        assert!(game.battlefield[0].tapped);
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(2)),
            None
        );
        assert_eq!(pool.total(), amount);
        let tapped = game.battlefield[0].clone();
        assert!(!activate_mana_mode(
            &cards, &mut game, &tapped, &mut pool, 1
        ));
        assert_eq!(pool.total(), amount);
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
            Some(amount)
        );
        assert_eq!(pool.total(), 0);
        assert_eq!(game.hand, [CardIdx(2)]);
        assert_eq!(game.graveyard, [CardIdx(1)]);
        assert!(game.library.is_empty() && game.exile.is_empty());
        assert_eq!(game.life, 20);
        for (tapped, sick, hidden) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            if sick && !cards[CardIdx(0)].is_creature {
                continue;
            }
            let mut game = state(vec![1], vec![]);
            let mut permanent = new_perm_with(7, &cards, CardIdx(0), 0, tapped);
            permanent.summoning_sick = sick;
            permanent.face_down = hidden;
            game.battlefield.push(permanent.clone());
            let mut pool = ManaPool::default();
            assert!(!activate_mana_mode(
                &cards, &mut game, &permanent, &mut pool, 1
            ));
            assert_eq!(pool.total(), 0);
            assert_eq!(game.hand, [CardIdx(1)]);
            assert_eq!(game.battlefield[0].tapped, tapped);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = "{T}: Add {R}.".into();
    let cards = deck(&[changed]);
    let mut game = state(vec![], vec![]);
    let mut permanent = new_perm_with(7, &cards, CardIdx(0), 0, false);
    permanent.summoning_sick = false;
    game.battlefield.push(permanent.clone());
    let mut pool = ManaPool::default();
    assert!(activate_mana_mode(
        &cards, &mut game, &permanent, &mut pool, 1
    ));
    assert_eq!(pool.fixed, [0, 0, 0, 1, 0]);
    assert_eq!(pool.total(), 1);
}

/// Fellwar Stone uses only a declared active seat's land colors.
pub(in crate::deck::simulator) fn assert_fellwar(source: &CardRow) {
    for source in renamed(source) {
        for (active, colors, lands, legal) in [
            (true, "U", 1, true),
            (true, "U", 0, false),
            (true, "", 1, false),
            (false, "U", 1, false),
        ] {
            let cards = deck(&[
                source.clone(),
                row("Blue payment", "{U}", "Sorcery", "Gain 1 life."),
            ]);
            let mut game = state(vec![1], vec![]);
            game.opponents.config.land_colors = Some(vec![colors.into()]);
            game.opponents.seats[0].active = active;
            game.opponents.seats[0].lands = lands;
            let permanent = new_perm_with(7, &cards, CardIdx(0), 0, false);
            game.battlefield.push(permanent.clone());
            let mut pool = ManaPool::default();
            assert_eq!(
                activate_mana_mode(&cards, &mut game, &permanent, &mut pool, 1),
                legal
            );
            assert_eq!(pool.total(), u32::from(legal));
            assert_eq!(game.battlefield[0].tapped, legal);
            if legal {
                assert!(!crate::deck::simulator::game_mana::payable(
                    &parse_cost("{G}"),
                    &pool
                ));
                assert!(!crate::deck::simulator::game_mana::payable(
                    &parse_cost("{C}"),
                    &pool
                ));
                assert_eq!(
                    attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
                    Some(1)
                );
                assert_eq!(game.graveyard, [CardIdx(1)]);
            } else {
                assert_eq!(
                    attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
                    None
                );
                assert_eq!(game.hand, [CardIdx(1)]);
                assert!(game.graveyard.is_empty());
            }
            assert_eq!(pool.total(), 0);
            assert_eq!(game.life, 20 + i32::from(legal));
            assert_eq!(game.life_gained, u32::from(legal));
            assert_eq!(game.spells_cast_this_turn, u32::from(legal));
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = "{T}: Add {R}.".into();
    let cards = deck(&[changed]);
    let mut game = state(vec![], vec![]);
    let permanent = new_perm_with(7, &cards, CardIdx(0), 0, false);
    game.battlefield.push(permanent.clone());
    let mut pool = ManaPool::default();
    assert!(activate_mana_mode(
        &cards, &mut game, &permanent, &mut pool, 1
    ));
    assert_eq!(pool.fixed, [0, 0, 0, 1, 0]);
    assert_eq!(pool.total(), 1);
}
