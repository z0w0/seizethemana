//! Source-bound library clauses. Every helper certifies only its named subset.

use super::card_test_support::{assert_library_rng, main_phase_state, named_row, pinned_revision};
use super::game::{CardRef, ManaPool};
use super::game_effects::activation::{pick_best_activation, resolve_activation};
use super::library_core_report_tests::{
    assert_report_clause, cast_action, executed_report, put_permanents,
};
use super::model::{CardIdx, SimEffect};
use super::scenario_test_support::{deck, row, state};
use crate::db::CardRow;

/// Faithless Looting pays red, draws first, and retains older cards when scores tie.
pub(super) fn assert_faithless_looting_hand_report(source: &CardRow) {
    pinned_revision(source);
    let mut rows = vec![
        source.clone(),
        row("Oldest held card", "{99}", "Artifact", ""),
        row("First physical draw", "{99}", "Sorcery", ""),
        row("Second physical draw", "{99}", "Sorcery", ""),
        row("Undrawn card", "{99}", "Sorcery", ""),
    ];
    rows.push(row(
        "Red payment",
        "",
        "Basic Land - Mountain",
        "{T}: Add {R}.",
    ));
    let cards = deck(&rows);
    assert!(matches!(
        cards.cards[0].spell_data.effects.as_slice(),
        [SimEffect::DrawThenDiscard(2)]
    ));
    let mut rejected = main_phase_state(vec![0, 1], vec![4, 3, 2]);
    let rng = rejected.library_rng.clone();
    let mut wrong_color = ManaPool {
        fixed: [0, 0, 0, 0, 1],
        ..Default::default()
    };
    assert_eq!(cast_action(&cards, &mut rejected, &mut wrong_color), 0.0);
    assert_eq!(wrong_color.fixed, [0, 0, 0, 0, 1]);
    assert_eq!(rejected.hand, [CardIdx(0), CardIdx(1)]);
    assert_eq!(rejected.library, [CardIdx(4), CardIdx(3), CardIdx(2)]);
    assert!(rejected.graveyard.is_empty());
    assert_eq!(rejected.seen, 0);
    assert_library_rng(&rejected.library_rng, &rng);

    let mut game = state(vec![0, 1], vec![4, 3, 2]);
    put_permanents(&cards, &mut game, &[5]);
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.graveyard, [CardIdx(3), CardIdx(2), CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(4)]);
    assert!(game.exile.is_empty());
    assert_eq!(log.cards_seen, [2]);
    assert_eq!(log.mana_spent, [1.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(log.replay_casts, 0);
    assert_eq!(log.card_first_seen.get(&2), Some(&1));
    assert_eq!(log.card_first_graveyard.get(&2), Some(&1));
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 2.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 3.0);
    assert_report_clause(
        &json,
        source,
        "Draw two cards, then discard two cards.",
        "compiler_only",
        None,
    );
    assert!(
        json["assumptions"]
            .as_array()
            .expect("assumptions")
            .iter()
            .any(|text| text.as_str().is_some_and(
                |text| text.contains("Loot completes all draws before choosing its discards")
            ))
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Relic's paid draw mode taps once and moves two drawn cards and one discard.
pub(super) fn assert_relic_draw_discard_report(source: &CardRow) {
    pinned_revision(source);
    let mut rows = vec![
        source.clone(),
        row("Relic oldest hand card", "{99}", "Artifact", ""),
        row("Relic first draw", "{99}", "Sorcery", ""),
        row("Relic second draw", "{99}", "Sorcery", ""),
        row("Relic undrawn card", "{99}", "Sorcery", ""),
    ];
    rows.extend((0..3).map(|index| {
        row(
            &format!("Relic payment {index}"),
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        )
    }));
    let cards = deck(&rows);
    let mut game = main_phase_state(vec![1], vec![4, 3, 2]);
    put_permanents(&cards, &mut game, &[0]);
    let rng = game.library_rng.clone();
    let mut pool = ManaPool {
        colorless: 3,
        ..Default::default()
    };
    let action =
        pick_best_activation(&cards, &game, &pool, 1).expect("paid draw activation exists");
    assert!(matches!(
        action.ability.effect_sequence(),
        [SimEffect::DrawDiscard {
            draw: 2,
            discard: 1
        }]
    ));
    let mut short = game.clone();
    let mut short_pool = ManaPool {
        colorless: 2,
        ..Default::default()
    };
    resolve_activation(&cards, &mut short, &mut short_pool, 1, &action);
    assert_eq!(short_pool.colorless, 2);
    assert_eq!(short.hand, [CardIdx(1)]);
    assert!(short.graveyard.is_empty());
    assert!(!short.battlefield[0].tapped);
    assert_eq!(short.library, game.library);
    let mut tapped = game.clone();
    tapped.battlefield[0].tapped = true;
    resolve_activation(&cards, &mut tapped, &mut pool, 1, &action);
    assert_eq!(pool.colorless, 3);
    assert_eq!(tapped.hand, [CardIdx(1)]);
    assert!(tapped.graveyard.is_empty());
    resolve_activation(&cards, &mut game, &mut pool, 1, &action);
    assert_eq!(pool.total(), 0);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.hand, [CardIdx(1), CardIdx(2)]);
    assert_eq!(game.graveyard, [CardIdx(3)]);
    assert_eq!(game.library, [CardIdx(4)]);
    assert_eq!(game.seen, 2);
    assert_library_rng(&game.library_rng, &rng);

    let mut report_game = state(vec![1], vec![4, 3, 2]);
    put_permanents(&cards, &mut report_game, &[0, 5, 6, 7]);
    let (report_game, log, json) = executed_report(&cards, report_game, 1, vec![]);
    assert_eq!(report_game.hand, [CardIdx(1), CardIdx(2)]);
    assert_eq!(report_game.graveyard, [CardIdx(3)]);
    assert_eq!(report_game.library, [CardIdx(4)]);
    assert_eq!(log.cards_seen, [2]);
    assert_eq!(
        log.mana_spent,
        [0.0],
        "activation costs are not cast spending"
    );
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 2.0);
    assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
    assert_report_clause(
        &json,
        source,
        "Draw two cards, then discard a card.",
        "approximated",
        Some("pilot_card_selection"),
    );
    assert_library_rng(&report_game.library_rng, &rng);
}

