//! Synthetic work-limit and mana-growth evidence. These are not source-card certificates.

use super::*;
use crate::deck::simulator::game::{GameCompletion, GamePhase, IncompleteReason};
use crate::deck::simulator::game_effects::activation::spend_leftover;
use crate::deck::simulator::model::MAX_LOOP_PASSES;
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};

/// Reduce actual turns into the current production report, including incomplete games.
fn run_report(cards: &SimDeck, game: GameState) -> (GameState, GameLog, Value) {
    let (game, log) = execute_turns(cards, game, 1, vec![]);
    let json =
        serde_json::to_value(report_log(cards, &log, 1)).expect("production report serializes");
    (game, log, json)
}

/// Incomplete observations retain diagnostics but cannot supply finished-game mana rates.
fn assert_incomplete_report(log: &GameLog, json: &Value, reason: IncompleteReason, positive: bool) {
    assert_eq!(
        log.completion,
        GameCompletion::Incomplete {
            reason,
            turn: 1,
            phase: GamePhase::Main
        }
    );
    assert_eq!(log.turns_observed, 0);
    assert!(!log.turn_observations[0].main && !log.turn_observations[0].end_turn);
    assert_eq!(log.infinite_mana_suspected, positive);
    assert_eq!(log.milestones_by_turn[0].positive_mana_loop, positive);
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 0);
    assert_eq!(json["outcomes"]["player_loss_games"], 0);
    assert_eq!(json["outcomes"]["incomplete_games"], 1);
    let observations = json["outcomes"]["incomplete_observations"]
        .as_array()
        .expect("cap observations");
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0]["reason"],
        serde_json::to_value(reason).expect("reason serializes")
    );
    assert_eq!(observations[0]["turn"], 1);
    assert_eq!(observations[0]["phase"], "main");
    assert_eq!(observations[0]["games"], 1);
    for key in [
        "avg_available_by_turn",
        "avg_cast_spent_by_turn",
        "avg_unused_by_turn",
    ] {
        assert!(
            json["mana"][key]
                .as_object()
                .expect("turn values")
                .is_empty()
        );
    }
    assert!(json["draw"]["avg_life_paid"].is_null());
    assert_eq!(json["phase_observations"]["main_by_turn"]["1"], 0.0);
    assert_eq!(json["phase_observations"]["end_turn_by_turn"]["1"], 0.0);
    let unavailable = &json["sampling"]["opening_hand.percent_0_lands"];
    assert_eq!(unavailable["status"], "no_observations");
    assert_eq!(unavailable["observed_games"], 0);
    assert!(unavailable["event_games"].is_null() && unavailable["percent"].is_null());
    assert_eq!(
        json["win_conditions"]["percent_games_with_suspected_infinite_mana"],
        if positive { 100.0 } else { 0.0 }
    );
    assert_eq!(
        json["milestones"]["percent_games_with_positive_mana_loop_by_turn"]["1"],
        if positive { 100.0 } else { 0.0 }
    );
    for path in [
        "win_conditions.percent_games_with_suspected_infinite_mana",
        "milestones.percent_games_with_positive_mana_loop_by_turn.1",
    ] {
        assert_eq!(json["sampling"][path]["status"], "observed");
        assert_eq!(json["sampling"][path]["observed_games"], 1);
        assert_eq!(json["sampling"][path]["event_games"], u32::from(positive));
    }
}

/// Leave one older identity land available when a late grant reaches the final main pass.
fn main_work_scenario(rocks: u32, provider: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic older identity land",
            "",
            "Land",
            "{T}: Add one mana of any color in your commander's color identity.",
        ),
        row("Wastes", "", "Basic Land - Wastes", "{T}: Add {C}."),
        row(
            "Synthetic one-mana work rock",
            "{1}",
            "Artifact",
            "{T}: Add {C}.",
        ),
        row(
            "Synthetic final-pass land grant",
            "{1}",
            "Artifact",
            "Lands you control have \"{T}: Add one mana of any color.\"",
        ),
        row("Synthetic pending green work", "{G}", "Sorcery", ""),
        row("Retained main-work library", "{99}", "Sorcery", ""),
    ]);
    assert_eq!(cards.commander_color_identity(), [false; 5]);
    let mut hand = vec![2; rocks as usize];
    if provider {
        hand.push(3);
    }
    hand.push(4);
    let mut game = main_phase_state(hand, vec![5]);
    put_permanents(&cards, &mut game, &[0, 1]);
    (cards, game)
}

