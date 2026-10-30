//! Physical replay and narrow recursion evidence from pinned source wording.

use super::card_test_support::{assert_library_rng, main_phase_state, named_row, pinned_revision};
use super::game::{CardRef, GameState, ManaPool};
use super::game_effects::activation::pick_best_activation;
use super::game_effects::{CardZone, move_to_graveyard};
use super::library_core_report_tests::{
    assert_report_clause, cast_action, executed_report, put_permanents,
};
use super::model::{CardIdx, SimDeck};
use super::scenario_test_support::{deck, row, state};
use crate::db::CardRow;

/// Faithless Looting's own flashback pays three mana and exiles the resolving source.
pub(super) fn assert_faithless_looting_flashback_report(source: &CardRow) {
    pinned_revision(source);
    let mut rows = vec![
        source.clone(),
        row("Flashback oldest card", "{99}", "Artifact", ""),
        row("Flashback first draw", "{99}", "Sorcery", ""),
        row("Flashback second draw", "{99}", "Sorcery", ""),
        row("Flashback remaining card", "{99}", "Sorcery", ""),
    ];
    rows.extend((0..3).map(|index| {
        row(
            &format!("Flashback payment {index}"),
            "",
            "Basic Land - Mountain",
            "{T}: Add {R}.",
        )
    }));
    let cards = deck(&rows);
    let cost = cards.cards[0]
        .spell_data
        .own_flashback
        .expect("printed flashback cost");
    assert_eq!(cost.total(), 3);
    assert_eq!(cost.pips[3], 1);
    for wrong_color in [false, true] {
        let mut game = main_phase_state(vec![1], vec![4, 3, 2]);
        move_to_graveyard(&cards, &mut game, CardIdx(0), 1, CardZone::Hand);
        let rng = game.library_rng.clone();
        let mut pool = if wrong_color {
            ManaPool {
                colorless: 3,
                ..Default::default()
            }
        } else {
            ManaPool {
                fixed: [0, 0, 0, 1, 0],
                colorless: 1,
                ..Default::default()
            }
        };
        let before = pool.clone();
        assert_eq!(cast_action(&cards, &mut game, &mut pool), 0.0);
        assert_eq!(pool, before);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert_eq!(game.library, [CardIdx(4), CardIdx(3), CardIdx(2)]);
        assert!(game.exile.is_empty());
        assert_eq!(game.replay_casts, 0);
        assert_library_rng(&game.library_rng, &rng);
    }
    let mut game = state(vec![1], vec![4, 3, 2]);
    move_to_graveyard(&cards, &mut game, CardIdx(0), 1, CardZone::Hand);
    put_permanents(&cards, &mut game, &[5, 6, 7]);
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.exile, [CardIdx(0)]);
    assert_eq!(game.graveyard, [CardIdx(3), CardIdx(2)]);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.library, [CardIdx(4)]);
    assert_eq!(game.life, 20);
    assert_eq!(
        (game.seen, game.awareness_cards, game.milled_self),
        (2, 2, 0)
    );
    assert!(!game.failed_draw);
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(log.cards_seen, [2]);
    assert_eq!(log.card_first_seen.get(&2), Some(&1));
    assert_eq!(log.card_first_seen.get(&3), Some(&1));
    assert_eq!(log.card_first_graveyard.get(&2), Some(&1));
    assert_eq!(log.card_first_graveyard.get(&3), Some(&1));
    assert!(!log.card_first_graveyard.contains_key(&1));
    assert_eq!(log.replay_casts, 1);
    assert_eq!(log.milestones_by_turn[0].graveyard_casts, 1);
    assert_eq!(json["graveyard"]["replay_casts_avg"], 1.0);
    assert_eq!(json["milestones"]["graveyard_casts_avg_by_turn"]["1"], 1.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 2.0);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 2.0);
    assert_eq!(json["velocity"]["self_milled_by_turn"]["1"], 0.0);
    assert_eq!(json["velocity"]["library_remaining_by_turn"]["1"], 1.0);
    assert!(source.oracle_text.contains("Flashback {2}{R}"));
    assert_report_clause(
        &json,
        source,
        "Draw two cards, then discard two cards.",
        "compiler_only",
        None,
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Past in Flames grants only the spell instances already in the graveyard at resolution.
pub(super) fn assert_past_in_flames_instance_report(source: &CardRow) {
    pinned_revision(source);
    let mut rows = vec![
        source.clone(),
        row("Granted physical spell", "{U}", "Instant", "Gain 2 life."),
        row("Granted physical spell", "{U}", "Instant", "Gain 2 life."),
        row("Nonspell graveyard card", "{99}", "Artifact", ""),
    ];
    rows.extend((0..3).map(|index| {
        row(
            &format!("Grant generic payment {index}"),
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        )
    }));
    rows.push(row(
        "Grant red payment",
        "",
        "Basic Land - Mountain",
        "{T}: Add {R}.",
    ));
    rows.push(row(
        "Grant blue payment",
        "",
        "Basic Land - Island",
        "{T}: Add {U}.",
    ));
    let cards = deck(&rows);
    assert!(cards.cards[0].spell_data.grants_flashback);
    let mut game = main_phase_state(vec![0], vec![]);
    move_to_graveyard(&cards, &mut game, CardIdx(1), 1, CardZone::Hand);
    move_to_graveyard(&cards, &mut game, CardIdx(3), 1, CardZone::Hand);
    let rng = game.library_rng.clone();
    let mut red = ManaPool {
        fixed: [0, 0, 0, 1, 0],
        colorless: 3,
        ..Default::default()
    };
    assert_eq!(cast_action(&cards, &mut game, &mut red), 4.0);
    assert_eq!(red.total(), 0);
    assert!(game.flashback_permissions.contains(&(CardIdx(1), 1)));
    assert!(
        !game
            .flashback_permissions
            .iter()
            .any(|(index, _)| *index == CardIdx(2))
    );
    assert!(
        !game
            .flashback_permissions
            .iter()
            .any(|(index, _)| *index == CardIdx(3))
    );
    move_to_graveyard(&cards, &mut game, CardIdx(2), 1, CardZone::Hand);
    let mut blue = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        ..Default::default()
    };
    assert_eq!(cast_action(&cards, &mut game, &mut blue), 1.0);
    assert_eq!(blue.total(), 0);
    assert_eq!(game.exile, [CardIdx(1)]);
    assert_eq!(game.graveyard, [CardIdx(3), CardIdx(0), CardIdx(2)]);
    assert_eq!(game.replay_casts, 1);
    assert_eq!(game.life, 22);
    let mut ungranted_payment = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        ..Default::default()
    };
    cast_action(&cards, &mut game, &mut ungranted_payment);
    assert_eq!(ungranted_payment.fixed[1], 1);
    assert_eq!(game.replay_casts, 1);
    assert!(game.flashback_permissions.is_empty());
    assert_library_rng(&game.library_rng, &rng);

    let mut game = state(vec![0], vec![]);
    move_to_graveyard(&cards, &mut game, CardIdx(1), 1, CardZone::Hand);
    move_to_graveyard(&cards, &mut game, CardIdx(3), 1, CardZone::Hand);
    put_permanents(&cards, &mut game, &[4, 5, 6, 7, 8]);
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.exile, [CardIdx(1)]);
    assert_eq!(game.graveyard, [CardIdx(3), CardIdx(0)]);
    assert_eq!(log.mana_spent, [5.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(log.replay_casts, 1);
    assert_eq!(log.card_first_graveyard.get(&1), Some(&1));
    assert_eq!(json["graveyard"]["replay_casts_avg"], 1.0);
    assert_eq!(json["milestones"]["graveyard_casts_avg_by_turn"]["1"], 1.0);
    assert_report_clause(
        &json,
        source,
        "Each instant and sorcery card in your graveyard gains flashback",
        "unsupported",
        None,
    );
}

/// A returned and discarded card needs its own printed route after its granted generation ends.
pub(super) fn assert_past_in_flames_generation_report(source: &CardRow) {
    pinned_revision(source);
    for (own_flashback, turns) in [(false, 1), (true, 1), (false, 2)] {
        let (cards, game) = past_generation_scenario(source, own_flashback);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, turns, vec![]);
        assert_eq!(game.graveyard_generations[&CardIdx(1)], 2);
        assert_eq!(game.graveyard_generations[&CardIdx(2)], 1);
        assert!(!game.flashback_permissions.contains(&(CardIdx(1), 2)));
        assert!(
            !game
                .flashback_permissions
                .iter()
                .any(|(index, _)| *index == CardIdx(2))
        );
        if turns == 2 {
            assert!(game.flashback_permissions.is_empty());
        }
        assert_eq!(
            game.exile,
            if own_flashback {
                vec![CardIdx(0), CardIdx(1)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(
            game.graveyard,
            if own_flashback {
                vec![CardIdx(2), CardIdx(3)]
            } else {
                vec![CardIdx(1), CardIdx(2), CardIdx(3)]
            }
        );
        assert_eq!(
            game.hand,
            if turns == 2 { vec![CardIdx(5)] } else { vec![] }
        );
        assert_eq!(
            game.library,
            if turns == 2 { vec![] } else { vec![CardIdx(5)] }
        );
        assert!(
            game.battlefield
                .iter()
                .find(|permanent| permanent.card == CardRef::Deck(CardIdx(4)))
                .expect("the paid return source remains on the board")
                .tapped
        );
        assert_eq!(game.life, if own_flashback { 22 } else { 20 });
        assert_eq!(log.card_first_seen.get(&1), Some(&1));
        assert_eq!(log.card_first_seen.get(&2), Some(&1));
        assert_eq!(log.card_first_graveyard.get(&1), Some(&1));
        assert_eq!(log.card_first_graveyard.get(&2), Some(&1));
        assert_eq!(
            log.cards_seen,
            if turns == 2 { vec![1, 2] } else { vec![1] }
        );
        assert_eq!(log.mana_available, vec![6.0; turns]);
        assert_eq!(
            log.mana_spent,
            if turns == 2 {
                vec![5.0, 0.0]
            } else {
                vec![if own_flashback { 11.0 } else { 5.0 }]
            }
        );
        assert_eq!(
            log.mana_remaining,
            vec![if own_flashback { 0.0 } else { 6.0 }; turns]
        );
        assert_eq!(log.replay_casts, 1 + u32::from(own_flashback));
        assert_eq!(
            log.milestones_by_turn[0].graveyard_casts,
            1 + u32::from(own_flashback)
        );
        assert_eq!(
            json["graveyard"]["replay_casts_avg"],
            if own_flashback { 2.0 } else { 1.0 }
        );
        assert_eq!(
            json["milestones"]["graveyard_casts_avg_by_turn"]["1"],
            if own_flashback { 2.0 } else { 1.0 }
        );
        assert_eq!(
            json["velocity"]["avg_cards_seen"][turns.to_string()],
            turns as f64
        );
        assert_report_clause(
            &json,
            source,
            "Each instant and sorcery card in your graveyard gains flashback",
            "unsupported",
            None,
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Use paid return, draw, and discard actions to create two later graveyard objects after the grant.
fn past_generation_scenario(source: &CardRow, own_flashback: bool) -> (SimDeck, GameState) {
    let target = row(
        "Physical replay target",
        "{6}",
        "Sorcery",
        if own_flashback {
            "Gain 2 life.\nFlashback {6} (You may cast this card from your graveyard for its flashback cost. Then exile it.)"
        } else {
            "Gain 2 life."
        },
    );
    let mut rows = vec![
        source.clone(),
        target.clone(),
        target,
        row(
            "Discard payment source",
            "{0}",
            "Sorcery",
            "As an additional cost to cast this spell, discard two cards.\nAdd {C}{C}{C}{C}{C}{C}.",
        ),
        row(
            "Paid return and draw",
            "{99}",
            "Artifact",
            "This artifact doesn't untap during your untap step.\n{1}, {T}: Return target card from your graveyard to your hand. Draw a card.",
        ),
        row("Next-turn library card", "{99}", "Sorcery", ""),
    ];
    rows.extend((0..5).map(|index| {
        row(
            &format!("Generation generic source {index}"),
            "",
            "Land",
            "{T}: Add {C}.",
        )
    }));
    rows.push(row("Generation red source", "", "Land", "{T}: Add {R}."));
    let cards = deck(&rows);
    assert!(cards.cards[0].spell_data.grants_flashback);
    assert_eq!(
        cards.cards[0]
            .spell_data
            .own_flashback
            .expect("the pinned source has its printed flashback route")
            .total(),
        5
    );
    assert_eq!(cards.cards[3].spell_data.additional_cost_discards, 2);
    assert!(cards.cards[4].flags.doesnt_untap);
    let mut game = state(vec![3], vec![5, 2]);
    move_to_graveyard(&cards, &mut game, CardIdx(1), 1, CardZone::Hand);
    move_to_graveyard(&cards, &mut game, CardIdx(0), 1, CardZone::Hand);
    put_permanents(&cards, &mut game, &[4, 6, 7, 8, 9, 10, 11]);
    (cards, game)
}

/// Breach grants a three-other-card Escape route, not a printed own-Escape route.
pub(super) fn assert_breach_granted_escape_report(source: &CardRow) {
    pinned_revision(source);
    let mut rows = vec![
        source.clone(),
        row("Escaped physical spell", "{U}", "Instant", "Draw a card."),
        row("Escape draw", "{99}", "Sorcery", ""),
        row("Escape fuel one", "", "Land", ""),
        row("Escape fuel two", "", "Land", ""),
        row("Escape fuel three", "", "Land", ""),
        row("Unescaped land", "", "Land", ""),
        row("Escape remaining draw", "{99}", "Sorcery", ""),
    ];
    rows.extend([
        row("Breach red", "", "Basic Land - Mountain", "{T}: Add {R}."),
        row("Breach blue", "", "Basic Land - Island", "{T}: Add {U}."),
        row("Breach generic", "", "Basic Land - Wastes", "{T}: Add {C}."),
    ]);
    let cards = deck(&rows);
    assert!(cards.cards[0].spell_data.grants_escape);
    assert!(cards.cards[1].spell_data.own_escape.is_none());
    for (source_live, fuel, mana) in [(false, 3, 1), (true, 2, 1), (true, 3, 0)] {
        let mut game = main_phase_state(vec![], vec![7, 2]);
        if source_live {
            put_permanents(&cards, &mut game, &[0]);
        }
        for index in std::iter::once(1).chain(3..3 + fuel) {
            move_to_graveyard(&cards, &mut game, CardIdx(index), 1, CardZone::Hand);
        }
        let before = game.graveyard.clone();
        let rng = game.library_rng.clone();
        let mut pool = ManaPool {
            fixed: [0, mana, 0, 0, 0],
            ..Default::default()
        };
        cast_action(&cards, &mut game, &mut pool);
        assert_eq!(pool.fixed[1], mana);
        assert_eq!(game.graveyard, before);
        assert_eq!(game.library, [CardIdx(7), CardIdx(2)]);
        assert!(game.exile.is_empty());
        assert_eq!(game.replay_casts, 0);
        assert_library_rng(&game.library_rng, &rng);
    }
    let mut game = state(vec![0], vec![7, 2]);
    for index in [1, 3, 4, 5, 6] {
        move_to_graveyard(&cards, &mut game, CardIdx(index), 1, CardZone::Hand);
    }
    put_permanents(&cards, &mut game, &[8, 9, 10]);
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(game.exile, [CardIdx(3), CardIdx(4), CardIdx(5)]);
    assert_eq!(
        game.graveyard
            .iter()
            .filter(|index| **index == CardIdx(1))
            .count(),
        1
    );
    assert!(
        game.graveyard.contains(&CardIdx(6)),
        "lands cannot use the grant"
    );
    assert_eq!(game.graveyard, [CardIdx(6), CardIdx(1)]);
    assert!(
        game.battlefield
            .iter()
            .any(|perm| perm.card == CardRef::Deck(CardIdx(0))),
        "the unsupported end-step sacrifice remains a separate gap"
    );
    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!(game.library, [CardIdx(7)]);
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(log.replay_casts, 1);
    assert_eq!(log.cards_seen, [1]);
    assert_eq!(log.card_first_graveyard.get(&1), Some(&1));
    assert_eq!(json["graveyard"]["replay_casts_avg"], 1.0);
    assert_eq!(json["milestones"]["graveyard_casts_avg_by_turn"]["1"], 1.0);
    assert_report_clause(
        &json,
        source,
        "Each nonland card in your graveyard has escape",
        "unsupported",
        None,
    );
    assert_library_rng(&game.library_rng, &rng);
}

/// Cauldron returns pre-cost creature targets under normal entry rules across repeat visits.
pub(super) fn assert_cauldron_creature_return_report(source: &CardRow) {
    pinned_revision(source);
    let mut rows = vec![
        source.clone(),
        row("Cauldron first sacrifice", "{99}", "Creature", ""),
        row(
            "Cauldron tapped return target",
            "{99}",
            "Creature",
            "This creature enters tapped.\nWhen this creature enters, gain 2 life.",
        ),
        row("Cauldron wrong target", "{99}", "Artifact", ""),
        row("Cauldron second-turn draw", "{99}", "Sorcery", ""),
        row("Cauldron third-turn draw", "{99}", "Sorcery", ""),
    ];
    rows.extend([
        row("Cauldron black", "", "Basic Land - Swamp", "{T}: Add {B}."),
        row("Cauldron green", "", "Basic Land - Forest", "{T}: Add {G}."),
        row(
            "Cauldron generic",
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        ),
    ]);
    let cards = deck(&rows);
    let mut game = state(vec![], vec![5, 4]);
    put_permanents(&cards, &mut game, &[0, 1, 6, 7, 8]);
    move_to_graveyard(&cards, &mut game, CardIdx(2), 1, CardZone::Hand);
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(&cards, game, 3, vec![]);
    let returned = game
        .battlefield
        .iter()
        .find(|perm| perm.card == CardRef::Deck(CardIdx(2)))
        .expect("the selected pre-cost creature returns on turns one and three");
    assert_eq!(returned.entered_turn, 3);
    assert!(returned.tapped);
    assert!(returned.summoning_sick);
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(game.hand, [CardIdx(4), CardIdx(5)]);
    assert!(game.library.is_empty());
    assert_eq!(game.life, 27);
    assert_eq!(game.graveyard_generations[&CardIdx(2)], 2);
    assert_eq!(log.card_first_graveyard.get(&2), Some(&1));
    assert_eq!(log.card_first_battlefield.get(&2), Some(&1));
    assert!(!log.card_first_seen.contains_key(&2));
    assert_eq!(log.replay_casts, 0);
    assert_eq!(log.mana_spent, [0.0, 0.0, 0.0]);
    assert_eq!(log.mana_remaining, [0.0, 0.0, 0.0]);
    assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
    assert_eq!(json["milestones"]["graveyard_casts_avg_by_turn"]["3"], 0.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["3"], 1.0);
    assert_report_clause(
        &json,
        source,
        "Return target creature card from your graveyard to the battlefield",
        "approximated",
        Some("pilot_card_selection"),
    );
    assert_library_rng(&game.library_rng, &rng);

    for target in [None, Some(3)] {
        let mut rejected = main_phase_state(vec![], vec![]);
        put_permanents(&cards, &mut rejected, &[0, 1]);
        if let Some(index) = target {
            move_to_graveyard(&cards, &mut rejected, CardIdx(index), 1, CardZone::Hand);
        }
        let pool = ManaPool {
            fixed: [0, 0, 1, 0, 1],
            colorless: 1,
            ..Default::default()
        };
        assert!(
            pick_best_activation(&cards, &rejected, &pool, 1).is_none(),
            "a sacrificed creature cannot supply a missing pre-cost target"
        );
    }
    for (sacrifice, tapped, black) in [(false, false, 1), (true, true, 1), (true, false, 0)] {
        let mut rejected = main_phase_state(vec![], vec![]);
        put_permanents(
            &cards,
            &mut rejected,
            if sacrifice { &[0, 1] } else { &[0] },
        );
        rejected.battlefield[0].tapped = tapped;
        move_to_graveyard(&cards, &mut rejected, CardIdx(2), 1, CardZone::Hand);
        let pool = ManaPool {
            fixed: [0, 0, black, 0, 1],
            colorless: 2 - black,
            ..Default::default()
        };
        assert!(pick_best_activation(&cards, &rejected, &pool, 1).is_none());
        assert_eq!(rejected.graveyard, [CardIdx(2)]);
    }
}

/// Bala Ged Recovery returns one physical card of any type to hand, not a filtered subset.
pub(super) fn assert_bala_ged_any_card_return_report(source: &CardRow) {
    pinned_revision(source);
    for kind in ["Artifact", "Creature", "Land"] {
        let cards = deck(&[
            source.clone(),
            row(
                "Recovery target",
                if kind == "Land" { "" } else { "{99}" },
                kind,
                "",
            ),
            row("Ordinary land choice", "", "Land", ""),
            row(
                "Recovery green one",
                "",
                "Basic Land - Forest",
                "{T}: Add {G}.",
            ),
            row(
                "Recovery green two",
                "",
                "Basic Land - Forest",
                "{T}: Add {G}.",
            ),
            row(
                "Recovery green three",
                "",
                "Basic Land - Forest",
                "{T}: Add {G}.",
            ),
        ]);
        let mut rejected = main_phase_state(vec![0], vec![]);
        rejected.exile.push(CardIdx(1));
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 0, 3],
            ..Default::default()
        };
        cast_action(&cards, &mut rejected, &mut pool);
        assert_eq!(pool.fixed[4], 3);
        assert_eq!(rejected.hand, [CardIdx(0)]);
        assert_eq!(rejected.exile, [CardIdx(1)]);
        assert!(rejected.graveyard.is_empty());
        let mut unpaid = main_phase_state(vec![0], vec![]);
        move_to_graveyard(&cards, &mut unpaid, CardIdx(1), 1, CardZone::Hand);
        let mut wrong_color = ManaPool {
            colorless: 3,
            ..Default::default()
        };
        cast_action(&cards, &mut unpaid, &mut wrong_color);
        assert_eq!(wrong_color.colorless, 3);
        assert_eq!(unpaid.hand, [CardIdx(0)]);
        assert_eq!(unpaid.graveyard, [CardIdx(1)]);
        assert!(unpaid.battlefield.is_empty());
        let mut game = state(vec![0, 2], vec![]);
        move_to_graveyard(&cards, &mut game, CardIdx(1), 1, CardZone::Hand);
        put_permanents(&cards, &mut game, &[3, 4, 5]);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert!(
            !game
                .battlefield
                .iter()
                .any(|perm| perm.card == CardRef::Deck(CardIdx(1)))
        );
        assert_eq!(log.mana_spent, [3.0]);
        assert_eq!(log.card_first_seen.get(&1), Some(&1));
        assert_eq!(log.card_first_graveyard.get(&1), Some(&1));
        assert_eq!(log.replay_casts, 0);
        assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 1.0);
        assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
        assert_report_clause(
            &json,
            source,
            "Return target card from your graveyard to your hand.",
            "compiler_only",
            None,
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Eternal Witness can return a land; a missing entry target does not block its creature cast.
pub(super) fn assert_witness_land_return_report(source: &CardRow) {
    pinned_revision(source);
    for target_present in [false, true] {
        let cards = deck(&[
            source.clone(),
            row("Witness land target", "", "Land", ""),
            row("Witness ordinary land", "", "Land", ""),
            row(
                "Witness green one",
                "",
                "Basic Land - Forest",
                "{T}: Add {G}.",
            ),
            row(
                "Witness green two",
                "",
                "Basic Land - Forest",
                "{T}: Add {G}.",
            ),
            row(
                "Witness green three",
                "",
                "Basic Land - Forest",
                "{T}: Add {G}.",
            ),
        ]);
        let mut game = state(vec![0, 2], vec![]);
        let mut unpaid = main_phase_state(vec![0], vec![]);
        move_to_graveyard(&cards, &mut unpaid, CardIdx(1), 1, CardZone::Hand);
        let mut wrong_color = ManaPool {
            colorless: 3,
            ..Default::default()
        };
        cast_action(&cards, &mut unpaid, &mut wrong_color);
        assert_eq!(wrong_color.colorless, 3);
        assert_eq!(unpaid.hand, [CardIdx(0)]);
        assert_eq!(unpaid.graveyard, [CardIdx(1)]);
        assert!(unpaid.battlefield.is_empty());
        if target_present {
            move_to_graveyard(&cards, &mut game, CardIdx(1), 1, CardZone::Hand);
        } else {
            game.exile.push(CardIdx(1));
        }
        put_permanents(&cards, &mut game, &[3, 4, 5]);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(
            game.hand,
            if target_present {
                vec![CardIdx(1)]
            } else {
                vec![]
            }
        );
        assert!(game.graveyard.is_empty());
        assert!(
            game.battlefield
                .iter()
                .any(|perm| perm.card == CardRef::Deck(CardIdx(0)))
        );
        assert!(
            !game
                .battlefield
                .iter()
                .any(|perm| perm.card == CardRef::Deck(CardIdx(1)))
        );
        assert_eq!(log.card_first_seen.get(&1), target_present.then_some(&1));
        assert_eq!(log.mana_spent, [3.0]);
        assert_eq!(log.replay_casts, 0);
        assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 0.0);
        assert_eq!(json["graveyard"]["replay_casts_avg"], 0.0);
        assert_report_clause(
            &json,
            source,
            "return target card from your graveyard to your hand",
            "approximated",
            Some("pilot_card_selection"),
        );
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Execute the pinned own-flashback clause independently of granted routes.
#[test]
fn faithless_looting_own_flashback_clause() {
    assert_faithless_looting_flashback_report(&named_row("Faithless Looting"));
}

/// Execute the current-instance grant without claiming all Past in Flames rules.
#[test]
fn past_in_flames_current_instance_clause() {
    assert_past_in_flames_instance_report(&named_row("Past in Flames"));
}

/// Generation-scoped grants cannot follow a physical card through hand and a new discard.
#[test]
fn past_in_flames_generation_clause() {
    assert_past_in_flames_generation_report(&named_row("Past in Flames"));
}

/// Execute Breach's grant without inventing an own-three-card Escape source.
#[test]
fn underworld_breach_granted_escape_clause() {
    assert_breach_granted_escape_report(&named_row("Underworld Breach"));
}

/// Execute Cauldron's paid return and repeated first-zone access.
#[test]
fn cauldron_pre_cost_creature_return_clause() {
    assert_cauldron_creature_return_report(&named_row("Cauldron of Essence"));
}

/// Execute Bala Ged Recovery's unfiltered hand-return clause.
#[test]
fn bala_ged_recovery_any_card_clause() {
    assert_bala_ged_any_card_return_report(&named_row("Bala Ged Recovery // Bala Ged Sanctuary"));
}

/// Execute the land case missing from the basic artifact-return fixture.
#[test]
fn eternal_witness_land_target_clause() {
    assert_witness_land_return_report(&named_row("Eternal Witness"));
}
