//! Synthetic library-core checks. These are not source-bound card certificates.
//! Basic and untyped landcycling stay inert. Only named land-type selectors run.
//! Composite cycling payments, X costs, snow costs, and unknown symbols stay inert.

use super::aggregate::aggregate;
use super::card_test_support::{assert_library_rng, corpus, main_phase_state, pinned_revision};
use super::cast_pass::cast_pass;
use super::game::{GameLog, GameState, ManaPool, new_perm_with};
use super::game_commander::CommanderProfile;
use super::game_run::{TurnCensus, census::finish_log, run_turn};
use super::model::{BasicLandType, CardIdx, DrawMatch, SimDeck, SimEffect, SimTrigger};
use super::oracle_ast::{
    KeywordArgument, OracleAbility, OracleEffect, OracleKeywordName, OracleTriggerEvent,
    PlayerScope,
};
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row, state};
use crate::db::CardRow;
use serde_json::Value;

/// Execute the ordinary cast policy with the supplied physical zones and mana.
pub(super) fn cast_action(cards: &SimDeck, game: &mut GameState, pool: &mut ManaPool) -> f64 {
    let turn = game.current_turn.max(1) as usize;
    let mut spent = vec![0.0; turn];
    cast_pass(
        cards,
        game,
        pool,
        turn,
        &mut spent,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    );
    spent[turn - 1]
}

/// Put distinct controlled sources on the board without granting entry effects.
pub(super) fn put_permanents(cards: &SimDeck, game: &mut GameState, indexes: &[u32]) {
    for &index in indexes {
        game.next_uid += 1;
        game.battlefield.push(new_perm_with(
            game.next_uid,
            cards,
            CardIdx(index),
            0,
            false,
        ));
    }
}

