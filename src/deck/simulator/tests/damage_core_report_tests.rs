//! Synthetic scope and report controls are not pinned card certificates.
//!
//! Every report comes from executed turns and the production census and log.
//! Target-player effects use the current deterministic opponent choice. These
//! tests do not add player target choice, multiplayer priority, or win rates.

use super::aggregate::aggregate;
use super::card_damage_claim_tests::{
    assert_output_report, assert_seat_life, commander_rules, damage_state,
};
use super::game::{
    GameCompletion, GameLog, GamePhase, GameState, PlayerLossReason, new_commander_perm,
    new_perm_with,
};
use super::game_commander::CommanderProfile;
use super::game_run::{TurnCensus, census::finish_log, run_turn};
use super::model::{CardIdx, SimDeck, SimEffect};
use super::oracle_lower::parse_sim_card;
use super::scenario_test_support::{deck, row};
use serde_json::Value;

/// Run a real turn with enough census slots for each declared commander.
fn execute(cards: &SimDeck, game: &mut GameState) -> TurnCensus {
    let mut census = TurnCensus::with_commander_count(1, cards.cards.len(), cards.commanders.len());
    run_turn(
        cards,
        game,
        &mut census,
        &CommanderProfile::new(cards),
        &mut Vec::new(),
        1,
        false,
    );
    census
}

/// Report only production observations, including a supported loss before end of turn.
fn executed_report(cards: &SimDeck, game: GameState, census: TurnCensus) -> (GameLog, Value) {
    let config = game.opponents.config.clone();
    let log = finish_log(cards, game, census, 0, false, 1);
    let stats = aggregate(std::slice::from_ref(&log), cards, 1);
    let mut report = super::report::json_report(
        &stats,
        cards,
        "Synthetic damage scope",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    super::report::set_opponent_model(&mut report, cards.format, config, &stats);
    let json = serde_json::to_value(report).expect("the executed report serializes");
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    for card in cards.cards.iter().chain(&cards.commanders) {
        let record = json["card_support"]
            .as_array()
            .expect("card compiler records")
            .iter()
            .find(|record| record["name"] == card.name)
            .expect("the compiled source is reported");
        assert_eq!(record["source_hash"], card.source_hash);
        let abilities = record["abilities"]
            .as_array()
            .expect("ability compiler records");
        assert_eq!(abilities.len(), card.support.len());
        for (actual, expected) in abilities.iter().zip(&card.support) {
            assert_eq!(actual["source"], expected.source);
            let status =
                serde_json::to_value(&expected.status).expect("compiler status serializes");
            for key in ["status", "reason", "rule_id"] {
                assert_eq!(actual[key], status[key]);
            }
        }
    }
    (log, json)
}

/// Build a paid synthetic spell and real colored sources without changing lowered data.
fn spell_scenario(text: &str, cost: &str, sources: u32) -> (SimDeck, GameState) {
    let mut rows = vec![
        row("Scope source", cost, "Sorcery", text),
        row("Draw marker", "{99}", "Sorcery", ""),
    ];
    let yield_text = if cost.contains('B') {
        "{T}: Add {B}."
    } else {
        "{T}: Add {R}."
    };
    rows.extend((0..sources).map(|index| {
        row(
            &format!("Funding land {index}"),
            "",
            "Basic Land",
            yield_text,
        )
    }));
    let mut cards = deck(&rows);
    commander_rules(&mut cards);
    let mut game = damage_state(vec![0], vec![1]);
    for index in 2..sources + 2 {
        game.battlefield
            .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
    }
    (cards, game)
}

/// Select a source record by name because public reports sort their card rows.
fn reported_ability<'a>(report: &'a Value, name: &str) -> &'a Value {
    let card = report["card_support"]
        .as_array()
        .expect("card compiler records")
        .iter()
        .find(|card| card["name"] == name)
        .expect("the source has a compiler record");
    &card["abilities"][0]
}

