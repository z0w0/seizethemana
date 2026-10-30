//! Card report evidence starts with an executed turn, not a hand-built game log.

use crate::db::CardRow;
use crate::deck::simulator::aggregate::aggregate;
use crate::deck::simulator::game::{GameLog, GameState, new_perm_with};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_run::{TurnCensus, census::finish_log, run_turn};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use serde_json::Value;

/// Execute the production turn pipeline without inventing census observations.
pub(super) fn execute_turn(cards: &SimDeck, game: &mut GameState) -> TurnCensus {
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

/// Build both report layers from a production census and its actual final game state.
pub(super) fn report_executed_turn(
    cards: &SimDeck,
    game: GameState,
    census: TurnCensus,
) -> (GameLog, Value) {
    let config = game.opponents.config.clone();
    let log = finish_log(cards, game, census, 0, false, 1);
    let stats = aggregate(std::slice::from_ref(&log), cards, 1);
    let mut report = crate::deck::simulator::report::json_report(
        &stats,
        cards,
        "Pinned source scenario",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    crate::deck::simulator::report::set_opponent_model(&mut report, cards.format, config, &stats);
    let json = serde_json::to_value(report).expect("production report serializes");
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(log.turns_observed, 1);
    assert_report_compiler_records(cards, &json);
    (log, json)
}

/// Public compiler records preserve exact source hashes, wording, gaps, and approximation IDs.
fn assert_report_compiler_records(cards: &SimDeck, report: &Value) {
    let records = report["card_support"]
        .as_array()
        .expect("card compiler records");
    for card in cards.cards.iter().chain(&cards.commanders) {
        let record = records
            .iter()
            .find(|record| record["name"] == card.name)
            .expect("compiled card remains in report");
        assert_eq!(
            record["source_hash"],
            if card.source_hash.is_empty() {
                Value::Null
            } else {
                Value::String(card.source_hash.clone())
            }
        );
        assert!(
            record["note"]
                .as_str()
                .expect("compiler evidence note")
                .contains("Compiler evidence only")
        );
        let abilities = record["abilities"]
            .as_array()
            .expect("ability compiler records");
        assert_eq!(abilities.len(), card.support.len());
        for (actual, expected) in abilities.iter().zip(&card.support) {
            assert_eq!(
                actual["identity"],
                format!("ability:{}", expected.ability_index)
            );
            assert_eq!(actual["source"], expected.source);
            assert_eq!(
                actual["span_start"],
                serde_json::json!(expected.span.map(|span| span.start))
            );
            assert_eq!(
                actual["span_end"],
                serde_json::json!(expected.span.map(|span| span.end))
            );
            let status =
                serde_json::to_value(&expected.status).expect("compiler status serializes");
            for key in ["status", "reason", "rule_id"] {
                assert_eq!(actual[key], status[key], "{}: {key}", card.name);
            }
        }
    }
}

/// Sol Ring's executed casts use two real colorless mana and reject a green pip.
pub(super) fn assert_sol_ring_report(source: &CardRow) {
    assert_mana_report(source, "{2}", "{G}", 2, false);
}

/// Llanowar Elves funds a green cast but does not fund white.
pub(super) fn assert_llanowar_report(source: &CardRow) {
    assert_mana_report(source, "{G}", "{W}", 1, false);
}

/// Birds of Paradise funds blue but not an explicit colorless pip.
pub(super) fn assert_birds_report(source: &CardRow) {
    assert_mana_report(source, "{U}", "{C}", 1, false);
}

/// Only Delighted Halfling's colorless mode contributes to this verified payment scenario.
pub(super) fn assert_halfling_report(source: &CardRow) {
    assert_mana_report(source, "{C}", "{G}", 1, false);
    let cards = deck(&[
        source.clone(),
        row("Colored legendary spell", "{G}", "Legendary Creature", ""),
    ]);
    let mut game = state(vec![1], vec![]);
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert!(game.graveyard.is_empty());
    let (log, json) = report_executed_turn(&cards, game, census);
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 0.0);
    let record = json["card_support"]
        .as_array()
        .expect("card support")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("Halfling record");
    assert_eq!(
        record["source_hash"],
        crate::deck::simulator::support::card_source_hash(source)
    );
    assert_eq!(record["abilities"][0]["status"], "supported");
    assert_eq!(record["abilities"][1]["status"], "unsupported");
    assert!(
        !record["abilities"][1]["reason"]
            .as_str()
            .expect("required-mode gap")
            .is_empty()
    );
}

/// Esper's unsupported printed payment remains visible without a free reported draw.
pub(super) fn assert_esper_report(source: &CardRow) {
    for source in super::card_runtime_tests::renamed(source) {
        for fixed_tax in [false, true] {
            let mut input = source.clone();
            if fixed_tax {
                input.oracle_text = input
                    .oracle_text
                    .replace("{X}, where X is this creature's power", "{1}");
            }
            let (cards, mut game) =
                super::card_runtime_tests::engines::opponent_scenario(&input, 1);
            crate::deck::simulator::opponents::run_opponent_round(&cards, &mut game, 1);
            let census = execute_turn(&cards, &mut game);
            assert_eq!(game.hand.len(), usize::from(fixed_tax));
            assert_eq!(game.library.len(), 2 - usize::from(fixed_tax));
            assert_eq!(game.life, 20);
            assert!(game.graveyard.is_empty() && game.exile.is_empty());
            let (log, json) = report_executed_turn(&cards, game, census);
            assert_eq!(log.cards_seen, [u32::from(fixed_tax)]);
            assert_eq!(log.opponent_activity.casts, 1);
            assert_eq!(log.opponent_activity.payments, 0);
            assert_eq!(
                json["velocity"]["avg_cards_seen"]["1"],
                f64::from(u32::from(fixed_tax))
            );
            let record = json["card_support"]
                .as_array()
                .expect("card support")
                .iter()
                .find(|card| card["name"] == input.name)
                .expect("Esper record");
            assert_eq!(
                record["source_hash"],
                crate::deck::simulator::support::card_source_hash(&input)
            );
            assert_eq!(record["abilities"][0]["source"], input.oracle_text);
            if fixed_tax {
                assert_eq!(record["abilities"][0]["status"], "approximated");
                assert_eq!(
                    record["abilities"][0]["rule_id"],
                    "synthetic_opponent_events"
                );
            } else {
                assert_eq!(record["abilities"][0]["status"], "unsupported");
                assert!(
                    !record["abilities"][0]["reason"]
                        .as_str()
                        .expect("required tax gap")
                        .is_empty()
                );
            }
        }
    }
}

/// Declared blue mana needs an opponent land before it can fund a cast or reward.
pub(super) fn assert_fellwar_report(source: &CardRow) {
    assert_mana_report(source, "{U}", "{G}", 1, true);
    for lands in [0, 1] {
        let legal = lands > 0;
        let cards = deck(&[
            source.clone(),
            row("Declared-color payoff", "{U}", "Sorcery", "Gain 1 life."),
        ]);
        let mut game = state(vec![1], vec![]);
        game.opponents.config.land_colors = Some(vec!["U".into()]);
        for seat in &mut game.opponents.seats {
            seat.active = false;
            seat.lands = 0;
        }
        game.opponents.seats[0].active = true;
        game.opponents.seats[0].lands = lands;
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].uid, 7);
        assert_eq!(game.battlefield[0].tapped, legal);
        assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(1)] });
        assert_eq!(
            game.graveyard,
            if legal { vec![CardIdx(1)] } else { vec![] }
        );
        assert!(game.library.is_empty() && game.exile.is_empty());
        assert_eq!(game.spells_cast_this_turn, u32::from(legal));
        assert_eq!(game.life, 20 + i32::from(legal));
        assert_eq!(game.life_gained, u32::from(legal));
        let (log, json) = report_executed_turn(&cards, game, census);
        let amount = f64::from(legal);
        assert_eq!(log.mana_available, [amount]);
        assert_eq!(log.mana_spent, [amount]);
        assert_eq!(log.mana_remaining, [0.0]);
        assert_eq!(json["mana"]["avg_available_by_turn"]["1"], amount);
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], amount);
        assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], 0.0);
        assert_eq!(json["draw"]["avg_life_gained"], amount);
        assert_eq!(json["outcomes"]["finished_games"], 1);
    }
}

