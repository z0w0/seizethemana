//! Parse static card flags used for roles, mana grants, and combat.

use super::super::model::{Grant, GrantTarget, Keyword, KeywordGrant, SimStaticFlags};
use super::super::oracle_ast::tribal::OracleTribalStaticEffect;
use super::super::oracle_ast::{
    ActivationCost, ActivationTarget, OracleAbility, OracleCard, OracleKeywordName,
    OracleStaticEffect, StaticTarget,
};

/// Parse static keywords, interaction shapes, grants, and equipment data.
pub(super) fn parse_static_flags(type_line: &str, oracle: &OracleCard) -> SimStaticFlags {
    // Haste on the card itself: the keyword list, or a static grant that
    // targets the source. Grants to other creatures never set it — the
    // granter does not attack on its entry turn (CR 302.6).
    let has_haste = oracle.has_keyword(&OracleKeywordName::Haste)
        || static_grants(oracle).any(|(target, keyword)| {
            target == StaticTarget::Source && keyword.name == OracleKeywordName::Haste
        });
    let extra_land_drops = static_effects(oracle)
        .filter_map(|effect| match effect {
            OracleStaticEffect::AdditionalLandDrop(count) => Some(*count),
            _ => None,
        })
        .sum();
    let doesnt_untap = oracle.static_data.doesnt_untap;
    // Instant speed: Instant type or flash. "Flashback" contains
    // "flash" as a substring; exclude it.
    let is_instant_speed = type_line.contains("Instant") || oracle.static_data.has_flash;
    // Interaction: removal or counterspell shapes (readiness metric).
    // Sweeps count too (capacity, not events). Damage shapes match any
    // "deals N damage to target …" with N >= 2;
    // player-targeted damage stays drain. Bounce ("return target … to
    // its owner's hand") answers a threat the same way removal does.
    let sweeps = oracle.static_data.sweeps;
    let is_interaction = oracle.static_data.is_interaction;
    let mut buff = None;
    for effect in static_effects(oracle) {
        if let OracleStaticEffect::CreatureBuff { power, toughness } = effect {
            buff = Some((*power, *toughness));
        }
    }
    // Equipment: equip cost, equipped buff, Skullclamp death-draws.
    let equipment = oracle.static_data.equipment;
    // No maximum hand size (CR 402.2): a static "you have no maximum
    // hand size" rider on the permanent.
    let no_max_hand_size = oracle.static_data.no_max_hand_size;
    // "As long as this permanent is saddled, it gets +N/+N" (CR 702.171b):
    // a static buff that applies only while the saddle cost is paid.
    let saddled_buff = oracle.static_data.saddled_buff;
    // Static keyword grants: which permanents receive which keyword.
    let keyword_grants = static_grants(oracle)
        .filter_map(|(target, keyword)| {
            Some(KeywordGrant {
                target: grant_target(target),
                keyword: runtime_keyword(&keyword.name)?,
            })
        })
        .collect();
    SimStaticFlags {
        has_haste,
        extra_land_drops,
        doesnt_untap,
        is_instant_speed,
        sweeps,
        is_interaction,
        grant: static_mana_grant(oracle),
        bonus_mana_on_nonland_tap: oracle.abilities.iter().any(|ability| match ability {
            super::super::oracle_ast::OracleAbility::Triggered(trigger) => {
                matches!(
                    &trigger.event,
                    super::super::oracle_ast::OracleTriggerEvent::TappedForMana
                ) && trigger
                    .effects
                    .iter()
                    .any(|effect| matches!(effect, super::super::oracle_ast::OracleEffect::Mana(_)))
            }
            _ => false,
        }),
        requires_metalcraft: oracle.abilities.iter().any(|ability| match ability {
            super::super::oracle_ast::OracleAbility::Activated(activation) => {
                activation.restrictions.iter().any(|restriction| {
                    matches!(
                        restriction,
                        super::super::oracle_ast::AbilityRestriction::Condition(condition)
                            if condition.contains("three or more artifacts")
                    )
                })
            }
            _ => false,
        }),
        buff,
        equipment,
        keyword_grants,
        no_max_hand_size,
        saddled_buff,
    }
}

