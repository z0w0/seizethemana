//! Pinned and renamed controls for scoped lords, untaps, grants, and equipment events.

use super::tribal_parser_tests::{activation, assert_inert, pinned};
use super::*;
use crate::deck::simulator::deck_test_support::card;
use crate::deck::simulator::oracle_ast::tribal::*;
use crate::deck::simulator::oracle_parser::parse_oracle_card;

/// Arbor Elf and Copperhorn keep target and source-exclusion requirements.
#[test]
fn untaps_distinguish_target_forests_from_other_controlled_creatures() {
    for renamed in [false, true] {
        let row = pinned("Arbor Elf", renamed);
        let oracle = parse_oracle_card(&row);
        let [
            OracleEffect::Tribal(OracleTribalEffect::Untap(OraclePermanentSelection::Target(
                selector,
            ))),
        ] = activation(&oracle).effects.as_slice()
        else {
            panic!("Forest target");
        };
        assert_eq!(selector.filter.card_types, ["land"]);
        assert_eq!(selector.filter.subtypes, ["forest"]);
        assert_eq!(selector.controller, PlayerScope::Any);
        assert!(!selector.exclude_source);
        assert_inert(&row, "untap effects");

        let row = pinned("Copperhorn Scout", renamed);
        let oracle = parse_oracle_card(&row);
        let [OracleAbility::Triggered(trigger)] = oracle.abilities.as_slice() else {
            panic!("attack trigger");
        };
        assert_eq!(
            trigger.event,
            OracleTriggerEvent::Attacks(ObjectSubject::ThisPermanent)
        );
        let [
            OracleEffect::Tribal(OracleTribalEffect::Untap(OraclePermanentSelection::Each(
                selector,
            ))),
        ] = trigger.effects.as_slice()
        else {
            panic!("each other creature");
        };
        assert_eq!(selector.filter.card_types, ["creature"]);
        assert_eq!(selector.controller, PlayerScope::You);
        assert!(selector.exclude_source);
        assert_inert(&row, "untap effects");
    }
}

/// Quirion Ranger retains its Forest return, creature target, and once-per-turn limit.
#[test]
fn filtered_return_cost_and_targeted_untap_remain_one_inert_activation() {
    for renamed in [false, true] {
        let row = pinned("Quirion Ranger", renamed);
        let oracle = parse_oracle_card(&row);
        let ability = activation(&oracle);
        let [ActivationCost::Tribal(OracleTribalCost::ReturnToHand { count: 1, selector })] =
            ability.costs.as_slice()
        else {
            panic!("Forest return payment");
        };
        assert_eq!(selector.filter.card_types, ["land"]);
        assert_eq!(selector.filter.subtypes, ["forest"]);
        assert_eq!(selector.controller, PlayerScope::You);
        assert!(
            ability
                .restrictions
                .contains(&AbilityRestriction::OncePerTurn)
        );
        assert!(
            matches!(ability.effects.first(), Some(OracleEffect::Tribal(OracleTribalEffect::Untap(OraclePermanentSelection::Target(selector)))) if selector.filter.card_types == ["creature"] && selector.controller == PlayerScope::Any)
        );
        assert_inert(&row, "return-to-hand costs");
    }
}

/// A scoped lord cannot become a bonus for every creature or its own source.
#[test]
fn elf_lords_keep_subtypes_control_scope_and_source_exclusion() {
    for renamed in [false, true] {
        for name in ["Leaf-Crowned Visionary", "Elvish Archdruid"] {
            let row = pinned(name, renamed);
            let oracle = parse_oracle_card(&row);
            let lord = oracle
                .abilities
                .iter()
                .find_map(|ability| match ability {
                    OracleAbility::Static(ability) => ability.effects.first(),
                    _ => None,
                })
                .expect("lord rule");
            let OracleStaticEffect::Tribal(OracleTribalStaticEffect::CreatureBuff {
                selector,
                power: 1,
                toughness: 1,
            }) = lord
            else {
                panic!("scoped bonus");
            };
            assert_eq!(selector.filter.subtypes, ["elf"]);
            assert_eq!(selector.controller, PlayerScope::You);
            assert!(selector.exclude_source);
            assert_eq!(
                selector.filter.card_types,
                if name == "Elvish Archdruid" {
                    vec!["creature"]
                } else {
                    vec![]
                }
            );
            let lowered = parse_sim_card(&row);
            assert!(lowered.flags.buff.is_none());
        }
    }
}

