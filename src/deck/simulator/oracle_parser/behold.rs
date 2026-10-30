//! Complete Behold clauses with typed qualities and linked conditional effects.

use crate::deck::simulator::model::{SearchCardType, SearchDestination};
use crate::deck::simulator::oracle_ast::OracleEffect;
use crate::deck::simulator::oracle_ast::behold::{OracleBeholdAlternative, OracleBeholdSpec};

/// Parse a complete counted quality, including combined subtype and card-type rules.
pub(in crate::deck::simulator) fn parse_quality(source: &str) -> Option<OracleBeholdSpec> {
    let lower = source.trim().to_ascii_lowercase();
    let (amount, body) = lower.split_once(' ')?;
    let count = match amount {
        "a" | "an" => 1,
        _ => super::effects::parse_oracle_number_word(amount).filter(|count| *count > 0)?,
    };
    let mut spec = OracleBeholdSpec {
        card_types: Vec::new(),
        subtypes: Vec::new(),
        count,
    };
    for word in body.split_whitespace() {
        let word = if count > 1 {
            singular_quality(word)
        } else {
            word
        };
        match word {
            "artifact" | "battle" | "creature" | "enchantment" | "instant" | "land"
            | "planeswalker" | "sorcery" | "legendary" | "basic" | "snow" | "kindred" => {
                spec.card_types.push(word.to_string());
            }
            "a" | "an" | "and" | "or" | "with" | "without" | "of" | "that" | "type" | "card"
            | "permanent" | "you" | "your" | "control" | "from" | "hand" | "in" | "graveyard"
            | "target" | "other" | "another" | "nonland" | "noncreature" | "tapped"
            | "untapped" | "white" | "blue" | "black" | "red" | "green" | "colorless"
            | "multicolored" | "chosen" | "any" => return None,
            subtype if subtype.chars().all(char::is_alphabetic) => {
                spec.subtypes.push(subtype.to_string());
            }
            _ => return None,
        }
    }
    if spec.card_types.is_empty() && spec.subtypes.is_empty() || spec.subtypes.len() > 1 {
        return None;
    }
    Some(spec)
}

/// Normalize only counted quality nouns; a singular quality keeps its spelling.
fn singular_quality(word: &str) -> &str {
    match word {
        "elves" => "elf",
        "dwarves" => "dwarf",
        "wolves" => "wolf",
        "plains" => "plains",
        word => word.strip_suffix('s').unwrap_or(word),
    }
}

/// Read only a complete mandatory Behold-or-generic-mana additional cost.
pub(in crate::deck::simulator) fn additional_cost_alternative(
    source: &str,
) -> Option<OracleBeholdAlternative> {
    let text = rules_text(source);
    let body = text
        .split('.')
        .next()?
        .strip_prefix("as an additional cost to cast this spell, behold ")?;
    let (quality, mana) = body.split_once(" or pay ")?;
    let generic = mana.strip_prefix('{')?.strip_suffix('}')?.parse().ok()?;
    Some(OracleBeholdAlternative {
        spec: parse_quality(quality)?,
        generic,
    })
}

/// Preserve a complete optional Behold instruction and its dependent reward.
pub(in crate::deck::simulator) fn conditional_reward(source: &str) -> Option<OracleEffect> {
    let text = rules_text(source);
    let (quality, reward) = text
        .strip_prefix("you may behold ")?
        .split_once(". if you do, ")?;
    if reward.trim_end_matches('.').is_empty() {
        return None;
    }
    Some(OracleEffect::Behold {
        spec: parse_quality(quality)?,
        effects: vec![
            if reward.trim_end_matches('.') == "create a treasure token" {
                OracleEffect::CreateTreasureTokens(1)
            } else {
                OracleEffect::Unsupported(reward.to_string())
            },
        ],
    })
}

/// Keep an optional untap linked to the land found by the preceding search.
pub(in crate::deck::simulator) fn fetch_untap_annotation(source: &str) -> Option<OracleEffect> {
    let text = rules_text(source);
    let (search, tail) = text.split_once(". you may behold ")?;
    let (quality, untap) = tail.split_once(". if you do, ")?;
    if untap.trim_end_matches('.') != "untap that land" {
        return None;
    }
    let parsed = super::effects::parse_oracle_effects(search);
    let [OracleEffect::Search(search)] = parsed.as_slice() else {
        return None;
    };
    if search.destination != SearchDestination::BattlefieldTapped
        || !matches!(
            search.card_type,
            Some(SearchCardType::Land | SearchCardType::BasicLand)
        )
    {
        return None;
    }
    Some(OracleEffect::SearchThenBeholdUntap {
        search: *search,
        spec: parse_quality(quality)?,
    })
}

/// Parse the full optional Behold replacement that otherwise makes a land enter tapped.
pub(in crate::deck::simulator) fn land_entry_condition(source: &str) -> Option<OracleBeholdSpec> {
    let text = rules_text(source);
    let (quality, failure) = text
        .strip_prefix("as this land enters, you may behold ")?
        .split_once(". if you don't, ")?;
    if failure.trim_end_matches('.') != "this land enters tapped" {
        return None;
    }
    parse_quality(quality)
}

/// Claim Behold-bearing resolution clauses before generic splitting can grant free rewards.
pub(in crate::deck::simulator) fn parse_effect(source: &str) -> Option<OracleEffect> {
    let text = rules_text(source);
    if !text
        .split(|character: char| !character.is_ascii_alphabetic())
        .any(|word| word == "behold")
    {
        return None;
    }
    Some(
        conditional_reward(source)
            .or_else(|| fetch_untap_annotation(source))
            .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
    )
}

/// Remove reminder text while preserving the order of all actual rules clauses.
fn rules_text(source: &str) -> String {
    let mut text = String::with_capacity(source.len());
    let mut depth = 0u32;
    for character in source.chars() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(character),
            _ => {}
        }
    }
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .replace('’', "'")
}

/// Grammar controls keep qualities and dependent effects intact.
#[cfg(test)]
#[path = "tests/behold_tests.rs"]
mod behold_tests;
