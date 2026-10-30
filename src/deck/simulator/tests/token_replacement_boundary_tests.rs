//! Conditional token replacements stay inert; the simple control proves only generic bodies.

use super::card_runtime_tests::{attempt_index, renamed};
use super::card_test_support::{assert_library_rng, main_phase_state, named_row};
use super::game::{CardRef, GameState, ManaPool, new_perm_with};
use super::model::{CardIdx, SimAbilityCondition, SimDeck, SimEffect, SimTrigger};
use super::oracle_ast::{OracleAbility, OracleEffect, OracleTriggerEvent, PlayerScope};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row};
use super::support::{AbilitySupportStatus, card_source_hash};
use crate::db::CardRow;
use serde_json::Value;

/// The printed branches choose an Insect or a copy; they do not create both.
const SCUTE_RESOLUTION: &str = "create a 1/1 green Insect creature token. If you control six or more lands, create a token that's a copy of this creature instead.";

/// Retain the whole conditional resolution and its original source evidence.
fn assert_replacement_inert(source: &CardRow, resolution: &str) {
    let oracle = parse_oracle_card(source);
    let [OracleAbility::Triggered(trigger)] = oracle.abilities.as_slice() else {
        panic!("the source retains one complete land-entry trigger");
    };
    assert_eq!(
        trigger.event,
        OracleTriggerEvent::LandEnters(PlayerScope::You)
    );
    assert_eq!(trigger.condition, None);
    assert_eq!(trigger.resolution_payment, None);
    assert!(
        matches!(trigger.effects.as_slice(), [OracleEffect::Unsupported(text)] if text == resolution)
    );
    assert!(trigger.to_runtime_all().is_empty());
    assert!(
        super::oracle_lower::lower_effect_sequence_with_fixed_tokens(&trigger.effects).is_none()
    );
    let card = parse_sim_card(source);
    assert!(card.unlocked_abilities(0).next().is_none());
    assert_eq!(card.support.len(), 1);
    assert_eq!(card.support[0].source, source.oracle_text);
    let span = card.support[0]
        .span
        .expect("the printed trigger has a source span");
    assert_eq!(span.start, 0);
    assert_eq!(span.end, source.oracle_text.len());
    assert!(matches!(
        &card.support[0].status,
        AbilitySupportStatus::Unsupported { reason }
        if reason == "A required cost, condition, event, target, payment, or effect cannot lower as a complete ability."
    ));
}

/// Printed identity and a renamed control cannot grant either unsupported branch.
#[test]
fn pinned_scute_swarm_replacement_has_no_partial_token_lowering() {
    for source in renamed(&named_row("Scute Swarm")) {
        assert_eq!(source.oracle_id, "aa854d50-444c-49d9-bfb1-5476b33c1c0b");
        assert_eq!(
            source.oracle_text,
            format!("Landfall — Whenever a land you control enters, {SCUTE_RESOLUTION}")
        );
        assert_replacement_inert(&source, SCUTE_RESOLUTION);
    }
}

