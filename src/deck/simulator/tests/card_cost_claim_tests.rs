//! Pinned mana modes and cast costs use physical resources and executed reports.
//!
//! These helpers prove individual clauses, not whole-card correctness. Emry's
//! affinity uses the wider `artifacts` source, not the selected top-250 set.

use super::card_report_tests::{execute_turn, report_executed_turn};
use super::card_runtime_tests::{attempt_index, renamed};
use super::card_test_support::{
    assert_library_rng, main_phase_state as state, named_row, pinned_revision,
};
use super::game::{Activation, GameLog, GameState, ManaPool, new_perm_with};
use super::game_effects::activation::resolve_activation;
use super::game_mana::{artifact_cost_reduction, effective_min_cost};
use super::game_run::mana::{activate_mana_mode, build_pool};
use super::model::{CardIdx, SimDeck};
use super::oracle_parser::cost::parse_cost;
use super::scenario_test_support::{deck, row};
use crate::db::CardRow;
use serde_json::Value;

/// Place a distinct physical permanent without firing an artificial entry event.
pub(super) fn place(cards: &SimDeck, game: &mut GameState, index: u32, tapped: bool) {
    game.next_uid += 1;
    game.battlefield.push(new_perm_with(
        game.next_uid,
        cards,
        CardIdx(index),
        0,
        tapped,
    ));
}

/// Select one printed mode while leaving its real payment to the action boundary.
pub(super) fn printed_activation(
    cards: &SimDeck,
    game: &GameState,
    pos: usize,
    id: usize,
) -> Activation {
    let source = &game.battlefield[pos];
    let index = source
        .card
        .deck_idx()
        .expect("the source is a physical card");
    let ability = cards[index]
        .unlocked_abilities(source.counters.charge)
        .find(|ability| ability.id == id)
        .expect("the requested printed mode lowered")
        .clone();
    let ability_cost = ability
        .activation
        .as_ref()
        .expect("the printed mode has costs")
        .mana_cost();
    Activation {
        pos,
        uid: source.uid,
        ability,
        ability_cost,
        paid_x: 0,
        draws: 0,
        sacrifice_uid: None,
        target_uid: None,
    }
}

/// A rejected action cannot consume zones, payment, counters, or random state.
pub(super) fn assert_unchanged(
    game: &GameState,
    before: &GameState,
    pool: &ManaPool,
    before_pool: &ManaPool,
) {
    assert_eq!(pool, before_pool);
    assert_eq!(game.hand, before.hand);
    assert_eq!(game.library, before.library);
    assert_eq!(game.graveyard, before.graveyard);
    assert_eq!(game.exile, before.exile);
    assert_eq!(
        format!("{:?}", game.battlefield),
        format!("{:?}", before.battlefield)
    );
    assert_eq!(game.next_uid, before.next_uid);
    assert_eq!(game.battlefield_seen, before.battlefield_seen);
    assert_eq!(game.graveyard_seen, before.graveyard_seen);
    assert_eq!(game.graveyard_generations, before.graveyard_generations);
    assert_eq!(game.card_first_seen, before.card_first_seen);
    assert_eq!(game.seen, before.seen);
    assert_eq!(game.awareness_cards, before.awareness_cards);
    assert_eq!(game.treasure_bank, before.treasure_bank);
    assert_eq!(game.life, before.life);
    assert_eq!(game.life_paid, before.life_paid);
    assert_eq!(game.life_gained, before.life_gained);
    assert_eq!(game.life_funded_draws, before.life_funded_draws);
    assert_eq!(game.spells_cast_this_turn, before.spells_cast_this_turn);
    assert_eq!(game.prowess_casts, before.prowess_casts);
    assert_eq!(game.replay_casts, before.replay_casts);
    assert_eq!(game.milled_self, before.milled_self);
    assert_eq!(game.milled_opp, before.milled_opp);
    assert_eq!(game.damage_dealt_this_turn, before.damage_dealt_this_turn);
    assert_eq!(game.opponent_life_lost, before.opponent_life_lost);
    assert_eq!(game.player_counters, before.player_counters);
    assert_eq!(game.activated_this_turn, before.activated_this_turn);
    assert_eq!(game.activated_once, before.activated_once);
    assert_eq!(game.triggered_this_turn, before.triggered_this_turn);
    assert_eq!(game.milestones_by_turn, before.milestones_by_turn);
    assert_eq!(game.flashback_permissions, before.flashback_permissions);
    assert_eq!(game.completion, before.completion);
    assert_eq!(game.failed_draw, before.failed_draw);
    assert_library_rng(&game.library_rng, &before.library_rng);
}

