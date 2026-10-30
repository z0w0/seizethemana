//! Per-ability compiler evidence. Recognition alone is not execution support.

use super::oracle_ast::tribal::OracleTribalEffect;
use super::oracle_ast::{
    OracleAbility, OracleCard, OracleEffect, OracleSourceSpan, OracleTriggerEvent,
    OracleTriggeredAbility,
};
use crate::db::CardRow;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Hash exact simulation inputs in a key-sorted JSON object.
pub(super) fn card_source_hash(row: &CardRow) -> String {
    let fields = BTreeMap::from([
        ("oracle_id", serde_json::json!(row.oracle_id)),
        ("name", serde_json::json!(row.name)),
        ("mana_cost", serde_json::json!(row.mana_cost)),
        ("cmc", serde_json::json!(row.cmc)),
        ("type_line", serde_json::json!(row.type_line)),
        ("colors", serde_json::json!(row.colors)),
        ("color_identity", serde_json::json!(row.color_identity)),
        ("keywords", serde_json::json!(row.keywords)),
        ("power", serde_json::json!(row.power)),
        ("toughness", serde_json::json!(row.toughness)),
        ("loyalty", serde_json::json!(row.loyalty)),
        ("oracle_text", serde_json::json!(row.oracle_text)),
        ("rarity", serde_json::json!(row.rarity)),
    ]);
    let source =
        serde_json::to_vec(&fields).expect("card source JSON contains only serializable values");
    format!("{:x}", Sha256::digest(source))
}

/// Lowering evidence for one Oracle ability, not a claim about the whole card.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AbilitySupport {
    /// Position in the parsed ability list, including synthesized abilities.
    pub ability_index: usize,
    /// Original Oracle wording, before source-name normalization.
    pub source: String,
    /// Byte range in the original text. Synthesized abilities have no range.
    pub span: Option<OracleSourceSpan>,
    /// What this compiler emitted and which approximation applies.
    pub status: AbilitySupportStatus,
}

/// Compiler evidence only. Runtime correctness needs a separate rules audit.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AbilitySupportStatus {
    /// Complete typed lowering exists; this is not proof of exact runtime rules.
    Supported,
    /// Typed effects exist, but ordinary runtime execution is not verified.
    CompilerOnly {
        /// Runtime path that still needs an execution audit.
        reason: String,
    },
    /// Lowering uses a declared runtime simplification.
    Approximated {
        /// Stable identifier for the relevant simulator approximation.
        rule_id: &'static str,
    },
    /// No complete executable ability was emitted.
    Unsupported {
        /// Requirement or lowering gap that prevents the ability from running.
        reason: String,
    },
}

/// Build evidence from actual lowering results rather than parser matches.
pub(super) fn inventory(oracle: &OracleCard) -> Vec<AbilitySupport> {
    oracle
        .abilities
        .iter()
        .enumerate()
        .map(|(ability_index, ability)| {
            let source = oracle.ability_sources.get(ability_index);
            AbilitySupport {
                ability_index,
                source: source.map_or_else(
                    || "Keyword-derived ability".to_string(),
                    |source| source.text.clone(),
                ),
                span: source.and_then(|source| source.span),
                status: support_status(oracle, ability),
            }
        })
        .collect()
}

/// Explain an omitted requirement before considering runtime approximations.
fn support_status(oracle: &OracleCard, ability: &OracleAbility) -> AbilitySupportStatus {
    if let Some(status) = blocked_requirement_status(oracle, ability) {
        return status;
    }
    let effects = match ability {
        OracleAbility::Activated(ability) => &ability.effects,
        OracleAbility::Triggered(ability) => &ability.effects,
        OracleAbility::Spell(ability) => &ability.effects,
        OracleAbility::SagaChapter(ability) => &ability.effects,
        OracleAbility::Unsupported(ability)
            if oracle.has_keyword(&super::oracle_ast::OracleKeywordName::Warp)
                && ability.source.to_ascii_lowercase().starts_with("warp ") =>
        {
            return unsupported(
                "Warp casting is disabled because its end-step exile and later cast from exile are not implemented. The normal printed cost remains legal.",
            );
        }
        OracleAbility::Unsupported(_) => {
            return unsupported("The Oracle statement has no executable grammar rule.");
        }
        OracleAbility::Static(ability)
            if !ability.effects.is_empty()
                && ability.effects.iter().all(|effect| {
                    matches!(
                        effect,
                        super::oracle_ast::OracleStaticEffect::SpellCostReduction(_)
                    )
                }) =>
        {
            return AbilitySupportStatus::CompilerOnly {
                reason: "Spell-cost selectors lower to live card-level reduction fields. This record does not verify complete casting interactions.".to_string(),
            };
        }
        OracleAbility::Static(_) => {
            return if ability.to_runtime().is_empty() {
                unsupported(
                    "No runtime ability was emitted. Separate card-level static fields are not verified by this record.",
                )
            } else {
                AbilitySupportStatus::Approximated {
                    rule_id: "upkeep_counter_threshold",
                }
            };
        }
    };
    let complete = match ability {
        OracleAbility::Spell(_) => {
            super::oracle_lower::lower_spell_effect_sequence(oracle, effects).is_some()
                && !effects.is_empty()
        }
        OracleAbility::SagaChapter(_) => super::oracle_lower::lower_effect_sequence(effects)
            .is_some_and(|effects| !effects.is_empty()),
        _ => !ability.to_runtime().is_empty(),
    };
    if matches!(ability, OracleAbility::Spell(_))
        && effects.is_empty()
        && (oracle.spell_data.additional_cost.is_some()
            || oracle.spell_data.behold_alternative.is_some())
    {
        return AbilitySupportStatus::CompilerOnly {
            reason: "Additional cost lowered to cast payment fields, not resolution effects."
                .to_string(),
        };
    }
    if !complete {
        return unsupported(
            "A required cost, condition, event, target, payment, or effect cannot lower as a complete ability.",
        );
    }
    if matches!(
        ability,
        OracleAbility::Spell(_) | OracleAbility::SagaChapter(_)
    ) {
        if oracle.spell_data.search_after_sacrifice.is_some() {
            return AbilitySupportStatus::CompilerOnly {
                reason: "Linked sacrifice search lowers only to its typed cast-resolution rule, not a second generic search.".to_string(),
            };
        }
        if oracle
            .spell_data
            .additional_cost
            .is_some_and(|cost| cost.sacrifice_artifacts_or_creatures > 0)
        {
            return AbilitySupportStatus::CompilerOnly {
                reason: "The cast pays the creature sacrifice route. Artifact-only sacrifice payment is not implemented.".to_string(),
            };
        }
        return AbilitySupportStatus::CompilerOnly {
            reason: "Ordered effects lowered, but their ordinary runtime resolution path is not verified.".to_string(),
        };
    }
    if let OracleAbility::Triggered(trigger) = ability
        && let Some(status) = trigger_runtime_status(trigger)
    {
        return status;
    }
    if let Some(rule_id) = effects.iter().find_map(approximation) {
        return AbilitySupportStatus::Approximated { rule_id };
    }
    AbilitySupportStatus::Supported
}