/// A real main cap distinguishes pending granted work from an unpayable hand or finished chain.
pub(super) fn assert_granted_main_work_report() {
    for (rocks, provider, capped) in [
        (MAX_LOOP_PASSES - 1, true, true),
        (MAX_LOOP_PASSES - 1, false, false),
        (MAX_LOOP_PASSES - 3, true, false),
    ] {
        let (cards, game) = main_work_scenario(rocks, provider);
        let rng = game.library_rng.clone();
        let (game, log, json) = run_report(&cards, game);
        let green_cast = provider && !capped;
        let spent = rocks + u32::from(provider) + u32::from(green_cast);
        assert_eq!(game.spells_cast_this_turn, spent);
        assert_eq!(log.mana_spent, [f64::from(spent)]);
        assert_eq!(log.mana_available, [1.0]);
        assert_eq!(
            game.hand,
            if green_cast { vec![] } else { vec![CardIdx(4)] }
        );
        assert_eq!(
            game.graveyard,
            if green_cast { vec![CardIdx(4)] } else { vec![] }
        );
        assert_eq!(game.library, [CardIdx(5)]);
        assert!(game.exile.is_empty());
        assert_eq!(
            game.battlefield.len(),
            2 + rocks as usize + usize::from(provider)
        );
        assert_eq!(game.next_uid, 2 + rocks + u32::from(provider));
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(game.battlefield[0].tapped, green_cast);
        assert!(
            game.battlefield[1..2 + rocks as usize]
                .iter()
                .all(|source| source.tapped)
        );
        if provider {
            let grant = game.battlefield.last().expect("cast grant remains");
            assert_eq!(grant.card.deck_idx(), Some(CardIdx(3)));
            assert_eq!(grant.entered_turn, 1);
            assert!(!grant.tapped);
        }
        if capped {
            assert_incomplete_report(
                &log,
                &json,
                IncompleteReason::TurnAction {
                    configured_limit: MAX_LOOP_PASSES,
                    remaining_work: 1,
                },
                false,
            );
        } else {
            assert_eq!(log.completion, GameCompletion::HorizonReached);
            assert_mana_report(&log, &json, &[1.0], &[f64::from(spent)]);
            assert_eq!(log.mana_remaining, [f64::from(u32::from(!provider))]);
            assert!(!log.infinite_mana_suspected);
        }
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Provide a supported native tap/untap profile and an optional independent land grant.
fn loop_scenario(text: &str, granted: bool, tapped: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row("Synthetic loop work land", "", "Land", text),
        row(
            "Synthetic loop tap grant",
            "{99}",
            "Enchantment",
            "Lands you control have \"{T}: Add one mana of any color.\"",
        ),
    ]);
    let mut game = main_phase_state(vec![], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    game.battlefield[0].tapped = tapped;
    if granted {
        put_permanents(&cards, &mut game, &[1]);
    }
    (cards, game)
}

/// Exact committed pool changes determine the diagnostic, not reaching the work cap alone.
fn assert_loop_action(
    text: &str,
    granted: bool,
    tapped: bool,
    capped: bool,
    positive: bool,
    mana: u32,
) {
    let (cards, mut game) = loop_scenario(text, granted, tapped);
    let before = game.clone();
    let mut pool = ManaPool::default();
    spend_leftover(&cards, &mut game, &mut pool, 1);
    assert_eq!(
        pool,
        ManaPool {
            flexible: if granted { mana } else { 0 },
            colorless: if granted { 0 } else { mana },
            ..ManaPool::default()
        }
    );
    assert_eq!(game.infinite_mana_suspected, positive);
    assert_eq!(
        game.milestones_by_turn
            .get(&1)
            .is_some_and(|row| row.positive_mana_loop),
        positive
    );
    if capped {
        assert_eq!(
            game.completion,
            Some(GameCompletion::Incomplete {
                reason: IncompleteReason::ActivationPass {
                    configured_limit: MAX_LOOP_PASSES,
                    remaining_work: 1
                },
                turn: 1,
                phase: GamePhase::Main
            })
        );
        assert!(!game.battlefield[0].tapped);
    } else {
        assert!(game.completion.is_none());
        assert_eq!(game.battlefield[0].tapped, tapped || mana > 0);
    }
    assert_eq!(game.battlefield.len(), before.battlefield.len());
    assert_eq!(game.battlefield[0].uid, before.battlefield[0].uid);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_paid, 0);
    assert!(
        game.hand.is_empty()
            && game.library.is_empty()
            && game.graveyard.is_empty()
            && game.exile.is_empty()
    );
    assert_library_rng(&game.library_rng, &before.library_rng);
}

