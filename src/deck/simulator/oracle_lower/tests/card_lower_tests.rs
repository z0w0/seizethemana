//! Isolate typed lowering from source parsing, then check pinned integration inputs.

use crate::deck::simulator::card_test_support::named_row;
use crate::deck::simulator::model::{
    Cost, ManaYield, MultiSearchSpec, SearchCardType, SearchDestination, SearchSpec,
    SimActivationCost, SimEffect,
};
use crate::deck::simulator::oracle_ast::{
    ActivationCost, ActivationTarget, CostObject, OracleAbility, OracleActivatedAbility,
    OracleCard, OracleEffect,
};
use crate::deck::simulator::oracle_lower::parse_sim_card;
use crate::deck::simulator::oracle_parser::{parse_oracle_card, parse_oracle_text};

/// Write the full split-search expectation independently of the parser.
fn split_search_plan() -> crate::deck::simulator::oracle_ast::search::OracleMultiSearch {
    use crate::deck::simulator::oracle_ast::search::{OracleMultiSearch, OracleSearchPlacement};
    OracleMultiSearch {
        card_type: SearchCardType::BasicLand,
        land_types: [false; 5],
        min_cards: 0,
        max_cards: 2,
        reveal: true,
        destinations: vec![
            OracleSearchPlacement {
                destination: SearchDestination::BattlefieldTapped,
                max_cards: 1,
            },
            OracleSearchPlacement {
                destination: SearchDestination::Hand,
                max_cards: 1,
            },
        ],
        shuffle: true,
    }
}

/// A complete hand-built multi-search plan retains both destinations and its later reward.
#[test]
fn typed_multi_search_lowers_as_one_complete_runtime_plan() {
    let effect = OracleEffect::MultiSearch(split_search_plan());
    let SimEffect::MultiSearch(actual) = effect.to_runtime().expect("the complete search lowers")
    else {
        panic!("the complete plan remains a multiple-result search");
    };
    assert_runtime_split_search(&actual, &split_search_plan());
    assert!(crate::deck::simulator::oracle_ast::is_non_mana_effect(
        &effect
    ));
    let lowered = crate::deck::simulator::oracle_lower::lower_effect_sequence(&[
        effect,
        OracleEffect::GainLife(5),
    ])
    .expect("the complete search and its linked reward lower together");
    let [SimEffect::MultiSearch(actual), SimEffect::GainLife(5)] = lowered.as_slice() else {
        panic!("both destinations must remain before the linked reward");
    };
    assert_runtime_split_search(actual, &split_search_plan());
}

/// Check every lowered search field, including each ordered destination capacity.
fn assert_runtime_split_search(
    actual: &MultiSearchSpec,
    expected: &crate::deck::simulator::oracle_ast::search::OracleMultiSearch,
) {
    assert_eq!(actual.card_type, expected.card_type);
    assert_eq!(actual.land_types, expected.land_types);
    assert_eq!(actual.min_cards, expected.min_cards);
    assert_eq!(actual.max_cards, expected.max_cards);
    assert_eq!(actual.reveal, expected.reveal);
    assert_eq!(actual.shuffle, expected.shuffle);
    assert_eq!(actual.destinations.len(), expected.destinations.len());
    for (actual, expected) in actual.destinations.iter().zip(&expected.destinations) {
        assert_eq!(actual.destination, expected.destination);
        assert_eq!(actual.max_cards, expected.max_cards);
    }
}

/// Both pinned split searches retain cardinality, selector, reveal, destinations, and shuffle.
#[test]
fn pinned_cultivate_and_reach_lower_the_whole_typed_search() {
    for name in ["Cultivate", "Kodama's Reach"] {
        let original = named_row(name);
        for renamed in [false, true] {
            let mut source = original.clone();
            if renamed {
                source.name = "Renamed split search".into();
            }
            assert_pinned_split_search_source(&source);
        }
        let mut linked = original.clone();
        linked.oracle_text.push_str(" You gain 2 life.");
        let lowered = parse_sim_card(&linked);
        let [SimEffect::MultiSearch(actual), SimEffect::GainLife(2)] =
            lowered.spell_data.effects.as_slice()
        else {
            panic!("the complete search must retain its linked life reward");
        };
        assert_runtime_split_search(actual, &split_search_plan());
        assert_eq!(lowered.spell_data.life_gain_on_cast, 2);
        assert_rejected_split_search_variants(&original);
        assert_supported_split_search_variants(&original);
        let mut single = original;
        single.oracle_text = "Search your library for a basic land card, put that card onto the battlefield tapped, then shuffle.".into();
        assert!(
            matches!(parse_sim_card(&single).spell_data.effects.as_slice(), [SimEffect::Search(spec)] if spec.basic_land_only && spec.destination == SearchDestination::BattlefieldTapped)
        );
    }
}

