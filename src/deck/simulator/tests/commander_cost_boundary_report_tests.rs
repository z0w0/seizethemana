//! Synthetic command-zone costs use real casts, removals, turn counts, and reports.
//!
//! These are engine controls, not Commander legality or source-card certificates.
//! The recast fixture sacrifices its own commander through a supported activation.
//! No commander cast counts, completion records, or census values are fabricated.

use super::card_cost_claim_tests::assert_mana_report;
use super::card_test_support::{assert_library_rng, main_phase_state as state};
use super::cost_boundary_report_tests::{assert_cost_loss_report, execute_cost_report};
use super::game::{CardRef, GameState};
use super::game_commander::CommanderProfile;
use super::game_run::{TurnCensus, commander_phase, mana::build_pool};
use super::library_core_report_tests::put_permanents;
use super::model::{CardIdx, SimDeck};
use super::scenario_test_support::{deck, row};

/// Give a self-sacrificing commander five actual mana each turn and enough real draws.
fn recast_scenario(blue: bool) -> (SimDeck, GameState) {
    let mut cards = deck(&[
        row(
            "Synthetic recast funding",
            "{99}",
            "Artifact",
            if blue {
                "{T}: Add {C}{C}{C}{C}{U}."
            } else {
                "{T}: Add {C}{C}{C}{C}{C}."
            },
        ),
        row("Unexposed bottom", "{99}", "Sorcery", ""),
        row("Third-turn draw", "{99}", "Sorcery", ""),
        row("Second sacrifice draw", "{99}", "Sorcery", ""),
        row("Second-turn draw", "{99}", "Sorcery", ""),
        row("First sacrifice draw", "{99}", "Sorcery", ""),
    ]);
    cards.commanders = deck(&[row(
        "Synthetic recast commander", "{1}{U}", "Legendary Creature",
        "When this creature enters, gain 2 life.\nPay 1 life, Sacrifice this creature: Draw a card.",
    )]).cards;
    let mut game = state(vec![], vec![1, 2, 3, 4, 5]);
    put_permanents(&cards, &mut game, &[0]);
    (cards, game)
}

