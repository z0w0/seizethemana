//! Combat power comes from visible typed rules, not unrelated text or hidden spell fronts.

use super::card_report_tests::{execute_turn, report_executed_turn};
use super::card_runtime_tests::{attempt_index, renamed};
use super::card_test_support::{main_phase_state, named_row};
use super::game::{GameState, ManaPool, condition_met, new_perm_with};
use super::game_effects::{current_creature_power, current_creature_power_signed};
use super::model::{CardIdx, SimAbilityCondition, SimDeck};
use super::oracle_ast::{OracleAbility, OracleEffect, OracleTriggerEvent, PlayerScope};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row};
use super::support::{AbilitySupportStatus, card_source_hash};
use crate::db::CardRow;

/// A ready printed 2/2 makes hidden or invented power visible at the Ferocious threshold.
fn ready_body() -> CardRow {
    let mut body = row("Ready physical Bear", "{1}{G}", "Creature — Bear", "");
    body.power = Some("2".into());
    body.toughness = Some("2".into());
    body
}

/// Seed one ready creature and two real funding lands, with the tested source still in hand.
fn power_scenario(source: &CardRow) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        ready_body(),
        row(
            "Funding Forest one",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ),
        row(
            "Funding Forest two",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ),
        row(
            "Ordinary hand Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ),
    ]);
    let mut game = main_phase_state(vec![0], vec![]);
    for index in [1, 2, 3] {
        game.battlefield
            .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
    }
    game.battlefield[0].summoning_sick = false;
    game.next_uid = 20;
    (cards, game)
}

/// Cast the printed two-mana front through the production payment boundary.
fn cast_power_source(cards: &SimDeck, game: &mut GameState) {
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 0, 1],
        colorless: 1,
        ..ManaPool::default()
    };
    assert_eq!(attempt_index(cards, game, &mut pool, CardIdx(0)), Some(2));
    assert_eq!(pool.total(), 0);
    assert!(game.hand.is_empty());
    assert_eq!(game.spells_cast_this_turn, 1);
}

/// Shared power and the production Ferocious gate must read the same visible static sources.
fn assert_shared_power(cards: &SimDeck, game: &GameState, expected: u32) {
    let body = game
        .battlefield
        .iter()
        .find(|permanent| permanent.card.deck_idx() == Some(CardIdx(1)))
        .expect("the original ready creature remains on the battlefield");
    assert_eq!(
        current_creature_power_signed(body, cards, &game.battlefield),
        i64::from(expected)
    );
    assert_eq!(
        current_creature_power(body, cards, &game.battlefield),
        expected
    );
    assert_eq!(
        condition_met(cards, game, &SimAbilityCondition::Ferocious),
        expected >= 4
    );
}

/// Measure the original ready creature before a newly entered source can attack.
fn assert_combat_power(cards: &SimDeck, game: &mut GameState, expected: u32) {
    let combat = super::game_combat::combat_phase(cards, game, 1, &[0]);
    assert_eq!(combat.attackers, 1);
    assert_eq!(combat.power, expected);
}

/// Casting a creature with an unsupported variable land-entry pump cannot buff another attacker.
#[test]
fn unsupported_variable_landfall_pump_grants_no_entry_power() {
    for mut source in renamed(&named_row("Lotus Cobra")) {
        let effect = "this creature gets +X/+X until end of turn, where X is the number of lands you control.";
        source.oracle_text = format!("Whenever a land you control enters, {effect}");
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Triggered(trigger)] = oracle.abilities.as_slice() else {
            panic!("the source preserves its land-entry event");
        };
        assert_eq!(
            trigger.event,
            OracleTriggerEvent::LandEnters(PlayerScope::You)
        );
        assert!(
            matches!(trigger.effects.as_slice(), [OracleEffect::Unsupported(text)] if text == effect.trim_end_matches('.'))
        );
        assert!(trigger.to_runtime_all().is_empty());
        let card = parse_sim_card(&source);
        assert!(card.unlocked_abilities(0).next().is_none());
        assert!(matches!(
            card.support[0].status,
            AbilitySupportStatus::Unsupported { .. }
        ));

        let (cards, mut game) = power_scenario(&source);
        cast_power_source(&cards, &mut game);
        assert_shared_power(&cards, &game, 2);
        assert_combat_power(&cards, &mut game, 2);
        assert_eq!(game.battlefield.len(), 4);
        assert_eq!(game.life_gained, 0);

        let (cards, mut game) = power_scenario(&source);
        let census = execute_turn(&cards, &mut game);
        assert_shared_power(&cards, &game, 2);
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [0]);
        assert_eq!(log.creatures, [2]);
        assert_eq!(log.attack_power, [2]);
        assert_eq!(log.mana_spent, [2.0]);
        assert_eq!(report["combat"]["attack_power_avg_by_turn"]["1"], 2.0);
        let evidence = report["card_support"]
            .as_array()
            .expect("compiler evidence is a list")
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

