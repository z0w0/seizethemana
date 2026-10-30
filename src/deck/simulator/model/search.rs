//! Executable multiple-result library searches.

use super::{SearchCardType, SearchDestination};

/// One search with distinct results and bounded destination capacities.
#[derive(Debug, Clone)]
pub struct MultiSearchSpec {
    /// Required card type for every selected card.
    pub card_type: SearchCardType,
    /// Allowed basic land subtypes, in WUBRG order.
    pub land_types: [bool; 5],
    /// Fewest cards the search asks the player to choose.
    pub min_cards: u32,
    /// Maximum number of distinct cards the search permits.
    pub max_cards: u32,
    /// Reveal every selected card before placement.
    pub reveal: bool,
    /// Ordered placement choices and their capacities.
    pub destinations: Vec<SearchPlacement>,
    /// Shuffle after all placements, including any cards returned to the library.
    pub shuffle: bool,
}

/// The maximum number of found cards assigned to one final zone.
#[derive(Debug, Clone, Copy)]
pub struct SearchPlacement {
    /// Final zone, including forced tapped entry when stated.
    pub destination: SearchDestination,
    /// Maximum number of distinct cards assigned here.
    pub max_cards: u32,
}
