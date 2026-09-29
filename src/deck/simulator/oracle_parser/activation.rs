//! Parsing for activated abilities, their costs, and restrictions.

use super::super::oracle_ast::*;
use super::super::oracle_parser::amount_after;
use super::effects::{parse_oracle_effects, parse_oracle_number_word};

/// Parse an activated ability and its separate cost parts.
pub(in crate::deck::simulator) fn parse_oracle_activated_ability(
    source: &str,
) -> Option<OracleActivatedAbility> {
    if !is_activation_start(source) {
        return None;
    }
    // Power-up (CR 702.193) and Exhaust (CR 702.177) prefix an ordinary
    // activated ability: "Power-up — {5}{U}: …". The prefix carries the
    // restriction; the cost and effect parse normally.
    let (body, keyword_restrictions) = strip_activation_keyword_prefix(source);
    let (cost_text, effect_text) = body.split_once(':')?;
    let costs = parse_oracle_activation_costs(cost_text.trim());
    let mut restrictions = parse_oracle_restrictions(source);
    restrictions.extend(keyword_restrictions);
    let mut effects = parse_oracle_effects(effect_text.trim());
    apply_mana_mode_restrictions(&mut effects, source, &restrictions);
    let lower_source = source.to_ascii_lowercase();
    Some(OracleActivatedAbility {
        mana_per_spell_cast: lower_source.contains("for each")
            && lower_source.contains("spell")
            && lower_source.contains("cast"),
        is_mana_ability: is_mana_ability(source, &costs, &effects),
        costs,
        effects,
        restrictions,
    })
}

/// Apply the CR 605.1a criteria to one activated ability.
fn is_mana_ability(source: &str, costs: &[ActivationCost], effects: &[OracleEffect]) -> bool {
    let lower = source.to_ascii_lowercase();
    !lower.contains("target")
        && !costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Loyalty(_)))
        && costs.iter().all(|cost| match cost {
            ActivationCost::Other(text) => !text.to_ascii_lowercase().contains("library"),
            _ => true,
        })
        && effects.iter().any(|effect| {
            matches!(
                effect,
                OracleEffect::Mana(_) | OracleEffect::ManaPerCounter(_)
            )
        })
        && !effects.iter().any(|effect| match effect {
            OracleEffect::Search(_)
            | OracleEffect::Draw(_)
            | OracleEffect::DrawThenDiscard(_)
            | OracleEffect::DrawAndMinusCounter
            | OracleEffect::Scry(_)
            | OracleEffect::Surveil(_)
            | OracleEffect::Mill(_) => true,
            OracleEffect::Unsupported(text) => text.to_ascii_lowercase().contains("library"),
            _ => false,
        })
}

/// Keep spending limits and activation gates on their own mana mode.
fn apply_mana_mode_restrictions(
    effects: &mut [OracleEffect],
    source: &str,
    restrictions: &[AbilityRestriction],
) {
    let lower = source.to_ascii_lowercase();
    let spend = super::land::spend_restriction(&lower);
    let cannot_pay_generic = lower.contains("this mana can't be spent to pay generic mana costs");
    let gated = restrictions
        .iter()
        .any(|restriction| matches!(restriction, AbilityRestriction::Condition(_)));
    for effect in effects {
        let (OracleEffect::Mana(yield_) | OracleEffect::ManaPerCounter(yield_)) = effect else {
            continue;
        };
        yield_.restriction = spend;
        yield_.cannot_pay_generic = cannot_pay_generic;
        if gated {
            yield_.choice = yield_.fixed.map(|pips| pips > 0);
            yield_.fixed = [0; 5];
        }
    }
}