/// Historical wording retains its complete runtime plan without making a stronger support claim.
fn assert_pinned_split_search_source(source: &crate::db::CardRow) {
    let oracle = parse_oracle_card(source);
    let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
        panic!("one complete split-search spell");
    };
    assert!(
        matches!(spell.effects.as_slice(), [OracleEffect::MultiSearch(actual)] if *actual == split_search_plan())
    );
    let lowered = parse_sim_card(source);
    let [SimEffect::MultiSearch(actual)] = lowered.spell_data.effects.as_slice() else {
        panic!("the printed search lowers without losing either destination");
    };
    assert_runtime_split_search(actual, &split_search_plan());
    assert_eq!(lowered.spell_data.extra_land_drops_on_cast, 0);
    assert_eq!(lowered.support.len(), 1);
    assert!(matches!(
        &lowered.support[0].status,
        crate::deck::simulator::support::AbilitySupportStatus::CompilerOnly { reason }
        if reason == "Ordered effects lowered, but their ordinary runtime resolution path is not verified."
    ));
}

/// Required restrictions and unmatched placement counts cannot leave any spell effect executable.
fn assert_rejected_split_search_variants(original: &crate::db::CardRow) {
    for (before, after) in [
        ("up to two", "up to three"),
        ("basic land", "snow basic land"),
        ("basic land cards", "basic land cards with different names"),
    ] {
        let mut changed = original.clone();
        changed.oracle_text = changed.oracle_text.replace(before, after);
        assert_ne!(changed.oracle_text, original.oracle_text);
        let oracle = parse_oracle_card(&changed);
        assert!(oracle.abilities.iter().all(|ability| !matches!(ability, OracleAbility::Spell(spell) if spell.effects.iter().any(|effect| matches!(effect, OracleEffect::MultiSearch(_))))));
        assert!(
            parse_sim_card(&changed).spell_data.effects.is_empty(),
            "{}: {after}",
            original.name
        );
    }
}

/// Supported wording changes keep their distinct reveal, destination, and shuffle rules.
fn assert_supported_split_search_variants(original: &crate::db::CardRow) {
    for (before, after) in [
        ("reveal those cards, ", ""),
        (
            "and the other into your hand",
            "and the other onto the battlefield",
        ),
        (", then shuffle", ""),
    ] {
        let mut expected = split_search_plan();
        match before {
            "reveal those cards, " => expected.reveal = false,
            "and the other into your hand" => {
                expected.destinations[1].destination = SearchDestination::Battlefield;
            }
            _ => expected.shuffle = false,
        }
        let mut changed = original.clone();
        changed.oracle_text = changed.oracle_text.replace(before, after);
        assert_ne!(changed.oracle_text, original.oracle_text);
        let oracle = parse_oracle_card(&changed);
        let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
            panic!("the changed search remains one complete spell");
        };
        assert!(
            matches!(spell.effects.as_slice(), [OracleEffect::MultiSearch(actual)] if *actual == expected)
        );
        let lowered = parse_sim_card(&changed);
        let [SimEffect::MultiSearch(actual)] = lowered.spell_data.effects.as_slice() else {
            panic!("the changed complete plan lowers");
        };
        assert_runtime_split_search(actual, &expected);
    }
}

