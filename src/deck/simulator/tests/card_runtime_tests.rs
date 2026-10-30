//! Source-card action assertions use the pilot's single-attempt boundary.

use super::card_test_support::{main_phase_state as state, named_row};
use crate::db::CardRow;
use crate::deck::simulator::cast_pass::try_cast_from_hand;
use crate::deck::simulator::game::{GameState, ManaPool, new_perm_with};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row};
use rand::RngCore;

/// Cast and declared-opponent event engines with finite resources.
#[path = "card_engine_tests.rs"]
pub(super) mod engines;
/// Native mana actions and their resource and color controls.
#[path = "card_mana_tests.rs"]
pub(super) mod mana;
/// Search and recursion actions with exact physical targets.
#[path = "card_search_tests.rs"]
pub(super) mod search;

/// Attempt exactly one physical hand card without choosing a later pilot candidate.
pub(super) fn attempt(cards: &SimDeck, game: &mut GameState, pool: &mut ManaPool) -> Option<u32> {
    attempt_index(cards, game, pool, CardIdx(0))
}

/// Request one physical hand identity through the shared production cast boundary.
pub(super) fn attempt_index(
    cards: &SimDeck,
    game: &mut GameState,
    pool: &mut ManaPool,
    index: CardIdx,
) -> Option<u32> {
    try_cast_from_hand(
        cards,
        game,
        pool,
        1,
        index,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    )
}

/// Keep name controls separate from the pinned identity used for coverage counts.
pub(super) fn renamed(source: &CardRow) -> [CardRow; 2] {
    let mut control = source.clone();
    control.name = "Renamed source control".into();
    control.oracle_text = source.oracle_text.replace(&source.name, &control.name);
    [source.clone(), control]
}

/// Dark Ritual pays black, not green, and resolves exactly its printed mana.
pub(super) fn assert_ritual(source: &CardRow) {
    for source in renamed(source) {
        for (color, legal) in [(2, true), (4, false)] {
            let cards = deck(std::slice::from_ref(&source));
            let mut game = state(vec![0], vec![]);
            let mut pool = ManaPool::default();
            pool.fixed[color] = 1;
            assert_eq!(attempt(&cards, &mut game, &mut pool), legal.then_some(1));
            assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(0)] });
            assert_eq!(
                game.graveyard,
                if legal { vec![CardIdx(0)] } else { vec![] }
            );
            assert_eq!(pool.total(), if legal { 3 } else { 1 });
            assert_eq!(pool.fixed[2], if legal { 3 } else { 0 });
            assert!(
                game.library.is_empty() && game.exile.is_empty() && game.battlefield.is_empty()
            );
            assert_eq!(game.life, 20);
            assert_eq!(game.spells_cast_this_turn, u32::from(legal));
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = "Add {R}{R}.".into();
    let cards = deck(&[changed]);
    let mut game = state(vec![0], vec![]);
    let mut pool = ManaPool {
        fixed: [0, 0, 1, 0, 0],
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(1));
    assert_eq!(pool.fixed, [0, 0, 0, 2, 0]);
}

