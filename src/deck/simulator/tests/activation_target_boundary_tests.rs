//! Activated graveyard targets keep their pre-cost physical identity.
//!
//! Cauldron checks use its pinned source. Synthetic cases cover bundled costs,
//! target generations, linked effects, and terminal payment boundaries.
//! Nontargeted and qualified graveyard choices stay inert until their rules are represented.

use super::card_cost_claim_tests::{assert_unchanged, printed_activation};
use super::card_runtime_tests::renamed;
use super::card_test_support::{assert_library_rng, main_phase_state as state, named_row};
use super::game::{
    Activation, CardRef, GameCompletion, GamePhase, GameState, ManaPool, PlayerLossReason,
    new_token_perm,
};
use super::game_effects::activation::{pick_best_activation, resolve_activation, spend_leftover};
use super::game_effects::{CardZone, move_to_graveyard};
use super::library_core_report_tests::{
    assert_report_clause, cast_action, executed_report, put_permanents,
};
use super::model::{CardIdx, Cost, SimActivationCost, SimDeck, SimEffect};
use super::scenario_test_support::{deck, row};
use crate::db::CardRow;
use serde_json::Value;

/// Select the source's sole printed activated ability without bypassing its costs.
fn printed_action(cards: &SimDeck, game: &GameState) -> Activation {
    let index = game.battlefield[0]
        .card
        .deck_idx()
        .expect("physical activation source");
    let id = cards[index]
        .unlocked_abilities(game.battlefield[0].counters.charge)
        .find(|ability| ability.kind.is_activated())
        .expect("the printed activated ability lowered")
        .id;
    printed_activation(cards, game, 0, id)
}

