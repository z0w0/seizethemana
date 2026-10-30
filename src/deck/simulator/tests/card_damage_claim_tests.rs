//! Pinned damage branches use physical actions and executed-turn reports.
//!
//! These tests certify only the stated entry, death, and inert-effect branches.
//! Combat output is potential. None of these tests makes a whole-card claim.

use super::card_report_tests::{execute_turn, report_executed_turn};
use super::card_runtime_tests::{attempt, attempt_index, renamed};
use super::card_test_support::{main_phase_state, named_row, pinned_revision, source_cards};
use super::game::{GamePhase, GameState, ManaPool, new_perm_with};
use super::game_effects::spend_leftover;
use super::model::{CardIdx, Format, SimDeck, SimEffect};
use super::opponents::{OpponentTable, pay_cumulative_upkeep};
use super::oracle_ast::{DamageTarget, OracleAbility, OracleEffect};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row};
use super::support::card_source_hash;
use crate::db::CardRow;
use serde_json::Value;

/// Preserve the exact selected creature-entry damage clause.
const IMPACT_TEXT: &str =
    "Whenever a creature you control enters, this enchantment deals 1 damage to each opponent.";

/// Preserve the selected death clause without certifying Cauldron's activation.
const CAULDRON_TEXT: &str =
    "Whenever a creature you control dies, each opponent loses 1 life and you gain 1 life.";

/// Use three fixed seats while disabling scheduled opponent activity, not effect targets.
pub(super) fn damage_state(hand: Vec<usize>, library: Vec<usize>) -> GameState {
    let mut game = main_phase_state(hand, library);
    game.life = 40;
    game.opponents = OpponentTable::for_format(Format::Commander);
    game.opponents.config.active = false;
    game.opponents.seats[1].life = 35;
    game.opponents.seats[2].life = 1;
    game.opponents.seats[2].active = false;
    game.next_uid = 100;
    game
}

/// Compare every seat, including an inactive seat with a lower life total.
pub(super) fn assert_seat_life(game: &GameState, expected: [i32; 3]) {
    assert_eq!(
        game.opponents
            .seats
            .iter()
            .map(|seat| seat.life)
            .collect::<Vec<_>>(),
        expected
    );
}

/// Match the immutable identity, input hash, wording, and actual selection tag.
fn assert_pinned_source(source: &CardRow, wording: &str) {
    let revision = pinned_revision(source);
    assert_eq!(revision["flattened_input_sha256"], card_source_hash(source));
    assert!(source.oracle_text.lines().any(|line| line == wording));
    let selected = source_cards()
        .into_iter()
        .find(|card| card["metadata"]["oracle_id"] == source.oracle_id)
        .expect("the pinned source has capture metadata");
    assert_eq!(selected["metadata_sha256"], revision["metadata_sha256"]);
    assert_eq!(
        selected["oracle_text_sha256"],
        revision["oracle_text_sha256"]
    );
    assert_eq!(
        selected["selection_tags"],
        serde_json::json!(["edhrec_top250"])
    );
}

/// Read one exact compiler record without turning its status into runtime evidence.
fn assert_compiler_clause(source: &CardRow, wording: &str, status: &str, reason: &str) {
    let card = parse_sim_card(source);
    assert_eq!(card.source_hash, card_source_hash(source));
    let ability = card
        .support
        .iter()
        .find(|ability| ability.source == wording)
        .expect("the exact source clause has a compiler record");
    let actual = serde_json::to_value(&ability.status).expect("compiler status serializes");
    assert_eq!(actual["status"], status);
    assert!(
        actual["reason"]
            .as_str()
            .expect("the record explains its limit")
            .contains(reason)
    );
    let span = ability.span.expect("a printed clause has a source span");
    assert_eq!(&source.oracle_text[span.start..span.end], wording);
}

/// Locate a report clause and require its exact compiler wording and source hash.
fn report_clause<'a>(report: &'a Value, source: &CardRow, wording: &str) -> &'a Value {
    let card = report["card_support"]
        .as_array()
        .expect("card support is a list")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("the source is reported");
    assert_eq!(card["source_hash"], card_source_hash(source));
    assert!(
        card["note"]
            .as_str()
            .expect("compiler note")
            .contains("Compiler evidence only")
    );
    card["abilities"]
        .as_array()
        .expect("ability support is a list")
        .iter()
        .find(|ability| ability["source"] == wording)
        .expect("the exact clause is reported")
}

