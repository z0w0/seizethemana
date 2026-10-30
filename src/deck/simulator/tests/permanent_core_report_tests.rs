//! Generic-core clause map. These synthetic sources are not pinned-card evidence.
//!
//! `assert_combat_event_reports`: attack and damage events, typed filters,
//! nonattackers, and capacity-only combat without a live seat.
//! `assert_token_body_cap_reports`: eight completed bodies versus nine requested
//! bodies, with exact incomplete-work evidence and report denominators.
//! Typed token clauses report the generic-body approximation, not exact characteristics.
//! Its executed bodies are still generic 2/2 creatures without Elf identity.
//! `assert_counter_consumer_reports`: typed positive, zero, and negative power
//! after a real counter effect, measured by combat, crew, and station.
//! `assert_vehicle_report`: insufficient crew and a shared tap budget.
//! `assert_station_reports`: tier boundaries, a fresh body's station tap, and
//! the next-turn pilot timing of an activation unlocked by that tap.
//! `assert_saga_reports`: unsupported chapters stay empty; read ahead skips them.
//! `assert_planeswalker_reports`: one loyalty activation and ultimate affordability.
//! Ultimates are not resolved in these scenarios. No selected or original-351
//! Vehicle, Spacecraft, Saga, or planeswalker backs these generic-core claims.
//! Whole-card claims remain zero. Token identity and exact copies remain gaps.

use crate::db::CardRow;
use crate::deck::simulator::aggregate::aggregate;
use crate::deck::simulator::card_event_claim_tests::assert_clause_source;
use crate::deck::simulator::card_test_support::main_phase_state as state;
use crate::deck::simulator::game::{
    CardRef, GameCompletion, GameLog, GameState, IncompleteReason, ManaPool, TokenKind,
    new_commander_perm, new_perm_with,
};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_effects::activation::{
    apply_minus_counter, pick_best_activation, resolve_activation,
};
use crate::deck::simulator::game_effects::{apply_effect_at, tap_budget};
use crate::deck::simulator::game_run::{TurnCensus, census::finish_log, run_turn};
use crate::deck::simulator::model::{CardIdx, SimDeck, SimEffect};
use crate::deck::simulator::report::json_report;
use crate::deck::simulator::report_schema::SimReport;
use crate::deck::simulator::scenario_test_support::{deck, row};

/// Execute full production turns and build a log from their actual census.
pub(super) fn execute_turns(
    cards: &SimDeck,
    mut game: GameState,
    turns: usize,
    mut sources: Vec<(u32, u32)>,
) -> (GameState, GameLog) {
    let mut census =
        TurnCensus::with_commander_count(turns, cards.cards.len(), cards.commanders.len());
    let commander = CommanderProfile::new(cards);
    for turn in 1..=turns {
        run_turn(
            cards,
            &mut game,
            &mut census,
            &commander,
            &mut sources,
            turn,
            false,
        );
        if game.completion.is_some() {
            break;
        }
    }
    let log = finish_log(cards, game.clone(), census, 0, false, turns);
    (game, log)
}