/// A missing dependency takes priority over every approximation or support claim.
fn blocked_requirement_status(
    oracle: &OracleCard,
    ability: &OracleAbility,
) -> Option<AbilitySupportStatus> {
    if matches!(ability, OracleAbility::Spell(_)) {
        if oracle.spell_data.unsupported_additional_cost {
            return Some(unsupported(
                "A required additional cast cost cannot be paid by the simulator.",
            ));
        }
        if oracle.abilities.iter().any(|ability| {
            matches!(ability, OracleAbility::Spell(spell)
            if super::oracle_lower::lower_spell_effect_sequence(oracle, &spell.effects).is_none())
        }) {
            return Some(unsupported(
                "A required clause blocks this spell's complete resolution.",
            ));
        }
    }
    if let OracleAbility::Triggered(trigger) = ability {
        if matches!(
            trigger.resolution_payment,
            Some(super::oracle_ast::OracleResolutionPayment::PayMana(_))
        ) && !matches!(
            trigger.event,
            OracleTriggerEvent::CastsSpell { this_spell: false }
        ) {
            return Some(unsupported(
                "Controller-paid resolution is only implemented for own spell-cast triggers.",
            ));
        }
        if oracle.static_data.unsupported_cumulative_upkeep
            && matches!(
                trigger.event,
                OracleTriggerEvent::OpponentDraw | OracleTriggerEvent::OpponentSpellCast { .. }
            )
        {
            return Some(unsupported(
                "The source has an unsupported cumulative upkeep payment.",
            ));
        }
        if let Some(filter) = &trigger.event_filter.unsupported {
            return Some(unsupported(&format!(
                "Unsupported event restriction: {filter}"
            )));
        }
    }
    None
}

/// Separate trigger lowering evidence from runtime approximations and audits.
fn trigger_runtime_status(trigger: &OracleTriggeredAbility) -> Option<AbilitySupportStatus> {
    let rule_id = match trigger.event {
        OracleTriggerEvent::WinsAtCounters { .. } => Some("upkeep_counter_threshold"),
        OracleTriggerEvent::OpponentDraw | OracleTriggerEvent::OpponentSpellCast { .. } => {
            Some("synthetic_opponent_events")
        }
        _ => None,
    };
    if let Some(rule_id) = rule_id {
        return Some(AbilitySupportStatus::Approximated { rule_id });
    }
    if trigger.resolution_payment.is_some() || trigger.event_filter != Default::default() {
        return Some(AbilitySupportStatus::CompilerOnly {
            reason: "Event filters or controller payments lowered, but their complete runtime path is not verified.".to_string(),
        });
    }
    None
}

/// Identify effects whose runtime omits printed characteristics or choices.
fn approximation(effect: &OracleEffect) -> Option<&'static str> {
    match effect {
        OracleEffect::CreateTokens(_)
        | OracleEffect::CreateTokensPerOpponent { .. }
        | OracleEffect::Tribal(OracleTribalEffect::CreateCreatureTokens { .. }) => {
            Some("generic_creature_tokens")
        }
        OracleEffect::Search(_)
        | OracleEffect::MultiSearch(_)
        | OracleEffect::SearchThenBeholdUntap { .. } => Some("bounded_search_selection"),
        OracleEffect::ReturnFromGraveyard { .. } => Some("pilot_card_selection"),
        OracleEffect::DiscardHandThenDraw => Some("fixed_seven_card_wheel"),
        OracleEffect::Explore
        | OracleEffect::Connive(_)
        | OracleEffect::DrawThenDiscard(_)
        | OracleEffect::Scry(_)
        | OracleEffect::Surveil(_)
        | OracleEffect::LookAtTop { .. }
        | OracleEffect::PutHandOnTop(_)
        | OracleEffect::DrawDiscard { .. } => Some("pilot_card_selection"),
        OracleEffect::Proliferate => Some("modeled_counter_kinds_only"),
        _ => None,
    }
}

/// Keep unsupported reasons consistent across support records.
fn unsupported(reason: &str) -> AbilitySupportStatus {
    AbilitySupportStatus::Unsupported {
        reason: reason.to_string(),
    }
}
