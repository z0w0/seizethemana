//! Parsing for activated abilities, their costs, and restrictions.

use super::super::oracle_ast::*;
use super::effects::{parse_activation_effects, parse_oracle_number_word};

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
    let has_x = costs
        .iter()
        .any(|cost| matches!(cost, ActivationCost::Mana(mana) if mana.x_symbols > 0));
    let effect_text = typed_condition_effect_text(effect_text, &restrictions);
    let mut effects = parse_activation_effects(effect_text.trim(), has_x);
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
                OracleEffect::Mana(_)
                    | OracleEffect::ManaPerCounter(_)
                    | OracleEffect::Tribal(
                        super::super::oracle_ast::tribal::OracleTribalEffect::Mana { .. }
                    )
            )
        })
        && !effects.iter().any(is_non_mana_effect)
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
        let (OracleEffect::Mana(yield_)
        | OracleEffect::ManaPerCounter(yield_)
        | OracleEffect::Tribal(super::super::oracle_ast::tribal::OracleTribalEffect::Mana {
            yield_,
            ..
        })) = effect
        else {
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
        || text
            .strip_prefix('0')
            .is_some_and(|tail| tail.trim_start().starts_with(&[':', ',', ';'][..]))
        || lower.starts_with("tap:")
        || lower.starts_with("tap ") && first_clause.contains(':')
        || lower.starts_with("return ") && first_clause.contains(':')
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
    super::tribal::exclude_source_tap(&mut costs);
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
    for component in activation_cost_components(source) {
        if component.is_empty() || supported_cost_component(component) {
            continue;
        }
        costs.push(ActivationCost::Other(component.to_string()));
    }
}

/// Use the same written payment boundaries for extraction and unknown-cost checks.
fn activation_cost_components(source: &str) -> impl Iterator<Item = &str> {
    source.split([',', ';']).map(|component| {
        component
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches("and ")
            .trim()
    })
}

/// Whether one printed cost component maps to a runtime payment.
fn supported_cost_component(component: &str) -> bool {
    if super::tribal::selected_permanent_cost(component).is_some() {
        return true;
    }
    let lower = component.to_ascii_lowercase();
    if lower.contains("sacrifice") {
        return parse_sacrifice_cost(&lower).is_some();
    }
    if lower.starts_with("pay ") {
        return parse_life_cost(&lower).is_some() || parse_energy_cost(&lower).is_some();
    }
    if lower.starts_with("discard ") {
        return parse_discard_cost(&lower).is_some();
    }
    if lower.starts_with("remove ") {
        return parse_counter_cost(&lower).is_some();
    }
    let mana = super::cost::parse_activation_cost(component);
    (mana.total() > 0 || mana.x_symbols > 0)
        && component.chars().all(|character| {
            character.is_ascii_whitespace() || "{}WUBRGCXYZS/P0123456789".contains(character)
        })
        || matches!(lower.as_str(), "{t}" | "{q}")
        || component
            .trim()
            .replace(['−', '–'], "-")
            .parse::<i32>()
            .is_ok()
        || matches!(component.trim(), "0" | "{0}")
}

/// Retain mana payments without treating separately paid energy as mana.
fn append_mana_cost(source: &str, costs: &mut Vec<ActivationCost>) {
    let mana_source = source.replace("{E}", "").replace("{e}", "");
    let mana = super::cost::parse_activation_cost(&mana_source);
    if mana.total() > 0 || mana.x_symbols > 0 {
        costs.push(ActivationCost::Mana(mana));
    }
}

