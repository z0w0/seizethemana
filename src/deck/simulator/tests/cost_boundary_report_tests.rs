//! Cost-boundary evidence uses real actions, turn censuses, and reports.
//!
//! Only the Metamorph callback binds a pinned card. Other callbacks are synthetic
//! engine checks. A typed dual-sacrifice fixture does not certify its unsupported
//! Oracle grammar. No callback certifies a whole card.

use super::card_cost_claim_tests::{assert_mana_report, assert_unchanged, printed_activation};
use super::card_runtime_tests::attempt_index;
use super::card_test_support::{
    assert_library_rng, main_phase_state as state, named_row, pinned_revision,
};
use super::game::{
    GameCompletion, GameLog, GamePhase, GameState, ManaPool, PlayerLossReason, new_token_perm,
};
use super::game_effects::activation::{pick_best_activation, resolve_activation};
use super::game_run::mana::{activate_mana_mode, build_pool};
use super::library_core_report_tests::{assert_report_clause, put_permanents};
use super::model::{CardIdx, SimActivationCost, SimDeck};
use super::permanent_core_report_tests::{execute_turns, report_log};
use super::scenario_test_support::{deck, row};
use serde_json::Value;

/// Preserve production outcome and phase counts while exposing the public JSON contract.
pub(super) fn execute_cost_report(
    cards: &SimDeck,
    game: GameState,
    turns: usize,
) -> (GameState, GameLog, Value) {
    let (game, log) = execute_turns(cards, game, turns, vec![]);
    let json = serde_json::to_value(report_log(cards, &log, turns as u32))
        .expect("cost report serializes");
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    (game, log, json)
}

/// A main-phase loss counts as finished but cannot supply completed-main or end-turn means.
pub(super) fn assert_cost_loss_report(log: &GameLog, json: &Value, life_paid: u32) {
    assert_eq!(
        log.completion,
        GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::ZeroLife],
            turn: 1,
            phase: GamePhase::Main
        }
    );
    assert_eq!(log.turns_observed, 0);
    assert!(log.turn_observations[0].draw);
    assert!(!log.turn_observations[0].main);
    assert!(!log.turn_observations[0].combat);
    assert!(!log.turn_observations[0].end_turn);
    assert_eq!(json["outcomes"]["player_loss_games"], 1);
    assert_eq!(json["phase_observations"]["draw_by_turn"]["1"], 1.0);
    assert_eq!(json["phase_observations"]["main_by_turn"]["1"], 0.0);
    assert_eq!(json["phase_observations"]["end_turn_by_turn"]["1"], 0.0);
    assert_eq!(json["mana"]["avg_available_by_turn"], serde_json::json!({}));
    assert_eq!(
        json["mana"]["avg_cast_spent_by_turn"],
        serde_json::json!({})
    );
    assert_eq!(json["mana"]["avg_unused_by_turn"], serde_json::json!({}));
    assert_eq!(json["velocity"]["avg_cards_seen"], serde_json::json!({}));
    assert_eq!(json["draw"]["avg_life_paid"], f64::from(life_paid));
}