/// Check spend and unused-mana observations after both a payable and a color-near-miss turn.
fn assert_mana_report(
    source: &CardRow,
    payable: &str,
    rejected: &str,
    amount: u32,
    opponent_blue: bool,
) {
    for (cost, legal) in [(payable, true), (rejected, false)] {
        let cards = deck(&[source.clone(), row("Payment target", cost, "Sorcery", "")]);
        let mut game = state(vec![1], vec![]);
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        if opponent_blue {
            game.opponents.config.land_colors = Some(vec!["U".into()]);
            game.opponents.seats[0].lands = 1;
        }
        let census = execute_turn(&cards, &mut game);
        assert!(game.battlefield[0].tapped);
        assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(1)] });
        assert_eq!(
            game.graveyard,
            if legal { vec![CardIdx(1)] } else { vec![] }
        );
        assert_eq!(game.life, 20);
        let (log, json) = report_executed_turn(&cards, game, census);
        let spent = if legal { amount } else { 0 };
        let unused = if legal { 0 } else { amount };
        assert_eq!(log.mana_spent, [f64::from(spent)]);
        assert_eq!(log.mana_remaining, [f64::from(unused)]);
        assert_eq!(
            json["mana"]["avg_cast_spent_by_turn"]["1"],
            f64::from(spent)
        );
        assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], f64::from(unused));
    }
}