/// Keep compiler evidence tied to the exact pinned wording and input hash.
pub(super) fn source_record<'a>(json: &'a Value, source: &CardRow) -> &'a Value {
    let revision = pinned_revision(source);
    let record = json["card_support"]
        .as_array()
        .expect("the report contains compiler evidence")
        .iter()
        .find(|record| record["name"] == source.name)
        .expect("the source remains in the report");
    assert_eq!(record["source_hash"], revision["flattened_input_sha256"]);
    assert!(
        record["note"]
            .as_str()
            .expect("scope note")
            .contains("Compiler evidence only")
    );
    for ability in record["abilities"].as_array().expect("ability records") {
        let wording = ability["source"].as_str().expect("original wording");
        assert!(source.oracle_text.contains(wording), "{wording}");
    }
    record
}

/// Read one ability record by exact wording rather than a card-name rule.
pub(super) fn ability_record<'a>(record: &'a Value, wording: &str) -> &'a Value {
    record["abilities"]
        .as_array()
        .expect("ability records")
        .iter()
        .find(|ability| ability["source"] == wording)
        .expect("the exact clause remains in the report")
}

/// Require the published model limit beside the measured result.
pub(super) fn assert_limit(json: &Value, text: &str) {
    assert!(
        json["assumptions"]
            .as_array()
            .expect("report limits")
            .iter()
            .any(|limit| { limit.as_str().is_some_and(|limit| limit.contains(text)) }),
        "missing report limit: {text}"
    );
}

/// Compare a one-turn census with its public aggregate mana fields.
pub(super) fn assert_mana_report(
    log: &GameLog,
    json: &Value,
    available: f64,
    spent: f64,
    remaining: f64,
) {
    assert_eq!(log.mana_available, [available]);
    assert_eq!(log.mana_spent, [spent]);
    assert_eq!(log.mana_remaining, [remaining]);
    assert_eq!(json["mana"]["avg_available_by_turn"]["1"], available);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], spent);
    assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], remaining);
}

/// Signet cannot borrow its own output, and its paid mode taps only once.
pub(super) fn assert_signet_cost(source: &CardRow) {
    for source in renamed(source) {
        let cards = deck(&[source, row("Paid spell", "{W}{U}", "Sorcery", "")]);
        for funding in [0, 1] {
            let mut game = state(vec![1], vec![]);
            place(&cards, &mut game, 0, false);
            let mut pool = ManaPool {
                colorless: funding,
                ..Default::default()
            };
            let before = game.clone();
            let before_pool = pool.clone();
            let permanent = game.battlefield[0].clone();
            assert_eq!(
                activate_mana_mode(&cards, &mut game, &permanent, &mut pool, 1),
                funding == 1
            );
            if funding == 0 {
                assert_unchanged(&game, &before, &pool, &before_pool);
                continue;
            }
            assert_eq!(pool.fixed, [1, 1, 0, 0, 0]);
            assert_eq!(pool.colorless, 0);
            assert!(game.battlefield[0].tapped);
            assert_eq!(game.life, 20);
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
                Some(2)
            );
            assert_eq!(game.graveyard, [CardIdx(1)]);
            assert!(game.hand.is_empty());
            pool.colorless = 1;
            let before = game.clone();
            let before_pool = pool.clone();
            let permanent = game.battlefield[0].clone();
            assert!(!activate_mana_mode(
                &cards, &mut game, &permanent, &mut pool, 1
            ));
            assert_unchanged(&game, &before, &pool, &before_pool);
        }
    }
}