/// Bind the last-life printed-cost route to Metamorph without granting its copy effect.
pub(super) fn assert_metamorph_last_life_report() {
    let source = named_row("Phyrexian Metamorph");
    pinned_revision(&source);
    let cards = deck(&[
        source.clone(),
        row(
            "Metamorph boundary funding",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}{C}{U}.",
        ),
        row("Unexposed card", "{99}", "Sorcery", ""),
    ]);
    for life in [1, 2, 3] {
        let mut game = state(vec![0], vec![2]);
        game.life = life;
        put_permanents(&cards, &mut game, &[1]);
        let rng = game.library_rng.clone();
        let mut action = game.clone();
        let mut pool = build_pool(&cards, &mut action, 1);
        let before = action.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut action, &mut pool, CardIdx(0)),
            (life >= 2).then_some(3)
        );
        if life == 1 {
            assert_unchanged(&action, &before, &pool, &before_pool);
        } else {
            assert_eq!(pool.colorless, 0);
            assert_eq!(pool.fixed[1], 1);
            assert_eq!(action.life_paid, 2);
            assert_eq!(action.life, life - 2);
            assert_eq!(action.battlefield.len(), if life == 2 { 1 } else { 2 });
        }
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.library, [CardIdx(2)]);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.life_gained, 0);
        assert_eq!(game.seen, 0);
        assert_eq!(game.battlefield.len(), if life == 3 { 2 } else { 1 });
        assert_eq!(game.hand, if life == 1 { vec![CardIdx(0)] } else { vec![] });
        assert_eq!(log.mana_spent, [if life >= 2 { 3.0 } else { 0.0 }]);
        if life == 2 {
            assert_eq!(game.life, 0);
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_cost_loss_report(&log, &json, 2);
        } else {
            let spent = if life == 3 { 3.0 } else { 0.0 };
            assert_mana_report(&log, &json, 4.0, spent, 4.0 - spent);
            assert_eq!(json["outcomes"]["player_loss_games"], 0);
        }
        assert_report_clause(
            &json,
            &source,
            "You may have this creature enter as a copy",
            "unsupported",
            None,
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Give a synthetic spell a combined additional discard/life cost plus a Phyrexian pip.
fn combined_cost_scenario(life: i32, fodder: bool, unsupported: bool) -> (SimDeck, GameState) {
    let text = if unsupported {
        "As an additional cost to cast this spell, discard a card and tap an artifact. Draw a card."
    } else {
        "As an additional cost to cast this spell, discard a card and pay 2 life. Draw a card."
    };
    let cards = deck(&[
        row("Synthetic combined spell", "{1}{B/P}", "Sorcery", text),
        row("Combined cost funding", "{99}", "Artifact", "{T}: Add {C}."),
        row("Physical discard", "{99}", "Sorcery", ""),
        row("Undrawn bottom", "{99}", "Sorcery", ""),
        row("Physical reward", "{99}", "Sorcery", ""),
    ]);
    assert_eq!(
        cards.cards[0].spell_data.unsupported_additional_cost,
        unsupported
    );
    if !unsupported {
        assert_eq!(cards.cards[0].spell_data.additional_cost_discards, 1);
        assert_eq!(cards.cards[0].spell_data.additional_cost_life, 2);
    }
    let mut game = state(if fodder { vec![0, 2] } else { vec![0] }, vec![3, 4]);
    game.life = life;
    put_permanents(&cards, &mut game, &[1]);
    (cards, game)
}

/// Combined life and discard costs commit together or leave every required resource untouched.
pub(super) fn assert_combined_discard_life_report() {
    for (life, fodder, unsupported) in [
        (5, true, false),
        (3, true, false),
        (5, false, false),
        (5, true, true),
        (4, true, false),
    ] {
        let (cards, game) = combined_cost_scenario(life, fodder, unsupported);
        let accepted = life >= 4 && fodder && !unsupported;
        let terminal = accepted && life == 4;
        let rng = game.library_rng.clone();
        let mut action = game.clone();
        let mut pool = build_pool(&cards, &mut action, 1);
        let before = action.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut action, &mut pool, CardIdx(0)),
            accepted.then_some(1)
        );
        if !accepted {
            assert_unchanged(&action, &before, &pool, &before_pool);
        }
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.life, if accepted { life - 4 } else { life });
        assert_eq!(game.life_paid, if accepted { 4 } else { 0 });
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        assert_eq!(game.seen, u32::from(accepted && !terminal));
        assert!(game.exile.is_empty());
        assert_eq!(game.battlefield.len(), 1);
        assert!(game.battlefield[0].tapped);
        if terminal {
            assert_eq!(game.graveyard, [CardIdx(2)]);
            assert!(game.hand.is_empty());
            assert_eq!(game.library, [CardIdx(3), CardIdx(4)]);
            assert_cost_loss_report(&log, &json, 4);
        } else {
            assert_eq!(
                game.graveyard,
                if accepted {
                    vec![CardIdx(2), CardIdx(0)]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                game.hand,
                if accepted {
                    vec![CardIdx(4)]
                } else if fodder {
                    vec![CardIdx(0), CardIdx(2)]
                } else {
                    vec![CardIdx(0)]
                }
            );
            assert_eq!(
                game.library,
                if accepted {
                    vec![CardIdx(3)]
                } else {
                    vec![CardIdx(3), CardIdx(4)]
                }
            );
            let spent = f64::from(u32::from(accepted));
            assert_mana_report(&log, &json, 1.0, spent, 1.0 - spent);
            assert_eq!(
                json["draw"]["avg_life_paid"],
                if accepted { 4.0 } else { 0.0 }
            );
            assert_eq!(log.cards_seen, [u32::from(accepted)]);
        }
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Keep source exclusion, a real tapped victim, and later pending-work checks in one small deck.
fn sacrifice_scenario(another: bool, other: bool) -> (SimDeck, GameState) {
    let selector = if another {
        "another creature"
    } else {
        "a creature"
    };
    let cards = deck(&[
        row(
            "Synthetic sacrifice source",
            "{99}",
            "Creature",
            &format!("{{1}}, {{T}}, Pay 1 life, Sacrifice {selector}: Draw a card."),
        ),
        row("Sacrifice funding", "{99}", "Artifact", "{T}: Add {C}."),
        row(
            "Tapped payment creature",
            "{99}",
            "Creature",
            "{T}: Add {C}.",
        ),
        row("Retained hand card", "{99}", "Sorcery", ""),
        row("Undrawn bottom", "{99}", "Sorcery", ""),
        row("Physical draw", "{99}", "Sorcery", ""),
        row(
            "Pending draw listener",
            "{99}",
            "Artifact",
            "Whenever you cast a spell, draw a card.",
        ),
        row("Pending event spell", "{0}", "Sorcery", ""),
    ]);
    let mut game = state(vec![3], vec![4, 5]);
    // Inactive seats prevent combat from adding an unrelated tap to a rejected source.
    game.opponents.seats[0].active = false;
    put_permanents(&cards, &mut game, &[0, 1]);
    game.battlefield[0].summoning_sick = false;
    if other {
        put_permanents(&cards, &mut game, &[2]);
        game.battlefield[2].summoning_sick = false;
        game.battlefield[2].tapped = true;
    }
    (cards, game)
}

/// Real victims, including tapped bodies, pay once; only an unrestricted cost can use its source.
pub(super) fn assert_activation_sacrifice_exclusion_report() {
    for (another, other) in [(false, false), (true, false), (true, true)] {
        let (cards, game) = sacrifice_scenario(another, other);
        let accepted = !another || other;
        let mut action = game.clone();
        let mut pool = build_pool(&cards, &mut action, 1);
        let before = action.clone();
        let before_pool = pool.clone();
        let activation = printed_activation(&cards, &action, 0, 0);
        assert_eq!(
            pick_best_activation(&cards, &action, &pool, 1).is_some(),
            accepted
        );
        if other {
            assert!(action.battlefield[2].tapped);
        }
        resolve_activation(&cards, &mut action, &mut pool, 1, &activation);
        if !accepted {
            assert_unchanged(&action, &before, &pool, &before_pool);
        } else {
            assert_eq!(pool.total(), 0);
        }
        let rng = game.library_rng.clone();
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.life_paid, u32::from(accepted));
        assert_eq!(game.life, if accepted { 19 } else { 20 });
        assert_eq!(
            game.graveyard,
            if !accepted {
                vec![]
            } else if other {
                vec![CardIdx(2)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(
            game.hand,
            if accepted {
                vec![CardIdx(3), CardIdx(5)]
            } else {
                vec![CardIdx(3)]
            }
        );
        assert_eq!(
            game.library,
            if accepted {
                vec![CardIdx(4)]
            } else {
                vec![CardIdx(4), CardIdx(5)]
            }
        );
        assert_eq!(game.battlefield.len(), if !another { 1 } else { 2 });
        if another {
            assert_eq!(game.battlefield[0].tapped, accepted);
        }
        assert_eq!(game.life_funded_draws, u32::from(accepted));
        assert_eq!(log.cards_seen, [u32::from(accepted)]);
        assert_mana_report(
            &log,
            &json,
            1.0,
            0.0,
            if other || !accepted { 1.0 } else { 0.0 },
        );
        assert_eq!(
            json["draw"]["avg_life_paid"],
            f64::from(u32::from(accepted))
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// A token pays after its unique convoke tap, without becoming a graveyard card.
pub(super) fn assert_tapped_token_sacrifice_report() {
    let cards = deck(&[
        row(
            "Synthetic token sacrifice",
            "{99}",
            "Artifact",
            "{U}, {T}, Pay 1 life, Sacrifice another creature: Draw a card.",
        ),
        row(
            "Token funding",
            "{99}",
            "Artifact",
            "{T}: Add {U}. This mana can't be spent to pay generic mana costs.",
        ),
        row("Token tap payment", "{1}", "Sorcery", "Convoke"),
        row("Retained card", "{99}", "Sorcery", ""),
        row("Undrawn bottom", "{99}", "Sorcery", ""),
        row("Physical draw", "{99}", "Sorcery", ""),
    ]);
    for token in [false, true] {
        let mut game = state(vec![2, 3], vec![4, 5]);
        if token {
            game.next_uid = 1;
            game.battlefield.push(new_token_perm(1, 0));
        }
        put_permanents(&cards, &mut game, &[0, 1]);
        let rng = game.library_rng.clone();
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.battlefield.len(), 2);
        assert!(
            game.battlefield
                .iter()
                .all(|permanent| permanent.card.deck_idx().is_some())
        );
        assert_eq!(game.battlefield[0].tapped, token);
        assert!(game.battlefield[1].tapped);
        assert_eq!(
            game.graveyard,
            if token { vec![CardIdx(2)] } else { vec![] }
        );
        assert_eq!(
            game.hand,
            if token {
                vec![CardIdx(3), CardIdx(5)]
            } else {
                vec![CardIdx(2), CardIdx(3)]
            }
        );
        assert_eq!(
            game.library,
            if token {
                vec![CardIdx(4)]
            } else {
                vec![CardIdx(4), CardIdx(5)]
            }
        );
        assert_eq!(game.life, if token { 19 } else { 20 });
        assert_eq!(game.life_paid, u32::from(token));
        assert_eq!(game.spells_cast_this_turn, u32::from(token));
        assert_eq!(game.seen, u32::from(token));
        assert_mana_report(&log, &json, 1.0, 0.0, if token { 0.0 } else { 1.0 });
        assert_eq!(log.graveyard_size, [u32::from(token)]);
        assert_eq!(
            json["draw"]["avg_life_funded_draws"],
            f64::from(u32::from(token))
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// A typed runtime source-plus-creature cost needs distinct identities; unsupported text stays inert.
pub(super) fn assert_distinct_sacrifice_bundle_report() {
    for other in [false, true] {
        let (mut cards, game) = sacrifice_scenario(false, other);
        // This is a synthetic runtime bundle, not compiler support for two sacrifice clauses.
        cards.cards[0].striations[0].abilities[0]
            .activation
            .as_mut()
            .expect("typed cost")
            .costs
            .push(SimActivationCost::SacrificeSource);
        let mut action = game.clone();
        let mut pool = build_pool(&cards, &mut action, 1);
        let before = action.clone();
        let before_pool = pool.clone();
        let activation = printed_activation(&cards, &action, 0, 0);
        resolve_activation(&cards, &mut action, &mut pool, 1, &activation);
        if !other {
            assert_unchanged(&action, &before, &pool, &before_pool);
        }
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(
            game.graveyard,
            if other {
                vec![CardIdx(0), CardIdx(2)]
            } else {
                vec![]
            }
        );
        assert_eq!(game.battlefield.len(), if other { 1 } else { 2 });
        assert_eq!(game.life_paid, u32::from(other));
        assert_eq!(log.cards_seen, [u32::from(other)]);
        assert_mana_report(&log, &json, 1.0, 0.0, 1.0);
    }
    let cards = deck(&[
        row(
            "Synthetic unsupported sacrifice",
            "{99}",
            "Creature",
            "{1}, {T}, Pay 1 life, Sacrifice this creature, Sacrifice another creature: Draw a card.",
        ),
        row("Available victim", "{99}", "Creature", ""),
        row("Available mana", "{99}", "Artifact", "{T}: Add {C}."),
        row("Undrawn reward", "{99}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].unlocked_abilities(0).next().is_none());
    let mut game = state(vec![], vec![3]);
    game.opponents.seats[0].active = false;
    put_permanents(&cards, &mut game, &[0, 1, 2]);
    let (game, log, json) = execute_cost_report(&cards, game, 1);
    assert_eq!(game.battlefield.len(), 3);
    assert!(!game.battlefield[0].tapped && !game.battlefield[1].tapped);
    assert!(game.graveyard.is_empty() && game.hand.is_empty());
    assert_eq!(game.library, [CardIdx(3)]);
    assert_eq!(game.life, 20);
    assert_mana_report(&log, &json, 1.0, 0.0, 1.0);
}

/// A rejected activation leaves an actual deferred cast trigger available for later settlement.
pub(super) fn assert_failed_activation_pending_work_report() {
    let (cards, mut game) = sacrifice_scenario(true, false);
    put_permanents(&cards, &mut game, &[6]);
    game.hand.push(CardIdx(7));
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    super::resolution::begin_resolution(&mut game);
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(7)),
        Some(0)
    );
    assert_eq!(game.hand, [CardIdx(3)]);
    assert_eq!(game.graveyard, [CardIdx(7)]);
    let action = printed_activation(&cards, &game, 0, 0);
    let before = game.clone();
    let before_pool = pool.clone();
    resolve_activation(&cards, &mut game, &mut pool, 1, &action);
    assert_unchanged(&game, &before, &pool, &before_pool);
    super::resolution::end_resolution(&cards, &mut game, Some(&mut pool));
    assert_eq!(game.hand, [CardIdx(3), CardIdx(5)]);
    assert_eq!(game.library, [CardIdx(4)]);
    assert_eq!(game.seen, 1);
    let rng = game.library_rng.clone();
    let (game, log, json) = execute_cost_report(&cards, game, 1);
    assert_eq!(game.hand, [CardIdx(3), CardIdx(5)]);
    assert_eq!(game.graveyard, [CardIdx(7)]);
    assert_eq!(game.library, [CardIdx(4)]);
    assert!(!game.battlefield[0].tapped);
    assert_eq!(game.life_paid, 0);
    assert_eq!(log.cards_seen, [1]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 1.0);
    assert_mana_report(&log, &json, 1.0, 0.0, 1.0);
    assert_library_rng(&game.library_rng, &rng);
}

/// Paid colored and life costs belong only to their own mode, not its legal fallback.
pub(super) fn assert_colored_life_mana_mode_report() {
    for (blue, life, unsupported) in [
        (true, 3, false),
        (false, 3, false),
        (true, 1, false),
        (true, 3, true),
    ] {
        let cost = if unsupported {
            "{U}, {T}, Pay 2 life, Discard a card"
        } else {
            "{U}, {T}, Pay 2 life"
        };
        let cards = deck(&[
            row(
                "Synthetic paid modes",
                "{99}",
                "Artifact",
                &format!("{{T}}: Add {{C}}.\n{cost}: Add {{G}}{{G}}."),
            ),
            row(
                "External colored payment",
                "{99}",
                "Artifact",
                if blue {
                    "{T}: Add {U}."
                } else {
                    "{T}: Add {C}."
                },
            ),
            row("Output payment", "{G}{G}", "Sorcery", "Gain 1 life."),
            row("Unused discard candidate", "{99}", "Sorcery", ""),
        ]);
        let paid = blue && life >= 2 && !unsupported;
        let mut game = state(vec![2, 3], vec![]);
        game.life = life;
        put_permanents(&cards, &mut game, &[1, 0]);
        let mut direct = state(vec![2, 3], vec![]);
        direct.life = life;
        put_permanents(&cards, &mut direct, &[0]);
        let mut pool = ManaPool::default();
        if blue {
            pool.fixed[1] = 1;
        } else {
            pool.colorless = 1;
        }
        let source = direct.battlefield[0].clone();
        assert!(activate_mana_mode(
            &cards,
            &mut direct,
            &source,
            &mut pool,
            1
        ));
        assert_eq!(pool.fixed[4], if paid { 2 } else { 0 });
        assert_eq!(pool.fixed[1], u32::from(blue && !paid));
        assert_eq!(
            pool.colorless,
            if paid {
                0
            } else if blue {
                1
            } else {
                2
            }
        );
        assert_eq!(pool.total(), 2);
        assert!(direct.battlefield[0].tapped);
        assert_eq!(direct.life_paid, if paid { 2 } else { 0 });
        assert_eq!(direct.life, life - if paid { 2 } else { 0 });
        assert_eq!(direct.hand, [CardIdx(2), CardIdx(3)]);
        assert!(direct.graveyard.is_empty());
        let before = direct.clone();
        let before_pool = pool.clone();
        let source = direct.battlefield[0].clone();
        assert!(!activate_mana_mode(
            &cards,
            &mut direct,
            &source,
            &mut pool,
            1
        ));
        assert_unchanged(&direct, &before, &pool, &before_pool);
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.life_paid, if paid { 2 } else { 0 });
        assert_eq!(game.life, life - if paid { 1 } else { 0 });
        assert_eq!(game.life_gained, u32::from(paid));
        assert_eq!(
            game.hand,
            if paid {
                vec![CardIdx(3)]
            } else {
                vec![CardIdx(2), CardIdx(3)]
            }
        );
        assert_eq!(game.graveyard, if paid { vec![CardIdx(2)] } else { vec![] });
        assert_eq!(game.battlefield.len(), 2);
        assert!(game.battlefield.iter().all(|permanent| permanent.tapped));
        assert_mana_report(
            &log,
            &json,
            2.0,
            if paid { 2.0 } else { 0.0 },
            if paid { 0.0 } else { 2.0 },
        );
        assert_eq!(json["draw"]["avg_life_paid"], if paid { 2.0 } else { 0.0 });
    }
}

/// The printed Phyrexian life boundary reports loss before entry.
#[test]
fn pinned_metamorph_last_life_cost_report() {
    assert_metamorph_last_life_report();
}

/// Synthetic combined spell costs cannot borrow their later draw reward.
#[test]
fn synthetic_combined_discard_and_life_cost_report() {
    assert_combined_discard_life_report();
}

/// Synthetic source exclusion and real tapped victims reach reports.
#[test]
fn synthetic_activation_source_exclusion_and_tapped_payment_report() {
    assert_activation_sacrifice_exclusion_report();
}

/// Synthetic token payment retains its earlier physical convoke tap.
#[test]
fn synthetic_tapped_token_payment_report() {
    assert_tapped_token_sacrifice_report();
}

/// Synthetic runtime dual costs and unsupported grammar remain separate.
#[test]
fn synthetic_distinct_sacrifice_cost_report() {
    assert_distinct_sacrifice_bundle_report();
}

/// Synthetic failed payment cannot discard a pending production event.
#[test]
fn synthetic_failed_activation_retains_pending_work_report() {
    assert_failed_activation_pending_work_report();
}

/// Synthetic paid mana modes do not inherit costs or output from siblings.
#[test]
fn synthetic_colored_and_life_mana_mode_report() {
    assert_colored_life_mana_mode_report();
}