/// Report one executed log without fabricating census observations.
pub(super) fn report_log(cards: &SimDeck, log: &GameLog, turns: u32) -> SimReport {
    let stats = aggregate(std::slice::from_ref(log), cards, turns);
    let report = json_report(
        &stats,
        cards,
        "permanent clause control",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    for entry in &report.card_support {
        let card = cards
            .cards
            .iter()
            .chain(&cards.commanders)
            .find(|card| card.name == entry.name)
            .expect("report source exists in the compiled inputs");
        assert_eq!(
            entry.source_hash.as_deref(),
            Some(card.source_hash.as_str())
        );
        assert_eq!(entry.abilities.len(), card.support.len());
        assert!(entry.note.contains("Compiler evidence only"));
        for (actual, expected) in entry.abilities.iter().zip(&card.support) {
            assert_eq!(actual.source, expected.source);
            assert_eq!(actual.span_start, expected.span.map(|span| span.start));
            assert_eq!(actual.span_end, expected.span.map(|span| span.end));
            assert_eq!(
                serde_json::Value::String(actual.status.clone()),
                serde_json::to_value(&expected.status).expect("compiler status serializes")["status"]
            );
        }
    }
    report
}

/// Give controlled bodies printed power and enough toughness for counter tests.
fn body(name: &str, kind: &str, power: u32, text: &str) -> CardRow {
    let mut source = row(name, "{0}", kind, text);
    source.power = Some(power.to_string());
    source.toughness = Some("20".into());
    source
}

/// Combat rewards require a matching declared attacker and an active defender.
pub(super) fn assert_combat_event_reports() {
    for wording in [
        "Whenever an Elf you control attacks, draw a card.",
        "Whenever an Elf you control deals combat damage to a player, draw a card.",
    ] {
        let observer = row("Combat observer", "{0}", "Enchantment", wording);
        assert_clause_source(&observer, "Whenever an Elf", "compiler_only");
        for (kind, haste, live, reward, power) in [
            ("Creature — Elf", true, true, 1, 3),
            ("Creature — Elf", false, true, 0, 0),
            ("Creature — Elf", true, false, 0, 3),
            ("Creature — Goblin", true, true, 0, 3),
        ] {
            let cards = deck(&[
                observer.clone(),
                body("Attacker", kind, 3, if haste { "Haste" } else { "" }),
                row("Remaining", "{99}", "Sorcery", ""),
                row("Reward", "{99}", "Sorcery", ""),
            ]);
            let mut game = state(vec![1], vec![2, 3]);
            game.next_uid = 100;
            game.opponents.seats[0].active = live;
            game.battlefield
                .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
            let (game, log) = execute_turns(&cards, game, 1, vec![]);
            assert_eq!(game.battlefield.len(), 2);
            assert_eq!(game.battlefield[0].uid, 10);
            assert_eq!(game.battlefield[1].uid, 101);
            assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
            assert_eq!(game.battlefield[1].tapped, haste && live);
            assert_eq!(game.attacked_this_turn, haste && live);
            assert_eq!(
                game.hand,
                if reward == 1 {
                    vec![CardIdx(3)]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                game.library,
                if reward == 1 {
                    vec![CardIdx(2)]
                } else {
                    vec![CardIdx(2), CardIdx(3)]
                }
            );
            assert_eq!(game.seen, reward);
            assert_eq!(game.life, 20);
            assert_eq!(game.opponent_life_lost, 0);
            assert_eq!(game.opponents.seats[0].life, 20);
            assert!(game.graveyard.is_empty() && game.exile.is_empty());
            assert_eq!(log.attack_power, [power]);
            assert_eq!(log.attackers, [u32::from(haste)]);
            assert_eq!(log.cards_seen, [reward]);
            let report = report_log(&cards, &log, 1);
            assert_eq!(report.outcomes.finished_games, 1);
            assert_eq!(report.combat.attack_power_avg_by_turn[&1], f64::from(power));
            assert_eq!(report.creatures_by_turn[&1], 1.0);
            assert_eq!(report.win_conditions.opponent_life_loss_by_turn[&1], 0.0);
        }
    }
}

/// Eight bodies complete normally; a ninth requested body records one unfinished object.
pub(super) fn assert_token_body_cap_reports() {
    for (word, count) in [("eight", 8), ("nine", 9)] {
        let source = row(
            "Token cap source",
            "{0}",
            "Artifact",
            &format!("When this artifact enters, create {word} 1/1 Elf creature tokens."),
        );
        assert_clause_source(&source, "When this artifact enters", "approximated");
        let cards = deck(&[source]);
        assert!(cards.cards[0].unlocked_abilities(0).any(|ability|
            matches!(ability.effect_sequence(), [SimEffect::Tokens(amount)] if *amount == count)));
        let mut game = state(vec![0], vec![]);
        game.next_uid = 100;
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.battlefield.len(), 9);
        assert_eq!(game.battlefield[0].uid, 101);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(game.life, 20);
        assert_eq!(game.life_gained, 0);
        assert_eq!(game.next_uid, 109);
        for (offset, token) in game.battlefield[1..].iter().enumerate() {
            assert_eq!(token.uid, 102 + offset as u32);
            assert_eq!(token.card, CardRef::Token(TokenKind::Creature));
            assert!(token.summoning_sick && !token.tapped);
            assert_eq!(token.entered_turn, 1);
            let token_card = crate::deck::simulator::game::card_of(&cards, token);
            assert_eq!(token_card.printed_power, Some(2));
            assert_eq!(token_card.printed_toughness, Some(2));
            assert!(token_card.subtypes.is_empty());
        }
        assert!(game.hand.is_empty() && game.library.is_empty() && game.exile.is_empty());
        assert!(game.graveyard.is_empty());
        let report = report_log(&cards, &log, 1);
        let source_report = report
            .card_support
            .iter()
            .find(|card| card.name == "Token cap source")
            .expect("token source is in the report");
        let clause = source_report
            .abilities
            .iter()
            .find(|ability| ability.source.starts_with("When this artifact enters"))
            .expect("printed token clause is in the report");
        assert_eq!(clause.status, "approximated");
        assert_eq!(clause.rule_id.as_deref(), Some("generic_creature_tokens"));
        assert_eq!(clause.reason, None);
        assert_eq!(report.outcomes.requested_games, 1);
        assert_eq!(report.outcomes.finished_games, u32::from(count == 8));
        assert_eq!(report.outcomes.incomplete_games, u32::from(count == 9));
        if count == 8 {
            assert!(matches!(log.completion, GameCompletion::HorizonReached));
            assert_eq!(log.creatures, [8]);
            assert_eq!(log.attack_power, [0]);
            assert_eq!(report.creatures_by_turn[&1], 8.0);
            assert_eq!(report.draw.avg_life_gained, Some(0.0));
            assert!(report.outcomes.incomplete_observations.is_empty());
        } else {
            assert!(matches!(
                log.completion,
                GameCompletion::Incomplete {
                    reason: IncompleteReason::EffectWork {
                        configured_limit: 8,
                        remaining_work: 1
                    },
                    turn: 1,
                    ..
                }
            ));
            assert!(report.creatures_by_turn.is_empty());
            assert_eq!(report.draw.avg_life_gained, None);
            assert_eq!(report.outcomes.incomplete_observations.len(), 1);
            assert!(matches!(
                report.outcomes.incomplete_observations[0].reason,
                IncompleteReason::EffectWork {
                    configured_limit: 8,
                    remaining_work: 1
                }
            ));
        }
    }
}

/// Execute typed counter changes without letting charge, burden, or loyalty alter power.
fn counter_body(cards: &SimDeck, plus: u32, removals: u32) -> GameState {
    let library_index = cards.cards.len() - 1;
    let mut game = state(vec![], vec![library_index]);
    game.battlefield
        .push(new_perm_with(10, cards, CardIdx(0), 0, false));
    game.battlefield[0].counters.charge = 11;
    game.battlefield[0].counters.loyalty = 12;
    game.battlefield[0].counters.burden = 13;
    for _ in 0..plus {
        apply_effect_at(cards, &SimEffect::Explore, &mut game, 1, false, None);
    }
    assert_eq!(game.battlefield[0].counters.plus1, plus);
    assert_eq!(game.library, [CardIdx(library_index as u32)]);
    assert_eq!(game.seen, 0);
    for _ in 0..removals {
        apply_minus_counter(cards, &mut game, 1, 10);
    }
    assert_eq!(
        game.battlefield[0].counters.plus1,
        plus.saturating_sub(removals)
    );
    assert_eq!(
        game.battlefield[0].counters.minus1,
        removals.saturating_sub(plus)
    );
    assert_eq!(game.battlefield[0].counters.charge, 11);
    assert_eq!(game.battlefield[0].counters.loyalty, 12);
    assert_eq!(game.battlefield[0].counters.burden, 13);
    assert_eq!(game.life, 20);
    assert!(game.hand.is_empty() && game.exile.is_empty());
    assert!(game.graveyard.is_empty());
    game
}

/// Combat, crew, and station consume the same counter-adjusted nonnegative power.
pub(super) fn assert_counter_consumer_reports() {
    for (plus, removals, power) in [(0, 3, 0), (0, 5, 0), (2, 1, 4)] {
        let cards = deck(&[
            body("Counter body", "Creature", 3, ""),
            row("Counter library control", "{99}", "Sorcery", ""),
        ]);
        let game = counter_body(&cards, plus, removals);
        let (_, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(log.attack_power, [power]);
        assert_eq!(
            report_log(&cards, &log, 1).combat.attack_power_avg_by_turn[&1],
            f64::from(power)
        );
        assert_counter_crew(plus, removals, power);
        assert_counter_station(plus, removals, power);
    }
}

/// Crew rejects insufficient current power without tapping it in the crew action.
fn assert_counter_crew(plus: u32, removals: u32, power: u32) {
    let cards = deck(&[
        body("Counter body", "Creature", 3, ""),
        body("Vehicle", "Artifact — Vehicle", 5, "Crew 3"),
        row("Counter library control", "{99}", "Sorcery", ""),
    ]);
    let mut game = counter_body(&cards, plus, removals);
    game.battlefield
        .push(new_perm_with(11, &cards, CardIdx(1), 0, false));
    let mut action = game.clone();
    tap_budget(
        &cards,
        &mut action.battlefield,
        &mut ManaPool::default(),
        &[],
    );
    assert_eq!(action.battlefield[0].tapped, power >= 3);
    assert_eq!(action.battlefield[1].crewed, power >= 3);
    assert!(!action.battlefield[1].tapped);
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert_eq!(game.battlefield[1].crewed, power >= 3);
    assert_eq!(log.attack_power, [if power >= 3 { 5 } else { 0 }]);
    let report = report_log(&cards, &log, 1);
    assert_eq!(
        report.creatures_by_turn[&1],
        if power >= 3 { 2.0 } else { 1.0 }
    );
    assert_eq!(
        report.combat.attack_power_avg_by_turn[&1],
        if power >= 3 { 5.0 } else { 0.0 }
    );
}

/// Station adds current power, including zero, rather than any unrelated counter kind.
fn assert_counter_station(plus: u32, removals: u32, power: u32) {
    let cards = deck(&[
        body("Counter body", "Creature", 3, ""),
        body(
            "Spacecraft",
            "Artifact — Spacecraft",
            8,
            "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 3+.)\n3+ | Flying",
        ),
        row("Counter library control", "{99}", "Sorcery", ""),
    ]);
    let mut game = counter_body(&cards, plus, removals);
    game.battlefield
        .push(new_perm_with(11, &cards, CardIdx(1), 0, false));
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.battlefield[0].counters.charge, 11);
    assert_eq!(game.battlefield[1].counters.charge, power);
    assert_eq!(game.battlefield[1].animated, power >= 3);
    assert_eq!(game.battlefield[1].tapped, power >= 3);
    assert_eq!(log.attack_power, [if power >= 3 { 8 } else { 0 }]);
    let report = report_log(&cards, &log, 1);
    assert_eq!(
        report.creatures_by_turn[&1],
        if power >= 3 { 2.0 } else { 1.0 }
    );
    assert_eq!(
        report.combat.attack_power_avg_by_turn[&1],
        if power >= 3 { 8.0 } else { 0.0 }
    );
}

/// A body cannot crew twice, and an insufficient second crew leaves the first ready.
pub(super) fn assert_vehicle_report() {
    let cards = deck(&[
        body("Crew member", "Creature", 4, ""),
        body("First Vehicle", "Artifact — Vehicle", 5, "Crew 3"),
        body("Second Vehicle", "Artifact — Vehicle", 9, "Crew 6"),
    ]);
    let mut game = state(vec![], vec![]);
    for index in 0..3 {
        game.battlefield
            .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
    }
    let mut action = game.clone();
    tap_budget(
        &cards,
        &mut action.battlefield,
        &mut ManaPool::default(),
        &[],
    );
    assert!(action.battlefield[0].tapped);
    assert!(action.battlefield[1].crewed && !action.battlefield[1].tapped);
    assert!(!action.battlefield[2].crewed && !action.battlefield[2].tapped);
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert!(game.battlefield[0].tapped && game.battlefield[1].tapped);
    assert!(!game.battlefield[2].tapped && !game.battlefield[2].crewed);
    assert_eq!(
        game.battlefield
            .iter()
            .map(|perm| perm.uid)
            .collect::<Vec<_>>(),
        [10, 11, 12]
    );
    assert_eq!(log.attack_power, [5]);
    assert_eq!(log.attackers, [1]);
    assert_eq!(game.life, 20);
    assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
    let report = report_log(&cards, &log, 1);
    assert_eq!(report.creatures_by_turn[&1], 2.0);
    assert_eq!(report.combat.attack_power_avg_by_turn[&1], 5.0);
    assert_eq!(report.outcomes.finished_games, 1);
}

/// Build a two-tier commander Spacecraft with explicit entry identity.
fn station_scenario(charge: u32, fresh_body: bool) -> (SimDeck, GameState) {
    let mut cards = deck(&[
        body("Fresh station body", "Creature", 4, ""),
        body("Vehicle", "Artifact — Vehicle", 5, "Crew 3"),
        row("Station library control", "{99}", "Sorcery", ""),
    ]);
    cards.commanders = deck(&[body("Station commander", "Legendary Artifact — Spacecraft", 8,
        "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 7+.)\n3+ | {T}: Gain 2 life.\n7+ | Flying")]).cards;
    let mut game = state(if fresh_body { vec![0] } else { vec![] }, vec![2]);
    game.next_uid = 100;
    game.commander_casts = vec![1];
    game.battlefield
        .push(new_commander_perm(10, 0, 0, 0, true, false));
    game.battlefield[0].counters.charge = charge;
    if fresh_body {
        game.battlefield
            .push(new_perm_with(11, &cards, CardIdx(1), 0, false));
    }
    (cards, game)
}

/// Tier three unlocks a tap activation; tier seven animates the Spacecraft.
pub(super) fn assert_station_reports() {
    for charge in [2, 3, 6, 7] {
        let (cards, game) = station_scenario(charge, false);
        assert_eq!(cards.commanders[0].animate_at(), Some(7));
        let activations = cards.commanders[0]
            .unlocked_abilities(charge)
            .filter(|ability| ability.kind.is_activated())
            .collect::<Vec<_>>();
        assert_eq!(activations.len(), usize::from(charge >= 3));
        if charge >= 3 {
            assert!(matches!(
                activations[0].effect_sequence(),
                [SimEffect::GainLife(2)]
            ));
            assert!(
                activations[0]
                    .activation
                    .as_ref()
                    .expect("tier activation has costs")
                    .taps_source()
            );
        }
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.battlefield[0].counters.charge, charge);
        assert_eq!(game.battlefield[0].animated, charge >= 7);
        assert_eq!(game.battlefield[0].tapped, charge >= 3);
        assert_eq!(game.life, if charge >= 3 { 22 } else { 20 });
        assert_eq!(game.life_gained, if charge >= 3 { 2 } else { 0 });
        assert_eq!(log.station_online, (charge >= 7).then_some(1));
        let report = report_log(&cards, &log, 1);
        let station = report
            .station
            .expect("Spacecraft commander has a station report");
        assert_eq!(station.online_games, u32::from(charge >= 7));
        assert_eq!(station.p50_online_turn, (charge >= 7).then_some(1));
        assert_eq!(
            report.creatures_by_turn[&1],
            if charge >= 7 { 1.0 } else { 0.0 }
        );
        assert_eq!(report.combat.attack_power_avg_by_turn[&1], 0.0);
        assert_eq!(
            report.draw.avg_life_gained,
            Some(if charge >= 3 { 2.0 } else { 0.0 })
        );
    }
    assert_station_charge_timing();
}

