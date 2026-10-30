//! Explicit lowering for Behold qualities and their complete linked effects.

use crate::deck::simulator::model::behold::{BeholdAlternative, BeholdSpec};
use crate::deck::simulator::model::{SearchCardType, SearchDestination, SearchSpec, SimEffect};
use crate::deck::simulator::oracle_ast::OracleEffect;
use crate::deck::simulator::oracle_ast::behold::{OracleBeholdAlternative, OracleBeholdSpec};

/// Lower one quality without treating a counted Behold instruction as singular.
pub(in crate::deck::simulator) fn lower_spec(spec: &OracleBeholdSpec) -> Option<BeholdSpec> {
    if spec.count != 1 || spec.card_types.is_empty() && spec.subtypes.is_empty() {
        return None;
    }
    Some(BeholdSpec {
        card_types: spec.card_types.clone(),
        subtypes: spec.subtypes.clone(),
        count: spec.count,
    })
}

/// Lower a complete Behold-or-mana cost without dropping its extra payment.
pub(in crate::deck::simulator) fn lower_alternative(
    cost: &OracleBeholdAlternative,
) -> Option<BeholdAlternative> {
    Some(BeholdAlternative {
        spec: lower_spec(&cost.spec)?,
        generic: cost.generic,
    })
}

/// Preserve the Behold gate and reject the whole branch if any reward is unsupported.
pub(in crate::deck::simulator) fn lower_conditional(
    spec: &OracleBeholdSpec,
    effects: &[OracleEffect],
) -> Option<SimEffect> {
    Some(SimEffect::Behold {
        spec: lower_spec(spec)?,
        effects: super::lower_effect_sequence_with_fixed_tokens(effects)?,
    })
}

/// Keep the untap instruction linked to this search's battlefield result.
pub(in crate::deck::simulator) fn lower_search_then_untap(
    search: &SearchSpec,
    spec: &OracleBeholdSpec,
) -> Option<SimEffect> {
    if search.destination != SearchDestination::BattlefieldTapped
        || !matches!(
            search.card_type,
            Some(SearchCardType::Land | SearchCardType::BasicLand)
        )
    {
        return None;
    }
    Some(SimEffect::SearchThenBeholdUntap {
        search: *search,
        spec: lower_spec(spec)?,
    })
}

/// Typed controls prevent broader qualities and partial conditional rewards.
#[cfg(test)]
#[path = "tests/behold_tests.rs"]
mod behold_tests;
