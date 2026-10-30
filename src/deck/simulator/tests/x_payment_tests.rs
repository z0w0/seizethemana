use super::*;
use crate::deck::simulator::turn_loop_tests::{cast, deck, row, state};

/// Repeated X symbols share one value and pay for every occurrence.
#[test]
fn repeated_x_leaves_the_indivisible_remainder() {
    let cards = deck(&[row(
        "Repeated X",
        "{X}{X}{B}",
        "Sorcery",
        "Target player loses X life.",
    )]);
    let mut st = state(vec![0], vec![]);
    let mut pool = ManaPool {
        colorless: 7,
        fixed: [0, 0, 1, 0, 0],
        ..Default::default()
    };
    let spent = cast(&cards, &mut st, &mut pool);
    assert_eq!(spent, [7.0]);
    assert_eq!(pool.total(), 1);
    assert_eq!(st.opponent_life_lost, 3);
}

/// An X value of zero is legal when only the fixed cost can be paid.
#[test]
fn zero_x_does_not_create_a_free_effect() {
    let cards = deck(&[row(
        "Zero X",
        "{X}{B}",
        "Sorcery",
        "Target player loses X life.",
    )]);
    let mut st = state(vec![0], vec![]);
    let mut pool = ManaPool {
        fixed: [0, 0, 1, 0, 0],
        ..Default::default()
    };
    cast(&cards, &mut st, &mut pool);
    assert_eq!(pool.total(), 0);
    assert_eq!(st.opponent_life_lost, 0);
    assert_eq!(st.graveyard, [CardIdx(0)]);
}
