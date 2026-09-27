//! Parsing for activated abilities, their costs, and restrictions.

use super::super::model::amount_after;
use super::super::oracle_ast::*;
use super::effects::{parse_oracle_effects, parse_oracle_number_word};

/// Parse an activated ability and its separate cost parts.
pub(in crate::deck::simulator) fn parse_oracle_activated_ability(
    source: &str,
) -> Option<ActivatedAbility> {
    if !is_activation_start(source) {
        return None;
    }
    let (cost_text, effect_text) = source.split_once(':')?;
    let costs = parse_oracle_activation_costs(cost_text.trim());
    let effects = parse_oracle_effects(effect_text.trim());
    let restrictions = parse_oracle_restrictions(source);
    Some(ActivatedAbility {
        costs,
        effects,
        restrictions,
    })
}

/// Return true for an activated-ability cost prefix.
pub(super) fn is_activation_start(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let first_clause = text.split(['.', '\n']).next().unwrap_or(text);
    text.starts_with('{')
        || text.starts_with(['−', '–', '+', '-'])
        || lower.starts_with("tap:")
        || lower.starts_with("sacrifice ")
        || (lower.starts_with("discard ") || lower.starts_with("exile "))
            && first_clause.contains(':')
        || lower.starts_with("pay ")
        || lower.starts_with("remove a charge counter")
}

/// Parse mana, tap, life, sacrifice, discard, counter, and loyalty costs.
fn parse_oracle_activation_costs(source: &str) -> Vec<ActivationCost> {
    let lower = source.to_ascii_lowercase();
    let mut costs = Vec::new();
    append_mana_cost(source, &mut costs);
    append_tap_costs(&lower, &mut costs);
    append_sacrifice_cost(&lower, &mut costs);
    append_life_and_discard_costs(&lower, &mut costs);
    append_counter_cost(&lower, &mut costs);
    append_loyalty_cost(source, &mut costs);
    if costs.is_empty() {
        costs.push(ActivationCost::Other(source.to_string()));
    }
    costs
}

/// Add a nonzero mana cost when Oracle text contains mana symbols.
fn append_mana_cost(source: &str, costs: &mut Vec<ActivationCost>) {
    let mana = super::super::parse_cost::parse_activation_cost(source);
    if mana.total() > 0 {
        costs.push(ActivationCost::Mana(mana));
    }
}

/// Add tap and untap costs with their target scope.
fn append_tap_costs(lower: &str, costs: &mut Vec<ActivationCost>) {
    if lower.contains("{t}") || lower.starts_with("tap") {
        costs.push(ActivationCost::Tap(activation_target(lower)));
    }
    if lower.contains("{q}") || lower.contains("untap ") {
        costs.push(ActivationCost::Untap(activation_target(lower)));
    }
}

/// Identify whether an activation cost applies to the source or another object.
fn activation_target(lower: &str) -> ActivationTarget {
    if lower.contains("another") || lower.contains("creature you control") {
        ActivationTarget::AnotherObject
    } else {
        ActivationTarget::Source
    }
}

/// Add a sacrifice cost with its count and object type.
fn append_sacrifice_cost(lower: &str, costs: &mut Vec<ActivationCost>) {
    if lower.contains("sacrifice") {
        costs.push(ActivationCost::Sacrifice {
            count: parse_oracle_count_after(lower, "sacrifice"),
            object: if lower.contains("creature") {
                CostObject::Creature
            } else {
                CostObject::Any
            },
        });
    }
}

/// Add life-payment and discard costs when stated.
fn append_life_and_discard_costs(lower: &str, costs: &mut Vec<ActivationCost>) {
    if lower.contains("pay ") && lower.contains("life") {
        costs.push(ActivationCost::PayLife(amount_after(lower, "pay ")));
    }
    if lower.contains("discard") {
        costs.push(ActivationCost::Discard(parse_oracle_count_after(
            lower, "discard",
        )));
    }
}

/// Add a counter-removal cost and retain its counter name.
fn append_counter_cost(lower: &str, costs: &mut Vec<ActivationCost>) {
    if lower.contains("remove") && lower.contains("counter") {
        costs.push(ActivationCost::RemoveCounter {
            kind: counter_kind(lower),
            count: parse_oracle_count_after(lower, "remove"),
        });
    }
}

/// Add a signed loyalty cost when the prefix is a valid integer.
fn append_loyalty_cost(source: &str, costs: &mut Vec<ActivationCost>) {
    if let Some(loyalty) = parse_oracle_loyalty_cost(source) {
        costs.push(ActivationCost::Loyalty(loyalty));
    }
}

/// Keep the counter name from a removal cost when Oracle text states one.
fn counter_kind(source: &str) -> Option<String> {
    let counter_phrase = source.split_once(" counter")?.0;
    let cost = counter_phrase.split_once("remove ")?.1;
    let mut words = cost.split_whitespace();
    let first = words.next()?;
    let kind = if parse_oracle_number_word(first).is_some() {
        words.collect::<Vec<_>>().join(" ")
    } else {
        cost.to_string()
    };
    (!kind.is_empty()).then_some(kind)
}

/// Parse a signed planeswalker loyalty cost.
fn parse_oracle_loyalty_cost(source: &str) -> Option<i32> {
    let cleaned = source.trim().replace(['−', '–'], "-");
    if cleaned.is_empty()
        || !cleaned
            .chars()
            .all(|character| character.is_ascii_digit() || character == '-' || character == '+')
    {
        return None;
    }
    cleaned.parse().ok()
}

/// Parse activation limits and timing restrictions from a statement.
fn parse_oracle_restrictions(source: &str) -> Vec<AbilityRestriction> {
    let lower = source.to_ascii_lowercase();
    let mut restrictions = Vec::new();
    if lower.contains("only once each turn")
        || lower.contains("only once each of your turns")
        || lower.contains("triggers only once each turn")
    {
        restrictions.push(AbilityRestriction::OncePerTurn);
    }
    if lower.contains("only as a sorcery") {
        restrictions.push(AbilityRestriction::SorcerySpeed);
    }
    if let Some((_, condition)) = lower.split_once("activate only if ") {
        restrictions.push(AbilityRestriction::Condition(
            condition.trim_end_matches('.').to_string(),
        ));
    }
    // Keyword reminder condition: "(You have metalcraft if you control
    // three or more artifacts.)" gates the ability the same way an
    // "activate only if" clause does (CR 702.43).
    if let Some((_, condition)) = lower.split_once("have metalcraft if ") {
        restrictions.push(AbilityRestriction::Condition(
            condition
                .trim_end_matches('.')
                .trim_end_matches(')')
                .trim()
                .to_string(),
        ));
    }
    restrictions
}

/// Parse a quantity word after a cost marker.
fn parse_oracle_count_after(text: &str, marker: &str) -> u32 {
    let Some(tail) = text.split_once(marker).map(|(_, tail)| tail.trim_start()) else {
        return 1;
    };
    parse_oracle_number_word(tail).unwrap_or(1)
}
