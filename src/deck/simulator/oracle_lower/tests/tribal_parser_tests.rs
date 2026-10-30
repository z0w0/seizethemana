//! Pinned and renamed grammar controls for complete tribal payments and quantities.

use super::*;
use crate::deck::simulator::deck_test_support::card;
use crate::deck::simulator::model::{ManaColor, ManaYield};
use crate::deck::simulator::oracle_ast::tribal::*;
use crate::deck::simulator::oracle_parser::parse_oracle_card;
use crate::deck::simulator::support::AbilitySupportStatus;

/// Read exact pinned wording, then optionally replace every source-name reference.
pub(super) fn pinned(name: &str, renamed: bool) -> CardRow {
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../tests/popular_card_fixtures/lathril_rows.json"
    ))
    .expect("pinned Lathril rows");
    let stored = rows
        .iter()
        .find(|row| row["name"] == name)
        .expect("pinned card");
    let mut row = card(
        name,
        stored["mana_cost"].as_str().expect("mana cost"),
        stored["type_line"].as_str().expect("type line"),
        stored["oracle_text"].as_str().expect("Oracle text"),
    );
    row.keywords = stored["keywords"]
        .as_str()
        .expect("keyword metadata")
        .to_string();
    if renamed {
        row.oracle_text = row.oracle_text.replace(name, "Renamed source");
        if let Some((short, _)) = name.split_once(',') {
            row.oracle_text = row.oracle_text.replace(short, "Renamed source");
        }
        row.name = "Renamed source".to_string();
    }
    row
}

/// Require one parsed activation rather than treating a missing ability as success.
pub(super) fn activation(oracle: &OracleCard) -> &OracleActivatedAbility {
    oracle
        .abilities
        .iter()
        .find_map(|ability| match ability {
            OracleAbility::Activated(ability) => Some(ability),
            _ => None,
        })
        .expect("one parsed activation")
}

/// Require explicit unsupported evidence and no executable payment or reward.
pub(super) fn assert_inert(row: &CardRow, reason: &str) {
    let lowered = parse_sim_card(row);
    assert!(lowered.unlocked_abilities(u32::MAX).next().is_none());
    assert!(lowered.spell_data.effects.is_empty());
    assert!(
        lowered.support.iter().any(|support| matches!(
            &support.status, AbilitySupportStatus::Unsupported { reason: actual }
                if actual.contains(reason)
        )),
        "{}: {reason}",
        row.oracle_text
    );
}

/// Combat damage supplies Lathril's token count without inventing a fixed amount.
#[test]
fn lathril_tokens_keep_event_quantity_and_complete_creature_data() {
    for renamed in [false, true] {
        let row = pinned("Lathril, Blade of the Elves", renamed);
        let oracle = parse_oracle_card(&row);
        let trigger = oracle
            .abilities
            .iter()
            .find_map(|ability| match ability {
                OracleAbility::Triggered(trigger) => Some(trigger),
                _ => None,
            })
            .expect("combat-damage trigger");
        assert_eq!(
            trigger.event,
            OracleTriggerEvent::CombatDamageToPlayer(ObjectSubject::ThisPermanent)
        );
        let [OracleEffect::Tribal(OracleTribalEffect::CreateCreatureTokens { quantity, token })] =
            trigger.effects.as_slice()
        else {
            panic!("complete creature-token effect");
        };
        assert!(matches!(quantity, OracleQuantity::TriggerCombatDamage));
        assert_eq!((token.power, token.toughness), (1, 1));
        assert_eq!(token.colors, [ManaColor::Green]);
        assert_eq!(token.subtypes, ["elf", "warrior"]);
        assert!(!token.tapped && !token.attacking);
        assert_inert(&row, "Token quantities");
    }
}

/// Written taps count distinct permanents and do not become source tap symbols.
#[test]
fn counted_taps_keep_source_eligibility_and_every_required_payment() {
    for renamed in [false, true] {
        for (name, count, source_tap, types, subtypes) in [
            ("Heritage Druid", 3, false, vec![], vec!["elf"]),
            ("Jaspera Sentinel", 1, true, vec!["creature"], vec![]),
            ("Lathril, Blade of the Elves", 10, true, vec![], vec!["elf"]),
        ] {
            let row = pinned(name, renamed);
            let oracle = parse_oracle_card(&row);
            let ability = activation(&oracle);
            assert_eq!(
                ability
                    .costs
                    .iter()
                    .any(|cost| matches!(cost, ActivationCost::Tap(ActivationTarget::Source))),
                source_tap
            );
            assert_eq!(ability.costs.len(), 1 + usize::from(source_tap));
            let (actual, selector) = ability
                .costs
                .iter()
                .find_map(|cost| match cost {
                    ActivationCost::Tribal(OracleTribalCost::TapPermanents { count, selector }) => {
                        Some((*count, selector))
                    }
                    _ => None,
                })
                .expect("written tap payment");
            assert_eq!(actual, count);
            assert_eq!(selector.filter.card_types, types);
            assert_eq!(selector.filter.subtypes, subtypes);
            assert_eq!(selector.controller, PlayerScope::You);
            assert!(selector.untapped);
            assert_eq!(selector.exclude_source, source_tap);
            match name {
                "Heritage Druid" => assert!(
                    matches!(ability.effects.as_slice(), [OracleEffect::Mana(mana)] if mana.fixed == [0, 0, 0, 0, 3])
                ),
                "Jaspera Sentinel" => assert!(
                    matches!(ability.effects.as_slice(), [OracleEffect::Mana(mana)] if mana.any_pips == 1)
                ),
                _ => assert!(matches!(
                    ability.effects.as_slice(),
                    [
                        OracleEffect::LoseLife {
                            amount: 10,
                            scope: crate::deck::simulator::model::LifeLossScope::EachOpponent
                        },
                        OracleEffect::GainLife(10)
                    ]
                )),
            }
            assert_inert(&row, "Selected-permanent");
        }
    }
}