/// Dark Ritual's mana funds the next actual spell in the same turn.
pub(super) fn assert_ritual_report(source: &CardRow) {
    let cards = deck(&[
        source.clone(),
        row("Follow-up", "{B}{B}{B}", "Sorcery", ""),
        row("Swamp", "", "Basic Land — Swamp", "{T}: Add {B}."),
    ]);
    let mut game = state(vec![0, 1], vec![]);
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(2), 0, false));
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.graveyard, [CardIdx(0), CardIdx(1)]);
    assert!(game.hand.is_empty());
    let (log, json) = report_executed_turn(&cards, game, census);
    assert_eq!(log.mana_spent, [4.0]);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 4.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 2.0);
}

/// A land search changes battlefield access without consuming a land-drop allowance.
pub(super) fn assert_land_search_report(source: &CardRow) {
    let farseek = source.name == "Farseek";
    let target_type = if farseek { "Mountain" } else { "Forest" };
    let cards = deck(&[
        source.clone(),
        row("Search result", "", &format!("Land — {target_type}"), ""),
        row("Mana one", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Mana two", "", "Basic Land — Forest", "{T}: Add {G}."),
    ]);
    let mut game = state(vec![0], vec![1]);
    for index in 2..=3 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 8;
    let census = execute_turn(&cards, &mut game);
    let found = game
        .battlefield
        .iter()
        .find(|permanent| permanent.card.deck_idx() == Some(CardIdx(1)))
        .expect("search result entered");
    assert_eq!(found.tapped, farseek);
    assert!(game.hand.is_empty() && game.library.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0)]);
    let (log, json) = report_executed_turn(&cards, game, census);
    assert_eq!(log.land_drops, [0]);
    assert_eq!(log.mana_spent, [2.0]);
    assert_eq!(log.library_size, [0]);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 2.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 1.0);
}