/// Executed turns distinguish a funded Signet from one with no activation payment.
pub(super) fn assert_signet_cost_report(source: &CardRow) {
    let cards = deck(&[
        source.clone(),
        row("Paid spell", "{W}{U}", "Sorcery", ""),
        row("External payment", "{99}", "Artifact", "{T}: Add {C}."),
    ]);
    for funded in [false, true] {
        let mut game = state(vec![1], vec![]);
        if funded {
            place(&cards, &mut game, 2, false);
        }
        place(&cards, &mut game, 0, false);
        let rng = game.library_rng.clone();
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.spells_cast_this_turn, u32::from(funded));
        assert_eq!(
            game.graveyard,
            if funded { vec![CardIdx(1)] } else { vec![] }
        );
        assert_eq!(game.hand, if funded { vec![] } else { vec![CardIdx(1)] });
        assert_eq!(
            game.battlefield.last().expect("Signet remains").tapped,
            funded
        );
        assert_library_rng(&game.library_rng, &rng);
        let (log, json) = report_executed_turn(&cards, game, census);
        let mana = if funded { 2.0 } else { 0.0 };
        assert_mana_report(&log, &json, mana, mana, 0.0);
        assert_eq!(
            ability_record(source_record(&json, source), &source.oracle_text)["status"],
            "supported"
        );
        assert_limit(&json, "Mana modes share the source's tap state");
    }
}

