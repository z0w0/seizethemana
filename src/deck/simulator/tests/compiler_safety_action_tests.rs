//! Compiler limits must survive paid actions and executed-turn reports.
//!
//! These scenarios do not certify counterspell resolution, Elf bonuses, or reach.
//! Combat values describe modeled unblocked potential, not applied opponent damage.

use super::aggregate::aggregate;
use super::card_runtime_tests::{attempt, renamed};
use super::card_test_support::{assert_library_rng, main_phase_state, named_row};
use super::game::{GameLog, GameState, ManaPool, Permanent, new_perm_with};
use super::game_commander::CommanderProfile;
use super::game_run::{TurnCensus, census::finish_log, run_turn};
use super::model::{
    AlternativeCostPayoff, CardIdx, Cost, ManaColor, SimDeck, SimEffect, SimTrigger,
};
use super::oracle_ast::{ObjectSubject, OracleAbility, OracleEffect, OracleTriggerEvent};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row};
use super::support::{AbilitySupportStatus, card_source_hash};
use crate::db::CardRow;
use serde_json::Value;

/// Supply a real blue pitch candidate and unrelated physical cards in every other zone.
fn alternate_scenario(source: &CardRow) -> (SimDeck, GameState) {
    let mut blue = row("Blue pitch candidate", "{6}{U}", "Sorcery", "");
    blue.colors = "[\"U\"]".into();
    let mut green = row("Wrong-color pitch candidate", "{6}{G}", "Sorcery", "");
    green.colors = "[\"G\"]".into();
    let cards = deck(&[
        source.clone(),
        blue,
        green,
        row("Library bottom", "{99}", "Sorcery", ""),
        row("Library middle", "{99}", "Sorcery", ""),
        row("Library top", "{99}", "Sorcery", ""),
        row("Graveyard marker", "{99}", "Sorcery", ""),
        row("Exile marker", "{99}", "Sorcery", ""),
        row("Battlefield marker", "{99}", "Artifact", ""),
        row("Hand marker", "{99}", "Sorcery", ""),
    ]);
    let mut game = main_phase_state(vec![0, 1, 2, 9], vec![3, 4, 5]);
    game.graveyard = vec![CardIdx(6)];
    game.exile = vec![CardIdx(7)];
    game.battlefield
        .push(new_perm_with(20, &cards, CardIdx(8), 0, false));
    game.next_uid = 21;
    assert!(cards[CardIdx(1)].colors[1]);
    assert!(!cards[CardIdx(2)].colors[1]);
    (cards, game)
}

/// Preserve unknown alternative wording without turning it into an executable partial payment.
fn assert_disabled_alternate_source(source: &CardRow, expected_cost: Cost, draws: bool) {
    let oracle = parse_oracle_card(source);
    assert!(oracle.spell_data.alternative_cost.is_none());
    let expected_source = if draws {
        source
            .oracle_text
            .lines()
            .next()
            .expect("the source has an alternative clause")
    } else {
        &source.oracle_text
    };
    assert_eq!(oracle.ability_sources[0].text, expected_source);
    assert!(
        matches!(&oracle.abilities[0], OracleAbility::Unsupported(ability)
        if ability.source.contains("rather than pay this spell's mana cost"))
    );
    let card = parse_sim_card(source);
    assert_eq!(card.cost, expected_cost);
    assert_eq!(card.min_cost, expected_cost);
    assert!(card.spell_data.alternative_cast_cost.is_none());
    assert!(!card.spell_data.unsupported_additional_cost);
    assert_eq!(card.spell_data.additional_cost_life, 0);
    assert!(matches!(
        card.support[0].status,
        AbilitySupportStatus::Unsupported { .. }
    ));
    assert_eq!(card.support[0].source, expected_source);
    assert_eq!(card.source_hash, card_source_hash(source));
}

/// An unavailable route cannot consume a pitch card or change occupied zones and resources.
fn assert_alternate_rejection(game: &GameState) {
    assert_eq!(game.hand, [CardIdx(0), CardIdx(1), CardIdx(2), CardIdx(9)]);
    assert_eq!(game.library, [CardIdx(3), CardIdx(4), CardIdx(5)]);
    assert_eq!(game.graveyard, [CardIdx(6)]);
    assert_eq!(game.exile, [CardIdx(7)]);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(8)));
    assert!(!game.battlefield[0].tapped);
    assert_eq!(game.next_uid, 21);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.life_gained, 0);
    assert_eq!(game.spells_cast_this_turn, 0);
    assert_eq!(game.seen, 0);
    assert!(game.card_first_seen.is_empty());
    assert!(game.graveyard_seen.is_empty());
    assert!(game.completion.is_none());
}