/// Top tutors leave their result on top instead of reporting hand access.
pub(super) fn assert_top_tutor_report(source: &CardRow) {
    let vampiric = source.name == "Vampiric Tutor";
    use super::card_runtime_tests::search::{TopTutorScenario, top_tutor_scenario};
    for has_target in [true, false] {
        let TopTutorScenario {
            cards,
            mut game,
            selected,
            expected_library,
            expected_exile,
            expected_rng,
        } = top_tutor_scenario(source, vampiric, has_target);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.library, expected_library);
        if let Some(selected) = selected {
            assert_eq!(game.library.last(), Some(&selected));
        }
        super::card_test_support::assert_library_rng(&game.library_rng, &expected_rng);
        assert!(game.hand.is_empty());
        assert_eq!(game.exile, expected_exile);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(6)));
        assert!(game.battlefield[0].tapped);
        for index in 1..=5 {
            assert!(!game.card_first_seen.contains_key(&index));
        }
        assert_eq!(game.life, if vampiric { 18 } else { 20 });
        assert_eq!(game.life_gained, 0);
        assert_eq!(game.treasure_bank, 0);
        assert!(!game.failed_draw);
        let expected_size =
            u32::try_from(expected_library.len()).expect("scenario library fits census");
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_eq!(log.library_size, [expected_size]);
        assert_eq!(log.life_paid, 0);
        assert_eq!(log.life_gained, 0);
        assert_eq!(log.cards_seen, [u32::from(has_target)]);
        assert_eq!(json["draw"]["avg_life_paid"], 0.0);
        assert_eq!(json["draw"]["avg_life_gained"], 0.0);
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            f64::from(u32::from(has_target))
        );
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 1.0);
        assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 1.0);
    }
}

/// Trophy Mage's actual entry contributes one seen card after its full cast payment.
pub(super) fn assert_trophy_report(source: &CardRow) {
    let mut rows = vec![source.clone(), row("Exact artifact", "{3}", "Artifact", "")];
    rows.extend((0..3).map(|_| row("Island source", "", "Basic Land — Island", "{T}: Add {U}.")));
    let cards = deck(&rows);
    let mut game = state(vec![0], vec![1]);
    for index in 2..=4 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 8;
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert!(game.library.is_empty() && game.graveyard.is_empty());
    let (log, json) = report_executed_turn(&cards, game, census);
    assert_eq!(log.cards_seen, [1]);
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 1.0);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 3.0);
}

/// Beast Whisperer's draw reaches reports only after a matching creature was cast.
pub(super) fn assert_beast_report(source: &CardRow) {
    for (kind, matching) in [("Creature", true), ("Sorcery", false)] {
        let cards = deck(&[
            source.clone(),
            row("Cast object", "{0}", kind, ""),
            row("Drawn", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![1], vec![2]);
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.hand, if matching { vec![CardIdx(2)] } else { vec![] });
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_eq!(log.cards_seen, [u32::from(matching)]);
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            f64::from(u32::from(matching))
        );
    }
}

