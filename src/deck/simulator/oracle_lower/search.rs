//! Lower complete multiple-result search syntax without losing placements.

use crate::deck::simulator::model::{MultiSearchSpec, SearchPlacement};
use crate::deck::simulator::oracle_ast::search::OracleMultiSearch;

/// Reject contradictory bounds before making a search executable.
pub(super) fn lower_multi_search(search: &OracleMultiSearch) -> Option<MultiSearchSpec> {
    let capacity = search
        .destinations
        .iter()
        .try_fold(0u32, |total, placement| {
            total.checked_add(placement.max_cards)
        })?;
    if search.min_cards > search.max_cards
        || search.destinations.is_empty()
        || capacity < search.max_cards
    {
        return None;
    }
    Some(MultiSearchSpec {
        card_type: search.card_type,
        land_types: search.land_types,
        min_cards: search.min_cards,
        max_cards: search.max_cards,
        reveal: search.reveal,
        destinations: search
            .destinations
            .iter()
            .map(|placement| SearchPlacement {
                destination: placement.destination,
                max_cards: placement.max_cards,
            })
            .collect(),
        shuffle: search.shuffle,
    })
}
