//! Pinned cost limits use real cast attempts and executed turn reports.
//!
//! Controlled starting zones do not prove opening-hand or timing rules. Ox's
//! eight-card Escape stays inert. Treasure rewards use the mana bank, not
//! battlefield artifacts. These assertions do not certify whole cards.

use super::aggregate::aggregate;
use super::card_runtime_tests::{attempt_index, renamed};
use super::card_test_support::{assert_library_rng, main_phase_state as state, named_row};
use super::cast_pass::{additional_costs_payable, cast_graveyard_spells, play_land};
use super::game::{GameLog, GameState, ManaPool, new_perm_with};
use super::game_commander::CommanderProfile;
use super::game_effects::current_creature_power;
use super::game_mana::{artifact_cost_reduction, payable};
use super::game_run::{TurnCensus, census::finish_log, mana::build_pool, run_turn};
use super::model::{CardIdx, SimDeck};
use super::oracle_ast::KeywordArgument;
use super::oracle_parser::{cost::parse_cost, parse_oracle_card};
use super::scenario_test_support::{deck, row};
use crate::db::CardRow;
use serde_json::Value;

/// Check cards, payment, counters, observations, and the separate library stream.
fn assert_rejected_resources(
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
    assert_eq!(game.battlefield_seen, before.battlefield_seen);
    assert_eq!(game.graveyard_seen, before.graveyard_seen);
    assert_eq!(game.graveyard_generations, before.graveyard_generations);
    assert_eq!(game.card_first_seen, before.card_first_seen);
    assert_eq!(game.next_uid, before.next_uid);
    assert_eq!(game.treasure_bank, before.treasure_bank);
    assert_eq!(game.seen, before.seen);
    assert_eq!(game.awareness_cards, before.awareness_cards);
    assert_eq!(game.life, before.life);
    assert_eq!(game.life_paid, before.life_paid);
    assert_eq!(game.life_gained, before.life_gained);
    assert_eq!(game.life_funded_draws, before.life_funded_draws);
    assert_eq!(game.milled_self, before.milled_self);
    assert_eq!(game.milled_opp, before.milled_opp);
    assert_eq!(game.replay_casts, before.replay_casts);
    assert_eq!(game.spells_cast_this_turn, before.spells_cast_this_turn);
    assert_eq!(game.prowess_casts, before.prowess_casts);
    assert_eq!(game.flashback_permissions, before.flashback_permissions);
    assert_eq!(game.milestones_by_turn, before.milestones_by_turn);
    assert_eq!(game.activated_this_turn, before.activated_this_turn);
    assert_eq!(game.activated_once, before.activated_once);
    assert_eq!(game.triggered_this_turn, before.triggered_this_turn);
    assert_eq!(game.player_counters, before.player_counters);
    assert_eq!(game.completion, before.completion);
    assert_eq!(game.failed_draw, before.failed_draw);
    assert_eq!(game.current_turn, before.current_turn);
    assert_eq!(game.current_phase, before.current_phase);
    assert_eq!(
        game.resolution.trigger_limit_reached,
        before.resolution.trigger_limit_reached
    );
    assert_library_rng(&game.library_rng, &before.library_rng);
}

/// Run scheduled turns through the same census used by ordinary simulations.
fn execute_turns(cards: &SimDeck, game: &mut GameState, turns: usize) -> TurnCensus {
    let mut census = TurnCensus::new(turns, cards.cards.len());
    let commander = CommanderProfile::new(cards);
    let mut sources = Vec::new();
    for turn in 1..=turns {
        run_turn(
            cards,
            game,
            &mut census,
            &commander,
            &mut sources,
            turn,
            false,
        );
    }
    census
}

