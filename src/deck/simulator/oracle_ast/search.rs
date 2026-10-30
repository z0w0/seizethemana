//! Complete multiple-result search instructions before runtime lowering.

use crate::deck::simulator::model::{SearchCardType, SearchDestination};

/// One search that selects distinct cards and assigns every result to a listed destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleMultiSearch {
    /// Required type and supertype of every selected card.
    pub card_type: SearchCardType,
    /// Allowed basic land subtypes, in WUBRG order.
    pub land_types: [bool; 5],
    /// Fewest cards the player may choose.
    pub min_cards: u32,
    /// Most cards the player may choose.
    pub max_cards: u32,
    /// Whether all selected cards must be revealed before placement.
    pub reveal: bool,
    /// Destination capacities; fewer found cards may leave a destination unused.
    pub destinations: Vec<OracleSearchPlacement>,
    /// Shuffle after all placements, including any cards returned to the library.
    pub shuffle: bool,
}

/// The maximum number of selected cards that may use one destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSearchPlacement {
    /// Required final zone and any tapped-entry instruction.
    pub destination: SearchDestination,
    /// Maximum results assigned here; a physical card cannot occupy two destinations.
    pub max_cards: u32,
}