/// Sacrifice rewards report actual draws; Deadly Dispute's Treasure funds one further cast.
pub(super) fn assert_sacrifice_report(source: &CardRow) {
    let dispute = source.name == "Deadly Dispute";
    let mana_count = 1 + u32::from(dispute);
    let mut rows = vec![
        source.clone(),
        row("Sacrifice payment", "{0}", "Creature", ""),
    ];
    rows.extend(
        (0..mana_count).map(|_| row("Swamp source", "", "Basic Land — Swamp", "{T}: Add {B}.")),
    );
    rows.extend((0..2).map(|_| row("Uncast draw", "{99}", "Sorcery", "")));
    rows.push(row(
        "Drawn follow-up",
        if dispute { "{1}" } else { "{99}" },
        "Sorcery",
        "",
    ));
    let cards = deck(&rows);
    let first_draw = 2 + mana_count;
    let mut game = state(
        vec![0],
        vec![
            first_draw as usize,
            first_draw as usize + 1,
            first_draw as usize + 2,
        ],
    );
    for index in 1..2 + mana_count {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 8;
    let census = execute_turn(&cards, &mut game);
    assert_eq!(
        game.hand,
        if dispute {
            vec![CardIdx(first_draw + 1)]
        } else {
            vec![CardIdx(first_draw + 2), CardIdx(first_draw + 1)]
        }
    );
    assert_eq!(game.library, [CardIdx(first_draw)]);
    let mut expected_graveyard = vec![CardIdx(1), CardIdx(0)];
    if dispute {
        expected_graveyard.push(CardIdx(first_draw + 2));
    }
    assert_eq!(game.graveyard, expected_graveyard);
    assert_eq!(game.treasure_bank, 0);
    let (log, json) = report_executed_turn(&cards, game, census);
    let graveyard_size = 2 + u32::from(dispute);
    let mana_spent = mana_count + u32::from(dispute);
    assert_eq!(log.cards_seen, [2]);
    assert_eq!(log.graveyard_size, [graveyard_size]);
    assert_eq!(log.mana_spent, [f64::from(mana_spent)]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 2.0);
    assert_eq!(
        json["graveyard"]["avg_size_by_turn"]["1"],
        f64::from(graveyard_size)
    );
    assert_eq!(
        json["mana"]["avg_cast_spent_by_turn"]["1"],
        f64::from(mana_spent)
    );
}

/// Dispute reports include its conservative artifact boundary and the persistent payment-bank limits.
pub(super) fn assert_dispute_report(source: &CardRow) {
    assert_sacrifice_report(source);
    super::card_boundary_cost_tests::assert_dispute_cost_boundary_report(source);
    super::card_boundary_cost_tests::assert_treasure_bank_report(source);
}

/// Eternal Witness returns one physical card during its actual cast and entry turn.
pub(super) fn assert_witness_report(source: &CardRow) {
    let mut rows = vec![source.clone(), row("Return target", "{99}", "Artifact", "")];
    rows.extend((0..3).map(|_| row("Forest source", "", "Basic Land — Forest", "{T}: Add {G}.")));
    let cards = deck(&rows);
    let mut game = state(vec![0], vec![]);
    game.graveyard.push(CardIdx(1));
    for index in 2..=4 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 8;
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert!(game.graveyard.is_empty());
    let (log, json) = report_executed_turn(&cards, game, census);
    assert_eq!(log.graveyard_size, [0]);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 0.0);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 3.0);
}

/// Finite opponent payments and engine rewards reach the report from executed rounds.
pub(super) fn assert_opponent_engine_report(source: &CardRow) {
    let treasure = source.name == "Smothering Tithe";
    for paid in [true, false] {
        let paid_lands = if treasure { 3 } else { 2 };
        let (mut cards, mut game) = super::card_runtime_tests::engines::opponent_scenario(
            source,
            if paid { paid_lands } else { 1 },
        );
        if treasure {
            cards
                .cards
                .push(crate::deck::simulator::oracle_lower::parse_sim_card(&row(
                    "Treasure-funded follow-up",
                    "{1}",
                    "Sorcery",
                    "",
                )));
            game.hand.push(CardIdx(3));
        }
        crate::deck::simulator::opponents::run_opponent_round(&cards, &mut game, 1);
        let census = execute_turn(&cards, &mut game);
        let reward = u32::from(!paid);
        assert_eq!(game.treasure_bank, 0);
        assert_eq!(
            game.hand.len(),
            if treasure {
                usize::from(paid)
            } else {
                reward as usize
            }
        );
        let treasure_spent = u32::from(treasure && !paid);
        assert_eq!(
            game.graveyard,
            if treasure_spent > 0 {
                vec![CardIdx(3)]
            } else {
                vec![]
            }
        );
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_eq!(log.mana_spent, [f64::from(treasure_spent)]);
        assert_eq!(
            json["mana"]["avg_cast_spent_by_turn"]["1"],
            f64::from(treasure_spent)
        );
        assert_eq!(log.opponent_activity.draws, 1);
        assert_eq!(log.opponent_activity.casts, 1);
        assert_eq!(log.opponent_activity.payments, u32::from(paid));
        assert_eq!(
            json["opponent_model"]["observed_activity"]["totals"]["draws"],
            1
        );
        assert_eq!(
            json["opponent_model"]["observed_activity"]["totals"]["casts"],
            1
        );
        assert_eq!(
            json["opponent_model"]["observed_activity"]["totals"]["payments"],
            u32::from(paid)
        );
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            if treasure { 0.0 } else { f64::from(reward) }
        );
    }
}