/// The printed payment moves only the spell and any independently supported draw.
fn assert_normal_cast(game: &GameState, draws: bool) {
    assert_eq!(
        game.hand,
        if draws {
            vec![CardIdx(1), CardIdx(2), CardIdx(9), CardIdx(5)]
        } else {
            vec![CardIdx(1), CardIdx(2), CardIdx(9)]
        }
    );
    assert_eq!(
        game.library,
        if draws {
            vec![CardIdx(3), CardIdx(4)]
        } else {
            vec![CardIdx(3), CardIdx(4), CardIdx(5)]
        }
    );
    assert_eq!(game.graveyard, [CardIdx(6), CardIdx(0)]);
    assert_eq!(game.exile, [CardIdx(7)]);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(8)));
    assert!(!game.battlefield[0].tapped);
    assert_eq!(game.next_uid, 21);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.life_gained, 0);
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_eq!(game.seen, u32::from(draws));
    assert_eq!(game.graveyard_seen.get(&CardIdx(0)), Some(&1));
    assert!(game.completion.is_none());
}

/// Rejected free, short, and wrong-color attempts must still allow the full printed payment.
fn assert_disabled_alternate_actions(source: &CardRow, expected_cost: Cost, draws: bool) {
    for rejected_pool in [
        ManaPool::default(),
        ManaPool {
            colorless: 1,
            ..ManaPool::default()
        },
        ManaPool {
            fixed: [0, 1, 0, 0, 0],
            colorless: expected_cost.generic,
            ..ManaPool::default()
        },
        ManaPool {
            fixed: [0, 1, 0, 0, 0],
            colorless: expected_cost.generic + 1,
            ..ManaPool::default()
        },
    ] {
        let (cards, mut game) = alternate_scenario(source);
        let expected_rng = game.library_rng.clone();
        let mut pool = rejected_pool.clone();
        assert_eq!(attempt(&cards, &mut game, &mut pool), None);
        assert_eq!(pool, rejected_pool);
        assert_alternate_rejection(&game);
        assert_library_rng(&game.library_rng, &expected_rng);
        pool = ManaPool {
            fixed: [0, 2, 0, 0, 0],
            colorless: expected_cost.generic,
            ..ManaPool::default()
        };
        assert_eq!(
            attempt(&cards, &mut game, &mut pool),
            Some(expected_cost.total())
        );
        assert_eq!(pool, ManaPool::default());
        assert_normal_cast(&game, draws);
        assert_library_rng(&game.library_rng, &expected_rng);
    }
}

/// Run a complete scheduled turn rather than calling an effect helper without its event.
fn execute_turn(cards: &SimDeck, game: &mut GameState, census: &mut TurnCensus, turn: usize) {
    run_turn(
        cards,
        game,
        census,
        &CommanderProfile::new(cards),
        &mut Vec::new(),
        turn,
        false,
    );
    assert!(census.turn_observed[turn - 1]);
    assert!(game.completion.is_none());
}

/// Emit the production report from an executed census and the actual final state.
fn report_game(
    cards: &SimDeck,
    game: GameState,
    census: TurnCensus,
    turns: usize,
) -> (GameLog, Value) {
    let config = game.opponents.config.clone();
    let log = finish_log(cards, game, census, 0, false, turns);
    let stats = aggregate(std::slice::from_ref(&log), cards, turns as u32);
    let mut report = super::report::json_report(
        &stats,
        cards,
        "Compiler safety actions",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    super::report::set_opponent_model(&mut report, cards.format, config, &stats);
    let json = serde_json::to_value(report).expect("the production report serializes");
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    assert_eq!(log.turns_observed, turns as u32);
    (log, json)
}

/// Find a source report without treating its compiler evidence as whole-card rules support.
fn source_report<'a>(json: &'a Value, source: &CardRow) -> &'a Value {
    let card = json["card_support"]
        .as_array()
        .expect("card support is a list")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("the source is reported");
    assert_eq!(card["source_hash"], card_source_hash(source));
    assert!(
        card["note"]
            .as_str()
            .expect("the source has a scope note")
            .contains("Compiler evidence only")
    );
    card
}

