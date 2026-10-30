use super::*;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::oracle_ast::{OracleAbility, OracleStaticEffect};
use crate::deck::simulator::oracle_parser::parse_oracle_card;

/// Captured reductions retain their whole selector after a source is renamed.
#[test]
fn captured_reducers_parse_to_typed_filters() {
    let instant_sorcery =
        OracleSpellFilter::Types(vec![OracleSpellType::Instant, OracleSpellType::Sorcery]);
    let artifact = OracleSpellFilter::Types(vec![OracleSpellType::Artifact]);
    for (name, filter) in [
        ("Goblin Electromancer", instant_sorcery),
        ("Foundry Inspector", artifact.clone()),
        ("Etherium Sculptor", artifact),
        (
            "Goblin Anarchomancer",
            OracleSpellFilter::Colors(vec![ManaColor::Red, ManaColor::Green]),
        ),
        (
            "Pearl Medallion",
            OracleSpellFilter::Colors(vec![ManaColor::White]),
        ),
        (
            "Sapphire Medallion",
            OracleSpellFilter::Colors(vec![ManaColor::Blue]),
        ),
        (
            "Jet Medallion",
            OracleSpellFilter::Colors(vec![ManaColor::Black]),
        ),
        (
            "Ruby Medallion",
            OracleSpellFilter::Colors(vec![ManaColor::Red]),
        ),
    ] {
        for renamed in [false, true] {
            let mut row = named_row(name);
            if renamed {
                row.name = "Renamed cost source".into();
            }
            let oracle = parse_oracle_card(&row);
            let reductions: Vec<_> = oracle
                .abilities
                .iter()
                .filter_map(|ability| match ability {
                    OracleAbility::Static(ability) => Some(&ability.effects),
                    _ => None,
                })
                .flatten()
                .filter_map(|effect| match effect {
                    OracleStaticEffect::SpellCostReduction(reduction) => Some(reduction),
                    _ => None,
                })
                .collect();
            assert_eq!(
                reductions,
                [&OracleSpellCostReduction {
                    amount: 1,
                    filter: filter.clone(),
                }],
                "{name}, renamed={renamed}"
            );
        }
    }
}

/// Every supported single type and color is parsed as a complete selector.
#[test]
fn flat_reduction_grammar_covers_types_colors_and_any_spell() {
    for (word, kind) in [
        ("artifact", OracleSpellType::Artifact),
        ("battle", OracleSpellType::Battle),
        ("creature", OracleSpellType::Creature),
        ("enchantment", OracleSpellType::Enchantment),
        ("instant", OracleSpellType::Instant),
        ("planeswalker", OracleSpellType::Planeswalker),
        ("sorcery", OracleSpellType::Sorcery),
    ] {
        assert_eq!(
            parse_cost_reduction(&format!("{word} spells you cast cost {{2}} less to cast.")),
            Some(OracleSpellCostReduction {
                amount: 2,
                filter: OracleSpellFilter::Types(vec![kind]),
            }),
        );
    }
    for color in ManaColor::ALL {
        let source = format!("{} spells you cast cost {{1}} less to cast.", color.name());
        assert_eq!(
            parse_cost_reduction(&source),
            Some(OracleSpellCostReduction {
                amount: 1,
                filter: OracleSpellFilter::Colors(vec![color]),
            })
        );
    }
    for source in [
        "Spells you cast cost {12} less to cast.",
        "All spells cost {12} less to cast.",
    ] {
        assert_eq!(
            parse_cost_reduction(source),
            Some(OracleSpellCostReduction {
                amount: 12,
                filter: OracleSpellFilter::Any,
            })
        );
    }
}

/// Type unions allow Oracle's conjunction; color unions require the word "or".
#[test]
fn complete_unions_do_not_parse_mixed_selectors() {
    for connector in ["and", "or"] {
        let source =
            format!("Artifact {connector} enchantment spells you cast cost {{1}} less to cast.");
        assert_eq!(
            parse_cost_reduction(&source),
            Some(OracleSpellCostReduction {
                amount: 1,
                filter: OracleSpellFilter::Types(vec![
                    OracleSpellType::Artifact,
                    OracleSpellType::Enchantment
                ]),
            })
        );
    }
    assert_eq!(
        parse_cost_reduction(" Red or green spells you cast cost {1} less to cast. "),
        Some(OracleSpellCostReduction {
            amount: 1,
            filter: OracleSpellFilter::Colors(vec![ManaColor::Red, ManaColor::Green]),
        })
    );
    assert!(parse_cost_reduction("Red and green spells you cast cost {1} less to cast.").is_none());
}