/// Seed a source, an entry reward, exact land counts, and occupied nonbattlefield zones.
fn land_entry_scenario(source: &CardRow, lands_before: u32) -> (SimDeck, GameState) {
    let mut rows = vec![
        source.clone(),
        row(
            "Entry reward control",
            "{99}",
            "Enchantment",
            "Whenever another creature you control enters, you gain 2 life.",
        ),
        row(
            "Entering Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ),
        row("Library marker", "{99}", "Sorcery", ""),
        row("Graveyard marker", "{99}", "Sorcery", ""),
        row("Exile marker", "{99}", "Sorcery", ""),
        row(
            "First sacrifice payoff",
            "{0}",
            "Sorcery",
            "As an additional cost to cast this spell, sacrifice a creature.\nYou gain 2 life.",
        ),
        row(
            "Second sacrifice payoff",
            "{0}",
            "Sorcery",
            "As an additional cost to cast this spell, sacrifice a creature.\nYou gain 2 life.",
        ),
        row("Nonland entry control", "{0}", "Artifact", ""),
    ];
    rows.extend((0..lands_before).map(|number| {
        row(
            &format!("Existing Forest {number}"),
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        )
    }));
    let cards = deck(&rows);
    assert!(cards[CardIdx(0)].is_creature);
    assert!(!cards[CardIdx(1)].is_creature);
    let mut game = main_phase_state(vec![2], vec![3]);
    game.graveyard = vec![CardIdx(4)];
    game.exile = vec![CardIdx(5)];
    for (index, uid) in [(0, 10), (1, 11)] {
        game.battlefield
            .push(new_perm_with(uid, &cards, CardIdx(index), 0, false));
    }
    for number in 0..lands_before {
        game.battlefield.push(new_perm_with(
            20 + number,
            &cards,
            CardIdx(9 + number),
            0,
            false,
        ));
    }
    game.next_uid = 100;
    (cards, game)
}

/// Count modeled token bodies without treating physical source cards as tokens.
fn token_count(game: &GameState) -> usize {
    game.battlefield
        .iter()
        .filter(|permanent| matches!(permanent.card, CardRef::Token(_)))
        .count()
}

/// Below, at, and above six lands, an actual land play cannot run half of the replacement.
#[test]
fn pinned_scute_land_entries_create_no_tokens_or_entry_rewards_at_either_threshold() {
    for source in renamed(&named_row("Scute Swarm")) {
        for lands_before in [4, 5, 6] {
            let (cards, mut game) = land_entry_scenario(&source, lands_before);
            let expected_rng = game.library_rng.clone();
            assert!(super::cast_pass::play_land(&cards, &mut game, 1));
            assert_eq!(game.battlefield.len(), lands_before as usize + 3);
            assert_eq!(token_count(&game), 0);
            assert_eq!(game.next_uid, 101);
            assert_eq!(game.life, 20);
            assert_eq!(game.life_gained, 0);
            assert_eq!(game.treasure_bank, 0);
            assert_eq!(game.library, [CardIdx(3)]);
            assert!(game.hand.is_empty());
            assert_eq!(game.graveyard, [CardIdx(4)]);
            assert_eq!(game.exile, [CardIdx(5)]);
            assert_eq!(game.seen, 0);
            assert!(game.completion.is_none());
            assert_library_rng(&game.library_rng, &expected_rng);
        }
    }
}

/// Unsupported tokens cannot fund a second real creature-sacrifice cast.
#[test]
fn pinned_scute_replacement_cannot_create_extra_sacrifice_payments() {
    for source in renamed(&named_row("Scute Swarm")) {
        for lands_before in [4, 5, 6] {
            let (cards, mut game) = land_entry_scenario(&source, lands_before);
            assert!(super::cast_pass::play_land(&cards, &mut game, 1));
            game.hand = vec![CardIdx(6), CardIdx(7)];
            let mut pool = ManaPool::default();
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(6)),
                Some(0)
            );
            assert_eq!(game.life_gained, 2);
            assert_eq!(game.graveyard, [CardIdx(4), CardIdx(0), CardIdx(6)]);
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(7)),
                None
            );
            assert_eq!(pool.total(), 0);
            assert_eq!(game.hand, [CardIdx(7)]);
            assert_eq!(game.life_gained, 2);
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_eq!(token_count(&game), 0);
        }
    }
}

/// Remove only the conditional replacement; the same source name can then make one generic body.
fn unconditional_control(source: &CardRow) -> CardRow {
    let mut changed = source.clone();
    changed.oracle_text =
        "Landfall — Whenever a land you control enters, create a 1/1 green Insect creature token."
            .into();
    assert_ne!(card_source_hash(&changed), card_source_hash(source));
    let card = parse_sim_card(&changed);
    let abilities: Vec<_> = card.unlocked_abilities(0).collect();
    let [ability] = abilities.as_slice() else {
        panic!("the unconditional control has exactly one runtime trigger");
    };
    assert_eq!(ability.trigger, SimTrigger::LandEnters);
    assert!(matches!(ability.effect_sequence(), [SimEffect::Tokens(1)]));
    changed
}