/// Require separate damage and life-loss measures and an honest threshold note.
pub(super) fn assert_output_report(report: &Value, damage: u32, loss: u32) {
    assert_eq!(
        report["combat"]["player_damage_avg_by_turn"]["1"],
        f64::from(damage)
    );
    assert_eq!(
        report["win_conditions"]["opponent_life_loss_by_turn"]["1"],
        f64::from(loss)
    );
    assert_eq!(
        report["win_conditions"]["percent_games_at_or_above_table_life_by_turn"]["1"],
        0.0
    );
    assert_eq!(report["win_conditions"]["damage_threshold_games"], 0);
    assert!(report["win_conditions"]["p50_lethal_turn"].is_null());
    assert!(
        report["win_conditions"]["lethal_note"]
            .as_str()
            .expect("threshold scope")
            .contains("not a win-rate estimate")
    );
    let assumptions = report["assumptions"]
        .as_array()
        .expect("model limits are reported");
    for text in [
        "Creature damage, ambiguous targets, prevention",
        "not a win-rate model",
        "Commander combat and table damage thresholds remain potential measures",
    ] {
        assert!(
            assumptions
                .iter()
                .any(|value| value.as_str().is_some_and(|line| line.contains(text))),
            "missing model limit: {text}"
        );
    }
}

/// Keep report rules and its three-seat starting-life threshold consistent.
pub(super) fn commander_rules(cards: &mut SimDeck) {
    cards.format = Format::Commander;
    cards.rules = super::format::rules_for("commander");
}

/// Impact Tremors responds to an actual creature entry, not an artifact or hidden source.
pub(super) fn assert_impact_tremors_actions(source: &CardRow) {
    assert_pinned_source(source, IMPACT_TEXT);
    assert_compiler_clause(source, IMPACT_TEXT, "compiler_only", "Event filters");
    for input in renamed(source) {
        for (kind, matching) in [("Creature", true), ("Artifact", false)] {
            for (hidden, live) in [(false, true), (true, true), (false, false)] {
                let cards = deck(&[
                    input.clone(),
                    row("Physical entry", "{1}", kind, ""),
                    row("Library marker", "{99}", "Sorcery", ""),
                ]);
                let mut game = damage_state(vec![1], vec![2]);
                let mut permanent = new_perm_with(10, &cards, CardIdx(0), 0, false);
                permanent.face_down = hidden;
                game.battlefield.push(permanent);
                if !live {
                    for seat in &mut game.opponents.seats {
                        seat.active = false;
                    }
                }
                let mut pool = ManaPool {
                    colorless: 1,
                    ..Default::default()
                };
                assert_eq!(
                    attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
                    Some(1)
                );
                let fired = matching && !hidden && live;
                assert_eq!(pool, ManaPool::default());
                assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
                assert_eq!(game.library, [CardIdx(2)]);
                assert_eq!(game.battlefield.len(), 2);
                assert_eq!(game.battlefield[0].uid, 10);
                assert_eq!(game.battlefield[1].card.deck_idx(), Some(CardIdx(1)));
                assert_eq!(game.battlefield[1].uid, 101);
                assert_eq!(game.next_uid, 101);
                assert_eq!(game.damage_dealt_this_turn, 2 * u32::from(fired));
                assert_eq!(game.opponent_life_lost, 0);
                assert_eq!(game.life, 40);
                assert_eq!(game.life_paid, 0);
                assert_eq!(game.life_gained, 0);
                assert_seat_life(&game, if fired { [39, 34, 1] } else { [40, 35, 1] });
                assert!(game.completion.is_none());
            }
        }
    }
}