/// Preserve actual observations through the production log, aggregate, and JSON report.
fn executed_report(
    cards: &SimDeck,
    game: GameState,
    census: TurnCensus,
    turns: usize,
) -> (GameLog, Value) {
    let log = finish_log(cards, game, census, 0, false, turns);
    assert_eq!(log.turns_observed as usize, turns);
    assert!(
        log.turn_observations.iter().all(|turn| {
            turn.draw && turn.main && turn.combat && turn.end_step && turn.end_turn
        })
    );
    let stats = aggregate(std::slice::from_ref(&log), cards, turns as u32);
    let report = super::report::json_report(
        &stats,
        cards,
        "Pinned cost boundary",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    let json = serde_json::to_value(report).expect("the production report serializes");
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    (log, json)
}

/// Bind report evidence to the pinned simulation input, not only its readable name.
fn source_support<'a>(json: &'a Value, source: &CardRow) -> &'a Value {
    let support = json["card_support"]
        .as_array()
        .expect("the report contains card support")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("the pinned source has a support record");
    assert_eq!(
        support["source_hash"],
        super::support::card_source_hash(source)
    );
    assert!(
        support["note"]
            .as_str()
            .expect("support has a scope note")
            .contains("Compiler evidence only")
    );
    support
}

/// Keep the artifact-only payment gap visible beside executed creature-route results.
fn assert_dispute_support(json: &Value, source: &CardRow) {
    let support = source_support(json, source);
    assert!(
        support["abilities"]
            .as_array()
            .expect("support lists abilities")
            .iter()
            .any(|ability| {
                ability["status"] == "compiler_only"
                    && ability["reason"]
                        == "The cast pays the creature sacrifice route. Artifact-only sacrifice payment is not implemented."
            })
    );
}

/// Build distinct hand, library, exile, graveyard, and battlefield identities for replay rejection.
fn ox_replay_scenario(source: &CardRow, fodder_count: u32) -> (SimDeck, GameState) {
    let mut rows = vec![
        source.clone(),
        row("Retained hand", "{99}", "Sorcery", ""),
        row("Library bottom", "{99}", "Sorcery", ""),
        row("Library middle one", "{99}", "Sorcery", ""),
        row("Library middle two", "{99}", "Sorcery", ""),
        row("Library top", "{99}", "Sorcery", ""),
        row("Prior exile", "{99}", "Artifact", ""),
    ];
    rows.extend(
        (0..fodder_count)
            .map(|number| row(&format!("Graveyard card {number}"), "{99}", "Sorcery", "")),
    );
    rows.push(row("Existing relic", "{99}", "Artifact", ""));
    let cards = deck(&rows);
    let mut game = state(vec![1], vec![2, 3, 4, 5]);
    game.exile.push(CardIdx(6));
    game.graveyard.push(CardIdx(0));
    game.graveyard.extend((7..7 + fodder_count).map(CardIdx));
    game.graveyard_generations.insert(CardIdx(0), 2);
    game.graveyard_seen.insert(CardIdx(0), 1);
    let mut relic = new_perm_with(41, &cards, CardIdx(7 + fodder_count), 0, true);
    relic.counters.charge = 2;
    game.battlefield.push(relic);
    game.next_uid = 41;
    (cards, game)
}

/// Reject Ox's printed eight-card route even with eight cards and enough normal-cast mana.
pub(super) fn assert_ox_cost_boundary(source: &CardRow) {
    for source in renamed(source) {
        super::card_ast_tests::assert_ox_escape_inert(&source);
        let oracle = parse_oracle_card(&source);
        assert!(
            oracle
                .keywords
                .iter()
                .flat_map(|keyword| &keyword.arguments)
                .any(|argument| matches!(argument, KeywordArgument::EscapeCards(8)))
        );
        for count in [3, 7, 8] {
            for generic in [0, 3] {
                let (cards, mut game) = ox_replay_scenario(&source, count);
                assert!(cards[CardIdx(0)].spell_data.own_escape.is_none());
                let mut pool = ManaPool {
                    fixed: [0, 0, 0, 2, 0],
                    colorless: generic,
                    ..Default::default()
                };
                let before = game.clone();
                let before_pool = pool.clone();
                let mut spent = [0.0];
                let mut sources = Vec::new();
                cast_graveyard_spells(&cards, &mut game, &mut pool, 1, &mut spent, &mut sources);
                assert_rejected_resources(&game, &before, &pool, &before_pool);
                assert_eq!(spent, [0.0]);
                assert!(sources.is_empty());
                assert_eq!(game.graveyard.len(), count as usize + 1);
            }
        }
        assert_ox_hand_cast(&source);
    }
}