/// Require the exact unsupported wording, source span, hash, and reason in an emitted report.
fn assert_report_unsupported(json: &Value, source: &CardRow, text: &str, reason: &str) {
    let card = source_report(json, source);
    let ability = card["abilities"]
        .as_array()
        .expect("ability evidence is a list")
        .iter()
        .find(|ability| ability["source"] == text)
        .expect("the exact ability wording is reported");
    assert_eq!(ability["status"], "unsupported");
    assert!(
        ability["reason"]
            .as_str()
            .expect("unsupported evidence has a reason")
            .contains(reason)
    );
    assert!(ability["rule_id"].is_null());
    let start = source
        .oracle_text
        .find(text)
        .expect("the wording comes from the source");
    assert_eq!(ability["span_start"], start);
    assert_eq!(ability["span_end"], start + text.len());
}

/// A full pilot turn cannot use an unsupported alternative even with a matching blue hand card.
fn assert_alternate_report(source: &CardRow) {
    let (cards, mut game) = alternate_scenario(source);
    let expected_rng = game.library_rng.clone();
    let mut census = TurnCensus::new(1, cards.cards.len());
    execute_turn(&cards, &mut game, &mut census, 1);
    assert_alternate_rejection(&game);
    assert_library_rng(&game.library_rng, &expected_rng);
    let (log, json) = report_game(&cards, game, census, 1);
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(log.life_paid, 0);
    assert_eq!(log.library_size, [3]);
    assert_report_unsupported(
        &json,
        source,
        &source.oracle_text,
        "has no executable grammar rule",
    );
}

/// Check Force of Will's disabled life payment and its separate full five-mana cast route.
pub(super) fn assert_force_of_will_payment(source: &CardRow) {
    assert_eq!(source.mana_cost, "{3}{U}{U}");
    let expected_cost = Cost {
        generic: 3,
        pips: [0, 2, 0, 0, 0],
        ..Cost::default()
    };
    for source in renamed(source) {
        assert_disabled_alternate_source(&source, expected_cost, false);
        assert_disabled_alternate_actions(&source, expected_cost, false);
        assert_alternate_report(&source);
    }
}

/// Check Force of Negation's disabled turn gate and its separate full three-mana cast route.
pub(super) fn assert_force_of_negation_payment(source: &CardRow) {
    assert_eq!(source.mana_cost, "{1}{U}{U}");
    let expected_cost = Cost {
        generic: 1,
        pips: [0, 2, 0, 0, 0],
        ..Cost::default()
    };
    for source in renamed(source) {
        assert_disabled_alternate_source(&source, expected_cost, false);
        assert_disabled_alternate_actions(&source, expected_cost, false);
        assert_alternate_report(&source);
    }
}

/// Pinned life-payment text cannot become a free pitch spell.
#[test]
fn pinned_force_of_will_declines_life_alternate_but_pays_printed_cost() {
    let source = named_row("Force of Will");
    assert_eq!(source.oracle_id, "956381ba-6d37-4a8a-846c-bad79222dbee");
    assert_force_of_will_payment(&source);
}

/// Pinned off-turn text cannot become an unconditional free pitch spell.
#[test]
fn pinned_force_of_negation_declines_turn_alternate_but_pays_printed_cost() {
    let source = named_row("Force of Negation");
    assert_eq!(source.oracle_id, "ac2173f9-f223-440a-9231-fd98762bdc6f");
    assert_force_of_negation_payment(&source);
}

/// Extra mana, life, and turn clauses must remain intact instead of enabling a weaker payment.
#[test]
fn alternate_cost_extra_mana_life_and_turn_requirements_remain_disabled() {
    let source = named_row("Force of Will");
    let expected_cost = Cost {
        generic: 3,
        pips: [0, 2, 0, 0, 0],
        ..Cost::default()
    };
    for clause in [
        "You may pay 1 life and exile a blue card from your hand rather than pay this spell's mana cost.",
        "You may pay {1} and exile a blue card from your hand rather than pay this spell's mana cost.",
        "You may exile a blue card from your hand and pay {1} rather than pay this spell's mana cost.",
        "If it's not your turn, you may exile a blue card from your hand rather than pay this spell's mana cost.",
        "You may exile a blue card from your hand rather than pay this spell's mana cost if it's not your turn.",
    ] {
        let mut changed = source.clone();
        changed.oracle_text = format!("{clause}\nDraw a card.");
        for changed in renamed(&changed) {
            assert_disabled_alternate_source(&changed, expected_cost, true);
            assert_disabled_alternate_actions(&changed, expected_cost, true);
        }
    }
}

