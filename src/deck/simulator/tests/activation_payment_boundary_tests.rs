use super::*;
use crate::deck::simulator::card_test_support::assert_library_rng;
use crate::deck::simulator::game_effects::activation::{pick_best_activation, resolve_activation};
use crate::deck::simulator::model::{CardIdx, SimActivationCost, SimDeck};
use crate::deck::simulator::oracle_ast::{ActivationCost, ActivationTarget};
use crate::deck::simulator::oracle_parser::parse_oracle_activated_ability;
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use game::{GameCompletion, GamePhase, GameState, ManaPool, PlayerLossReason, new_perm_with};

/// Give a ready source real life, counters, cards, and a separate library random stream.
fn payment_scenario(text: &str) -> (SimDeck, GameState, ManaPool) {
    let cards = deck(&[
        row("Payment source", "{1}", "Artifact", text),
        row("First draw", "{99}", "Sorcery", ""),
        row("Second draw", "{99}", "Sorcery", ""),
        row("Retained hand", "{99}", "Sorcery", ""),
        row("Prior graveyard", "{99}", "Sorcery", ""),
        row("Prior exile", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![3], vec![2, 1]);
    game.current_turn = 1;
    game.current_phase = GamePhase::Main;
    game.life = 4;
    game.player_counters.energy = 4;
    game.graveyard.push(CardIdx(4));
    game.exile.push(CardIdx(5));
    let mut source = new_perm_with(11, &cards, CardIdx(0), 0, false);
    source.counters.charge = 3;
    game.battlefield.push(source);
    game.next_uid = 11;
    let pool = ManaPool {
        colorless: 2,
        fixed: [0, 0, 0, 1, 0],
        ..ManaPool::default()
    };
    (cards, game, pool)
}

/// Rejection cannot spend partial costs, change card access, or advance randomness.
fn assert_unchanged(game: &GameState, before: &GameState) {
    assert_eq!(game.life, before.life);
    assert_eq!(game.life_paid, before.life_paid);
    assert_eq!(game.life_gained, before.life_gained);
    assert_eq!(game.life_funded_draws, before.life_funded_draws);
    assert_eq!(game.player_counters, before.player_counters);
    assert_eq!(game.hand, before.hand);
    assert_eq!(game.library, before.library);
    assert_eq!(game.graveyard, before.graveyard);
    assert_eq!(game.exile, before.exile);
    assert_eq!(
        format!("{:?}", game.battlefield),
        format!("{:?}", before.battlefield)
    );
    assert_eq!(game.card_first_seen, before.card_first_seen);
    assert_eq!(game.battlefield_seen, before.battlefield_seen);
    assert_eq!(game.graveyard_seen, before.graveyard_seen);
    assert_eq!(game.graveyard_generations, before.graveyard_generations);
    assert_eq!(game.seen, before.seen);
    assert_eq!(game.awareness_cards, before.awareness_cards);
    assert_eq!(game.activated_once, before.activated_once);
    assert_eq!(game.activated_this_turn, before.activated_this_turn);
    assert_eq!(game.triggered_this_turn, before.triggered_this_turn);
    assert_eq!(game.milestones_by_turn, before.milestones_by_turn);
    assert_eq!(game.next_uid, before.next_uid);
    assert_eq!(game.completion, before.completion);
    assert_eq!(game.failed_draw, before.failed_draw);
    assert_library_rng(&game.library_rng, &before.library_rng);
}

/// Separate fixed life clauses remain separate in syntax and pay one complete runtime total.
#[test]
fn repeated_life_activation_pays_all_three_life_before_drawing() {
    let text = "{T}, Pay 1 life, Pay 2 life: Draw a card.";
    let oracle = parse_oracle_activated_ability(text).expect("the activation is recognized");
    assert!(matches!(
        oracle.costs.as_slice(),
        [
            ActivationCost::Tap(ActivationTarget::Source),
            ActivationCost::PayLife(1),
            ActivationCost::PayLife(2),
        ]
    ));
    let (cards, mut game, mut pool) = payment_scenario(text);
    let before_pool = pool.clone();
    let expected_rng = game.library_rng.clone();
    let activation =
        pick_best_activation(&cards, &game, &pool, 1).expect("four life covers all three life");
    let costs = &activation
        .ability
        .activation
        .as_ref()
        .expect("the activation has costs")
        .costs;
    assert_eq!(
        costs,
        &[SimActivationCost::TapSource, SimActivationCost::PayLife(3)]
    );

    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);

    assert_eq!(game.life, 1);
    assert_eq!(game.life_paid, 3);
    assert_eq!(game.hand, [CardIdx(3), CardIdx(1)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.seen, 1);
    assert_eq!(game.life_funded_draws, 1);
    assert_eq!(pool, before_pool);
    assert!(game.completion.is_none());
    assert_library_rng(&game.library_rng, &expected_rng);
}

/// Both selection and stale resolution reject life shortfalls without paying any resource.
#[test]
fn repeated_life_activation_rejects_insufficient_total_life_without_changes() {
    let (cards, initial, initial_pool) =
        payment_scenario("{T}, Pay 1 life, Pay 2 life: Draw a card.");
    let activation = pick_best_activation(&cards, &initial, &initial_pool, 1)
        .expect("the initial payment is affordable");
    for life in [1, 2] {
        let mut game = initial.clone();
        game.life = life;
        let before = game.clone();
        let mut pool = initial_pool.clone();
        assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
        assert_unchanged(&game, &before);
        resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
        assert_unchanged(&game, &before);
        assert_eq!(pool, initial_pool);
    }
}

/// Paying the last three life commits the tap and payment but cannot grant a draw reward.
#[test]
fn repeated_life_activation_stops_before_rewards_when_it_uses_the_last_life() {
    let (cards, mut game, mut pool) = payment_scenario("{T}, Pay 1 life, Pay 2 life: Draw a card.");
    game.life = 3;
    let before_pool = pool.clone();
    let expected_rng = game.library_rng.clone();
    let activation = pick_best_activation(&cards, &game, &pool, 1)
        .expect("exact life can pay the complete cost");

    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);

    assert_eq!(game.life, 0);
    assert_eq!(game.life_paid, 3);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.hand, [CardIdx(3)]);
    assert_eq!(game.library, [CardIdx(2), CardIdx(1)]);
    assert_eq!(game.seen, 0);
    assert_eq!(game.life_funded_draws, 0);
    assert_eq!(
        game.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::ZeroLife],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert_eq!(pool, before_pool);
    assert_library_rng(&game.library_rng, &expected_rng);
}