/// The positive control proves one generic body and its entry event, not a copy or exact token stats.
#[test]
fn same_name_unconditional_token_control_runs_only_on_real_land_entry() {
    for source in renamed(&named_row("Scute Swarm")) {
        let changed = unconditional_control(&source);
        for lands_before in [4, 5, 6] {
            let (cards, mut game) = land_entry_scenario(&changed, lands_before);
            game.hand.push(CardIdx(8));
            assert_eq!(
                attempt_index(&cards, &mut game, &mut ManaPool::default(), CardIdx(8)),
                Some(0)
            );
            assert_eq!(token_count(&game), 0);
            assert_eq!(game.life_gained, 0);
            assert!(super::cast_pass::play_land(&cards, &mut game, 1));
            assert_eq!(token_count(&game), 1);
            assert_eq!(game.life_gained, 2);
            assert_eq!(game.life, 22);
            assert_eq!(game.next_uid, 103);
            assert!(game.hand.is_empty());
            assert_eq!(game.graveyard, [CardIdx(4)]);
            assert_eq!(game.exile, [CardIdx(5)]);
        }
    }
}

/// Build real two-turn report data so any token would become an eligible attacker on turn two.
fn token_report(
    source: &CardRow,
    lands_before: u32,
    expected_tokens: usize,
) -> (super::game::GameLog, Value) {
    let (cards, mut game) = land_entry_scenario(source, lands_before);
    let mut census = super::game_run::TurnCensus::new(2, cards.cards.len());
    let commander = super::game_commander::CommanderProfile::new(&cards);
    let mut repeatable = Vec::new();
    for turn in 1..=2 {
        super::game_run::run_turn(
            &cards,
            &mut game,
            &mut census,
            &commander,
            &mut repeatable,
            turn,
            false,
        );
        assert_eq!(token_count(&game), expected_tokens);
        assert_eq!(
            game.battlefield.len(),
            lands_before as usize + 3 + expected_tokens
        );
        assert_eq!(
            game.battlefield
                .iter()
                .filter(|permanent| permanent.card.deck_idx() == Some(CardIdx(0)))
                .count(),
            1
        );
    }
    assert!(game.completion.is_none());
    let log = super::game_run::census::finish_log(&cards, game, census, 0, false, 2);
    let stats = super::aggregate::aggregate(std::slice::from_ref(&log), &cards, 2);
    let report = super::report::json_report(
        &stats,
        &cards,
        "Unsupported token replacement boundary",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    let json = serde_json::to_value(report).expect("the production report serializes");
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    (log, json)
}

/// Production reports retain the exact unsupported source and cannot show phantom bodies or combat.
#[test]
fn scute_reports_keep_conditional_replacement_inert_and_only_report_real_bodies() {
    for source in renamed(&named_row("Scute Swarm")) {
        for lands_before in [4, 5, 6] {
            let (log, json) = token_report(&source, lands_before, 0);
            assert_eq!(log.land_drops, [1, 0]);
            assert_eq!(log.creatures, [1, 1]);
            assert_eq!(log.attack_power, [1, 1]);
            assert_eq!(log.life_gained, 0);
            assert_eq!(log.cards_seen, [0, 1]);
            assert_eq!(json["creatures_by_turn"]["1"], 1.0);
            assert_eq!(json["creatures_by_turn"]["2"], 1.0);
            assert_eq!(json["combat"]["attack_power_avg_by_turn"]["2"], 1.0);
            assert_eq!(json["draw"]["avg_life_gained"], 0.0);
            let evidence = json["card_support"]
                .as_array()
                .expect("compiler evidence is a list")
                .iter()
                .find(|card| card["name"] == source.name)
                .expect("the exact source is reported");
            assert_eq!(evidence["source_hash"], card_source_hash(&source));
            assert_eq!(evidence["abilities"].as_array().map(Vec::len), Some(1));
            let ability = &evidence["abilities"][0];
            assert_eq!(ability["source"], source.oracle_text);
            assert_eq!(ability["status"], "unsupported");
            assert!(ability["rule_id"].is_null());
            assert_eq!(ability["span_start"], 0);
            assert_eq!(ability["span_end"], source.oracle_text.len());
            assert_eq!(
                ability["reason"],
                "A required cost, condition, event, target, payment, or effect cannot lower as a complete ability."
            );
            let (control, control_json) =
                token_report(&unconditional_control(&source), lands_before, 1);
            assert_eq!(control.creatures, [2, 2]);
            assert_eq!(control.attack_power, [1, 3]);
            assert_eq!(control.life_gained, 2);
            assert_eq!(control_json["creatures_by_turn"]["2"], 2.0);
            assert_eq!(control_json["combat"]["attack_power_avg_by_turn"]["2"], 3.0);
            assert_eq!(control_json["draw"]["avg_life_gained"], 2.0);
        }
    }
}

/// A later branch cannot be split into an unconditional token, draw, or life reward.
#[test]
fn conditional_token_near_misses_reject_the_whole_resolution() {
    for resolution in [
        "create a 1/1 green Insect creature token. If you control six or more lands, create two Treasure tokens instead.",
        "create a 1/1 green Insect creature token; if you control six or more lands, create two Treasure tokens instead.",
        "create a 1/1 green Insect creature token, and if you control six or more lands, create two Treasure tokens instead.",
        "create a 1/1 green Insect creature token. If you control six or more lands, create two Treasure tokens.",
        "create a Treasure token if you control six or more lands.",
        "create a Treasure token instead of a creature token.",
        "create a 1/1 green Insect creature token. Draw a card instead.",
        "create a 1/1 green Insect creature token. If you control six or more lands, you gain 2 life.",
        "create a 1/1 green Insect creature token. IF you control six or more lands, create a token INSTEAD.",
    ] {
        let mut source = named_row("Scute Swarm");
        source.oracle_text =
            format!("Whenever a land you control enters, {resolution} You gain 2 life.");
        let complete_resolution = format!("{resolution} You gain 2 life.");
        assert_replacement_inert(&source, &complete_resolution);
        let (cards, mut game) = land_entry_scenario(&source, 5);
        assert!(super::cast_pass::play_land(&cards, &mut game, 1));
        assert_eq!(token_count(&game), 0);
        assert_eq!(game.life_gained, 0);
        assert_eq!(game.library, [CardIdx(3)]);
    }
}

/// Typed optional payments remove their own if-clause before the resolution guard runs.
#[test]
fn supported_if_you_do_payment_still_requires_its_printed_mana() {
    let source = row(
        "Paid cast draw control",
        "{1}",
        "Enchantment",
        "Whenever you cast a creature spell, you may pay {G}. If you do, draw a card.",
    );
    let cards = deck(&[
        source,
        row("Cast creature", "{0}", "Creature", ""),
        row("Draw target", "{99}", "Sorcery", ""),
    ]);
    for green in [0, 1] {
        let mut game = main_phase_state(vec![1], vec![2]);
        game.battlefield
            .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
        game.next_uid = 10;
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 0, green],
            ..ManaPool::default()
        };
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
            Some(0)
        );
        assert_eq!(
            game.hand,
            if green == 1 { vec![CardIdx(2)] } else { vec![] }
        );
        assert_eq!(game.seen, green);
        assert_eq!(pool.total(), 0);
    }
}