/// Hand-built unequal counts cannot be exchanged, equalized, or dropped during lowering.
#[test]
fn typed_draw_discard_lowers_each_fixed_count() {
    for (draw, discard) in [(3, 1), (1, 3), (4, 2)] {
        let effect = OracleEffect::DrawDiscard { draw, discard };
        assert!(
            matches!(effect.to_runtime(), Some(SimEffect::DrawDiscard { draw: actual_draw, discard: actual_discard }) if actual_draw == draw && actual_discard == discard)
        );
    }
}

/// Pinned Sift carries three draws and one discard through both compiler boundaries.
#[test]
fn pinned_sift_keeps_unequal_counts_and_rejects_unknown_riders() {
    let original = named_row("Sift");
    for renamed in [false, true] {
        let mut source = original.clone();
        if renamed {
            source.name = "Renamed draw source".into();
        }
        let oracle = parse_oracle_card(&source);
        assert!(oracle.abilities.iter().any(|ability| matches!(ability, OracleAbility::Spell(spell) if matches!(spell.effects.as_slice(), [OracleEffect::DrawDiscard { draw: 3, discard: 1 }]))));
        assert!(matches!(
            parse_sim_card(&source).spell_data.effects.as_slice(),
            [SimEffect::DrawDiscard {
                draw: 3,
                discard: 1
            }]
        ));
    }
    for text in [
        "Draw three cards, then discard a creature card.",
        "Draw three cards, then discard a card at random.",
        "Draw three cards, then discard a card unless you pay 2 life.",
    ] {
        let mut source = original.clone();
        source.oracle_text = text.into();
        assert!(
            parse_sim_card(&source).spell_data.effects.is_empty(),
            "{text}"
        );
    }
    let mut changed = original;
    changed.oracle_text = "Draw a card, then discard three cards.".into();
    assert!(matches!(
        parse_sim_card(&changed).spell_data.effects.as_slice(),
        [SimEffect::DrawDiscard {
            draw: 1,
            discard: 3
        }]
    ));
}

/// Gravebreaker Lamia's search retains its graveyard destination and mandatory shuffle.
#[test]
fn pinned_graveyard_search_keeps_destination_and_unknown_destination_is_inert() {
    let original = named_row("Gravebreaker Lamia");
    for renamed in [false, true] {
        let mut source = original.clone();
        if renamed {
            source.name = "Renamed graveyard search".into();
        }
        let oracle = parse_oracle_card(&source);
        let entry = oracle
            .abilities
            .iter()
            .find_map(|ability| match ability {
                OracleAbility::Triggered(trigger) => Some(trigger),
                _ => None,
            })
            .expect("entry trigger parses");
        assert!(
            matches!(entry.effects.as_slice(), [OracleEffect::Search(spec)] if spec.destination == SearchDestination::Graveyard && spec.shuffle && spec.card_type.is_none())
        );
        assert!(
            matches!(entry.to_runtime_all()[0].effect_sequence(), [SimEffect::Search(spec)] if spec.destination == SearchDestination::Graveyard)
        );
    }
    let parsed = parse_oracle_text(
        "Search your library for a card, put it into an opponent's graveyard, then shuffle.",
        &[],
    );
    assert!(parsed.abilities.iter().all(|ability| matches!(ability, OracleAbility::Spell(spell) if spell.effects.iter().any(|effect| matches!(effect, OracleEffect::Unsupported(_))))));
    let mut changed = original;
    changed.oracle_text = "When this creature enters, search your library for a card, put it into your hand, then shuffle.".into();
    let lowered = parse_sim_card(&changed);
    assert!(lowered.unlocked_abilities(0).any(|ability| matches!(ability.effect_sequence(), [SimEffect::Search(spec)] if spec.destination == SearchDestination::Hand)));
}

