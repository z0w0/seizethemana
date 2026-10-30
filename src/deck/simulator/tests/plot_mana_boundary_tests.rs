//! Plotted casts stop at terminal boundaries, and opponent mana keeps tap bonuses.

use crate::deck::simulator::card_test_support::{main_phase_state, named_row};
use crate::deck::simulator::cast_pass::try_cast_from_hand;
use crate::deck::simulator::game::{
    GameCompletion, GamePhase, GameState, IncompleteReason, ManaPool, PlayerLossReason,
    new_perm_with,
};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_effects::activation::spend_leftover;
use crate::deck::simulator::game_run::mana::activate_mana_mode;
use crate::deck::simulator::game_run::{TurnCensus, run_turn};
use crate::deck::simulator::model::{CardIdx, SimAbilityKind, SimDeck, SimEffect};
use crate::deck::simulator::scenario_test_support::{deck, row};

/// Execute one scheduled turn through plotting, mana production, and later cast selection.
fn play_turn(cards: &SimDeck, game: &mut GameState, census: &mut TurnCensus, turn: usize) {
    run_turn(
        cards,
        game,
        census,
        &CommanderProfile::new(cards),
        &mut Vec::new(),
        turn,
        false,
    );
}

/// Plot three real hand cards on turn one, with later life and creature payments available.
fn plotted_scenario(
    first_effect: &str,
    life: i32,
    recursive_ring: bool,
) -> (SimDeck, GameState, TurnCensus) {
    let cards = deck(&[
        row(
            "First plotted spell",
            "{100}",
            "Sorcery",
            &format!(
                "Plot {{1}}\nAs an additional cost to cast this spell, pay 1 life.\n{first_effect}"
            ),
        ),
        row(
            "Later life payment",
            "{100}",
            "Sorcery",
            "Plot {1}\nAs an additional cost to cast this spell, pay 1 life.\nYou gain 3 life.",
        ),
        row(
            "Later creature payment",
            "{100}",
            "Sorcery",
            "Plot {1}\nAs an additional cost to cast this spell, sacrifice a creature.\nYou gain 4 life.",
        ),
        row("Normal turn draw", "{100}", "Sorcery", ""),
        row("Available sacrifice", "{100}", "Creature", ""),
        row(
            "Recursive Ring listener",
            "{100}",
            "Enchantment",
            "Whenever the Ring tempts you, the Ring tempts you.",
        ),
        row(
            "Plot payment source",
            "{3}",
            "Artifact",
            "{T}: Add {C}{C}{C}.",
        ),
    ]);
    for card in &cards.cards[..3] {
        assert_eq!(
            card.keyword_abilities
                .plot
                .expect("parsed plot payment")
                .total(),
            1
        );
        assert!(!card.spell_data.unsupported_additional_cost);
    }
    assert_eq!(cards.cards[0].spell_data.additional_cost_life, 1);
    assert_eq!(cards.cards[1].spell_data.additional_cost_life, 1);
    assert_eq!(cards.cards[2].spell_data.additional_cost_creatures, 1);
    let mut game = main_phase_state(vec![0, 1, 2], vec![3]);
    game.life = life;
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(4), 0, false));
    game.battlefield
        .push(new_perm_with(9, &cards, CardIdx(6), 0, false));
    game.next_uid = 9;
    if recursive_ring {
        assert!(
            cards.cards[5].striations[0].abilities[0]
                .effect_sequence()
                .iter()
                .any(|effect| matches!(effect, SimEffect::RingTempts))
        );
        game.battlefield
            .push(new_perm_with(8, &cards, CardIdx(5), 0, false));
    }
    let mut census = TurnCensus::new(2, cards.cards.len());
    play_turn(&cards, &mut game, &mut census, 1);
    assert_eq!(
        game.plotted,
        [(CardIdx(0), 1), (CardIdx(1), 1), (CardIdx(2), 1)]
    );
    assert!(game.hand.is_empty());
    assert_eq!(game.spells_cast_this_turn, 0);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.life, life);
    assert!(
        game.battlefield
            .iter()
            .any(|permanent| permanent.uid == 9 && permanent.tapped)
    );
    assert_eq!(census.mana_remaining[0], 0.0);
    assert!(game.completion.is_none());
    (cards, game, census)
}