/// The exact pitch-only control must use a matching physical blue card and resolve its draw.
#[test]
fn exact_pitch_only_control_pays_with_a_real_blue_hand_card() {
    let source = named_row("Force of Will");
    for mut changed in renamed(&source) {
        changed.oracle_text = "You may exile a blue card from your hand rather than pay this spell's mana cost.\nDraw a card.".into();
        assert_ne!(card_source_hash(&changed), card_source_hash(&source));
        let (cards, mut game) = alternate_scenario(&changed);
        game.hand = vec![CardIdx(0), CardIdx(2), CardIdx(1), CardIdx(9)];
        let cost = cards[CardIdx(0)]
            .spell_data
            .alternative_cast_cost
            .expect("the exact pitch-only grammar lowers");
        assert_eq!(cost.count, 1);
        assert_eq!(cost.filter.color, Some(ManaColor::Blue));
        assert_eq!(cost.payoff, AlternativeCostPayoff::None);
        let expected_rng = game.library_rng.clone();
        let original_pool = ManaPool {
            colorless: 1,
            ..ManaPool::default()
        };
        let mut pool = original_pool.clone();
        assert_eq!(attempt(&cards, &mut game, &mut pool), Some(0));
        assert_eq!(pool, original_pool);
        assert_eq!(game.hand, [CardIdx(2), CardIdx(9), CardIdx(5)]);
        assert_eq!(game.library, [CardIdx(3), CardIdx(4)]);
        assert_eq!(game.exile, [CardIdx(7), CardIdx(1)]);
        assert_eq!(game.graveyard, [CardIdx(6), CardIdx(0)]);
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.seen, 1);
        assert_eq!(game.spells_cast_this_turn, 1);
        assert_library_rng(&game.library_rng, &expected_rng);
    }
}

/// Keep the pinned attack source and a renamed source with the same self-reference grammar.
fn dwynen_sources(source: &CardRow) -> [CardRow; 2] {
    let mut control = source.clone();
    control.name = "Renamed attack source".into();
    control.oracle_text = source.oracle_text.replace("Dwynen", &control.name);
    [source.clone(), control]
}

/// Retain the source attack event while refusing its unsupported scaled life result.
fn assert_dwynen_source(source: &CardRow, fixed_gain: bool) {
    let oracle = parse_oracle_card(source);
    let trigger = oracle
        .abilities
        .iter()
        .find_map(|ability| match ability {
            OracleAbility::Triggered(trigger) => Some(trigger),
            _ => None,
        })
        .expect("the printed source attack trigger parses");
    assert_eq!(
        trigger.event,
        OracleTriggerEvent::Attacks(ObjectSubject::ThisPermanent)
    );
    let card = parse_sim_card(source);
    if fixed_gain {
        assert!(matches!(
            trigger.effects.as_slice(),
            [OracleEffect::GainLife(2)]
        ));
        let abilities = trigger.to_runtime_all();
        assert_eq!(abilities.len(), 1);
        assert!(matches!(
            abilities[0].effect_sequence(),
            [SimEffect::GainLife(2)]
        ));
        assert!(
            card.unlocked_abilities(0)
                .any(|ability| ability.trigger == SimTrigger::Attacks)
        );
    } else {
        assert!(
            matches!(trigger.effects.as_slice(), [OracleEffect::Unsupported(text)]
            if text.to_ascii_lowercase().contains("for each attacking elf"))
        );
        assert!(trigger.to_runtime_all().is_empty());
        assert!(
            card.unlocked_abilities(0)
                .all(|ability| ability.trigger != SimTrigger::Attacks)
        );
    }
}

/// Give controlled Elf bodies their printed power, toughness, and actual haste wording.
fn elf(name: &str, cost: &str, power: u32, haste: bool) -> CardRow {
    let mut card = row(
        name,
        cost,
        "Creature — Elf",
        if haste { "Haste" } else { "" },
    );
    card.power = Some(power.to_string());
    card.toughness = Some("6".into());
    card
}

