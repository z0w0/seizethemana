//! Synthetic library boundaries and pinned scry evidence through ordinary turns.

use crate::db::CardRow;
use crate::deck::simulator::card_test_support::{assert_library_rng, named_row, pinned_revision};
use crate::deck::simulator::game::{CardRef, GameCompletion, GameLog, GameState, ManaPool};
use crate::deck::simulator::game_effects::{CardZone, move_to_graveyard};
use crate::deck::simulator::library_core_report_tests::{
    assert_report_clause, cast_action, executed_report, put_permanents,
};
use crate::deck::simulator::model::{CardIdx, SimDeck, SimEffect};
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use serde_json::Value;

/// Require an unsupported public record for the exact synthetic input.
fn assert_inert_record(report: &Value, source: &CardRow) {
    let record = report["card_support"]
        .as_array()
        .expect("support records")
        .iter()
        .find(|record| record["name"] == source.name)
        .expect("source record");
    assert_eq!(
        record["source_hash"],
        crate::deck::simulator::support::card_source_hash(source)
    );
    assert!(
        record["abilities"]
            .as_array()
            .expect("ability records")
            .iter()
            .any(|ability| {
                ability["status"] == "unsupported"
                    && ability["reason"]
                        .as_str()
                        .is_some_and(|reason| !reason.is_empty())
            })
    );
}

/// Seed real graveyard instances without granting a replay or a new hand visit.
fn bury(cards: &SimDeck, game: &mut GameState, indexes: &[u32]) {
    for &index in indexes {
        move_to_graveyard(cards, game, CardIdx(index), 1, CardZone::Hand);
    }
}