/// Impact Tremors' actual entry damage reaches the report without becoming combat power.
pub(super) fn assert_impact_tremors_report(source: &CardRow) {
    assert_pinned_source(source, IMPACT_TEXT);
    for (kind, matching) in [("Creature", true), ("Artifact", false)] {
        for live in [true, false] {
            let mut cards = deck(&[
                source.clone(),
                row("Physical entry", "{1}", kind, ""),
                row("Funding land", "", "Basic Land", "{T}: Add {C}."),
                row("Draw marker", "{99}", "Sorcery", ""),
            ]);
            commander_rules(&mut cards);
            let mut game = damage_state(vec![1], vec![3]);
            for index in [0, 2] {
                game.battlefield
                    .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
            }
            if !live {
                for seat in &mut game.opponents.seats {
                    seat.active = false;
                }
            }
            let census = execute_turn(&cards, &mut game);
            let damage = 2 * u32::from(matching && live);
            assert_eq!(game.hand, [CardIdx(3)]);
            assert_eq!(game.battlefield.len(), 3);
            assert_eq!(game.battlefield[2].card.deck_idx(), Some(CardIdx(1)));
            assert_eq!(game.life, 40);
            assert_eq!(game.damage_dealt_this_turn, damage);
            assert_eq!(game.opponent_life_lost, 0);
            assert_seat_life(&game, if damage > 0 { [39, 34, 1] } else { [40, 35, 1] });
            let (log, json) = report_executed_turn(&cards, game, census);
            assert_eq!(log.player_damage, [damage]);
            assert_eq!(log.opponent_life_loss, [0]);
            assert_eq!(log.attack_power, [0]);
            assert_eq!(log.mana_spent, [1.0]);
            assert_output_report(&json, damage, 0);
            assert_eq!(json["combat"]["attack_power_avg_by_turn"]["1"], 0.0);
            assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 1.0);
            let clause = report_clause(&json, source, IMPACT_TEXT);
            assert_eq!(clause["status"], "compiler_only");
            assert!(
                clause["reason"]
                    .as_str()
                    .expect("filter audit limit")
                    .contains("Event filters")
            );
        }
    }
}

/// Cauldron checks the exact victim's type after its paid entry and unpaid upkeep.
pub(super) fn assert_cauldron_life_loss_actions(source: &CardRow) {
    assert_pinned_source(source, CAULDRON_TEXT);
    assert_compiler_clause(source, CAULDRON_TEXT, "compiler_only", "Event filters");
    for input in renamed(source) {
        for (kind, matching) in [("Creature", true), ("Artifact", false)] {
            for live in [true, false] {
                let cards = deck(&[
                    input.clone(),
                    row("Physical victim", "{1}", kind, "Cumulative upkeep {99}"),
                    row("Library marker", "{99}", "Sorcery", ""),
                ]);
                let mut game = damage_state(vec![1], vec![2]);
                game.battlefield
                    .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
                if !live {
                    for seat in &mut game.opponents.seats {
                        seat.active = false;
                    }
                }
                let mut pool = ManaPool {
                    colorless: 1,
                    ..Default::default()
                };
                assert_eq!(
                    attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
                    Some(1)
                );
                assert_eq!(game.opponent_life_lost, 0);
                assert_eq!(game.life_gained, 0);
                assert_seat_life(&game, [40, 35, 1]);
                let victim_uid = game.battlefield[1].uid;
                assert_eq!(game.battlefield[1].card.deck_idx(), Some(CardIdx(1)));
                game.current_turn = 2;
                game.current_phase = GamePhase::Upkeep;
                pay_cumulative_upkeep(&cards, &mut game, 2);
                assert_eq!(pool, ManaPool::default());
                assert_eq!(game.battlefield.len(), 1);
                assert_eq!(game.battlefield[0].uid, 10);
                assert!(
                    game.battlefield
                        .iter()
                        .all(|permanent| permanent.uid != victim_uid)
                );
                assert_eq!(game.graveyard, [CardIdx(1)]);
                assert_eq!(game.library, [CardIdx(2)]);
                assert!(game.hand.is_empty() && game.exile.is_empty());
                assert_eq!(game.damage_dealt_this_turn, 0);
                assert_eq!(game.opponent_life_lost, 2 * u32::from(matching && live));
                assert_eq!(game.life, 40 + i32::from(matching));
                assert_eq!(game.life_gained, u32::from(matching));
                assert_eq!(game.life_paid, 0);
                assert_seat_life(
                    &game,
                    if matching && live {
                        [39, 34, 1]
                    } else {
                        [40, 35, 1]
                    },
                );
                assert!(game.completion.is_none());
            }
        }
    }
}

