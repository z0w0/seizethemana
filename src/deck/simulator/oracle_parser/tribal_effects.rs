//! Complete creature-token, scaled-mana, and selected-untap effect grammar.

use super::super::model::ManaColor;
use super::super::oracle_ast::tribal::{
    OracleCreatureToken, OraclePermanentSelection, OracleQuantity, OracleTribalEffect,
};
use super::super::oracle_ast::{OracleEffect, OracleTriggerEvent, PlayerScope};
use super::effects::parse_oracle_number_word;
use super::tribal::permanent_selector;

/// Recognize complete new effect forms before older lossy projections can run.
pub(super) fn parse_effect(
    source: &str,
    event: Option<&OracleTriggerEvent>,
) -> Option<OracleEffect> {
    let lower = source.trim().to_ascii_lowercase();
    if lower.starts_with("create ") && (lower.contains("creature token") || lower.contains('/')) {
        if let Some(effect) = creature_tokens(&lower, event) {
            return Some(OracleEffect::Tribal(effect));
        }
        // Do not create a generic token when the printed characteristics
        // cannot be represented by the typed token model.
        Some(OracleEffect::Unsupported(source.to_string()))
    } else if lower.starts_with("add ")
        && (lower.contains(" for each ") || lower.contains(" equal to "))
        && !lower.contains("charge counter")
        && !lower.contains("for each color among permanents you control")
    {
        Some(
            scaled_mana(&lower)
                .map(OracleEffect::Tribal)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else if lower.starts_with("untap ") {
        untap_selection(&lower)
    } else {
        None
    }
}

/// Preserve fixed or event-linked token counts and every creature characteristic.
fn creature_tokens(lower: &str, event: Option<&OracleTriggerEvent>) -> Option<OracleTribalEffect> {
    let body = lower.strip_prefix("create ")?;
    let (body, matching) = body
        .split_once(" for each ")
        .map_or((body, None), |(body, objects)| (body, Some(objects)));
    let (quantity, token) = if let Some(token) = body.strip_prefix("that many ") {
        if !matches!(event, Some(OracleTriggerEvent::CombatDamageToPlayer(_))) || matching.is_some()
        {
            return None;
        }
        (OracleQuantity::TriggerCombatDamage, token)
    } else {
        let (amount, token) = body.split_once(' ')?;
        let count = parse_oracle_number_word(amount).filter(|count| *count > 0)?;
        let quantity = if let Some(objects) = matching {
            if count != 1 {
                return None;
            }
            OracleQuantity::MatchingPermanents(permanent_selector(objects)?)
        } else {
            OracleQuantity::Fixed(count)
        };
        (quantity, token)
    };
    Some(OracleTribalEffect::CreateCreatureTokens {
        quantity,
        token: creature_token(token)?,
    })
}

/// Read one complete creature-token definition without dropping extra abilities.
fn creature_token(body: &str) -> Option<OracleCreatureToken> {
    let body = body.trim_end_matches('.');
    let body = body
        .strip_suffix(" creature tokens")
        .or_else(|| body.strip_suffix(" creature token"))?;
    let (tapped, attacking, body) = if let Some(body) = body.strip_prefix("tapped and attacking ") {
        (true, true, body)
    } else if let Some(body) = body.strip_prefix("tapped ") {
        (true, false, body)
    } else {
        (false, false, body)
    };
    let (stats, characteristics) = body.split_once(' ').unwrap_or((body, ""));
    let (power, toughness) = stats.split_once('/')?;
    let mut colors = Vec::new();
    let mut subtypes = Vec::new();
    let mut colorless = false;
    let mut needs_color = false;
    for word in characteristics.split_whitespace() {
        if let Some(color) = token_color(word) {
            if !subtypes.is_empty()
                || colorless
                || colors.contains(&color)
                || !colors.is_empty() && !needs_color
            {
                return None;
            }
            colors.push(color);
            needs_color = false;
        } else if word == "colorless" && colors.is_empty() && subtypes.is_empty() && !colorless {
            colorless = true;
        } else if word == "and" && !colors.is_empty() && subtypes.is_empty() && !needs_color {
            needs_color = true;
            continue;
        } else if needs_color
            || !word.chars().all(char::is_alphabetic)
            || matches!(
                word,
                "with"
                    | "without"
                    | "that"
                    | "named"
                    | "and"
                    | "or"
                    | "except"
                    | "for"
                    | "legendary"
                    | "artifact"
                    | "enchantment"
                    | "snow"
                    | "colorless"
            )
        {
            return None;
        } else {
            subtypes.push(word.to_string());
        }
    }
    if needs_color {
        return None;
    }
    Some(OracleCreatureToken {
        power: power.parse().ok()?,
        toughness: toughness.parse().ok()?,
        colors,
        subtypes,
        tapped,
        attacking,
    })
}

/// Map the written token colors without reading subtype names as colors.
fn token_color(word: &str) -> Option<ManaColor> {
    match word {
        "white" => Some(ManaColor::White),
        "blue" => Some(ManaColor::Blue),
        "black" => Some(ManaColor::Black),
        "red" => Some(ManaColor::Red),
        "green" => Some(ManaColor::Green),
        _ => None,
    }
}

/// Preserve the complete mana bundle and the exact source of its multiplier.
fn scaled_mana(lower: &str) -> Option<OracleTribalEffect> {
    let (production, quantity) = if let Some((production, objects)) = lower.split_once(" for each ")
    {
        (
            production.to_string(),
            OracleQuantity::MatchingPermanents(permanent_selector(objects)?),
        )
    } else {
        let production = lower
            .strip_prefix("add an amount of ")?
            .strip_suffix(" equal to this permanent's power")?;
        (format!("add {production}"), OracleQuantity::SourcePower)
    };
    let yield_ = super::land::parse_tap_yield(&production)?;
    Some(OracleTribalEffect::Mana { yield_, quantity })
}

/// Distinguish a source untap from a targeted or whole-set untap.
fn untap_selection(lower: &str) -> Option<OracleEffect> {
    let body = lower.strip_prefix("untap ")?;
    if matches!(
        body,
        "this permanent" | "this creature" | "this artifact" | "this land"
    ) {
        return Some(OracleEffect::UntapSource);
    }
    let selection = if let Some(objects) = body.strip_prefix("target ") {
        let selector = permanent_selector(objects)?;
        if selector.untapped {
            return None;
        }
        OraclePermanentSelection::Target(selector)
    } else {
        let objects = body.strip_prefix("each ")?;
        let selector = permanent_selector(objects)?;
        if selector.controller != PlayerScope::You || selector.untapped {
            return None;
        }
        OraclePermanentSelection::Each(selector)
    };
    Some(OracleEffect::Tribal(OracleTribalEffect::Untap(selection)))
}