/// Add tap and untap costs with their target scope.
fn append_tap_costs(lower: &str, costs: &mut Vec<ActivationCost>) {
    for component in lower.split([',', ';']).map(str::trim) {
        if let Some(cost) = super::tribal::selected_permanent_cost(component) {
            costs.push(ActivationCost::Tribal(cost));
            continue;
        }
        if component.contains("{t}") || component.starts_with("tap") {
            costs.push(ActivationCost::Tap(activation_target(component)));
        }
        if component.contains("{q}") || component.contains("untap ") {
            costs.push(ActivationCost::Untap(activation_target(component)));
        }
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

/// Retain one complete sacrifice payment; multiple clauses remain unsupported.
fn append_sacrifice_cost(lower: &str, costs: &mut Vec<ActivationCost>) {
    if lower.matches("sacrifice").count() > 1 {
        costs.push(ActivationCost::Other(lower.to_string()));
        return;
    }
    for component in lower.split([',', ';']) {
        if let Some(cost) = parse_sacrifice_cost(component.trim().trim_start_matches("and ")) {
            costs.push(cost);
        }
    }
}

/// Preserve exact sacrifice selectors without dropping required qualifiers.
fn parse_sacrifice_cost(component: &str) -> Option<ActivationCost> {
    let selector = component.strip_prefix("sacrifice ")?;
    if matches!(
        selector,
        "this creature"
            | "this artifact"
            | "this enchantment"
            | "this land"
            | "this planeswalker"
            | "this battle"
            | "this permanent"
            | "this token"
            | "this food"
            | "this clue"
            | "this treasure"
    ) {
        return Some(ActivationCost::Sacrifice {
            count: 1,
            object: CostObject::Source,
            exclude_source: false,
        });
    }
    let selector = selector.strip_suffix(" you control").unwrap_or(selector);
    let (amount, object) = selector.split_once(' ')?;
    let object = match object {
        "creature" | "creatures" => CostObject::Creature,
        "artifact" | "artifacts" | "enchantment" | "enchantments" | "land" | "lands"
        | "permanent" | "permanents" => CostObject::Any,
        _ => return None,
    };
    let exclude_source = amount == "another";
    let count = if exclude_source {
        1
    } else {
        parse_oracle_number_word(amount).filter(|count| *count > 0)?
    };
    Some(ActivationCost::Sacrifice {
        count,
        object,
        exclude_source,
    })
}

/// Retain every complete life and discard payment instead of only the first one.
fn append_life_and_discard_costs(lower: &str, costs: &mut Vec<ActivationCost>) {
    for component in activation_cost_components(lower) {
        if let Some(cost) = parse_life_cost(component).or_else(|| parse_discard_cost(component)) {
            costs.push(cost);
        }
    }
}

/// Accept fixed life payments without dropping a scale or a condition.
fn parse_life_cost(component: &str) -> Option<ActivationCost> {
    let amount = component.strip_prefix("pay ")?.strip_suffix(" life")?;
    fixed_payment_count(amount).map(ActivationCost::PayLife)
}

/// Keep complete fixed discard costs typed, even though the runtime cannot pay them.
fn parse_discard_cost(component: &str) -> Option<ActivationCost> {
    let amount = component.strip_prefix("discard ")?;
    let amount = amount
        .strip_suffix(" cards")
        .or_else(|| amount.strip_suffix(" card"))?;
    fixed_payment_count(amount)
        .filter(|count| *count > 0)
        .map(ActivationCost::Discard)
}

/// Require a whole number token so trailing payment requirements remain unknown.
fn fixed_payment_count(amount: &str) -> Option<u32> {
    if !amount
        .chars()
        .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }
    parse_oracle_number_word(amount)
}

/// Add a counter-removal cost and retain its counter name.
fn append_counter_cost(lower: &str, costs: &mut Vec<ActivationCost>) {
    for component in activation_cost_components(lower) {
        if let Some(cost) = parse_counter_cost(component) {
            costs.push(cost);
        }
    }
    // Energy cost: "Pay {E}{E}". The symbols carry the count.
    if let Some(tail) = lower.split_once("pay ").map(|(_, tail)| tail.trim_start())
        && tail.contains("{e}")
    {
        costs.push(ActivationCost::Energy(tail.matches("{e}").count() as u32));
    }
}

/// Retain fixed counter removal only when its complete source selector is known.
fn parse_counter_cost(component: &str) -> Option<ActivationCost> {
    let payment = component.strip_prefix("remove ")?;
    let (amount, counter) = payment.split_once(' ')?;
    let count = fixed_payment_count(amount).filter(|count| *count > 0)?;
    let (kind, suffix) = counter.split_once(" counter")?;
    let source = suffix.strip_prefix('s').unwrap_or(suffix);
    if kind.is_empty()
        || kind.contains(char::is_whitespace)
        || !matches!(
            source,
            "" | " from this artifact"
                | " from this creature"
                | " from this enchantment"
                | " from this land"
                | " from this permanent"
                | " from this planeswalker"
                | " from this battle"
                | " from this vehicle"
        )
    {
        return None;
    }
    Some(ActivationCost::RemoveCounter {
        kind: Some(kind.to_string()),
        count,
    })
}

/// Accept the complete adjacent energy symbols, without ignoring a required qualifier.
fn parse_energy_cost(component: &str) -> Option<u32> {
    let payment = component.strip_prefix("pay ")?;
    let count = payment.matches("{e}").count();
    if count == 0 || !payment.replace("{e}", "").is_empty() {
        return None;
    }
    u32::try_from(count).ok()
}

/// Keep sole loyalty costs executable and preserve compound loyalty requirements as unsupported.
fn append_loyalty_cost(source: &str, costs: &mut Vec<ActivationCost>) {
    if let Some(loyalty) = parse_oracle_loyalty_cost(source) {
        costs.push(ActivationCost::Loyalty(loyalty));
        return;
    }
    for component in activation_cost_components(source) {
        if parse_oracle_loyalty_cost(component).is_some() {
            costs.push(ActivationCost::Other(component.to_string()));
        }
    }
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
        let condition = condition.trim_end_matches('.');
        restrictions.push(
            super::land::parse_controlled_land_types(condition)
                .map(AbilityRestriction::ControlsLandTypes)
                .unwrap_or_else(|| AbilityRestriction::Condition(condition.to_string())),
        );
    }
    restrictions
}

/// A complete typed activation gate is not a second resolution instruction.
fn typed_condition_effect_text<'a>(text: &'a str, restrictions: &[AbilityRestriction]) -> &'a str {
    if restrictions
        .iter()
        .any(|restriction| matches!(restriction, AbilityRestriction::ControlsLandTypes(_)))
    {
        let lower = text.to_ascii_lowercase();
        if let Some(position) = lower.find(". activate only if ") {
            return &text[..position];
        }
    }
    text
}