/// Fund the printed source and two other Elf casts before testing their attack timing.
fn dwynen_scenario(source: &CardRow) -> (SimDeck, GameState) {
    let mut rows = vec![
        source.clone(),
        elf("Mature Elf", "{99}", 2, false),
        elf("New Elf without haste", "{1}", 5, false),
        elf("New Elf with haste", "{1}", 1, true),
        row("Library bottom", "{99}", "Sorcery", ""),
        row("Library middle", "{99}", "Sorcery", ""),
        row("Next turn draw", "{99}", "Sorcery", ""),
        row("Graveyard marker", "{99}", "Sorcery", ""),
        row("Exile marker", "{99}", "Sorcery", ""),
    ];
    rows.extend((0..6).map(|index| {
        row(
            &format!("Funding Forest {index}"),
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        )
    }));
    rows.push(row("Hand marker", "{99}", "Sorcery", ""));
    let cards = deck(&rows);
    let expected_cost = Cost {
        generic: 2,
        pips: [0, 0, 0, 0, 2],
        ..Cost::default()
    };
    assert_eq!(cards[CardIdx(0)].cost, expected_cost);
    assert_eq!(cards[CardIdx(0)].min_cost, expected_cost);
    for index in 0..=3 {
        assert!(
            cards[CardIdx(index)]
                .subtypes
                .iter()
                .any(|subtype| subtype == "elf")
        );
    }
    let mut game = main_phase_state(vec![0, 2, 3, 15], vec![4, 5, 6]);
    game.graveyard = vec![CardIdx(7)];
    game.exile = vec![CardIdx(8)];
    game.battlefield
        .push(new_perm_with(20, &cards, CardIdx(1), 0, false));
    for index in 9..=14 {
        game.battlefield
            .push(new_perm_with(index + 20, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 40;
    assert!(!cards[CardIdx(0)].flags.has_haste);
    assert!(!cards[CardIdx(2)].flags.has_haste);
    assert!(cards[CardIdx(3)].flags.has_haste);
    assert_eq!(game.opponents.seats.len(), 1);
    assert!(game.opponents.seats[0].active);
    (cards, game)
}

/// Find one physical creature rather than assuming the pilot's cast order.
fn permanent(game: &GameState, index: u32) -> &Permanent {
    game.battlefield
        .iter()
        .find(|permanent| permanent.card.deck_idx() == Some(CardIdx(index)))
        .expect("the physical creature is on the battlefield")
}

/// Haste permits only the new hasty Elf to attack on the source's paid entry turn.
fn assert_dwynen_entry_turn(game: &GameState, census: &TurnCensus) {
    assert_eq!(game.hand, [CardIdx(15)]);
    assert_eq!(game.library, [CardIdx(4), CardIdx(5), CardIdx(6)]);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_gained, 0);
    assert_eq!(game.spells_cast_this_turn, 3);
    assert!(game.attacked_this_turn);
    for index in [0, 2] {
        let source = permanent(game, index);
        assert_eq!(source.entered_turn, 1);
        assert!(source.summoning_sick && !source.tapped);
    }
    assert!(permanent(game, 1).tapped && permanent(game, 3).tapped);
    assert_eq!(census.mana_spent, [6.0, 0.0]);
    assert_eq!(census.attackers_turn, [2, 0]);
    assert_eq!(census.attack_power, [3, 0]);
}

/// The second turn must declare real attacks against a live defender and leave combat potential unapplied.
fn assert_dwynen_attack_turn(game: &GameState, fixed_gain: bool) {
    assert!(game.attacked_this_turn);
    for index in 0..=3 {
        let attacker = permanent(game, index);
        assert!(attacker.tapped && !attacker.summoning_sick);
    }
    assert_eq!(game.battlefield.len(), 10);
    assert_eq!(game.hand, [CardIdx(15), CardIdx(6)]);
    assert_eq!(game.library, [CardIdx(4), CardIdx(5)]);
    assert_eq!(game.graveyard, [CardIdx(7)]);
    assert_eq!(game.exile, [CardIdx(8)]);
    assert_eq!(game.life, if fixed_gain { 22 } else { 20 });
    assert_eq!(game.life_gained, if fixed_gain { 2 } else { 0 });
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.spells_cast_this_turn, 0);
    assert_eq!(game.seen, 1);
    assert_eq!(game.damage_dealt_this_turn, 0);
    assert_eq!(game.opponent_life_lost, 0);
    assert!(game.opponents.seats[0].active);
    assert_eq!(game.opponents.seats[0].life, 20);
    assert!(game.completion.is_none());
}

/// Reports must distinguish the unsupported reward from actual cast spend and combat potential.
fn assert_dwynen_report(source: &CardRow, fixed_gain: bool, log: &GameLog, json: &Value) {
    assert_eq!(log.mana_spent, [6.0, 0.0]);
    assert_eq!(log.mana_available, [6.0, 6.0]);
    assert_eq!(log.mana_remaining, [0.0, 6.0]);
    assert_eq!(log.attackers, [2, 4]);
    assert_eq!(log.attack_power, [3, 11]);
    assert_eq!(log.player_damage, [3, 14]);
    assert_eq!(log.opponent_life_loss, [0, 0]);
    assert_eq!(log.life_gained, if fixed_gain { 2 } else { 0 });
    assert_eq!(log.life_paid, 0);
    assert_eq!(log.library_size, [3, 2]);
    assert_eq!(log.card_first_battlefield.get(&0), Some(&1));
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 6.0);
    assert_eq!(json["combat"]["attack_power_avg_by_turn"]["2"], 11.0);
    assert_eq!(json["combat"]["attackers_by_turn"]["2"], 4.0);
    assert_eq!(
        json["draw"]["avg_life_gained"],
        if fixed_gain { 2.0 } else { 0.0 }
    );
    let text = source
        .oracle_text
        .lines()
        .last()
        .expect("the source ends with its attack trigger");
    if fixed_gain {
        let card = source_report(json, source);
        let ability = card["abilities"]
            .as_array()
            .expect("ability evidence is a list")
            .iter()
            .find(|ability| ability["source"] == text)
            .expect("the changed attack wording is reported");
        assert_eq!(ability["status"], "supported");
        assert!(ability["reason"].is_null());
    } else {
        assert_report_unsupported(json, source, text, "cannot lower as a complete ability");
    }
}

