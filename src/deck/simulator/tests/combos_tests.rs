use crate::deck::simulator::combos::{candidates, measure};
use crate::deck::simulator::format::rules_for;
use crate::deck::simulator::game::{GameLog, run_game};
use crate::deck::simulator::model::{Cost, Format, Role, SimCard, SimDeck};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

fn combo_variant(
    id: &str,
    pieces: &[(&str, &str)],
    format: &str,
) -> (
    crate::spellbook::ComboVariant,
    Vec<crate::spellbook::ComboPieceRow>,
) {
    let legalities = std::collections::HashMap::from([(format.to_string(), true)]);
    (
        crate::spellbook::ComboVariant {
            id: id.to_string(),
            produces: vec!["Win the game".to_string()],
            mana_value_needed: 3,
            bracket_tag: Some("S".to_string()),
            popularity: Some(1000),
            legalities,
        },
        pieces
            .iter()
            .enumerate()
            .map(|(ordinal, (name, zones))| crate::spellbook::ComboPieceRow {
                name: name.to_string(),
                ordinal: ordinal as i64,
                zones: zones.split(',').map(str::to_string).collect(),
                must_be_commander: false,
            })
            .collect(),
    )
}

/// A spell deck with enough lands to keep a hand and play spells.
fn spell_deck(names: &[&str]) -> SimDeck {
    let mut cards: Vec<SimCard> = (0..24)
        .map(|_| SimCard {
            name: "Plains".into(),
            cost: Cost::default(),
            min_cost: Cost::default(),
            role: Role::Land,
            ..SimCard::default()
        })
        .collect();
    cards.extend(names.iter().map(|name| SimCard {
        name: (*name).to_string(),
        cost: Cost {
            generic: 2,
            ..Cost::default()
        },
        min_cost: Cost {
            generic: 2,
            ..Cost::default()
        },
        role: Role::Other,
        ..SimCard::default()
    }));
    SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: rules_for("constructed"),
    }
}

#[test]
fn hand_pair_assembles_by_target_turn() {
    // Both pieces are ordinary spells: hand-zone assembly. Some games
    // hold both by the target turn (two copies each among 36 spells).
    let mut names: Vec<&str> = vec!["Piece 0", "Piece 1"];
    names.extend(std::iter::repeat_n("Filler", 34));
    let deck = spell_deck(&names);
    let variants = vec![combo_variant(
        "1-2",
        &[("Piece 0", "H"), ("Piece 1", "H")],
        "modern",
    )];
    let (candidates, excluded) = candidates(variants, &deck, "modern");
    assert_eq!(excluded, 0);
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].complete);

    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let logs: Vec<GameLog> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let assembly = measure(&candidates, &logs, &deck, 8);
    assert_eq!(assembly.variants_considered, 1);
    let row = &assembly.complete[0];
    assert!(row.pct_games > 0.01, "hand pair never assembled: {row:?}");
    assert_eq!(row.target_turn, 3);
}

#[test]
fn near_miss_reports_missing_piece() {
    let deck = spell_deck(&["Piece 0"]);
    let variants = vec![combo_variant(
        "3-4",
        &[("Piece 0", "H"), ("Missing Piece", "B")],
        "modern",
    )];
    let (candidates, _) = candidates(variants, &deck, "modern");
    assert!(!candidates[0].complete);
    assert_eq!(candidates[0].missing, vec!["Missing Piece"]);
    let logs = Vec::new();
    // Empty logs measure nothing: the row still reports its missing
    // piece with 0% assembly.
    let assembly = measure(&candidates, &logs, &deck, 8);
    assert!(assembly.near_misses.is_empty());
    assert_eq!(candidates[0].missing, vec!["Missing Piece"]);
}

#[test]
fn format_illegal_variants_are_excluded() {
    let deck = spell_deck(&["Piece 0"]);
    // Legal only in commander: excluded for a modern deck.
    let variants = vec![combo_variant(
        "1-2",
        &[("Piece 0", "H"), ("Piece 1", "H")],
        "commander",
    )];
    let (candidates, excluded) = candidates(variants, &deck, "modern");
    assert!(candidates.is_empty());
    assert_eq!(excluded, 1);
}

#[test]
fn exile_zone_variant_is_excluded() {
    let deck = spell_deck(&["Piece 0"]);
    let variants = vec![combo_variant(
        "5-6",
        &[("Piece 0", "E"), ("Piece 1", "H")],
        "modern",
    )];
    let (candidates, excluded) = candidates(variants, &deck, "modern");
    assert!(candidates.is_empty());
    assert_eq!(excluded, 1);
}
