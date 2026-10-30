//! Parse one-shot effects and extra cast costs from Oracle data.

use super::super::model::Cost;
use super::super::oracle_ast::{OracleCard, OracleSpellData};
use crate::db::CardRow;

/// Collect one-shot effects from a spell or its self-cast trigger.
fn cast_effects(
    oracle: &super::super::oracle_ast::OracleCard,
) -> Vec<super::super::oracle_ast::OracleEffect> {
    let mut effects = Vec::new();
    for ability in &oracle.abilities {
        match ability {
            super::super::oracle_ast::OracleAbility::Spell(spell) => {
                if super::lower_spell_effect_sequence(oracle, &spell.effects).is_none() {
                    return Vec::new();
                }
                effects.extend(
                    spell
                        .effects
                        .iter()
                        .filter(|effect| !super::modeled_spell_annotation(oracle, effect))
                        .cloned(),
                );
            }
            super::super::oracle_ast::OracleAbility::Triggered(trigger)
                if matches!(
                    &trigger.event,
                    super::super::oracle_ast::OracleTriggerEvent::CastsSpell { this_spell: true }
                ) && trigger.condition.is_none()
                    && trigger.resolution_payment.is_none()
                    && trigger.event_filter.unsupported.is_none()
                    && super::lower_effect_sequence_with_fixed_tokens(&trigger.effects)
                        .is_some() =>
            {
                effects.extend(trigger.effects.iter().cloned());
            }
            _ => {}
        }
    }
    effects
}

/// Sum the draw count carried by parsed spell effects.
fn effect_draw_count(effect: &super::super::oracle_ast::OracleEffect) -> u32 {
    match effect {
        super::super::oracle_ast::OracleEffect::Draw(count)
        | super::super::oracle_ast::OracleEffect::DrawThenDiscard(count) => *count,
        super::super::oracle_ast::OracleEffect::DrawDiscard { draw, .. } => *draw,
        super::super::oracle_ast::OracleEffect::DrawAndMinusCounter => 1,
        _ => 0,
    }
}

