//! Source-pinned MDFCs keep hidden spell text out of battlefield entry and cleanup.

use crate::db::CardRow;
use crate::deck::simulator::card_report_tests::{execute_turn, report_executed_turn};
use crate::deck::simulator::card_runtime_tests::attempt_index;
use crate::deck::simulator::card_test_support::{
    assert_library_rng, byte_hash, main_phase_state, named_row, pinned_revision,
};
use crate::deck::simulator::cast_pass::play_land;
use crate::deck::simulator::deck_test_support::fixture_cards;
use crate::deck::simulator::game::{
    CardRef, GameState, condition_met, is_creature_permanent, is_land_permanent, new_perm_with,
};
use crate::deck::simulator::game_run::mana::build_pool;
use crate::deck::simulator::model::{CardIdx, SimAbilityCondition, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use serde_json::Value;

/// The preserved deck fixture owns this exact combined card name.
const SEA_GATE_NAME: &str = "Sea Gate Restoration // Sea Gate, Reborn";

/// Pin the exact legacy simulation inputs, including its absent Oracle identity.
const SEA_GATE_SOURCE_HASH: &str =
    "2604dadfe0b80c0e699220b13e376b6258edcfa5f48dd4f56bd9f63942f79e29";

/// Load the unchanged source fixture rather than synthesizing the reported card.
fn sea_gate_source() -> CardRow {
    let bytes = include_bytes!("../../tests/deck_fixtures/merfolk_modern.json");
    assert_eq!(
        byte_hash(bytes),
        "e30990bf63b46d7f3bccbe48ebeeb3cceb2b464ced8b17f0fa254a8bb57bc5f5"
    );
    let source = fixture_cards("merfolk modern")
        .remove(SEA_GATE_NAME)
        .expect("the pinned Merfolk fixture contains Sea Gate Restoration");
    assert_eq!(source.type_line, "Sorcery // Land");
    assert_eq!(source.mana_cost, "{4}{U}{U}{U} // ");
    assert_eq!(source.cmc, 7.0);
    assert_eq!(
        source.oracle_text,
        "Draw cards equal to the number of cards in your hand plus one. You have no maximum hand size for the rest of the game.\n// As this land enters, you may pay 3 life. If you don't, it enters tapped.\n{T}: Add {U}."
    );
    assert_eq!(
        crate::deck::simulator::support::card_source_hash(&source),
        SEA_GATE_SOURCE_HASH
    );
    source
}

/// Retain source wording, input hash, and the unsupported lasting spell-effect limit.
fn assert_sea_gate_report(report: &Value) {
    let record = report["card_support"]
        .as_array()
        .expect("compiler records")
        .iter()
        .find(|record| record["name"] == SEA_GATE_NAME)
        .expect("the original source remains in the report");
    assert_eq!(record["source_hash"], SEA_GATE_SOURCE_HASH);
    let ability = record["abilities"]
        .as_array()
        .expect("source ability records")
        .iter()
        .find(|ability| {
            ability["source"].as_str().is_some_and(|text| {
                text.contains("You have no maximum hand size for the rest of the game.")
            })
        })
        .expect("the lasting spell clause remains visible");
    assert_eq!(ability["status"], "unsupported");
    assert!(
        !ability["reason"]
            .as_str()
            .expect("the unsupported reason is explicit")
            .is_empty()
    );
}

/// Keep each over-limit hand card physical and too expensive to cast during cleanup tests.
fn hand_rows(source: CardRow) -> Vec<CardRow> {
    let mut rows = vec![source];
    rows.extend((1..=9).map(|index| {
        row(
            &format!("Physical hand card {index}"),
            "{99}",
            "Sorcery",
            "",
        )
    }));
    rows
}

/// The real land face must pay its entry cost and discard from an eight-card hand.
#[test]
fn sea_gate_land_face_does_not_grant_the_hidden_hand_size_effect() {
    let cards = deck(&hand_rows(sea_gate_source()));
    assert!(cards.cards[0].flags.no_max_hand_size);
    assert!(!cards.cards[0].is_permanent);
    let mut game = main_phase_state((0..=8).collect(), vec![9]);
    assert!(play_land(&cards, &mut game, 1));
    assert_eq!(game.hand, (1..=8).map(CardIdx).collect::<Vec<_>>());
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert!(is_land_permanent(&game.battlefield[0]));
    assert!(!is_creature_permanent(&cards, &game.battlefield[0]));
    assert!(!game.battlefield[0].tapped);
    assert_eq!((game.life, game.life_paid), (17, 3));

    let mut game = state((0..=8).collect(), vec![9]);
    let random = game.library_rng.clone();
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.hand, (1..=7).map(CardIdx).collect::<Vec<_>>());
    assert_eq!(game.graveyard, [CardIdx(8)]);
    assert_eq!(game.library, [CardIdx(9)]);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!((game.life, game.life_paid), (17, 3));
    assert_library_rng(&game.library_rng, &random);
    let (log, report) = report_executed_turn(&cards, game, census);
    assert_eq!(log.land_drops, [1]);
    assert_eq!(log.cards_seen, [0]);
    assert_eq!(log.creatures, [0]);
    assert_eq!(log.card_first_battlefield.get(&0), Some(&1));
    assert_eq!(log.card_first_graveyard.get(&8), Some(&1));
    assert_eq!(report["graveyard"]["avg_size_by_turn"]["1"], 1.0);
    assert_eq!(report["mana"]["avg_available_by_turn"]["1"], 1.0);
    assert_eq!(report["draw"]["avg_life_paid"], 3.0);
    assert_sea_gate_report(&report);
}