/// Unknown life scales, conditions, and mixed fragments cannot leave a cheaper payable mode.
#[test]
fn incomplete_life_costs_remain_unknown_and_disable_the_whole_mode() {
    for payment in [
        "Pay X life",
        "Pay life equal to the number of creatures you control",
        "Pay 1 life for each creature you control",
        "Pay 1 life if you control a creature",
        "Pay 1 life unless you control a creature",
        "Pay 1 life and pay 2 life",
        "Pay 1 life or pay 2 life",
        "{T} Pay 1 life",
        "Pay {E} if you control a creature",
    ] {
        let text = format!("{{T}}, {payment}: Draw a card.");
        let oracle =
            parse_oracle_activated_ability(&text).expect("the cost retains its unknown part");
        assert!(oracle.costs.iter().any(|cost| matches!(cost, ActivationCost::Other(source) if source.eq_ignore_ascii_case(payment))), "{payment}: {:?}", oracle.costs);
        let (cards, game, pool) = payment_scenario(&text);
        let before = game.clone();
        assert!(
            pick_best_activation(&cards, &game, &pool, 1).is_none(),
            "{payment}"
        );
        assert_unchanged(&game, &before);
    }
}

/// Repeated discard and counter clauses stay typed, but no partial payment reaches runtime.
#[test]
fn repeated_discard_and_counter_costs_do_not_pay_only_the_first_clause() {
    for (payment, discard) in [
        ("Discard a card, Discard two cards", true),
        (
            "Remove a charge counter from this artifact, Remove one charge counter from this artifact",
            false,
        ),
    ] {
        let text = format!("{{T}}, {payment}: Draw a card.");
        let oracle =
            parse_oracle_activated_ability(&text).expect("the full repeated cost is recognized");
        if discard {
            assert!(matches!(
                oracle.costs.as_slice(),
                [
                    ActivationCost::Tap(_),
                    ActivationCost::Discard(1),
                    ActivationCost::Discard(2)
                ]
            ));
        } else {
            assert!(
                matches!(oracle.costs.as_slice(), [ActivationCost::Tap(_), ActivationCost::RemoveCounter { count: 1, kind: Some(first) }, ActivationCost::RemoveCounter { count: 1, kind: Some(second) }] if first == "charge" && second == "charge")
            );
        }
        let (cards, game, pool) = payment_scenario(&text);
        assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
    }
}

