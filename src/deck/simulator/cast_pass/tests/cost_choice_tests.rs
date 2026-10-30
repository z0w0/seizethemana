use super::*;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::library_core_report_tests::put_permanents;
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use crate::deck::simulator::turn_loop_tests::cast;

/// A captured reduction can cover generic kicker mana even when the base has no generic.
#[test]
fn unused_generic_reduction_covers_kicker_before_one_payment() {
    let cards = deck(&[
        named_row("Goblin Electromancer"),
        row(
            "Kicked loss",
            "{R}",
            "Instant",
            "Kicker {4}\nTarget player loses 2 life.",
        ),
    ]);
    let mut game = state(vec![1], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let mut pool = ManaPool {
        colorless: 3,
        fixed: [0, 0, 0, 1, 0],
        ..Default::default()
    };
    let choice = choose_cast_cost(&cards, &game, &pool, CardIdx(1), cards.cards[1].min_cost)
        .expect("one generic reduction covers part of the kicker");
    assert!(choice.kicked);
    assert_eq!(choice.cost, parse_cost("{3}{R}"));
    assert_eq!(cast(&cards, &mut game, &mut pool), [4.0]);
    assert_eq!(game.opponent_life_lost, 4);
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(pool.total(), 0);
}

/// Base generic mana and kicker share a single reduction rather than receiving one each.
#[test]
fn base_generic_and_kicker_cannot_spend_the_same_reduction_twice() {
    let cards = deck(&[
        named_row("Goblin Electromancer"),
        row(
            "Kicked generic loss",
            "{1}{R}",
            "Sorcery",
            "Kicker {4}\nTarget player loses 2 life.",
        ),
    ]);
    for generic in [3, 4] {
        let mut game = state(vec![1], vec![]);
        put_permanents(&cards, &mut game, &[0]);
        let mut pool = ManaPool {
            colorless: generic,
            fixed: [0, 0, 0, 1, 0],
            ..Default::default()
        };
        let choice = choose_cast_cost(&cards, &game, &pool, CardIdx(1), cards.cards[1].min_cost)
            .expect("the base spell remains payable");
        assert_eq!(choice.kicked, generic == 4);
        assert_eq!(
            choice.cost,
            parse_cost(if generic == 4 { "{4}{R}" } else { "{R}" })
        );
        assert_eq!(
            cast(&cards, &mut game, &mut pool),
            [if generic == 4 { 5.0 } else { 1.0 }]
        );
        assert_eq!(game.opponent_life_lost, if generic == 4 { 4 } else { 2 });
        assert_eq!(pool.total(), if generic == 4 { 0 } else { 3 });
    }
}

/// A generic reduction cannot replace a colored kicker pip.
#[test]
fn optional_colored_pips_remain_in_the_combined_payment() {
    let cards = deck(&[
        named_row("Goblin Electromancer"),
        row(
            "Colored kicker",
            "{R}",
            "Instant",
            "Kicker {2}{U}\nTarget player loses 2 life.",
        ),
    ]);
    for blue in [false, true] {
        let mut game = state(vec![1], vec![]);
        put_permanents(&cards, &mut game, &[0]);
        let mut pool = ManaPool {
            colorless: 1,
            fixed: [0, u32::from(blue), 0, 1, 0],
            ..Default::default()
        };
        let choice = choose_cast_cost(&cards, &game, &pool, CardIdx(1), cards.cards[1].min_cost)
            .expect("the red base pip is available");
        assert_eq!(choice.kicked, blue);
        assert_eq!(
            choice.cost,
            parse_cost(if blue { "{1}{U}{R}" } else { "{R}" })
        );
        assert_eq!(
            cast(&cards, &mut game, &mut pool),
            [if blue { 3.0 } else { 1.0 }]
        );
        assert_eq!(game.opponent_life_lost, if blue { 4 } else { 2 });
        assert_eq!(pool.total(), if blue { 0 } else { 1 });
    }
}

/// Kicker and offspring both use the same remaining generic reduction.
#[test]
fn kicker_and_offspring_are_added_before_the_single_generic_cut() {
    let cards = deck(&[
        row(
            "Optional reducer",
            "{99}",
            "Artifact",
            "Spells you cast cost {3} less to cast.",
        ),
        row(
            "Optional creature",
            "{G}",
            "Creature",
            "Kicker {2}\nOffspring {2}",
        ),
    ]);
    let mut game = state(vec![1], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let mut pool = ManaPool {
        colorless: 1,
        fixed: [0, 0, 0, 0, 1],
        ..Default::default()
    };
    let choice = choose_cast_cost(&cards, &game, &pool, CardIdx(1), cards.cards[1].min_cost)
        .expect("both optional costs fit the combined total");
    assert!(choice.kicked && choice.offspring);
    assert_eq!(choice.cost, parse_cost("{1}{G}"));
    assert_eq!(cast(&cards, &mut game, &mut pool), [2.0]);
    assert_eq!(game.battlefield.len(), 3);
    assert!(game.hand.is_empty());
    assert_eq!(pool.total(), 0);
}

/// Optional choices cannot lower the X value selected by the existing maximum-X policy.
#[test]
fn maximum_x_stays_ahead_of_optional_kicker() {
    let cards = deck(&[
        named_row("Goblin Electromancer"),
        row(
            "Kicked X",
            "{X}{B}",
            "Sorcery",
            "Kicker {1}\nTarget player loses X life.",
        ),
    ]);
    let mut game = state(vec![1], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let pool = ManaPool {
        colorless: 3,
        fixed: [0, 0, 1, 0, 0],
        ..Default::default()
    };
    let choice = choose_cast_cost(&cards, &game, &pool, CardIdx(1), cards.cards[1].min_cost)
        .expect("maximum X is payable");
    assert_eq!(choice.x, 4);
    assert!(!choice.kicked);
    assert_eq!(choice.cost, parse_cost("{3}{B}"));
}

/// An indivisible X remainder can still fund kicker without changing X.
#[test]
fn repeated_x_leaves_a_remainder_for_optional_costs() {
    let cards = deck(&[row(
        "Repeated kicked X",
        "{X}{X}{B}",
        "Sorcery",
        "Kicker {1}\nTarget player loses X life.",
    )]);
    let game = state(vec![0], vec![]);
    let pool = ManaPool {
        colorless: 3,
        fixed: [0, 0, 1, 0, 0],
        ..Default::default()
    };
    let choice = choose_cast_cost(&cards, &game, &pool, CardIdx(0), cards.cards[0].min_cost)
        .expect("the remainder funds kicker");
    assert_eq!(choice.x, 1);
    assert!(choice.kicked);
    assert_eq!(choice.cost, parse_cost("{3}{B}"));
}

/// Mandatory life and all base and optional Phyrexian pips share one life budget.
#[test]
fn optional_phyrexian_mana_reserves_mandatory_life() {
    let cards = deck(&[row(
        "Phyrexian optional",
        "{B/P}",
        "Sorcery",
        "Kicker {R/P}\nAs an additional cost to cast this spell, pay 3 life.\nGain 1 life.",
    )]);
    for life in [6, 7] {
        let mut game = state(vec![0], vec![]);
        game.life = life;
        let mut pool = ManaPool::default();
        let choice = choose_cast_cost(&cards, &game, &pool, CardIdx(0), cards.cards[0].min_cost)
            .expect("the base mana and mandatory life fit");
        assert_eq!(choice.kicked, life == 7);
        assert_eq!(
            phyrexian_life_charge(&choice.cost),
            if life == 7 { 4 } else { 2 }
        );
        assert_eq!(cast(&cards, &mut game, &mut pool), [0.0]);
        assert_eq!(game.life_paid, if life == 7 { 7 } else { 5 });
        assert!(game.life >= 0);
        assert_eq!(pool.total(), 0);
    }
}

/// Cascade's waived base cost still combines kicker and one generic reduction.
#[test]
fn free_cascade_casts_can_use_a_generic_reduction_for_kicker() {
    let cards = deck(&[
        row(
            "Cascade reducer",
            "{99}",
            "Artifact",
            "Spells you cast cost {1} less to cast.",
        ),
        row("Cascade parent", "{5}", "Sorcery", "Cascade"),
        row(
            "Kicked cascade hit",
            "{R}",
            "Instant",
            "Kicker {4}\nTarget player loses 2 life.",
        ),
    ]);
    let mut game = state(vec![1], vec![2]);
    put_permanents(&cards, &mut game, &[0]);
    let mut pool = ManaPool {
        colorless: 7,
        ..Default::default()
    };
    assert_eq!(cast(&cards, &mut game, &mut pool), [7.0]);
    assert_eq!(game.opponent_life_lost, 4);
    assert_eq!(game.spells_cast_this_turn, 2);
    assert!(game.library.is_empty());
    assert_eq!(pool.total(), 0);
}

/// A pitch cast combines Behold's extra mana and kicker before applying one cut.
#[test]
fn alternative_casts_combine_behold_and_kicker_without_paying_the_base() {
    let mut pitch = row("Blue pitch card", "{99}{U}", "Creature — Human", "");
    pitch.colors = "[\"U\"]".into();
    let cards = deck(&[
        row(
            "Alternative reducer",
            "{99}",
            "Artifact",
            "Spells you cast cost {3} less to cast.",
        ),
        row(
            "Optional alternative",
            "{9}{R}",
            "Sorcery",
            "You may exile one blue card from your hand rather than pay this spell's mana cost.\nKicker {4}\nAs an additional cost to cast this spell, behold an Elf or pay {2}.\nTarget player loses 2 life.",
        ),
        pitch,
    ]);
    let mut game = state(vec![1, 2], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let mut pool = ManaPool {
        colorless: 3,
        ..Default::default()
    };
    assert_eq!(cast(&cards, &mut game, &mut pool), [3.0]);
    assert_eq!(game.exile, [CardIdx(2)]);
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(game.opponent_life_lost, 4);
    assert_eq!(pool.total(), 0);
}

/// Combining optional costs retains every supported non-generic mana symbol.
#[test]
fn combined_costs_keep_all_mana_symbol_counts() {
    let base = parse_cost("{1}{W}{W/U}{2/B}{R/P}{C}{S}");
    let extra = parse_cost("{2}{W}{W/U}{2/B}{R/P}{C}{S}");
    assert_eq!(
        combine_costs(base, extra),
        Some(parse_cost(
            "{3}{W}{W}{W/U}{W/U}{2/B}{2/B}{R/P}{R/P}{C}{C}{S}{S}"
        ))
    );
    assert!(combine_costs(base, parse_cost("{X}")).is_none());
    assert!(combine_costs(base, parse_cost("{Q}")).is_none());
}
