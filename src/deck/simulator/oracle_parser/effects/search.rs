//! Search selectors and destinations reject requirements the runtime cannot preserve.

use super::parse_oracle_number_word;
use crate::deck::simulator::model::{
    BasicLandType, ManaColor, SearchCardType, SearchDestination, SearchSpec,
};

/// Parse supported search words into explicit card constraints.
pub(super) fn parse_oracle_search_spec(text: &str) -> Option<SearchSpec> {
    let selector = search_selector(text)?;
    // These selectors need card metadata or source state that SearchSpec
    // cannot check. Never turn them into a broader search.
    if text.contains("legendary")
        || text.contains("less than ")
        || text.contains("greater than ")
        || text.contains("power")
        || text.contains("any number")
        || text.matches("shuffle").count() > 1
        || text.matches("mana value").count() > 1
        || !search_filter_supported(selector)
    {
        return None;
    }
    let mana_value = search_mana_value(selector);
    let card_type = search_card_type(selector);
    let destination = search_destination(text, selector)?;
    if destination == SearchDestination::LibraryTop
        && let Some(shuffle) = text.find("shuffle")
        && text
            .find("on top")
            .is_none_or(|placement| placement < shuffle)
    {
        return None;
    }
    if selector.contains("mana value") && mana_value.is_none() {
        return None;
    }
    let land_types = if matches!(
        card_type,
        Some(SearchCardType::Land | SearchCardType::BasicLand)
    ) {
        BasicLandType::ALL.map(|land| selector.contains(&land.name().to_ascii_lowercase()))
    } else {
        [false; 5]
    };
    Some(SearchSpec {
        card_type,
        color: search_color(selector),
        colorless: selector.contains("colorless"),
        mana_value: if !selector.contains("or less")
            && !selector.contains("or more")
            && !selector.contains("or greater")
        {
            mana_value
        } else {
            None
        },
        max_mana_value: if selector.contains("or less") {
            mana_value
        } else {
            None
        },
        min_mana_value: if selector.contains("or more") || selector.contains("or greater") {
            mana_value
        } else {
            None
        },
        destination,
        optional: text.contains("you may search")
            || text.contains("you may put")
            || text.contains("up to one")
            || text.contains("up to 1 "),
        top_count: search_top_count(text),
        shuffle: text.contains("shuffle"),
        non_human: selector.contains("non-human"),
        land_types,
        basic_land_only: matches!(card_type, Some(SearchCardType::BasicLand)),
    })
}

/// Keep list commas until a written reveal, movement, or shuffle instruction begins.
fn search_selector(text: &str) -> Option<&str> {
    let selector = if let Some((source, selector)) = text.split_once(" for ") {
        if !matches!(
            source,
            "search your library" | "you may search your library"
        ) {
            return None;
        }
        selector
    } else if let Some((source, selector)) = text.split_once("put ") {
        if !bounded_search_source(source) {
            return None;
        }
        selector
    } else if text.starts_with("reveal the top") && text.contains("into your hand") {
        "a card"
    } else {
        return None;
    };
    let end = [",", ".", " and "]
        .into_iter()
        .flat_map(|separator| {
            selector
                .match_indices(separator)
                .map(move |(index, _)| (index, separator.len()))
        })
        .filter(|(index, length)| {
            let tail = selector[index + length..].trim_start();
            let tail = tail.strip_prefix("then ").unwrap_or(tail);
            ["reveal ", "put ", "exile ", "shuffle"]
                .iter()
                .any(|instruction| tail.starts_with(instruction))
        })
        .map(|(index, _)| index)
        .chain(selector.find(" from among them"))
        .min()
        .unwrap_or(selector.len());
    Some(selector[..end].trim().trim_end_matches('.'))
}

/// A bounded search keeps its printed count and reads only the player's library.
fn bounded_search_source(text: &str) -> bool {
    let Some(source) = text
        .strip_prefix("look at the top ")
        .or_else(|| text.strip_prefix("reveal the top "))
    else {
        return false;
    };
    let Some((amount, source)) = source.split_once(' ') else {
        return false;
    };
    let Some(tail) = source
        .strip_prefix("cards of your library")
        .or_else(|| source.strip_prefix("card of your library"))
    else {
        return false;
    };
    search_top_amount(amount).is_some() && matches!(search_connector(tail).trim(), "" | "you may")
}

