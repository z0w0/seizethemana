//! Degenerate mana-base edges for the game loop: zero lands, all-tapland
//! bases, and a curve the base cannot pay for. These pin the loop's
//! behavior where the calibration sweeps (mana_base_tests) never go.

use super::deck::build_sim_deck;
use super::game::run_game;
use crate::db::CardRow;
use crate::deck::grammar::{Deck, DeckEntry};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

/// A land card row; `tapped` marks a tapland via the oracle text.
fn land_row(name: &str, tapped: bool) -> (String, CardRow) {
    (
        name.to_string(),
        CardRow {
            name: name.to_string(),
            oracle_id: "oid".into(),
            mana_cost: String::new(),
            cmc: 0.0,
            type_line: "Basic Land — Test".to_string(),
            colors: "[]".into(),
            color_identity: "[]".into(),
            keywords: "[]".into(),
            power: None,
            toughness: None,
            loyalty: None,
            oracle_text: if tapped {
                "Enters tapped. {T}: Add {U}.".to_string()
            } else {
                "{T}: Add {U}.".to_string()
            },
            rarity: "basic".into(),
            edhrec_rank: None,
            legalities: "{}".into(),
            set_code: "tst".into(),
            collector_number: "1".into(),
            scryfall_id: format!("sid-{name}"),
            released_at: "2020-01-01".into(),
            game_changer: None,
            penny_rank: None,
            reserved: None,
        },
    )
}

/// A two-mana filler spell row.
fn spell_row(name: &str) -> (String, CardRow) {
    (
        name.to_string(),
        CardRow {
            name: name.to_string(),
            oracle_id: "oid".into(),
            mana_cost: "{2}".into(),
            cmc: 2.0,
            type_line: "Instant".to_string(),
            colors: "[]".into(),
            color_identity: "[]".into(),
            keywords: "[]".into(),
            power: None,
            toughness: None,
            loyalty: None,
            oracle_text: "Do nothing.".to_string(),
            rarity: "common".into(),
            edhrec_rank: None,
            legalities: "{}".into(),
            set_code: "tst".into(),
            collector_number: "1".into(),
            scryfall_id: format!("sid-{name}"),
            released_at: "2020-01-01".into(),
            game_changer: None,
            penny_rank: None,
            reserved: None,
        },
    )
}

/// A commander deck of `lands` (name, tapped) plus 2-mana filler.
fn deck_with_base(lands: &[(&str, bool)], n_spells: usize) -> (Deck, HashMap<String, CardRow>) {
    let mut deck = Deck::default();
    deck.section_entries_mut("COMMANDER").push(DeckEntry {
        quantity: 1,
        name: "Test Commander".into(),
        set_code: None,
        collector_number: None,
        foil: false,
    });
    for (land, _) in lands {
        deck.section_entries_mut("DECK").push(DeckEntry {
            quantity: 1,
            name: (*land).into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    for _ in 0..n_spells {
        deck.section_entries_mut("DECK").push(DeckEntry {
            quantity: 1,
            name: "Filler Spell".into(),
            set_code: None,
            collector_number: None,
            foil: false,
        });
    }
    let cards: HashMap<String, CardRow> = lands
        .iter()
        .map(|(name, tapped)| land_row(name, *tapped))
        .chain([spell_row("Filler Spell")])
        .collect();
    (deck, cards)
}

#[test]
fn zero_land_deck_casts_nothing_but_runs() {
    // No mana sources: every turn ends with an empty pool; the loop
    // still completes without panicking.
    let (deck, cards) = deck_with_base(&[], 98);
    let sim_deck = build_sim_deck(&deck, &cards, Some("commander"));
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs: Vec<_> = (0..30).map(|_| run_game(&sim_deck, &mut rng, 6)).collect();
    for log in &logs {
        for mana in &log.mana_available {
            assert_eq!(*mana, 0.0, "no land: no mana any turn");
        }
        for drops in &log.land_drops {
            assert_eq!(*drops, 0, "no land: no drops possible");
        }
    }
}

#[test]
fn all_tapland_deck_ramps_one_turn_behind() {
    // Every land enters tapped, so turn 1 has no pool: first usable mana
    // is turn 2. Cards still flow (the commander engine and draws), so
    // the seen census must clear the opener by late turns.
    let lands = vec![("Tapland Island", true)];
    let (deck, cards) = deck_with_base(&lands, 98);
    let sim_deck = build_sim_deck(&deck, &cards, Some("commander"));
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs: Vec<_> = (0..100)
        .map(|_| run_game(&sim_deck, &mut rng, 10))
        .collect();
    for log in &logs {
        assert_eq!(log.mana_available[0], 0.0, "tapland: turn 1 is dry");
    }
    let stats = super::aggregate::aggregate(&logs, &sim_deck, 10);
    assert!(
        stats.unused_mana[1] > 0.0,
        "taplands start paying from turn 2: turn2 {:.1}",
        stats.unused_mana[1]
    );
}

#[test]
fn mixed_tapland_base_beats_the_all_tapland_base() {
    // Untapped lands pay a turn earlier: by turn 2 the all-untapped base
    // must have a live pool while the all-tapland base is still dry, and
    // turn 1 must be dry only for the tapland base.
    let untapped = vec![("Island", false)];
    let (deck_u, cards_u) = deck_with_base(&untapped, 98);
    let sim_untapped = build_sim_deck(&deck_u, &cards_u, Some("commander"));
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs_untapped: Vec<_> = (0..100)
        .map(|_| run_game(&sim_untapped, &mut rng, 10))
        .collect();
    let stats_untapped = super::aggregate::aggregate(&logs_untapped, &sim_untapped, 10);
    assert!(
        stats_untapped.unused_mana[0] > 0.0,
        "untapped lands pay from turn 1: {:.1}",
        stats_untapped.unused_mana[0]
    );

    let tapped = vec![("Tapland Island", true)];
    let (deck_t, cards_t) = deck_with_base(&tapped, 98);
    let sim_tapped = build_sim_deck(&deck_t, &cards_t, Some("commander"));
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs_tapped: Vec<_> = (0..100)
        .map(|_| run_game(&sim_tapped, &mut rng, 10))
        .collect();
    for log in &logs_tapped {
        assert_eq!(log.mana_available[0], 0.0, "tapland: turn 1 is dry");
    }
    let stats_tapped = super::aggregate::aggregate(&logs_tapped, &sim_tapped, 10);
    // The untapped base floats mana on turn 1; the tapland base does not.
    assert!(
        stats_untapped.unused_mana[0] > stats_tapped.unused_mana[0],
        "untapped base pays on turn 1 and the tapland base does not ({} vs {})",
        stats_untapped.unused_mana[0],
        stats_tapped.unused_mana[0]
    );
}