/// Priest, Archdruid, and Marwyn retain different mana multipliers.
#[test]
fn native_scaled_mana_keeps_battlefield_controller_and_source_power_scope() {
    for renamed in [false, true] {
        for (name, controller) in [
            ("Priest of Titania", Some(PlayerScope::Any)),
            ("Elvish Archdruid", Some(PlayerScope::You)),
            ("Marwyn, the Nurturer", None),
        ] {
            let row = pinned(name, renamed);
            let oracle = parse_oracle_card(&row);
            let ability = activation(&oracle);
            assert!(ability.is_mana_ability);
            let [OracleEffect::Tribal(OracleTribalEffect::Mana { yield_, quantity })] =
                ability.effects.as_slice()
            else {
                panic!("complete mana quantity: {name}");
            };
            assert_eq!(yield_.fixed, [0, 0, 0, 0, 1]);
            match (quantity, controller) {
                (OracleQuantity::MatchingPermanents(selector), Some(controller)) => {
                    assert_eq!(selector.controller, controller);
                    assert_eq!(selector.filter.subtypes, ["elf"]);
                    assert!(!selector.exclude_source);
                }
                (OracleQuantity::SourcePower, None) => {}
                _ => panic!("wrong quantity: {name}"),
            }
            assert_inert(
                &row,
                if name == "Elvish Archdruid" {
                    "Scoped creature"
                } else {
                    "mana quantities"
                },
            );
        }
    }
}

/// Fixed and matching token quantities retain their full printed token definition.
#[test]
fn token_quantities_keep_fixed_counts_and_matching_permanent_selectors() {
    for (text, matching) in [
        (
            "Create two 2/3 green and white Scout creature tokens.",
            false,
        ),
        (
            "Create a 1/1 green Elf creature token for each Elf you control.",
            true,
        ),
    ] {
        let row = card("Different source", "{G}", "Sorcery", text);
        let oracle = parse_oracle_card(&row);
        let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
            panic!("spell");
        };
        let [OracleEffect::Tribal(OracleTribalEffect::CreateCreatureTokens { quantity, token })] =
            spell.effects.as_slice()
        else {
            panic!("complete token");
        };
        if matching {
            assert!(
                matches!(quantity, OracleQuantity::MatchingPermanents(selector) if selector.controller == PlayerScope::You && selector.filter.subtypes == ["elf"])
            );
        } else {
            assert!(matches!(quantity, OracleQuantity::Fixed(2)));
            assert_eq!((token.power, token.toughness), (2, 3));
            assert_eq!(token.colors, [ManaColor::Green, ManaColor::White]);
            assert_eq!(token.subtypes, ["scout"]);
        }
        assert_inert(&row, "Token quantities");
    }
}

/// Native mana bundles keep all simultaneous colors rather than one generic yield.
#[test]
fn fixed_mana_bundles_remain_complete_and_independent_from_new_scaling() {
    let row = card(
        "Different source",
        "{G}",
        "Creature — Scout",
        "{T}: Add {W}{G}{G}.",
    );
    let oracle = parse_oracle_card(&row);
    assert!(matches!(
        activation(&oracle).effects.as_slice(),
        [OracleEffect::Mana(ManaYield {
            fixed: [1, 0, 0, 0, 2],
            ..
        })]
    ));
    let scaled = card(
        "Different source",
        "{G}",
        "Creature — Scout",
        "{T}: Add {W}{G}{G} for each Scout you control.",
    );
    let oracle = parse_oracle_card(&scaled);
    assert!(
        matches!(activation(&oracle).effects.as_slice(), [OracleEffect::Tribal(OracleTribalEffect::Mana { yield_: ManaYield { fixed: [1, 0, 0, 0, 2], .. }, quantity: OracleQuantity::MatchingPermanents(selector) })] if selector.filter.subtypes == ["scout"])
    );
    assert_inert(&scaled, "mana quantities");
}

/// Unknown counts, zones, token properties, and conditions cannot emit free rewards.
#[test]
fn tribal_near_matches_stay_atomic_and_inert() {
    for text in [
        "Tap up to three untapped Elves you control: Add {G}{G}{G}.",
        "Tap three tapped Elves you control: Add {G}{G}{G}.",
        "Tap three untapped Elves an opponent controls: Add {G}{G}{G}.",
        "{T}: Add {G} for each Elf in your graveyard.",
        "{T}: Add an amount of {G} equal to target creature's power.",
        "{T}: Create that many 1/1 green Elf Warrior creature tokens.",
        "Create a 1/1 green and Elf creature token.",
        "Create a 1/1 green white Elf creature token.",
        "Create a 1/1 green Elf creature token with flying.",
        "Create a 1/1 green artifact Elf creature token.",
        "Create two 1/1 green Elf Warrior tokens.",
        "Create a 1/1 green Elf creature token for each Elf you control unless an opponent pays {1}.",
        "Whenever this creature attacks, create that many 1/1 green Elf Warrior creature tokens.",
    ] {
        let row = card("Different source", "{G}", "Creature — Scout", text);
        let lowered = parse_sim_card(&row);
        assert!(
            lowered.unlocked_abilities(u32::MAX).next().is_none(),
            "{text}"
        );
        assert!(lowered.spell_data.effects.is_empty(), "{text}");
        assert!(
            lowered
                .support
                .iter()
                .all(|support| matches!(support.status, AbilitySupportStatus::Unsupported { .. })),
            "{text}"
        );
    }
}
