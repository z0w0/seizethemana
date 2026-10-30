use super::*;
use crate::deck::simulator::card_test_support::assert_library_rng;
use crate::deck::simulator::cast_pass::{cast_graveyard_spells, try_cast_from_hand};
use crate::deck::simulator::model::{CardIdx, Cost, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use game::{CardRef, GamePhase, GameState, ManaPool, new_perm_with};

/// Keep Escape costs separate from the creature's normal printed-cost entry reward.
fn escape_scenario(payment: &str) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Escape payment control",
            "{3}{R}{R}",
            "Creature",
            &format!("When this creature enters, gain 1 life.\nEscape—{payment}."),
        ),
        row("First graveyard card", "{99}", "Sorcery", ""),
        row("Second graveyard card", "{99}", "Sorcery", ""),
        row("Third graveyard card", "{99}", "Sorcery", ""),
        row("Retained hand", "{99}", "Sorcery", ""),
        row("Undrawn card", "{99}", "Sorcery", ""),
        row("Prior exile", "{99}", "Artifact", ""),
        row("Prior permanent", "{99}", "Artifact", ""),
    ]);
    let mut game = state(vec![4], vec![5]);
    game.current_turn = 1;
    game.current_phase = GamePhase::Main;
    game.graveyard = vec![CardIdx(0), CardIdx(1), CardIdx(2), CardIdx(3)];
    game.graveyard_seen.insert(CardIdx(0), 1);
    game.graveyard_generations.insert(CardIdx(0), 2);
    game.exile.push(CardIdx(6));
    game.battlefield
        .push(new_perm_with(41, &cards, CardIdx(7), 0, true));
    game.next_uid = 41;
    (cards, game)
}

/// Rejected Escape routes cannot spend resources or record casts and zone visits.
fn assert_replay_rejected(cards: &SimDeck, game: &mut GameState, pool: &mut ManaPool) {
    let before = game.clone();
    let before_pool = pool.clone();
    let mut spent = [0.0];
    let mut sources = Vec::new();
    cast_graveyard_spells(cards, game, pool, 1, &mut spent, &mut sources);
    assert_eq!(*pool, before_pool);
    assert_eq!(game.hand, before.hand);
    assert_eq!(game.library, before.library);
    assert_eq!(game.graveyard, before.graveyard);
    assert_eq!(game.exile, before.exile);
    assert_eq!(
        format!("{:?}", game.battlefield),
        format!("{:?}", before.battlefield)
    );
    assert_eq!(game.battlefield_seen, before.battlefield_seen);
    assert_eq!(game.graveyard_seen, before.graveyard_seen);
    assert_eq!(game.graveyard_generations, before.graveyard_generations);
    assert_eq!(game.card_first_seen, before.card_first_seen);
    assert_eq!(game.life, before.life);
    assert_eq!(game.life_paid, before.life_paid);
    assert_eq!(game.life_gained, before.life_gained);
    assert_eq!(game.spells_cast_this_turn, before.spells_cast_this_turn);
    assert_eq!(game.replay_casts, before.replay_casts);
    assert_eq!(game.next_uid, before.next_uid);
    assert_eq!(game.seen, before.seen);
    assert_eq!(game.milestones_by_turn, before.milestones_by_turn);
    assert_eq!(game.completion, before.completion);
    assert_eq!(spent, [0.0]);
    assert!(sources.is_empty());
    assert_library_rng(&game.library_rng, &before.library_rng);
}

/// Extra Escape life costs stay inert even when the player can afford them.
#[test]
fn escape_with_extra_life_cannot_replay_for_only_mana_and_three_cards() {
    for payment in [
        "Pay 1 life, {R}{R}, Exile three other cards from your graveyard",
        "{R}{R}, Pay 1 life, Exile three other cards from your graveyard",
        "{R}{R}, Exile three other cards from your graveyard, Pay 1 life",
    ] {
        let (cards, initial) = escape_scenario(payment);
        assert!(cards.cards[0].spell_data.own_escape.is_none());
        for life in [1, 20] {
            for generic in [0, 3] {
                let mut game = initial.clone();
                game.life = life;
                let mut pool = ManaPool {
                    fixed: [0, 0, 0, 2, 0],
                    colorless: generic,
                    ..ManaPool::default()
                };
                assert_replay_rejected(&cards, &mut game, &mut pool);
            }
        }
    }
}