/// An unpaid upkeep causes a physical death inside the turn, then Cauldron reports only life loss.
pub(super) fn assert_cauldron_life_loss_report(source: &CardRow) {
    assert_pinned_source(source, CAULDRON_TEXT);
    for (kind, matching) in [("Creature", true), ("Artifact", false)] {
        for live in [true, false] {
            let mut cards = deck(&[
                source.clone(),
                row(
                    "Unpaid upkeep victim",
                    "{1}",
                    kind,
                    "Cumulative upkeep {99}",
                ),
                row("Draw marker", "{99}", "Sorcery", ""),
            ]);
            commander_rules(&mut cards);
            let mut game = damage_state(vec![], vec![2]);
            for index in [0, 1] {
                game.battlefield
                    .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
            }
            if !live {
                for seat in &mut game.opponents.seats {
                    seat.active = false;
                }
            }
            let census = execute_turn(&cards, &mut game);
            let loss = 2 * u32::from(matching && live);
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].uid, 10);
            assert_eq!(game.graveyard, [CardIdx(1)]);
            assert_eq!(game.hand, [CardIdx(2)]);
            assert_eq!(game.life, 40 + i32::from(matching));
            assert_eq!(game.damage_dealt_this_turn, 0);
            assert_eq!(game.opponent_life_lost, loss);
            assert_seat_life(&game, if loss > 0 { [39, 34, 1] } else { [40, 35, 1] });
            let (log, json) = report_executed_turn(&cards, game, census);
            assert_eq!(log.player_damage, [0]);
            assert_eq!(log.opponent_life_loss, [loss]);
            assert_eq!(log.life_gained, u32::from(matching));
            assert_eq!(log.life_paid, 0);
            assert_eq!(log.graveyard_size, [1]);
            assert_output_report(&json, 0, loss);
            assert_eq!(
                json["draw"]["avg_life_gained"],
                f64::from(u32::from(matching))
            );
            assert_eq!(json["draw"]["avg_life_paid"], 0.0);
            assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 1.0);
            let clause = report_clause(&json, source, CAULDRON_TEXT);
            assert_eq!(clause["status"], "compiler_only");
            assert!(
                clause["reason"]
                    .as_str()
                    .expect("filter audit limit")
                    .contains("Event filters")
            );
        }
    }
}

/// Pin Bolt's parsed AnyTarget without granting it an executable spell effect.
fn assert_bolt_compiler(source: &CardRow) {
    let text = "Lightning Bolt deals 3 damage to any target.";
    assert_pinned_source(source, text);
    assert_compiler_clause(source, text, "unsupported", "required clause");
    let oracle = parse_oracle_card(source);
    assert!(matches!(&oracle.abilities[0], OracleAbility::Spell(ability)
        if matches!(ability.effects.as_slice(), [OracleEffect::Damage { amount: 3, target: DamageTarget::AnyTarget, .. }])));
    let card = parse_sim_card(source);
    assert!(card.spell_data.effects.is_empty());
    assert_eq!(card.spell_data.damage_on_resolve, 0);
    assert_eq!(card.spell_data.life_loss_on_resolve, 0);
}

/// A paid ordinary Bolt cast consumes red mana and the spell, but removes nothing and deals no damage.
pub(super) fn assert_lightning_bolt_safety_actions(source: &CardRow) {
    assert_bolt_compiler(source);
    for input in renamed(source) {
        for legal in [true, false] {
            let cards = deck(&[
                input.clone(),
                row("Creature marker", "{99}", "Creature", ""),
                row("Artifact marker", "{99}", "Artifact", ""),
                row("Library marker", "{99}", "Sorcery", ""),
                row("Graveyard marker", "{99}", "Sorcery", ""),
                row("Exile marker", "{99}", "Sorcery", ""),
            ]);
            let mut game = damage_state(vec![0], vec![3]);
            game.graveyard.push(CardIdx(4));
            game.exile.push(CardIdx(5));
            for index in [1, 2] {
                game.battlefield
                    .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
            }
            game.opponents.seats[0].creatures = 1;
            let before_seats = game.opponents.seats.clone();
            let mut pool = ManaPool::default();
            pool.fixed[if legal { 3 } else { 4 }] = 1;
            let before_pool = pool.clone();
            assert_eq!(attempt(&cards, &mut game, &mut pool), legal.then_some(1));
            assert_eq!(
                pool,
                if legal {
                    ManaPool::default()
                } else {
                    before_pool
                }
            );
            assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(0)] });
            assert_eq!(
                game.graveyard,
                if legal {
                    vec![CardIdx(4), CardIdx(0)]
                } else {
                    vec![CardIdx(4)]
                }
            );
            assert_eq!(game.exile, [CardIdx(5)]);
            assert_eq!(game.library, [CardIdx(3)]);
            assert_eq!(game.battlefield.len(), 2);
            assert_eq!(game.battlefield[0].uid, 11);
            assert_eq!(game.battlefield[1].uid, 12);
            assert_eq!(game.opponents.seats, before_seats);
            assert_eq!(game.damage_dealt_this_turn, 0);
            assert_eq!(game.opponent_life_lost, 0);
            assert_eq!(game.life, 40);
            assert_eq!(game.life_paid, 0);
            assert_eq!(game.life_gained, 0);
            assert_eq!(game.spells_cast_this_turn, u32::from(legal));
            assert!(game.completion.is_none());
        }
    }
}