/// Reject required search words that SearchSpec cannot check.
fn search_filter_supported(selector: &str) -> bool {
    let words: Vec<_> = selector
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '-'
            })
        })
        .collect();
    if !search_selects_one_card(&words) {
        return false;
    }
    if !words.iter().copied().all(search_word_supported) {
        return false;
    }
    let card_types = [
        "artifact",
        "creature",
        "enchantment",
        "instant",
        "sorcery",
        "planeswalker",
        "permanent",
        "land",
    ]
    .iter()
    .filter(|kind| words.contains(kind))
    .count();
    let supported_union = words.windows(3).any(|phrase| {
        matches!(
            phrase,
            ["artifact", "or", "enchantment"] | ["instant", "or", "sorcery"]
        )
    });
    let colors = ["white", "blue", "black", "red", "green"]
        .iter()
        .filter(|color| words.contains(color))
        .count();
    let has_land_subtype = BasicLandType::ALL
        .iter()
        .any(|kind| words.contains(&kind.name().to_ascii_lowercase().as_str()));
    colors <= 1
        && !(colors > 0 && words.contains(&"colorless"))
        && (card_types <= 1 || card_types == 2 && supported_union)
        && (!has_land_subtype || card_types == 0 || card_types == 1 && words.contains(&"land"))
        && (!words.contains(&"basic") || has_land_subtype || words.contains(&"land"))
        && search_land_types_supported(selector)
}

/// The land-type mask supports alternatives, not intersections or mixed list operators.
fn search_land_types_supported(selector: &str) -> bool {
    let mut start = selector.len();
    let mut end = 0;
    let mut count = 0;
    for kind in BasicLandType::ALL {
        let name = kind.name().to_ascii_lowercase();
        for (offset, _) in selector.match_indices(&name) {
            start = start.min(offset);
            end = end.max(offset + name.len());
            count += 1;
        }
    }
    if count <= 1 {
        return true;
    }
    let Some((head, last)) = selector[start..end].rsplit_once(" or ") else {
        return false;
    };
    let is_land_type = |word: &str| {
        BasicLandType::ALL
            .iter()
            .any(|kind| kind.name().eq_ignore_ascii_case(word))
    };
    is_land_type(last)
        && head
            .trim_end_matches(',')
            .split(',')
            .all(|word| is_land_type(word.trim()))
}

/// Admit only selector words with a retained runtime restriction.
fn search_word_supported(word: &str) -> bool {
    matches!(
        word,
        "up" | "card"
            | "basic"
            | "land"
            | "creature"
            | "artifact"
            | "enchantment"
            | "planeswalker"
            | "permanent"
            | "instant"
            | "sorcery"
            | "or"
            | "and"
            | "white"
            | "blue"
            | "black"
            | "red"
            | "green"
            | "colorless"
            | "with"
            | "mana"
            | "value"
            | "exactly"
            | "equal"
            | "to"
            | "less"
            | "more"
            | "greater"
            | "than"
            | "non-human"
            | "plains"
            | "island"
            | "swamp"
            | "mountain"
            | "forest"
    ) || parse_oracle_number_word(word).is_some()
}

/// Keep result counts separate from numeric mana-value restrictions.
fn search_selects_one_card(words: &[&str]) -> bool {
    let selector = words.strip_prefix(&["up", "to"]).unwrap_or(words);
    selector
        .first()
        .is_some_and(|count| parse_oracle_number_word(count) == Some(1))
        && selector.iter().filter(|word| **word == "card").count() == 1
        && !selector.contains(&"cards")
        && selector
            .iter()
            .skip(1)
            .take_while(|word| **word != "card")
            .all(|word| parse_oracle_number_word(word).is_none())
}

/// Find the card type named by a supported search phrase.
fn search_card_type(text: &str) -> Option<SearchCardType> {
    let has_land_type = text.contains("land")
        || BasicLandType::ALL
            .iter()
            .any(|kind| text.contains(&kind.name().to_ascii_lowercase()));
    if text.contains("basic ") && has_land_type {
        Some(SearchCardType::BasicLand)
    } else if text.contains("artifact or enchantment") {
        Some(SearchCardType::ArtifactOrEnchantment)
    } else if text.contains("instant or sorcery") {
        Some(SearchCardType::InstantSorcery)
    } else if text.contains("instant") {
        Some(SearchCardType::Instant)
    } else if text.contains("sorcery") {
        Some(SearchCardType::Sorcery)
    } else if text.contains("creature") {
        Some(SearchCardType::Creature)
    } else if text.contains("planeswalker") {
        Some(SearchCardType::Planeswalker)
    } else if text.contains("enchantment") {
        Some(SearchCardType::Enchantment)
    } else if text.contains("artifact") {
        Some(SearchCardType::Artifact)
    } else if text.contains("permanent") {
        Some(SearchCardType::Permanent)
    } else if has_land_type {
        Some(SearchCardType::Land)
    } else {
        None
    }
}