/// Commander payments increase from two to four to six and retain the blue requirement.
pub(super) fn assert_commander_tax_and_pips_report() {
    for blue in [false, true] {
        let (cards, game) = recast_scenario(blue);
        let rng = game.library_rng.clone();
        let (game, log, json) = execute_cost_report(&cards, game, 3);
        assert_eq!(game.commander_casts, [if blue { 2 } else { 0 }]);
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert!(game.battlefield[0].tapped);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.life, if blue { 22 } else { 20 });
        assert_eq!(game.life_paid, if blue { 2 } else { 0 });
        assert_eq!(game.life_gained, if blue { 4 } else { 0 });
        assert_eq!(
            game.hand,
            if blue {
                vec![CardIdx(5), CardIdx(4), CardIdx(3), CardIdx(2)]
            } else {
                vec![CardIdx(5), CardIdx(4)]
            }
        );
        assert_eq!(
            game.library,
            if blue {
                vec![CardIdx(1)]
            } else {
                vec![CardIdx(1), CardIdx(2), CardIdx(3)]
            }
        );
        assert_eq!(game.next_uid, if blue { 3 } else { 1 });
        assert_eq!(log.mana_available, [5.0, 5.0, 5.0]);
        assert_eq!(
            log.mana_spent,
            if blue {
                vec![2.0, 4.0, 0.0]
            } else {
                vec![0.0; 3]
            }
        );
        assert_eq!(
            log.mana_remaining,
            if blue {
                vec![3.0, 1.0, 5.0]
            } else {
                vec![5.0; 3]
            }
        );
        assert_eq!(
            log.cards_seen,
            if blue { vec![1, 3, 4] } else { vec![0, 1, 2] }
        );
        assert_eq!(log.commander_castable, blue.then_some(1));
        assert_eq!(log.turns_observed, 3);
        assert_eq!(json["commander"]["payable_games"], u32::from(blue));
        assert_eq!(
            json["commander"]["avg_first_cast_turn"],
            if blue {
                serde_json::json!(1.0)
            } else {
                serde_json::Value::Null
            }
        );
        assert_eq!(
            json["sampling"]["commander.percent_castable_by_turn.3"]["event_games"],
            u32::from(blue)
        );
        assert_eq!(
            json["sampling"]["commander.percent_castable_by_turn.3"]["observed_games"],
            1
        );
        assert_eq!(
            json["mana"]["avg_cast_spent_by_turn"]["2"],
            if blue { 4.0 } else { 0.0 }
        );
        assert_eq!(json["mana"]["avg_unused_by_turn"]["3"], 5.0);
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// An unavailable first commander cannot consume resources or block a payable partner.
pub(super) fn assert_commander_unsupported_costs_report() {
    for (cost, text, accepted) in [
        ("{U}", "When this creature enters, gain 3 life.", true),
        ("{S}", "When this creature enters, gain 3 life.", false),
        ("{Q}", "When this creature enters, gain 3 life.", false),
        ("", "When this creature enters, gain 3 life.", false),
        (
            "{U}",
            "As an additional cost to cast this spell, discard a card.\nWhen this creature enters, gain 3 life.",
            false,
        ),
        (
            "{U}",
            "As an additional cost to cast this spell, pay 3 life.\nWhen this creature enters, gain 3 life.",
            false,
        ),
        (
            "{U}",
            "As an additional cost to cast this spell, tap an artifact.\nWhen this creature enters, gain 3 life.",
            false,
        ),
    ] {
        let mut cards = deck(&[
            row(
                "Synthetic commander funding",
                "{99}",
                "Artifact",
                "{T}: Add {U}.",
            ),
            row("Available additional-cost card", "{99}", "Sorcery", ""),
        ]);
        cards.commanders = deck(&[
            row(
                "Synthetic unavailable commander",
                cost,
                "Legendary Creature",
                text,
            ),
            row(
                "Synthetic payable partner",
                "{0}",
                "Legendary Creature",
                "When this creature enters, gain 2 life.",
            ),
        ])
        .cards;
        let mut game = state(vec![1], vec![]);
        put_permanents(&cards, &mut game, &[0]);
        let rng = game.library_rng.clone();
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.commander_casts, [u32::from(accepted), 1]);
        assert_eq!(game.battlefield.len(), if accepted { 3 } else { 2 });
        assert!(
            game.battlefield
                .iter()
                .any(|permanent| permanent.card == CardRef::Commander { slot: 1 })
        );
        assert_eq!(
            game.battlefield
                .iter()
                .any(|permanent| permanent.card == CardRef::Commander { slot: 0 }),
            accepted
        );
        assert_eq!(game.hand, [CardIdx(1)]);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.life, if accepted { 25 } else { 22 });
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.spells_cast_this_turn, 1 + u32::from(accepted));
        assert_eq!(
            log.commander_castable,
            Some(1),
            "the partner's payability is not a claim that slot zero cast"
        );
        assert_mana_report(
            &log,
            &json,
            1.0,
            f64::from(u32::from(accepted)),
            f64::from(u32::from(!accepted)),
        );
        assert_eq!(json["commander"]["payable_games"], 1);
        assert_eq!(
            json["sampling"]["commander.percent_castable_by_turn.1"]["event_games"],
            1
        );
        assert_eq!(
            json["sampling"]["commander.percent_castable_by_turn.1"]["observed_games"],
            1
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// A partner's entry can fund a later life payment without making the first pass legal.
fn assert_partner_enables_later_life_payment(cards: &SimDeck, initial: &GameState) {
    assert_eq!(initial.life, 1);
    let mut game = initial.clone();
    let mut pool = build_pool(cards, &mut game, 1);
    let profile = CommanderProfile::new(cards);
    let mut census = TurnCensus::with_commander_count(1, cards.cards.len(), cards.commanders.len());
    let mut sources = Vec::new();
    commander_phase(
        cards,
        &mut game,
        &mut pool,
        &mut census,
        &profile,
        1,
        &mut sources,
        true,
    );

    assert_eq!(game.commander_casts, [0, 1]);
    assert_eq!(game.life, 3);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.life_gained, 2);
    assert_eq!(game.battlefield.len(), 2);
    assert_eq!(game.battlefield[1].card, CardRef::Commander { slot: 1 });
    assert_eq!(game.battlefield[1].uid, 2);
    assert_eq!(census.commander_castable, Some(1));
    assert_eq!(pool.colorless, 1);

    commander_phase(
        cards,
        &mut game,
        &mut pool,
        &mut census,
        &profile,
        1,
        &mut sources,
        false,
    );

    assert_eq!(game.commander_casts, [1, 1]);
    assert_eq!(game.life, 4);
    assert_eq!(game.life_paid, 2);
    assert_eq!(game.life_gained, 5);
    assert_eq!(game.battlefield.len(), 3);
    assert_eq!(game.battlefield[2].card, CardRef::Commander { slot: 0 });
    assert_eq!(game.battlefield[2].uid, 3);
    assert_eq!(pool.colorless, 1);
    assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
    assert!(game.completion.is_none());
    assert_library_rng(&game.library_rng, &initial.library_rng);
}