/// Windfall uses the declared seven-card wheel, not the printed multiplayer maximum.
pub(super) fn assert_windfall_fixed_seven_report(source: &CardRow) {
    pinned_revision(source);
    let mut rows = vec![
        source.clone(),
        row("Wheel discard one", "{99}", "Artifact", ""),
        row("Wheel discard two", "{99}", "Artifact", ""),
    ];
    rows.extend(
        (3..=10).map(|index| row(&format!("Wheel library {index}"), "{99}", "Sorcery", "")),
    );
    rows.extend((11..14).map(|index| {
        row(
            &format!("Wheel payment {index}"),
            "",
            "Basic Land - Island",
            "{T}: Add {U}.",
        )
    }));
    let cards = deck(&rows);
    assert!(matches!(
        cards.cards[0].spell_data.effects.as_slice(),
        [SimEffect::DiscardHandThenDrawSeven]
    ));
    let mut rejected = main_phase_state(vec![0, 1, 2], (3..=10).rev().collect());
    let rng = rejected.library_rng.clone();
    let mut pool = ManaPool {
        colorless: 3,
        ..Default::default()
    };
    cast_action(&cards, &mut rejected, &mut pool);
    assert_eq!(pool.colorless, 3);
    assert_eq!(rejected.hand, [CardIdx(0), CardIdx(1), CardIdx(2)]);
    assert!(rejected.graveyard.is_empty());
    assert_eq!(rejected.seen, 0);
    assert_library_rng(&rejected.library_rng, &rng);
    let mut game = state(vec![0, 1, 2], (3..=10).rev().collect());
    put_permanents(&cards, &mut game, &[11, 12, 13]);
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.hand, (3..10).map(CardIdx).collect::<Vec<_>>());
    assert_eq!(game.library, [CardIdx(10)]);
    assert_eq!(game.graveyard, [CardIdx(1), CardIdx(2), CardIdx(0)]);
    assert_eq!(log.cards_seen, [7]);
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 7.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 3.0);
    assert_report_clause(
        &json,
        source,
        "Each player discards their hand",
        "compiler_only",
        None,
    );
    assert!(
        json["assumptions"]
            .as_array()
            .expect("assumptions")
            .iter()
            .any(|text| text
                .as_str()
                .is_some_and(|text| text.contains("wheel discards the hand and draws seven")))
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Wanderer's boolean flag executes one ordered cascade, not both printed cascades.
pub(super) fn assert_wanderer_one_cascade_subset_report(source: &CardRow) {
    pinned_revision(source);
    assert!(source.oracle_text.contains("Cascade, cascade"));
    let mut rows = vec![
        source.clone(),
        row(
            "First cascade hit",
            "{1}",
            "Creature",
            "When this creature enters, gain 3 life.",
        ),
        row("Uncast second cascade candidate", "{1}", "Creature", ""),
        row("Equal value miss", "{8}", "Artifact", ""),
        row("Cascade land miss", "", "Land - Island", ""),
    ];
    rows.extend((0..8).map(|index| {
        row(
            &format!("Cascade payment {index}"),
            "",
            "Basic Land - Forest",
            if index == 0 {
                "{T}: Add {U}."
            } else if index == 1 {
                "{T}: Add {R}."
            } else {
                "{T}: Add {G}."
            },
        )
    }));
    let cards = deck(&rows);
    assert!(cards.cards[0].has_cascade);
    let mut rejected = main_phase_state(vec![0], vec![2, 1, 3, 4]);
    let rejected_rng = rejected.library_rng.clone();
    let mut wrong_pool = ManaPool {
        colorless: 8,
        ..Default::default()
    };
    cast_action(&cards, &mut rejected, &mut wrong_pool);
    assert_eq!(wrong_pool.colorless, 8);
    assert_eq!(rejected.hand, [CardIdx(0)]);
    assert_eq!(
        rejected.library,
        [CardIdx(2), CardIdx(1), CardIdx(3), CardIdx(4)]
    );
    assert!(rejected.battlefield.is_empty());
    assert_eq!(rejected.seen, 0);
    assert_library_rng(&rejected.library_rng, &rejected_rng);
    for found in [false, true] {
        let library = if found { vec![2, 1, 3, 4] } else { vec![3, 4] };
        let mut game = state(vec![0], library);
        let rng = game.library_rng.clone();
        put_permanents(&cards, &mut game, &(5..13).collect::<Vec<_>>());
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert!(game.hand.is_empty());
        assert_eq!(
            game.library,
            if found {
                vec![CardIdx(4), CardIdx(3), CardIdx(2)]
            } else {
                vec![CardIdx(4), CardIdx(3)]
            }
        );
        assert!(game.graveyard.is_empty());
        assert!(game.exile.is_empty());
        assert_eq!(game.life, if found { 23 } else { 20 });
        assert_eq!(
            game.battlefield
                .iter()
                .filter(|perm| perm.card == CardRef::Deck(CardIdx(1)))
                .count(),
            usize::from(found)
        );
        assert!(
            !game
                .battlefield
                .iter()
                .any(|perm| perm.card == CardRef::Deck(CardIdx(2)))
        );
        assert_eq!(log.cards_seen, [if found { 3 } else { 2 }]);
        assert_eq!(log.mana_spent, [8.0]);
        assert_eq!(log.mana_remaining, [0.0]);
        assert_eq!(
            log.milestones_by_turn[0].free_cast_permanents_entered,
            u32::from(found)
        );
        assert_eq!(
            json["milestones"]["free_cast_permanents_entered_avg_by_turn"]["1"],
            f64::from(u32::from(found))
        );
        assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
        assert_report_clause(&json, source, "Cascade, cascade", "unsupported", None);
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Life from the Loam supplies Dredge 3; its filtered land recursion stays unsupported.
pub(super) fn assert_loam_dredge_subset_report(source: &CardRow) {
    pinned_revision(source);
    for full in [false, true] {
        let cards = deck(&[
            source.clone(),
            row("Loam mill one", "{99}", "Sorcery", ""),
            row("Loam mill two", "{99}", "Sorcery", ""),
            row("Loam mill three", "{99}", "Sorcery", ""),
        ]);
        assert_eq!(cards.cards[0].dredge, Some(3));
        assert!(cards.cards[0].spell_data.effects.is_empty());
        let mut game = state(vec![], if full { vec![1, 2, 3] } else { vec![1, 2] });
        super::game_effects::move_to_graveyard(
            &cards,
            &mut game,
            CardIdx(0),
            1,
            super::game_effects::CardZone::Hand,
        );
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 2, vec![]);
        assert_eq!(game.hand, [CardIdx(if full { 0 } else { 2 })]);
        assert_eq!(game.library, if full { vec![] } else { vec![CardIdx(1)] });
        assert_eq!(
            game.graveyard,
            if full {
                vec![CardIdx(3), CardIdx(2), CardIdx(1)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(log.self_milled, [0, if full { 3 } else { 0 }]);
        assert_eq!(log.milestones_by_turn[1].dredge_uses, u32::from(full));
        assert_eq!(log.replay_casts, 0);
        assert_eq!(
            json["velocity"]["self_milled_by_turn"]["2"],
            if full { 3.0 } else { 0.0 }
        );
        assert_eq!(
            json["milestones"]["dredge_uses_avg_by_turn"]["2"],
            f64::from(u32::from(full))
        );
        assert_report_clause(
            &json,
            source,
            "Return up to three target land cards",
            "unsupported",
            None,
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Emry's paid entry mills four physical cards without granting artifact replay support.
pub(super) fn assert_emry_entry_mill_subset_report(source: &CardRow) {
    pinned_revision(source);
    let mut rows = vec![source.clone()];
    rows.extend((1..=5).map(|index| row(&format!("Emry library {index}"), "{99}", "Artifact", "")));
    rows.extend((0..3).map(|index| {
        row(
            &format!("Emry payment {index}"),
            "",
            "Basic Land - Island",
            "{T}: Add {U}.",
        )
    }));
    let cards = deck(&rows);
    let mut rejected = main_phase_state(vec![0], vec![5, 4, 3, 2, 1]);
    let mut wrong = ManaPool {
        colorless: 3,
        ..Default::default()
    };
    cast_action(&cards, &mut rejected, &mut wrong);
    assert_eq!(wrong.colorless, 3);
    assert_eq!(rejected.hand, [CardIdx(0)]);
    assert!(rejected.graveyard.is_empty());
    assert_eq!(rejected.milled_self, 0);
    let mut game = state(vec![0], vec![5, 4, 3, 2, 1]);
    let rng = game.library_rng.clone();
    put_permanents(&cards, &mut game, &[6, 7, 8]);
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.library, [CardIdx(5)]);
    assert_eq!(
        game.graveyard,
        [CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4)]
    );
    assert!(game.hand.is_empty());
    assert_eq!(log.cards_seen, [4]);
    assert_eq!(log.self_milled, [4]);
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(log.replay_casts, 0);
    assert_eq!(log.card_first_battlefield.get(&0), Some(&1));
    assert!(!log.card_first_seen.contains_key(&1));
    assert_eq!(log.card_first_graveyard.get(&1), Some(&1));
    assert_eq!(json["velocity"]["self_milled_by_turn"]["1"], 4.0);
    assert_report_clause(
        &json,
        source,
        "When Emry enters, mill four cards.",
        "supported",
        None,
    );
    assert_report_clause(
        &json,
        source,
        "Choose target artifact card",
        "unsupported",
        None,
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Execute Faithless Looting's hand-cast subset.
#[test]
fn faithless_looting_hand_clause() {
    assert_faithless_looting_hand_report(&named_row("Faithless Looting"));
}

/// Execute Relic's paid draw/discard subset.
#[test]
fn relic_paid_draw_discard_clause() {
    assert_relic_draw_discard_report(&named_row("Relic of Sauron"));
}

/// Execute Windfall's declared wheel approximation.
#[test]
fn windfall_fixed_seven_clause() {
    assert_windfall_fixed_seven_report(&named_row("Windfall"));
}

/// Execute one cascade while retaining the missing-second-cascade gap.
#[test]
fn wanderer_one_cascade_subset_clause() {
    assert_wanderer_one_cascade_subset_report(&named_row("Maelstrom Wanderer"));
}

/// Execute Dredge 3 without certifying Loam's land return.
#[test]
fn loam_dredge_three_subset_clause() {
    assert_loam_dredge_subset_report(&named_row("Life from the Loam"));
}

/// Execute Emry's entry mill without certifying artifact replay.
#[test]
fn emry_entry_mill_four_subset_clause() {
    assert_emry_entry_mill_subset_report(&named_row("Emry, Lurker of the Loch"));
}