/// Pinned top tutors retain exact filters, shuffle, destination, and the whole drawback.
#[test]
fn pinned_top_tutors_keep_complete_search_specs_and_vampiric_life_loss() {
    for (name, card_type, loses_life) in [
        (
            "Enlightened Tutor",
            Some(SearchCardType::ArtifactOrEnchantment),
            false,
        ),
        (
            "Mystical Tutor",
            Some(SearchCardType::InstantSorcery),
            false,
        ),
        ("Vampiric Tutor", None, true),
    ] {
        let source = named_row(name);
        let expected = SearchSpec {
            card_type,
            destination: SearchDestination::LibraryTop,
            shuffle: true,
            ..SearchSpec::default()
        };
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
            panic!("expected a single complete spell: {name}");
        };
        let Some(OracleEffect::Search(actual)) = spell.effects.first() else {
            panic!("expected parsed search");
        };
        assert_search_spec(actual, &expected);
        let lowered = parse_sim_card(&source);
        let Some(SimEffect::Search(actual)) = lowered.spell_data.effects.first() else {
            panic!("expected lowered search");
        };
        assert_search_spec(actual, &expected);
        assert_eq!(spell.effects.len(), 1 + usize::from(loses_life));
        assert_eq!(
            lowered.spell_data.effects.len(),
            1 + usize::from(loses_life)
        );
        if loses_life {
            assert!(matches!(
                spell.effects.last(),
                Some(OracleEffect::LoseLife {
                    amount: 2,
                    scope: crate::deck::simulator::model::LifeLossScope::SelfOnly
                })
            ));
            assert!(matches!(
                lowered.spell_data.effects.last(),
                Some(SimEffect::LoseLife {
                    amount: 2,
                    scope: crate::deck::simulator::model::LifeLossScope::SelfOnly
                })
            ));
        }
    }
}

/// Check every typed search field against an independently written expected rule.
fn assert_search_spec(actual: &SearchSpec, expected: &SearchSpec) {
    assert_eq!(actual.card_type, expected.card_type);
    assert_eq!(actual.color, expected.color);
    assert_eq!(actual.colorless, expected.colorless);
    assert_eq!(actual.mana_value, expected.mana_value);
    assert_eq!(actual.max_mana_value, expected.max_mana_value);
    assert_eq!(actual.min_mana_value, expected.min_mana_value);
    assert_eq!(actual.destination, expected.destination);
    assert_eq!(actual.optional, expected.optional);
    assert_eq!(actual.top_count, expected.top_count);
    assert_eq!(actual.shuffle, expected.shuffle);
    assert_eq!(actual.non_human, expected.non_human);
    assert_eq!(actual.land_types, expected.land_types);
    assert_eq!(actual.basic_land_only, expected.basic_land_only);
}

/// Relic of Sauron's fixed unequal counts cannot drop its mana or tap payment.
#[test]
fn pinned_relic_of_sauron_keeps_draw_discard_and_complete_activation_cost() {
    let mut source = named_row("Relic of Sauron");
    let oracle = parse_oracle_card(&source);
    let activation = oracle
        .abilities
        .iter()
        .find_map(|ability| match ability {
            OracleAbility::Activated(ability)
                if matches!(
                    ability.effects.as_slice(),
                    [OracleEffect::DrawDiscard {
                        draw: 2,
                        discard: 1
                    }]
                ) =>
            {
                Some(ability)
            }
            _ => None,
        })
        .expect("printed draw activation parses");
    assert!(
        matches!(activation.costs.as_slice(), [ActivationCost::Mana(cost), ActivationCost::Tap(ActivationTarget::Source)] if cost.generic == 3 && cost.total() == 3)
    );
    let lowered = activation.to_runtime_all();
    assert_eq!(lowered.len(), 1);
    let costs = &lowered[0]
        .activation
        .as_ref()
        .expect("activation payment lowers")
        .costs;
    assert_eq!(
        costs,
        &[
            SimActivationCost::Mana(Cost {
                generic: 3,
                ..Default::default()
            }),
            SimActivationCost::TapSource
        ]
    );
    assert!(matches!(
        lowered[0].effect_sequence(),
        [SimEffect::DrawDiscard {
            draw: 2,
            discard: 1
        }]
    ));
    source.oracle_text = "{3}, {T}: Draw two cards, then discard a creature card.".into();
    assert!(
        parse_sim_card(&source)
            .unlocked_abilities(0)
            .next()
            .is_none()
    );
}