/// Lower supported spell effects into one-shot card fields.
pub(super) fn parse_cast_riders(
    row: &CardRow,
    cast_face: bool,
    tap_is_none: bool,
    oracle: &super::super::oracle_ast::OracleCard,
) -> super::super::model::SimSpellData {
    let effects = if cast_face {
        cast_effects(oracle)
    } else {
        Vec::new()
    };
    let mana_on_cast = if tap_is_none {
        effects.iter().find_map(|effect| match effect {
            super::super::oracle_ast::OracleEffect::Mana(yield_) => Some(yield_.clone()),
            _ => None,
        })
    } else {
        None
    };
    let mut draws_on_cast = effects.iter().map(effect_draw_count).sum();
    let life_gain_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::GainLife(amount) => total + amount,
        _ => total,
    });
    let mut tokens_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::CreateTokens(amount) => total + amount,
        _ => total,
    });
    let mut treasures_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::CreateTreasureTokens(amount) => total + amount,
        _ => total,
    });
    let scry_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Scry(count) => total + count,
        _ => total,
    });
    let surveils_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Surveil(count) => total + count,
        _ => total,
    });
    let mut life_loss_on_resolve = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::LoseLife { amount, .. } => total + amount,
        _ => total,
    });
    let mut damage_on_resolve = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Damage {
            amount,
            target: super::super::oracle_ast::DamageTarget::Player,
            ..
        } => total + amount,
        _ => total,
    });
    let has_x_cost = row.mana_cost.to_ascii_uppercase().contains("{X}");
    let spell_data = if cast_face {
        oracle.spell_data.clone()
    } else {
        OracleSpellData::default()
    };
    let x_class = if cast_face && has_x_cost {
        spell_data.x_class
    } else {
        None
    };
    match x_class {
        Some(super::super::model::XClass::Drain) => {
            life_loss_on_resolve = 0;
            damage_on_resolve = 0;
        }
        Some(super::super::model::XClass::Draw) => draws_on_cast = 0,
        Some(super::super::model::XClass::Tokens) => tokens_on_cast = 0,
        Some(super::super::model::XClass::Treasures) => treasures_on_cast = 0,
        _ => {}
    }
    // Repeatable per-cast mana: "add {N} for each spell you've cast this
    // turn". Fires per spell cast while the host is untapped.
    let mana_per_cast = cast_face
        .then(|| {
            oracle.abilities.iter().find_map(|ability| match ability {
                super::super::oracle_ast::OracleAbility::Activated(ability)
                    if ability.mana_per_spell_cast =>
                {
                    ability.effects.iter().find_map(|effect| match effect {
                        super::super::oracle_ast::OracleEffect::Mana(yield_)
                        | super::super::oracle_ast::OracleEffect::ManaPerCounter(yield_) => {
                            Some(yield_.clone())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
        })
        .flatten();
    let cycling = cycling_keyword(oracle);
    super::super::model::SimSpellData {
        behold_alternative: spell_data
            .behold_alternative
            .as_ref()
            .and_then(super::behold::lower_alternative),
        unsupported_additional_cost: oracle.spell_data.unsupported_additional_cost,
        effects: super::lower_effect_sequence_with_fixed_tokens(&effects)
            .expect("cast effects are checked before collection"),
        mana_on_cast,
        draws_on_cast,
        life_gain_on_cast,
        alternative_cast_cost: spell_data.alternative_cost,
        reveal_rule: spell_data.reveal_rule,
        tokens_on_cast,
        treasures_on_cast,
        scry_on_cast,
        surveils_on_cast,
        extra_land_drops_on_cast: 0,
        life_loss_on_resolve,
        damage_on_resolve,
        additional_cost_creatures: spell_data
            .additional_cost
            .map(|cost| cost.sacrifice_creatures + cost.sacrifice_artifacts_or_creatures)
            .unwrap_or(0),
        additional_cost_discards: spell_data
            .additional_cost
            .map(|cost| cost.discard_cards)
            .unwrap_or(0),
        additional_cost_life: spell_data
            .additional_cost
            .map(|cost| cost.pay_life)
            .unwrap_or(0),
        x_class,
        mana_per_cast,
        kicker: oracle.keyword_cost(&super::super::oracle_ast::OracleKeywordName::Kicker),
        search_after_sacrifice: spell_data.search_after_sacrifice.filter(|_| {
            oracle.abilities.iter().all(|ability| {
                !matches!(ability, super::super::oracle_ast::OracleAbility::Spell(spell)
                if super::lower_spell_effect_sequence(oracle, &spell.effects).is_none())
            })
        }),
        graveyard_creature_exchange: spell_data.graveyard_creature_exchange,
        grants_flashback: spell_data.grants_flashback,
        grants_escape: spell_data.grants_escape,
        own_flashback: oracle.keyword_cost(&super::super::oracle_ast::OracleKeywordName::Flashback),
        own_escape: escape_cost(oracle),
        cycling_cost: cycling.and_then(|keyword| oracle.keyword_cost(&keyword.name)),
        cycling_life: cycling
            .filter(|keyword| keyword.name == super::super::oracle_ast::OracleKeywordName::Cycling)
            .and_then(|keyword| oracle.keyword_life(&keyword.name))
            .unwrap_or(0),
        landcycling_type: cycling
            .filter(|keyword| {
                keyword.name == super::super::oracle_ast::OracleKeywordName::Landcycling
            })
            .and_then(|_| oracle.basic_land_type()),
    }
}

/// Lower Escape only when its complete printed graveyard payment matches
/// the runtime's fixed three-card payment.
fn escape_cost(oracle: &super::super::oracle_ast::OracleCard) -> Option<Cost> {
    use super::super::oracle_ast::{KeywordArgument, OracleKeywordName};

    let keyword = oracle
        .keywords
        .iter()
        .find(|keyword| keyword.name == OracleKeywordName::Escape)?;
    match keyword.arguments.as_slice() {
        [KeywordArgument::Mana(cost), KeywordArgument::EscapeCards(3)]
        | [KeywordArgument::EscapeCards(3), KeywordArgument::Mana(cost)]
            if !cost.unsupported && cost.snow == 0 && cost.hybrid[0] == 0 =>
        {
            Some(*cost)
        }
        _ => None,
    }
}

/// Admit only complete cycling payments and named land selectors the runtime can execute.
/// Basic and untyped landcycling have no selector. Composite payments stay inert.
fn cycling_keyword(
    oracle: &super::super::oracle_ast::OracleCard,
) -> Option<&super::super::oracle_ast::OracleKeyword> {
    use super::super::oracle_ast::{KeywordArgument, OracleKeywordName};

    oracle.keywords.iter().find(
        |keyword| match (&keyword.name, keyword.arguments.as_slice()) {
            (OracleKeywordName::Cycling, [KeywordArgument::Mana(cost)])
            | (
                OracleKeywordName::Landcycling,
                [
                    KeywordArgument::BasicLandType(_),
                    KeywordArgument::Mana(cost),
                ],
            ) => !cost.unsupported && cost.x_symbols == 0 && cost.snow == 0 && cost.hybrid[0] == 0,
            (OracleKeywordName::Cycling, [KeywordArgument::Life(life)]) => *life > 0,
            _ => false,
        },
    )
}

/// Keep the normal cast cost and mark artifact-based payment for ranking.
pub(super) fn parse_min_cost(oracle: &OracleCard, cost: &Cost) -> (Cost, bool) {
    use super::super::oracle_ast::OracleKeywordName;

    let battlefield_discount = oracle.has_keyword(&OracleKeywordName::Improvise)
        || super::keyword_lower::has_artifact_affinity(oracle);
    (*cost, battlefield_discount)
}