/// Pay the normal printed cost without inventing Escape counters or the unsupported entry effect.
fn assert_ox_hand_cast(source: &CardRow) {
    let cards = deck(&[
        source.clone(),
        row("Retained hand", "{99}", "Sorcery", ""),
        row("Undrawn bottom", "{99}", "Sorcery", ""),
        row("Undrawn middle", "{99}", "Sorcery", ""),
        row("Undrawn top", "{99}", "Sorcery", ""),
    ]);
    assert_eq!(cards[CardIdx(0)].min_cost, parse_cost("{3}{R}{R}"));
    for red in [1, 2] {
        let mut game = state(vec![0, 1], vec![2, 3, 4]);
        let mut pool = ManaPool {
            fixed: [0, 0, 0, red, 0],
            colorless: 5 - red,
            ..Default::default()
        };
        let before = game.clone();
        let before_pool = pool.clone();
        let cast = attempt_index(&cards, &mut game, &mut pool, CardIdx(0));
        if red == 1 {
            assert_eq!(cast, None);
            assert_rejected_resources(&game, &before, &pool, &before_pool);
            continue;
        }
        assert_eq!(cast, Some(5));
        assert_eq!(pool.total(), 0);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert_eq!(game.library, [CardIdx(2), CardIdx(3), CardIdx(4)]);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.seen, 0);
        assert_eq!(game.replay_casts, 0);
        assert_eq!(game.spells_cast_this_turn, 1);
        assert_eq!(game.battlefield.len(), 1);
        let ox = &game.battlefield[0];
        assert_eq!(ox.card.deck_idx(), Some(CardIdx(0)));
        assert_eq!(ox.counters.plus1, 0);
        assert_eq!(cards[CardIdx(0)].printed_power, Some(4));
        assert_eq!(cards[CardIdx(0)].printed_toughness, Some(2));
        assert_eq!(current_creature_power(ox, &cards, &game.battlefield), 4);
        assert_eq!(game.life, 20);
        assert_library_rng(&game.library_rng, &before.library_rng);
    }
}

/// Observe a normal Ox cast while reporting its unsupported entry wording as unsupported.
pub(super) fn assert_ox_cost_boundary_report(source: &CardRow) {
    let mut rows = vec![
        source.clone(),
        row("Retained hand", "{99}", "Sorcery", ""),
        row("Library bottom", "{99}", "Sorcery", ""),
        row("Library middle one", "{99}", "Sorcery", ""),
        row("Library middle two", "{99}", "Sorcery", ""),
        row("Library top", "{99}", "Sorcery", ""),
    ];
    rows.extend((0..5).map(|_| {
        row(
            "Mountain source",
            "",
            "Basic Land — Mountain",
            "{T}: Add {R}.",
        )
    }));
    let cards = deck(&rows);
    let mut game = state(vec![0, 1], vec![2, 3, 4, 5]);
    for index in 6..=10 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 20;
    let expected_rng = game.library_rng.clone();
    let census = execute_turns(&cards, &mut game, 1);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(
        game.library,
        [CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(5)]
    );
    assert!(game.graveyard.is_empty() && game.exile.is_empty());
    assert_eq!(game.battlefield.len(), 6);
    let ox = game.battlefield.last().expect("the normal cast entered");
    assert_eq!(ox.card.deck_idx(), Some(CardIdx(0)));
    assert_eq!(ox.uid, 21);
    assert_eq!(ox.counters.plus1, 0);
    assert_eq!(current_creature_power(ox, &cards, &game.battlefield), 4);
    assert_eq!(cards[CardIdx(0)].printed_toughness, Some(2));
    assert_eq!(game.replay_casts, 0);
    assert_eq!(game.seen, 0);
    assert_library_rng(&game.library_rng, &expected_rng);
    let (log, json) = executed_report(&cards, game, census, 1);
    assert_eq!(log.mana_available, [5.0]);
    assert_eq!(log.mana_spent, [5.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(log.cards_seen, [0]);
    assert_eq!(log.creatures, [1]);
    assert_eq!(log.library_size, [4]);
    assert_eq!(log.graveyard_size, [0]);
    assert_eq!(log.replay_casts, 0);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 5.0);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 0.0);
    let support = source_support(&json, source);
    let entry = support["abilities"]
        .as_array()
        .expect("Ox has ability records")
        .iter()
        .find(|ability| {
            ability["source"]
                .as_str()
                .expect("ability retains source text")
                .starts_with("When this creature enters, discard your hand")
        })
        .expect("the entry clause has compiler evidence");
    assert_eq!(entry["status"], "unsupported");
    assert!(
        entry["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
}

/// Place one exact victim beside unrelated physical cards for Dispute's cost boundary.
fn dispute_scenario(source: &CardRow, artifact_creature: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row(
            "Sacrifice candidate",
            "{99}",
            if artifact_creature {
                "Artifact Creature — Construct"
            } else {
                "Artifact"
            },
            "",
        ),
        row("Retained hand", "{99}", "Sorcery", ""),
        row("Undrawn bottom", "{99}", "Sorcery", ""),
        row("Second draw", "{99}", "Sorcery", ""),
        row("First draw", "{99}", "Sorcery", ""),
        row("Prior exile", "{99}", "Sorcery", ""),
        row("Prior graveyard", "{99}", "Sorcery", ""),
        row("Swamp one", "", "Basic Land — Swamp", "{T}: Add {B}."),
        row("Swamp two", "", "Basic Land — Swamp", "{T}: Add {B}."),
    ]);
    let mut game = state(vec![0, 2], vec![3, 4, 5]);
    game.exile.push(CardIdx(6));
    game.graveyard.push(CardIdx(7));
    game.battlefield
        .push(new_perm_with(31, &cards, CardIdx(1), 0, false));
    for index in 8..=9 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 40;
    (cards, game)
}