/// Give Cauldron a distinct sacrifice, pre-cost target, and ordinary mana sources.
fn cauldron_scenario(source: &CardRow, target: Option<u32>) -> (SimDeck, GameState) {
    let mut sacrifice = row("Cauldron payment body", "{99}", "Creature", "");
    sacrifice.power = Some("0".into());
    sacrifice.toughness = Some("1".into());
    let cards = deck(&[
        source.clone(),
        sacrifice,
        row(
            "Cauldron pre-cost target",
            "{99}",
            "Creature",
            "This creature enters tapped.\nWhen this creature enters, gain 2 life.",
        ),
        row("Cauldron wrong target", "{99}", "Artifact", ""),
        row("Black payment", "", "Basic Land - Swamp", "{T}: Add {B}."),
        row("Green payment", "", "Basic Land - Forest", "{T}: Add {G}."),
        row(
            "Generic payment",
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        ),
        row("Undrawn card", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![], vec![7]);
    put_permanents(&cards, &mut game, &[0, 1]);
    if let Some(index) = target {
        move_to_graveyard(&cards, &mut game, CardIdx(index), 1, CardZone::Hand);
    }
    (cards, game)
}

/// Supply Cauldron's full activation payment without granting any cast payment.
fn cauldron_pool() -> ManaPool {
    ManaPool {
        colorless: 1,
        fixed: [0, 0, 1, 0, 1],
        ..Default::default()
    }
}

/// Cauldron returns its pre-cost target rather than the creature sacrificed to pay.
pub(super) fn assert_cauldron_pre_cost_activation(source: &CardRow) {
    for source in renamed(source) {
        for tapped_sacrifice in [false, true] {
            let (cards, mut game) = cauldron_scenario(&source, Some(2));
            game.battlefield[1].tapped = tapped_sacrifice;
            let mut pool = cauldron_pool();
            let rng = game.library_rng.clone();
            let action = pick_best_activation(&cards, &game, &pool, 1)
                .expect("the pre-cost target and full payment exist");
            assert_eq!(action.sacrifice_uid, Some(2));
            resolve_activation(&cards, &mut game, &mut pool, 1, &action);

            assert_eq!(pool.total(), 0);
            assert_eq!(game.graveyard, [CardIdx(1)]);
            assert_eq!(game.battlefield.len(), 2);
            assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
            assert!(game.battlefield[0].tapped);
            let target = &game.battlefield[1];
            assert_eq!(target.card, CardRef::Deck(CardIdx(2)));
            assert_eq!(target.uid, 3);
            assert_eq!(target.entered_turn, 1);
            assert!(target.tapped && target.summoning_sick);
            assert_eq!(target.counters.plus1, 0);
            assert_eq!(game.graveyard_generations[&CardIdx(1)], 1);
            assert_eq!(game.graveyard_generations[&CardIdx(2)], 1);
            assert_eq!(game.life, 23);
            assert_eq!(game.life_gained, 3);
            assert_eq!(game.life_paid, 0);
            assert_eq!(game.opponent_life_lost, 1);
            assert_eq!(game.library, [CardIdx(7)]);
            assert!(game.hand.is_empty() && game.exile.is_empty());
            assert_eq!(game.spells_cast_this_turn, 0);
            assert_eq!(game.replay_casts, 0);
            assert_eq!(game.seen, 0);
            assert!(game.completion.is_none());
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// Missing targets or payment resources cannot consume any part of Cauldron's cost.
pub(super) fn assert_cauldron_rejected_activation(source: &CardRow) {
    for target in [None, Some(3), Some(2)] {
        for missing_payment in [false, true] {
            if target == Some(2) && !missing_payment {
                continue;
            }
            let (cards, mut game) = cauldron_scenario(source, target);
            let mut pool = cauldron_pool();
            if missing_payment {
                pool.fixed[2] = 0;
                pool.colorless += 1;
            }
            let action = printed_action(&cards, &game);
            let before = game.clone();
            let before_pool = pool.clone();
            assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
            resolve_activation(&cards, &mut game, &mut pool, 1, &action);
            assert_unchanged(&game, &before, &pool, &before_pool);
        }
    }
    for missing_creature in [false, true] {
        let (cards, mut game) = cauldron_scenario(source, Some(2));
        if missing_creature {
            game.battlefield.pop();
        } else {
            game.battlefield[0].tapped = true;
        }
        let action = printed_action(&cards, &game);
        let mut pool = cauldron_pool();
        let before = game.clone();
        let before_pool = pool.clone();
        assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
        resolve_activation(&cards, &mut game, &mut pool, 1, &action);
        assert_unchanged(&game, &before, &pool, &before_pool);
    }
}

/// Actual Cauldron turns report committed costs and preserve the target-selection limit.
pub(super) fn assert_cauldron_activation_boundary_report(source: &CardRow) {
    for target in [None, Some(3), Some(2)] {
        let (cards, mut game) = cauldron_scenario(source, target);
        put_permanents(&cards, &mut game, &[4, 5, 6]);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        let accepted = target == Some(2);
        assert_eq!(game.life, if accepted { 23 } else { 20 });
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.opponent_life_lost, u32::from(accepted));
        assert_eq!(game.battlefield[0].tapped, accepted);
        assert_eq!(game.battlefield.len(), 5);
        assert_eq!(
            game.graveyard,
            if accepted {
                vec![CardIdx(1)]
            } else {
                target.into_iter().map(CardIdx).collect()
            }
        );
        assert_eq!(game.library, [CardIdx(7)]);
        assert!(game.hand.is_empty() && game.exile.is_empty());
        assert_eq!(game.spells_cast_this_turn, 0);
        assert_eq!(log.mana_available, [3.0]);
        assert_eq!(log.mana_spent, [0.0]);
        let unused = if accepted { 0.0 } else { 3.0 };
        assert_eq!(log.mana_remaining, [unused]);
        assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], unused);
        assert_eq!(
            json["draw"]["avg_life_gained"],
            if accepted { 3.0 } else { 0.0 }
        );
        assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
        assert_report_clause(
            &json,
            source,
            "Return target creature card from your graveyard to the battlefield",
            "approximated",
            Some("pilot_card_selection"),
        );
        if accepted {
            let returned = game
                .battlefield
                .iter()
                .find(|permanent| permanent.card == CardRef::Deck(CardIdx(2)))
                .expect("the pre-cost target returned during the executed turn");
            assert!(returned.tapped && returned.summoning_sick);
            assert_eq!(log.card_first_battlefield.get(&2), Some(&1));
        }
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Build a synthetic return with a complete mana, life, tap, energy, and counter cost.
fn bundled_scenario(target_present: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic bundled source",
            "{99}",
            "Creature",
            "{1}, {T}, Pay 2 life, Pay {E}, Remove a charge counter from this creature, Sacrifice another creature: Return target creature card from your graveyard to the battlefield. Gain 5 life. Draw a card.",
        ),
        row("Synthetic payment creature", "{99}", "Creature", ""),
        row("Synthetic pre-cost target", "{99}", "Creature", ""),
        row("Synthetic reward draw", "{99}", "Sorcery", ""),
        row("Synthetic undrawn card", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![], vec![4, 3]);
    put_permanents(&cards, &mut game, &[0, 1]);
    game.battlefield[0].summoning_sick = false;
    game.battlefield[0].counters.charge = 2;
    game.player_counters.energy = 2;
    if target_present {
        move_to_graveyard(&cards, &mut game, CardIdx(2), 1, CardZone::Hand);
    }
    (cards, game)
}

/// Every bundled cost commits once before the pre-cost target and linked rewards resolve.
#[test]
fn activated_return_pays_complete_costs_before_resolving_its_linked_effects() {
    let (cards, mut game) = bundled_scenario(true);
    let rng = game.library_rng.clone();
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let action = pick_best_activation(&cards, &game, &pool, 1).expect("the whole action is legal");
    resolve_activation(&cards, &mut game, &mut pool, 1, &action);

    assert_eq!(pool.total(), 0);
    assert_eq!(game.life, 23);
    assert_eq!(game.life_paid, 2);
    assert_eq!(game.life_gained, 5);
    assert_eq!(game.life_funded_draws, 1);
    assert_eq!(game.player_counters.energy, 1);
    assert_eq!(game.battlefield.len(), 2);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.battlefield[0].counters.charge, 1);
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(2)));
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(game.hand, [CardIdx(3)]);
    assert_eq!(game.library, [CardIdx(4)]);
    assert_eq!(game.seen, 1);
    assert_eq!(game.next_uid, 3);
    assert_eq!(game.spells_cast_this_turn, 0);
    assert_library_rng(&game.library_rng, &rng);
}