/// Observe Harmonize through turn execution, production census, log, aggregate, and report.
pub(super) fn assert_harmonize_report(source: &CardRow) {
    let mut rows = vec![source.clone()];
    rows.extend((0..4).map(|_| row("Forest source", "", "Basic Land — Forest", "{T}: Add {G}.")));
    rows.extend((0..4).map(|_| row("Uncast draw", "{99}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut game = state(vec![0], vec![5, 6, 7, 8]);
    for index in 1..=4 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 5;
    let mut census = TurnCensus::new(1, cards.cards.len());
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &CommanderProfile::new(&cards),
        &mut Vec::new(),
        1,
        false,
    );
    assert_eq!(game.hand, [CardIdx(8), CardIdx(7), CardIdx(6)]);
    assert_eq!(game.library, [CardIdx(5)]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.life, 20);
    let log = finish_log(&cards, game, census, 0, false, 1);
    assert_eq!(log.cards_seen, [3]);
    assert_eq!(log.mana_spent, [4.0]);
    assert_eq!(log.graveyard_size, [1]);
    assert_eq!(log.library_size, [1]);
    assert_eq!(log.turns_observed, 1);
    let stats = aggregate(&[log], &cards, 1);
    let report = crate::deck::simulator::report::json_report(
        &stats,
        &cards,
        "Harmonize source scenario",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    let json = serde_json::to_value(report).expect("production report serializes");
    assert_report_compiler_records(&cards, &json);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 3.0);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 4.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 1.0);
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
}

/// Entry ramp reports the paid creature cast without counting the searched land as a land play.
pub(super) fn assert_entry_land_report(source: &CardRow) {
    let basic_only = source.name == "Farhaven Elf";
    for matching in [true, false] {
        let mut rows = vec![
            source.clone(),
            super::card_runtime_tests::search::entry_land_target(basic_only, matching),
        ];
        rows.extend(
            (0..3).map(|_| row("Funding Forest", "", "Basic Land — Forest", "{T}: Add {G}.")),
        );
        let cards = deck(&rows);
        let mut game = state(vec![0], vec![1]);
        for index in 2..=4 {
            game.battlefield
                .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
        }
        game.next_uid = 8;
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.library,
            if matching { vec![] } else { vec![CardIdx(1)] }
        );
        let found = game
            .battlefield
            .iter()
            .find(|permanent| permanent.card.deck_idx() == Some(CardIdx(1)));
        assert_eq!(found.is_some(), matching);
        if let Some(found) = found {
            assert_eq!(found.tapped, basic_only);
        }
        assert!(game.hand.is_empty() && game.graveyard.is_empty());
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [0]);
        assert_eq!(log.mana_spent, [3.0]);
        assert_eq!(log.library_size, [u32::from(!matching)]);
        assert_eq!(log.cards_seen, [u32::from(matching)]);
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 3.0);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], f64::from(matching));
        assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 0.0);
    }
}