/// Unknown discard selectors and counter sources cannot be dropped after a known cost prefix.
#[test]
fn discard_and_counter_qualifiers_keep_the_complete_mode_inert() {
    for payment in [
        "Discard a card at random",
        "Discard X cards",
        "Remove a charge counter from another artifact",
        "Remove a charge counter from this artifact if you control a creature",
        "Remove a charge counter from this artifact and remove a charge counter from this artifact",
    ] {
        let text = format!("{{T}}, {payment}: Draw a card.");
        let oracle = parse_oracle_activated_ability(&text).expect("the full cost is retained");
        assert!(oracle.costs.iter().any(|cost| matches!(cost, ActivationCost::Other(source) if source.eq_ignore_ascii_case(payment))));
        let (cards, game, pool) = payment_scenario(&text);
        assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
    }
}

/// A compound loyalty payment must not become a payable life-only activation.
#[test]
fn compound_loyalty_costs_remain_unknown_and_reject_action_selection() {
    for payment in [
        "-1, Pay 1 life",
        "−1, Pay 1 life",
        "–1, Pay 1 life",
        "Pay 1 life, -1",
        "+1, Pay 1 life",
        "0, Pay 1 life",
        "-1, -1",
        "−1, –1",
        "+1, -1",
        "0, 0",
        "-1 if you control a creature, Pay 1 life",
    ] {
        let text = format!("{payment}: Draw a card.");
        let oracle =
            parse_oracle_activated_ability(&text).expect("the full activation is retained");
        assert!(
            oracle
                .costs
                .iter()
                .any(|cost| matches!(cost, ActivationCost::Other(_))),
            "{payment}: {:?}",
            oracle.costs
        );
        let (cards, game, pool) = payment_scenario(&text);
        let before = game.clone();
        let before_pool = pool.clone();
        assert!(
            pick_best_activation(&cards, &game, &pool, 1).is_none(),
            "{payment}"
        );
        assert_unchanged(&game, &before);
        assert_eq!(pool, before_pool);
    }
}

/// Compound loyalty components remain visible instead of disappearing beside a life cost.
#[test]
fn compound_loyalty_parser_retains_each_rejected_component() {
    for (payment, component) in [
        ("-1, Pay 1 life", "-1"),
        ("−1, Pay 1 life", "−1"),
        ("–1, Pay 1 life", "–1"),
        ("Pay 1 life, -1", "-1"),
        ("0, Pay 1 life", "0"),
    ] {
        let oracle = parse_oracle_activated_ability(&format!("{payment}: Draw a card."))
            .expect("the full activation is retained");
        assert!(
            oracle
                .costs
                .iter()
                .any(|cost| matches!(cost, ActivationCost::PayLife(1)))
        );
        assert!(
            oracle
                .costs
                .iter()
                .any(|cost| matches!(cost, ActivationCost::Other(source) if source == component))
        );
    }
    let oracle = parse_oracle_activated_ability("−1, -1: Draw a card.")
        .expect("both loyalty payments remain visible");
    assert!(
        matches!(oracle.costs.as_slice(), [ActivationCost::Other(first), ActivationCost::Other(second)] if first == "−1" && second == "-1")
    );
}