/// Build a complete sacrifice activation without invoking the source parser.
fn sacrifice_activation(cost: ActivationCost, mana: bool) -> OracleActivatedAbility {
    OracleActivatedAbility {
        costs: vec![
            ActivationCost::Mana(Cost {
                generic: 1,
                ..Cost::default()
            }),
            ActivationCost::Tap(ActivationTarget::Source),
            ActivationCost::PayLife(1),
            cost,
        ],
        effects: vec![if mana {
            OracleEffect::Mana(ManaYield {
                colorless: 2,
                ..ManaYield::default()
            })
        } else {
            OracleEffect::Draw(1)
        }],
        restrictions: vec![],
        is_mana_ability: mana,
        mana_per_spell_cast: false,
    }
}

/// Lower a hand-built ability through the card-level mana and activation gates.
fn lower_sacrifice_card(ability: OracleActivatedAbility) -> crate::deck::simulator::model::SimCard {
    let source = crate::deck::simulator::deck_test_support::card(
        "Sacrifice compiler control",
        "{1}",
        "Creature",
        "",
    );
    let oracle = OracleCard {
        keywords: vec![],
        abilities: vec![OracleAbility::Activated(ability)],
        ability_sources: vec![],
        spell_data: Default::default(),
        static_data: Default::default(),
        land: Default::default(),
        stations: Default::default(),
        library_graveyard_trigger: None,
    };
    super::card::lower_oracle_card(&source, &oracle)
}

/// Both creature selectors retain the source flag and every other payment.
#[test]
fn typed_creature_sacrifice_retains_explicit_source_exclusion() {
    for exclude_source in [false, true] {
        let ability = sacrifice_activation(
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Creature,
                exclude_source,
            },
            false,
        );
        let lowered = ability.to_runtime_all();
        let [lowered] = lowered.as_slice() else {
            panic!("complete typed creature sacrifice lowers");
        };
        let activation = lowered.activation.as_ref().expect("costs stay attached");
        assert_eq!(
            activation.costs,
            [
                SimActivationCost::Mana(Cost {
                    generic: 1,
                    ..Cost::default()
                }),
                SimActivationCost::TapSource,
                SimActivationCost::PayLife(1),
                SimActivationCost::SacrificeCreatures {
                    count: 1,
                    exclude_source,
                },
            ]
        );
        assert_eq!(activation.creature_sacrifices(), 1);
        assert_eq!(
            activation.creature_sacrifice_excludes_source(),
            exclude_source
        );
        assert!(matches!(lowered.effect_sequence(), [SimEffect::Draw(1)]));
    }
}

/// A source sacrifice requires exactly one source without an exclusion flag.
#[test]
fn typed_source_sacrifice_rejects_exclusion_and_invalid_counts() {
    for (object, count, exclude_source, supported) in [
        (CostObject::Source, 1, false, true),
        (CostObject::Source, 1, true, false),
        (CostObject::Source, 0, false, false),
        (CostObject::Source, 2, false, false),
        (CostObject::Creature, 0, false, false),
        (CostObject::Creature, 2, false, false),
        (CostObject::Any, 1, false, false),
        (CostObject::Any, 1, true, false),
    ] {
        let ability = sacrifice_activation(
            ActivationCost::Sacrifice {
                count,
                object,
                exclude_source,
            },
            false,
        );
        let lowered = ability.to_runtime_all();
        assert_eq!(lowered.len(), usize::from(supported), "{ability:?}");
        if supported {
            let costs = lowered[0].activation.as_ref().expect("source payment");
            assert_eq!(costs.costs.len(), ability.costs.len());
            assert!(costs.sacrifices_source());
            assert_eq!(costs.creature_sacrifices(), 0);
            assert!(!costs.creature_sacrifice_excludes_source());
        }
    }
}

/// Card tap data cannot bypass the typed source-sacrifice requirements.
#[test]
fn typed_sacrifice_card_mana_gate_matches_activation_support() {
    for (object, count, exclude_source, supported) in [
        (CostObject::Creature, 1, false, true),
        (CostObject::Creature, 1, true, true),
        (CostObject::Source, 1, false, true),
        (CostObject::Source, 1, true, false),
        (CostObject::Source, 2, false, false),
        (CostObject::Creature, 0, false, false),
        (CostObject::Creature, 2, false, false),
        (CostObject::Any, 1, false, false),
    ] {
        let ability = sacrifice_activation(
            ActivationCost::Sacrifice {
                count,
                object,
                exclude_source,
            },
            true,
        );
        let card = lower_sacrifice_card(ability);
        assert_eq!(
            card.tap.is_some(),
            supported,
            "{object:?}/{count}/{exclude_source}"
        );
        assert_eq!(card.unlocked_abilities(0).count(), usize::from(supported));
        if let Some(yield_) = card.tap {
            assert_eq!(yield_.colorless, 2);
        }
    }
}