/// A fresh body unlocks a legal activation after the pilot's activation pass.
fn assert_station_charge_timing() {
    let (cards, mut game) = station_scenario(2, true);
    let mut census = TurnCensus::with_commander_count(2, cards.cards.len(), 1);
    let commander = CommanderProfile::new(&cards);
    let mut sources = vec![];
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut sources,
        1,
        false,
    );
    let fresh = game
        .battlefield
        .iter()
        .find(|perm| perm.uid == 101)
        .expect("fresh body was cast");
    assert!(fresh.summoning_sick && fresh.tapped);
    assert_eq!(fresh.entered_turn, 1);
    assert_eq!(fresh.card, CardRef::Deck(CardIdx(0)));
    assert_eq!(game.battlefield[0].counters.charge, 6);
    assert!(!game.battlefield[0].animated && !game.battlefield[0].tapped);
    assert!(!game.battlefield[1].crewed && !game.battlefield[1].tapped);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_gained, 0);
    assert_eq!(census.station_online, None);
    let mut action = game.clone();
    action.current_phase = crate::deck::simulator::game::GamePhase::Main;
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &action, &pool, 1)
        .expect("charging unlocked a legal tap activation");
    assert_eq!(activation.uid, 10);
    resolve_activation(&cards, &mut action, &mut pool, 1, &activation);
    assert_eq!(action.life, 22);
    assert_eq!(action.life_gained, 2);
    assert_eq!(action.battlefield[0].counters.charge, 6);
    assert!(action.battlefield[0].tapped);
    assert_eq!(pool, ManaPool::default());
    assert_eq!(game.life, 20);
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut sources,
        2,
        false,
    );
    assert_eq!(game.life, 22);
    assert_eq!(game.life_gained, 2);
    assert_eq!(game.battlefield[0].counters.charge, 10);
    assert!(game.battlefield[0].animated && game.battlefield[0].tapped);
    assert!(!game.battlefield[1].crewed && !game.battlefield[1].tapped);
    assert!(game.battlefield[2].tapped && !game.battlefield[2].summoning_sick);
    assert_eq!(
        game.battlefield
            .iter()
            .map(|perm| perm.uid)
            .collect::<Vec<_>>(),
        [10, 11, 101]
    );
    assert_eq!(game.next_uid, 101);
    assert_eq!(game.hand, [CardIdx(2)]);
    assert!(game.library.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
    let log = finish_log(&cards, game, census, 0, false, 2);
    assert_eq!(log.station_online, Some(2));
    assert_eq!(log.attack_power, [0, 0]);
    assert_eq!(log.attackers, [0, 0]);
    assert_eq!(log.cards_seen, [0, 1]);
    assert_eq!(log.creatures, [1, 2]);
    let report = report_log(&cards, &log, 2);
    let station = report.station.expect("station timing report");
    assert_eq!(station.online_games, 1);
    assert_eq!(station.p50_online_turn, Some(2));
    assert_eq!(report.creatures_by_turn[&1], 1.0);
    assert_eq!(report.creatures_by_turn[&2], 2.0);
    assert_eq!(report.combat.attack_power_avg_by_turn[&1], 0.0);
    assert_eq!(report.combat.attack_power_avg_by_turn[&2], 0.0);
    assert_eq!(report.draw.avg_life_gained, Some(2.0));
}