/// A terminal first spell preserves the later plotted cards and their unpaid costs.
fn assert_later_plotted_costs_untouched(game: &GameState, census: &TurnCensus) {
    assert_eq!(game.plotted, [(CardIdx(1), 1), (CardIdx(2), 1)]);
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_eq!(game.life_paid, 1);
    assert_eq!(game.life_gained, 0);
    assert!(game.battlefield.iter().any(|permanent| permanent.uid == 7));
    assert!(!game.graveyard.contains(&CardIdx(4)));
    assert!(!game.graveyard.contains(&CardIdx(1)));
    assert!(!game.graveyard.contains(&CardIdx(2)));
    assert_eq!(game.hand, [CardIdx(3)]);
    assert!(game.library.is_empty());
    assert!(census.turn_observations[1].draw);
    assert!(!census.turn_observations[1].main);
    assert!(!census.turn_observations[1].combat);
    assert!(!census.turn_observations[1].end_turn);
}

/// Failed draws stop selection of the remaining plotted spells after committed life is paid.
#[test]
fn plotted_failed_draw_preserves_later_life_and_creature_payments() {
    let (cards, mut game, mut census) = plotted_scenario("Draw a card.", 3, false);
    play_turn(&cards, &mut game, &mut census, 2);
    assert_later_plotted_costs_untouched(&game, &census);
    assert_eq!(game.life, 2);
    assert!(game.failed_draw);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert!(matches!(
        &game.completion,
        Some(GameCompletion::PlayerLost { reasons, turn: 2, phase: GamePhase::Main })
            if reasons == &[PlayerLossReason::FailedDraw]
    ));
}

/// A last-life payment cannot resolve its own reward or pay another plotted card's creature cost.
#[test]
fn plotted_last_life_payment_stops_before_rewards_and_later_casts() {
    let (cards, mut game, mut census) = plotted_scenario("You gain 9 life.", 1, false);
    play_turn(&cards, &mut game, &mut census, 2);
    assert_later_plotted_costs_untouched(&game, &census);
    assert_eq!(game.life, 0);
    assert!(matches!(
        &game.completion,
        Some(GameCompletion::PlayerLost { reasons, turn: 2, phase: GamePhase::Main })
            if reasons == &[PlayerLossReason::ZeroLife]
    ));
}

/// Trigger-settlement incompleteness stops the same ready-card loop as a supported loss.
#[test]
fn plotted_incomplete_resolution_preserves_later_ready_cards() {
    let (cards, mut game, mut census) = plotted_scenario("The Ring tempts you.", 3, true);
    play_turn(&cards, &mut game, &mut census, 2);
    assert_later_plotted_costs_untouched(&game, &census);
    assert_eq!(game.life, 2);
    assert!(game.resolution.trigger_limit_reached);
    assert!(matches!(
        game.completion,
        Some(GameCompletion::Incomplete {
            reason: IncompleteReason::TriggerSettlement {
                configured_limit: 1024,
                remaining_work: 1
            },
            turn: 2,
            phase: GamePhase::Main,
        })
    ));
}

/// A nonterminal first spell still allows the remaining plotted costs and rewards to resolve.
#[test]
fn nonterminal_plotted_spells_keep_all_legal_casts() {
    let (cards, mut game, mut census) = plotted_scenario("You gain 2 life.", 3, false);
    play_turn(&cards, &mut game, &mut census, 2);
    assert!(game.completion.is_none());
    assert!(game.plotted.is_empty());
    assert_eq!(game.spells_cast_this_turn, 3);
    assert_eq!(game.life, 10);
    assert_eq!(game.life_paid, 2);
    assert_eq!(game.life_gained, 9);
    assert_eq!(
        game.graveyard,
        [CardIdx(0), CardIdx(1), CardIdx(4), CardIdx(2)]
    );
    assert!(census.turn_observations[1].end_turn);
}

/// Mana production can finish the game, but its already paid life and tap remain committed.
#[test]
fn terminal_pool_build_cannot_plot_a_new_hand_card() {
    let cards = deck(&[
        row(
            "Life-paid mana",
            "{1}",
            "Artifact",
            "{T}, Pay 1 life: Add {C}.",
        ),
        row(
            "Plot candidate",
            "{100}",
            "Sorcery",
            "Plot {1}\nYou gain 3 life.",
        ),
    ]);
    assert_eq!(
        cards.cards[1]
            .keyword_abilities
            .plot
            .expect("parsed plot cost")
            .total(),
        1
    );
    for life in [1, 2] {
        let mut game = main_phase_state(vec![1], vec![]);
        game.life = life;
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        game.next_uid = 7;
        let mut census = TurnCensus::new(1, cards.cards.len());
        play_turn(&cards, &mut game, &mut census, 1);
        assert_eq!(game.life, life - 1);
        assert_eq!(game.life_paid, 1);
        assert!(game.battlefield[0].tapped);
        assert_eq!(game.spells_cast_this_turn, 0);
        assert!(game.graveyard.is_empty());
        if life == 1 {
            assert_eq!(game.hand, [CardIdx(1)]);
            assert!(game.plotted.is_empty());
            assert!(matches!(
                game.completion,
                Some(GameCompletion::PlayerLost { .. })
            ));
            assert!(!census.turn_observations[0].main);
        } else {
            assert!(game.hand.is_empty());
            assert_eq!(game.plotted, [(CardIdx(1), 1)]);
            assert!(game.completion.is_none());
            assert!(census.turn_observations[0].end_turn);
        }
    }
}