/// An artifact alone is rejected; an artifact creature pays once through the creature route.
pub(super) fn assert_dispute_cost_boundary(source: &CardRow) {
    for source in renamed(source) {
        for artifact_creature in [false, true] {
            for tapped in [false, true] {
                let (cards, mut game) = dispute_scenario(&source, artifact_creature);
                game.battlefield[0].tapped = tapped;
                let mut pool = build_pool(&cards, &mut game, 1);
                assert_eq!(pool.total(), 2);
                assert_eq!(cards[CardIdx(0)].spell_data.additional_cost_creatures, 1);
                let before = game.clone();
                let before_pool = pool.clone();
                let cast = attempt_index(&cards, &mut game, &mut pool, CardIdx(0));
                if !artifact_creature {
                    assert_eq!(cast, None);
                    assert_rejected_resources(&game, &before, &pool, &before_pool);
                    continue;
                }
                assert_eq!(cast, Some(2));
                assert_eq!(game.graveyard, [CardIdx(7), CardIdx(1), CardIdx(0)]);
                assert_eq!(game.hand, [CardIdx(2), CardIdx(5), CardIdx(4)]);
                assert_eq!(game.library, [CardIdx(3)]);
                assert_eq!(game.exile, [CardIdx(6)]);
                assert_eq!(game.battlefield.len(), 2);
                assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(8)));
                assert_eq!(game.battlefield[1].card.deck_idx(), Some(CardIdx(9)));
                assert_eq!(game.graveyard_generations.get(&CardIdx(1)), Some(&1));
                assert_eq!(game.seen, 2);
                assert_eq!(game.treasure_bank, 1);
                assert_eq!(pool.total(), 0);
                assert_eq!(game.spells_cast_this_turn, 1);
                assert_eq!(game.life, 20);
                assert_library_rng(&game.library_rng, &before.library_rng);
            }
        }
    }
}