/// A modeled intervening condition retains its runtime gate rather than becoming unsupported.
#[test]
fn modeled_intervening_condition_still_gates_a_token_reward() {
    let source = row(
        "Raid token control",
        "{1}",
        "Enchantment",
        "At the beginning of your end step, if you attacked this turn, create a 1/1 green Insect creature token.",
    );
    let cards = deck(&[source]);
    let ability = cards[CardIdx(0)]
        .unlocked_abilities(0)
        .next()
        .expect("the modeled condition lowers");
    assert_eq!(ability.condition, Some(SimAbilityCondition::Raid));
    assert!(matches!(ability.effect_sequence(), [SimEffect::Tokens(1)]));
    for attacked in [false, true] {
        let mut game = main_phase_state(vec![], vec![]);
        game.battlefield
            .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
        game.next_uid = 10;
        game.attacked_this_turn = attacked;
        super::game::fire_triggers(&cards, &mut game, SimTrigger::EndStep, 1);
        assert_eq!(token_count(&game), usize::from(attacked));
    }
}

/// Give a ready land-entry source two physical lands and enough cards for real draw rewards.
fn land_reward_scenario(source: &CardRow) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row(
            "First entered Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ),
        row(
            "Second entered Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ),
        row("Bottom draw target", "{99}", "Sorcery", ""),
        row("Middle draw target", "{99}", "Sorcery", ""),
        row("Top draw target", "{99}", "Sorcery", ""),
    ]);
    let mut game = main_phase_state(vec![1, 2], vec![3, 4, 5]);
    game.battlefield
        .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
    game.battlefield[0].summoning_sick = false;
    game.next_uid = 10;
    (cards, game)
}