/// Synthetic self, single-target, and table scopes keep separate counters and actual seat life.
pub(super) fn assert_damage_life_scope_reports() {
    for damage in [true, false] {
        for (recipient, player_charged, targeted, each) in [
            ("you", true, false, false),
            ("target player", false, true, false),
            ("target opponent", false, true, false),
            ("each opponent", false, false, true),
            ("each player", true, false, true),
        ] {
            for live in [true, false] {
                let text = if damage {
                    format!("Scope source deals 3 damage to {recipient}.")
                } else if recipient == "you" {
                    "You lose 3 life.".into()
                } else {
                    format!("{recipient} loses 3 life.")
                };
                let (cards, mut game) = spell_scenario(&text, "{R}", 1);
                if !live {
                    for seat in &mut game.opponents.seats {
                        seat.active = false;
                    }
                }
                let census = execute(&cards, &mut game);
                let output = if !live {
                    0
                } else if targeted {
                    3
                } else if each {
                    6
                } else {
                    0
                };
                let expected_seats = if !live || player_charged && !each {
                    [40, 35, 1]
                } else if targeted {
                    [40, 32, 1]
                } else {
                    [37, 32, 1]
                };
                assert_seat_life(&game, expected_seats);
                assert_eq!(game.life, if player_charged { 37 } else { 40 });
                assert_eq!(game.damage_dealt_this_turn, if damage { output } else { 0 });
                assert_eq!(game.opponent_life_lost, if damage { 0 } else { output });
                assert_eq!(game.life_paid, 0);
                assert_eq!(game.life_gained, 0);
                assert_eq!(game.hand, [CardIdx(1)]);
                assert_eq!(game.graveyard, [CardIdx(0)]);
                assert!(game.library.is_empty() && game.exile.is_empty());
                assert_eq!(game.battlefield.len(), 1);
                assert!(game.battlefield[0].tapped);
                assert_eq!(game.spells_cast_this_turn, 1);
                assert!(game.completion.is_none());
                let (log, json) = executed_report(&cards, game, census);
                assert_eq!(log.completion, GameCompletion::HorizonReached);
                assert_eq!(log.turns_observed, 1);
                assert_eq!(log.mana_spent, [1.0]);
                assert_eq!(log.attack_power, [0]);
                assert_eq!(log.player_damage, [if damage { output } else { 0 }]);
                assert_eq!(log.opponent_life_loss, [if damage { 0 } else { output }]);
                assert_output_report(
                    &json,
                    if damage { output } else { 0 },
                    if damage { 0 } else { output },
                );
                assert_eq!(json["draw"]["avg_life_paid"], 0.0);
                assert_eq!(json["draw"]["avg_life_gained"], 0.0);
                assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 1.0);
                let ability = reported_ability(&json, "Scope source");
                assert_eq!(ability["source"], text);
                assert_eq!(ability["status"], "compiler_only");
                assert!(
                    ability["reason"]
                        .as_str()
                        .expect("spell audit limit")
                        .contains("ordinary runtime resolution path")
                );
            }
        }
    }
}

/// Paid X reaches the report at its supported single-player or each-opponent scope.
pub(super) fn assert_paid_x_scope_reports() {
    for damage in [true, false] {
        for (recipient, each) in [
            ("target player", false),
            ("target opponent", false),
            ("each opponent", true),
        ] {
            for live in [true, false] {
                let text = if damage {
                    format!("Scope source deals X damage to {recipient}.")
                } else {
                    format!("{recipient} loses X life.")
                };
                let (cards, mut game) =
                    spell_scenario(&text, if damage { "{X}{R}" } else { "{X}{B}" }, 4);
                if !live {
                    for seat in &mut game.opponents.seats {
                        seat.active = false;
                    }
                }
                let census = execute(&cards, &mut game);
                let output = if !live {
                    0
                } else if each {
                    6
                } else {
                    3
                };
                assert_seat_life(
                    &game,
                    if !live {
                        [40, 35, 1]
                    } else if each {
                        [37, 32, 1]
                    } else {
                        [40, 32, 1]
                    },
                );
                assert_eq!(game.life, 40);
                assert_eq!(game.damage_dealt_this_turn, if damage { output } else { 0 });
                assert_eq!(game.opponent_life_lost, if damage { 0 } else { output });
                assert_eq!(game.graveyard, [CardIdx(0)]);
                assert_eq!(game.hand, [CardIdx(1)]);
                assert_eq!(
                    game.battlefield
                        .iter()
                        .filter(|permanent| permanent.tapped)
                        .count(),
                    4
                );
                let (log, json) = executed_report(&cards, game, census);
                assert_eq!(log.mana_spent, [4.0]);
                assert_eq!(log.player_damage, [if damage { output } else { 0 }]);
                assert_eq!(log.opponent_life_loss, [if damage { 0 } else { output }]);
                assert_output_report(
                    &json,
                    if damage { output } else { 0 },
                    if damage { 0 } else { output },
                );
                assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 4.0);
                assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], 0.0);
            }
        }
    }
}