/// Actual turns report both the conservative rejection and the single artifact-creature cost.
pub(super) fn assert_dispute_cost_boundary_report(source: &CardRow) {
    for artifact_creature in [false, true] {
        let (cards, mut game) = dispute_scenario(source, artifact_creature);
        let census = execute_turns(&cards, &mut game, 1);
        if artifact_creature {
            assert_eq!(game.graveyard, [CardIdx(7), CardIdx(1), CardIdx(0)]);
            assert_eq!(game.hand, [CardIdx(2), CardIdx(5), CardIdx(4)]);
            assert_eq!(game.library, [CardIdx(3)]);
            assert_eq!(game.treasure_bank, 1);
            assert_eq!(game.battlefield.len(), 2);
        } else {
            assert_eq!(game.hand, [CardIdx(0), CardIdx(2)]);
            assert_eq!(game.library, [CardIdx(3), CardIdx(4), CardIdx(5)]);
            assert_eq!(game.graveyard, [CardIdx(7)]);
            assert_eq!(game.treasure_bank, 0);
            assert_eq!(game.battlefield.len(), 3);
            assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(1)));
        }
        assert_eq!(game.exile, [CardIdx(6)]);
        assert_eq!(game.spells_cast_this_turn, u32::from(artifact_creature));
        let (log, json) = executed_report(&cards, game, census, 1);
        let spent = if artifact_creature { 2.0 } else { 0.0 };
        let draws = if artifact_creature { 2 } else { 0 };
        assert_eq!(log.mana_available, [2.0]);
        assert_eq!(log.mana_spent, [spent]);
        assert_eq!(log.mana_remaining, [2.0 - spent]);
        assert_eq!(log.cards_seen, [draws]);
        assert_eq!(log.graveyard_size, [1 + draws]);
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], spent);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], f64::from(draws));
        assert_dispute_support(&json, source);
    }
}

/// Prepare a genuine Dispute cast with three land sources and five distinct library cards.
fn treasure_scenario(source: &CardRow, targets: &[CardRow]) -> (SimDeck, GameState) {
    let mut rows = vec![
        source.clone(),
        row(
            "Creature payment",
            "{99}",
            "Artifact Creature — Construct",
            "",
        ),
        row("Swamp one", "", "Basic Land — Swamp", "{T}: Add {B}."),
        row("Swamp two", "", "Basic Land — Swamp", "{T}: Add {B}."),
        row("Swamp three", "", "Basic Land — Swamp", "{T}: Add {B}."),
    ];
    rows.extend((0..5).map(|number| row(&format!("Library card {number}"), "{99}", "Sorcery", "")));
    rows.extend_from_slice(targets);
    let cards = deck(&rows);
    let mut hand = vec![0];
    hand.extend(10..10 + targets.len());
    let mut game = state(hand, vec![5, 6, 7, 8, 9]);
    for index in 1..=4 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 40;
    (cards, game)
}

/// Produce the bank through a paid source spell before isolated follow-up action checks.
fn generate_treasure_bank(source: &CardRow, targets: &[CardRow]) -> (SimDeck, GameState, ManaPool) {
    let (cards, mut game) = treasure_scenario(source, targets);
    let mut pool = build_pool(&cards, &mut game, 1);
    assert_eq!(pool.fixed, [0, 0, 3, 0, 0]);
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        Some(2)
    );
    assert_eq!(pool.fixed, [0, 0, 1, 0, 0]);
    assert_eq!(pool.treasures, 0);
    assert_eq!(game.treasure_bank, 1);
    assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(5), CardIdx(6), CardIdx(7)]);
    let mut expected_hand: Vec<_> = (10..cards.cards.len())
        .map(|index| CardIdx(index as u32))
        .collect();
    expected_hand.extend([CardIdx(9), CardIdx(8)]);
    assert_eq!(game.hand, expected_hand);
    assert_eq!(game.battlefield.len(), 3);
    assert_eq!(game.seen, 2);
    assert!(!game.failed_draw);
    // This is the turn loop's bank transfer, not a newly granted mana credit.
    pool.treasures += std::mem::take(&mut game.treasure_bank);
    assert_eq!(pool.total(), 2);
    assert_eq!(pool.mana_total(), 1);
    (cards, game, pool)
}

/// Check source-created bank payments without treating the bank as battlefield artifacts.
pub(super) fn assert_treasure_bank_boundary(source: &CardRow) {
    for source in renamed(source) {
        assert_treasure_payment_boundary(&source);
        assert_treasure_dispute_boundary(&source);
        assert_treasure_artifact_boundary(&source);
    }
}