/// A missing target or excluded source cannot consume any bundled resource.
#[test]
fn missing_pre_cost_target_and_missing_other_creature_preserve_the_full_bundle() {
    for target_present in [false, true] {
        let (cards, mut game) = bundled_scenario(target_present);
        if target_present {
            game.battlefield.pop();
        }
        let action = printed_action(&cards, &game);
        let mut pool = ManaPool {
            colorless: 1,
            ..Default::default()
        };
        let before = game.clone();
        let before_pool = pool.clone();
        assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
        resolve_activation(&cards, &mut game, &mut pool, 1, &action);
        assert_unchanged(&game, &before, &pool, &before_pool);
    }
}

/// Exact remaining life can pay the cost, but its terminal boundary grants no reward.
#[test]
fn terminal_activation_payment_commits_costs_without_return_draw_or_life_reward() {
    let (cards, mut game) = bundled_scenario(true);
    game.life = 2;
    let rng = game.library_rng.clone();
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let action = pick_best_activation(&cards, &game, &pool, 1).expect("exact life covers the cost");
    resolve_activation(&cards, &mut game, &mut pool, 1, &action);

    assert_eq!(
        game.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::ZeroLife],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert_eq!(pool.total(), 0);
    assert_eq!(game.life, 0);
    assert_eq!(game.life_paid, 2);
    assert_eq!(game.life_gained, 0);
    assert_eq!(game.life_funded_draws, 0);
    assert_eq!(game.player_counters.energy, 1);
    assert_eq!(game.battlefield.len(), 1);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.battlefield[0].counters.charge, 1);
    assert_eq!(game.graveyard, [CardIdx(2), CardIdx(1)]);
    assert!(game.hand.is_empty());
    assert_eq!(game.library, [CardIdx(4), CardIdx(3)]);
    assert_eq!(game.seen, 0);
    assert_eq!(game.next_uid, 2);
    assert_library_rng(&game.library_rng, &rng);
}