/// A complete spell can recover from temporary zero life before its supported loss check.
pub(super) fn assert_damage_resolution_boundaries() {
    for survives in [true, false] {
        let rider = if survives {
            "You gain 1 life."
        } else {
            "You lose 1 life."
        };
        let (cards, mut game) = spell_scenario(
            &format!("Scope source deals 3 damage to each player. {rider}"),
            "{R}",
            1,
        );
        game.life = 3;
        let census = execute(&cards, &mut game);
        assert_eq!(game.life, if survives { 1 } else { -1 });
        assert_eq!(game.damage_dealt_this_turn, 6);
        assert_eq!(game.opponent_life_lost, 0);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.life_gained, u32::from(survives));
        assert_seat_life(&game, [37, 32, 1]);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.hand, [CardIdx(1)]);
        if survives {
            assert!(game.completion.is_none());
        } else {
            assert_eq!(
                game.completion,
                Some(GameCompletion::PlayerLost {
                    reasons: vec![PlayerLossReason::ZeroLife],
                    turn: 1,
                    phase: GamePhase::Main
                })
            );
        }
        let (log, json) = executed_report(&cards, game, census);
        assert_eq!(log.player_damage, [6]);
        assert_eq!(log.opponent_life_loss, [0]);
        assert_eq!(log.life_paid, 0);
        assert_eq!(log.life_gained, u32::from(survives));
        assert_eq!(json["outcomes"]["player_loss_games"], u32::from(!survives));
        assert_eq!(
            json["phase_observations"]["end_turn_by_turn"]["1"],
            f64::from(u32::from(survives))
        );
        assert_eq!(
            json["draw"]["avg_life_gained"],
            f64::from(u32::from(survives))
        );
        if survives {
            assert_output_report(&json, 6, 0);
            assert_eq!(log.turns_observed, 1);
        } else {
            assert_eq!(log.turns_observed, 0);
            assert_eq!(json["phase_observations"]["main_by_turn"]["1"], 0.0);
            assert!(
                json["combat"]["player_damage_avg_by_turn"]
                    .as_object()
                    .expect("phase means")
                    .is_empty()
            );
            assert!(
                json["win_conditions"]["opponent_life_loss_by_turn"]
                    .as_object()
                    .expect("phase means")
                    .is_empty()
            );
        }
        assert_eq!(json["win_conditions"]["damage_threshold_games"], 0);
    }
    assert_seat_loss_and_potential_threshold();
}