/// Failed payments keep the source-created bank unit; a colored payment consumes it once.
fn assert_treasure_payment_boundary(source: &CardRow) {
    let targets = [
        row("Too much mana", "{3}", "Sorcery", ""),
        row("True colorless", "{C}", "Sorcery", ""),
        row("Blue payment", "{U}", "Sorcery", ""),
        row("Second token use", "{2}", "Sorcery", ""),
    ];
    let (cards, mut game, mut pool) = generate_treasure_bank(source, &targets);
    for index in [10, 11] {
        let before = game.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(index)),
            None
        );
        assert_rejected_resources(&game, &before, &pool, &before_pool);
    }
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(12)),
        Some(1)
    );
    assert_eq!(pool.fixed, [0, 0, 1, 0, 0]);
    assert_eq!(pool.treasures, 0);
    assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0), CardIdx(12)]);
    assert_eq!(
        game.hand,
        [
            CardIdx(10),
            CardIdx(11),
            CardIdx(13),
            CardIdx(9),
            CardIdx(8)
        ]
    );
    let before = game.clone();
    let before_pool = pool.clone();
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(13)),
        None
    );
    assert_rejected_resources(&game, &before, &pool, &before_pool);
    assert_eq!(game.battlefield.len(), 3);
}

/// A source-created Treasure bank cannot supply Dispute's unsupported artifact sacrifice route.
fn assert_treasure_dispute_boundary(source: &CardRow) {
    let targets = [
        source.clone(),
        row("Fourth swamp", "", "Basic Land — Swamp", "{T}: Add {B}."),
    ];
    let (cards, mut game) = treasure_scenario(source, &targets);
    assert!(play_land(&cards, &mut game, 1));
    let mut pool = build_pool(&cards, &mut game, 1);
    assert_eq!(pool.fixed, [0, 0, 4, 0, 0]);
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        Some(2)
    );
    assert_eq!(game.treasure_bank, 1);
    assert_eq!(pool.fixed, [0, 0, 2, 0, 0]);
    assert_eq!(pool.treasures, 0);
    assert_eq!(game.battlefield.len(), 4);
    assert!(payable(&parse_cost("{1}{B}"), &pool));
    assert!(!additional_costs_payable(&cards, &game, CardIdx(10)));
    let before = game.clone();
    let before_pool = pool.clone();
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(10)),
        None
    );
    assert_rejected_resources(&game, &before, &pool, &before_pool);
    assert_eq!(game.hand, [CardIdx(10), CardIdx(9), CardIdx(8)]);
}

/// The bank cannot count as an artifact and also pay mana for affinity or improvise.
fn assert_treasure_artifact_boundary(source: &CardRow) {
    for keyword in ["Affinity for artifacts", "Improvise"] {
        let targets = [
            row("Artifact payment boundary", "{3}", "Artifact", keyword),
            row("Ordinary payment", "{2}", "Sorcery", ""),
            row("Repeated payment", "{1}", "Sorcery", ""),
        ];
        let (cards, mut game, mut pool) = generate_treasure_bank(source, &targets);
        let card = &cards[CardIdx(10)];
        assert_eq!(
            card.keyword_abilities.affinity,
            keyword == "Affinity for artifacts"
        );
        assert_eq!(card.keyword_abilities.improvise, keyword == "Improvise");
        assert_eq!(artifact_cost_reduction(&cards, card, &game.battlefield), 0);
        let before = game.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(10)),
            None
        );
        assert_rejected_resources(&game, &before, &pool, &before_pool);
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(11)),
            Some(2)
        );
        assert_eq!(pool.total(), 0);
        assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0), CardIdx(11)]);
        assert_eq!(game.battlefield.len(), 3);
        let before = game.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(12)),
            None
        );
        assert_rejected_resources(&game, &before, &pool, &before_pool);
    }
}