/// Change a selected target's graveyard generation through an actual cost death trigger.
fn generation_scenario(partial: bool) -> (SimDeck, GameState) {
    let mut cards = deck(&[
        row(
            "Synthetic generation source",
            "{99}",
            "Artifact",
            "{1}, {T}, Sacrifice a creature: Return target creature card from your graveyard to the battlefield. Gain 5 life. Draw a card.",
        ),
        row(
            "Synthetic cost death trigger",
            "{99}",
            "Enchantment",
            "Whenever a creature you control dies, return target card from your graveyard to your hand.",
        ),
        row("Synthetic first target", "{99}", "Creature", ""),
        row("Synthetic reward draw", "{99}", "Sorcery", ""),
        row("Synthetic undrawn card", "{99}", "Sorcery", ""),
        row("Synthetic second target", "{99}", "Creature", ""),
    ]);
    cards.cards[0].striations[0].abilities[0].effects = vec![
        SimEffect::GainLife(5),
        SimEffect::ReturnFromGraveyard {
            to_hand: false,
            count: if partial { 2 } else { 1 },
        },
        SimEffect::Draw(1),
    ];
    cards.cards[1].striations[0].abilities[0].effects = vec![
        SimEffect::ReturnFromGraveyard {
            to_hand: true,
            count: 1,
        },
        SimEffect::DrawDiscard {
            draw: 0,
            discard: 1,
        },
    ];
    let mut game = state(vec![], vec![4, 3]);
    put_permanents(&cards, &mut game, &[0, 1]);
    game.next_uid += 1;
    game.battlefield.push(new_token_perm(game.next_uid, 0));
    move_to_graveyard(&cards, &mut game, CardIdx(2), 1, CardZone::Hand);
    if partial {
        move_to_graveyard(&cards, &mut game, CardIdx(5), 1, CardZone::Hand);
    }
    (cards, game)
}

/// All illegal targets stop linked rewards; a surviving target keeps legal effects.
#[test]
fn cost_death_trigger_cannot_retarget_a_new_graveyard_generation() {
    for partial in [false, true] {
        let (cards, mut game) = generation_scenario(partial);
        let rng = game.library_rng.clone();
        let mut pool = ManaPool {
            colorless: 1,
            ..Default::default()
        };
        let action = pick_best_activation(&cards, &game, &pool, 1).expect("original targets exist");
        assert_eq!(action.sacrifice_uid, Some(3));
        resolve_activation(&cards, &mut game, &mut pool, 1, &action);

        let stale = CardIdx(if partial { 5 } else { 2 });
        assert_eq!(pool.total(), 0);
        assert_eq!(game.graveyard, [stale]);
        assert_eq!(game.graveyard_generations[&stale], 2);
        assert_eq!(game.battlefield.len(), if partial { 3 } else { 2 });
        assert!(game.battlefield[0].tapped);
        assert!(
            !game
                .battlefield
                .iter()
                .any(|permanent| permanent.card == CardRef::Deck(stale))
        );
        assert_eq!(game.life, if partial { 25 } else { 20 });
        assert_eq!(game.life_gained, if partial { 5 } else { 0 });
        assert_eq!(game.hand, if partial { vec![CardIdx(3)] } else { vec![] });
        assert_eq!(
            game.library,
            if partial {
                vec![CardIdx(4)]
            } else {
                vec![CardIdx(4), CardIdx(3)]
            }
        );
        assert_eq!(game.seen, u32::from(partial));
        assert_eq!(game.card_first_seen.get(&stale.index()), Some(&1));
        assert!(game.completion.is_none());
        if partial {
            assert_eq!(game.battlefield[2].card, CardRef::Deck(CardIdx(2)));
        }
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Build synthetic Oracle abilities that change a captured card's generation during payment.
fn stale_target_report_scenario() -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic stale-target source",
            "{99}",
            "Artifact",
            "{1}, {T}, Sacrifice a creature: Gain 5 life. Return target creature card from your graveyard to the battlefield. Draw a card.",
        ),
        row(
            "Synthetic payment return listener",
            "{99}",
            "Enchantment",
            "Whenever a creature you control dies, return target card from your graveyard to your hand. Gain 2 life.",
        ),
        row(
            "Synthetic payment discard listener",
            "{99}",
            "Enchantment",
            "Whenever a creature you control dies, draw 0 cards, then discard a card.",
        ),
        row(
            "Synthetic unselected graveyard creature",
            "{99}",
            "Creature",
            "",
        ),
        row(
            "Synthetic captured graveyard creature",
            "{99}",
            "Creature",
            "",
        ),
        row("Synthetic suppressed reward draw", "{99}", "Sorcery", ""),
        row("Synthetic undrawn generation marker", "{99}", "Sorcery", ""),
        row(
            "Synthetic generation funding",
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        ),
    ]);
    let mut game = state(vec![], vec![6, 5]);
    put_permanents(&cards, &mut game, &[0, 1, 2]);
    game.next_uid += 1;
    game.battlefield.push(new_token_perm(game.next_uid, 0));
    put_permanents(&cards, &mut game, &[7]);
    move_to_graveyard(&cards, &mut game, CardIdx(3), 1, CardZone::Hand);
    move_to_graveyard(&cards, &mut game, CardIdx(4), 1, CardZone::Hand);
    (cards, game)
}