/// Existing paid Elf-cast draws keep their filter, payment, and complete reward.
#[test]
fn leaf_crowned_visionary_paid_draw_stays_separate_from_its_inert_lord() {
    for renamed in [false, true] {
        let row = pinned("Leaf-Crowned Visionary", renamed);
        let oracle = parse_oracle_card(&row);
        let draw = oracle
            .abilities
            .iter()
            .find_map(|ability| match ability {
                OracleAbility::Triggered(trigger) => Some(trigger),
                _ => None,
            })
            .expect("paid cast draw");
        assert_eq!(draw.event_filter.subtypes, ["elf"]);
        assert!(
            matches!(draw.resolution_payment, Some(OracleResolutionPayment::PayMana(cost)) if cost.pips[4] == 1)
        );
        assert!(matches!(draw.effects.as_slice(), [OracleEffect::Draw(1)]));
        let lowered = draw.to_runtime_all();
        assert_eq!(lowered.len(), 1);
        assert_eq!(lowered[0].event_filter.subtypes, ["elf"]);
        assert!(
            matches!(lowered[0].resolution_payment, Some(SimResolutionPayment::PayMana(cost)) if cost.pips[4] == 1)
        );
    }
}

/// A mana grant retains its full yield instead of setting a broad mana flag.
#[test]
fn elven_chorus_and_renamed_grants_keep_complete_activated_mana_abilities() {
    for renamed in [false, true] {
        let row = pinned("Elven Chorus", renamed);
        let oracle = parse_oracle_card(&row);
        let (selector, ability) = oracle
            .abilities
            .iter()
            .find_map(|ability| match ability {
                OracleAbility::Static(ability) => {
                    ability.effects.iter().find_map(|effect| match effect {
                        OracleStaticEffect::Tribal(OracleTribalStaticEffect::ManaGrant {
                            selector,
                            ability,
                        }) => Some((selector, ability)),
                        _ => None,
                    })
                }
                _ => None,
            })
            .expect("complete mana grant");
        assert_eq!(selector.filter.card_types, ["creature"]);
        assert_eq!(selector.controller, PlayerScope::You);
        assert!(matches!(
            ability.costs.as_slice(),
            [ActivationCost::Tap(ActivationTarget::Source)]
        ));
        assert!(
            matches!(ability.effects.as_slice(), [OracleEffect::Mana(mana)] if mana.any_pips == 1)
        );
        assert_eq!(
            parse_sim_card(&row).flags.grant,
            Some(crate::deck::simulator::model::Grant::Creatures)
        );
    }
    for (production, fixed, any_pips) in [
        ("{G}", [0, 0, 0, 0, 1], 0),
        ("{W}{G}", [1, 0, 0, 0, 1], 0),
        ("one mana of any color", [0; 5], 1),
    ] {
        let row = card(
            "Different source",
            "{G}",
            "Enchantment",
            &format!("Other Scout creatures you control have \"{{T}}: Add {production}.\""),
        );
        let oracle = parse_oracle_card(&row);
        let [OracleAbility::Static(rule)] = oracle.abilities.as_slice() else {
            panic!("grant");
        };
        assert!(
            matches!(rule.effects.as_slice(), [OracleStaticEffect::Tribal(OracleTribalStaticEffect::ManaGrant { selector, ability })] if selector.exclude_source && selector.filter.subtypes == ["scout"] && matches!(ability.effects.as_slice(), [OracleEffect::Mana(mana)] if mana.fixed == fixed && mana.any_pips == any_pips))
        );
        assert_inert(&row, "granted mana abilities");
    }
}

/// Skullclamp listens for its equipped creature, not every creature death.
#[test]
fn equipped_creature_death_retains_the_relationship_without_global_draws() {
    for renamed in [false, true] {
        let row = pinned("Skullclamp", renamed);
        let oracle = parse_oracle_card(&row);
        let trigger = oracle
            .abilities
            .iter()
            .find_map(|ability| match ability {
                OracleAbility::Triggered(ability) => Some(ability),
                _ => None,
            })
            .expect("equipped death trigger");
        assert_eq!(trigger.event, OracleTriggerEvent::EquippedCreatureDies);
        assert!(matches!(
            trigger.effects.as_slice(),
            [OracleEffect::Draw(2)]
        ));
        assert!(trigger.to_runtime_all().is_empty());
        assert_inert(&row, "equipped-creature death");
    }
}

/// Extra filters, unknown grant clauses, and unmodeled returns stay atomic.
#[test]
fn scoped_near_matches_never_grant_unrestricted_bonuses_or_mana() {
    for text in [
        "Other Elf creatures you control get +1/+1 until end of turn.",
        "Other Elf creatures you control get +1/+1 and have flying.",
        "Other Elf creatures in your graveyard get +1/+1.",
        "Creatures you control have \"{T}: Add {G}. Draw a card.\"",
        "Creatures you control have \"{T}: Add {G}. Activate only if you control an Elf.\"",
        "{T}: Untap target Forest with mana value 0.",
        "{T}: Untap each other creature you control unless an opponent pays {1}.",
        "Return up to one Forest you control to its owner's hand: Untap target creature.",
        "Whenever another equipped creature dies, draw two cards.",
    ] {
        let row = card("Different source", "{G}", "Creature — Scout", text);
        let lowered = parse_sim_card(&row);
        assert!(
            lowered.unlocked_abilities(u32::MAX).next().is_none(),
            "{text}"
        );
        assert!(lowered.flags.buff.is_none(), "{text}");
        assert!(lowered.flags.grant.is_none(), "{text}");
    }
}