/// Talisman's printed modes differ in color and self-damage but share one tap.
pub(super) fn assert_talisman_modes(source: &CardRow) {
    for source in renamed(source) {
        for (mode, cost, accepted) in [
            (0, "{C}", true),
            (0, "{U}", false),
            (1, "{U}", true),
            (1, "{B}", true),
            (1, "{R}", false),
            (1, "{C}", false),
            (1, "{U}{B}", false),
        ] {
            let cards = deck(&[source.clone(), row("Mode payment", cost, "Sorcery", "")]);
            let mut game = state(vec![1], vec![]);
            place(&cards, &mut game, 0, false);
            let rng = game.library_rng.clone();
            let mut pool = ManaPool::default();
            let activation = printed_activation(&cards, &game, 0, mode);
            resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
            assert_eq!(pool.total(), 1);
            assert_eq!(pool.colorless, u32::from(mode == 0));
            assert_eq!(game.life, if mode == 0 { 20 } else { 19 });
            assert_eq!(game.life_paid, 0, "damage is not a life payment");
            assert_eq!(game.opponent_life_lost, 0);
            assert_eq!(game.damage_dealt_this_turn, 0);
            assert!(game.battlefield[0].tapped);
            let other = printed_activation(&cards, &game, 0, 1 - mode);
            let before = game.clone();
            let before_pool = pool.clone();
            resolve_activation(&cards, &mut game, &mut pool, 1, &other);
            assert_unchanged(&game, &before, &pool, &before_pool);
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
                accepted.then_some(1)
            );
            if !accepted {
                assert_unchanged(&game, &before, &pool, &before_pool);
            }
            assert_eq!(pool.total(), u32::from(!accepted));
            assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
            assert_eq!(
                game.graveyard,
                if accepted { vec![CardIdx(1)] } else { vec![] }
            );
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// The native turn policy chooses one colored mode, not both printed mana modes.
pub(super) fn assert_talisman_modes_report(source: &CardRow) {
    for (cost, accepted) in [("{U}", true), ("{C}", false)] {
        let cards = deck(&[source.clone(), row("Mode payment", cost, "Sorcery", "")]);
        let mut game = state(vec![1], vec![]);
        place(&cards, &mut game, 0, false);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.life, 19);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        assert_eq!(game.hand, if accepted { vec![] } else { vec![CardIdx(1)] });
        assert!(game.battlefield[0].tapped);
        let (log, json) = report_executed_turn(&cards, game, census);
        let spent = f64::from(u32::from(accepted));
        assert_mana_report(&log, &json, 1.0, spent, 1.0 - spent);
        assert_eq!(json["draw"]["avg_life_paid"], 0.0);
        let record = source_record(&json, source);
        for wording in source.oracle_text.lines() {
            assert_eq!(ability_record(record, wording)["status"], "supported");
        }
        assert_limit(&json, "Mana modes share the source's tap state");
    }
}

/// Mind Stone needs separate mana and its shared untapped source for a draw.
pub(super) fn assert_mind_stone_modes(source: &CardRow) {
    for source in renamed(source) {
        let cards = deck(&[
            source,
            row("Undrawn bottom", "{99}", "Sorcery", ""),
            row("Physical draw", "{99}", "Sorcery", ""),
        ]);
        for (mana_mode_first, funding, accepted) in
            [(false, 0, false), (false, 1, true), (true, 0, false)]
        {
            let mut game = state(vec![], vec![1, 2]);
            place(&cards, &mut game, 0, false);
            let rng = game.library_rng.clone();
            let mut pool = ManaPool {
                colorless: funding,
                ..Default::default()
            };
            if mana_mode_first {
                let source = game.battlefield[0].clone();
                assert!(activate_mana_mode(&cards, &mut game, &source, &mut pool, 1));
                assert_eq!(pool.colorless, 1);
                assert!(game.graveyard.is_empty());
            }
            let draw = printed_activation(&cards, &game, 0, 1);
            let before = game.clone();
            let before_pool = pool.clone();
            resolve_activation(&cards, &mut game, &mut pool, 1, &draw);
            if !accepted {
                assert_unchanged(&game, &before, &pool, &before_pool);
            } else {
                assert_eq!(game.graveyard, [CardIdx(0)]);
                assert_eq!(game.hand, [CardIdx(2)]);
                assert_eq!(game.library, [CardIdx(1)]);
                assert!(game.battlefield.is_empty());
                assert_eq!(game.graveyard_generations.get(&CardIdx(0)), Some(&1));
                assert_eq!(game.seen, 1);
                assert_eq!(pool.total(), 0);
                let before = game.clone();
                let before_pool = pool.clone();
                resolve_activation(&cards, &mut game, &mut pool, 1, &draw);
                assert_unchanged(&game, &before, &pool, &before_pool);
            }
            assert_eq!(game.life, 20);
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// Ordinary turns reserve Mind Stone's tap for mana and cannot also draw from it.
pub(super) fn assert_mind_stone_modes_report(source: &CardRow) {
    let cards = deck(&[
        source.clone(),
        row("Physical draw", "{99}", "Sorcery", ""),
        row("External payment", "{99}", "Artifact", "{T}: Add {C}."),
    ]);
    let mut game = state(vec![], vec![1]);
    place(&cards, &mut game, 0, false);
    place(&cards, &mut game, 2, false);
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.battlefield.len(), 2);
    assert!(game.battlefield.iter().all(|permanent| permanent.tapped));
    assert!(game.hand.is_empty() && game.graveyard.is_empty());
    assert_eq!(game.library, [CardIdx(1)]);
    let (log, json) = report_executed_turn(&cards, game, census);
    assert_mana_report(&log, &json, 2.0, 0.0, 2.0);
    assert_eq!(log.cards_seen, [0]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 0.0);
    let record = source_record(&json, source);
    for wording in source.oracle_text.lines() {
        assert_eq!(ability_record(record, wording)["status"], "supported");
    }
    assert_limit(&json, "Mana modes share the source's tap state");
}

/// Build a Thrill scenario whose rewards cannot act as its discard payment.
fn discard_scenario(source: &CardRow, fodder: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row("Discard payment", "{99}", "Sorcery", ""),
        row("Library bottom", "{99}", "Sorcery", ""),
        row("Second reward", "{99}", "Sorcery", ""),
        row("First reward", "{99}", "Sorcery", ""),
        row("Red funding", "{99}", "Artifact", "{T}: Add {R}{R}."),
    ]);
    let mut game = state(if fodder { vec![0, 1] } else { vec![0] }, vec![2, 3, 4]);
    place(&cards, &mut game, 5, false);
    (cards, game)
}

/// Thrill pays one actual hand card before drawing two distinct reward cards.
pub(super) fn assert_discard_cost(source: &CardRow) {
    for source in renamed(source) {
        for fodder in [false, true] {
            let (cards, mut game) = discard_scenario(&source, fodder);
            let mut pool = build_pool(&cards, &mut game, 1);
            assert_eq!(cards[CardIdx(0)].spell_data.additional_cost_discards, 1);
            let before = game.clone();
            let before_pool = pool.clone();
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
                fodder.then_some(2)
            );
            if !fodder {
                assert_unchanged(&game, &before, &pool, &before_pool);
                continue;
            }
            assert_eq!(game.hand, [CardIdx(4), CardIdx(3)]);
            assert_eq!(game.library, [CardIdx(2)]);
            assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0)]);
            assert_eq!(game.graveyard_generations.get(&CardIdx(1)), Some(&1));
            assert_eq!(game.seen, 2);
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_eq!(game.treasure_bank, 0);
            assert_eq!(game.life, 20);
            assert_eq!(pool.total(), 0);
            assert!(game.exile.is_empty());
            assert_library_rng(&game.library_rng, &before.library_rng);
        }
    }
}

