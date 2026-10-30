use super::*;
use crate::deck::simulator::model::ManaColor;
use crate::deck::simulator::oracle_ast::{
    OracleAbilitySource, OracleSourceSpan, OracleStaticAbility,
};
use crate::deck::simulator::oracle_lower::parse_sim_card;
use crate::deck::simulator::oracle_parser::parse_oracle_card;
use crate::deck::simulator::scenario_test_support::row;

/// The compiler maps every typed card selector without source text.
#[test]
fn typed_cost_reductions_lower_without_text_or_names() {
    for (oracle, runtime) in [
        (OracleSpellType::Artifact, SpellCostCardType::Artifact),
        (OracleSpellType::Battle, SpellCostCardType::Battle),
        (OracleSpellType::Creature, SpellCostCardType::Creature),
        (OracleSpellType::Enchantment, SpellCostCardType::Enchantment),
        (OracleSpellType::Instant, SpellCostCardType::Instant),
        (
            OracleSpellType::Planeswalker,
            SpellCostCardType::Planeswalker,
        ),
        (OracleSpellType::Sorcery, SpellCostCardType::Sorcery),
    ] {
        let reduction = OracleSpellCostReduction {
            amount: 3,
            filter: OracleSpellFilter::Types(vec![oracle]),
        };
        assert_eq!(
            lower_spell_cost_reduction(&reduction),
            SpellCostReduction {
                amount: 3,
                filter: SpellCostFilter::Types(vec![runtime]),
            }
        );
    }
    for (oracle, runtime) in [
        (OracleSpellFilter::Any, SpellCostFilter::Any),
        (
            OracleSpellFilter::Colors(vec![ManaColor::Red, ManaColor::Green]),
            SpellCostFilter::Colors(vec![ManaColor::Red, ManaColor::Green]),
        ),
    ] {
        assert_eq!(
            lower_spell_cost_reduction(&OracleSpellCostReduction {
                amount: 4,
                filter: oracle,
            }),
            SpellCostReduction {
                amount: 4,
                filter: runtime
            }
        );
    }
}

/// Ability spans prevent the hidden MDFC face from contributing static discounts.
#[test]
fn only_front_face_cost_reductions_are_lowered() {
    let source = row(
        "Split source",
        "{2} // ",
        "Artifact // Land",
        "front\n//\nback",
    );
    let front = OracleSpellCostReduction {
        amount: 1,
        filter: OracleSpellFilter::Any,
    };
    let back = OracleSpellCostReduction {
        amount: 7,
        filter: OracleSpellFilter::Any,
    };
    let mut oracle = OracleCard {
        keywords: Vec::new(),
        abilities: vec![
            OracleAbility::Static(OracleStaticAbility {
                effects: vec![OracleStaticEffect::SpellCostReduction(front.clone())],
            }),
            OracleAbility::Static(OracleStaticAbility {
                effects: vec![OracleStaticEffect::SpellCostReduction(back)],
            }),
        ],
        ability_sources: vec![
            OracleAbilitySource {
                text: "front".into(),
                span: Some(OracleSourceSpan { start: 0, end: 5 }),
            },
            OracleAbilitySource {
                text: "back".into(),
                span: Some(OracleSourceSpan { start: 9, end: 13 }),
            },
        ],
        spell_data: Default::default(),
        static_data: Default::default(),
        land: Default::default(),
        stations: Default::default(),
        library_graveyard_trigger: None,
    };
    assert_eq!(
        lower_spell_cost_reductions(&source, &oracle),
        [lower_spell_cost_reduction(&front)]
    );
    oracle.ability_sources.clear();
    assert!(lower_spell_cost_reductions(&source, &oracle).is_empty());
}

/// Full parsing and lowering keep a back-face discount off the front card.
#[test]
fn parsed_mdfc_discounts_remain_attached_to_the_front_face() {
    let front_text = "Artifact spells you cast cost {1} less to cast.";
    let back_text = "Creature spells you cast cost {7} less to cast.";
    for (text, expected) in [
        (format!("{front_text}\n//\n{back_text}"), 1),
        (format!("{{T}}: Add {{C}}.\n//\n{back_text}"), 0),
    ] {
        let source = row("Split source", "{2} // ", "Artifact // Land", &text);
        let oracle = parse_oracle_card(&source);
        assert_eq!(
            lower_spell_cost_reductions(&source, &oracle).len(),
            expected
        );
        assert_eq!(parse_sim_card(&source).cost_reductions.len(), expected);
    }
}

/// Changing a captured selector or adding a colored symbol leaves no runtime discount.
#[test]
fn changed_source_grammar_cannot_leave_a_partial_runtime_reduction() {
    let original = crate::deck::simulator::commander_test_support::named_row("Foundry Inspector");
    assert_eq!(parse_sim_card(&original).cost_reductions.len(), 1);
    for (from, to) in [
        ("Artifact spells", "Goblin spells"),
        ("Artifact spells", "Red artifact spells"),
        ("{1}", "{1}{U}"),
        ("less to cast.", "less to cast if you control a creature."),
        (
            "less to cast.",
            "less to cast for each artifact you control.",
        ),
    ] {
        let mut source = original.clone();
        source.oracle_text = source.oracle_text.replace(from, to);
        assert_ne!(source.oracle_text, original.oracle_text);
        assert!(parse_sim_card(&source).cost_reductions.is_empty(), "{to}");
    }
}