/// Resolve one real land play inside the same live-pool boundary used by mana-producing effects.
fn play_reward_land(cards: &SimDeck, game: &mut GameState, pool: &mut ManaPool, turn: u32) {
    super::resolution::begin_resolution(game);
    assert!(super::cast_pass::play_land(cards, game, turn));
    super::resolution::end_resolution(cards, game, Some(pool));
}

/// Land-entry rewards retain their exact effects without changing the source's printed power.
fn assert_land_reward_power(source: &CardRow, power: u32, mana: u32, draws: u32, life: u32) {
    let (cards, mut game) = land_reward_scenario(source);
    assert_eq!(cards[CardIdx(0)].printed_power, Some(power));
    let mut mana_produced = 0;
    for turn in 1..=2 {
        game.current_turn = turn;
        // Each controlled main phase starts after the source's normal untap step.
        game.battlefield[0].tapped = false;
        let mut pool = ManaPool::default();
        play_reward_land(&cards, &mut game, &mut pool, turn);
        mana_produced += pool.total();
        let combat = super::game_combat::combat_phase(
            &cards,
            &mut game,
            turn as usize,
            &[1, 1][..turn as usize],
        );
        assert_eq!(combat.power, power);
        assert_eq!(combat.attackers, 1);
        assert_eq!(token_count(&game), 0);
        assert_eq!(game.battlefield[0].counters.plus1, 0);
        assert_eq!(game.battlefield[0].counters.charge, 0);
    }
    assert_eq!(mana_produced, mana);
    assert_eq!(game.seen, draws);
    assert_eq!(game.life_gained, life);
    assert_eq!(game.life, 20 + life as i32);
    assert_eq!(game.library.len(), 3 - draws as usize);
    assert_eq!(game.hand.len(), draws as usize);
    assert_eq!(game.battlefield.len(), 3);
    assert_eq!(game.treasure_bank, 0);
    assert!(game.graveyard.is_empty() && game.exile.is_empty());
}

/// Pinned mana and draw creatures keep their rewards, but actual land entries grant no power.
#[test]
fn pinned_landfall_mana_and_draw_rewards_never_grant_unprinted_power() {
    for (name, power, mana, draws, life) in [
        ("Lotus Cobra", 2, 2, 0, 0),
        ("Tatyova, Benthic Druid", 3, 0, 2, 2),
    ] {
        for source in renamed(&named_row(name)) {
            assert_land_reward_power(&source, power, mana, draws, life);
            let mut changed = source.clone();
            changed.oracle_text =
                "Landfall — Whenever a land you control enters, draw a card.".into();
            assert_ne!(card_source_hash(&changed), card_source_hash(&source));
            assert_land_reward_power(&changed, power, 0, 2, 0);
        }
    }
}