/// Find the first supported color named by a search phrase.
fn search_color(text: &str) -> Option<ManaColor> {
    [
        ("white", ManaColor::White),
        ("blue", ManaColor::Blue),
        ("black", ManaColor::Black),
        ("red", ManaColor::Red),
        ("green", ManaColor::Green),
    ]
    .into_iter()
    .find_map(|(word, symbol)| text.contains(word).then_some(symbol))
}

/// Parse a mana value following the words "mana value".
fn search_mana_value(selector: &str) -> Option<u32> {
    let (_, relation) = selector.split_once("mana value ")?;
    let relation = relation
        .strip_prefix("equal to ")
        .or_else(|| relation.strip_prefix("exactly "))
        .unwrap_or(relation);
    let (value, comparison) = relation.split_once(' ').unwrap_or((relation, ""));
    if !matches!(comparison, "" | "or less" | "or more" | "or greater") {
        return None;
    }
    value.parse().ok()
}

/// Preserve the complete placement instead of discarding controller or entry requirements.
fn search_destination(text: &str, selector: &str) -> Option<SearchDestination> {
    let mut remaining = if let Some((_, selection)) = text.split_once(" for ") {
        selection.strip_prefix(selector)?
    } else {
        let (_, selection) = text.split_once("put ")?;
        let placement = selection
            .strip_prefix(selector)?
            .strip_prefix(" from among them ")?;
        return search_placement(placement);
    };
    loop {
        remaining = search_connector(remaining);
        let placement = remaining.strip_prefix("you may ").unwrap_or(remaining);
        if let Some(placement) = placement.strip_prefix("put ") {
            return search_placement(search_card_reference(placement)?.trim_start());
        }
        if let Some(placement) = remaining.strip_prefix("exile ") {
            return search_placement_tail(search_card_reference(placement)?)
                .then_some(SearchDestination::Exile);
        }
        if let Some(reveal) = remaining.strip_prefix("reveal ") {
            remaining = search_card_reference(reveal)?;
        } else {
            let shuffle = remaining.strip_prefix("shuffle")?;
            remaining = shuffle.strip_prefix(" your library").unwrap_or(shuffle);
        }
    }
}

/// Only written connectors can join the supported search instructions.
fn search_connector(text: &str) -> &str {
    let text = text.trim_start();
    let text = text.strip_prefix([',', '.']).unwrap_or(text).trim_start();
    let text = text.strip_prefix("and ").unwrap_or(text);
    text.strip_prefix("then ").unwrap_or(text)
}

/// A single search result can use these equivalent references.
fn search_card_reference(text: &str) -> Option<&str> {
    ["that card", "the card", "it"]
        .into_iter()
        .find_map(|reference| text.strip_prefix(reference))
}

/// Admit only placements whose zone, controller, and entry state the runtime preserves.
fn search_placement(text: &str) -> Option<SearchDestination> {
    use SearchDestination::{Battlefield, BattlefieldTapped, Graveyard, Hand, LibraryTop};

    [
        (
            "onto the battlefield tapped under your control",
            BattlefieldTapped,
        ),
        (
            "onto the battlefield under your control tapped",
            BattlefieldTapped,
        ),
        ("onto the battlefield under your control", Battlefield),
        ("onto the battlefield tapped", BattlefieldTapped),
        ("onto the battlefield", Battlefield),
        ("on top of your library", LibraryTop),
        ("on top of it", LibraryTop),
        ("on top", LibraryTop),
        ("into your graveyard", Graveyard),
        ("into its owner's graveyard", Graveyard),
        ("into your hand", Hand),
        ("into its owner's hand", Hand),
    ]
    .into_iter()
    .find_map(|(placement, destination)| {
        text.strip_prefix(placement)
            .filter(|tail| search_placement_tail(tail))
            .map(|_| destination)
    })
}

/// A placement can end or shuffle the player's library, but cannot discard required remainder rules.
fn search_placement_tail(text: &str) -> bool {
    let tail = search_connector(text).trim_end_matches('.').trim_end();
    matches!(tail, "" | "shuffle" | "shuffle your library")
}

/// Parse a supported count after the words "top".
fn search_top_count(text: &str) -> Option<usize> {
    text.split("top ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(search_top_amount)
}

/// Read only top-card counts the search model can retain.
fn search_top_amount(word: &str) -> Option<usize> {
    match word {
        "five" => Some(5),
        "four" => Some(4),
        "three" => Some(3),
        "two" => Some(2),
        "one" => Some(1),
        digits => digits.parse().ok(),
    }
}