/// Unsupported chapters grant no payoff; read ahead starts at the first supported chapter.
pub(super) fn assert_saga_reports() {
    for read_ahead in [false, true] {
        let source = row(
            "Synthetic Saga",
            "{0}",
            "Enchantment — Saga",
            &format!(
                "{}I — Exile target permanent.\nII — Draw a card.\nIII — Gain 3 life.",
                if read_ahead { "Read ahead\n" } else { "" }
            ),
        );
        assert_clause_source(&source, "I — Exile target permanent", "unsupported");
        let mut rows = vec![source];
        rows.extend((0..6).map(|index| row(&format!("Library {index}"), "{99}", "Sorcery", "")));
        let cards = deck(&rows);
        assert_eq!(cards.cards[0].read_ahead, read_ahead);
        assert_eq!(cards.cards[0].saga.chapters.len(), 3);
        assert!(cards.cards[0].saga.chapters[0].is_empty());
        let mut game = state(vec![0], (1..7).collect());
        game.next_uid = 100;
        let turns = if read_ahead { 2 } else { 3 };
        let (game, log) = execute_turns(&cards, game, turns, vec![]);
        assert!(game.battlefield.is_empty());
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.life, 23);
        assert_eq!(game.life_gained, 3);
        assert_eq!(game.next_uid, 101);
        assert_eq!(game.seen, if read_ahead { 2 } else { 3 });
        assert_eq!(
            game.hand,
            if read_ahead {
                vec![CardIdx(6), CardIdx(5)]
            } else {
                vec![CardIdx(6), CardIdx(5), CardIdx(4)]
            }
        );
        assert_eq!(
            game.library,
            if read_ahead {
                vec![CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4)]
            } else {
                vec![CardIdx(1), CardIdx(2), CardIdx(3)]
            }
        );
        assert_eq!(
            log.cards_seen,
            if read_ahead {
                vec![1, 2]
            } else {
                vec![0, 2, 3]
            }
        );
        assert_eq!(
            log.graveyard_size,
            if read_ahead {
                vec![0, 1]
            } else {
                vec![0, 0, 1]
            }
        );
        let report = report_log(&cards, &log, turns as u32);
        assert_eq!(report.outcomes.finished_games, 1);
        assert_eq!(report.draw.avg_life_gained, Some(3.0));
        assert_eq!(report.graveyard.avg_size_by_turn[&(turns as u32)], 1.0);
        assert!(report.creatures_by_turn.values().all(|count| *count == 0.0));
        assert!(
            report
                .card_support
                .iter()
                .find(|card| card.name == "Synthetic Saga")
                .expect("Saga source is in the report")
                .abilities
                .iter()
                .any(|entry| entry.source.contains("I — Exile target permanent")
                    && entry.status == "unsupported")
        );
    }
}

