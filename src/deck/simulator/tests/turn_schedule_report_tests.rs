//! Synthetic phase and game-driver reports. Potential combat does not apply seat damage.

use crate::deck::simulator::aggregate::aggregate;
use crate::deck::simulator::card_test_support::assert_library_rng;
use crate::deck::simulator::game::{CardRef, GameCompletion, GameLog, new_commander_perm};
use crate::deck::simulator::game_run::run_game_with_config;
use crate::deck::simulator::library_core_report_tests::{executed_report, put_permanents};
use crate::deck::simulator::model::{CardIdx, Format, SimDeck, SimEffect, SimTrigger};
use crate::deck::simulator::opponents::{OpponentConfig, OpponentTable};
use crate::deck::simulator::oracle_ast::{
    OracleAbility, OracleEffect, OracleTriggerEvent, PlayerScope,
};
use crate::deck::simulator::oracle_lower::parse_sim_card;
use crate::deck::simulator::oracle_parser::parse_oracle_card;
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde_json::Value;

/// First-main draws can fund the land phase, while end-step draws must wait.
pub(super) fn assert_phase_boundary_report() {
    for early in [true, false] {
        let cards = deck(&[
            row(
                "First main listener",
                "{0}",
                "Enchantment",
                "At the beginning of your first main phase, draw a card.",
            ),
            row(
                "End listener",
                "{99}",
                "Enchantment",
                "At the beginning of your end step, draw a card.",
            ),
            row("Later Island", "", "Basic Land — Island", "{T}: Add {U}."),
            row("Earlier Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        ]);
        let mut game = state(if early { vec![] } else { vec![0] }, vec![2, 3]);
        if early {
            put_permanents(&cards, &mut game, &[0]);
        }
        put_permanents(&cards, &mut game, &[1]);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(game.hand, [CardIdx(if early { 2 } else { 3 })]);
        assert_eq!(game.library, if early { vec![] } else { vec![CardIdx(2)] });
        assert_eq!(log.land_drops, [u8::from(early)]);
        assert_eq!(log.cards_seen, [if early { 2 } else { 1 }]);
        assert_eq!(game.battlefield.len(), if early { 3 } else { 2 });
        if early {
            assert_eq!(game.battlefield[2].card, CardRef::Deck(CardIdx(3)));
            assert_eq!(game.battlefield[2].uid, 3);
        } else {
            assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(0)));
            assert_eq!(game.battlefield[1].uid, 2);
        }
        assert_library_rng(&game.library_rng, &rng);
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            if early { 2.0 } else { 1.0 }
        );
        assert_eq!(json["phase_observations"]["main_by_turn"]["1"], 1.0);
        assert_eq!(json["phase_observations"]["end_step_by_turn"]["1"], 1.0);
        assert_eq!(json["outcomes"]["player_loss_games"], 0);
    }
    assert_monarch_entry_timing();
}

