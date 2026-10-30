//! Pinned engines use production casts and the declared finite opponent schedule.

use super::{attempt_index, renamed};
use crate::db::CardRow;
use crate::deck::simulator::card_test_support::main_phase_state as state;
use crate::deck::simulator::game::{GameState, ManaPool, new_perm_with};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::opponents::{TaxPolicy, run_opponent_round};
use crate::deck::simulator::scenario_test_support::{deck, row};

/// Beast Whisperer observes an actual creature cast, not an unrelated noncreature cast.
pub(in crate::deck::simulator) fn assert_beast_whisperer(source: &CardRow) {
    crate::deck::simulator::card_ast_tests::assert_event_source(source);
    for source in renamed(source) {
        for (kind, matching) in [("Creature — Elf", true), ("Sorcery", false)] {
            let cards = deck(&[
                source.clone(),
                row("Cast object", "{0}", kind, ""),
                row("Drawn", "{99}", "Sorcery", ""),
            ]);
            let mut game = state(vec![1], vec![2]);
            game.battlefield
                .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
            assert_eq!(
                attempt_index(&cards, &mut game, &mut ManaPool::default(), CardIdx(1)),
                Some(0)
            );
            assert_eq!(game.hand, if matching { vec![CardIdx(2)] } else { vec![] });
            assert_eq!(
                game.library,
                if matching { vec![] } else { vec![CardIdx(2)] }
            );
            assert_eq!(game.battlefield.len(), 1 + usize::from(matching));
            assert_eq!(
                game.graveyard,
                if matching { vec![] } else { vec![CardIdx(1)] }
            );
            assert_eq!(game.seen, u32::from(matching));
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_eq!(game.life, 20);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = "Whenever you cast an artifact spell, draw a card.".into();
    let cards = deck(&[
        changed,
        row("Wrong cast", "{0}", "Creature", ""),
        row("Undrawn", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![1], vec![2]);
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
    assert_eq!(
        attempt_index(&cards, &mut game, &mut ManaPool::default(), CardIdx(1)),
        Some(0)
    );
    assert_eq!(game.library, [CardIdx(2)]);
    assert!(game.hand.is_empty());
}

/// Start one finite seat just before its second scheduled turn.
pub(in crate::deck::simulator) fn opponent_scenario(
    source: &CardRow,
    lands: u32,
) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row("Reward one", "{99}", "Sorcery", ""),
        row("Reward two", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![], vec![1, 2]);
    game.current_phase = crate::deck::simulator::game::GamePhase::OpponentRound;
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
    game.opponents.config.active = true;
    game.opponents.config.land_target = 0;
    game.opponents.config.spell_costs = vec![1];
    game.opponents.config.tax_policy = TaxPolicy::Spare;
    game.opponents.seats[0].turns = 1;
    game.opponents.seats[0].lands = lands;
    game.opponents.seats[0].hand = 1;
    (cards, game)
}

/// Rhystic Study draws only when its event's seat cannot pay from remaining sources.
pub(in crate::deck::simulator) fn assert_rhystic(source: &CardRow) {
    assert_opponent_engine(source, false, 2);
}

/// Smothering Tithe's tax reserves the next spell and banks exactly one unpaid Treasure.
pub(in crate::deck::simulator) fn assert_tithe(source: &CardRow) {
    assert_opponent_engine(source, true, 3);
}

/// Check finite shared resources and a same-name reward change for the declared model.
fn assert_opponent_engine(source: &CardRow, treasure: bool, paid_lands: u32) {
    crate::deck::simulator::card_ast_tests::assert_event_source(source);
    for source in renamed(source) {
        for paid in [true, false] {
            let (cards, mut game) = opponent_scenario(&source, if paid { paid_lands } else { 1 });
            run_opponent_round(&cards, &mut game, 1);
            assert_eq!(game.opponents.draws, 1);
            assert_eq!(game.opponents.casts, 1);
            assert_eq!(game.opponents.payments, u32::from(paid));
            assert_eq!(game.opponents.seats[0].unspent_sources, 0);
            assert_eq!(game.opponents.seats[0].hand, 1);
            let reward = u32::from(!paid);
            assert_eq!(game.treasure_bank, if treasure { reward } else { 0 });
            assert_eq!(game.seen, if treasure { 0 } else { reward });
            assert_eq!(game.hand.len(), if treasure { 0 } else { reward as usize });
            assert_eq!(game.library.len(), 2 - game.hand.len());
            assert_eq!(game.life, 20);
            assert!(game.graveyard.is_empty() && game.exile.is_empty());
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = if treasure {
        "Whenever an opponent draws a card, you gain 2 life."
    } else {
        "Whenever an opponent casts a spell, draw two cards."
    }
    .into();
    let (cards, mut game) = opponent_scenario(&changed, paid_lands);
    run_opponent_round(&cards, &mut game, 1);
    assert_eq!(game.opponents.payments, 0);
    assert_eq!(game.treasure_bank, 0);
    assert_eq!(game.seen, if treasure { 0 } else { 2 });
    assert_eq!(game.life, if treasure { 22 } else { 20 });
}

/// Esper's pinned tax wording cannot grant a draw or spend player resources.
pub(in crate::deck::simulator) fn assert_esper_inert(source: &CardRow) {
    for source in renamed(source) {
        crate::deck::simulator::card_ast_tests::assert_esper_payment_inert(&source);
        let (cards, mut game) = opponent_scenario(&source, 1);
        let before = game.clone();
        run_opponent_round(&cards, &mut game, 1);
        assert_eq!(game.opponents.casts, 1);
        assert_eq!(game.opponents.payments, 0);
        assert_eq!(game.hand, before.hand);
        assert_eq!(game.library, before.library);
        assert_eq!(game.graveyard, before.graveyard);
        assert_eq!(game.exile, before.exile);
        assert_eq!(game.life, before.life);
        assert_eq!(game.life_paid, before.life_paid);
        assert_eq!(game.seen, before.seen);
        assert_eq!(game.treasure_bank, before.treasure_bank);
        assert_eq!(game.battlefield[0].uid, before.battlefield[0].uid);
        assert_eq!(game.battlefield[0].tapped, before.battlefield[0].tapped);
        crate::deck::simulator::card_test_support::assert_library_rng(
            &game.library_rng,
            &before.library_rng,
        );
    }
    let mut changed = source.clone();
    changed.oracle_text = changed
        .oracle_text
        .replace("{X}, where X is this creature's power", "{1}");
    assert_ne!(changed.oracle_text, source.oracle_text);
    let (cards, mut game) = opponent_scenario(&changed, 1);
    run_opponent_round(&cards, &mut game, 1);
    assert_eq!(game.opponents.casts, 1);
    assert_eq!(game.opponents.payments, 0);
    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!(game.library, [CardIdx(1)]);
    assert_eq!(game.seen, 1);
}