/// Unknown required qualifiers block the entire draw or mana reward.
#[test]
fn typed_sacrifice_unknown_qualifiers_cannot_leave_free_rewards() {
    for mana in [false, true] {
        for exclude_source in [false, true] {
            let mut ability = sacrifice_activation(
                ActivationCost::Sacrifice {
                    count: 1,
                    object: CostObject::Creature,
                    exclude_source,
                },
                mana,
            );
            ability
                .costs
                .push(ActivationCost::Other("with flying".into()));
            assert!(ability.to_runtime_all().is_empty());
            let card = lower_sacrifice_card(ability);
            assert!(card.tap.is_none());
            assert!(card.unlocked_abilities(0).next().is_none());
        }
    }
}

/// Pinned sacrifice wording justifies each inventory flag without granting unsupported rewards.
#[test]
fn pinned_sacrifice_costs_keep_selectors_and_source_exclusion() {
    for (name, object, count, supported) in [
        ("Cauldron of Essence", CostObject::Creature, 1, true),
        ("Commander's Sphere", CostObject::Source, 1, true),
        ("Elven Passage", CostObject::Source, 1, true),
        ("Fabled Passage", CostObject::Source, 1, false),
        ("Krark-Clan Ironworks", CostObject::Any, 1, false),
        ("Lotus Petal", CostObject::Source, 1, true),
        ("Mind Stone", CostObject::Source, 1, true),
        ("Nurturing Peatland", CostObject::Source, 1, true),
        ("Sai, Master Thopterist", CostObject::Any, 2, false),
        ("Sakura-Tribe Elder", CostObject::Source, 1, true),
        ("Viscera Seer", CostObject::Creature, 1, true),
    ] {
        let source = named_row(name);
        let oracle = parse_oracle_card(&source);
        let ability = oracle
            .abilities
            .iter()
            .find_map(|ability| match ability {
                OracleAbility::Activated(ability)
                    if ability
                        .costs
                        .iter()
                        .any(|cost| matches!(cost, ActivationCost::Sacrifice { .. })) =>
                {
                    Some(ability)
                }
                _ => None,
            })
            .expect("pinned source has one sacrifice activation");
        let sacrifices: Vec<_> = ability
            .costs
            .iter()
            .filter_map(|cost| match cost {
                ActivationCost::Sacrifice {
                    count,
                    object,
                    exclude_source,
                } => Some((*object, *count, *exclude_source)),
                _ => None,
            })
            .collect();
        assert_eq!(sacrifices, [(object, count, false)], "{name}");
        assert!(
            !ability
                .costs
                .iter()
                .any(|cost| matches!(cost, ActivationCost::Other(_))),
            "{name}"
        );
        assert_eq!(!ability.to_runtime_all().is_empty(), supported, "{name}");
    }
}