/// Run actual turns and reduce their census into the public production report.
pub(super) fn executed_report(
    cards: &SimDeck,
    mut game: GameState,
    turns: usize,
    mut repeatable: Vec<(u32, u32)>,
) -> (GameState, GameLog, Value) {
    let mut census = TurnCensus::new(turns, cards.cards.len());
    for turn in 1..=turns {
        run_turn(
            cards,
            &mut game,
            &mut census,
            &CommanderProfile::new(cards),
            &mut repeatable,
            turn,
            false,
        );
    }
    let config = game.opponents.config.clone();
    let log = finish_log(cards, game.clone(), census, 0, false, turns);
    let stats = aggregate(std::slice::from_ref(&log), cards, turns as u32);
    let mut report = super::report::json_report(
        &stats,
        cards,
        "Library clause scenario",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    super::report::set_opponent_model(&mut report, cards.format, config, &stats);
    let json = serde_json::to_value(report).expect("production report serializes");
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    assert_eq!(log.turns_observed, turns as u32);
    for card in &cards.cards {
        let record = json["card_support"]
            .as_array()
            .expect("compiler records")
            .iter()
            .find(|record| record["name"] == card.name)
            .expect("card remains in the report");
        assert_eq!(record["source_hash"], card.source_hash);
        let abilities = record["abilities"].as_array().expect("ability records");
        assert_eq!(abilities.len(), card.support.len());
        for (actual, expected) in abilities.iter().zip(&card.support) {
            assert_eq!(actual["source"], expected.source);
            assert_eq!(
                actual["identity"],
                format!("ability:{}", expected.ability_index)
            );
            assert_eq!(
                actual["span_start"],
                serde_json::json!(expected.span.map(|span| span.start))
            );
            assert_eq!(
                actual["span_end"],
                serde_json::json!(expected.span.map(|span| span.end))
            );
            let status = serde_json::to_value(&expected.status).expect("status serializes");
            for key in ["status", "reason", "rule_id"] {
                assert_eq!(actual[key], status[key]);
            }
        }
    }
    (game, log, json)
}

/// Require a pinned source revision and one exact public compiler disposition.
pub(super) fn assert_report_clause(
    report: &Value,
    source: &CardRow,
    wording: &str,
    status: &str,
    rule: Option<&str>,
) {
    let revision = pinned_revision(source);
    let card = report["card_support"]
        .as_array()
        .expect("card support")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("pinned source remains in report");
    assert_eq!(card["source_hash"], revision["flattened_input_sha256"]);
    let ability = card["abilities"]
        .as_array()
        .expect("ability support")
        .iter()
        .find(|ability| {
            ability["source"]
                .as_str()
                .is_some_and(|text| text.contains(wording))
        })
        .expect("report preserves the claimed Oracle wording");
    assert_eq!(ability["status"], status);
    if let Some(rule) = rule {
        assert_eq!(ability["rule_id"], rule);
    }
}

/// Synthetic cycling pays real mana and discards exactly its own physical card.
pub(super) fn assert_synthetic_cycling_report() {
    let cards = deck(&[
        row("Synthetic mana cycler", "{99}", "Creature", "Cycling {1}"),
        row("Synthetic draw", "{99}", "Sorcery", ""),
        row("Synthetic remaining", "{99}", "Sorcery", ""),
        row(
            "Cycling payment",
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        ),
    ]);
    let mut rejected = state(vec![0], vec![2, 1]);
    let rng = rejected.library_rng.clone();
    assert_eq!(
        cast_action(&cards, &mut rejected, &mut ManaPool::default()),
        0.0
    );
    assert_eq!(rejected.hand, [CardIdx(0)]);
    assert_eq!(rejected.library, [CardIdx(2), CardIdx(1)]);
    assert!(rejected.graveyard.is_empty());
    assert_eq!(rejected.seen, 0);
    assert_library_rng(&rejected.library_rng, &rng);

    let mut game = state(vec![0], vec![2, 1]);
    put_permanents(&cards, &mut game, &[3]);
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(game.life, 20);
    assert_eq!(log.card_first_seen.get(&1), Some(&1));
    assert_eq!(log.card_first_graveyard.get(&0), Some(&1));
    assert_eq!(log.cards_seen, [1]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 1.0);
    assert_eq!(json["velocity"]["library_remaining_by_turn"]["1"], 1.0);
    assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
    assert_library_rng(&game.library_rng, &rng);
}

/// Synthetic landcycling rejects non-Forests and shuffles after a failed search too.
pub(super) fn assert_synthetic_landcycling_report() {
    for matched in [false, true] {
        let cards = deck(&[
            row(
                "Synthetic Forest cycler",
                "{99}",
                "Creature",
                "Forestcycling {1} ({1}, Discard this card: Search your library for a Forest card, reveal it, put it into your hand, then shuffle.)",
            ),
            row(
                "Typed target",
                "",
                if matched {
                    "Land - Forest"
                } else {
                    "Land - Island"
                },
                "",
            ),
            row("Forest name without type", "{99}", "Artifact", ""),
            row("Wrong land", "", "Land - Swamp", ""),
            row(
                "Cycling payment",
                "",
                "Basic Land - Wastes",
                "{T}: Add {C}.",
            ),
        ]);
        let mut rejected = state(vec![0], vec![1, 2, 3]);
        let rng = rejected.library_rng.clone();
        cast_action(&cards, &mut rejected, &mut ManaPool::default());
        assert_eq!(rejected.hand, [CardIdx(0)]);
        assert!(rejected.graveyard.is_empty());
        assert_eq!(rejected.library, [CardIdx(1), CardIdx(2), CardIdx(3)]);
        assert_library_rng(&rejected.library_rng, &rng);

        let mut game = state(vec![0], vec![1, 2, 3]);
        put_permanents(&cards, &mut game, &[4]);
        let mut expected_rng = game.library_rng.clone();
        let mut expected = if matched {
            vec![CardIdx(2), CardIdx(3)]
        } else {
            game.library.clone()
        };
        super::rng::shuffle(&mut expected, &mut expected_rng);
        assert_eq!(expected_rng.get_word_pos(), if matched { 2 } else { 4 });
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.library, expected);
        assert!(game.hand.is_empty(), "a found land uses the late land drop");
        assert_eq!(log.card_first_seen.get(&1), matched.then_some(&1));
        assert_eq!(log.land_drops, [u8::from(matched)]);
        assert_eq!(
            log.cards_seen,
            [u32::from(matched)],
            "a selected library card counts as exposure"
        );
        assert_eq!(
            game.awareness_cards, 0,
            "full-library searches do not add scry awareness"
        );
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            f64::from(u32::from(matched))
        );
        assert_eq!(json["velocity"]["library_awareness_by_turn"]["1"], 0.0);
        assert_eq!(
            json["velocity"]["library_remaining_by_turn"]["1"],
            if matched { 2.0 } else { 3.0 }
        );
        assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 1.0);
        assert_library_rng(&game.library_rng, &expected_rng);
    }
}