/// Visible static sources retain their rule; face-down and hidden-back text do not.
#[test]
fn cleanup_uses_only_visible_front_static_flags() {
    let controls = [
        (
            row(
                "Visible static source",
                "{99}",
                "Artifact",
                "You have no maximum hand size.",
            ),
            false,
            true,
        ),
        (
            row(
                "Face-down static source",
                "{99}",
                "Artifact",
                "You have no maximum hand size.",
            ),
            true,
            false,
        ),
        (
            row(
                "Front creature // Back land",
                "{99}",
                "Creature // Land",
                "You have no maximum hand size.\n//\n{T}: Add {U}.",
            ),
            false,
            true,
        ),
        (
            row(
                "Front land // Hidden spell",
                "",
                "Land // Sorcery",
                "{T}: Add {U}.\n//\nYou have no maximum hand size for the rest of the game.",
            ),
            false,
            false,
        ),
        (
            row(
                "Visible land source",
                "",
                "Land",
                "You have no maximum hand size.\n{T}: Add {C}.",
            ),
            false,
            true,
        ),
    ];
    for (source, hidden, keep_all) in controls {
        let cards = deck(&hand_rows(source));
        let mut game = state((1..=8).collect(), vec![9]);
        let mut permanent = new_perm_with(50, &cards, CardIdx(0), 0, false);
        permanent.face_down = hidden;
        game.battlefield.push(permanent);
        game.next_uid = 50;
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.hand,
            (1..=if keep_all { 8 } else { 7 })
                .map(CardIdx)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            game.graveyard,
            if keep_all { vec![] } else { vec![CardIdx(8)] }
        );
        assert_eq!(game.library, [CardIdx(9)]);
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.cards_seen, [0]);
        assert_eq!(
            report["graveyard"]["avg_size_by_turn"]["1"],
            f64::from(u32::from(!keep_all))
        );
    }
}

/// The front spell pays its printed cost; its lasting hand-size effect is still unsupported.
#[test]
fn sea_gate_front_spell_does_not_gain_new_duration_support() {
    let mut rows = vec![sea_gate_source()];
    rows.extend((1..=7).map(|_| row("Funding Island", "", "Basic Land — Island", "{T}: Add {U}.")));
    rows.extend((8..=16).map(|index| {
        row(
            &format!("Physical hand card {index}"),
            "{99}",
            "Sorcery",
            "",
        )
    }));
    let cards = deck(&rows);
    let mut game = state(vec![0, 7, 8, 9, 10, 11, 12, 13, 14, 15], vec![16]);
    for index in 1..=6 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 6;
    let random = game.library_rng.clone();
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.hand, (8..=14).map(CardIdx).collect::<Vec<_>>());
    assert_eq!(game.graveyard, [CardIdx(0), CardIdx(15)]);
    assert_eq!(game.library, [CardIdx(16)]);
    assert!(
        !game
            .battlefield
            .iter()
            .any(|permanent| permanent.card == CardRef::Deck(CardIdx(0)))
    );
    assert_eq!((game.life, game.life_paid), (20, 0));
    assert_library_rng(&game.library_rng, &random);
    let (log, report) = report_executed_turn(&cards, game, census);
    assert_eq!(log.mana_spent, [7.0]);
    assert_eq!(log.cards_seen, [0]);
    assert_eq!(report["graveyard"]["avg_size_by_turn"]["1"], 2.0);
    assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 7.0);
    assert_sea_gate_report(&report);
}

