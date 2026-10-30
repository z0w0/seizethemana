//! Synthetic X, optional-cost, and restriction reports close specific engine branches.
//!
//! The free-cast callback binds Maelstrom Wanderer's pinned one-cascade subset.
//! All other sources are synthetic. Kicker checks the single-payment policy and
//! its modeled life-loss rider, not a real card's complete kicker instructions.

use super::card_cost_claim_tests::{assert_mana_report, assert_unchanged};
use super::card_runtime_tests::attempt_index;
use super::card_test_support::{
    assert_library_rng, main_phase_state as state, named_row, pinned_revision,
};
use super::cost_boundary_report_tests::{assert_cost_loss_report, execute_cost_report};
use super::game::{CardRef, GameState};
use super::game_effects::activation::{pick_best_activation, resolve_activation};
use super::game_run::mana::build_pool;
use super::library_core_report_tests::{assert_report_clause, put_permanents};
use super::model::{CardIdx, SimDeck, SpendRestriction};
use super::scenario_test_support::{deck, row};

/// Keep a creature-only lot visible while a repeated-X sorcery uses only eligible mana.
fn repeated_x_scenario(generic: u32, black: bool) -> (SimDeck, GameState) {
    let symbols = format!(
        "{}{}",
        "{C}".repeat(generic as usize),
        if black { "{B}" } else { "" }
    );
    let cards = deck(&[
        row(
            "Synthetic repeated X",
            "{X}{X}{B}",
            "Sorcery",
            "Target player loses X life.",
        ),
        row(
            "General X funding",
            "{99}",
            "Artifact",
            &format!("{{T}}: Add {symbols}."),
        ),
        row(
            "Unrelated X funding",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}{C}. Spend this mana only to cast creature spells.",
        ),
        row("Restricted class witness", "{99}", "Creature", ""),
        row("Unpayable follow-up", "{2}", "Sorcery", ""),
    ]);
    let mut game = state(if black { vec![0, 3, 4] } else { vec![0, 3] }, vec![]);
    put_permanents(&cards, &mut game, &[1, 2]);
    (cards, game)
}

/// Repeated spell X shares one value, permits zero, and cannot spend an unrelated lot.
pub(super) fn assert_repeated_x_spell_report() {
    for (generic, black, x, spent, remaining) in
        [(7, true, 3, 7, 4), (0, true, 0, 1, 3), (7, false, 0, 0, 10)]
    {
        let (cards, game) = repeated_x_scenario(generic, black);
        let mut action = game.clone();
        let mut pool = build_pool(&cards, &mut action, 1);
        let before = action.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut action, &mut pool, CardIdx(0)),
            black.then_some(spent)
        );
        if !black {
            assert_unchanged(&action, &before, &pool, &before_pool);
        }
        assert_eq!(pool.total(), remaining);
        assert_eq!(
            pool.lots
                .iter()
                .filter(|lot| lot.restriction == Some(SpendRestriction::Creature))
                .map(|lot| lot.amount)
                .sum::<u32>(),
            3
        );
        if black {
            let before = action.clone();
            let before_pool = pool.clone();
            assert_eq!(
                attempt_index(&cards, &mut action, &mut pool, CardIdx(4)),
                None
            );
            assert_unchanged(&action, &before, &pool, &before_pool);
        }
        let rng = game.library_rng.clone();
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.opponent_life_lost, x);
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.spells_cast_this_turn, u32::from(black));
        assert_eq!(
            game.hand,
            if black {
                vec![CardIdx(3), CardIdx(4)]
            } else {
                vec![CardIdx(0), CardIdx(3)]
            }
        );
        assert_eq!(
            game.graveyard,
            if black { vec![CardIdx(0)] } else { vec![] }
        );
        assert!(game.library.is_empty() && game.exile.is_empty());
        assert_eq!(game.battlefield.len(), 2);
        assert!(game.battlefield.iter().all(|permanent| permanent.tapped));
        assert_mana_report(
            &log,
            &json,
            f64::from(generic + u32::from(black) + 3),
            f64::from(spent),
            f64::from(remaining),
        );
        assert_eq!(log.opponent_life_loss, [x]);
        assert_eq!(
            json["win_conditions"]["opponent_life_loss_by_turn"]["1"],
            f64::from(x)
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Pinned cascade uses X zero despite spare mana; an unpaid source cannot reveal its hit.
pub(super) fn assert_cascade_free_x_zero_report() {
    let source = named_row("Maelstrom Wanderer");
    pinned_revision(&source);
    let cards = deck(&[
        source.clone(),
        row("Synthetic free X hit", "{X}{U}", "Sorcery", "Draw X cards."),
        row("Unexposed bottom", "{99}", "Sorcery", ""),
        row(
            "Cascade funding",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}{C}{C}{C}{C}{U}{R}{G}.",
        ),
    ]);
    let mut unpaid = state(vec![0], vec![2, 1]);
    let mut pool = super::game::ManaPool {
        colorless: 4,
        fixed: [0, 1, 0, 1, 1],
        ..Default::default()
    };
    let before = unpaid.clone();
    let before_pool = pool.clone();
    assert_eq!(
        attempt_index(&cards, &mut unpaid, &mut pool, CardIdx(0)),
        None
    );
    assert_unchanged(&unpaid, &before, &pool, &before_pool);
    let mut game = state(vec![0], vec![2, 1]);
    put_permanents(&cards, &mut game, &[3]);
    let rng = game.library_rng.clone();
    let (game, log, json) = execute_cost_report(&cards, game, 1);
    assert_eq!(game.spells_cast_this_turn, 2);
    assert!(game.hand.is_empty() && game.exile.is_empty());
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(
        game.seen, 1,
        "only cascade's revealed hit is an exposure; X zero draws nothing"
    );
    assert_eq!(game.battlefield.len(), 2);
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(game.life, 20);
    assert_mana_report(&log, &json, 9.0, 8.0, 1.0);
    assert_eq!(log.cards_seen, [1]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 1.0);
    assert_report_clause(&json, &source, "Cascade, cascade", "unsupported", None);
    assert_library_rng(&game.library_rng, &rng);
}