/// Lower a complete all-land or all-creature mana grant to its runtime flag.
fn static_mana_grant(oracle: &OracleCard) -> Option<Grant> {
    static_effects(oracle).find_map(|effect| {
        let OracleStaticEffect::Tribal(OracleTribalStaticEffect::ManaGrant { selector, ability }) =
            effect
        else {
            return None;
        };
        if selector.controller != super::super::oracle_ast::PlayerScope::You
            || selector.exclude_source
            || selector.untapped
            || !matches!(ability.costs.as_slice(), [ActivationCost::Tap(ActivationTarget::Source)])
            || !ability.restrictions.is_empty()
            || !matches!(ability.effects.as_slice(), [super::super::oracle_ast::OracleEffect::Mana(yield_)] if is_unrestricted_any_color(yield_))
        {
            return None;
        }
        if selector.filter.card_types == ["land"] && selector.filter.subtypes.is_empty() {
            Some(Grant::Lands)
        } else if selector.filter.card_types == ["creature"]
            && selector.filter.subtypes.is_empty()
        {
            Some(Grant::Creatures)
        } else {
            None
        }
    })
}

/// Check that the grant replaces one source tap with one unrestricted pip.
fn is_unrestricted_any_color(yield_: &super::super::model::ManaYield) -> bool {
    yield_.any_pips == 1
        && yield_.fixed.iter().all(|pip| *pip == 0)
        && !yield_.choice.iter().any(|color| *color)
        && !yield_.choice_colorless
        && !yield_.legendary_colors
        && !yield_.one_color
        && !yield_.opponent_any
        && !yield_.cannot_pay_generic
        && yield_.scaling.is_none()
        && yield_.colorless == 0
        && !yield_.alternatives
        && yield_.restriction.is_none()
}

/// Map an AST static target onto the runtime grant target.
fn grant_target(target: StaticTarget) -> GrantTarget {
    match target {
        StaticTarget::Source => GrantTarget::Source,
        StaticTarget::CreaturesYouControl => GrantTarget::CreaturesYouControl,
        StaticTarget::LandsYouControl => GrantTarget::LandsYouControl,
        StaticTarget::PermanentsYouControl => GrantTarget::PermanentsYouControl,
        StaticTarget::SpellsYouCast => GrantTarget::SpellsYouCast,
    }
}

/// Map an AST keyword name onto the runtime keyword set, when the runtime
/// models it. Keywords outside the set stay inert (documented in
/// `docs/simulator.md`).
pub(super) fn runtime_keyword(name: &OracleKeywordName) -> Option<Keyword> {
    Some(match name {
        OracleKeywordName::Flying => Keyword::Flying,
        OracleKeywordName::FirstStrike => Keyword::FirstStrike,
        OracleKeywordName::DoubleStrike => Keyword::DoubleStrike,
        OracleKeywordName::Deathtouch => Keyword::Deathtouch,
        OracleKeywordName::Lifelink => Keyword::Lifelink,
        OracleKeywordName::Menace => Keyword::Menace,
        OracleKeywordName::Reach => Keyword::Reach,
        OracleKeywordName::Trample => Keyword::Trample,
        OracleKeywordName::Vigilance => Keyword::Vigilance,
        OracleKeywordName::Haste => Keyword::Haste,
        OracleKeywordName::Prowess => Keyword::Prowess,
        OracleKeywordName::Hexproof => Keyword::Hexproof,
        OracleKeywordName::Indestructible => Keyword::Indestructible,
        _ => return None,
    })
}

/// Keyword grants carried by the card's static rules.
fn static_grants(
    oracle: &OracleCard,
) -> impl Iterator<Item = (StaticTarget, &super::super::oracle_ast::OracleKeyword)> {
    static_effects(oracle).filter_map(|effect| match effect {
        OracleStaticEffect::KeywordGrant { target, keyword } => Some((*target, keyword)),
        _ => None,
    })
}

/// Effects of the card's static rules.
fn static_effects(oracle: &OracleCard) -> impl Iterator<Item = &OracleStaticEffect> {
    oracle
        .abilities
        .iter()
        .filter_map(|ability| match ability {
            OracleAbility::Static(ability) => Some(ability.effects.iter()),
            _ => None,
        })
        .flatten()
}
