use super::*;
use crate::deck::simulator::{
    format,
    model::{Format, SimCard},
    rng::{RngDomain, game_rng},
};

/// London bottoming records each selected physical card and preserves the deck.
#[test]
fn redraws_and_bottomed_cards_are_explicit() {
    let deck = SimDeck {
        cards: (0..12)
            .map(|index| SimCard {
                name: format!("Spell {index}"),
                ..SimCard::default()
            })
            .collect(),
        commanders: Vec::new(),
        companion: None,
        format: Format::Constructed,
        rules: format::rules_for("constructed"),
    };
    let opener = deal_opener(&deck, &mut game_rng(42, 0, RngDomain::Opening));
    assert_eq!(opener.redraws, 6);
    assert_eq!(opener.bottomed.len(), 6);
    assert_eq!(opener.hand.len(), 1);
    assert!(opener.mulliganed);
    let mut cards = opener
        .hand
        .iter()
        .chain(&opener.library)
        .map(|index| index.0)
        .collect::<Vec<_>>();
    cards.sort_unstable();
    assert_eq!(cards, (0..12).collect::<Vec<_>>());
    assert_eq!(
        opener.library[..6]
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>(),
        opener.bottomed
    );
}

/// The pilot's free multiplayer redraw does not bottom any cards.
#[test]
fn free_redraw_keeps_all_seven_cards() {
    let deck = SimDeck {
        cards: vec![SimCard::default(); 12],
        commanders: Vec::new(),
        companion: None,
        format: Format::Commander,
        rules: format::rules_for("commander"),
    };
    let opener = deal_opener(&deck, &mut game_rng(42, 0, RngDomain::Opening));
    assert_eq!(opener.redraws, 1);
    assert!(opener.bottomed.is_empty());
    assert_eq!(opener.hand.len(), 7);
}