/// Each draw rechecks competing fully payable dredgers rather than taking the first card.
pub(super) fn assert_largest_payable_dredge_report() {
    for (size, draws, selected, milled) in [
        (2, 1, vec![4], 0),
        (3, 1, vec![1], 3),
        (5, 1, vec![2], 5),
        (8, 2, vec![2, 1], 8),
    ] {
        let mut rows = vec![
            row(
                "Dredge draw action",
                "{1}",
                "Sorcery",
                if draws == 2 {
                    "Draw two cards."
                } else {
                    "Draw a card."
                },
            ),
            row("Smaller dredger", "{99}", "Creature", "Dredge 3"),
            row("Larger dredger", "{99}", "Creature", "Dredge 5"),
        ];
        rows.extend(
            (3..11).map(|index| row(&format!("Dredge library {index}"), "{99}", "Sorcery", "")),
        );
        rows.push(row(
            "Dredge funding",
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        ));
        let cards = deck(&rows);
        assert_eq!(cards.cards[1].dredge, Some(3));
        assert_eq!(cards.cards[2].dredge, Some(5));
        let mut game = state(vec![0], (3..3 + size).collect());
        bury(&cards, &mut game, &[1, 2]);
        put_permanents(&cards, &mut game, &[11]);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(
            game.hand,
            selected
                .iter()
                .map(|&index| CardIdx(index))
                .collect::<Vec<_>>()
        );
        let mut expected_graveyard: Vec<_> = [1, 2]
            .into_iter()
            .filter(|index| !selected.contains(index))
            .map(CardIdx)
            .collect();
        expected_graveyard.extend(
            (3 + size - milled..3 + size)
                .rev()
                .map(|index| CardIdx(index as u32)),
        );
        expected_graveyard.push(CardIdx(0));
        assert_eq!(game.graveyard, expected_graveyard);
        assert_eq!(
            game.library,
            if milled == 0 {
                vec![CardIdx(3)]
            } else {
                vec![]
            }
        );
        assert_eq!(game.milled_self, milled as u32);
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert!(game.exile.is_empty());
        assert_eq!(game.graveyard_generations[&CardIdx(1)], 1);
        assert_eq!(game.graveyard_generations[&CardIdx(2)], 1);
        let uses = if milled == 8 {
            2
        } else {
            u32::from(milled > 0)
        };
        assert_eq!(log.milestones_by_turn[0].dredge_uses, uses);
        assert_eq!(
            log.cards_seen,
            [if milled == 0 { 1 } else { milled as u32 }]
        );
        assert_eq!(log.mana_spent, [1.0]);
        assert_eq!(
            json["milestones"]["dredge_uses_avg_by_turn"]["1"],
            f64::from(uses)
        );
        assert_eq!(json["velocity"]["self_milled_by_turn"]["1"], milled as f64);
        assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// A paid unsupported scale leaves all rewards inert even when many artifacts match.
pub(super) fn assert_unsupported_scaled_draw_report() {
    for text in [
        "Draw a card for each artifact you control.",
        "Draw a card for each omen you remember.",
        "Draw two cards.",
    ] {
        for count in [0, 9] {
            let source = row("Paid scale control", "{1}", "Sorcery", text);
            let mut rows = vec![source.clone()];
            rows.extend(
                (1..=9)
                    .map(|index| row(&format!("Scale artifact {index}"), "{99}", "Artifact", "")),
            );
            rows.extend(
                (10..13).map(|index| row(&format!("Scale library {index}"), "{99}", "Sorcery", "")),
            );
            rows.push(row(
                "Scale funding",
                "",
                "Basic Land - Wastes",
                "{T}: Add {C}.",
            ));
            let cards = deck(&rows);
            let supported = text == "Draw two cards.";
            if supported {
                assert!(matches!(
                    cards.cards[0].spell_data.effects.as_slice(),
                    [SimEffect::Draw(2)]
                ));
            } else {
                assert!(cards.cards[0].spell_data.effects.is_empty());
            }
            let mut rejected = state(vec![0], vec![10, 11, 12]);
            let rng = rejected.library_rng.clone();
            assert_eq!(
                cast_action(&cards, &mut rejected, &mut ManaPool::default()),
                0.0
            );
            assert_eq!(rejected.hand, [CardIdx(0)]);
            assert!(rejected.graveyard.is_empty());
            assert_eq!(rejected.library, [CardIdx(10), CardIdx(11), CardIdx(12)]);
            let mut game = state(vec![0], vec![10, 11, 12]);
            put_permanents(&cards, &mut game, &[13]);
            put_permanents(&cards, &mut game, &(1..=count).collect::<Vec<_>>());
            let (game, log, json) = executed_report(&cards, game, 1, vec![]);
            assert_eq!(
                game.hand,
                if supported {
                    vec![CardIdx(12), CardIdx(11)]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                game.library,
                if supported {
                    vec![CardIdx(10)]
                } else {
                    vec![CardIdx(10), CardIdx(11), CardIdx(12)]
                }
            );
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(game.life, 20);
            assert_eq!(log.mana_spent, [1.0]);
            assert_eq!(
                json["velocity"]["avg_cards_seen"]["1"],
                if supported { 2.0 } else { 0.0 }
            );
            assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 1.0);
            if !supported {
                assert_inert_record(&json, &source);
            }
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// Mandatory reveals can lose the game; optional repeats never peek at hidden order.
pub(super) fn assert_life_reveal_risk_report() {
    let cards = deck(&[
        row(
            "Synthetic repeat reveal",
            "{1}",
            "Sorcery",
            "Reveal the top card of your library and put that card into your hand. You lose life equal to its mana value. You may repeat this process any number of times.",
        ),
        row("Reveal known large", "{6}", "Sorcery", ""),
        row("Reveal small", "{1}", "Sorcery", ""),
        row("Reveal medium", "{2}", "Sorcery", ""),
        row("Reveal funding", "", "Basic Land - Wastes", "{T}: Add {C}."),
    ]);
    assert!(cards.cards[0].spell_data.reveal_rule.is_some());
    for (life, library, expected_hand, remaining_life) in [
        (5, vec![2, 1], vec![1], -1),
        (6, vec![2, 1], vec![1], 0),
        (7, vec![1, 2], vec![2], 6),
        (7, vec![3, 2], vec![2], 6),
        (8, vec![1, 2], vec![2, 1], 1),
        (8, vec![3, 2], vec![2, 3], 5),
        (5, vec![], vec![], 5),
    ] {
        let mut game = state(vec![0], library.clone());
        game.life = life;
        put_permanents(&cards, &mut game, &[4]);
        let rng = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        let report = report_log(&cards, &log, 1);
        assert_eq!(
            game.hand,
            expected_hand
                .iter()
                .map(|&index| CardIdx(index))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            game.library,
            library[..library.len() - expected_hand.len()]
                .iter()
                .map(|&index| CardIdx(index as u32))
                .collect::<Vec<_>>()
        );
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.life, remaining_life);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.life_funded_draws, expected_hand.len() as u32);
        assert!(!game.failed_draw);
        assert_eq!(
            report.draw.avg_life_funded_draws,
            Some(expected_hand.len() as f64)
        );
        assert_eq!(report.draw.avg_life_paid, Some(0.0));
        assert_eq!(
            report.outcomes.player_loss_games,
            u32::from(remaining_life <= 0)
        );
        assert_eq!(report.outcomes.incomplete_games, 0);
        assert_eq!(log.turns_observed, u32::from(remaining_life > 0));
        if remaining_life <= 0 {
            assert!(matches!(
                game.completion,
                Some(GameCompletion::PlayerLost { .. })
            ));
            assert!(report.velocity.avg_cards_seen.is_empty());
            assert!(report.mana.avg_cast_spent_by_turn.is_empty());
        } else {
            assert_eq!(log.mana_spent, [1.0]);
            assert_eq!(
                report.velocity.avg_cards_seen[&1],
                expected_hand.len() as f64
            );
        }
        assert_library_rng(&game.library_rng, &rng);
    }
    assert_unfunded_reveal(&cards);
}

/// A missing printed spell payment cannot reveal a card or spend life.
fn assert_unfunded_reveal(cards: &SimDeck) {
    let game = state(vec![0], vec![1, 2]);
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(cards, game, 1, vec![]);
    assert_eq!(game.hand, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(1), CardIdx(2)]);
    assert!(game.graveyard.is_empty());
    assert_eq!(game.life, 20);
    assert_eq!(game.life_funded_draws, 0);
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(json["draw"]["avg_life_funded_draws"], 0.0);
    assert_library_rng(&game.library_rng, &rng);
}

/// Paid selection uses exposed cards and visible land needs without inventing draws.
pub(super) fn assert_scry_surveil_report() {
    for text in ["Scry 3.", "Surveil 3."] {
        let cards = selection_report_cards(text);
        for funded in [false, true] {
            for surplus_lands in [false, true] {
                for hidden in [[1, 6], [6, 1]] {
                    assert_selection_choices_report(&cards, &hidden, surplus_lands, funded);
                }
            }
        }
        assert_short_selection_reports(&cards);
    }
    assert_pinned_opt_selection_report();
}

/// Give selection both useful and rejected candidates without granting extra mana.
fn selection_report_cards(text: &str) -> SimDeck {
    let mut rows = vec![
        row("Synthetic selection action", "{1}", "Sorcery", text),
        row("Selection hidden expensive", "{99}", "Creature", ""),
        row("Selection affordable", "{2}", "Sorcery", ""),
        row("Selection exposed expensive", "{99}", "Artifact", ""),
        row(
            "Selection funding",
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        ),
        row("Selection exposed land", "", "Land", ""),
        row("Selection hidden cheap", "{0}", "Sorcery", ""),
    ];
    rows.extend(
        (7..11).map(|index| row(&format!("Selection visible land {index}"), "", "Land", "")),
    );
    deck(&rows)
}

/// Check paid keep, bottom, and graveyard choices against both hidden-prefix orders.
fn assert_selection_choices_report(
    cards: &SimDeck,
    hidden: &[usize],
    surplus_lands: bool,
    funded: bool,
) {
    let surveil = matches!(
        cards.cards[0].spell_data.effects.as_slice(),
        [SimEffect::Surveil(3)]
    );
    assert!(
        surveil
            || matches!(
                cards.cards[0].spell_data.effects.as_slice(),
                [SimEffect::Scry(3)]
            )
    );
    let library: Vec<_> = hidden.iter().copied().chain([5, 2, 3]).collect();
    let mut game = state(vec![0], library.clone());
    if surplus_lands {
        put_permanents(cards, &mut game, &[7, 8, 9, 10]);
    }
    if funded {
        put_permanents(cards, &mut game, &[4]);
    }
    let before = selection_zone_indexes(&game);
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(cards, game, 1, vec![]);
    let kept = if surplus_lands {
        vec![2]
    } else if surveil {
        vec![5, 2]
    } else {
        vec![2, 5]
    };
    let rejected = if surplus_lands { vec![3, 5] } else { vec![3] };
    let expected_library: Vec<_> = if !funded {
        library
    } else if surveil {
        hidden.iter().copied().chain(kept).collect()
    } else {
        rejected
            .iter()
            .rev()
            .copied()
            .chain(hidden.iter().copied())
            .chain(kept)
            .collect()
    };
    let expected_graveyard: Vec<_> = if !funded {
        vec![]
    } else if surveil {
        rejected.iter().copied().chain([0]).collect()
    } else {
        vec![0]
    };
    assert_eq!(
        game.library,
        expected_library
            .into_iter()
            .map(|index| CardIdx(index as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        game.graveyard,
        expected_graveyard
            .into_iter()
            .map(|index| CardIdx(index as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(game.hand, if funded { vec![] } else { vec![CardIdx(0)] });
    assert_eq!(selection_zone_indexes(&game), before);
    if funded {
        assert!(
            game.battlefield
                .iter()
                .find(|perm| perm.card == CardRef::Deck(CardIdx(4)))
                .expect("paid source remains on the battlefield")
                .tapped
        );
    }
    let milled = if funded && surveil {
        rejected.len() as u32
    } else {
        0
    };
    assert_selection_census(
        cards,
        &game,
        &log,
        &json,
        if funded { 3 } else { 0 },
        milled,
        funded,
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Empty and short libraries expose only available cards without causing draw loss.
fn assert_short_selection_reports(cards: &SimDeck) {
    let surveil = matches!(
        cards.cards[0].spell_data.effects.as_slice(),
        [SimEffect::Surveil(3)]
    );
    for library in [vec![], vec![2], vec![3]] {
        let mut game = state(vec![0], library.clone());
        put_permanents(cards, &mut game, &[4]);
        let before = selection_zone_indexes(&game);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(cards, game, 1, vec![]);
        let milled = u32::from(surveil && library == [3]);
        assert_eq!(
            game.library,
            if milled == 1 {
                vec![]
            } else {
                library.iter().map(|&index| CardIdx(index as u32)).collect()
            }
        );
        assert_eq!(
            game.graveyard,
            if milled == 1 {
                vec![CardIdx(3), CardIdx(0)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert!(game.hand.is_empty());
        assert_eq!(selection_zone_indexes(&game), before);
        assert_selection_census(
            cards,
            &game,
            &log,
            &json,
            library.len() as u32,
            milled,
            true,
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Compare physical zones with the executed census and the rounded public report.
fn assert_selection_census(
    cards: &SimDeck,
    game: &GameState,
    log: &GameLog,
    json: &Value,
    awareness: u32,
    milled: u32,
    funded: bool,
) {
    let share = f64::from(awareness) / cards.library_len() as f64;
    let spent = f64::from(u32::from(funded));
    assert_eq!(game.awareness_cards, awareness);
    assert_eq!(game.seen, milled);
    assert!(game.card_first_seen.is_empty());
    assert_eq!(game.milled_self, milled);
    assert_eq!(game.milled_opp, 0);
    assert_eq!(game.life, 20);
    assert!(!game.failed_draw);
    assert!(game.exile.is_empty());
    assert_eq!(log.completion, GameCompletion::HorizonReached);
    assert_eq!(log.cards_seen, [milled]);
    assert_eq!(log.awareness, [share]);
    assert_eq!(log.library_size, [game.library.len() as u32]);
    assert_eq!(log.graveyard_size, [game.graveyard.len() as u32]);
    assert_eq!(log.mana_spent, [spent]);
    assert_eq!(
        json["velocity"]["library_awareness_by_turn"]["1"],
        (share * 10000.0).round() / 100.0
    );
    assert_eq!(
        json["velocity"]["self_milled_by_turn"]["1"],
        f64::from(milled)
    );
    assert_eq!(json["velocity"]["opponent_milled_by_turn"]["1"], 0.0);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], f64::from(milled));
    assert_eq!(
        json["velocity"]["library_remaining_by_turn"]["1"],
        game.library.len() as f64
    );
    assert_eq!(
        json["graveyard"]["avg_size_by_turn"]["1"],
        game.graveyard.len() as f64
    );
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], spent);
}

/// Require each seeded physical card to remain in exactly one tracked zone.
fn selection_zone_indexes(game: &GameState) -> Vec<CardIdx> {
    let mut indexes: Vec<_> = game
        .hand
        .iter()
        .chain(&game.library)
        .chain(&game.graveyard)
        .chain(&game.exile)
        .copied()
        .chain(
            game.battlefield
                .iter()
                .filter_map(|perm| perm.card.deck_idx()),
        )
        .collect();
    indexes.sort_by_key(|index| index.index());
    assert!(indexes.windows(2).all(|pair| pair[0] != pair[1]));
    indexes
}

/// The unchanged pinned Opt source can keep a cheap card or bottom an expensive one.
fn assert_pinned_opt_selection_report() {
    let source = named_row("Opt");
    pinned_revision(&source);
    for top_cost in ["{2}", "{99}"] {
        let cards = deck(&[
            source.clone(),
            row("Opt next card", "{99}", "Sorcery", ""),
            row("Opt top card", top_cost, "Sorcery", ""),
            row("Opt funding", "", "Basic Land - Island", "{T}: Add {U}."),
        ]);
        for funded in [false, true] {
            let mut game = state(vec![0], vec![1, 2]);
            if funded {
                put_permanents(&cards, &mut game, &[3]);
            }
            let before = selection_zone_indexes(&game);
            let rng = game.library_rng.clone();
            let (game, log, json) = executed_report(&cards, game, 1, vec![]);
            let drawn = if top_cost == "{2}" { 2 } else { 1 };
            assert_eq!(
                game.hand,
                if funded {
                    vec![CardIdx(drawn)]
                } else {
                    vec![CardIdx(0)]
                }
            );
            assert_eq!(
                game.library,
                if funded {
                    vec![CardIdx(3 - drawn)]
                } else {
                    vec![CardIdx(1), CardIdx(2)]
                }
            );
            assert_eq!(
                game.graveyard,
                if funded { vec![CardIdx(0)] } else { vec![] }
            );
            assert_eq!(selection_zone_indexes(&game), before);
            assert_eq!(game.awareness_cards, if funded { 2 } else { 0 });
            assert_eq!(game.seen, u32::from(funded));
            assert_eq!(game.milled_self, 0);
            assert_eq!(game.card_first_seen.len(), usize::from(funded));
            if funded {
                assert_eq!(game.card_first_seen.get(&(drawn as usize)), Some(&1));
                assert!(game.battlefield[0].tapped);
            }
            assert_eq!(log.cards_seen, [u32::from(funded)]);
            assert_eq!(log.mana_spent, [f64::from(u32::from(funded))]);
            assert_eq!(
                json["velocity"]["library_awareness_by_turn"]["1"],
                if funded { 50.0 } else { 0.0 }
            );
            assert_eq!(
                json["velocity"]["avg_cards_seen"]["1"],
                f64::from(u32::from(funded))
            );
            assert_eq!(json["velocity"]["self_milled_by_turn"]["1"], 0.0);
            assert_eq!(
                json["velocity"]["library_remaining_by_turn"]["1"],
                if funded { 1.0 } else { 2.0 }
            );
            assert_eq!(
                json["graveyard"]["avg_size_by_turn"]["1"],
                f64::from(u32::from(funded))
            );
            assert_report_clause(&json, &source, "Scry 1", "compiler_only", None);
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// Mill listeners require a library origin; opponent mill remains counter-only evidence.
pub(super) fn assert_mill_origin_report() {
    for (text, origin) in [
        ("Mill two cards.", "library"),
        ("Target opponent mills two cards.", "opponent"),
        (
            "As an additional cost to cast this spell, discard a card. Gain 1 life.",
            "discard",
        ),
        (
            "As an additional cost to cast this spell, sacrifice a creature. Gain 1 life.",
            "sacrifice",
        ),
    ] {
        let cards = mill_origin_cards(text);
        assert!(cards.cards[1].library_graveyard_trigger.is_some());
        assert!(cards.cards[2].library_graveyard_trigger.is_some());
        let game = mill_origin_state(&cards, origin);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        let from_library = origin == "library";
        assert_eq!(game.milled_self, if from_library { 2 } else { 0 });
        assert_eq!(game.milled_opp, if origin == "opponent" { 2 } else { 0 });
        assert_eq!(
            game.library,
            if origin == "opponent" {
                vec![CardIdx(3), CardIdx(2), CardIdx(1)]
            } else {
                vec![CardIdx(3)]
            }
        );
        assert_eq!(
            game.exile,
            if from_library {
                vec![CardIdx(2)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.graveyard,
            match origin {
                "discard" => vec![CardIdx(2), CardIdx(0)],
                "sacrifice" => vec![CardIdx(1), CardIdx(0)],
                _ => vec![CardIdx(0)],
            }
        );
        assert_eq!(
            game.battlefield
                .iter()
                .filter(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
                .count(),
            usize::from(from_library)
        );
        assert_eq!(
            game.life_gained,
            if from_library {
                3
            } else if origin == "opponent" {
                0
            } else {
                1
            }
        );
        assert_eq!(
            log.card_first_graveyard.get(&2),
            matches!(origin, "library" | "discard").then_some(&1)
        );
        assert_eq!(
            log.card_first_battlefield.get(&1),
            from_library.then_some(&1)
        );
        assert_eq!(log.mana_spent, [1.0]);
        assert_eq!(
            json["velocity"]["self_milled_by_turn"]["1"],
            if from_library { 2.0 } else { 0.0 }
        );
        assert_eq!(
            json["velocity"]["opponent_milled_by_turn"]["1"],
            if origin == "opponent" { 2.0 } else { 0.0 }
        );
        assert_eq!(
            json["velocity"]["avg_cards_seen"]["1"],
            if from_library { 2.0 } else { 0.0 }
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Put the listener in the actual zone required by each ordinary cost or mill action.
fn mill_origin_state(cards: &SimDeck, origin: &str) -> GameState {
    let mut game = state(
        if origin == "discard" {
            vec![0, 2]
        } else {
            vec![0]
        },
        if matches!(origin, "library" | "opponent") {
            vec![3, 2, 1]
        } else {
            vec![3]
        },
    );
    put_permanents(cards, &mut game, &[4]);
    if origin == "sacrifice" {
        put_permanents(cards, &mut game, &[1]);
    }
    game
}

/// Give origin controls the same listeners, library markers, and printed payment source.
fn mill_origin_cards(text: &str) -> SimDeck {
    deck(&[
        row("Synthetic mill action", "{1}", "Sorcery", text),
        row(
            "Library return listener",
            "{99}",
            "Creature",
            "When Library Return Listener is put into your graveyard from your library, you may put it onto the battlefield.",
        ),
        row(
            "Library drain listener",
            "{99}",
            "Sorcery",
            "When Library Drain Listener is put into your graveyard from your library, each opponent loses 3 life and you gain 3 life.",
        ),
        row("Mill retained", "{99}", "Sorcery", ""),
        row("Mill funding", "", "Basic Land - Wastes", "{T}: Add {C}."),
    ])
}