/// Check synthetic payment-trigger settlement, not full priority or multiplayer timing.
pub(super) fn assert_synthetic_stale_target_generation_report() {
    let (cards, game) = stale_target_report_scenario();
    let rng = game.library_rng.clone();
    assert_eq!(game.graveyard, [CardIdx(3), CardIdx(4)]);
    assert_eq!(game.graveyard_generations[&CardIdx(3)], 1);
    assert_eq!(game.graveyard_generations[&CardIdx(4)], 1);
    let action = pick_best_activation(
        &cards,
        &game,
        &ManaPool {
            colorless: 1,
            ..Default::default()
        },
        1,
    )
    .expect("the original graveyard target and complete payment exist");
    assert_eq!(action.uid, 1);
    assert_eq!(action.sacrifice_uid, Some(4));
    assert_eq!(
        action
            .ability
            .activation
            .as_ref()
            .expect("typed activation costs")
            .costs,
        [
            SimActivationCost::Mana(Cost {
                generic: 1,
                ..Cost::default()
            }),
            SimActivationCost::TapSource,
            SimActivationCost::SacrificeCreatures {
                count: 1,
                exclude_source: false
            },
        ]
    );

    let (game, log, report) = executed_report(&cards, game, 1, vec![]);

    assert_eq!(game.graveyard, [CardIdx(3), CardIdx(4)]);
    assert_eq!(game.graveyard_generations[&CardIdx(3)], 1);
    assert_eq!(game.graveyard_generations[&CardIdx(4)], 2);
    assert_eq!(
        game.battlefield
            .iter()
            .map(|permanent| permanent.uid)
            .collect::<Vec<_>>(),
        [1, 2, 3, 5]
    );
    assert_eq!(
        game.battlefield
            .iter()
            .map(|permanent| permanent.card)
            .collect::<Vec<_>>(),
        [0, 1, 2, 7].map(|index| CardRef::Deck(CardIdx(index)))
    );
    assert!(game.battlefield[0].tapped);
    assert!(game.battlefield[3].tapped);
    assert_eq!(game.next_uid, 5);
    assert!(game.hand.is_empty() && game.exile.is_empty());
    assert_eq!(game.library, [CardIdx(6), CardIdx(5)]);
    assert_eq!(game.card_first_seen.get(&4), Some(&1));
    assert!(!game.card_first_seen.contains_key(&3));
    assert_eq!(game.life, 22);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.life_gained, 2);
    assert_eq!(game.seen, 0);
    assert_eq!(game.life_funded_draws, 0);
    assert_eq!(game.spells_cast_this_turn, 0);
    assert!(game.completion.is_none());
    assert_library_rng(&game.library_rng, &rng);

    assert_eq!(log.completion, GameCompletion::HorizonReached);
    assert_eq!(log.turns_observed, 1);
    assert!(log.turn_observations[0].main && log.turn_observations[0].end_turn);
    assert_eq!(log.mana_available, [1.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(log.cards_seen, [0]);
    assert_eq!(log.graveyard_size, [2]);
    assert_eq!(log.creatures, [0]);
    assert_eq!(log.life_gained, 2);
    assert_eq!(log.card_first_seen.get(&4), Some(&1));
    for index in [3, 4] {
        assert_eq!(log.card_first_graveyard.get(&index), Some(&1));
        assert!(!log.card_first_battlefield.contains_key(&index));
    }
    assert_eq!(report["outcomes"]["finished_games"], 1);
    assert_eq!(report["mana"]["avg_available_by_turn"]["1"], 1.0);
    assert_eq!(report["mana"]["avg_unused_by_turn"]["1"], 0.0);
    assert_eq!(report["draw"]["avg_life_gained"], 2.0);
    assert_eq!(report["draw"]["avg_life_paid"], 0.0);
    assert_eq!(report["velocity"]["avg_cards_seen"]["1"], 0.0);
    assert_eq!(report["velocity"]["library_remaining_by_turn"]["1"], 2.0);
    assert_eq!(report["graveyard"]["replay_casts_avg"], 0.0);
}

/// Keep synthetic stale-target report evidence callable without granting pinned source credit.
#[test]
fn synthetic_stale_target_generation_has_executed_report_evidence() {
    assert_synthetic_stale_target_generation_report();
}

/// Each linked return keeps its original target, even after the first return moves it.
#[test]
fn linked_activation_returns_never_substitute_the_newly_sacrificed_creature() {
    let (mut cards, mut game) = bundled_scenario(true);
    cards.cards[0].striations[0].abilities[0].effects = vec![
        SimEffect::ReturnFromGraveyard {
            to_hand: true,
            count: 1,
        },
        SimEffect::ReturnFromGraveyard {
            to_hand: false,
            count: 1,
        },
        SimEffect::GainLife(5),
        SimEffect::Draw(1),
    ];
    let rng = game.library_rng.clone();
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let action = pick_best_activation(&cards, &game, &pool, 1).expect("both linked targets exist");
    resolve_activation(&cards, &mut game, &mut pool, 1, &action);

    assert_eq!(pool.total(), 0);
    assert_eq!(game.battlefield.len(), 1);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.hand, [CardIdx(2), CardIdx(3)]);
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(game.library, [CardIdx(4)]);
    assert_eq!(game.life, 23);
    assert_eq!(game.life_paid, 2);
    assert_eq!(game.life_gained, 5);
    assert_eq!(game.seen, 1);
    assert_library_rng(&game.library_rng, &rng);
}