/// Other payment requirements and non-three-card counts cannot use the fixed replay route.
#[test]
fn escape_with_unknown_duplicate_or_wrong_count_costs_stays_inert() {
    for payment in [
        "Discard a card, {R}{R}, Exile three other cards from your graveyard",
        "Sacrifice a creature, {R}{R}, Exile three other cards from your graveyard",
        "{R}{R}, Discard a card, Exile three other cards from your graveyard",
        "{R}{R}, Exile three other creature cards from your graveyard",
        "{R}{R}, Exile three or more other cards from your graveyard",
        "{R}{R}, Exile three other cards from your graveyard at random",
        "{R}{R} and pay 1 life, Exile three other cards from your graveyard",
        "{R}{R}, {1}, Exile three other cards from your graveyard",
        "{R}{R}, Exile three other cards from your graveyard, Exile three other cards from your graveyard",
        "{R}{R}, Exile 0 other cards from your graveyard",
        "{R}{R}, Exile eight other cards from your graveyard",
    ] {
        let (cards, mut game) = escape_scenario(payment);
        assert!(cards.cards[0].spell_data.own_escape.is_none(), "{payment}");
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 2, 0],
            colorless: 3,
            ..ManaPool::default()
        };
        assert_replay_rejected(&cards, &mut game, &mut pool);
    }
}

/// The supported route pays red mana and exiles exactly three distinct other cards.
#[test]
fn modeled_escape_pays_its_complete_mana_and_three_card_cost() {
    let (cards, mut game) = escape_scenario("{R}{R}, Exile three other cards from your graveyard");
    assert_eq!(
        cards.cards[0].spell_data.own_escape,
        Some(Cost {
            pips: [0, 0, 0, 2, 0],
            ..Cost::default()
        })
    );
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 2, 0],
        colorless: 3,
        ..ManaPool::default()
    };
    let mut spent = [0.0];
    cast_graveyard_spells(&cards, &mut game, &mut pool, 1, &mut spent, &mut Vec::new());
    assert_eq!(spent, [2.0]);
    assert_eq!(pool.fixed, [0; 5]);
    assert_eq!(pool.colorless, 3);
    assert!(game.graveyard.is_empty());
    assert_eq!(game.exile, [CardIdx(6), CardIdx(1), CardIdx(2), CardIdx(3)]);
    assert_eq!(game.battlefield.len(), 2);
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(game.battlefield_seen.get(&CardIdx(0)), Some(&1));
    assert_eq!(game.replay_casts, 1);
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_eq!(game.life, 21);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.life_gained, 1);
    assert!(game.completion.is_none());
}

/// An inert Escape cost does not block a legal normal cast or charge its extra payments.
#[test]
fn unsupported_escape_preserves_the_normal_printed_cost_hand_route() {
    let (cards, initial) =
        escape_scenario("{R}{R}, Pay 1 life, Exile three other cards from your graveyard");
    assert_eq!(
        cards.cards[0].min_cost,
        Cost {
            generic: 3,
            pips: [0, 0, 0, 2, 0],
            ..Cost::default()
        }
    );
    for generic in [0, 3] {
        let mut game = initial.clone();
        game.graveyard.remove(0);
        game.hand.insert(0, CardIdx(0));
        let before = game.clone();
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 2, 0],
            colorless: generic,
            ..ManaPool::default()
        };
        let before_pool = pool.clone();
        let result = try_cast_from_hand(
            &cards,
            &mut game,
            &mut pool,
            1,
            CardIdx(0),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut [false; 5],
        );
        assert_eq!(result, (generic == 3).then_some(5));
        if generic == 0 {
            assert_eq!(pool, before_pool);
            assert_eq!(game.hand, before.hand);
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.spells_cast_this_turn, 0);
        } else {
            assert_eq!(pool.total(), 0);
            assert_eq!(game.hand, [CardIdx(4)]);
            assert_eq!(game.battlefield.len(), 2);
            assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(0)));
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_eq!(game.life, 21);
            assert_eq!(game.life_paid, 0);
        }
        assert_eq!(game.graveyard, before.graveyard);
        assert_eq!(game.exile, before.exile);
        assert_eq!(game.library, before.library);
        assert_eq!(game.replay_casts, 0);
        assert!(game.completion.is_none());
    }
}