/// Elven Passage's supported activation retains its search, Elf choice, and every source cost.
#[test]
fn pinned_elven_passage_keeps_the_complete_result_linked_untap() {
    let original = named_row("Elven Passage");
    let expected_search = SearchSpec {
        card_type: Some(SearchCardType::BasicLand),
        basic_land_only: true,
        destination: SearchDestination::BattlefieldTapped,
        shuffle: true,
        ..Default::default()
    };
    for renamed in [false, true] {
        let mut source = original.clone();
        if renamed {
            source.name = "Renamed result-linked fetch".into();
        }
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
            panic!("the historical fetch keeps one complete activation");
        };
        let [OracleEffect::SearchThenBeholdUntap { search, spec }] = ability.effects.as_slice()
        else {
            panic!("the optional untap cannot detach from its search result");
        };
        assert_search_spec(search, &expected_search);
        assert!(spec.card_types.is_empty());
        assert_eq!(spec.subtypes, ["elf"]);
        assert_eq!(spec.count, 1);
        let runtime = ability.to_runtime_all();
        let [runtime] = runtime.as_slice() else {
            panic!("the complete activation lowers once");
        };
        assert_eq!(
            runtime
                .activation
                .as_ref()
                .expect("all source costs remain attached")
                .costs,
            [
                SimActivationCost::TapSource,
                SimActivationCost::SacrificeSource,
                SimActivationCost::PayLife(1)
            ]
        );
        let [SimEffect::SearchThenBeholdUntap { search, spec }] = runtime.effect_sequence() else {
            panic!("runtime must preserve the result-linked untap");
        };
        assert_search_spec(search, &expected_search);
        assert!(spec.card_types.is_empty());
        assert_eq!(spec.subtypes, ["elf"]);
        assert_eq!(spec.count, 1);
        let card = parse_sim_card(&source);
        assert!(card.is_fetch_land && card.fetch_basic_only && card.fetch_enters_tapped);
        assert_eq!(card.fetch_life_cost, 1);
        assert_eq!(card.fetch_behold_untap.as_ref(), Some(spec));
    }
}

/// New fetch support cannot discard a required choice count or an extra conditional effect.
#[test]
fn pinned_elven_passage_rejects_unmodeled_untap_requirements() {
    let original = named_row("Elven Passage");
    for (before, after) in [
        ("behold an Elf", "behold three Elves"),
        ("untap that land", "untap that land and draw a card"),
    ] {
        let mut source = original.clone();
        source.oracle_text = source.oracle_text.replace(before, after);
        assert_ne!(source.oracle_text, original.oracle_text);
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
            panic!("the changed fetch keeps its complete activation source");
        };
        assert!(ability.to_runtime_all().is_empty());
        let card = parse_sim_card(&source);
        assert!(!card.is_fetch_land);
        assert!(card.fetch_behold_untap.is_none());
        assert!(card.unlocked_abilities(0).next().is_none());
    }
}

/// Required qualifiers cannot become invented generic sacrifice selectors.
#[test]
fn pinned_unknown_sacrifice_qualifiers_keep_the_whole_activation_inert() {
    for (name, required) in [
        ("Bolas's Citadel", "Sacrifice ten nonland permanents"),
        ("Chatterfang, Squirrel General", "Sacrifice X Squirrels"),
        ("Professional Face-Breaker", "Sacrifice a Treasure"),
    ] {
        let oracle = parse_oracle_card(&named_row(name));
        let ability = oracle
            .abilities
            .iter()
            .find_map(|ability| match ability {
                OracleAbility::Activated(ability) => Some(ability),
                _ => None,
            })
            .expect("pinned source has one sacrifice activation");
        assert!(
            ability
                .costs
                .iter()
                .any(|cost| matches!(cost, ActivationCost::Other(text) if text == required)),
            "{name}"
        );
        assert!(
            !ability
                .costs
                .iter()
                .any(|cost| matches!(cost, ActivationCost::Sacrifice { .. })),
            "{name}"
        );
        assert!(ability.to_runtime_all().is_empty(), "{name}");
    }
}

/// Changing the printed selector to another creature changes the lowered source flag.
#[test]
fn changed_pinned_sacrifice_selector_excludes_the_source() {
    let mut changed = named_row("Viscera Seer");
    changed.oracle_text = "Sacrifice another creature: Scry 1.".into();
    let oracle = parse_oracle_card(&changed);
    let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
        panic!("changed sacrifice source parses");
    };
    assert!(matches!(
        ability.costs.as_slice(),
        [ActivationCost::Sacrifice {
            count: 1,
            object: CostObject::Creature,
            exclude_source: true
        }]
    ));
    let lowered = parse_sim_card(&changed);
    let ability = lowered
        .unlocked_abilities(0)
        .next()
        .expect("another-creature payment lowers");
    let costs = ability.activation.as_ref().expect("payment stays attached");
    assert_eq!(
        costs.costs,
        [SimActivationCost::SacrificeCreatures {
            count: 1,
            exclude_source: true
        }]
    );
    assert!(matches!(ability.effect_sequence(), [SimEffect::Scry(1)]));
}
