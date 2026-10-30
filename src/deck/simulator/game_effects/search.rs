//! Library search selection, placement, and entry-event capture.

use super::{CardZone, move_to_graveyard};
use crate::deck::simulator::cast_pass::{fire_entry_triggers, new_entry_permanent};
use crate::deck::simulator::game::{GameState, record_card_seen};
use crate::deck::simulator::model::{
    CardIdx, MultiSearchSpec, SearchCardType, SearchDestination, SearchPlacement, SearchSpec,
    SimDeck,
};
use crate::deck::simulator::rng::shuffle;

/// Move one search result and return its new battlefield uid, if it entered there.
pub(super) fn search_one(
    deck: &SimDeck,
    st: &mut GameState,
    spec: SearchSpec,
    turn: u32,
) -> Option<u32> {
    let found = if let Some(limit) = spec.top_count {
        let start = st.library.len().saturating_sub(limit);
        let mut revealed = st.library.split_off(start);
        st.seen += revealed.len() as u32;
        st.awareness_cards += revealed.len() as u32;
        let found = revealed
            .iter()
            .rposition(|index| search_matches(deck, *index, &spec))
            .map(|position| revealed.remove(position));
        st.library.extend(revealed);
        found
    } else {
        st.library
            .iter()
            .rposition(|index| search_matches(deck, *index, &spec))
            .map(|position| {
                st.seen += 1;
                st.library.remove(position)
            })
    };
    if spec.shuffle {
        shuffle(&mut st.library, &mut st.library_rng);
    }
    found.and_then(|index| place_search_result(deck, st, index, spec.destination, turn))
}

/// Place distinct results together before the search's printed trailing shuffle.
pub(super) fn search_many(deck: &SimDeck, st: &mut GameState, spec: &MultiSearchSpec, turn: u32) {
    debug_assert!(spec.min_cards <= spec.max_cards);
    let filter = SearchSpec {
        card_type: Some(spec.card_type),
        land_types: spec.land_types,
        ..Default::default()
    };
    let positions: Vec<_> = st
        .library
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, index)| search_matches(deck, **index, &filter))
        .take(spec.max_cards as usize)
        .map(|(position, _)| position)
        .collect();
    let selected: Vec<_> = positions
        .into_iter()
        .map(|position| st.library.remove(position))
        .collect();
    st.seen += selected.len() as u32;
    if spec.reveal {
        st.awareness_cards += selected.len() as u32;
    }
    let assigned = assign_search_results(&selected, &spec.destinations);
    place_search_results(deck, st, &assigned, turn);
    if spec.shuffle {
        shuffle(&mut st.library, &mut st.library_rng);
    }
}

/// Check a library card's front face against every parsed search restriction.
pub(super) fn search_matches(deck: &SimDeck, index: CardIdx, spec: &SearchSpec) -> bool {
    let card = &deck[index];
    let has_type = |kind: &str| card.card_types.iter().any(|value| value == kind);
    spec.card_type.is_none_or(|kind| match kind {
        SearchCardType::Creature => has_type("creature"),
        SearchCardType::Land => has_type("land"),
        SearchCardType::BasicLand => has_type("land") && card.is_basic_land,
        SearchCardType::Artifact => has_type("artifact"),
        SearchCardType::Enchantment => has_type("enchantment"),
        SearchCardType::ArtifactOrEnchantment => has_type("artifact") || has_type("enchantment"),
        SearchCardType::Instant => has_type("instant"),
        SearchCardType::Sorcery => has_type("sorcery"),
        SearchCardType::InstantSorcery => has_type("instant") || has_type("sorcery"),
        SearchCardType::Planeswalker => has_type("planeswalker"),
        SearchCardType::Permanent => [
            "creature",
            "land",
            "artifact",
            "enchantment",
            "planeswalker",
            "battle",
        ]
        .iter()
        .any(|kind| has_type(kind)),
    }) && (!spec.non_human || !card.is_human)
        && (!spec.basic_land_only || has_type("land") && card.is_basic_land)
        && (!spec.land_types.iter().any(|allowed| *allowed)
            || has_type("land")
                && spec
                    .land_types
                    .iter()
                    .zip(card.land_types)
                    .any(|(allowed, present)| *allowed && present))
        && spec.color.is_none_or(|color| {
            card.search_colors
                .is_some_and(|colors| colors[color.index()])
        })
        && (!spec.colorless
            || card
                .search_colors
                .is_some_and(|colors| colors.iter().all(|color| !color)))
        && spec.mana_value.is_none_or(|value| card.mana_value == value)
        && spec
            .max_mana_value
            .is_none_or(|value| card.mana_value <= value)
        && spec
            .min_mana_value
            .is_none_or(|value| card.mana_value >= value)
}