/// Synthetic life cycling rejects an unpaid life cost and reports the paid draw separately.
pub(super) fn assert_synthetic_life_cycling_report() {
    let cards = deck(&[
        row(
            "Synthetic life cycler",
            "{99}",
            "Creature",
            "Cycling—Pay 2 life. (Pay 2 life, Discard this card: Draw a card.)",
        ),
        row("Life cycling draw", "{99}", "Sorcery", ""),
        row("Life cycling remaining card", "{99}", "Sorcery", ""),
    ]);
    let mut rejected = state(vec![0], vec![2, 1]);
    rejected.life = 1;
    let rng = rejected.library_rng.clone();
    cast_action(&cards, &mut rejected, &mut ManaPool::default());
    assert_eq!(rejected.life, 1);
    assert_eq!(rejected.life_paid, 0);
    assert_eq!(rejected.life_funded_draws, 0);
    assert_eq!(rejected.hand, [CardIdx(0)]);
    assert_eq!(rejected.library, [CardIdx(2), CardIdx(1)]);
    assert!(rejected.graveyard.is_empty());
    assert_library_rng(&rejected.library_rng, &rng);
    let mut game = state(vec![0], vec![2, 1]);
    game.life = 3;
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.life, 1);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(log.life_paid, 2);
    assert_eq!(log.life_funded_draws, 1);
    assert_eq!(json["draw"]["avg_life_paid"], 2.0);
    assert_eq!(json["draw"]["avg_life_funded_draws"], 1.0);
    assert_eq!(
        json["milestones"]["life_funded_draws_avg_by_turn"]["1"],
        1.0
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Separate inert board-count wording from synthetic runtime scale checks at zero, one, and nine.
pub(super) fn assert_synthetic_board_draw_report() {
    let unlowered = row(
        "Synthetic board draw",
        "{99}",
        "Enchantment",
        "At the beginning of your upkeep, draw a card for each artifact you control.",
    );
    let oracle = parse_oracle_card(&unlowered);
    assert_eq!(
        oracle.static_data.draw_per_controlled_permanent,
        Some(DrawMatch::Artifacts)
    );
    assert!(
        matches!(oracle.abilities.as_slice(), [OracleAbility::Triggered(trigger)]
        if trigger.event == OracleTriggerEvent::BeginningOfUpkeep(PlayerScope::You)
            && matches!(trigger.effects.as_slice(), [OracleEffect::Unsupported(text)] if text == "draw a card for each artifact you control"))
    );
    for count in [0u32, 1, 9] {
        let mut rows = board_draw_rows(unlowered.clone());
        assert_unlowered_board_draw_report(&rows, count);
        rows[0] = row(
            "Synthetic configured scale",
            "{99}",
            "Enchantment",
            "At the beginning of your upkeep, draw a card.",
        );
        let mut cards = deck(&rows);
        assert!(
            cards.cards[0]
                .unlocked_abilities(0)
                .any(|ability| ability.trigger == SimTrigger::Upkeep
                    && matches!(ability.effect_sequence(), [SimEffect::Draw(1)]))
        );
        // Only the scale is configured. This tests runtime data, not lowering of the rejected sentence.
        cards.cards[0].draws_per_matching = oracle.static_data.draw_per_controlled_permanent;
        let mut game = state(vec![], (13..25).collect());
        put_permanents(&cards, &mut game, &[0, 10, 11, 12]);
        put_permanents(&cards, &mut game, &(1..=count).collect::<Vec<_>>());
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![(1, 1)]);
        assert_eq!(game.seen, count);
        assert_eq!(
            game.library,
            (13..25 - count).map(CardIdx).collect::<Vec<_>>()
        );
        assert_eq!(log.cards_seen, [count]);
        assert_eq!(log.library_size, [12 - count]);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], f64::from(count));
        assert_eq!(
            json["velocity"]["library_remaining_by_turn"]["1"],
            f64::from(12 - count)
        );
        assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
        assert_eq!(game.hand.len() + game.graveyard.len(), count as usize);
        assert_eq!(
            game.hand,
            (25 - count.min(7)..25)
                .rev()
                .map(CardIdx)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            game.graveyard,
            if count == 9 {
                vec![CardIdx(16), CardIdx(17)]
            } else {
                vec![]
            }
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Give board-count controls matching artifacts, nonmatching creatures, and a fixed library.
fn board_draw_rows(source: CardRow) -> Vec<CardRow> {
    let mut rows = vec![source];
    rows.extend((0..9).map(|index| {
        row(
            &format!("Matching artifact {index}"),
            "{99}",
            "Artifact",
            "",
        )
    }));
    rows.extend((0..3).map(|index| {
        row(
            &format!("Nonmatching creature {index}"),
            "{99}",
            "Creature",
            "",
        )
    }));
    rows.extend((0..12).map(|index| row(&format!("Library card {index}"), "{99}", "Sorcery", "")));
    rows
}

/// Recognition of a scale does not authorize a draw whose complete wording cannot lower.
fn assert_unlowered_board_draw_report(rows: &[CardRow], count: u32) {
    let cards = deck(rows);
    assert_eq!(
        cards.cards[0].draws_per_matching,
        Some(DrawMatch::Artifacts)
    );
    assert!(
        !cards.cards[0]
            .unlocked_abilities(0)
            .any(|ability| ability.trigger == SimTrigger::Upkeep)
    );
    let mut game = state(vec![], (13..25).collect());
    put_permanents(&cards, &mut game, &[0, 10, 11, 12]);
    put_permanents(&cards, &mut game, &(1..=count).collect::<Vec<_>>());
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(&cards, game, 1, vec![(1, 1)]);
    assert!(game.hand.is_empty());
    assert!(game.graveyard.is_empty());
    assert_eq!(game.library, (13..25).map(CardIdx).collect::<Vec<_>>());
    assert_eq!(log.cards_seen, [0]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 0.0);
    let record = json["card_support"]
        .as_array()
        .expect("compiler records")
        .iter()
        .find(|record| record["name"] == rows[0].name)
        .expect("inert source record");
    assert_eq!(record["abilities"][0]["status"], "unsupported");
    assert_library_rng(&game.library_rng, &rng);
}

/// Unsupported cycling selectors and incomplete costs preserve every physical resource.
pub(super) fn assert_synthetic_cycling_admission_safety_report() {
    for (text, kind) in [
        ("Basic landcycling {1}", OracleKeywordName::BasicLandcycling),
        ("Landcycling {1}", OracleKeywordName::Landcycling),
        (
            "Basic landcycling—Pay 2 life",
            OracleKeywordName::BasicLandcycling,
        ),
        (
            "Cycling {1}, Sacrifice a creature",
            OracleKeywordName::Cycling,
        ),
        ("Cycling {1} and discard a card", OracleKeywordName::Cycling),
        ("Cycling {1}, Pay 2 life", OracleKeywordName::Cycling),
        (
            "Cycling—Pay 2 life and discard a card",
            OracleKeywordName::Cycling,
        ),
        ("Cycling {1}{Q}", OracleKeywordName::Cycling),
        ("Cycling {X}", OracleKeywordName::Cycling),
        ("Forestcycling {S}", OracleKeywordName::Landcycling),
        (
            "Forestcycling {1}, Pay 2 life",
            OracleKeywordName::Landcycling,
        ),
    ] {
        assert_rejected_cycling_route(text, kind);
    }
}

/// Carry one unsupported keyword from its retained parser arguments into a real turn and report.
fn assert_rejected_cycling_route(text: &str, kind: OracleKeywordName) {
    let input = row("Synthetic rejected cycler", "{99}", "Creature", text);
    let oracle = parse_oracle_card(&input);
    let keyword = oracle
        .keywords
        .iter()
        .find(|keyword| keyword.name == kind)
        .expect("keyword identity is retained");
    assert!(
        !keyword.arguments.is_empty(),
        "required payment remains visible: {text}"
    );
    if text.contains("Sacrifice") || text.contains("discard a card") || text.contains(", Pay") {
        assert!(keyword.arguments.iter().any(|argument| matches!(argument, KeywordArgument::Other(payment) if !payment.is_empty())), "unknown complete payment is not reduced to its mana prefix: {text}");
    }
    let cards = deck(&[
        input,
        row("Rejected search target", "", "Land — Forest", ""),
        row("Rejected draw target", "{99}", "Artifact", ""),
        row("Rejected route payment", "", "Land", "{T}: Add {C}."),
    ]);
    assert!(cards.cards[0].spell_data.cycling_cost.is_none(), "{text}");
    assert_eq!(cards.cards[0].spell_data.cycling_life, 0, "{text}");
    assert!(
        cards.cards[0].spell_data.landcycling_type.is_none(),
        "{text}"
    );
    let mut action = main_phase_state(vec![0], vec![1, 2]);
    let action_rng = action.library_rng.clone();
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    assert_eq!(cast_action(&cards, &mut action, &mut pool), 0.0);
    assert_eq!(pool.colorless, 1);
    assert_eq!(action.hand, [CardIdx(0)]);
    assert_eq!(action.library, [CardIdx(1), CardIdx(2)]);
    assert!(action.graveyard.is_empty());
    assert_eq!(action.life, 20);
    assert_eq!(action.life_paid, 0);
    assert_library_rng(&action.library_rng, &action_rng);
    let mut game = state(vec![0], vec![1, 2]);
    put_permanents(&cards, &mut game, &[3]);
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.hand, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(1), CardIdx(2)]);
    assert!(game.graveyard.is_empty());
    assert!(game.exile.is_empty());
    assert_eq!(log.cards_seen, [0]);
    assert_eq!(log.mana_available, [1.0]);
    assert_eq!(log.mana_remaining, [1.0]);
    assert_eq!(log.life_paid, 0);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 0.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 0.0);
    assert_library_rng(&game.library_rng, &action_rng);
}