/// Sift's three draws and ranked discard survive the complete turn and report path.
pub(super) fn assert_sift_report(source: &CardRow) {
    for legal in [true, false] {
        let mut rows = vec![source.clone(), row("Old hand", "{99}", "Sorcery", "")];
        rows.extend((0..4).map(|_| row("Physical draw", "{99}", "Sorcery", "")));
        rows.extend((0..4).map(|index| {
            if index == 0 && legal {
                row("Funding Island", "", "Basic Land — Island", "{T}: Add {U}.")
            } else {
                row("Funding Forest", "", "Basic Land — Forest", "{T}: Add {G}.")
            }
        }));
        let cards = deck(&rows);
        let mut game = state(vec![0, 1], vec![2, 3, 4, 5]);
        for index in 6..=9 {
            game.battlefield
                .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
        }
        game.next_uid = 10;
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.hand,
            if legal {
                vec![CardIdx(1), CardIdx(5), CardIdx(4)]
            } else {
                vec![CardIdx(0), CardIdx(1)]
            }
        );
        assert_eq!(
            game.graveyard,
            if legal {
                vec![CardIdx(3), CardIdx(0)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.library,
            if legal {
                vec![CardIdx(2)]
            } else {
                vec![CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(5)]
            }
        );
        assert_eq!(game.awareness_cards, if legal { 3 } else { 0 });
        assert_eq!(game.milled_self, 0);
        assert!(!game.failed_draw);
        let (log, json) = report_executed_turn(&cards, game, census);
        let drawn = if legal { 3 } else { 0 };
        let spent = if legal { 4.0 } else { 0.0 };
        let graveyard_size = if legal { 2 } else { 0 };
        assert_eq!(log.cards_seen, [drawn]);
        assert_eq!(log.card_first_seen.get(&3), legal.then_some(&1));
        assert_eq!(log.card_first_graveyard.get(&3), legal.then_some(&1));
        assert!(!log.card_first_graveyard.contains_key(&1));
        assert_eq!(log.library_size, [if legal { 1 } else { 4 }]);
        assert_eq!(log.mana_spent, [spent]);
        assert_eq!(log.graveyard_size, [graveyard_size]);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], f64::from(drawn));
        assert_eq!(json["velocity"]["self_milled_by_turn"]["1"], 0.0);
        assert_eq!(
            json["velocity"]["library_remaining_by_turn"]["1"],
            if legal { 1.0 } else { 4.0 }
        );
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], spent);
        assert_eq!(
            json["graveyard"]["avg_size_by_turn"]["1"],
            f64::from(graveyard_size)
        );
    }
}

/// Lamia's paid entry search reports graveyard access rather than a draw or hand access.
pub(super) fn assert_lamia_report(source: &CardRow) {
    for has_target in [true, false] {
        let mut rows = vec![
            source.clone(),
            row("Graveyard-search result", "{99}", "Sorcery", ""),
        ];
        rows.extend(
            (0..5).map(|_| row("Funding Swamp", "", "Basic Land — Swamp", "{T}: Add {B}.")),
        );
        let cards = deck(&rows);
        let mut game = state(vec![0], if has_target { vec![1] } else { vec![] });
        if !has_target {
            game.exile.push(CardIdx(1));
        }
        for index in 2..=6 {
            game.battlefield
                .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
        }
        game.next_uid = 8;
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.graveyard,
            if has_target { vec![CardIdx(1)] } else { vec![] }
        );
        assert!(game.hand.is_empty() && game.library.is_empty());
        assert!(!game.failed_draw);
        assert!(!game.card_first_seen.contains_key(&1));
        let (log, json) = report_executed_turn(&cards, game, census);
        let graveyard_size = u32::from(has_target);
        assert_eq!(log.cards_seen, [graveyard_size]);
        assert_eq!(log.graveyard_size, [graveyard_size]);
        assert_eq!(log.mana_spent, [5.0]);
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            f64::from(graveyard_size)
        );
        assert_eq!(
            json["graveyard"]["avg_size_by_turn"]["1"],
            f64::from(graveyard_size)
        );
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 5.0);
    }
}