/// Each turn uses one loyalty ability; reaching six is measured before the main phase.
pub(super) fn assert_planeswalker_reports() {
    for initial in [4, 5] {
        let mut walker = row(
            "Synthetic planeswalker",
            "{0}",
            "Legendary Planeswalker — Test",
            "+1: Draw a card.\n-2: Gain 2 life.\n-6: Draw six cards.",
        );
        walker.loyalty = Some(initial.to_string());
        assert_clause_source(&walker, "+1: Draw a card", "supported");
        let mut rows = vec![walker];
        rows.extend((0..8).map(|index| row(&format!("Library {index}"), "{99}", "Sorcery", "")));
        let cards = deck(&rows);
        let mut game = state(vec![0], (1..9).collect());
        game.next_uid = 100;
        let (game, log) = execute_turns(&cards, game, 2, vec![]);
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].uid, 101);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(game.battlefield[0].counters.loyalty, initial + 2);
        assert!(game.battlefield[0].fired);
        assert!(!game.battlefield[0].tapped);
        assert!(pick_best_activation(&cards, &game, &ManaPool::default(), 2).is_none());
        assert_eq!(game.hand, [CardIdx(8), CardIdx(7), CardIdx(6)]);
        assert_eq!(
            game.library,
            [CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(5)]
        );
        assert_eq!(game.life, 20);
        assert_eq!(game.life_gained, 0);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(log.cards_seen, [1, 3]);
        assert_eq!(log.ultimate_online, (initial == 5).then_some(2));
        let report = report_log(&cards, &log, 2);
        assert_eq!(report.outcomes.finished_games, 1);
        assert_eq!(
            report
                .win_conditions
                .percent_games_with_affordable_ultimate
                .expect("finished denominator")
                .value(),
            if initial == 5 { 100.0 } else { 0.0 }
        );
        assert_eq!(report.creatures_by_turn[&1], 0.0);
        assert_eq!(report.creatures_by_turn[&2], 0.0);
    }
}

/// Check generic combat events through production turns and reports.
#[test]
fn generic_combat_event_action_and_report() {
    assert_combat_event_reports();
}

/// Check complete and incomplete token-body reports at the work cap.
#[test]
fn generic_token_body_cap_action_and_report() {
    assert_token_body_cap_reports();
}

/// Check each current-power consumer after typed counter effects.
#[test]
fn generic_counter_consumers_action_and_report() {
    assert_counter_consumer_reports();
}

/// Check the shared crew tap budget in an executed report.
#[test]
fn generic_vehicle_action_and_report() {
    assert_vehicle_report();
}

/// Check station tiers and summoning-sick station fuel in executed reports.
#[test]
fn generic_station_action_and_report() {
    assert_station_reports();
}

/// Check unsupported chapters and the read-ahead starting policy.
#[test]
fn generic_saga_action_and_report() {
    assert_saga_reports();
}

/// Check loyalty pacing and ultimate affordability without resolving the ultimate.
#[test]
fn generic_planeswalker_action_and_report() {
    assert_planeswalker_reports();
}
