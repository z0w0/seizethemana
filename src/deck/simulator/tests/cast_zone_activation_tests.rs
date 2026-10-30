//! Life-paid draw and sacrifice activation regressions.

use super::cast_zone_tests::{cast_once, deck, row, state};
use super::game::ManaPool;
use super::*;

#[test]
fn ad_nauseam_reveals_in_order_and_can_cause_a_lethal_reveal() {
    let cards = deck(&[
        row(
            "Reveal spell",
            "{3}{B}{B}",
            "Sorcery",
            "Reveal the top card of your library and put that card into your hand. You lose life equal to its mana value. Repeat this process any number of times.",
        ),
        row("Zero", "{0}", "Sorcery", ""),
        row("One", "{1}", "Sorcery", ""),
        row("Seven", "{7}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].spell_data.reveal_rule.is_some());
    // Library order is the draw order; pop() reveals from the end.
    let mut st = state(vec![0], vec![1, 2, 3]);
    st.life = 5;
    let mut pool = ManaPool {
        fixed: [0, 0, 2, 0, 0],
        flexible: 3,
        ..ManaPool::default()
    };

    cast_once(&cards, &mut st, &mut pool);

    // CR 119.4: the top card (Seven, mana value 7) would drive life below
    // zero, so the loop stops before revealing it.
    assert!(st.hand.is_empty(), "no card is revealed: {:?}", st.hand);
    assert_eq!(st.seen, 0);
    assert_eq!(st.life, 5);
    assert_eq!(st.library, [1, 2, 3].map(model::CardIdx));
    assert_eq!(st.life_funded_draws, 0);
    assert_eq!(
        st.milestones_by_turn
            .get(&1)
            .map_or(0, |m| m.life_funded_draws),
        0
    );

    // At life 7 the reveal is legal: Seven is drawn, life hits 0, and the
    // next card would go below zero, so the loop stops.
    let mut st_ok = state(vec![0], vec![1, 2, 3]);
    st_ok.life = 7;
    let mut pool = ManaPool {
        fixed: [0, 0, 2, 0, 0],
        flexible: 3,
        ..ManaPool::default()
    };
    cast_once(&cards, &mut st_ok, &mut pool);
    assert_eq!(st_ok.hand, [model::CardIdx(3)]);
    assert_eq!(st_ok.life, 0);
    assert_eq!(st_ok.life_funded_draws, 1);
    assert_eq!(st_ok.milestones_by_turn[&1].life_funded_draws, 1);
}

#[test]
fn griselbrand_repeats_life_paid_draws_only_while_life_can_pay() {
    let mut rows = vec![row(
        "Draw engine",
        "{4}{B}{B}{B}{B}",
        "Creature — Demon",
        "Pay 7 life: Draw seven cards.",
    )];
    rows.extend((0..14).map(|_| row("Filler", "{0}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut st = state(Vec::new(), (1..15).collect());
    st.battlefield
        .push(game::new_perm_with(1, &cards, model::CardIdx(0), 0, false));

    game_effects::spend_leftover(&cards, &mut st, &mut ManaPool::default(), 1);

    assert_eq!(st.hand.len(), 14);
    assert_eq!(st.life, 6);
    assert_eq!(st.life_paid, 14);
    assert_eq!(st.seen, 14);
    assert_eq!(st.life_funded_draws, 14);
    assert_eq!(st.milestones_by_turn[&1].life_funded_draws, 14);

    let mut rows = vec![
        row(
            "Draw engine",
            "{4}{B}{B}{B}{B}",
            "Creature — Demon",
            "Pay 7 life: Draw seven cards.",
        ),
        row("Life spell", "{0}", "Sorcery", "You gain 7 life."),
    ];
    rows.extend((0..14).map(|_| row("Filler", "{0}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut st = state(vec![1], (2..16).collect());
    st.life = 13;
    st.battlefield
        .push(game::new_perm_with(1, &cards, model::CardIdx(0), 0, false));
    cast_once(&cards, &mut st, &mut ManaPool::default());
    game_effects::spend_leftover(&cards, &mut st, &mut ManaPool::default(), 1);

    assert_eq!(st.life_gained, 7);
    assert_eq!(st.life, 6);
    assert_eq!(st.hand.len(), 14);
    assert_eq!(st.life_paid, 14);
    assert_eq!(st.life_funded_draws, 14);
}

#[test]
fn sacrifice_draw_resolves_undying_and_cancels_the_targets_counter() {
    let cards = deck(&[
        row(
            "Sacrifice engine",
            "{2}{B}{B}",
            "Creature — Human Cleric",
            "Pay 1 life, Sacrifice another creature: Draw a card, then put a -1/-1 counter on up to one target creature.",
        ),
        row("Fodder", "{G}", "Creature — Wolf", "Undying"),
        row("Target", "{G}", "Creature — Wolf", "Undying"),
        row("Drawn", "{0}", "Sorcery", ""),
        row("Plain fodder", "{G}", "Creature — Wolf", ""),
    ]);
    let mut st = state(Vec::new(), vec![3]);
    st.battlefield
        .push(game::new_perm_with(1, &cards, model::CardIdx(0), 0, false));
    st.battlefield
        .push(game::new_perm_with(2, &cards, model::CardIdx(1), 0, false));
    let mut target = game::new_perm_with(3, &cards, model::CardIdx(2), 0, false);
    target.counters = model::Counters {
        plus1: 1,
        ..Default::default()
    };
    st.battlefield.push(target);
    let ability = cards.cards[0]
        .unlocked_abilities(0)
        .next()
        .expect("the text parses as an activation")
        .clone();
    let costs = ability.activation.as_ref().expect("typed activation costs");
    assert_eq!(costs.life_payment(), 1);
    assert_eq!(costs.creature_sacrifices(), 1);
    assert_eq!(costs.mana_cost().total(), 0);
    assert!(matches!(
        ability.effect,
        model::SimEffect::DrawAndMinusCounter
    ));
    let activation =
        game_effects::activation::pick_best_activation(&cards, &st, &ManaPool::default(), 1)
            .expect("the ability has a legal sacrifice and target");

    game_effects::activation::resolve_activation(
        &cards,
        &mut st,
        &mut ManaPool::default(),
        1,
        &activation,
    );

    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 1);
    assert_eq!(st.hand, [model::CardIdx(3)]);
    assert_eq!(st.life_funded_draws, 1);
    assert!(st.battlefield.iter().any(|permanent| {
        permanent.card == game::CardRef::Deck(model::CardIdx(1)) && permanent.counters.plus1 == 1
    }));
    assert!(st.battlefield.iter().any(|permanent| {
        permanent.card == game::CardRef::Deck(model::CardIdx(2))
            && permanent.counters == Default::default()
    }));

    let mut no_target = state(Vec::new(), vec![3]);
    no_target
        .battlefield
        .push(game::new_perm_with(10, &cards, model::CardIdx(0), 0, false));
    no_target
        .battlefield
        .push(game::new_perm_with(11, &cards, model::CardIdx(4), 0, false));
    let activation =
        game_effects::activation::pick_best_activation(&cards, &no_target, &ManaPool::default(), 1)
            .expect("the optional target can be absent");
    game_effects::activation::resolve_activation(
        &cards,
        &mut no_target,
        &mut ManaPool::default(),
        1,
        &activation,
    );
    assert_eq!(no_target.life_paid, 1);
    assert_eq!(no_target.life_funded_draws, 1);
    assert_eq!(no_target.hand, [model::CardIdx(3)]);
    assert!(
        no_target
            .battlefield
            .iter()
            .all(|permanent| permanent.card != game::CardRef::Deck(model::CardIdx(4)))
    );
}

#[test]
fn undying_returns_through_minus_counters() {
    // Undying (CR 702.93) checks only +1/+1 counters: a creature carrying
    // a -1/-1 counter (and no +1/+1) still returns when it dies.
    let cards = deck(&[
        row(
            "Sacrifice engine",
            "{2}{B}{B}",
            "Creature — Human Cleric",
            "Pay 1 life, Sacrifice another creature: Draw a card, then put a -1/-1 counter on up to one target creature.",
        ),
        row("Fodder", "{G}", "Creature — Wolf", "Undying"),
        row("Drawn", "{0}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![2]);
    st.battlefield
        .push(game::new_perm_with(1, &cards, model::CardIdx(0), 0, false));
    let mut fodder = game::new_perm_with(2, &cards, model::CardIdx(1), 0, false);
    fodder.counters = model::Counters {
        minus1: 1,
        ..Default::default()
    };
    st.battlefield.push(fodder);
    let activation =
        game_effects::activation::pick_best_activation(&cards, &st, &ManaPool::default(), 1)
            .expect("the ability has a legal sacrifice");
    game_effects::activation::resolve_activation(
        &cards,
        &mut st,
        &mut ManaPool::default(),
        1,
        &activation,
    );
    assert!(st.battlefield.iter().any(|permanent| {
        permanent.card == game::CardRef::Deck(model::CardIdx(1)) && permanent.counters.plus1 == 1
    }));
}