/// Set up a real opponent-dependent mana ability and its supported nonland-tap reward.
fn opponent_mana_game(
    cost: &str,
    colors: Option<Vec<String>>,
    active: bool,
    turn: u32,
) -> (SimDeck, GameState) {
    let cards = deck(&[
        named_row("Fellwar Stone"),
        row("Actual mana payment", cost, "Sorcery", ""),
        row(
            "Nonland mana reward",
            "{100}",
            "Enchantment",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        ),
    ]);
    assert!(cards.cards[0].unlocked_abilities(0).any(|ability| {
        ability.kind == SimAbilityKind::ManaActivated
            && ability
                .activation
                .as_ref()
                .is_some_and(|cost| cost.taps_source())
    }));
    assert!(cards.cards[2].flags.bonus_mana_on_nonland_tap);
    let mut game = main_phase_state(vec![1], vec![]);
    game.current_turn = turn;
    game.opponents.config.land_colors = colors;
    game.opponents.seats[0].active = active;
    game.opponents.seats[0].lands = u32::from(active);
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
    game.battlefield
        .push(new_perm_with(8, &cards, CardIdx(2), 0, false));
    game.next_uid = 8;
    (cards, game)
}

/// Native and leftover activations add both real mana and its bonus without inventing colors.
#[test]
fn opponent_mana_native_and_leftover_bonuses_keep_color_and_turn_limits() {
    for colors in [None, Some(vec!["G".to_string()]), Some(vec![String::new()])] {
        for active in [false, true] {
            for turn in [1, 2] {
                let known_colors = colors
                    .as_ref()
                    .map(|colors| colors.iter().any(|colors| !colors.is_empty()));
                let produces = active && known_colors.unwrap_or(turn >= 2);
                for leftover in [false, true] {
                    for cost in ["{2}", "{G}{G}", "{U}", "{C}"] {
                        let (cards, mut game) =
                            opponent_mana_game(cost, colors.clone(), active, turn);
                        let mut pool = ManaPool::default();
                        if leftover {
                            spend_leftover(&cards, &mut game, &mut pool, turn);
                        } else {
                            let source = game.battlefield[0].clone();
                            assert_eq!(
                                activate_mana_mode(&cards, &mut game, &source, &mut pool, turn),
                                produces
                            );
                        }
                        let expected = if produces { 2 } else { 0 };
                        assert_eq!(
                            pool.total(),
                            expected,
                            "{colors:?} active={active} t{turn} leftover={leftover}"
                        );
                        if colors.is_none() {
                            assert_eq!(pool.generic_credit, expected);
                            assert_eq!(pool.fixed, [0; 5]);
                            assert_eq!(pool.flexible, 0);
                            assert_eq!(pool.colorless, 0);
                            assert!(pool.lots.is_empty());
                        }
                        if produces {
                            assert!(game.battlefield[0].tapped);
                        } else if !active || known_colors == Some(false) {
                            assert!(!game.battlefield[0].tapped);
                        }
                        assert!(game.completion.is_none());
                        assert!(!game.infinite_mana_suspected);
                        let legal =
                            produces && (cost == "{2}" || colors.is_some() && cost == "{G}{G}");
                        assert_eq!(
                            try_cast_from_hand(
                                &cards,
                                &mut game,
                                &mut pool,
                                turn as usize,
                                CardIdx(1),
                                &mut Vec::new(),
                                &mut Vec::new(),
                                &mut [false; 5],
                            ),
                            legal.then_some(2)
                        );
                        assert_eq!(pool.total(), if legal { 0 } else { expected });
                        assert_eq!(game.graveyard.contains(&CardIdx(1)), legal);
                    }
                }
            }
        }
    }
}