/// Assign each selected physical card to one printed destination slot.
fn assign_search_results(
    mut selected: &[CardIdx],
    destinations: &[SearchPlacement],
) -> Vec<(CardIdx, SearchDestination)> {
    let mut assigned = Vec::with_capacity(selected.len());
    for placement in destinations {
        let count = selected.len().min(placement.max_cards as usize);
        let (batch, remaining) = selected.split_at(count);
        assigned.extend(batch.iter().map(|index| (*index, placement.destination)));
        selected = remaining;
    }
    debug_assert!(
        selected.is_empty(),
        "lowered placement capacities cover every result"
    );
    assigned
}

/// Build every battlefield result against the pre-entry board, then capture the complete batch.
fn place_search_results(
    deck: &SimDeck,
    st: &mut GameState,
    assigned: &[(CardIdx, SearchDestination)],
    turn: u32,
) {
    let first = st.battlefield.len();
    let entries: Vec<_> = assigned
        .iter()
        .filter(|(_, destination)| is_battlefield_destination(*destination))
        .map(|(index, destination)| {
            st.battlefield_seen.entry(*index).or_insert(turn);
            new_entry_permanent(
                deck,
                st,
                *index,
                turn,
                *destination == SearchDestination::BattlefieldTapped,
            )
        })
        .collect();
    st.battlefield.extend(entries);
    for (index, destination) in assigned {
        if !is_battlefield_destination(*destination) {
            let _ = place_search_result(deck, st, *index, *destination, turn);
        }
    }
    for position in first..st.battlefield.len() {
        fire_entry_triggers(deck, st, position, turn);
    }
}

/// Identify placements that share the compound instruction's battlefield entry event.
fn is_battlefield_destination(destination: SearchDestination) -> bool {
    matches!(
        destination,
        SearchDestination::Battlefield | SearchDestination::BattlefieldTapped
    )
}

/// Place one selected card and retain its new permanent identity before entry events.
pub(super) fn place_search_result(
    deck: &SimDeck,
    st: &mut GameState,
    index: CardIdx,
    destination: SearchDestination,
    turn: u32,
) -> Option<u32> {
    match destination {
        SearchDestination::Hand => {
            record_card_seen(st, index, turn);
            st.hand.push(index);
        }
        SearchDestination::LibraryTop => st.library.push(index),
        SearchDestination::Exile => st.exile.push(index),
        SearchDestination::Graveyard => move_to_graveyard(deck, st, index, turn, CardZone::Library),
        SearchDestination::Battlefield | SearchDestination::BattlefieldTapped => {
            st.battlefield_seen.entry(index).or_insert(turn);
            let entry = new_entry_permanent(
                deck,
                st,
                index,
                turn,
                destination == SearchDestination::BattlefieldTapped,
            );
            let uid = entry.uid;
            st.battlefield.push(entry);
            fire_entry_triggers(deck, st, st.battlefield.len() - 1, turn);
            return Some(uid);
        }
    }
    None
}

/// Source-driven checks for result counts, physical cards, shuffles, and entry ordering.
#[cfg(test)]
#[path = "tests/multi_search_tests.rs"]
mod multi_search_tests;
