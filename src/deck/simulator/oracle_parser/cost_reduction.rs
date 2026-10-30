//! Complete grammar for unconditional flat generic spell-cost reductions.

use crate::deck::simulator::model::ManaColor;
use crate::deck::simulator::oracle_ast::{
    OracleSpellCostReduction, OracleSpellFilter, OracleSpellType,
};

/// Parse one whole flat generic reduction that applies to its controller's spells.
pub(super) fn parse_cost_reduction(source: &str) -> Option<OracleSpellCostReduction> {
    let lower = source.trim().to_ascii_lowercase();
    let text = lower.strip_suffix('.').unwrap_or(&lower);
    let (filter, cost) = if let Some(cost) = text
        .strip_prefix("spells you cast cost ")
        .or_else(|| text.strip_prefix("all spells cost "))
    {
        (OracleSpellFilter::Any, cost)
    } else if let Some(relative) = text.strip_prefix("each spell you cast that's ") {
        let (selector, cost) = relative.split_once(" costs ")?;
        let colors = parse_union(selector, parse_color, &["or"])?;
        (OracleSpellFilter::Colors(colors), cost)
    } else {
        let (selector, cost) = text.split_once(" spells you cast cost ")?;
        (parse_spell_filter(selector)?, cost)
    };
    let amount = cost.strip_prefix('{')?.strip_suffix("} less to cast")?;
    if amount.is_empty() || !amount.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(OracleSpellCostReduction {
        amount: amount.parse().ok()?,
        filter,
    })
}

/// Accept complete type unions and color unions without mixed selectors.
fn parse_spell_filter(selector: &str) -> Option<OracleSpellFilter> {
    if let Some(types) = parse_union(selector, parse_spell_type, &["and", "or"]) {
        return Some(OracleSpellFilter::Types(types));
    }
    parse_union(selector, parse_color, &["or"]).map(OracleSpellFilter::Colors)
}

/// Require each selector word and connector to belong to one supported grammar.
fn parse_union<T>(
    selector: &str,
    parse: fn(&str) -> Option<T>,
    connectors: &[&str],
) -> Option<Vec<T>> {
    let mut words = selector.split_whitespace();
    let mut values = vec![parse(words.next()?)?];
    while let Some(connector) = words.next() {
        if !connectors.contains(&connector) {
            return None;
        }
        values.push(parse(words.next()?)?);
    }
    Some(values)
}

/// Card types with a supported front-face runtime selector.
fn parse_spell_type(word: &str) -> Option<OracleSpellType> {
    match word {
        "artifact" => Some(OracleSpellType::Artifact),
        "battle" => Some(OracleSpellType::Battle),
        "creature" => Some(OracleSpellType::Creature),
        "enchantment" => Some(OracleSpellType::Enchantment),
        "instant" => Some(OracleSpellType::Instant),
        "planeswalker" => Some(OracleSpellType::Planeswalker),
        "sorcery" => Some(OracleSpellType::Sorcery),
        _ => None,
    }
}

/// Color words used by single-color and color-union reductions.
fn parse_color(word: &str) -> Option<ManaColor> {
    ManaColor::ALL
        .into_iter()
        .find(|color| color.name() == word)
}

/// Check whole-ability parsing against source cards and unsupported near matches.
#[cfg(test)]
#[path = "tests/cost_reduction_tests.rs"]
mod cost_reduction_tests;
