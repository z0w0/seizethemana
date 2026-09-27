//! Parse static card flags used for roles, mana grants, and combat.

use super::super::model::Grant;
use super::super::oracle_ast::{
    KeywordName, OracleAbility, OracleCard, StaticEffect, StaticTarget,
};
use crate::db::CardRow;

/// Static card flags read from the keywords array and the oracle text.
pub(super) struct StaticFlags {
    pub(super) double_strike: bool,
    pub(super) prowess: bool,
    pub(super) landfall: bool,
    pub(super) evasion: bool,
    pub(super) has_haste: bool,
    pub(super) extra_land_drops: bool,
    pub(super) is_instant_speed: bool,
    pub(super) wipe: bool,
    pub(super) is_interaction: bool,
    pub(super) grant: Option<Grant>,
    pub(super) bonus_mana_on_nonland_tap: bool,
    pub(super) requires_metalcraft: bool,
    pub(super) buff: Option<(i32, i32)>,
    pub(super) equipment: Option<super::super::model::Equipment>,
    pub(super) treasures_on_token: bool,
}

/// Parse static keywords, interaction shapes, grants, and equipment data.
pub(super) fn parse_static_flags(
    row: &CardRow,
    text: &str,
    type_line: &str,
    cast_face: bool,
    oracle: &OracleCard,
) -> StaticFlags {
    let double_strike =
        oracle.has_keyword(&KeywordName::DoubleStrike) || text.contains("double strike");
    let prowess = oracle.has_keyword(&KeywordName::Prowess) || text.contains("prowess");
    let landfall = text.contains("landfall");
    // Evasion census: the card's own keyword list (text keyword lines and
    // the Scryfall array both land there) plus static keyword grants of an
    // evasion keyword — a granter joins the evasive-body census when it
    // attacks.
    let evasion = oracle.has_keyword(&KeywordName::Trample)
        || oracle.has_keyword(&KeywordName::Flying)
        || oracle.has_keyword(&KeywordName::Menace)
        || static_grants(oracle).any(|(_, keyword)| {
            matches!(
                keyword.name,
                KeywordName::Trample | KeywordName::Flying | KeywordName::Menace
            )
        });
    // Haste on the card itself: the keyword list, or a static grant that
    // targets the source. Grants to other creatures never set it — the
    // granter does not attack on its entry turn (CR 302.6).
    let has_haste = oracle.has_keyword(&KeywordName::Haste)
        || static_grants(oracle).any(|(target, keyword)| {
            target == StaticTarget::Source && keyword.name == KeywordName::Haste
        });
    // "You may play an additional land" / "an additional land on each of
    // your turns" (Aesi, Wayward Swordtooth, Burrowing Power).
    let extra_land_drops =
        static_effects(oracle).any(|effect| matches!(effect, StaticEffect::AdditionalLandDrop));
    // Instant speed: Instant type or flash. "Flashback" contains
    // "flash" as a substring; exclude it.
    let is_instant_speed = type_line.contains("Instant")
        || oracle.has_keyword(&super::super::oracle_ast::KeywordName::Flash)
        || (text.contains("flash") && !text.contains("flashback"));
    // Interaction: removal or counterspell shapes (readiness metric).
    // Wipes count too (capacity, not events). Damage shapes match any
    // "deals N damage to target …" with N >= 2;
    // player-targeted damage stays drain. Bounce ("return target … to
    // its owner's hand") answers a threat the same way removal does.
    let wipe = cast_face
        && (text.contains("destroy all")
            || text.contains("exile all")
            || text.contains("return all")
            || text.contains("sacrifice all")
            || (text.contains("each creature") && text.contains("-x/-x")));
    let damage_removal = super::parse_oracle_damage_removal_shape(&row.oracle_text);
    let is_interaction = type_line.contains("Instant") || type_line.contains("Sorcery");
    let is_interaction = is_interaction
        && (text.contains("destroy target")
            || text.contains("exile target")
            || text.contains("counter target")
            || text.contains("return target")
                && (text.contains("to its owner's hand")
                    || text.contains("to their owner's hand"))
            || damage_removal
            || wipe);
    let mut grant = None;
    let mut buff = None;
    for (target, effect) in static_grants_and_effects(oracle) {
        match effect {
            // The grant's mana yield is runtime data the pool code does not
            // read yet; the target class is the consumed part.
            StaticEffect::ManaGrant { .. } => {
                grant = match target {
                    StaticTarget::LandsYouControl => Some(Grant::Lands),
                    StaticTarget::CreaturesYouControl => Some(Grant::Creatures),
                    _ => grant,
                };
            }
            StaticEffect::CreatureBuff { power, toughness } => {
                buff = Some((*power, *toughness));
            }
            _ => {}
        }
    }
    // Equipment: equip cost, equipped buff, Skullclamp death-draws.
    let equipment = if type_line.contains("Equipment") {
        super::super::parse_keywords::parse_equipment(text)
    } else {
        None
    };
    // Treasure creation: "create a Treasure token" / "create N Treasure
    // tokens". Each treasure is a banked flexible pip.
    let treasures_on_token = text.contains("treasure token");
    StaticFlags {
        double_strike,
        prowess,
        landfall,
        evasion,
        has_haste,
        extra_land_drops,
        is_instant_speed,
        wipe,
        is_interaction,
        grant,
        bonus_mana_on_nonland_tap: oracle.abilities.iter().any(|ability| match ability {
            super::super::oracle_ast::OracleAbility::Triggered(trigger) => {
                matches!(
                    &trigger.event,
                    super::super::oracle_ast::TriggerEvent::TappedForMana {
                        nonland: true,
                        player: super::super::oracle_ast::PlayerScope::You
                    }
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
        treasures_on_token,
    }
}

/// Every static effect on the card, with its grant target when the effect
/// carries one.
fn static_grants_and_effects(
    oracle: &OracleCard,
) -> impl Iterator<Item = (StaticTarget, &StaticEffect)> {
    static_effects(oracle).map(|effect| {
        (
            match effect {
                StaticEffect::KeywordGrant { target, .. }
                | StaticEffect::ManaGrant { target, .. } => *target,
                _ => StaticTarget::Source,
            },
            effect,
        )
    })
}

/// Keyword grants carried by the card's static rules.
fn static_grants(
    oracle: &OracleCard,
) -> impl Iterator<Item = (StaticTarget, &super::super::oracle_ast::KeywordAbility)> {
    static_effects(oracle).filter_map(|effect| match effect {
        StaticEffect::KeywordGrant { target, keyword } => Some((*target, keyword)),
        _ => None,
    })
}

/// Effects of the card's static rules.
fn static_effects(oracle: &OracleCard) -> impl Iterator<Item = &StaticEffect> {
    oracle
        .abilities
        .iter()
        .filter_map(|ability| match ability {
            OracleAbility::Static(ability) => Some(ability.effects.iter()),
            _ => None,
        })
        .flatten()
}
