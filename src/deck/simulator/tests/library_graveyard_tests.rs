use super::*;
use crate::deck::simulator::{game_effects, model};

/// A mill resolves supported return and drain triggers after moving the cards.
#[test]
fn library_mill_triggers_return_creatures_and_exile_drain_cards() {
    let cards = deck(&[
        row(
            "Library returner",
            "{1}{U}",
            "Creature — Illusion",
            "[\"U\"]",
            "When Library Returner is put into your graveyard from your library, you may put it onto the battlefield.",
        ),
        row(
            "Library returner",
            "{1}{U}",
            "Creature — Illusion",
            "[\"U\"]",
            "When Library Returner is put into your graveyard from your library, you may put it onto the battlefield.",
        ),
        row(
            "Life drain card",
            "{2}{B}",
            "Sorcery",
            "[\"B\"]",
            "When Life Drain Card is put into your graveyard from your library, each opponent loses 3 life and you gain 3 life.",
        ),
        row("Blank", "{1}", "Sorcery", "[]", ""),
    ]);
    assert!(cards.cards[0].library_graveyard_trigger.is_some());
    assert!(cards.cards[1].library_graveyard_trigger.is_some());
    let mut game_state = state(Vec::new(), vec![3, 2, 1, 0]);

    game_effects::apply_effect_at(
        &cards,
        &model::SimEffect::Mill(3),
        &mut game_state,
        4,
        false,
        None,
    );

    assert_eq!(game_state.milled_self, 3);
    assert_eq!(game_state.seen, 3);
    assert_eq!(game_state.awareness_cards, 3);
    assert_eq!(
        game_state
            .battlefield
            .iter()
            .map(|permanent| permanent.card.deck_idx().unwrap().index())
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(
        game_state.exile,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(game_state.graveyard.is_empty());
    assert_eq!(
        game_state.library,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(game_state.opponent_life_lost, 3);
    assert_eq!(game_state.life_gained, 3);
    assert_eq!(game_state.life, 23);
}

/// A discard cannot fire an ability that requires a library-to-graveyard move.
#[test]
fn discarding_a_library_trigger_card_does_not_fire_its_mill_ability() {
    let cards = deck(&[
        row(
            "Life drain card",
            "{2}{B}",
            "Sorcery",
            "[\"B\"]",
            "When Life Drain Card is put into your graveyard from your library, each opponent loses 3 life and you gain 3 life.",
        ),
        row("Draw", "{1}", "Sorcery", "[]", ""),
    ]);
    let mut game_state = state(vec![0], vec![1]);

    game_effects::apply_effect_at(
        &cards,
        &model::SimEffect::DiscardHandThenDrawSeven,
        &mut game_state,
        1,
        false,
        None,
    );

    assert_eq!(
        game_state.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(game_state.opponent_life_lost, 0);
    assert_eq!(game_state.life_gained, 0);
}