/// Give an X activation real generic funding and a separate, unusable spell-only lot.
fn counter_scenario(generic: u32, unknown: bool) -> (SimDeck, GameState) {
    let counter = if unknown { "tower" } else { "charge" };
    let cards = deck(&[
        row(
            "Synthetic X counter source",
            "{99}",
            "Artifact",
            &format!("{{1}}{{X}}{{X}}, {{T}}: Put X {counter} counters on this artifact."),
        ),
        row(
            "General counter funding",
            "{99}",
            "Artifact",
            &format!("{{T}}: Add {}.", "{C}".repeat(generic.max(1) as usize)),
        ),
        row(
            "Restricted counter funding",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}{C}. Spend this mana only to cast creature spells.",
        ),
        row("Held class witness", "{99}", "Creature", ""),
    ]);
    let mut game = state(vec![3], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    if generic > 0 {
        put_permanents(&cards, &mut game, &[1]);
    }
    put_permanents(&cards, &mut game, &[2]);
    (cards, game)
}

/// Charge-counter X is fixed before payment; repeated symbols, zero, and unknown kinds stay distinct.
pub(super) fn assert_x_activation_boundaries_report() {
    for (generic, unknown, x, remaining) in [
        (6, false, 2, 4),
        (1, false, 0, 3),
        (0, false, 0, 3),
        (6, true, 0, 9),
    ] {
        let (cards, game) = counter_scenario(generic, unknown);
        let accepted = generic > 0 && !unknown;
        let mut action = game.clone();
        let mut pool = build_pool(&cards, &mut action, 1);
        let before_pool = pool.clone();
        let selected = pick_best_activation(&cards, &action, &pool, 1);
        assert_eq!(selected.is_some(), accepted);
        if let Some(selected) = selected {
            assert_eq!(selected.paid_x, x);
            assert_eq!(selected.ability_cost.generic, 1 + 2 * x);
            resolve_activation(&cards, &mut action, &mut pool, 1, &selected);
        } else {
            assert_eq!(pool, before_pool);
        }
        assert_eq!(action.battlefield[0].counters.charge, x);
        assert_eq!(action.battlefield[0].tapped, accepted);
        assert_eq!(pool.total(), remaining);
        assert_eq!(pool.lots.iter().map(|lot| lot.amount).sum::<u32>(), 3);
        let rng = game.library_rng.clone();
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.battlefield[0].counters.charge, x);
        assert_eq!(game.battlefield[0].tapped, accepted);
        assert_eq!(game.hand, [CardIdx(3)]);
        assert!(game.graveyard.is_empty() && game.exile.is_empty() && game.library.is_empty());
        assert_eq!(game.life, 20);
        assert_eq!(game.spells_cast_this_turn, 0);
        assert_mana_report(
            &log,
            &json,
            f64::from(generic + 3),
            0.0,
            f64::from(remaining),
        );
        assert_eq!(log.cards_seen, [0]);
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Keep the restricted source eligible without allowing an expensive witness to consume it.
fn class_restriction_scenario(legendary: bool, matching: bool) -> (SimDeck, GameState) {
    let class = if legendary {
        "legendary"
    } else {
        "instant and sorcery"
    };
    let kind = if legendary {
        if matching {
            "Legendary Artifact"
        } else {
            "Artifact"
        }
    } else if matching {
        "Sorcery"
    } else {
        "Creature"
    };
    let cards = deck(&[
        row("Synthetic class target", "{G}", kind, ""),
        row(
            "Synthetic class source",
            "{99}",
            "Artifact",
            &format!("{{T}}: Add {{G}}. Spend this mana only to cast {class} spells."),
        ),
        row(
            "Held eligible witness",
            "{99}",
            if legendary {
                "Legendary Artifact"
            } else {
                "Sorcery"
            },
            "",
        ),
    ]);
    let mut game = state(vec![0, 2], vec![]);
    put_permanents(&cards, &mut game, &[1]);
    (cards, game)
}

/// Legendary and instant/sorcery lots reject wrong classes through full actions and reports.
pub(super) fn assert_remaining_cast_class_restrictions_report() {
    for legendary in [false, true] {
        for matching in [false, true] {
            let (cards, game) = class_restriction_scenario(legendary, matching);
            let mut action = game.clone();
            let mut pool = build_pool(&cards, &mut action, 1);
            let before = action.clone();
            let before_pool = pool.clone();
            assert_eq!(pool.total(), 1);
            assert_eq!(
                attempt_index(&cards, &mut action, &mut pool, CardIdx(0)),
                matching.then_some(1)
            );
            if !matching {
                assert_unchanged(&action, &before, &pool, &before_pool);
            }
            assert_eq!(pool.total(), u32::from(!matching));
            let (game, log, json) = execute_cost_report(&cards, game, 1);
            assert_eq!(
                game.hand,
                if matching {
                    vec![CardIdx(2)]
                } else {
                    vec![CardIdx(0), CardIdx(2)]
                }
            );
            assert_eq!(
                game.graveyard,
                if matching && !legendary {
                    vec![CardIdx(0)]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                game.battlefield.len(),
                if matching && legendary { 2 } else { 1 }
            );
            assert!(game.battlefield[0].tapped);
            assert_eq!(game.spells_cast_this_turn, u32::from(matching));
            assert_eq!(game.life, 20);
            assert_mana_report(
                &log,
                &json,
                1.0,
                f64::from(u32::from(matching)),
                f64::from(u32::from(!matching)),
            );
        }
    }
}

/// Pip-only mana can pay blue but cannot pay a generic cost or increase generic X.
pub(super) fn assert_pip_only_and_generic_x_report() {
    for (cost, generic, accepted, draws, spent, remaining) in [
        ("{U}", false, true, 0, 1, 1),
        ("{1}", false, false, 0, 0, 2),
        ("{X}{U}", false, true, 0, 1, 1),
        ("{X}{U}", true, true, 2, 3, 1),
    ] {
        let cards = deck(&[
            row(
                "Synthetic pip target",
                cost,
                "Sorcery",
                if cost.contains('X') {
                    "Draw X cards."
                } else {
                    ""
                },
            ),
            row(
                "Synthetic pip-only source",
                "{99}",
                "Artifact",
                "{T}: Add {U}{U}. This mana can't be spent to pay generic mana costs.",
            ),
            row("Generic X funding", "{99}", "Artifact", "{T}: Add {C}{C}."),
            row("First physical draw", "{99}", "Sorcery", ""),
            row("Second physical draw", "{99}", "Sorcery", ""),
            row("Undrawn bottom", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![0], vec![5, 4, 3]);
        put_permanents(&cards, &mut game, &[1]);
        if generic {
            put_permanents(&cards, &mut game, &[2]);
        }
        let mut action = game.clone();
        let mut pool = build_pool(&cards, &mut action, 1);
        let before = action.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut action, &mut pool, CardIdx(0)),
            accepted.then_some(spent)
        );
        if !accepted {
            assert_unchanged(&action, &before, &pool, &before_pool);
        }
        assert_eq!(pool.fixed[1], if accepted { 1 } else { 2 });
        assert_eq!(pool.fixed_no_generic[1], pool.fixed[1]);
        assert_eq!(pool.colorless, 0);
        let rng = game.library_rng.clone();
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(
            game.hand,
            if !accepted {
                vec![CardIdx(0)]
            } else if draws == 2 {
                vec![CardIdx(3), CardIdx(4)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.library,
            if draws == 2 {
                vec![CardIdx(5)]
            } else {
                vec![CardIdx(5), CardIdx(4), CardIdx(3)]
            }
        );
        assert_eq!(
            game.graveyard,
            if accepted { vec![CardIdx(0)] } else { vec![] }
        );
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        assert_eq!(game.seen, draws);
        assert_mana_report(
            &log,
            &json,
            if generic { 4.0 } else { 2.0 },
            f64::from(spent),
            f64::from(remaining),
        );
        assert_eq!(log.cards_seen, [draws]);
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Single kicker payment uses only eligible mana and life; an optional unpaid cost stays optional.
pub(super) fn assert_optional_kicker_payment_report() {
    for (mode, life, paid, terminal) in [
        ("general", 3, true, false),
        ("none", 3, false, false),
        ("creature", 3, false, false),
        ("spell", 3, true, false),
        ("general", 1, false, false),
        ("general", 2, true, true),
    ] {
        let text = match mode {
            "creature" => "{T}: Add {C}. Spend this mana only to cast creature spells.",
            "spell" => "{T}: Add {C}. Spend this mana only to cast instant and sorcery spells.",
            _ => "{T}: Add {C}.",
        };
        let cards = deck(&[
            row(
                "Synthetic single kicker",
                "{0}",
                "Sorcery",
                "Kicker {1}{U/P}\nTarget player loses 1 life.",
            ),
            row("Kicker source", "{99}", "Artifact", text),
            row(
                "Eligible held witness",
                "{99}",
                if mode == "creature" {
                    "Creature"
                } else {
                    "Sorcery"
                },
                "",
            ),
        ]);
        assert!(cards.cards[0].spell_data.kicker.is_some());
        let mut game = state(vec![0, 2], vec![]);
        game.life = life;
        if mode != "none" {
            put_permanents(&cards, &mut game, &[1]);
        }
        let mut action = game.clone();
        let mut pool = build_pool(&cards, &mut action, 1);
        assert_eq!(
            attempt_index(&cards, &mut action, &mut pool, CardIdx(0)),
            Some(u32::from(paid))
        );
        assert_eq!(pool.total(), u32::from(mode != "none") - u32::from(paid));
        assert_eq!(action.life_paid, if paid { 2 } else { 0 });
        let (game, log, json) = execute_cost_report(&cards, game, 1);
        assert_eq!(game.life, life - if paid { 2 } else { 0 });
        assert_eq!(game.life_paid, if paid { 2 } else { 0 });
        assert_eq!(game.hand, [CardIdx(2)]);
        assert_eq!(
            game.opponent_life_lost,
            if terminal {
                0
            } else if paid {
                2
            } else {
                1
            }
        );
        assert_eq!(game.spells_cast_this_turn, 1);
        assert_eq!(
            game.graveyard,
            if terminal { vec![] } else { vec![CardIdx(0)] }
        );
        if terminal {
            assert_cost_loss_report(&log, &json, 2);
        } else {
            let available = f64::from(u32::from(mode != "none"));
            assert_mana_report(
                &log,
                &json,
                available,
                f64::from(u32::from(paid)),
                available - f64::from(u32::from(paid)),
            );
            assert_eq!(log.opponent_life_loss, [if paid { 2 } else { 1 }]);
            assert_eq!(json["draw"]["avg_life_paid"], if paid { 2.0 } else { 0.0 });
        }
    }
}

/// Synthetic repeated spell X preserves unrelated mana in its public report.
#[test]
fn synthetic_repeated_x_spell_boundary_report() {
    assert_repeated_x_spell_report();
}

/// Pinned one-cascade execution does not spend spare mana to increase free-cast X.
#[test]
fn pinned_cascade_free_x_zero_boundary_report() {
    assert_cascade_free_x_zero_report();
}

/// Synthetic charge-counter X covers repeated symbols, zero, restrictions, and unknown kinds.
#[test]
fn synthetic_x_activation_boundary_report() {
    assert_x_activation_boundaries_report();
}

/// Synthetic remaining cast classes cannot leak mana to another spell class.
#[test]
fn synthetic_remaining_cast_restriction_report() {
    assert_remaining_cast_class_restrictions_report();
}

/// Synthetic pip-only sources do not become generic or X mana.
#[test]
fn synthetic_pip_only_and_generic_x_boundary_report() {
    assert_pip_only_and_generic_x_report();
}

/// Synthetic optional kicker reports exact mana, life, rider, and loss boundaries.
#[test]
fn synthetic_optional_kicker_boundary_report() {
    assert_optional_kicker_payment_report();
}