/// Nontargeted activations still resolve without a graveyard target.
#[test]
fn nontargeted_activation_keeps_its_existing_cost_and_reward_path() {
    let (mut cards, mut game) = bundled_scenario(false);
    cards.cards[0].striations[0].abilities[0].effects =
        vec![SimEffect::GainLife(5), SimEffect::Draw(1)];
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let action = pick_best_activation(&cards, &game, &pool, 1).expect("no targets are required");
    resolve_activation(&cards, &mut game, &mut pool, 1, &action);

    assert_eq!(pool.total(), 0);
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(game.hand, [CardIdx(3)]);
    assert_eq!(game.library, [CardIdx(4)]);
    assert_eq!(game.battlefield.len(), 1);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.life, 23);
    assert_eq!(game.life_paid, 2);
}

/// A return-only activation needs its target, not an unrelated reward or sacrifice cost.
#[test]
fn return_only_activation_requires_and_keeps_a_pre_cost_hand_return_target() {
    let cards = deck(&[
        row(
            "Synthetic return-only source",
            "{99}",
            "Artifact",
            "{1}, {T}: Return target card from your graveyard to your hand.",
        ),
        row("Synthetic hand-return target", "{99}", "Creature", ""),
    ]);
    for target_present in [false, true] {
        let mut game = state(vec![], vec![]);
        put_permanents(&cards, &mut game, &[0]);
        if target_present {
            move_to_graveyard(&cards, &mut game, CardIdx(1), 1, CardZone::Hand);
        }
        let action = printed_action(&cards, &game);
        let mut pool = ManaPool {
            colorless: 1,
            ..Default::default()
        };
        let before = game.clone();
        let before_pool = pool.clone();
        assert_eq!(
            pick_best_activation(&cards, &game, &pool, 1).is_some(),
            target_present
        );
        resolve_activation(&cards, &mut game, &mut pool, 1, &action);
        if target_present {
            assert_eq!(pool.total(), 0);
            assert_eq!(game.hand, [CardIdx(1)]);
            assert!(game.graveyard.is_empty());
            assert_eq!(game.battlefield.len(), 1);
            assert!(game.battlefield[0].tapped);
            assert_eq!(game.life, 20);
            assert_eq!(game.life_paid, 0);
            assert_eq!(game.seen, 0);
            assert_library_rng(&game.library_rng, &before.library_rng);
        } else {
            assert_unchanged(&game, &before, &pool, &before_pool);
        }
    }
}

