//! Database-only query listings with the existing card-field filters.

use super::Hit;
use crate::{db, search::CardFilters};
use std::collections::HashSet;

/// Apply card filters and ownership/price scope without loading a vector index.
/// Unicode lowercase keys set the order; exact stored names break ties.
/// Names are not normalized, merged, or changed for display.
pub(super) fn filter_only(
    conn: &rusqlite::Connection,
    filters: &CardFilters,
    limit: usize,
    restrict: Option<&HashSet<String>>,
) -> anyhow::Result<Vec<Hit>> {
    let mut matches: Vec<_> = db::load_all_cards(conn)?
        .into_iter()
        .filter(|card| {
            restrict.is_none_or(|names| names.contains(&card.name)) && filters.matches(card)
        })
        .map(|card| Hit { card, score: 0.0 })
        .collect();
    matches.sort_by_cached_key(|hit| (hit.card.name.to_lowercase(), hit.card.name.clone()));
    matches.truncate(limit);
    Ok(matches)
}