/// Last-life payment stops before entry; a surviving partner can enable a later retry.
pub(super) fn assert_commander_last_life_report() {
    for life in [1, 2, 3] {
        let mut cards = deck(&[row(
            "Unspent commander funding",
            "{99}",
            "Artifact",
            "{T}: Add {C}.",
        )]);
        cards.commanders = deck(&[
            row(
                "Synthetic life commander",
                "{B/P}",
                "Legendary Creature",
                "When this creature enters, gain 3 life.",
            ),
            row(
                "Synthetic life partner",
                "{0}",
                "Legendary Creature",
                "When this creature enters, gain 2 life.",
            ),
        ])
        .cards;
        let mut game = state(vec![], vec![]);
        game.life = life;
        put_permanents(&cards, &mut game, &[0]);
        let rng = game.library_rng.clone();
        if life == 1 {
            assert_partner_enables_later_life_payment(&cards, &game);
        }
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
        assert!(game.battlefield[0].tapped);
        assert_eq!(log.commander_castable, Some(1));
        assert_eq!(log.mana_spent, [0.0]);
        assert_eq!(json["commander"]["payable_games"], 1);
        // This is the first payable turn, not an entry time or a count of casts.
        assert_eq!(json["commander"]["avg_first_cast_turn"], 1.0);
        assert_eq!(
            json["sampling"]["commander.percent_castable_by_turn.1"]["event_games"],
            1
        );
        assert_eq!(
            json["sampling"]["commander.percent_castable_by_turn.1"]["observed_games"],
            1
        );
        if life == 2 {
            assert_eq!(game.life, 0);
            assert_eq!(game.life_gained, 0);
            assert_eq!(game.commander_casts, [1, 0]);
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_eq!(game.battlefield.len(), 1);
            assert_cost_loss_report(&log, &json, 2);
        } else {
            assert_eq!(game.commander_casts, [1, 1]);
            assert_eq!(game.spells_cast_this_turn, 2);
            assert_eq!(game.life_paid, 2);
            assert_eq!(game.life_gained, 5);
            assert_eq!(game.life, if life == 1 { 4 } else { 6 });
            assert_eq!(game.battlefield.len(), 3);
            let first_slot = if life == 1 { 1 } else { 0 };
            assert_eq!(
                game.battlefield[1].card,
                CardRef::Commander { slot: first_slot }
            );
            assert_eq!(
                game.battlefield[2].card,
                CardRef::Commander {
                    slot: 1 - first_slot
                }
            );
            assert_mana_report(&log, &json, 1.0, 0.0, 1.0);
            assert_eq!(json["outcomes"]["player_loss_games"], 0);
        }
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Actual commander casts and sacrifices prove the increasing-tax report.
#[test]
fn synthetic_commander_tax_and_colored_payment_report() {
    assert_commander_tax_and_pips_report();
}

/// Unsupported first-slot payments stay inert while a legal partner can cast.
#[test]
fn synthetic_commander_unsupported_cost_and_partner_report() {
    assert_commander_unsupported_costs_report();
}

/// Last-life command-zone payment cannot receive its entry reward or a partner entry.
#[test]
fn synthetic_commander_last_life_boundary_report() {
    assert_commander_last_life_report();
}