/// A replacement source with a new uid cannot pay an old activation's costs.
#[test]
fn target_preparation_does_not_bypass_the_source_identity_guard() {
    let (cards, mut game) = bundled_scenario(true);
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let action =
        pick_best_activation(&cards, &game, &pool, 1).expect("the original source is legal");
    game.battlefield[0].uid = 30;
    game.next_uid = 30;
    let before = game.clone();
    let before_pool = pool.clone();
    resolve_activation(&cards, &mut game, &mut pool, 1, &action);
    assert_unchanged(&game, &before, &pool, &before_pool);
}

/// Keep Cauldron's pinned action, rejection, and real report boundaries callable.
#[test]
fn pinned_cauldron_selects_targets_before_payment_and_reports_the_original_target() {
    let source = named_row("Cauldron of Essence");
    assert_cauldron_pre_cost_activation(&source);
    assert_cauldron_rejected_activation(&source);
    assert_cauldron_activation_boundary_report(&source);
}

/// Keep a source creature, optional prior graveyard card, and unrelated zones distinct.
fn unsupported_return_scenario(source: &CardRow, prior_card: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row("Prior return candidate", "{99}", "Creature", ""),
        row("Undrawn return marker", "{99}", "Sorcery", ""),
        row("Retained return marker", "{99}", "Sorcery", ""),
        row("Exiled return marker", "{99}", "Artifact", ""),
        row("Return funding", "", "Basic Land - Wastes", "{T}: Add {C}."),
    ]);
    let mut game = state(vec![3], vec![2]);
    game.exile.push(CardIdx(4));
    if cards.cards[0].is_instant_or_sorcery {
        game.hand.insert(0, CardIdx(0));
    } else {
        put_permanents(&cards, &mut game, &[0]);
    }
    if prior_card {
        move_to_graveyard(&cards, &mut game, CardIdx(1), 1, CardZone::Hand);
    }
    (cards, game)
}