/// Every named landcycling selector keeps its complete mana cost and finds a nonbasic matching land.
pub(super) fn assert_synthetic_named_landcycling_controls_report() {
    for (name, kind) in [
        ("Plains", BasicLandType::Plains),
        ("Island", BasicLandType::Island),
        ("Swamp", BasicLandType::Swamp),
        ("Mountain", BasicLandType::Mountain),
        ("Forest", BasicLandType::Forest),
    ] {
        let mut input = row(
            "Synthetic named landcycler",
            "{99}",
            "Creature",
            &format!("{name}cycling {{1}}"),
        );
        input.keywords = serde_json::to_string(&[format!("{name}cycling")])
            .expect("keyword metadata serializes");
        let oracle = parse_oracle_card(&input);
        assert_eq!(oracle.basic_land_type(), Some(kind));
        let cards = deck(&[
            input,
            row("Named land target", "", &format!("Land — {name}"), ""),
            row("Named search remainder", "{99}", "Artifact", ""),
            row("Named search payment", "", "Land", "{T}: Add {C}."),
        ]);
        assert_eq!(cards.cards[0].spell_data.landcycling_type, Some(kind));
        assert_eq!(
            cards.cards[0]
                .spell_data
                .cycling_cost
                .expect("complete named cycling cost")
                .total(),
            1
        );
        let mut game = state(vec![0], vec![1, 2]);
        put_permanents(&cards, &mut game, &[3]);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert!(game.hand.is_empty());
        assert_eq!(game.library, [CardIdx(2)]);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(log.card_first_seen.get(&1), Some(&1));
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.cards_seen, [1]);
        assert_eq!(log.mana_remaining, [0.0]);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 1.0);
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// No historical source supplies cycling; synthetic fixtures cannot close a whole-card claim.
#[test]
fn synthetic_library_core_reports_have_no_pinned_cycling_source() {
    for card in corpus()["cards"]
        .as_array()
        .expect("historical source cards")
    {
        let source = super::card_test_support::metadata_row(&card["metadata"]);
        assert!(!source.oracle_text.to_ascii_lowercase().contains("cycling"));
    }
}

/// Exercise the synthetic cycling branch through the real turn and report pipeline.
#[test]
fn synthetic_cycling_executes_and_reports() {
    assert_synthetic_cycling_report();
}

/// Exercise synthetic life costs without implying a source-bound cycling certificate.
#[test]
fn synthetic_life_cycling_executes_and_reports() {
    assert_synthetic_life_cycling_report();
}

/// Exercise successful and failed synthetic landcycling searches.
#[test]
fn synthetic_landcycling_executes_and_reports() {
    assert_synthetic_landcycling_report();
}

/// Rejected keyword routes cannot become free draws or searches.
#[test]
fn synthetic_cycling_admission_guards_execute_and_report() {
    assert_synthetic_cycling_admission_safety_report();
}

/// All five named landcycling forms keep their supported route.
#[test]
fn synthetic_named_landcycling_controls_execute_and_report() {
    assert_synthetic_named_landcycling_controls_report();
}

/// Exercise synthetic board counts without substituting a pinned source.
#[test]
fn synthetic_board_scaled_draw_executes_and_reports() {
    assert_synthetic_board_draw_report();
}