/// The captured relative clause keeps its complete color selector without a name check.
#[test]
fn captured_color_relative_clause_parses_without_accepting_extra_restrictions() {
    let source = named_row("Goblin Anarchomancer");
    assert_eq!(
        source.oracle_text,
        "Each spell you cast that's red or green costs {1} less to cast."
    );
    assert_eq!(
        parse_cost_reduction(&source.oracle_text),
        Some(OracleSpellCostReduction {
            amount: 1,
            filter: OracleSpellFilter::Colors(vec![ManaColor::Red, ManaColor::Green]),
        })
    );
    for (before, after) in [
        ("red or green", "red and green"),
        ("red or green", "red or creature"),
        ("red or green", "red or green creature"),
        ("red or green", "red or green with mana value 3 or greater"),
        ("red or green", "of the chosen color"),
        ("red or green", "colorless"),
        ("{1}", "{1}{R}"),
        ("{1}", "{X}"),
        (
            "less to cast.",
            "less to cast for each artifact you control.",
        ),
        ("less to cast.", "less to cast if you control an artifact."),
        ("less to cast.", "less to cast. Gain 1 life."),
    ] {
        let changed = source.oracle_text.replace(before, after);
        assert_ne!(changed, source.oracle_text);
        assert!(parse_cost_reduction(&changed).is_none(), "{changed}");
    }
}

/// Relative color clauses accept one color or a complete union joined with "or".
#[test]
fn relative_color_clauses_use_the_same_typed_color_union() {
    for color in ManaColor::ALL {
        let source = format!(
            "Each spell you cast that's {} costs {{2}} less to cast.",
            color.name()
        );
        assert_eq!(
            parse_cost_reduction(&source),
            Some(OracleSpellCostReduction {
                amount: 2,
                filter: OracleSpellFilter::Colors(vec![color]),
            })
        );
    }
    for source in [
        "Each spell you cast that's red or costs {1} less to cast.",
        "Each spell you cast that's red green costs {1} less to cast.",
        "Each spell you cast that's red or green cost {1} less to cast.",
        "Each spell you cast that's an artifact costs {1} less to cast.",
        "Each spell your opponents cast that's red or green costs {1} less to cast.",
    ] {
        assert!(parse_cost_reduction(source).is_none(), "{source}");
    }
}

/// Conditions, extra symbols, and incomplete text cannot become partial reductions.
#[test]
fn unsupported_reductions_are_rejected_as_whole_abilities() {
    for source in [
        "Creature spells of the chosen type you cast cost {1} less to cast.",
        "Spells you cast of the chosen type cost {1} less to cast.",
        "Goblin spells you cast cost {1} less to cast.",
        "Red creature spells you cast cost {1} less to cast.",
        "Noncreature spells you cast cost {1} less to cast.",
        "Legendary spells you cast cost {1} less to cast.",
        "Instant and spells you cast cost {1} less to cast.",
        "Instant sorcery spells you cast cost {1} less to cast.",
        "Instant or green spells you cast cost {1} less to cast.",
        "Red or creature spells you cast cost {1} less to cast.",
        "Artifactspells you cast cost {1} less to cast.",
        "Spells your opponents cast cost {1} less to cast.",
        "Spells you cast cost {U} less to cast.",
        "Spells you cast cost {1}{U} less to cast.",
        "Spells you cast cost {U}{1} less to cast.",
        "Spells you cast cost {1}{1} less to cast.",
        "Spells you cast cost {W/U} less to cast.",
        "Spells you cast cost {X} less to cast.",
        "Spells you cast cost {} less to cast.",
        "Spells you cast cost {-1} less to cast.",
        "Spells you cast cost {4294967296} less to cast.",
        "Spells you cast cost one less to cast.",
        "Spells you cast cost {1} less to cast for each artifact you control.",
        "Spells you cast cost {1} less to cast if you control a creature.",
        "Spells you cast cost {1} less to cast during your turn.",
        "Spells you cast cost {1} less to cast until end of turn.",
        "Spells you cast cost {1} less to cast. You gain 1 life.",
        "When this creature enters, spells you cast cost {1} less to cast.",
        "{T}: Spells you cast cost {1} less to cast.",
        "Spells you cast cost {1} less to cast..",
    ] {
        assert!(parse_cost_reduction(source).is_none(), "{source}");
    }
}