/// Put the rejected sorcery beside genuine front permanents in one paid reveal batch.
fn reveal_scenario() -> (SimDeck, GameState) {
    let witch = named_row("Witch Enchanter // Witch-Blessed Meadow");
    pinned_revision(&witch);
    let mut rows = vec![
        row(
            "Reveal batch",
            "{X}",
            "Sorcery",
            "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
        ),
        sea_gate_source(),
        witch,
        named_row("Deathcap Glade"),
        named_row("Deathcap Glade"),
        row("Plain batch land", "", "Land", ""),
        row("Excluded instant", "{99}", "Instant", ""),
        row("Excluded artifact", "{99}", "Artifact", ""),
        row("Physical entry draw", "{99}", "Sorcery", ""),
        row("Funding land", "", "Land", "{T}: Add {U}."),
    ];
    rows.extend((0..6).map(|_| row("Funding artifact", "", "Artifact", "{T}: Add {C}.")));
    rows.push(row(
        "Creature entry observer",
        "{99}",
        "Enchantment",
        "Whenever another creature you control enters, draw a card.",
    ));
    let cards = deck(&rows);
    let mut game = main_phase_state(vec![0], vec![8, 1, 2, 3, 4, 5, 6, 7]);
    for (uid, index) in (9..=16).enumerate() {
        game.battlefield.push(new_perm_with(
            100 + uid as u32,
            &cards,
            CardIdx(index),
            0,
            false,
        ));
    }
    game.next_uid = 107;
    (cards, game)
}

/// Check physical movements, one creature entry draw, and a shared pre-entry land snapshot.
fn assert_reveal_state(game: &GameState) {
    assert_eq!(game.hand, [CardIdx(8)]);
    assert!(game.library.is_empty());
    assert_eq!(
        game.graveyard,
        [CardIdx(7), CardIdx(6), CardIdx(1), CardIdx(0)]
    );
    assert_eq!(game.battlefield.len(), 12);
    assert!(
        !game
            .battlefield
            .iter()
            .any(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
    );
    let entered: Vec<_> = game
        .battlefield
        .iter()
        .skip(8)
        .map(|permanent| {
            (
                permanent.uid,
                permanent.card,
                permanent.tapped,
                permanent.land_face,
            )
        })
        .collect();
    assert_eq!(
        entered,
        [
            (108, CardRef::Deck(CardIdx(5)), false, true),
            (109, CardRef::Deck(CardIdx(4)), true, true),
            (110, CardRef::Deck(CardIdx(3)), true, true),
            (111, CardRef::Deck(CardIdx(2)), false, false),
        ]
    );
    assert_eq!((game.life, game.life_paid), (20, 0));
    assert!(!game.battlefield_seen.contains_key(&CardIdx(1)));
    assert_eq!(game.battlefield_seen.get(&CardIdx(2)), Some(&1));
    assert_eq!(game.graveyard_seen.get(&CardIdx(1)), Some(&1));
    assert!(!game.card_first_seen.contains_key(&1));
    assert_eq!(game.card_first_seen.get(&8), Some(&1));
    assert_eq!(game.seen, 8);
    assert!(!game.failed_draw);
}

/// A paid reveal rejects Sea Gate's sorcery front even when X covers its mana value.
#[test]
fn source_bound_reveal_rejects_the_sorcery_and_keeps_front_permanent_entries() {
    let (cards, mut game) = reveal_scenario();
    assert!(!cards.cards[1].is_permanent);
    assert!(cards.cards[2].is_permanent);
    let random = game.library_rng.clone();
    let mut pool = build_pool(&cards, &mut game, 1);
    assert_eq!(pool.total(), 7);
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        Some(7)
    );
    assert_eq!(pool.total(), 0);
    assert_reveal_state(&game);
    assert_library_rng(&game.library_rng, &random);

    let (cards, mut game) = reveal_scenario();
    let census = execute_turn(&cards, &mut game);
    assert_reveal_state(&game);
    assert_library_rng(&game.library_rng, &random);
    let (log, report) = report_executed_turn(&cards, game, census);
    assert_eq!(log.land_drops, [0]);
    assert_eq!(log.cards_seen, [8]);
    assert_eq!(log.creatures, [1]);
    assert_eq!(log.mana_spent, [7.0]);
    assert!(!log.card_first_battlefield.contains_key(&1));
    assert_eq!(log.card_first_graveyard.get(&1), Some(&1));
    assert_eq!(report["velocity"]["avg_cards_seen"]["1"], 8.0);
    assert_eq!(report["graveyard"]["avg_size_by_turn"]["1"], 4.0);
    assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 7.0);
    assert_eq!(report["mana"]["avg_unused_by_turn"]["1"], 0.0);
    assert_sea_gate_report(&report);
}

/// Graveyard permanent counts use the same front-face eligibility as a library reveal.
#[test]
fn descend_counts_front_permanents_not_a_sorcerys_land_back_face() {
    let cards = deck(&[
        sea_gate_source(),
        named_row("Witch Enchanter // Witch-Blessed Meadow"),
        row("Physical land", "", "Land", ""),
    ]);
    let mut game = state(vec![], vec![]);
    game.graveyard = vec![CardIdx(0), CardIdx(1)];
    assert!(condition_met(
        &cards,
        &game,
        &SimAbilityCondition::Descend(1)
    ));
    assert!(!condition_met(
        &cards,
        &game,
        &SimAbilityCondition::Descend(2)
    ));
    game.graveyard.push(CardIdx(2));
    assert!(condition_met(
        &cards,
        &game,
        &SimAbilityCondition::Descend(2)
    ));
}