/// The normal pilot holds Bolt as interaction; its executed report must not invent a cast or damage.
pub(super) fn assert_lightning_bolt_safety_report(source: &CardRow) {
    assert_bolt_compiler(source);
    let cards = deck(&[
        source.clone(),
        row("Funding Mountain", "", "Basic Land", "{T}: Add {R}."),
        row("Creature marker", "{99}", "Creature", ""),
    ]);
    let mut game = damage_state(vec![0], vec![]);
    for index in [1, 2] {
        game.battlefield
            .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
    }
    game.battlefield[1].face_down = true;
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.hand, [CardIdx(0)]);
    assert!(game.graveyard.is_empty());
    assert_eq!(game.battlefield.len(), 2);
    assert_eq!(game.life, 40);
    assert_seat_life(&game, [40, 35, 1]);
    let (log, json) = report_executed_turn(&cards, game, census);
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(log.opponent_life_loss, [0]);
    assert_eq!(log.player_damage, [2]);
    assert_eq!(log.attack_power, [2]);
    assert_output_report(&json, 2, 0);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 0.0);
    assert_eq!(
        json["interaction"]["percent_games_with_ready_interaction_by_turn"]["1"],
        100.0
    );
    let clause = report_clause(&json, source, &source.oracle_text);
    assert_eq!(clause["status"], "unsupported");
    assert!(
        clause["reason"]
            .as_str()
            .expect("target gap")
            .contains("required clause")
    );
}

/// Pin Zacama's creature target and ensure that only its separate life-gain activation lowers.
fn assert_zacama_compiler(source: &CardRow) {
    let text = "{2}{R}: Zacama deals 3 damage to target creature.";
    assert_pinned_source(source, text);
    assert_compiler_clause(source, text, "unsupported", "complete ability");
    let oracle = parse_oracle_card(source);
    assert!(oracle.abilities.iter().any(|ability| matches!(ability, OracleAbility::Activated(ability)
        if matches!(ability.effects.as_slice(), [OracleEffect::Damage { amount: 3, target: DamageTarget::Permanent, .. }]))));
    let card = parse_sim_card(source);
    assert!(!card.unlocked_abilities(0).any(|ability| {
        ability
            .effect_sequence()
            .iter()
            .any(|effect| matches!(effect, SimEffect::Damage { .. }))
    }));
    assert!(
        card.unlocked_abilities(0)
            .any(|ability| matches!(ability.effect_sequence(), [SimEffect::GainLife(3)]))
    );
}

/// Zacama can pay its ordinary cast, but a red-only activation budget cannot damage or remove a creature.
pub(super) fn assert_zacama_damage_safety_actions(source: &CardRow) {
    assert_zacama_compiler(source);
    for input in renamed(source) {
        let cards = deck(&[
            input.clone(),
            row("Creature marker", "{99}", "Creature", ""),
            row("Library marker", "{99}", "Sorcery", ""),
        ]);
        let mut game = damage_state(vec![0], vec![2]);
        game.battlefield
            .push(new_perm_with(10, &cards, CardIdx(1), 0, false));
        game.opponents.seats[0].creatures = 1;
        let before_seats = game.opponents.seats.clone();
        let mut pool = ManaPool {
            colorless: 6,
            fixed: [1, 0, 0, 1, 1],
            ..Default::default()
        };
        assert_eq!(attempt(&cards, &mut game, &mut pool), Some(9));
        assert_eq!(pool, ManaPool::default());
        assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.battlefield.len(), 2);
        assert_eq!(game.battlefield[0].uid, 10);
        assert_eq!(game.battlefield[1].card.deck_idx(), Some(CardIdx(0)));
        assert!(game.battlefield[1].summoning_sick);
        pool.fixed[3] = 3;
        let budget = pool.clone();
        spend_leftover(&cards, &mut game, &mut pool, 1);
        assert_eq!(pool, budget);
        assert_eq!(game.opponents.seats, before_seats);
        assert_eq!(game.battlefield.len(), 2);
        assert!(game.activated_this_turn.is_empty());
        assert_eq!(game.library, [CardIdx(2)]);
        assert_eq!(game.damage_dealt_this_turn, 0);
        assert_eq!(game.opponent_life_lost, 0);
        assert_eq!(game.life_gained, 0);
        assert_eq!(game.life, 40);
        assert!(game.completion.is_none());
    }
}