/// Executed reports retain the cast-field and ordered-effect compiler-only records.
pub(super) fn assert_discard_cost_report(source: &CardRow) {
    for fodder in [false, true] {
        let (cards, mut game) = discard_scenario(source, fodder);
        let rng = game.library_rng.clone();
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.hand,
            if fodder {
                vec![CardIdx(4), CardIdx(3)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(
            game.library,
            if fodder {
                vec![CardIdx(2)]
            } else {
                vec![CardIdx(2), CardIdx(3), CardIdx(4)]
            }
        );
        assert_eq!(
            game.graveyard,
            if fodder {
                vec![CardIdx(1), CardIdx(0)]
            } else {
                vec![]
            }
        );
        assert_library_rng(&game.library_rng, &rng);
        let (log, json) = report_executed_turn(&cards, game, census);
        let spent = if fodder { 2.0 } else { 0.0 };
        assert_mana_report(&log, &json, 2.0, spent, 2.0 - spent);
        assert_eq!(log.cards_seen, [if fodder { 2 } else { 0 }]);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], spent);
        let record = source_record(&json, source);
        let cost = ability_record(
            record,
            "As an additional cost to cast this spell, discard a card.",
        );
        assert_eq!(cost["status"], "compiler_only");
        assert_eq!(
            cost["reason"],
            "Additional cost lowered to cast payment fields, not resolution effects."
        );
        let reward = ability_record(record, "Draw two cards.");
        assert_eq!(reward["status"], "compiler_only");
        assert_eq!(
            reward["reason"],
            "Ordered effects lowered, but their ordinary runtime resolution path is not verified."
        );
        assert_limit(
            &json,
            "Supported additional cast costs consume actual cards",
        );
    }
}

/// Give Emry distinct artifacts, a nonartifact, and five physical library cards.
fn affinity_scenario(source: &CardRow, artifacts: u32, tapped: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row("First physical artifact", "{99}", "Artifact", ""),
        row("Second physical artifact", "{99}", "Artifact", ""),
        row("Third physical artifact", "{99}", "Artifact", ""),
        row("Nonartifact", "{99}", "Enchantment", ""),
        row("Library bottom", "{99}", "Sorcery", ""),
        row("Mill fourth", "{99}", "Sorcery", ""),
        row("Mill third", "{99}", "Sorcery", ""),
        row("Mill second", "{99}", "Sorcery", ""),
        row("Mill first", "{99}", "Sorcery", ""),
        row("Blue funding", "", "Basic Land - Island", "{T}: Add {U}."),
    ]);
    let mut game = state(vec![0], vec![5, 6, 7, 8, 9]);
    for index in 1..=artifacts {
        place(&cards, &mut game, index, tapped);
    }
    place(&cards, &mut game, 4, false);
    (cards, game)
}