/// Strip a "Power-up —" / "Exhaust —" prefix from an activation line and
/// return the restrictions that prefix implies.
fn strip_activation_keyword_prefix(source: &str) -> (&str, Vec<AbilityRestriction>) {
    let lower = source.to_ascii_lowercase();
    for (head, restriction) in [
        ("power-up", AbilityRestriction::PowerUp),
        ("exhaust", AbilityRestriction::OncePerGame),
    ] {
        let Some(rest) = lower.strip_prefix(head) else {
            continue;
        };
        if !(rest.is_empty()
            || rest.starts_with(' ')
            || rest.starts_with('—')
            || rest.starts_with('–')
            || rest.starts_with('-'))
        {
            continue;
        }
        // Byte length is stable across the ASCII-lowercase copy, so the
        // body starts right after the prefix in the original slice.
        let body = source[head.len()..].trim_start_matches([' ', '—', '–', '-']);
        return (
            body.trim(),
            vec![restriction, AbilityRestriction::OncePerGame],
        );
    }
    (source, Vec::new())
}

/// Return true for an activated-ability cost prefix.
pub(super) fn is_activation_start(text: &str) -> bool {
    let text = text.trim_start_matches('(').trim_start();
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
        || is_keyword_prefixed_activation(&lower)
}

/// True for "Power-up — …:" and "Exhaust — …:" activation lines
/// (CR 702.193 / 702.177). The prefix must be followed by a colon
/// somewhere on the line to be an activation, not a keyword list.
fn is_keyword_prefixed_activation(lower: &str) -> bool {
    ["power-up", "exhaust"].iter().any(|head| {
        lower
            .strip_prefix(head)
            .is_some_and(|rest| rest.contains(':') && !rest.contains(", "))
    })
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
    append_unknown_costs(source, &mut costs);
    if costs.is_empty() {
        if matches!(source.trim(), "0" | "{0}") {
            costs.push(ActivationCost::Mana(super::super::model::Cost::default()));
        } else {
            costs.push(ActivationCost::Other(source.to_string()));
        }
    }
    costs
}

/// Preserve any cost component that the runtime cannot pay in full.
fn append_unknown_costs(source: &str, costs: &mut Vec<ActivationCost>) {
    for component in source.split([',', ';']) {
        let component = component
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches("and ")
            .trim();
        if component.is_empty() || supported_cost_component(component) {
            continue;
        }
        costs.push(ActivationCost::Other(component.to_string()));
    }
}

/// Whether one printed cost component maps to a runtime payment.
fn supported_cost_component(component: &str) -> bool {
    let lower = component.to_ascii_lowercase();
    let mana = super::cost::parse_activation_cost(component);
    mana.total() > 0
        && component.chars().all(|character| {
            character.is_ascii_whitespace() || "{}WUBRGCXYZS/P0123456789".contains(character)
        })
        || lower.contains("{t}") && (lower.starts_with("{t}") || lower.starts_with("tap "))
        || lower.contains("{q}") && (lower.starts_with("{q}") || lower.starts_with("untap "))
        || lower.starts_with("sacrifice this ")
        || lower.starts_with("sacrifice a creature")
        || lower.starts_with("sacrifice another creature")
        || lower.starts_with("pay ") && lower.contains("life")
        || lower.contains("{e}")
        || lower.starts_with("remove one charge counter")
        || lower.starts_with("remove a charge counter")
        || component
            .trim()
            .replace(['−', '–'], "-")
            .parse::<i32>()
            .is_ok()
        || matches!(component.trim(), "0" | "{0}")
}

/// Add a nonzero mana cost when Oracle text contains mana symbols.
fn append_mana_cost(source: &str, costs: &mut Vec<ActivationCost>) {
    let mana = super::cost::parse_activation_cost(source);
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
            object: if lower.contains("sacrifice this ") {
                CostObject::Source
            } else if lower.contains("creature") {
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
    // Energy cost: "Pay {E}{E}". The symbols carry the count.
    if let Some(tail) = lower.split_once("pay ").map(|(_, tail)| tail.trim_start())
        && tail.contains("{e}")
    {
        costs.push(ActivationCost::Energy(tail.matches("{e}").count() as u32));
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
    // "activate only if" clause does (CR 602.5b).
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
