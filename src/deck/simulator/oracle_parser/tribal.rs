//! Shared battlefield selectors, written payments, and scoped continuous rules.

use super::super::model::BasicLandType;
use super::super::oracle_ast::tribal::{
    OraclePermanentSelector, OracleTribalCost, OracleTribalStaticEffect,
};
use super::super::oracle_ast::{
    ActivationCost, OracleEventFilter, OracleStaticEffect, PlayerScope,
};
use super::effects::parse_oracle_number_word;

/// Parse a complete battlefield selector with explicit controller and source scope.
pub(super) fn permanent_selector(source: &str) -> Option<OraclePermanentSelector> {
    let lower = source.to_ascii_lowercase();
    let (body, controller) = if let Some(body) = lower.strip_suffix(" you control") {
        (body, PlayerScope::You)
    } else if let Some(body) = lower.strip_suffix(" on the battlefield") {
        (body, PlayerScope::Any)
    } else {
        (lower.as_str(), PlayerScope::Any)
    };
    let mut body = body.trim();
    let exclude_source = body.starts_with("other ") || body.starts_with("another ");
    if exclude_source {
        body = body.split_once(' ')?.1;
    }
    let untapped = body.starts_with("untapped ");
    if untapped {
        body = body.strip_prefix("untapped ")?;
    }
    if body.is_empty() {
        return None;
    }
    if body.split_whitespace().any(|word| {
        matches!(
            word,
            "you"
                | "control"
                | "on"
                | "the"
                | "battlefield"
                | "in"
                | "graveyard"
                | "target"
                | "each"
                | "all"
                | "unless"
                | "if"
                | "during"
                | "until"
                | "at"
                | "of"
                | "for"
                | "this"
                | "colorless"
                | "card"
                | "cards"
                | "basic"
        )
    }) {
        return None;
    }
    let words = body
        .split_whitespace()
        .map(singular_object)
        .collect::<Vec<_>>();
    let mut filter = OracleEventFilter::default();
    super::triggers::append_object_filter(&mut filter, &words.join(" "), source);
    if filter.unsupported.is_some() {
        return None;
    }
    if filter.card_types.is_empty()
        && filter.subtypes.is_empty()
        && body != "permanents"
        && body != "permanent"
    {
        return None;
    }
    if words.len() == 1
        && BasicLandType::ALL
            .iter()
            .any(|kind| kind.name().eq_ignore_ascii_case(&words[0]))
    {
        filter.card_types.push("land".to_string());
    }
    Some(OraclePermanentSelector {
        filter,
        controller,
        exclude_source,
        untapped,
    })
}

/// Normalize the plural nouns used by counted permanent instructions.
fn singular_object(word: &str) -> String {
    match word {
        "elves" => "elf".to_string(),
        "dwarves" => "dwarf".to_string(),
        "wolves" => "wolf".to_string(),
        "plains" => "plains".to_string(),
        word => word.strip_suffix('s').unwrap_or(word).to_string(),
    }
}

/// Retain counted tap and filtered battlefield-return payments without lowering them.
pub(super) fn selected_permanent_cost(source: &str) -> Option<OracleTribalCost> {
    let lower = source.to_ascii_lowercase();
    if let Some(body) = lower.strip_prefix("tap ") {
        let (count, body) = counted_objects(body)?;
        let selector = permanent_selector(body)?;
        if !selector.untapped || selector.controller != PlayerScope::You {
            return None;
        }
        return Some(OracleTribalCost::TapPermanents { count, selector });
    }
    let body = lower
        .strip_prefix("return ")?
        .strip_suffix(" to its owner's hand")?;
    let (count, body) = counted_objects(body)?;
    let selector = permanent_selector(body)?;
    if count != 1 || selector.controller != PlayerScope::You || selector.untapped {
        return None;
    }
    Some(OracleTribalCost::ReturnToHand { count, selector })
}

/// Separate a complete positive count from its object requirements.
fn counted_objects(body: &str) -> Option<(u32, &str)> {
    if body.starts_with("another ") {
        return Some((1, body));
    }
    let (amount, body) = body.split_once(' ')?;
    let count = parse_oracle_number_word(amount).filter(|count| *count > 0)?;
    Some((count, body))
}

/// A separate source tap cannot also pay one of the written tap selections.
pub(super) fn exclude_source_tap(costs: &mut [ActivationCost]) {
    let source_tapped = costs.iter().any(|cost| {
        matches!(
            cost,
            ActivationCost::Tap(super::super::oracle_ast::ActivationTarget::Source)
        )
    });
    if source_tapped {
        for cost in costs {
            if let ActivationCost::Tribal(OracleTribalCost::TapPermanents { selector, .. }) = cost {
                selector.exclude_source = true;
            }
        }
    }
}

/// Preserve a complete scoped lord effect or granted mana ability.
pub(super) fn scoped_static_effect(source: &str) -> Option<OracleStaticEffect> {
    let lower = source.trim().to_ascii_lowercase();
    let body = lower.trim_end_matches('.');
    if let Some((objects, adjustment)) = body.split_once(" get ") {
        let selector = permanent_selector(objects)?;
        let (power, toughness) = adjustment.split_once('/')?;
        if !power.starts_with(['+', '-']) || !toughness.starts_with(['+', '-']) {
            return None;
        }
        let power = power.parse().ok()?;
        let toughness = toughness.parse().ok()?;
        if selector.controller == PlayerScope::You
            && !selector.exclude_source
            && !selector.untapped
            && selector.filter.card_types == ["creature"]
            && selector.filter.subtypes.is_empty()
        {
            return Some(OracleStaticEffect::CreatureBuff { power, toughness });
        }
        return Some(OracleStaticEffect::Tribal(
            OracleTribalStaticEffect::CreatureBuff {
                selector,
                power,
                toughness,
            },
        ));
    }
    let (objects, granted) = source.trim().split_once(" have ")?;
    let selector = permanent_selector(objects)?;
    let granted = granted.strip_prefix('"')?.strip_suffix('"')?;
    let ability = super::activation::parse_oracle_activated_ability(granted)?;
    if !ability.is_mana_ability || !ability.effects.iter().all(complete_mana_effect) {
        return None;
    }
    Some(OracleStaticEffect::Tribal(
        OracleTribalStaticEffect::ManaGrant { selector, ability },
    ))
}

/// Reject incomplete granted abilities instead of keeping a free mana component.
fn complete_mana_effect(effect: &super::super::oracle_ast::OracleEffect) -> bool {
    matches!(
        effect,
        super::super::oracle_ast::OracleEffect::Mana(_)
            | super::super::oracle_ast::OracleEffect::Tribal(
                super::super::oracle_ast::tribal::OracleTribalEffect::Mana { .. }
            )
    )
}
