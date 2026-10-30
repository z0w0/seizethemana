//! Explicit lowering from Oracle spell selectors to runtime cost reductions.

use crate::db::CardRow;
use crate::deck::simulator::model::{SpellCostCardType, SpellCostFilter, SpellCostReduction};
use crate::deck::simulator::oracle_ast::{
    OracleAbility, OracleCard, OracleSpellCostReduction, OracleSpellFilter, OracleSpellType,
    OracleStaticEffect,
};

/// Convert one typed reduction without parsing source text or choosing a card name.
pub(super) fn lower_spell_cost_reduction(
    reduction: &OracleSpellCostReduction,
) -> SpellCostReduction {
    let filter = match &reduction.filter {
        OracleSpellFilter::Any => SpellCostFilter::Any,
        OracleSpellFilter::Types(types) => {
            SpellCostFilter::Types(types.iter().copied().map(lower_spell_type).collect())
        }
        OracleSpellFilter::Colors(colors) => SpellCostFilter::Colors(colors.clone()),
    };
    SpellCostReduction {
        amount: reduction.amount,
        filter,
    }
}

/// Keep only front-face static reductions; a hidden face cannot grant an ability.
pub(super) fn lower_spell_cost_reductions(
    row: &CardRow,
    oracle: &OracleCard,
) -> Vec<SpellCostReduction> {
    let multiple_faces = row.type_line.contains(" // ");
    let separator = row.oracle_text.find("//");
    oracle
        .abilities
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            !multiple_faces
                || separator.is_some_and(|separator| {
                    oracle
                        .ability_sources
                        .get(*index)
                        .and_then(|source| source.span)
                        .is_some_and(|span| span.end <= separator)
                })
        })
        .filter_map(|(_, ability)| match ability {
            OracleAbility::Static(ability) => Some(&ability.effects),
            _ => None,
        })
        .flatten()
        .filter_map(|effect| match effect {
            OracleStaticEffect::SpellCostReduction(reduction) => {
                Some(lower_spell_cost_reduction(reduction))
            }
            _ => None,
        })
        .collect()
}

/// Translate each supported Oracle card type into its runtime counterpart.
fn lower_spell_type(kind: OracleSpellType) -> SpellCostCardType {
    match kind {
        OracleSpellType::Artifact => SpellCostCardType::Artifact,
        OracleSpellType::Battle => SpellCostCardType::Battle,
        OracleSpellType::Creature => SpellCostCardType::Creature,
        OracleSpellType::Enchantment => SpellCostCardType::Enchantment,
        OracleSpellType::Instant => SpellCostCardType::Instant,
        OracleSpellType::Planeswalker => SpellCostCardType::Planeswalker,
        OracleSpellType::Sorcery => SpellCostCardType::Sorcery,
    }
}

/// Check typed lowering and front-face source boundaries separately from casting.
#[cfg(test)]
#[path = "tests/cost_reduction_tests.rs"]
mod cost_reduction_tests;
