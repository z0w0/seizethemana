//! Flashback and escape casting regressions.

use super::cast_zone_tests::{cast_once, deck, row, state};
use super::game::ManaPool;
use super::*;

#[test]
fn flashback_grants_only_the_graveyard_instances_present_at_resolution() {
    let cards = deck(&[
        row(
            "Flashback grant",
            "{3}{R}",
            "Sorcery",
            "Until end of turn, instant and sorcery cards in your graveyard gain flashback. The flashback cost is equal to its mana cost.",
        ),
        row("Ritual", "{R}", "Instant", "Add {R}{R}."),
        row("Later spell", "{R}", "Instant", "Draw a card."),
        row("Drawn", "{0}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].spell_data.grants_flashback);
    let mut st = state(vec![0], vec![3]);
    st.graveyard.push(model::CardIdx(1));
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 1, 0],
        flexible: 6,
        ..ManaPool::default()
    };
    cast_once(&cards, &mut st, &mut pool);

    assert_eq!(st.replay_casts, 1);
    assert_eq!(st.milestones_by_turn[&1].graveyard_casts, 1);
    assert_eq!(st.exile, [model::CardIdx(1)]);
    assert_eq!(pool.fixed[3], 2);
    assert!(st.flashback_permissions.is_empty());

    st.graveyard.push(model::CardIdx(2));
    cast_once(&cards, &mut st, &mut ManaPool::default());
    assert_eq!(st.replay_casts, 1);
    assert!(st.graveyard.contains(&model::CardIdx(2)));
}

#[test]
fn escape_pays_mana_and_exiles_three_other_graveyard_instances() {
    let cards = deck(&[
        row(
            "Escape grant",
            "{1}{R}",
            "Enchantment",
            "Nonland cards in your graveyard have escape. The escape cost is equal to the card's mana cost plus exile three other cards from your graveyard.",
        ),
        row("First spell", "{R}", "Instant", "Draw a card."),
        row("Second spell", "{R}", "Instant", "Draw a card."),
        row("Fuel one", "{9}", "Sorcery", ""),
        row("Fuel two", "{9}", "Sorcery", ""),
        row("Fuel three", "{9}", "Sorcery", ""),
        row("Fuel four", "{9}", "Sorcery", ""),
        row("Fuel five", "{9}", "Sorcery", ""),
        row("Fuel six", "{9}", "Sorcery", ""),
        row("Draw one", "{0}", "Sorcery", ""),
        row("Draw two", "{0}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].spell_data.grants_escape);
    let mut st = state(vec![0], vec![]);
    st.graveyard
        .extend([1, 3, 4, 5, 2, 6, 7, 8].map(model::CardIdx));
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 3, 0],
        flexible: 1,
        ..ManaPool::default()
    };
    cast_once(&cards, &mut st, &mut pool);

    assert_eq!(pool.fixed[3], 0);
    assert_eq!(st.replay_casts, 2);
    assert_eq!(st.milestones_by_turn[&1].graveyard_casts, 2);
    assert_eq!(st.graveyard, [model::CardIdx(1), model::CardIdx(2)]);
    assert_eq!(st.exile, [3, 4, 5, 6, 7, 8].map(model::CardIdx));
    assert!(
        st.battlefield
            .iter()
            .any(|permanent| permanent.card == game::CardRef::Deck(model::CardIdx(0)))
    );

    let mut no_fuel = state(Vec::new(), vec![]);
    no_fuel
        .battlefield
        .push(game::new_perm_with(10, &cards, model::CardIdx(0), 0, false));
    no_fuel.graveyard.extend([1, 3, 4].map(model::CardIdx));
    cast_once(&cards, &mut no_fuel, &mut ManaPool::default());
    assert!(no_fuel.graveyard.contains(&model::CardIdx(1)));
    assert!(no_fuel.exile.is_empty());
    assert_eq!(no_fuel.replay_casts, 0);
}

#[test]
fn sacrificed_escape_artifact_can_be_escaped_again_with_remaining_fuel() {
    let cards = deck(&[
        row(
            "Escape grant",
            "{1}{R}",
            "Enchantment",
            "Nonland cards in your graveyard have escape. The escape cost is equal to the card's mana cost plus exile three other cards from your graveyard.",
        ),
        row(
            "Mana artifact",
            "{0}",
            "Artifact",
            "Sacrifice this permanent: Add one mana of any color.",
        ),
        row("Fuel one", "{9}", "Sorcery", ""),
        row("Fuel two", "{9}", "Sorcery", ""),
        row("Fuel three", "{9}", "Sorcery", ""),
        row("Fuel four", "{9}", "Sorcery", ""),
        row("Fuel five", "{9}", "Sorcery", ""),
        row("Fuel six", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), Vec::new());
    assert!(cards.cards[1].unlocked_abilities(0).any(|ability| {
        ability
            .activation
            .as_ref()
            .is_some_and(|activation| activation.sacrifices_source())
    }));
    assert!(cards.cards[1].tap.is_some());
    st.next_uid = 1;
    st.battlefield
        .push(game::new_perm_with(1, &cards, model::CardIdx(0), 0, false));
    st.graveyard
        .extend([1, 2, 3, 4, 5, 6, 7].map(model::CardIdx));
    let mut pool = ManaPool::default();

    cast_once(&cards, &mut st, &mut pool);
    assert!(
        st.battlefield
            .iter()
            .any(|permanent| permanent.card == game::CardRef::Deck(model::CardIdx(1)))
    );
    assert_eq!(
        st.battlefield
            .iter()
            .find(|permanent| permanent.card == game::CardRef::Deck(model::CardIdx(1)))
            .map(|permanent| permanent.entered_turn),
        Some(1)
    );
    let permanent = st
        .battlefield
        .iter()
        .find(|permanent| permanent.card == game::CardRef::Deck(model::CardIdx(1)))
        .expect("the escaped artifact stays in play");
    let parsed_artifact = game::card_of(&cards, permanent);
    assert!(!parsed_artifact.is_creature);
    assert!(parsed_artifact.unlocked_abilities(0).any(|ability| {
        ability
            .activation
            .as_ref()
            .is_some_and(|activation| activation.sacrifices_source())
    }));
    assert!(parsed_artifact.gate_types.is_empty());
    assert!(
        parsed_artifact
            .tap
            .as_ref()
            .is_some_and(|yield_| yield_.scaling.is_none())
    );
    assert!(!permanent.tapped);
    pool = game_run::build_pool(&cards, &mut st, 2);
    assert!(
        !st.battlefield
            .iter()
            .any(|permanent| permanent.card == game::CardRef::Deck(model::CardIdx(1)))
    );
    assert!(st.graveyard.contains(&model::CardIdx(1)));
    assert_eq!(pool.total(), 1);

    cast_once(&cards, &mut st, &mut pool);

    assert_eq!(st.replay_casts, 2);
    pool = game_run::build_pool(&cards, &mut st, 3);
    assert_eq!(st.exile.len(), 6);
    assert!(
        !st.battlefield
            .iter()
            .any(|permanent| permanent.card == game::CardRef::Deck(model::CardIdx(1)))
    );
    assert_eq!(st.graveyard, [model::CardIdx(1)]);
    assert_eq!(pool.total(), 1);
}