/// Emry discounts only generic mana from real artifacts without tapping them.
pub(super) fn assert_affinity_cost(source: &CardRow) {
    let [original, mut control] = renamed(source);
    // The entry trigger uses Emry's short name, not its full card name.
    let short_name = source
        .name
        .split(',')
        .next()
        .expect("a name has a first part");
    control.oracle_text = control.oracle_text.replace(short_name, &control.name);
    for source in [original, control] {
        for (artifacts, generic, blue, accepted) in [
            (0, 2, 1, true),
            (1, 1, 1, true),
            (2, 0, 1, true),
            (3, 0, 1, true),
            (2, 1, 0, false),
            (0, 1, 1, false),
        ] {
            for tapped in [false, true] {
                let (cards, mut game) = affinity_scenario(&source, artifacts, tapped);
                assert!(cards[CardIdx(0)].keyword_abilities.affinity);
                assert_eq!(
                    artifact_cost_reduction(&cards, &cards[CardIdx(0)], &game.battlefield),
                    artifacts
                );
                let mut pool = ManaPool {
                    colorless: generic,
                    fixed: [0, blue, 0, 0, 0],
                    ..Default::default()
                };
                let before = game.clone();
                let before_pool = pool.clone();
                let expected = accepted.then_some(3u32.saturating_sub(artifacts).max(1));
                assert_eq!(
                    attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
                    expected
                );
                if !accepted {
                    assert_unchanged(&game, &before, &pool, &before_pool);
                    continue;
                }
                for (index, permanent) in
                    game.battlefield.iter().take(artifacts as usize).enumerate()
                {
                    assert_eq!(permanent.tapped, tapped);
                    assert_eq!(permanent.card.deck_idx(), Some(CardIdx(index as u32 + 1)));
                }
                assert_eq!(game.battlefield.len(), artifacts as usize + 2);
                assert_eq!(
                    game.battlefield
                        .last()
                        .expect("Emry entered")
                        .card
                        .deck_idx(),
                    Some(CardIdx(0))
                );
                assert_eq!(
                    game.graveyard,
                    [CardIdx(9), CardIdx(8), CardIdx(7), CardIdx(6)],
                    "source={}, artifacts={artifacts}, tapped={tapped}",
                    source.name
                );
                assert_eq!(game.library, [CardIdx(5)]);
                assert_eq!(game.milled_self, 4);
                assert!(game.hand.is_empty() && game.exile.is_empty());
                // Self-mill exposes four library cards without drawing them into hand.
                assert_eq!(game.seen, 4);
                assert_eq!(game.life, 20);
                assert_library_rng(&game.library_rng, &before.library_rng);
            }
        }
    }
}

/// Emry's wider-source report keeps affinity payment separate from its other clauses.
pub(super) fn assert_affinity_cost_report(source: &CardRow) {
    for artifacts in [0, 2] {
        let (cards, mut game) = affinity_scenario(source, artifacts, false);
        place(&cards, &mut game, 10, false);
        let rng = game.library_rng.clone();
        let census = execute_turn(&cards, &mut game);
        let accepted = artifacts == 2;
        let exposed = if accepted { 4 } else { 0 };
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        assert_eq!(game.milled_self, exposed);
        assert_eq!(game.seen, exposed);
        for permanent in game.battlefield.iter().take(artifacts as usize) {
            assert!(!permanent.tapped);
        }
        assert_eq!(
            game.library,
            if accepted {
                vec![CardIdx(5)]
            } else {
                (5..=9).map(CardIdx).collect()
            }
        );
        assert_library_rng(&game.library_rng, &rng);
        let (log, json) = report_executed_turn(&cards, game, census);
        let spent = f64::from(u32::from(accepted));
        assert_mana_report(&log, &json, 1.0, spent, 1.0 - spent);
        assert_eq!(log.self_milled, [exposed]);
        assert_eq!(log.cards_seen, [exposed]);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], f64::from(exposed));
        let record = source_record(&json, source);
        assert_eq!(
            ability_record(record, "When Emry enters, mill four cards.")["status"],
            "supported"
        );
        let replay = record["abilities"]
            .as_array()
            .expect("Emry clauses")
            .iter()
            .find(|ability| {
                ability["source"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("{T}: Choose target artifact card"))
            })
            .expect("unsupported replay clause remains visible");
        assert_eq!(replay["status"], "unsupported");
        assert_limit(
            &json,
            "Affinity discounts generic cost from the artifact count without tapping",
        );
    }
}

