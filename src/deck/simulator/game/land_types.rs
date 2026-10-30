//! Basic land types on the controlled physical battlefield faces.

use super::{Permanent, card_of, is_land_permanent};
use crate::deck::simulator::model::SimDeck;

/// Collect basic land types from face-up lands, including tapped lands.
pub(in crate::deck::simulator) fn controlled_land_types(
    deck: &SimDeck,
    battlefield: &[Permanent],
) -> [bool; 5] {
    let mut types = [false; 5];
    for permanent in battlefield {
        if !is_land_permanent(permanent) {
            continue;
        }
        let card = card_of(deck, permanent);
        if !card.is_mdfc_spell && !card.card_types.iter().any(|kind| kind == "land") {
            continue;
        }
        for (controlled, printed) in types.iter_mut().zip(card.land_types) {
            *controlled |= printed;
        }
    }
    types
}

/// Test whether any named basic land type is on a controlled land.
pub(in crate::deck::simulator) fn controls_land_types(
    deck: &SimDeck,
    battlefield: &[Permanent],
    required: [bool; 5],
) -> bool {
    controlled_land_types(deck, battlefield)
        .into_iter()
        .zip(required)
        .any(|(controlled, required)| controlled && required)
}