/// Zacama's mature combat capacity stays separate from applied life and its inert red activation.
pub(super) fn assert_zacama_damage_safety_report(source: &CardRow) {
    assert_zacama_compiler(source);
    for live in [true, false] {
        let mut rows = vec![source.clone(), row("Draw marker", "{99}", "Sorcery", "")];
        rows.extend((0..3).map(|_| row("Funding Mountain", "", "Basic Land", "{T}: Add {R}.")));
        let mut cards = deck(&rows);
        commander_rules(&mut cards);
        let mut game = damage_state(vec![], vec![1]);
        for index in [0, 2, 3, 4] {
            game.battlefield
                .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
        }
        game.opponents.seats[0].creatures = 1;
        if !live {
            for seat in &mut game.opponents.seats {
                seat.active = false;
            }
        }
        let before_seats = game.opponents.seats.clone();
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.battlefield.len(), 4);
        assert!(game.graveyard.is_empty());
        assert_eq!(game.hand, [CardIdx(1)]);
        assert_eq!(game.opponents.seats, before_seats);
        assert_eq!(game.damage_dealt_this_turn, 0);
        assert_eq!(game.opponent_life_lost, 0);
        assert_eq!(game.life_gained, 0);
        assert_eq!(game.life, 40);
        assert_eq!(game.attacked_this_turn, live);
        assert!(!game.battlefield[0].tapped);
        assert!(game.activated_this_turn.is_empty());
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_eq!(log.attack_power, [9]);
        assert_eq!(log.player_damage, [9]);
        assert_eq!(log.opponent_life_loss, [0]);
        assert_eq!(log.mana_spent, [0.0]);
        assert_eq!(log.mana_remaining, [3.0]);
        assert_output_report(&json, 9, 0);
        assert_eq!(json["combat"]["attack_power_avg_by_turn"]["1"], 9.0);
        assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], 3.0);
        assert_eq!(json["combat"]["commander_potential"], serde_json::json!([]));
        let clause = report_clause(
            &json,
            source,
            "{2}{R}: Zacama deals 3 damage to target creature.",
        );
        assert_eq!(clause["status"], "unsupported");
        assert!(
            clause["reason"]
                .as_str()
                .expect("creature target gap")
                .contains("complete ability")
        );
    }
}

/// Run both source-bound entry damage evidence stages.
#[test]
fn pinned_impact_tremors_actions_and_report() {
    let source = named_row("Impact Tremors");
    assert_impact_tremors_actions(&source);
    assert_impact_tremors_report(&source);
}

/// Run both source-bound death life-loss evidence stages.
#[test]
fn pinned_cauldron_life_loss_actions_and_report() {
    let source = named_row("Cauldron of Essence");
    assert_cauldron_life_loss_actions(&source);
    assert_cauldron_life_loss_report(&source);
}

/// Keep a paid Bolt cast distinct from the pilot's held-interaction report.
#[test]
fn pinned_lightning_bolt_paid_safety_and_report() {
    let source = named_row("Lightning Bolt");
    assert_lightning_bolt_safety_actions(&source);
    assert_lightning_bolt_safety_report(&source);
}

/// A lowered life-gain activation cannot make Zacama's unsupported damage execute.
#[test]
fn pinned_zacama_damage_safety_and_report() {
    let source = named_row("Zacama, Primal Calamity");
    assert_zacama_damage_safety_actions(&source);
    assert_zacama_damage_safety_report(&source);
}
