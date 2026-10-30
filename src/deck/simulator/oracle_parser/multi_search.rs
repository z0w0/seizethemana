//! Complete land searches with fixed result counts and explicit placement capacities.

use super::effects::parse_oracle_number_word;
use crate::deck::simulator::model::{BasicLandType, SearchCardType, SearchDestination};
use crate::deck::simulator::oracle_ast::search::{OracleMultiSearch, OracleSearchPlacement};

/// Retain one complete placement instruction followed only by an optional shuffle.
pub(super) fn parse_multi_search(source: &str) -> Option<OracleMultiSearch> {
    let normalized = source.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = normalized.to_ascii_lowercase();
    let body = normalized.strip_suffix('.').unwrap_or(&normalized);
    let optional = body.starts_with("you may search ");
    let body = body.strip_prefix("you may ").unwrap_or(body);
    let body = body.strip_prefix("search your library for ")?;
    let up_to = body.starts_with("up to ");
    let body = body.strip_prefix("up to ").unwrap_or(body);
    let (count, body) = body.split_once(' ')?;
    let max_cards = fixed_count(count)?;
    if max_cards < 2 {
        return None;
    }
    let (selector, instructions) = selector_and_instructions(body)?;
    let (card_type, land_types) = land_filter(selector)?;
    let instructions = connector(instructions);
    let (reveal, instructions) = reveal_instruction(instructions);
    let instructions = connector(instructions).strip_prefix("put ")?;
    let (placement, shuffle) = placement_and_shuffle(instructions);
    let destinations = placements(placement, max_cards)?;
    Some(OracleMultiSearch {
        card_type,
        min_cards: if optional || up_to { 0 } else { max_cards },
        max_cards,
        reveal,
        destinations,
        shuffle,
        land_types,
    })
}

/// Find an instruction boundary without splitting a list of allowed land types.
fn selector_and_instructions(body: &str) -> Option<(&str, &str)> {
    [", ", ". ", " and "]
        .into_iter()
        .flat_map(|separator| {
            body.match_indices(separator)
                .map(move |(index, _)| (index, separator.len()))
        })
        .filter(|(index, length)| {
            let tail = connector(&body[index + length..]);
            tail.starts_with("reveal ") || tail.starts_with("put ")
        })
        .min_by_key(|(index, _)| *index)
        .map(|(index, length)| (&body[..index], &body[index + length..]))
}

/// Accept only land types and the Basic supertype that the runtime can check.
fn land_filter(selector: &str) -> Option<(SearchCardType, [bool; 5])> {
    let selector = selector.strip_suffix(" cards")?;
    let basic = selector.starts_with("basic ");
    let selector = selector.strip_prefix("basic ").unwrap_or(selector);
    let selector = selector.strip_suffix(" land").unwrap_or(selector);
    let mut land_types = [false; 5];
    if selector != "land" {
        let types = selector.replace(", or ", " or ").replace(", ", " or ");
        let names: Vec<_> = types.split(" or ").collect();
        if names.len() > 1 && !selector.contains(" or ") {
            return None;
        }
        for name in names {
            let kind = BasicLandType::ALL
                .into_iter()
                .find(|kind| kind.name().eq_ignore_ascii_case(name))?;
            if land_types[kind.index()] {
                return None;
            }
            land_types[kind.index()] = true;
        }
    }
    Some((
        if basic {
            SearchCardType::BasicLand
        } else {
            SearchCardType::Land
        },
        land_types,
    ))
}

/// Remove a written connector without dropping any rule words.
fn connector(body: &str) -> &str {
    let body = body.trim_start();
    let body = body.strip_prefix([',', '.']).unwrap_or(body).trim_start();
    let body = body.strip_prefix("and ").unwrap_or(body);
    body.strip_prefix("then ").unwrap_or(body)
}

/// A reveal instruction must refer to the whole selected group.
fn reveal_instruction(body: &str) -> (bool, &str) {
    ["reveal those cards", "reveal the cards", "reveal them"]
        .into_iter()
        .find_map(|instruction| {
            body.strip_prefix(instruction).filter(|tail| {
                tail.starts_with([',', '.'])
                    || tail.starts_with(" and ")
                    || tail.starts_with(" then ")
            })
        })
        .map_or((false, body), |tail| (true, tail))
}

/// Retain only a trailing shuffle; a shuffle before placement needs a different operation order.
fn placement_and_shuffle(body: &str) -> (&str, bool) {
    ["shuffle your library", "shuffle"]
        .into_iter()
        .find_map(|shuffle| {
            let head = body.strip_suffix(shuffle)?;
            [
                ", then ",
                ", and then ",
                " and then ",
                " and ",
                ", ",
                ". then ",
                ". ",
            ]
            .into_iter()
            .find_map(|separator| head.strip_suffix(separator))
        })
        .map_or((body, false), |head| (head, true))
}

/// Keep every destination in one compound placement with simultaneous battlefield entry.
fn placements(body: &str, max_cards: u32) -> Option<Vec<OracleSearchPlacement>> {
    let body = body.replace(", and ", " and ").replace(", ", " and ");
    let mut remaining = max_cards;
    let mut placements = Vec::new();
    for clause in body.split(" and ") {
        let (count, destination) = placement_count(clause, remaining, max_cards)?;
        remaining = remaining.checked_sub(count)?;
        placements.push(OracleSearchPlacement {
            destination: destination_for(destination)?,
            max_cards: count,
        });
    }
    (remaining == 0).then_some(placements)
}

/// Resolve group references only when their count follows from the selected group.
fn placement_count(body: &str, remaining: u32, maximum: u32) -> Option<(u32, &str)> {
    for reference in ["those cards ", "the cards ", "them "] {
        if let Some(destination) = body.strip_prefix(reference) {
            return (remaining == maximum).then_some((maximum, destination));
        }
    }
    if let Some(destination) = body.strip_prefix("the other ") {
        return (remaining == 1 && remaining < maximum).then_some((1, destination));
    }
    if let Some(destination) = body.strip_prefix("the rest ") {
        return (remaining > 0 && remaining < maximum).then_some((remaining, destination));
    }
    let (amount, destination) = body.split_once(' ')?;
    let count = fixed_count(amount)?;
    let destination = destination
        .strip_prefix("of those cards ")
        .or_else(|| destination.strip_prefix("of them "))
        .unwrap_or(destination);
    (count > 0).then_some((count, destination))
}

/// Admit only complete placements whose controller, zone, and entry state are retained.
fn destination_for(body: &str) -> Option<SearchDestination> {
    use SearchDestination::{Battlefield, BattlefieldTapped, Graveyard, Hand, LibraryTop};
    match body {
        "onto the battlefield" | "onto the battlefield under your control" => Some(Battlefield),
        "onto the battlefield tapped"
        | "onto the battlefield tapped under your control"
        | "onto the battlefield under your control tapped" => Some(BattlefieldTapped),
        "into your hand" => Some(Hand),
        "into your graveyard" => Some(Graveyard),
        "on top of your library" => Some(LibraryTop),
        _ => None,
    }
}

/// Read one fixed count without admitting variables or trailing conditions.
fn fixed_count(word: &str) -> Option<u32> {
    if !word
        .chars()
        .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }
    parse_oracle_number_word(word)
}

/// Source-bound checks for complete selectors and destination counts.
#[cfg(test)]
#[path = "tests/multi_search_tests.rs"]
mod multi_search_tests;