/// Full turns retain an unspent source-created bank without reporting it as floated unused mana.
pub(super) fn assert_treasure_bank_report(source: &CardRow) {
    let (cards, mut game) =
        treasure_scenario(source, &[row("Unpayable colorless", "{C}", "Sorcery", "")]);
    let mut census = TurnCensus::new(2, cards.cards.len());
    let commander = CommanderProfile::new(&cards);
    let mut sources = Vec::new();
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut sources,
        1,
        false,
    );
    assert_eq!(game.treasure_bank, 1);
    assert_eq!(game.hand, [CardIdx(10), CardIdx(9), CardIdx(8)]);
    assert_eq!(game.library, [CardIdx(5), CardIdx(6), CardIdx(7)]);
    assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0)]);
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut sources,
        2,
        false,
    );
    assert_eq!(game.treasure_bank, 1);
    assert_eq!(game.hand, [CardIdx(10), CardIdx(9), CardIdx(8), CardIdx(7)]);
    assert_eq!(game.library, [CardIdx(5), CardIdx(6)]);
    assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0)]);
    assert_eq!(game.battlefield.len(), 3);
    assert!(game.battlefield.iter().all(|permanent| {
        let index = permanent
            .card
            .deck_idx()
            .expect("only physical lands remain");
        (2..=4).contains(&index.0) && cards[index].is_land && !cards[index].is_artifact
    }));
    assert!(game.exile.is_empty());
    assert!(!game.failed_draw);
    let (log, json) = executed_report(&cards, game, census, 2);
    assert_eq!(log.mana_available, [3.0, 4.0]);
    assert_eq!(log.mana_spent, [2.0, 0.0]);
    assert_eq!(log.mana_remaining, [1.0, 3.0]);
    assert_eq!(log.cards_seen, [2, 3]);
    assert_eq!(log.library_size, [3, 2]);
    assert_eq!(log.graveyard_size, [2, 2]);
    assert_eq!(json["mana"]["avg_available_by_turn"]["1"], 3.0);
    assert_eq!(json["mana"]["avg_available_by_turn"]["2"], 4.0);
    assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], 1.0);
    assert_eq!(json["mana"]["avg_unused_by_turn"]["2"], 3.0);
    assert_dispute_support(&json, source);
    let assumptions = json["assumptions"]
        .as_array()
        .expect("the report lists limits");
    assert!(assumptions.iter().any(|limit| {
        limit.as_str().is_some_and(|text| {
            text.contains("Treasure effects create a payment bank")
                && text.contains("not battlefield artifacts or floated mana")
                && text.contains("Banked Treasures do not count for artifact totals, artifact payments, or token entry and sacrifice events")
        })
    }));
    assert!(assumptions.iter().any(|limit| {
        limit.as_str().is_some_and(|text| {
            text.contains("Unused mana excludes untapped sources and unspent Treasure tokens")
        })
    }));
}

/// Keep the unsupported replay and the normal hand-cast route under a pinned source test.
#[test]
fn pinned_ox_eight_card_escape_is_inert_but_normal_hand_cost_still_applies() {
    assert_ox_cost_boundary(&named_row("Ox of Agonas"));
}

/// Keep Ox's normal-cast census separate from any claim that its entry effect executes.
#[test]
fn pinned_ox_normal_turn_reports_no_escape_or_entry_draw_reward() {
    assert_ox_cost_boundary_report(&named_row("Ox of Agonas"));
}

/// Keep artifact-only rejection distinct from the accepted artifact-creature payment.
#[test]
fn pinned_dispute_rejects_artifact_only_and_consumes_an_artifact_creature_once() {
    assert_dispute_cost_boundary(&named_row("Deadly Dispute"));
}

/// Preserve the unsupported artifact-only reason alongside actual cost and draw observations.
#[test]
fn pinned_dispute_turn_reports_preserve_the_artifact_only_cost_limit() {
    assert_dispute_cost_boundary_report(&named_row("Deadly Dispute"));
}

/// The bank survives two actual turns while reports separate capacity from unused mana.
#[test]
fn pinned_dispute_treasure_bank_persists_without_inflating_unused_mana() {
    assert_treasure_bank_report(&named_row("Deadly Dispute"));
}

/// Failed attempts preserve the bank, and a successful payment cannot spend its unit twice.
#[test]
fn source_created_treasure_rejects_failed_payments_and_cannot_be_spent_twice() {
    assert_treasure_payment_boundary(&named_row("Deadly Dispute"));
}

/// Enough mana does not make a bank unit an eligible sacrifice artifact.
#[test]
fn source_created_treasure_is_not_a_dispute_sacrifice_artifact() {
    assert_treasure_dispute_boundary(&named_row("Deadly Dispute"));
}

/// Affinity and improvise cannot reuse banked payment capacity as an extra artifact.
#[test]
fn source_created_treasure_cannot_pay_an_extra_affinity_or_improvise_unit() {
    assert_treasure_artifact_boundary(&named_row("Deadly Dispute"));
}

/// Renamed Oracle controls must obey the same bank-only action limits.
#[test]
fn renamed_dispute_rewards_obey_the_same_treasure_bank_limits() {
    assert_treasure_bank_boundary(&named_row("Deadly Dispute"));
}