/// Actual turn reports keep incomplete-loop diagnostics on their declared all-game denominator.
fn assert_loop_report(text: &str, granted: bool, capped: bool, positive: bool) {
    let (cards, game) = loop_scenario(text, granted, false);
    let rng = game.library_rng.clone();
    let (game, log, json) = run_report(&cards, game);
    assert_eq!(game.infinite_mana_suspected, positive);
    if capped {
        assert_incomplete_report(
            &log,
            &json,
            IncompleteReason::ActivationPass {
                configured_limit: MAX_LOOP_PASSES,
                remaining_work: 1,
            },
            positive,
        );
    } else {
        assert_eq!(log.completion, GameCompletion::HorizonReached);
        assert_eq!(json["outcomes"]["finished_games"], 1);
        assert_eq!(json["outcomes"]["incomplete_games"], 0);
        assert_eq!(
            json["win_conditions"]["percent_games_with_suspected_infinite_mana"],
            0.0
        );
    }
    assert_library_rng(&game.library_rng, &rng);
}

/// A finite granted batch can hit a real activation cap without being a positive loop.
fn assert_finite_grant_report() {
    let cards = deck(&[
        row("Synthetic finite old land", "", "Land", ""),
        row(
            "Synthetic newly cast finite grant",
            "{0}",
            "Enchantment",
            "Lands you control have \"{T}: Add one mana of any color.\"",
        ),
    ]);
    let mut game = main_phase_state(vec![1], vec![]);
    for _ in 0..=MAX_LOOP_PASSES {
        put_permanents(&cards, &mut game, &[0]);
    }
    let rng = game.library_rng.clone();
    let (game, log, json) = run_report(&cards, game);
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_eq!(game.battlefield.len(), MAX_LOOP_PASSES as usize + 2);
    assert_eq!(
        game.battlefield
            .iter()
            .filter(|source| source.tapped)
            .count(),
        MAX_LOOP_PASSES as usize
    );
    assert!(!game.battlefield[MAX_LOOP_PASSES as usize].tapped);
    assert!(!game.battlefield.last().expect("new grant remains").tapped);
    assert!(game.hand.is_empty() && game.graveyard.is_empty());
    assert_incomplete_report(
        &log,
        &json,
        IncompleteReason::ActivationPass {
            configured_limit: MAX_LOOP_PASSES,
            remaining_work: 1,
        },
        false,
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Mix two one-shot sources with a native tap/untap cycle and an optional external grant.
fn mixed_prefix_scenario(
    granted: bool,
    growing: bool,
    late_provider: bool,
) -> (SimDeck, GameState) {
    let finite_text = if granted {
        ""
    } else if late_provider {
        "{T}: Add {C}. Activate only if you control three or more artifacts."
    } else {
        "{T}: Add {C}."
    };
    let cards = deck(&[
        row("Synthetic finite-prefix land", "", "Land", finite_text),
        row(
            "Synthetic mixed-cycle land",
            "",
            "Land",
            if growing {
                "{T}: Add {C}.\n{0}: Untap this land."
            } else {
                "{T}: Add {C}.\n{1}: Untap this land."
            },
        ),
        row(
            "Synthetic mixed-prefix provider",
            "{0}",
            if granted { "Enchantment" } else { "Artifact" },
            if granted {
                "Lands you control have \"{T}: Add one mana of any color.\""
            } else {
                ""
            },
        ),
        row("Retained mixed-prefix library", "{99}", "Sorcery", ""),
        row("Retained mixed-prefix graveyard", "{99}", "Sorcery", ""),
        row("Retained mixed-prefix exile", "{99}", "Sorcery", ""),
    ]);
    let hand = if late_provider {
        vec![2; if granted { 1 } else { 3 }]
    } else {
        vec![]
    };
    let mut game = main_phase_state(hand, vec![3]);
    game.graveyard.push(CardIdx(4));
    game.exile.push(CardIdx(5));
    put_permanents(&cards, &mut game, &[0, 0, 1]);
    if granted && !late_provider {
        put_permanents(&cards, &mut game, &[2]);
    }
    (cards, game)
}

/// Real one-mana payments restore the cycle's readiness without increasing the finite prefix.
fn assert_paid_neutral_mixed_cycle(granted: bool) {
    use crate::deck::simulator::game_effects::activation::{
        pick_best_activation, resolve_activation,
    };

    let (cards, mut game) = mixed_prefix_scenario(granted, false, false);
    let mut pool = ManaPool::default();
    let initial_sources = game.battlefield[..3].to_vec();
    for source in initial_sources {
        assert!(activate_mana_mode(&cards, &mut game, &source, &mut pool, 1));
    }
    assert_eq!(pool.total(), 3);
    let untap =
        pick_best_activation(&cards, &game, &pool, 1).expect("paid native untap is available");
    assert_eq!(untap.uid, 3);
    assert_eq!(untap.ability_cost, parse_cost("{1}"));
    resolve_activation(&cards, &mut game, &mut pool, 1, &untap);
    assert_eq!(pool.total(), 2);
    let ready = game.clone();
    let ready_pool = pool.clone();
    for _ in 0..2 {
        let source = game.battlefield[2].clone();
        assert!(activate_mana_mode(&cards, &mut game, &source, &mut pool, 1));
        assert_eq!(pool.total(), 3);
        let untap = pick_best_activation(&cards, &game, &pool, 1)
            .expect("the next native untap still costs one");
        assert_eq!(untap.ability_cost, parse_cost("{1}"));
        resolve_activation(&cards, &mut game, &mut pool, 1, &untap);
        assert_eq!(pool, ready_pool);
        assert_resources_unchanged(&game, &ready);
    }
}

/// One-shot prefix mana cannot make a neutral repeated cycle look positive at the action cap.
fn assert_mixed_prefix_action(granted: bool, growing: bool) {
    let (cards, mut game) = mixed_prefix_scenario(granted, growing, false);
    let rng = game.library_rng.clone();
    let mut pool = ManaPool::default();
    spend_leftover(&cards, &mut game, &mut pool, 1);
    let expected = 2 + if growing {
        (MAX_LOOP_PASSES - 2) / 2
    } else {
        0
    };
    assert_eq!(
        pool,
        ManaPool {
            flexible: if granted { expected } else { 0 },
            colorless: if granted { 0 } else { expected },
            ..ManaPool::default()
        }
    );
    assert!(game.battlefield[0].tapped && game.battlefield[1].tapped);
    assert!(!game.battlefield[2].tapped);
    assert_eq!(game.battlefield[2].uid, 3);
    assert_eq!(game.battlefield.len(), 3 + usize::from(granted));
    assert!(game.hand.is_empty());
    assert_eq!(game.library, [CardIdx(3)]);
    assert_eq!(game.graveyard, [CardIdx(4)]);
    assert_eq!(game.exile, [CardIdx(5)]);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_paid, 0);
    assert_eq!(
        game.completion,
        Some(GameCompletion::Incomplete {
            reason: IncompleteReason::ActivationPass {
                configured_limit: MAX_LOOP_PASSES,
                remaining_work: 1
            },
            turn: 1,
            phase: GamePhase::Main
        })
    );
    assert_eq!(
        game.infinite_mana_suspected, growing,
        "granted={granted} growing={growing}: finite-prefix mana is not cycle growth"
    );
    assert_eq!(
        game.milestones_by_turn
            .get(&1)
            .is_some_and(|row| row.positive_mana_loop),
        growing
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Late providers put finite prefix mana inside the actual activation pass being reported.
fn assert_mixed_prefix_report(granted: bool, growing: bool) {
    let (cards, game) = mixed_prefix_scenario(granted, growing, true);
    assert_eq!(cards.cards[0].flags.requires_metalcraft, !granted);
    let rng = game.library_rng.clone();
    let (game, log, json) = run_report(&cards, game);
    let providers = if granted { 1 } else { 3 };
    assert_eq!(game.spells_cast_this_turn, providers);
    assert_eq!(log.mana_available, [1.0]);
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(game.battlefield.len(), 3 + providers as usize);
    assert_eq!(game.next_uid, 3 + providers);
    assert!(game.battlefield[..3].iter().all(|source| source.tapped));
    assert!(
        game.battlefield[3..]
            .iter()
            .all(|source| !source.tapped && source.entered_turn == 1)
    );
    assert!(game.hand.is_empty());
    assert_eq!(game.library, [CardIdx(3)]);
    assert_eq!(game.graveyard, [CardIdx(4)]);
    assert_eq!(game.exile, [CardIdx(5)]);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_paid, 0);
    assert_incomplete_report(
        &log,
        &json,
        IncompleteReason::ActivationPass {
            configured_limit: MAX_LOOP_PASSES,
            remaining_work: 1,
        },
        growing,
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Supported native and granted loops report growth without treating diagnostics as completion.
pub(super) fn assert_granted_positive_loop_report() {
    for granted in [true, false] {
        assert_paid_neutral_mixed_cycle(granted);
        for growing in [false, true] {
            assert_mixed_prefix_action(granted, growing);
            assert_mixed_prefix_report(granted, growing);
        }
    }
    let positive = "{T}: Add {C}.\n{0}: Untap this land.";
    let neutral = "{T}: Add {C}.\n{1}: Untap this land.";
    for granted in [false, true] {
        assert_loop_action(positive, granted, false, true, true, MAX_LOOP_PASSES / 2);
        assert_loop_report(positive, granted, true, true);
    }
    for granted in [false, true] {
        assert_loop_action(neutral, granted, false, true, false, 0);
        assert_loop_report(neutral, granted, true, false);
        assert_loop_action("{T}: Add {C}.", granted, false, false, false, 1);
        assert_loop_action("{T}: Add {C}.", granted, true, false, false, 0);
        assert_loop_report("{T}: Add {C}.", granted, false, false);
    }
    assert_loop_action("", false, false, false, false, 0);
    assert_loop_report("", false, false, false);
    assert_finite_grant_report();
}