/// Harmonize draws three physical copies after paying its exact mana cost.
pub(super) fn assert_draw_three(source: &CardRow) {
    for source in renamed(source) {
        for (green, legal) in [(2, true), (1, false)] {
            let cards = deck(&[
                source.clone(),
                row("One", "{99}", "Sorcery", ""),
                row("Two", "{99}", "Sorcery", ""),
                row("Three", "{99}", "Sorcery", ""),
                row("Four", "{99}", "Sorcery", ""),
            ]);
            let mut game = state(vec![0], vec![1, 2, 3, 4]);
            let mut pool = ManaPool {
                fixed: [0, 0, 0, 0, green],
                colorless: 4 - green,
                ..Default::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), legal.then_some(4));
            assert_eq!(
                game.hand,
                if legal {
                    vec![CardIdx(4), CardIdx(3), CardIdx(2)]
                } else {
                    vec![CardIdx(0)]
                }
            );
            assert_eq!(
                game.library,
                if legal {
                    vec![CardIdx(1)]
                } else {
                    vec![CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4)]
                }
            );
            assert_eq!(
                game.graveyard,
                if legal { vec![CardIdx(0)] } else { vec![] }
            );
            assert_eq!(game.seen, if legal { 3 } else { 0 });
            assert_eq!(pool.total(), if legal { 0 } else { 4 });
            assert_eq!(game.life, 20);
            assert!(game.battlefield.is_empty() && game.exile.is_empty());
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = "Draw a card.".into();
    let cards = deck(&[
        changed,
        row("One", "{99}", "Sorcery", ""),
        row("Two", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![1, 2]);
    let mut pool = ManaPool {
        flexible: 4,
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(4));
    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!(game.library, [CardIdx(1)]);
}

/// Land searches retain subtype filters, destination, tapped state, and physical identity.
pub(super) fn assert_land_search(source: &CardRow) {
    let farseek = source.name == "Farseek";
    for source in renamed(source) {
        for matching in [true, false] {
            let kind = if matching != farseek {
                "Forest"
            } else {
                "Mountain"
            };
            let cards = deck(&[
                source.clone(),
                row("Candidate", "", &format!("Land — {kind}"), ""),
            ]);
            let mut game = state(vec![0], vec![1]);
            let mut pool = ManaPool {
                flexible: 2,
                ..Default::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), Some(2));
            assert_eq!(game.battlefield.len(), usize::from(matching));
            if matching {
                assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(1)));
                assert_eq!(game.battlefield[0].tapped, farseek);
            }
            assert_eq!(
                game.library,
                if matching { vec![] } else { vec![CardIdx(1)] }
            );
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert!(game.hand.is_empty() && game.exile.is_empty());
            assert_eq!(pool.total(), 0);
            assert_eq!(game.life, 20);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text =
        "Search your library for an artifact card, put it into your hand, then shuffle.".into();
    let cards = deck(&[
        changed,
        row("Changed selector result", "{99}", "Artifact", ""),
    ]);
    let mut game = state(vec![0], vec![1]);
    let mut pool = ManaPool {
        flexible: 2,
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(2));
    assert_eq!(game.hand, [CardIdx(1)]);
    assert!(game.library.is_empty() && game.battlefield.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0)]);
}

