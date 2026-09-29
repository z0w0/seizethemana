//! Parse static card flags used for roles, mana grants, and combat.

use super::super::model::{Grant, GrantTarget, Keyword, KeywordGrant, SimStaticFlags};
use super::super::oracle_ast::{
    OracleAbility, OracleCard, OracleKeywordName, OracleStaticEffect, StaticTarget,
};

/// Parse static keywords, interaction shapes, grants, and equipment data.
pub(super) fn parse_static_flags(type_line: &str, oracle: &OracleCard) -> SimStaticFlags {
    let landfall = oracle.static_data.landfall;
    // Haste on the card itself: the keyword list, or a static grant that
    // targets the source. Grants to other creatures never set it — the
    // granter does not attack on its entry turn (CR 302.6).
    let has_haste = oracle.has_keyword(&OracleKeywordName::Haste)
        || static_grants(oracle).any(|(target, keyword)| {
            target == StaticTarget::Source && keyword.name == OracleKeywordName::Haste
        });
    // "You may play an additional land" / "an additional land on each of
    // your turns" (Aesi, Wayward Swordtooth, Burrowing Power).
    let extra_land_drops = static_effects(oracle)
        .any(|effect| matches!(effect, OracleStaticEffect::AdditionalLandDrop));
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
    let mut grant = None;
    let mut buff = None;
    for (target, effect) in static_grants_and_effects(oracle) {
        match effect {
            // The grant's mana yield is runtime data the pool code does not
            // read yet; the target class is the consumed part.
            OracleStaticEffect::ManaGrant { .. } => {
                grant = match target {
                    StaticTarget::LandsYouControl => Some(Grant::Lands),
                    StaticTarget::CreaturesYouControl => Some(Grant::Creatures),
                    _ => grant,
                };
            }
            OracleStaticEffect::CreatureBuff { power, toughness } => {
                buff = Some((*power, *toughness));
            }
            _ => {}
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
        landfall,
        has_haste,
        extra_land_drops,
        doesnt_untap,
        is_instant_speed,
        sweeps,
        is_interaction,
        grant,
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

/// Every static effect on the card, with its grant target when the effect
/// carries one.
fn static_grants_and_effects(
    oracle: &OracleCard,
) -> impl Iterator<Item = (StaticTarget, &OracleStaticEffect)> {
    static_effects(oracle).map(|effect| {
        (
            match effect {
                OracleStaticEffect::KeywordGrant { target, .. }
                | OracleStaticEffect::ManaGrant { target, .. } => *target,
                _ => StaticTarget::Source,
            },
            effect,
        )
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