/// Pay the source's printed cost, observe its sick turn, then exercise its real attack and report.
fn execute_dwynen_scenario(source: &CardRow, fixed_gain: bool) {
    assert_dwynen_source(source, fixed_gain);
    let (cards, mut game) = dwynen_scenario(source);
    let expected_rng = game.library_rng.clone();
    let mut census = TurnCensus::new(2, cards.cards.len());
    execute_turn(&cards, &mut game, &mut census, 1);
    assert_dwynen_entry_turn(&game, &census);
    execute_turn(&cards, &mut game, &mut census, 2);
    assert_dwynen_attack_turn(&game, fixed_gain);
    assert_library_rng(&game.library_rng, &expected_rng);
    let (log, json) = report_game(&cards, game, census, 2);
    assert_dwynen_report(source, fixed_gain, &log, &json);
}

/// Dwynen's unsupported scaled result stays inert after a full paid cast and later attack.
pub(super) fn assert_dwynen_attack_safety(source: &CardRow) {
    assert_eq!(source.mana_cost, "{2}{G}{G}");
    for source in dwynen_sources(source) {
        execute_dwynen_scenario(&source, false);
    }
}

/// Changing only the result wording enables fixed life gain on the same source's actual attack.
pub(super) fn assert_dwynen_fixed_attack_control(source: &CardRow) {
    for source in dwynen_sources(source) {
        let mut changed = source.clone();
        changed.oracle_text = source.oracle_text.replace(
            "you gain 1 life for each attacking Elf you control",
            "you gain 2 life",
        );
        assert_ne!(changed.oracle_text, source.oracle_text);
        assert_ne!(card_source_hash(&changed), card_source_hash(&source));
        execute_dwynen_scenario(&changed, true);
    }
}

/// Several real attacking Elves must not turn an unsupported scaled reward into fixed life gain.
#[test]
fn pinned_dwynen_attacks_without_invented_scaled_life() {
    let source = named_row("Dwynen, Gilt-Leaf Daen");
    assert_eq!(source.oracle_id, "30d0d75f-e94c-460b-b957-9f1d655c0f65");
    assert_dwynen_attack_safety(&source);
}

/// The fixed-result control proves that the attack timing and report path can observe life gain.
#[test]
fn dwynen_fixed_attack_life_control_executes_and_reaches_report() {
    assert_dwynen_fixed_attack_control(&named_row("Dwynen, Gilt-Leaf Daen"));
}