/// Changing the same source to a supported static grant changes power only by the written amount.
#[test]
fn same_name_typed_static_buff_keeps_its_printed_power_grant() {
    for mut source in renamed(&named_row("Lotus Cobra")) {
        source.oracle_text = "Creatures you control get +2/+2.".into();
        let (cards, mut game) = power_scenario(&source);
        assert_eq!(cards[CardIdx(0)].flags.buff, Some((2, 2)));
        cast_power_source(&cards, &mut game);
        assert_shared_power(&cards, &game, 4);
        assert_combat_power(&cards, &mut game, 4);

        let (cards, mut game) = power_scenario(&source);
        let census = execute_turn(&cards, &mut game);
        assert_shared_power(&cards, &game, 4);
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.attack_power, [4]);
        assert_eq!(report["combat"]["attack_power_avg_by_turn"]["1"], 4.0);
    }
}

/// Use the stored newline face boundary to keep the front static grant separate.
fn modal_buff_source() -> CardRow {
    row(
        "Static grant front // Land back",
        "{1}{G} // ",
        "Enchantment // Land",
        "Creatures you control get +2/+2.\n// {T}: Add {G}.",
    )
}

/// A played land face and a face-down source cannot expose their front-face static buff.
#[test]
fn modal_land_and_face_down_buff_sources_do_not_raise_shared_or_combat_power() {
    for (as_land, face_down, power) in [(true, false, 2), (false, true, 2), (false, false, 4)] {
        let (cards, mut game) = power_scenario(&modal_buff_source());
        assert!(cards[CardIdx(0)].is_mdfc_spell);
        assert_eq!(cards[CardIdx(0)].flags.buff, Some((2, 2)));
        if as_land {
            assert!(super::cast_pass::play_land(&cards, &mut game, 1));
            assert_eq!(game.spells_cast_this_turn, 0);
        } else {
            cast_power_source(&cards, &mut game);
        }
        let source = game.battlefield.last_mut().expect("the source entered");
        assert_eq!(source.card.deck_idx(), Some(CardIdx(0)));
        assert_eq!(source.land_face, as_land);
        source.face_down = face_down;
        assert_shared_power(&cards, &game, power);
        assert_combat_power(&cards, &mut game, power);
    }
}

/// An ordinary land's own printed static grant remains active when that land is played.
#[test]
fn ordinary_land_static_buff_is_not_mistaken_for_a_hidden_spell_front() {
    let source = row(
        "Printed static buff land",
        "",
        "Land",
        "Creatures you control get +2/+2.\n{T}: Add {G}.",
    );
    let (cards, mut game) = power_scenario(&source);
    assert!(!cards[CardIdx(0)].is_mdfc_spell);
    assert_eq!(cards[CardIdx(0)].flags.buff, Some((2, 2)));
    assert!(super::cast_pass::play_land(&cards, &mut game, 1));
    assert!(game.battlefield.last().expect("the land entered").land_face);
    assert_shared_power(&cards, &game, 4);
    assert_combat_power(&cards, &mut game, 4);
}

/// Production reports distinguish the same modal card played as a land or cast as its front.
#[test]
fn modal_static_buff_source_reports_only_its_active_face_power() {
    for as_land in [false, true] {
        let (cards, mut game) = power_scenario(&modal_buff_source());
        if !as_land {
            game.hand.push(CardIdx(4));
        }
        let census = execute_turn(&cards, &mut game);
        let source = game
            .battlefield
            .iter()
            .find(|permanent| permanent.card.deck_idx() == Some(CardIdx(0)))
            .expect("the modal card reached the battlefield");
        assert_eq!(source.land_face, as_land);
        let power = if as_land { 2 } else { 4 };
        assert_shared_power(&cards, &game, power);
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.attack_power, [power]);
        assert_eq!(log.creatures, [1]);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.mana_spent, [if as_land { 0.0 } else { 2.0 }]);
        assert_eq!(
            report["combat"]["attack_power_avg_by_turn"]["1"],
            f64::from(power)
        );
    }
}