/// Printed creature sacrifices are mandatory; rejection preserves cards and payment.
pub(super) fn assert_sacrifice_draw(source: &CardRow) {
    let dispute = source.name == "Deadly Dispute";
    for source in renamed(source) {
        for has_creature in [true, false] {
            let cards = deck(&[
                source.clone(),
                row("Payment", "{0}", "Creature — Elf", ""),
                row("One", "{99}", "Sorcery", ""),
                row("Two", "{99}", "Sorcery", ""),
            ]);
            let mut game = state(vec![0], vec![2, 3]);
            if has_creature {
                game.battlefield
                    .push(new_perm_with(7, &cards, CardIdx(1), 0, false));
            }
            let cost = 1 + u32::from(dispute);
            let mut pool = ManaPool {
                fixed: [0, 0, cost, 0, 0],
                ..Default::default()
            };
            assert_eq!(
                attempt(&cards, &mut game, &mut pool),
                has_creature.then_some(cost)
            );
            assert_eq!(
                game.graveyard,
                if has_creature {
                    vec![CardIdx(1), CardIdx(0)]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                game.hand,
                if has_creature {
                    vec![CardIdx(3), CardIdx(2)]
                } else {
                    vec![CardIdx(0)]
                }
            );
            assert_eq!(game.library.len(), if has_creature { 0 } else { 2 });
            assert!(game.battlefield.is_empty());
            assert_eq!(game.treasure_bank, u32::from(dispute && has_creature));
            assert_eq!(pool.total(), if has_creature { 0 } else { cost });
            assert_eq!(game.life, 20);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = changed.oracle_text.replace("Draw two cards", "Draw a card");
    let cards = deck(&[
        changed,
        row("Payment", "{0}", "Creature", ""),
        row("Unchanged bottom", "{99}", "Sorcery", ""),
        row("Single draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![2, 3]);
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(1), 0, false));
    let cost = 1 + u32::from(dispute);
    let mut pool = ManaPool {
        fixed: [0, 0, cost, 0, 0],
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(cost));
    assert_eq!(game.hand, [CardIdx(3)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(game.seen, 1);
    assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0)]);
    assert_eq!(game.treasure_bank, u32::from(dispute));
}

/// Dispute's supported creature route also enforces the artifact and bank-payment boundaries.
pub(super) fn assert_dispute_actions(source: &CardRow) {
    assert_sacrifice_draw(source);
    super::card_boundary_cost_tests::assert_dispute_cost_boundary(source);
    super::card_boundary_cost_tests::assert_treasure_bank_boundary(source);
}

/// Unequal draw/discard counts finish drawing before discarding the newest tied worst card.
pub(super) fn assert_sift(source: &CardRow) {
    for source in renamed(source) {
        for legal in [true, false] {
            let cards = sift_cards(&source);
            let mut game = state(vec![0, 1], vec![2, 3, 4, 5]);
            let before_rng = game.library_rng.clone();
            let mut pool = ManaPool {
                fixed: if legal {
                    [0, 1, 0, 0, 0]
                } else {
                    [0, 0, 0, 0, 1]
                },
                colorless: 3,
                ..Default::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), legal.then_some(4));
            assert_eq!(
                game.hand,
                if legal {
                    vec![CardIdx(1), CardIdx(5), CardIdx(4)]
                } else {
                    vec![CardIdx(0), CardIdx(1)]
                }
            );
            assert_eq!(
                game.graveyard,
                if legal {
                    vec![CardIdx(3), CardIdx(0)]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                game.library,
                if legal {
                    vec![CardIdx(2)]
                } else {
                    vec![CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(5)]
                }
            );
            assert_eq!(game.seen, if legal { 3 } else { 0 });
            assert_eq!(game.awareness_cards, if legal { 3 } else { 0 });
            assert_eq!(game.milled_self, 0);
            assert!(!game.failed_draw);
            assert_eq!(pool.total(), if legal { 0 } else { 4 });
            super::card_test_support::assert_library_rng(&game.library_rng, &before_rng);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = "Draw two cards, then discard a card.".into();
    let cards = sift_cards(&changed);
    let mut game = state(vec![0, 1], vec![2, 3, 4, 5]);
    let mut pool = ManaPool {
        flexible: 4,
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(4));
    assert_eq!(game.hand, [CardIdx(1), CardIdx(5)]);
    assert_eq!(game.graveyard, [CardIdx(4), CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(2), CardIdx(3)]);
    assert_eq!(
        (game.seen, game.awareness_cards, game.milled_self),
        (2, 2, 0)
    );
    assert!(!game.failed_draw);
}

/// Keep the old hand distinct from all four physical library cards.
fn sift_cards(source: &CardRow) -> SimDeck {
    let mut rows = vec![source.clone(), row("Old hand", "{99}", "Sorcery", "")];
    rows.extend((0..4).map(|_| row("Physical draw", "{99}", "Sorcery", "")));
    deck(&rows)
}

/// Graveyard search moves the selected physical card without awarding a draw or hand access.
pub(super) fn assert_graveyard_search(source: &CardRow) {
    for source in renamed(source) {
        for has_target in [true, false] {
            let cards = deck(&[source.clone(), row("Search result", "{99}", "Sorcery", "")]);
            let mut game = state(vec![0], if has_target { vec![1] } else { vec![] });
            if !has_target {
                game.exile.push(CardIdx(1));
            }
            let mut pool = ManaPool {
                flexible: 5,
                ..Default::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), Some(5));
            assert_eq!(
                game.graveyard,
                if has_target { vec![CardIdx(1)] } else { vec![] }
            );
            assert!(game.hand.is_empty() && game.library.is_empty());
            assert_eq!(
                game.exile,
                if has_target { vec![] } else { vec![CardIdx(1)] }
            );
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
            assert_eq!(
                game.graveyard_seen.get(&CardIdx(1)),
                has_target.then_some(&1)
            );
            assert!(!game.card_first_seen.contains_key(&1));
            assert_eq!(game.life, 20);
            assert_eq!(game.life_gained, 0);
            assert_eq!(pool.total(), 0);
            assert!(!game.failed_draw);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = changed
        .oracle_text
        .replace("into your graveyard", "into your hand");
    assert_ne!(changed.oracle_text, source.oracle_text);
    let cards = deck(&[changed, row("Changed destination", "{99}", "Sorcery", "")]);
    let mut game = state(vec![0], vec![1]);
    let mut pool = ManaPool {
        flexible: 5,
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(5));
    assert_eq!(game.hand, [CardIdx(1)]);
    assert!(game.graveyard.is_empty() && game.library.is_empty());
}

/// Rejected search casts do not advance the library random stream or settle pending work.
#[test]
fn rejected_attempt_preserves_rng_and_pending_trigger_work() {
    let cards = deck(&[
        named_row("Nature's Lore"),
        row("Library", "", "Land — Forest", ""),
        row(
            "Listener",
            "{0}",
            "Creature",
            "Whenever you cast a creature spell, draw a card.",
        ),
    ]);
    let mut game = state(vec![0], vec![1]);
    game.battlefield
        .push(new_perm_with(1, &cards, CardIdx(2), 0, false));
    crate::deck::simulator::resolution::begin_resolution(&mut game);
    crate::deck::simulator::cast_pass::fire_cast_triggers(
        &cards,
        &mut game,
        &mut ManaPool::default(),
        1,
        &cards[CardIdx(2)],
    );
    let mut expected_rng = game.library_rng.clone();
    let mut pool = ManaPool::default();
    assert_eq!(attempt(&cards, &mut game, &mut pool), None);
    assert_eq!(game.library_rng.next_u64(), expected_rng.next_u64());
    assert_eq!(game.hand, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(1)]);
    assert_eq!(game.seen, 0);
    crate::deck::simulator::resolution::end_resolution(&cards, &mut game, Some(&mut pool));
    assert_eq!(game.hand, [CardIdx(0), CardIdx(1)]);
    assert!(game.library.is_empty());
    assert_eq!(game.seen, 1);
}

/// Paying the last life cannot be rescued by a queued cast-trigger life gain.
#[test]
fn last_life_cast_cost_stops_before_queued_life_gain() {
    let cards = deck(&[
        row(
            "Life cost",
            "{0}",
            "Sorcery",
            "As an additional cost to cast this spell, pay 2 life.\nDraw a card.",
        ),
        row(
            "Listener",
            "{0}",
            "Enchantment",
            "Whenever you cast a spell, you gain 3 life.",
        ),
        row("Draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![2]);
    game.life = 2;
    game.battlefield
        .push(new_perm_with(1, &cards, CardIdx(1), 0, false));
    assert_eq!(
        attempt(&cards, &mut game, &mut ManaPool::default()),
        Some(0)
    );
    assert_eq!(game.life, 0);
    assert_eq!(game.life_paid, 2);
    assert_eq!(game.life_gained, 0);
    assert!(game.completion.is_some());
    assert_eq!(game.library, [CardIdx(2)]);
    assert!(game.hand.is_empty());
}

/// The last life paid for cycling ends the game before its draw or discard trigger resolves.
#[test]
fn last_life_cycling_cost_stops_before_draw_and_queued_life_gain() {
    let cards = deck(&[
        row("Life cycling", "{99}", "Sorcery", "Cycling—Pay 2 life."),
        row(
            "Discard listener",
            "{0}",
            "Enchantment",
            "Whenever you discard a card, you gain 3 life.",
        ),
        row("Undrawn", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![2]);
    game.life = 2;
    game.battlefield
        .push(new_perm_with(1, &cards, CardIdx(1), 0, false));
    let rng = game.library_rng.clone();
    let census = super::card_report_tests::execute_turn(&cards, &mut game);
    assert_eq!(game.life, 0);
    assert_eq!(game.life_paid, 2);
    assert_eq!(game.life_gained, 0);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert!(game.hand.is_empty());
    assert_eq!(game.seen, 0);
    assert_eq!(game.life_funded_draws, 0);
    assert_eq!(
        game.milestones_by_turn
            .get(&1)
            .map_or(0, |milestone| milestone.life_funded_draws),
        0
    );
    assert_eq!(
        game.completion,
        Some(super::game::GameCompletion::PlayerLost {
            reasons: vec![super::game::PlayerLossReason::ZeroLife],
            turn: 1,
            phase: super::game::GamePhase::Main,
        })
    );
    super::card_test_support::assert_library_rng(&game.library_rng, &rng);
    let (log, json) = cycling_boundary_report(&cards, game, census);
    assert_eq!(log.turns_observed, 0);
    assert_eq!(log.life_funded_draws, 0);
    assert_eq!(log.milestones_by_turn[0].life_funded_draws, 0);
    assert_eq!(json["outcomes"]["player_loss_games"], 1);
    assert_eq!(json["draw"]["avg_life_paid"], 2.0);
    assert_eq!(json["draw"]["avg_life_funded_draws"], 0.0);
    assert!(
        json["milestones"]["life_funded_draws_avg_by_turn"]
            .as_object()
            .expect("per-turn observations")
            .is_empty()
    );
}

/// Completed life-paid draws earn credit; failed draws still pay the complete cycling cost.
#[test]
fn cycling_life_draw_credit_requires_a_successful_draw() {
    for cost in ["Cycling—Pay 2 life.", "Cycling {B/P}"] {
        for available in [false, true] {
            let cards = deck(&[
                row("Synthetic paid cycler", "{99}", "Sorcery", cost),
                row("Physical cycling draw", "{99}", "Sorcery", ""),
            ]);
            let mut game = state(vec![0], if available { vec![1] } else { vec![] });
            game.life = 3;
            let rng = game.library_rng.clone();
            let census = super::card_report_tests::execute_turn(&cards, &mut game);
            assert_eq!(game.life, 1);
            assert_eq!(game.life_paid, 2);
            assert_eq!(game.hand, if available { vec![CardIdx(1)] } else { vec![] });
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert!(game.library.is_empty());
            assert!(game.exile.is_empty());
            assert_eq!(game.seen, u32::from(available));
            assert_eq!(game.life_funded_draws, u32::from(available));
            assert_eq!(game.failed_draw, !available);
            if !available {
                assert_eq!(
                    game.completion,
                    Some(super::game::GameCompletion::PlayerLost {
                        reasons: vec![super::game::PlayerLossReason::FailedDraw],
                        turn: 1,
                        phase: super::game::GamePhase::Main,
                    })
                );
            }
            super::card_test_support::assert_library_rng(&game.library_rng, &rng);
            let (log, json) = cycling_boundary_report(&cards, game, census);
            assert_eq!(log.life_funded_draws, u32::from(available));
            assert_eq!(
                log.milestones_by_turn[0].life_funded_draws,
                u32::from(available)
            );
            assert_eq!(log.mana_spent, [0.0]);
            assert_eq!(json["outcomes"]["player_loss_games"], u32::from(!available));
            assert_eq!(json["draw"]["avg_life_paid"], 2.0);
            assert_eq!(
                json["draw"]["avg_life_funded_draws"],
                f64::from(u32::from(available))
            );
            if available {
                assert_eq!(
                    json["milestones"]["life_funded_draws_avg_by_turn"]["1"],
                    1.0
                );
            } else {
                assert!(
                    json["milestones"]["life_funded_draws_avg_by_turn"]
                        .as_object()
                        .expect("per-turn observations")
                        .is_empty()
                );
            }
        }
    }
}

/// A life-paid land search is library access, not a life-funded draw.
#[test]
fn life_paid_landcycling_never_counts_as_a_funded_draw() {
    let cards = deck(&[
        row(
            "Synthetic life-paid landcycler",
            "{99}",
            "Sorcery",
            "Forestcycling {B/P}",
        ),
        row("Physical Forest target", "", "Land — Forest", ""),
        row("Unsearched card", "{99}", "Artifact", ""),
    ]);
    let mut game = state(vec![0], vec![2, 1]);
    game.life = 3;
    let rng = game.library_rng.clone();
    let census = super::card_report_tests::execute_turn(&cards, &mut game);
    assert_eq!(game.life, 1);
    assert_eq!(game.life_paid, 2);
    assert_eq!(game.life_funded_draws, 0);
    assert_eq!(game.seen, 1);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert!(game.hand.is_empty());
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(1)));
    super::card_test_support::assert_library_rng(&game.library_rng, &rng);
    let (log, json) = cycling_boundary_report(&cards, game, census);
    assert_eq!(log.life_funded_draws, 0);
    assert_eq!(log.card_first_seen.get(&1), Some(&1));
    assert_eq!(log.milestones_by_turn[0].life_funded_draws, 0);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 1.0);
    assert_eq!(json["draw"]["avg_life_paid"], 2.0);
    assert_eq!(json["draw"]["avg_life_funded_draws"], 0.0);
    assert_eq!(
        json["milestones"]["life_funded_draws_avg_by_turn"]["1"],
        0.0
    );
}

/// Preserve partial turn observations when cycling ends the game at a loss boundary.
fn cycling_boundary_report(
    cards: &SimDeck,
    game: GameState,
    census: super::game_run::TurnCensus,
) -> (super::game::GameLog, serde_json::Value) {
    let config = game.opponents.config.clone();
    let log = super::game_run::census::finish_log(cards, game, census, 0, false, 1);
    let stats = super::aggregate::aggregate(std::slice::from_ref(&log), cards, 1);
    let mut report = super::report::json_report(
        &stats,
        cards,
        "Cycling payment boundary",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    super::report::set_opponent_model(&mut report, cards.format, config, &stats);
    let json = serde_json::to_value(report).expect("production cycling report serializes");
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    (log, json)
}