/// Supply Henge with a large creature that must not grant an unsupported discount.
fn reducer_scenario(source: &CardRow, generic: u32) -> (SimDeck, GameState) {
    let mut creature = row("Large creature", "{99}", "Creature", "");
    creature.power = Some("20".into());
    creature.toughness = Some("20".into());
    let mana = format!("{{T}}: Add {}{{G}}{{G}}.", "{C}".repeat(generic as usize));
    let cards = deck(&[
        source.clone(),
        creature,
        row("External funding", "{99}", "Artifact", &mana),
        row("Undrawn card", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![3]);
    place(&cards, &mut game, 1, false);
    place(&cards, &mut game, 2, false);
    (cards, game)
}

/// Henge's unsupported power reducer cannot lower its printed nine-mana cost.
pub(super) fn assert_unsupported_reducer(source: &CardRow) {
    for source in renamed(source) {
        for generic in [0, 7] {
            let (cards, mut game) = reducer_scenario(&source, generic);
            assert_eq!(
                effective_min_cost(&cards, &cards[CardIdx(0)], &game.battlefield),
                parse_cost("{7}{G}{G}")
            );
            let mut pool = build_pool(&cards, &mut game, 1);
            let before = game.clone();
            let before_pool = pool.clone();
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
                (generic == 7).then_some(9)
            );
            if generic == 0 {
                assert_unchanged(&game, &before, &pool, &before_pool);
            } else {
                assert_eq!(pool.total(), 0);
                assert!(game.hand.is_empty() && game.graveyard.is_empty());
                assert_eq!(game.battlefield.len(), 3);
                assert_eq!(game.library, [CardIdx(3)]);
                assert_eq!(game.life, 20);
                assert_eq!(game.seen, 0);
                assert_library_rng(&game.library_rng, &before.library_rng);
            }
        }
    }
}

/// Executed Henge reports preserve the reducer gap while its independent mana mode works.
pub(super) fn assert_unsupported_reducer_report(source: &CardRow) {
    for generic in [0, 7] {
        let (cards, mut game) = reducer_scenario(source, generic);
        let census = execute_turn(&cards, &mut game);
        let accepted = generic == 7;
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        assert_eq!(game.life, if accepted { 22 } else { 20 });
        assert_eq!(game.life_gained, if accepted { 2 } else { 0 });
        assert_eq!(game.seen, 0);
        assert_eq!(game.library, [CardIdx(3)]);
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_mana_report(
            &log,
            &json,
            f64::from(generic + 2),
            if accepted { 9.0 } else { 0.0 },
            2.0,
        );
        let record = source_record(&json, source);
        let reducer = ability_record(
            record,
            "This spell costs {X} less to cast, where X is the greatest power among creatures you control.",
        );
        assert_eq!(reducer["status"], "unsupported");
        assert_eq!(
            reducer["reason"],
            "No runtime ability was emitted. Separate card-level static fields are not verified by this record."
        );
        assert_eq!(
            ability_record(record, "{T}: Add {G}{G}. You gain 2 life.")["status"],
            "supported"
        );
        assert_limit(
            &json,
            "Variable or conditional static reducers stay unsupported",
        );
    }
}

/// Bind the paid Signet mode to its preserved source revision.
#[test]
fn pinned_signet_pays_before_tapping_and_reports_the_funded_cast() {
    let source = named_row("Azorius Signet");
    assert_signet_cost(&source);
    assert_signet_cost_report(&source);
}

/// Bind separate Talisman modes to real one-tap action and report checks.
#[test]
fn pinned_talisman_modes_share_a_tap_and_only_colored_mode_deals_damage() {
    let source = named_row("Talisman of Dominance");
    assert_talisman_modes(&source);
    assert_talisman_modes_report(&source);
}

/// Keep Mind Stone's successful draw action separate from the pilot's mana choice.
#[test]
fn pinned_mind_stone_needs_a_separate_payment_and_cannot_use_both_tap_modes() {
    let source = named_row("Mind Stone");
    assert_mind_stone_modes(&source);
    assert_mind_stone_modes_report(&source);
}

/// Pin one selected discard-cost source without duplicating Treasure-bank coverage.
#[test]
fn pinned_thrill_discards_a_physical_card_before_its_two_draw_rewards() {
    let source = named_row("Thrill of Possibility");
    assert_discard_cost(&source);
    assert_discard_cost_report(&source);
}

/// Wider artifact evidence proves affinity without widening the selected source set.
#[test]
fn pinned_emry_affinity_counts_real_artifacts_and_keeps_the_blue_payment() {
    let source = named_row("Emry, Lurker of the Loch");
    assert_affinity_cost(&source);
    assert_affinity_cost_report(&source);
}

/// A selected unsupported reducer cannot grant a cheap Henge cast.
#[test]
fn pinned_henge_power_reducer_stays_inert_in_actions_and_reports() {
    let source = named_row("The Great Henge");
    assert_unsupported_reducer(&source);
    assert_unsupported_reducer_report(&source);
}