/// Removing a lethal seat and crossing table life do not declare a simulated win.
fn assert_seat_loss_and_potential_threshold() {
    for (text, life, output, threshold) in [
        (
            "Scope source deals 3 damage to target opponent.",
            3,
            3,
            false,
        ),
        (
            "Scope source deals 60 damage to each opponent.",
            35,
            120,
            true,
        ),
    ] {
        let (cards, mut game) = spell_scenario(text, "{R}", 1);
        game.opponents.seats[1].life = life;
        let census = execute(&cards, &mut game);
        assert!(!game.opponents.seats[1].active);
        assert_eq!(game.opponents.seats[2].life, 1);
        assert!(!game.opponents.seats[2].active);
        assert_eq!(
            game.opponents.seats[0].life,
            if threshold { -20 } else { 40 }
        );
        assert_eq!(
            game.opponents.seats[1].life,
            if threshold { -25 } else { 0 }
        );
        assert_eq!(game.opponents.seats[0].active, !threshold);
        assert!(game.completion.is_none());
        let (log, json) = executed_report(&cards, game, census);
        assert_eq!(log.completion, GameCompletion::HorizonReached);
        assert_eq!(log.player_damage, [output]);
        assert_eq!(log.opponent_life_loss, [0]);
        assert_eq!(json["outcomes"]["player_loss_games"], 0);
        assert_eq!(
            json["win_conditions"]["damage_threshold_games"],
            u32::from(threshold)
        );
        assert_eq!(
            json["win_conditions"]["percent_games_at_or_above_table_life_by_turn"]["1"],
            if threshold { 100.0 } else { 0.0 }
        );
        assert_eq!(
            json["win_conditions"]["p50_lethal_turn"],
            if threshold {
                serde_json::json!(1)
            } else {
                Value::Null
            }
        );
        assert!(
            json["win_conditions"]["lethal_note"]
                .as_str()
                .expect("threshold note")
                .contains("not a win-rate estimate")
        );
    }
}

/// Unsupported recipients and prevention or replacement effects stay inert after actual paid entries.
pub(super) fn assert_unsupported_damage_boundaries() {
    for text in [
        "When this creature enters, Boundary source deals 3 damage to target creature.",
        "When this creature enters, Boundary source deals 3 damage to any target.",
        "When this creature enters, Boundary source deals 3 damage to target planeswalker.",
        "When this creature enters, Boundary source deals 3 damage to target battle.",
        "When this creature enters, Boundary source deals 3 damage.",
        "When this creature enters, prevent the next 3 damage that would be dealt to target player this turn.",
        "When this creature enters, if a source would deal damage to a player this turn, it deals twice that much damage instead.",
    ] {
        let mut source = row("Boundary source", "{R}", "Creature", text);
        source.power = Some("0".into());
        source.toughness = Some("3".into());
        let mut cards = deck(&[
            source,
            row("Funding land", "", "Basic Land", "{T}: Add {R}."),
            row("Draw marker", "{99}", "Sorcery", ""),
            row("Creature marker", "{99}", "Creature", ""),
        ]);
        commander_rules(&mut cards);
        let mut game = damage_state(vec![0], vec![2]);
        game.battlefield
            .push(new_perm_with(11, &cards, CardIdx(1), 0, false));
        let mut target = new_perm_with(13, &cards, CardIdx(3), 0, false);
        target.summoning_sick = true;
        target.entered_turn = 1;
        game.battlefield.push(target);
        game.opponents.seats[0].creatures = 1;
        let before_seats = game.opponents.seats.clone();
        assert!(!cards[CardIdx(0)].unlocked_abilities(0).any(|ability| {
            ability
                .effect_sequence()
                .iter()
                .any(|effect| matches!(effect, SimEffect::Damage { .. }))
        }));
        let census = execute(&cards, &mut game);
        assert_eq!(game.hand, [CardIdx(2)]);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.battlefield.len(), 3);
        assert_eq!(game.battlefield[1].uid, 13);
        assert_eq!(game.battlefield[2].card.deck_idx(), Some(CardIdx(0)));
        assert_eq!(game.opponents.seats, before_seats);
        assert_eq!(game.damage_dealt_this_turn, 0);
        assert_eq!(game.opponent_life_lost, 0);
        assert_eq!(game.life, 40);
        let (log, json) = executed_report(&cards, game, census);
        assert_eq!(log.mana_spent, [1.0]);
        assert_eq!(log.opponent_life_loss, [0]);
        assert_eq!(log.player_damage, [0]);
        assert_output_report(&json, 0, 0);
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 1.0);
        let clause = reported_ability(&json, "Boundary source");
        assert_eq!(clause["source"], text);
        assert_eq!(clause["status"], "unsupported");
        assert!(
            !clause["reason"]
                .as_str()
                .expect("an unsupported clause has a reason")
                .is_empty()
        );
    }
}

