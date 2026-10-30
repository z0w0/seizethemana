use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::game::ManaPool;
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_run::{TurnCensus, commander_phase};
use crate::deck::simulator::library_core_report_tests::put_permanents;
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use crate::deck::simulator::turn_loop_tests::cast;

/// A captured reducer makes the ordinary hand cast affordable without reducing its pip.
#[test]
fn captured_instant_reducer_changes_a_complete_cast_payment() {
    let cards = deck(&[
        named_row("Goblin Electromancer"),
        row("Discounted instant", "{3}{U}", "Instant", "Gain 2 life."),
    ]);
    for (sources, blue, expected) in [(0, true, false), (1, true, true), (1, false, false)] {
        let mut game = state(vec![1], vec![]);
        if sources == 1 {
            put_permanents(&cards, &mut game, &[0]);
        }
        let mut pool = ManaPool {
            colorless: if blue { 2 } else { 3 },
            fixed: [0, u32::from(blue), 0, 0, 0],
            ..Default::default()
        };
        let before = pool.clone();
        let spent = cast(&cards, &mut game, &mut pool);
        assert_eq!(game.hand.is_empty(), expected);
        assert_eq!(game.life_gained, if expected { 2 } else { 0 });
        if expected {
            assert_eq!(spent, [3.0]);
            assert_eq!(game.graveyard, [CardIdx(1)]);
            assert_eq!(pool.total(), 0);
        } else {
            assert_eq!(spent, [0.0]);
            assert_eq!(pool, before);
            assert!(game.graveyard.is_empty());
        }
    }
}

/// A reducer cast during the pass changes the next spell's live total cost.
#[test]
fn a_new_source_reduces_later_casts_in_the_same_pass() {
    let cards = deck(&[
        named_row("Foundry Inspector"),
        row("Follow-up artifact", "{4}", "Artifact", ""),
    ]);
    let mut game = state(vec![0, 1], vec![]);
    let mut pool = ManaPool {
        colorless: 6,
        ..Default::default()
    };
    assert_eq!(cast(&cards, &mut game, &mut pool), [6.0]);
    assert!(game.hand.is_empty());
    assert_eq!(game.battlefield.len(), 2);
    assert_eq!(pool.total(), 0);
}

/// Flat discounts extend X's search bound and apply once after X is chosen.
#[test]
fn generic_reduction_can_pay_for_x_without_generic_mana() {
    for (cost, generic, expected_x) in [("{X}{B}", 0, 5), ("{X}{X}{B}", 1, 3)] {
        let cards = deck(&[
            row(
                "X reducer",
                "{8}",
                "Artifact",
                "Spells you cast cost {5} less to cast.",
            ),
            row("X drain", cost, "Sorcery", "Target player loses X life."),
        ]);
        let mut game = state(vec![1], vec![]);
        put_permanents(&cards, &mut game, &[0]);
        let mut pool = ManaPool {
            colorless: generic,
            fixed: [0, 0, 1, 0, 0],
            ..Default::default()
        };
        assert_eq!(cast(&cards, &mut game, &mut pool), [f64::from(generic + 1)]);
        assert_eq!(game.opponent_life_lost, expected_x);
        assert_eq!(game.graveyard, [CardIdx(1)]);
        assert_eq!(pool.total(), 0);
    }
}

/// Flashback replaces the base cost, then applies the same live generic reduction.
#[test]
fn flashback_cost_is_reduced_once_and_still_exiles_the_spell() {
    let cards = deck(&[
        named_row("Goblin Electromancer"),
        row(
            "Replay instant",
            "{7}{U}",
            "Instant",
            "Gain 2 life.\nFlashback {2}{U}",
        ),
    ]);
    let mut game = state(vec![], vec![]);
    game.graveyard.push(CardIdx(1));
    put_permanents(&cards, &mut game, &[0]);
    let mut pool = ManaPool {
        colorless: 1,
        fixed: [0, 1, 0, 0, 0],
        ..Default::default()
    };
    assert_eq!(cast(&cards, &mut game, &mut pool), [2.0]);
    assert_eq!(game.exile, [CardIdx(1)]);
    assert!(game.graveyard.is_empty());
    assert_eq!(game.life_gained, 2);
    assert_eq!(pool.total(), 0);
}

/// A nonmana alternative still pays its exile cost and never receives a mana refund.
#[test]
fn a_reducer_does_not_replace_nonmana_alternative_costs() {
    let mut pitch = row("Blue pitch card", "{8}{U}", "Sorcery", "");
    pitch.colors = "[\"U\"]".into();
    let cards = deck(&[
        named_row("Goblin Electromancer"),
        row(
            "Alternative spell",
            "{10}{U}",
            "Sorcery",
            "You may exile one blue card from your hand rather than pay this spell's mana cost.\nGain 2 life.",
        ),
        pitch,
    ]);
    let mut game = state(vec![1, 2], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let mut pool = ManaPool::default();
    assert_eq!(cast(&cards, &mut game, &mut pool), [0.0]);
    assert!(game.hand.is_empty());
    assert_eq!(game.exile, [CardIdx(2)]);
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(game.life_gained, 2);
    assert_eq!(pool, ManaPool::default());
}

/// A real recast adds commander tax before applying a live creature-spell reduction.
#[test]
fn commander_recasts_reduce_the_tax_but_keep_the_colored_pip() {
    let mut cards = deck(&[row(
        "Creature reducer",
        "{8}",
        "Artifact",
        "Creature spells you cast cost {3} less to cast.",
    )]);
    cards.commanders = deck(&[row("Recast commander", "{U}", "Legendary Creature", "")]).cards;
    assert_eq!(cards.cards[0].cost_reductions.len(), 1);
    assert!(cards.commanders[0].has_mana_cost);
    let profile = CommanderProfile::new(&cards);
    let mut census = TurnCensus::new(3, cards.cards.len());
    let mut game = state(vec![], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let mut repeatable = Vec::new();
    for turn in 1..=2 {
        let mut pool = ManaPool {
            fixed: [0, 1, 0, 0, 0],
            ..Default::default()
        };
        commander_phase(
            &cards,
            &mut game,
            &mut pool,
            &mut census,
            &profile,
            turn,
            &mut repeatable,
            false,
        );
        assert_eq!(game.commander_casts, [turn as u32]);
        assert_eq!(pool.total(), 0);
        game.battlefield.retain(|permanent| {
            !matches!(
                permanent.card,
                crate::deck::simulator::game::CardRef::Commander { .. }
            )
        });
    }
    let mut pool = ManaPool {
        colorless: 2,
        ..Default::default()
    };
    commander_phase(
        &cards,
        &mut game,
        &mut pool,
        &mut census,
        &profile,
        3,
        &mut repeatable,
        false,
    );
    assert_eq!(game.commander_casts, [2]);
    assert_eq!(census.mana_spent, [1.0, 1.0, 0.0]);
    assert_eq!(pool.colorless, 2);
}
