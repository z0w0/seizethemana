//! Oracle-text parsing for mana-paid and life-paid cycling.

use super::model::Cost;

/// Read a mana cost from the supported cycling reminder text.
pub(super) fn cycling_cost(text: &str) -> Option<Cost> {
    let segment = text
        .split(['.', '\n'])
        .find(|segment| segment.to_ascii_lowercase().contains("cycling"))?;
    let lower = segment.to_ascii_lowercase();
    let cycling = lower.find("cycling")? + "cycling".len();
    let cost = segment[cycling..].trim_start();
    let (_, tail) = cost.split_once('{')?;
    let symbols = format!(
        "{{{}",
        tail.split_once('}').map_or(tail, |(symbol, _)| symbol)
    );
    Some(super::parse_cost::parse_cost(&symbols))
}

/// Read a life payment from the supported cycling reminder text.
pub(super) fn cycling_life(text: &str) -> u32 {
    text.split(['.', '\n'])
        .find(|segment| segment.to_ascii_lowercase().contains("cycling"))
        .and_then(|segment| {
            let lower = segment.to_ascii_lowercase();
            let position = lower.find("pay ")? + 4;
            lower[position..]
                .split_whitespace()
                .next()?
                .parse::<u32>()
                .ok()
        })
        .unwrap_or(0)
}

/// Read the basic land subtype named by a landcycling keyword.
pub(super) fn landcycling_type(text: &str) -> Option<char> {
    let lower = text.to_ascii_lowercase();
    [
        ("plainscycling", 'W'),
        ("islandcycling", 'U'),
        ("swampcycling", 'B'),
        ("mountaincycling", 'R'),
        ("forestcycling", 'G'),
    ]
    .into_iter()
    .find_map(|(word, color)| lower.contains(word).then_some(color))
}