/// Unsupported return choices cannot be mislabeled as executable targeted returns in reports.
fn assert_unsupported_return_report(report: &Value, source: &CardRow) {
    let card = report["card_support"]
        .as_array()
        .expect("compiler records")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("the source remains in the report");
    assert_eq!(
        card["source_hash"],
        super::support::card_source_hash(source)
    );
    let abilities = card["abilities"].as_array().expect("ability records");
    assert_eq!(abilities.len(), 1);
    let ability = &abilities[0];
    assert_eq!(ability["source"], source.oracle_text);
    assert_eq!(ability["span_start"], 0);
    assert_eq!(ability["span_end"], source.oracle_text.len());
    assert_eq!(ability["status"], "unsupported");
    assert!(
        ability["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert!(ability["rule_id"].is_null());
}

/// A resolution-time choice cannot consume a creature through the pre-cost target path.
#[test]
fn nontargeted_graveyard_return_activation_stays_inert_before_any_sacrifice() {
    for instruction in [
        "Return a creature card from your graveyard to the battlefield",
        "Return a card from your graveyard to your hand",
        "You may return a creature card from your graveyard to the battlefield",
    ] {
        let source = row(
            "Unsupported return source",
            "{99}",
            "Creature",
            &format!("Sacrifice a creature: {instruction}. Gain 3 life."),
        );
        for prior_card in [false, true] {
            let (cards, mut game) = unsupported_return_scenario(&source, prior_card);
            assert!(
                cards.cards[0]
                    .unlocked_abilities(0)
                    .all(|ability| !ability.kind.is_activated())
            );
            let mut pool = ManaPool {
                colorless: 1,
                ..Default::default()
            };
            let before = game.clone();
            let before_pool = pool.clone();
            assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
            spend_leftover(&cards, &mut game, &mut pool, 1);
            assert_unchanged(&game, &before, &pool, &before_pool);
        }
    }
}

/// Ordinary printed spell payment can commit without running an unsupported return or linked gain.
#[test]
fn unsupported_graveyard_return_spell_pays_only_its_printed_cost() {
    for instruction in [
        "Return a creature card from your graveyard to the battlefield",
        "Return a card from your graveyard to your hand",
        "Return target creature card from your graveyard to the battlefield tapped",
        "Return target artifact card from your graveyard to the battlefield",
    ] {
        let source = row(
            "Unsupported return spell",
            "{1}",
            "Sorcery",
            &format!("{instruction}. Gain 3 life."),
        );
        let (cards, mut game) = unsupported_return_scenario(&source, true);
        assert!(cards.cards[0].spell_data.effects.is_empty());
        let rng = game.library_rng.clone();
        let mut pool = ManaPool {
            colorless: 1,
            ..Default::default()
        };

        assert_eq!(cast_action(&cards, &mut game, &mut pool), 1.0);

        assert_eq!(pool.total(), 0);
        assert_eq!(game.hand, [CardIdx(3)]);
        assert_eq!(game.library, [CardIdx(2)]);
        assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0)]);
        assert_eq!(game.exile, [CardIdx(4)]);
        assert!(game.battlefield.is_empty());
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.life_gained, 0);
        assert_eq!(game.spells_cast_this_turn, 1);
        assert_eq!(game.seen, 0);
        assert!(game.completion.is_none());
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Executed reports keep unsupported return requirements and cannot claim partial rewards.
#[test]
fn unsupported_return_choices_report_inert_abilities_and_committed_spell_costs() {
    for instruction in [
        "Return a creature card from your graveyard to the battlefield",
        "Return a card from your graveyard to your hand",
        "Return target creature card from your graveyard to the battlefield tapped",
        "Return target artifact card from your graveyard to the battlefield",
    ] {
        for activation in [true, false] {
            let source = row(
                "Reported unsupported return",
                if activation { "{99}" } else { "{1}" },
                if activation { "Creature" } else { "Sorcery" },
                &if activation {
                    format!("Sacrifice a creature: {instruction}. Gain 3 life.")
                } else {
                    format!("{instruction}. Gain 3 life.")
                },
            );
            let (cards, mut game) = unsupported_return_scenario(&source, true);
            put_permanents(&cards, &mut game, &[5]);
            let rng = game.library_rng.clone();
            let (game, log, report) = executed_report(&cards, game, 1, vec![]);
            assert_eq!(game.spells_cast_this_turn, u32::from(!activation));
            assert_eq!(game.life, 20);
            assert_eq!(game.life_paid, 0);
            assert_eq!(game.life_gained, 0);
            assert_eq!(game.library, [CardIdx(2)]);
            assert_eq!(game.exile, [CardIdx(4)]);
            assert_eq!(game.hand, [CardIdx(3)]);
            assert_eq!(
                game.graveyard,
                if activation {
                    vec![CardIdx(1)]
                } else {
                    vec![CardIdx(1), CardIdx(0)]
                }
            );
            assert_eq!(log.mana_available, [1.0]);
            assert_eq!(log.mana_spent, [if activation { 0.0 } else { 1.0 }]);
            assert_eq!(log.life_gained, 0);
            assert_eq!(report["draw"]["avg_life_gained"], 0.0);
            assert_eq!(
                report["mana"]["avg_unused_by_turn"]["1"],
                if activation { 1.0 } else { 0.0 }
            );
            assert_unsupported_return_report(&report, &source);
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}