/// Nonzero sole loyalty abilities still pay their counters and fire once per turn.
#[test]
fn sole_nonzero_loyalty_activations_keep_their_payment_and_turn_limit() {
    for (payment, change) in [("+1", 1), ("-1", -1), ("−1", -1), ("–1", -1)] {
        let oracle = parse_oracle_activated_ability(&format!("{payment}: Draw a card."))
            .expect("sole loyalty payment");
        assert!(
            matches!(oracle.costs.as_slice(), [ActivationCost::Loyalty(amount)] if *amount == change)
        );
        let mut source = row(
            "Loyalty control",
            "{1}",
            "Planeswalker",
            &format!("{payment}: Draw a card."),
        );
        source.loyalty = Some("3".into());
        let cards = deck(&[
            source,
            row("First draw", "{99}", "Sorcery", ""),
            row("Second draw", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![], vec![2, 1]);
        game.current_turn = 1;
        game.current_phase = GamePhase::Main;
        game.battlefield
            .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
        let mut pool = ManaPool::default();
        let activation = pick_best_activation(&cards, &game, &pool, 1)
            .expect("the loyalty payment is affordable");
        resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
        assert_eq!(
            game.battlefield[0].counters.loyalty,
            if change > 0 { 4 } else { 2 }
        );
        assert!(game.battlefield[0].fired);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert_eq!(game.life_paid, 0);
        assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
        let before = game.clone();
        resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
        assert_unchanged(&game, &before);
    }
}

/// A zero loyalty cost still uses the source's one loyalty activation for the turn.
#[test]
fn sole_zero_loyalty_draw_keeps_its_once_per_turn_limit() {
    let mut source = row(
        "Zero loyalty control",
        "{1}",
        "Planeswalker",
        "0: Draw a card.",
    );
    source.loyalty = Some("3".into());
    let cards = deck(&[
        source,
        row("First draw", "{99}", "Sorcery", ""),
        row("Second draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![], vec![2, 1]);
    game.current_turn = 1;
    game.current_phase = GamePhase::Main;
    game.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &game, &pool, 1)
        .expect("zero loyalty is a legal loyalty payment");

    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);

    assert_eq!(game.battlefield[0].counters.loyalty, 3);
    assert!(game.battlefield[0].fired);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
    let before = game.clone();
    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
    assert_unchanged(&game, &before);
}

/// Prepare two printed loyalty modes and enough cards for a real next-turn reset.
fn sibling_loyalty_scenario(first: &str, second: &str) -> (SimDeck, GameState) {
    let mut source = row(
        "Sibling loyalty control",
        "{1}",
        "Planeswalker",
        &format!("{first}: Draw a card.\n{second}: Draw a card."),
    );
    source.loyalty = Some("3".into());
    let cards = deck(&[
        source,
        row("First draw", "{99}", "Sorcery", ""),
        row("Second draw", "{99}", "Sorcery", ""),
        row("Third draw", "{99}", "Sorcery", ""),
        row("Fourth draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![], vec![4, 3, 2, 1]);
    game.current_turn = 1;
    game.current_phase = GamePhase::Main;
    game.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    game.next_uid = 1;
    (cards, game)
}

/// Zero and nonzero modes share one permanent-wide limit, which normal turn cleanup resets.
#[test]
fn sibling_loyalty_modes_share_the_turn_limit_and_reset_next_turn() {
    for (first, second, expected_loyalty) in [("0", "+1", 3), ("+1", "0", 5), ("-1", "0", 1)] {
        let (cards, mut game) = sibling_loyalty_scenario(first, second);
        let mut pool = ManaPool::default();
        let first_action = pick_best_activation(&cards, &game, &pool, 1)
            .expect("the first printed loyalty mode is selected");
        let mut sibling_action = pick_best_activation(&cards, &game, &pool, 1)
            .expect("the sibling can be prepared before either loyalty payment");
        sibling_action.ability = game::permanent_abilities(&cards, &game.battlefield[0])
            .find(|ability| ability.id != first_action.ability.id)
            .expect("the other printed loyalty mode is available before payment")
            .clone();

        resolve_activation(&cards, &mut game, &mut pool, 1, &first_action);

        assert_eq!(game.hand, [CardIdx(1)]);
        assert!(game.battlefield[0].fired);
        assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
        let before = game.clone();
        resolve_activation(&cards, &mut game, &mut pool, 1, &sibling_action);
        assert_unchanged(&game, &before);
        assert_eq!(pool.total(), 0);

        let mut census = game_run::TurnCensus::new(2, cards.cards.len());
        let commander = game_commander::CommanderProfile::new(&cards);
        game_run::run_turn(
            &cards,
            &mut game,
            &mut census,
            &commander,
            &mut Vec::new(),
            2,
            false,
        );

        assert_eq!(game.current_turn, 2);
        assert_eq!(game.hand, [CardIdx(1), CardIdx(2), CardIdx(3)]);
        assert_eq!(game.library, [CardIdx(4)]);
        assert_eq!(game.battlefield[0].counters.loyalty, expected_loyalty);
        assert!(game.battlefield[0].fired);
        assert!(game.completion.is_none());
        assert!(census.turn_observations[1].end_turn);
    }
}

/// A zero mana activation has no loyalty limit, even when another action used the fired flag.
#[test]
fn zero_mana_activation_does_not_gain_a_loyalty_turn_limit() {
    let (cards, mut game, mut pool) = payment_scenario("{0}: Add {C}.");
    for expected in [4, 5] {
        let action = pick_best_activation(&cards, &game, &pool, 1)
            .expect("an ordinary zero mana mode remains available");
        assert!(
            !action
                .ability
                .activation
                .as_ref()
                .expect("typed activation costs")
                .has_loyalty_cost()
        );
        resolve_activation(&cards, &mut game, &mut pool, 1, &action);
        assert_eq!(pool.total(), expected);
        if expected == 4 {
            assert!(!game.battlefield[0].fired);
            game.battlefield[0].fired = true;
        }
    }
    assert_eq!(game.life_paid, 0);
    assert!(game.completion.is_none());
}
