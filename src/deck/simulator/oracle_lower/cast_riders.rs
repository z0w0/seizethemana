//! Parse one-shot effects and extra cast costs from Oracle data.

use super::super::model::Cost;
use super::super::oracle_ast::{OracleCard, OracleSpellData};
use crate::db::CardRow;

/// Collect one-shot effects from a spell or its self-cast trigger.
fn cast_effects(
    oracle: &super::super::oracle_ast::OracleCard,
) -> Vec<&super::super::oracle_ast::OracleEffect> {
    let mut effects = Vec::new();
    for ability in &oracle.abilities {
        match ability {
            super::super::oracle_ast::OracleAbility::Spell(spell) => effects.extend(&spell.effects),
            super::super::oracle_ast::OracleAbility::Triggered(trigger)
                if matches!(
                    &trigger.event,
                    super::super::oracle_ast::OracleTriggerEvent::CastsSpell { this_spell: true }
                ) =>
            {
                effects.extend(&trigger.effects);
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
    let draws_on_cast = effects.iter().map(|effect| effect_draw_count(effect)).sum();
    let life_gain_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::GainLife(amount) => total + amount,
        _ => total,
    });
    let tokens_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::CreateTokens(amount) => total + amount,
        _ => total,
    });
    let treasures_on_cast = effects.iter().fold(0, |total, effect| match effect {
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
    let extra_turns_on_cast = effects
        .iter()
        .any(|effect| matches!(effect, super::super::oracle_ast::OracleEffect::ExtraTurn));
    let life_loss_on_resolve = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::LoseLife { amount, .. } => total + amount,
        _ => total,
    });
    let life_loss_scope = effects
        .iter()
        .find_map(|effect| match effect {
            super::super::oracle_ast::OracleEffect::LoseLife { scope, .. } => Some(*scope),
            _ => None,
        })
        .unwrap_or_default();
    let damage_on_resolve = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Damage {
            amount,
            target: super::super::oracle_ast::DamageTarget::Player,
            ..
        } => total + amount,
        _ => total,
    });
    let damage_scope = effects
        .iter()
        .find_map(|effect| match effect {
            super::super::oracle_ast::OracleEffect::Damage {
                scope,
                target: super::super::oracle_ast::DamageTarget::Player,
                ..
            } => Some(*scope),
            _ => None,
        })
        .unwrap_or_default();
    let mills_on_enter = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Mill(amount) => total + amount,
        _ => total,
    });
    let energy_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Energy(amount) => total + amount,
        _ => total,
    });
    let discard_hand_then_draw_seven_on_cast = effects.iter().any(|effect| {
        matches!(
            effect,
            super::super::oracle_ast::OracleEffect::DiscardHandThenDraw
        )
    });
    // Keyword actions that ride a spell's resolution (amass, empower
    // Jace, the Ring tempts, explore, connive).
    let amass_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Amass(amount) => total.max(*amount),
        _ => total,
    });
    let empower_jace_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::EmpowerJace(amount) => total.max(*amount),
        _ => total,
    });
    let tempts_ring_on_cast = effects
        .iter()
        .any(|effect| matches!(effect, super::super::oracle_ast::OracleEffect::RingTempts));
    let explores_on_cast = effects
        .iter()
        .filter(|effect| matches!(effect, super::super::oracle_ast::OracleEffect::Explore))
        .count() as u32;
    let connives_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Connive(count) => total.max(*count),
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
    super::super::model::SimSpellData {
        mana_on_cast,
        draws_on_cast,
        life_gain_on_cast,
        alternative_cast_cost: spell_data.alternative_cost,
        reveal_rule: spell_data.reveal_rule,
        tokens_on_cast,
        treasures_on_cast,
        scry_on_cast,
        surveils_on_cast,
        extra_turns_on_cast,
        extra_land_drops_on_cast: 0,
        life_loss_on_resolve,
        life_loss_scope,
        damage_on_resolve,
        damage_scope,
        mills_on_enter,
        energy_on_cast,
        amass_on_cast,
        empower_jace_on_cast,
        tempts_ring_on_cast,
        explores_on_cast,
        connives_on_cast,
        discard_hand_then_draw_seven_on_cast,
        additional_cost_creatures: spell_data
            .additional_cost
            .map(|cost| cost.sacrifice_creatures)
            .unwrap_or(0),
        additional_cost_discards: spell_data
            .additional_cost
            .map(|cost| cost.discard_cards)
            .unwrap_or(0),
        additional_cost_life: spell_data
            .additional_cost
            .map(|cost| cost.pay_life)
            .unwrap_or(0),
        counters_on_cast: spell_data.counters_on_cast,
        x_class,
        mana_per_cast,
        kicker: oracle.keyword_cost(&super::super::oracle_ast::OracleKeywordName::Kicker),
        search_after_sacrifice: spell_data.search_after_sacrifice,
        graveyard_creature_exchange: spell_data.graveyard_creature_exchange,
        grants_flashback: spell_data.grants_flashback,
        grants_escape: spell_data.grants_escape,
        own_flashback: oracle.keyword_cost(&super::super::oracle_ast::OracleKeywordName::Flashback),
        own_escape: oracle.keyword_cost(&super::super::oracle_ast::OracleKeywordName::Escape),
        cycling_cost: cycling_cost(oracle),
        cycling_life: oracle
            .keyword_life(&super::super::oracle_ast::OracleKeywordName::Cycling)
            .unwrap_or(0),
        landcycling_type: oracle.basic_land_type(),
    }
}

/// The cycling activation cost: the `Cycling` keyword's mana argument, or
/// the mana argument of a landcycling keyword (whose discard still counts
/// as cycling, CR 702.29).
fn cycling_cost(oracle: &super::super::oracle_ast::OracleCard) -> Option<Cost> {
    use super::super::oracle_ast::OracleKeywordName;
    [
        OracleKeywordName::Cycling,
        OracleKeywordName::BasicLandcycling,
        OracleKeywordName::Landcycling,
    ]
    .iter()
    .find_map(|name| oracle.keyword_cost(name))
}

/// Parse a cast-time reduction as a minimum cost and a board-scaling flag.
///
/// Warp (CR 702.185) is an alternative cost read from the typed `Warp`
/// keyword. Improvise (CR 702.126a) and affinity (CR 702.41a) scale with
/// the artifact board at runtime and only set the discount flag here.
pub(super) fn parse_min_cost(oracle: &OracleCard, cost: &Cost) -> (Cost, bool) {
    use super::super::oracle_ast::OracleKeywordName;

    let mut min_cost = *cost;
    let mut battlefield_discount = false;
    if let Some(warp) = oracle.keyword_cost(&OracleKeywordName::Warp)
        && warp.total() < min_cost.total()
    {
        min_cost = warp;
    }
    // Improvise and affinity scale with the artifact board at the real
    // rate: one generic less per artifact, resolved at runtime against
    // the live board (game_mana::effective_min_cost).
    if oracle.has_keyword(&OracleKeywordName::Improvise)
        || oracle.has_keyword(&OracleKeywordName::Affinity)
    {
        battlefield_discount = true;
    }
    (min_cost, battlefield_discount)
}
