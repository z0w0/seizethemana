//! Cut scoring uses the same per-game random streams as normal simulation.

use super::*;
use crate::deck::simulator::{aggregate::SimAccumulator, format, game, model::*, rng};

/// Use a mixed deck so random opening hands change mana-readiness measurements.
fn simulation_deck() -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..18 {
        cards.push(SimCard {
            name: "Island".to_string(),
            role: crate::deck::simulator::model::Role::Land,
            tap: Some(ManaYield {
                fixed: [0, 1, 0, 0, 0],
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    for mana in [2, 4, 7] {
        for _ in 0..6 {
            let cost = Cost {
                generic: mana,
                ..Default::default()
            };
            cards.push(SimCard {
                name: format!("Spell {mana}"),
                cost,
                min_cost: cost,
                has_mana_cost: true,
                is_instant_or_sorcery: true,
                ..Default::default()
            });
        }
    }
    SimDeck {
        cards,
        commanders: vec![],
        companion: None,
        format: Format::Constructed,
        rules: format::rules_for("modern"),
    }
}

#[test]
fn cut_readiness_matches_versioned_per_game_streams() {
    let deck = simulation_deck();
    let actual = castability_stats(&deck).expect("stream cut measurements");
    let mut expected = SimAccumulator::new(&deck, deck.rules.default_turns);
    for game_id in 0..2000 {
        let mut random = rng::game_rng(42, game_id, rng::RngDomain::Opening);
        let log = game::run_game(&deck, &mut random, deck.rules.default_turns);
        expected.push(&log);
    }
    let expected = expected.finish();
    assert_eq!(actual.runs, 2000);
    assert_eq!(actual.opener_pct, expected.opener_pct);
    assert_eq!(actual.mana_available, expected.mana_available);
    assert_eq!(actual.p50_drops_by_4, expected.p50_drops_by_4);
    for (actual, expected) in actual
        .card_castability
        .iter()
        .zip(&expected.card_castability)
    {
        assert_eq!(actual.name, expected.name);
        assert_eq!(actual.pct_by_target, expected.pct_by_target);
        assert_eq!(actual.payable_games, expected.payable_games);
        assert_eq!(
            actual.avg_first_castable_turn,
            expected.avg_first_castable_turn
        );
    }
}

/// Missing horizon coverage must not become evidence that a card cannot be paid for.
#[test]
fn cuts_do_not_penalize_unobserved_readiness_deadlines() {
    let directory = tempfile::tempdir().expect("temporary card database");
    let connection =
        crate::db::open(&directory.path().join("cards.db")).expect("open card database");
    for (name, cost, mana_value, type_line, text) in [
        ("Island", "", 0, "Basic Land - Island", "{T}: Add {U}."),
        ("Cheap", "{2}", 2, "Creature", ""),
        ("Beyond horizon", "{100}", 100, "Creature", ""),
    ] {
        connection
            .execute(
                "INSERT INTO cards (name, oracle_id, mana_cost, cmc, type_line, colors,
                color_identity, keywords, oracle_text, rarity, legalities, set_code,
                collector_number, scryfall_id, released_at)
             VALUES (?1, ?1, ?2, ?3, ?4, '[]', '[]', '[]', ?5, 'common',
                '{\"modern\":\"legal\"}', 'tst', '1', ?1, '2020-01-01')",
                rusqlite::params![name, cost, mana_value, type_line, text],
            )
            .expect("insert fixture card");
    }
    let deck = super::super::Deck::parse("// DECK\n24 Island\n20 Cheap\n1 Beyond horizon\n")
        .expect("parse fixture deck");
    let cards = super::super::stats::lookup_names(&connection, &deck).expect("load fixture cards");
    let rows = cut_rows(
        &connection,
        &deck,
        &cards,
        None,
        20,
        None,
        Some("modern"),
        None,
    )
    .expect("score fixture cuts");
    let costly = rows
        .iter()
        .find(|row| row.name == "Beyond horizon")
        .expect("curve outlier remains visible");
    assert!(costly.reasons.iter().any(|reason| reason.kind == "curve"));
    assert!(
        costly
            .reasons
            .iter()
            .all(|reason| reason.kind != "castability")
    );
}