/// Produce combat and reward reports from two ordinary turns with one actual land play each.
fn land_reward_report(source: &CardRow) -> (super::game::GameLog, Value) {
    let (cards, mut game) = land_reward_scenario(source);
    let mut census = super::game_run::TurnCensus::new(2, cards.cards.len());
    let commander = super::game_commander::CommanderProfile::new(&cards);
    let mut repeatable = Vec::new();
    for turn in 1..=2 {
        super::game_run::run_turn(
            &cards,
            &mut game,
            &mut census,
            &commander,
            &mut repeatable,
            turn,
            false,
        );
        assert_eq!(token_count(&game), 0);
    }
    assert!(game.completion.is_none());
    let log = super::game_run::census::finish_log(&cards, game, census, 0, false, 2);
    let stats = super::aggregate::aggregate(std::slice::from_ref(&log), &cards, 2);
    let report = super::report::json_report(
        &stats,
        &cards,
        "Land-entry rewards without unprinted power",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    let json = serde_json::to_value(report).expect("the production report serializes");
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    assert_eq!(log.land_drops, [1, 1]);
    assert_eq!(log.creatures, [1, 1]);
    (log, json)
}

/// Reports use printed Cobra and Tatyova power while their supported rewards remain active.
#[test]
fn pinned_landfall_reports_do_not_turn_mana_or_draw_events_into_power() {
    for (name, power, seen, life) in [
        ("Lotus Cobra", 2, [0, 1], 0),
        ("Tatyova, Benthic Druid", 3, [1, 3], 2),
    ] {
        for source in renamed(&named_row(name)) {
            let (log, json) = land_reward_report(&source);
            assert_eq!(log.attack_power, [power, power]);
            assert_eq!(log.cards_seen, seen);
            assert_eq!(log.life_gained, life);
            for turn in ["1", "2"] {
                assert_eq!(
                    json["combat"]["attack_power_avg_by_turn"][turn],
                    f64::from(power)
                );
                assert_eq!(json["creatures_by_turn"][turn], 1.0);
            }
        }
    }
}

/// Unmodeled printed pump and +1/+1 counters remain inert and visible as unsupported evidence.
#[test]
fn unsupported_landfall_power_and_counter_wording_grants_nothing() {
    for original in renamed(&named_row("Lotus Cobra")) {
        for effect in [
            "this creature gets +2/+2 until end of turn.",
            "put a +1/+1 counter on this creature.",
        ] {
            let mut source = original.clone();
            source.oracle_text = format!("Landfall — Whenever a land you control enters, {effect}");
            let oracle = parse_oracle_card(&source);
            let [OracleAbility::Triggered(trigger)] = oracle.abilities.as_slice() else {
                panic!("the source preserves its land-entry trigger");
            };
            assert_eq!(
                trigger.event,
                OracleTriggerEvent::LandEnters(PlayerScope::You)
            );
            assert!(
                matches!(trigger.effects.as_slice(), [OracleEffect::Unsupported(text)] if text == effect.trim_end_matches('.'))
            );
            assert!(trigger.to_runtime_all().is_empty());
            assert_land_reward_power(&source, 2, 0, 0, 0);
            let (log, json) = land_reward_report(&source);
            assert_eq!(log.attack_power, [2, 2]);
            assert_eq!(log.life_gained, 0);
            let evidence = json["card_support"]
                .as_array()
                .expect("the report lists compiler evidence")
                .iter()
                .find(|card| card["name"] == source.name)
                .expect("the changed source remains in the report");
            assert_eq!(evidence["source_hash"], card_source_hash(&source));
            assert_eq!(evidence["abilities"][0]["source"], source.oracle_text);
            assert_eq!(evidence["abilities"][0]["status"], "unsupported");
            assert!(evidence["abilities"][0]["rule_id"].is_null());
            assert_eq!(
                evidence["abilities"][0]["reason"],
                "A required cost, condition, event, target, payment, or effect cannot lower as a complete ability."
            );
        }
    }
}