/// A paid main-phase Monarch acquisition draws at that turn's end step, not its land phase.
fn assert_monarch_entry_timing() {
    for funded in [true, false] {
        let cards = deck(&[
            row(
                "Monarch source",
                "{1}",
                "Artifact",
                "When this artifact enters, you become the monarch.",
            ),
            row("Second land", "", "Basic Land — Island", "{T}: Add {U}."),
            row("First land", "", "Basic Land — Forest", "{T}: Add {G}."),
            row("Funding land", "", "Basic Land", "{T}: Add {C}."),
            row("End marker", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![0], vec![4, 1, 2]);
        if funded {
            put_permanents(&cards, &mut game, &[3]);
        }
        let rng = game.library_rng.clone();
        let (first, log, json) = executed_report(&cards, game.clone(), 1, vec![]);
        assert_eq!(first.is_monarch, funded);
        assert_eq!(
            first.hand,
            if funded {
                vec![CardIdx(2)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(
            first.library,
            if funded {
                vec![CardIdx(4), CardIdx(1)]
            } else {
                vec![CardIdx(4), CardIdx(1), CardIdx(2)]
            }
        );
        assert_eq!(log.land_drops, [0]);
        assert_eq!(log.mana_spent, [if funded { 1.0 } else { 0.0 }]);
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            f64::from(u32::from(funded))
        );
        assert_library_rng(&first.library_rng, &rng);
        if funded {
            let (second, log, json) = executed_report(&cards, game, 2, vec![]);
            assert_eq!(second.hand, [CardIdx(1), CardIdx(4)]);
            assert!(second.library.is_empty());
            assert_eq!(second.battlefield[2].card, CardRef::Deck(CardIdx(2)));
            assert_eq!(second.battlefield[2].uid, 3);
            assert_eq!(log.land_drops, [0, 1]);
            assert_eq!(log.cards_seen, [1, 3]);
            assert_eq!(json["velocity"]["avg_cards_seen"]["2"], 3.0);
            assert_eq!(json["outcomes"]["finished_games"], 1);
            assert_library_rng(&second.library_rng, &rng);
        }
    }
}

/// Run the production game driver and publish its actual opponent activity.
fn full_game_report(cards: &SimDeck, active: bool, turns: u32) -> (GameLog, Value) {
    let config = OpponentConfig {
        active,
        ..Default::default()
    };
    let mut opener_rng = ChaCha8Rng::seed_from_u64(1);
    let library_rng = crate::deck::simulator::rng::game_rng(
        1,
        0,
        crate::deck::simulator::rng::RngDomain::Library,
    );
    let log = run_game_with_config(cards, &mut opener_rng, turns, Some(&config), library_rng);
    let mut report = report_log(cards, &log, turns);
    let stats = aggregate(std::slice::from_ref(&log), cards, turns);
    crate::deck::simulator::report::set_opponent_model(&mut report, cards.format, config, &stats);
    (
        log,
        serde_json::to_value(report).expect("the executed report serializes"),
    )
}

/// Extra turns consume slots, replay land and upkeep actions, and suppress intervening rounds.
pub(super) fn assert_extra_slot_report() {
    for extra in [true, false] {
        let text = if extra {
            "{T}: Add {G}.\nWhen this land enters, take an extra turn after this one.\nAt the beginning of your upkeep, gain 1 life."
        } else {
            "{T}: Add {G}.\nAt the beginning of your upkeep, gain 1 life."
        };
        let source = row("Synthetic slot land", "", "Basic Land — Forest", text);
        let oracle = parse_oracle_card(&source);
        assert!(oracle.abilities.iter().any(|ability| matches!(ability,
            OracleAbility::Triggered(ability)
                if ability.event == OracleTriggerEvent::BeginningOfUpkeep(PlayerScope::You)
                    && matches!(ability.effects.as_slice(), [OracleEffect::GainLife(1)]))));
        let cards = deck(&vec![source; 24]);
        assert!(
            cards.cards[0]
                .unlocked_abilities(0)
                .any(|ability| ability.trigger == SimTrigger::Upkeep
                    && matches!(ability.effect_sequence(), [SimEffect::GainLife(1)]))
        );
        assert_eq!(
            cards.cards[0]
                .unlocked_abilities(0)
                .any(|ability| ability.trigger == SimTrigger::Enters
                    && matches!(ability.effect_sequence(), [SimEffect::ExtraTurn])),
            extra
        );
        let (log, json) = full_game_report(&cards, true, 3);
        assert_eq!(log.completion, GameCompletion::HorizonReached);
        assert_eq!(log.turns_observed, 3);
        assert_eq!(
            log.extra_turns,
            if extra { vec![0, 1, 1] } else { vec![0, 0, 0] }
        );
        assert_eq!(log.land_drops, [1, 1, 1]);
        assert_eq!(log.life_gained, 3);
        assert_eq!(log.life_paid, 0);
        assert_eq!(log.mana_spent, [0.0, 0.0, 0.0]);
        assert_eq!(log.card_first_battlefield.len(), 3);
        for turn in 1..=3 {
            assert_eq!(
                log.card_first_battlefield
                    .values()
                    .filter(|value| **value == turn)
                    .count(),
                1
            );
        }
        assert_eq!(log.opponent_activity.draws, if extra { 0 } else { 2 });
        assert_eq!(log.opponent_activity.casts, if extra { 0 } else { 1 });
        assert_eq!(
            json["opponent_model"]["observed_activity"]["totals"]["draws"],
            if extra { 0 } else { 2 }
        );
        assert_eq!(
            json["opponent_model"]["observed_activity"]["totals"]["casts"],
            if extra { 0 } else { 1 }
        );
        assert_eq!(json["draw"]["avg_life_gained"], 3.0);
        assert_eq!(
            json["win_conditions"]["percent_games_with_extra_turn"],
            if extra { 100.0 } else { 0.0 }
        );
        let rate = &json["sampling"]["win_conditions.percent_games_with_extra_turn"];
        assert_eq!(rate["event_games"], u32::from(extra));
        assert_eq!(rate["observed_games"], 1);
        assert_eq!(json["phase_observations"]["end_turn_by_turn"]["3"], 1.0);
    }
}

/// Production opponent rounds add damage after the turn census without merging direct life loss.
pub(super) fn assert_opponent_round_damage_report() {
    let cards = deck(&vec![
        row(
            "Synthetic round land",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.\nWhenever an opponent draws a card, this land deals 1 damage to each opponent.\nWhenever an opponent draws a card, each opponent loses 2 life."
        );
        24
    ]);
    for active in [true, false] {
        let (log, json) = full_game_report(&cards, active, 3);
        assert_eq!(log.completion, GameCompletion::HorizonReached);
        assert_eq!(
            log.player_damage,
            if active { vec![1, 3, 3] } else { vec![0, 0, 0] }
        );
        assert_eq!(
            log.opponent_life_loss,
            if active { vec![2, 6, 6] } else { vec![0, 0, 0] }
        );
        assert_eq!(log.attack_power, [0, 0, 0]);
        assert_eq!(log.life_paid, 0);
        assert_eq!(log.life_gained, 0);
        assert_eq!(log.opponent_activity.draws, if active { 2 } else { 0 });
        for (turn, damage, loss) in [(1, 1.0, 2.0), (2, 3.0, 6.0), (3, 3.0, 6.0)] {
            assert_eq!(
                json["combat"]["player_damage_avg_by_turn"][turn.to_string()],
                if active { damage } else { 0.0 }
            );
            assert_eq!(
                json["win_conditions"]["opponent_life_loss_by_turn"][turn.to_string()],
                if active { loss } else { 0.0 }
            );
        }
        assert_eq!(
            json["opponent_model"]["observed_activity"]["totals"]["draws"],
            if active { 2 } else { 0 }
        );
        assert_eq!(json["outcomes"]["finished_games"], 1);
        assert_eq!(json["win_conditions"]["damage_threshold_games"], 0);
        assert!(json["win_conditions"]["p50_lethal_turn"].is_null());
    }
}

/// Partner potential has separate thresholds and a finished-game denominator including losses.
pub(super) fn assert_partner_potential_report() {
    for first_power in [10, 21] {
        let mut cards = deck(&[row("Draw marker", "{99}", "Sorcery", "")]);
        cards.format = Format::Commander;
        cards.rules = crate::deck::simulator::format::rules_for("commander");
        for (slot, power) in [first_power, 11].into_iter().enumerate() {
            let mut source = row(
                &format!("Synthetic partner {slot}"),
                "{99}",
                "Legendary Creature",
                "",
            );
            source.power = Some(power.to_string());
            source.toughness = Some("30".into());
            cards.commanders.push(parse_sim_card(&source));
        }
        let mut logs = Vec::new();
        for can_draw in [true, false] {
            let mut game = state(vec![], if can_draw { vec![0] } else { vec![] });
            game.opponents = OpponentTable::for_rules(cards.rules);
            game.commander_casts = vec![1, 1];
            for slot in 0..2 {
                game.battlefield
                    .push(new_commander_perm(slot as u32 + 1, slot, 0, 0, true, false));
            }
            game.next_uid = 2;
            let (game, log) = execute_turns(&cards, game, 1, vec![]);
            assert!(
                game.opponents
                    .seats
                    .iter()
                    .all(|seat| seat.active && seat.life == 40)
            );
            assert_eq!(game.damage_dealt_this_turn, 0);
            assert_eq!(game.opponent_life_lost, 0);
            assert_eq!(log.turns_observed, u32::from(can_draw));
            logs.push(log);
        }
        let stats = aggregate(&logs, &cards, 1);
        let report = crate::deck::simulator::report::json_report(
            &stats,
            &cards,
            "Synthetic partner denominator",
            0,
            &[],
            Default::default(),
            &Default::default(),
        );
        assert_eq!(report.outcomes.finished_games, 2);
        assert_eq!(report.outcomes.player_loss_games, 1);
        assert_eq!(report.phase_observations.combat_by_turn[&1], 1.0);
        assert_eq!(
            report.combat.attack_power_avg_by_turn[&1],
            f64::from(first_power + 11)
        );
        assert_eq!(report.combat.commander_potential.len(), 2);
        for (slot, power) in [first_power, 11].into_iter().enumerate() {
            let potential = &report.combat.commander_potential[slot];
            let reached = power >= 21;
            assert_eq!(potential.slot, slot as u32);
            assert_eq!(potential.damage_by_turn[&1], f64::from(power));
            assert_eq!(potential.observed_games, 2);
            assert_eq!(potential.games_reaching_21, u32::from(reached));
            assert_eq!(
                potential
                    .percent_games_reaching_21
                    .expect("finished denominator")
                    .value(),
                if reached { 50.0 } else { 0.0 }
            );
            assert_eq!(potential.median_turn_to_21, reached.then_some(1));
            assert!(potential.note.contains("not a kill or win probability"));
        }
        assert_eq!(report.win_conditions.damage_threshold_games, 0);
        assert_eq!(
            report
                .win_conditions
                .percent_games_at_or_above_table_life_by_turn[&1]
                .value(),
            0.0
        );
    }
}