/// Ordinary and commander combat capacity cannot lower seat life or invent no-defender events.
pub(super) fn assert_combat_potential_report_boundaries() {
    for live in [true, false] {
        let mut ordinary = row("Ordinary attacker", "{99}", "Creature", "");
        ordinary.power = Some("4".into());
        ordinary.toughness = Some("4".into());
        let mut rows = vec![ordinary];
        rows.extend(
            (0..8).map(|index| row(&format!("Physical draw {index}"), "{99}", "Sorcery", "")),
        );
        let mut cards = deck(&rows);
        commander_rules(&mut cards);
        let mut commander = row(
            "Synthetic commander",
            "{99}",
            "Legendary Creature",
            "Lifelink\nWhenever this creature attacks, draw a card.\nWhenever this creature deals combat damage to a player, draw a card.",
        );
        commander.power = Some("21".into());
        commander.toughness = Some("21".into());
        cards.commanders.push(parse_sim_card(&commander));
        let mut game = damage_state(vec![], (1..=8).collect());
        game.commander_casts = vec![1];
        game.battlefield
            .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
        game.battlefield
            .push(new_commander_perm(11, 0, 0, 0, true, false));
        if !live {
            for seat in &mut game.opponents.seats {
                seat.active = false;
            }
        }
        let census = execute(&cards, &mut game);
        assert_seat_life(&game, [40, 35, 1]);
        assert_eq!(game.damage_dealt_this_turn, 0);
        assert_eq!(game.opponent_life_lost, 0);
        assert_eq!(game.life_gained, if live { 21 } else { 0 });
        assert_eq!(game.life, if live { 61 } else { 40 });
        assert_eq!(game.hand.len(), if live { 3 } else { 1 });
        assert_eq!(game.library.len(), if live { 5 } else { 7 });
        assert_eq!(game.attacked_this_turn, live);
        if !live {
            assert!(game.battlefield.iter().all(|permanent| !permanent.tapped));
        }
        assert!(game.completion.is_none());
        let (log, json) = executed_report(&cards, game, census);
        assert_eq!(log.completion, GameCompletion::HorizonReached);
        assert_eq!(log.attack_power, [25]);
        assert_eq!(log.player_damage, [25]);
        assert_eq!(log.commander_combat_damage, [vec![21]]);
        assert_eq!(log.opponent_life_loss, [0]);
        assert_output_report(&json, 25, 0);
        assert_eq!(json["combat"]["attack_power_avg_by_turn"]["1"], 25.0);
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            if live { 3.0 } else { 1.0 }
        );
        let potential = &json["combat"]["commander_potential"][0];
        assert_eq!(potential["damage_by_turn"]["1"], 21.0);
        assert_eq!(potential["cumulative_damage_by_turn"]["1"], 21.0);
        assert_eq!(potential["games_reaching_21"], 1);
        assert_eq!(potential["observed_games"], 1);
        assert_eq!(potential["percent_games_reaching_21"], 100.0);
        assert_eq!(potential["median_turn_to_21"], 1);
        assert!(
            potential["note"]
                .as_str()
                .expect("potential limit")
                .contains("not a kill or win probability")
        );
    }
}

/// Exercise the synthetic fixed-scope report contract through real casts.
#[test]
fn synthetic_fixed_damage_and_life_scopes_reach_reports() {
    assert_damage_life_scope_reports();
}

/// Exercise paid-X scope reports independently of existing action-only controls.
#[test]
fn synthetic_paid_x_damage_and_life_scopes_reach_reports() {
    assert_paid_x_scope_reports();
}

/// Keep whole-resolution life checks and diagnostic table thresholds separate.
#[test]
fn synthetic_damage_resolution_and_loss_boundaries_reach_reports() {
    assert_damage_resolution_boundaries();
}

/// A normal physical cast does not make unsupported damage clauses execute.
#[test]
fn synthetic_unsupported_damage_boundaries_reach_reports() {
    assert_unsupported_damage_boundaries();
}

/// Combat and commander thresholds stay potential even with no live defender.
#[test]
fn synthetic_combat_potential_and_no_defender_controls_reach_reports() {
    assert_combat_potential_report_boundaries();
}
