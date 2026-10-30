//! Static-rule and unsupported-statement syntax tests.
use super::*;
use crate::deck::simulator::oracle_ast::tribal::OracleTribalStaticEffect;
use crate::deck::simulator::oracle_ast::{OracleSpellFilter, OracleSpellType};

/// Parse static targets, mana grants, buffs, cost reductions, and limits.
#[test]
fn parses_static_targets_buffs_costs_and_unknown_rules() {
    assert!(matches!(
        static_effect("Creatures you control get +2/-1."),
        OracleStaticEffect::CreatureBuff {
            power: 2,
            toughness: -1
        }
    ));
    for (text, expected, expected_keyword) in [
        (
            "Creatures you control have flying.",
            StaticTarget::CreaturesYouControl,
            OracleKeywordName::Flying,
        ),
        (
            "Lands you control have vigilance.",
            StaticTarget::LandsYouControl,
            OracleKeywordName::Vigilance,
        ),
        (
            "Spells you cast have lifelink.",
            StaticTarget::SpellsYouCast,
            OracleKeywordName::Lifelink,
        ),
        (
            "Permanents you control have hexproof.",
            StaticTarget::PermanentsYouControl,
            OracleKeywordName::Hexproof,
        ),
    ] {
        assert!(
            matches!(static_effect(text), OracleStaticEffect::KeywordGrant { target, keyword } if target == expected && keyword.name == expected_keyword)
        );
    }
    for (text, expected) in [
        (
            "Creatures you control have \"{T}: Add one mana of any color.\"",
            "creature",
        ),
        (
            "Lands you control have \"{T}: Add one mana of any color.\"",
            "land",
        ),
    ] {
        assert!(
            matches!(static_effect(text), OracleStaticEffect::Tribal(OracleTribalStaticEffect::ManaGrant { selector, ability }) if selector.filter.card_types == [expected] && selector.controller == PlayerScope::You && matches!(ability.effects.as_slice(), [OracleEffect::Mana(yield_)] if yield_.any_pips == 1))
        );
    }
    assert!(
        matches!(static_effect("Permanents you control have \"{T}: Add {C}.\""), OracleStaticEffect::Tribal(OracleTribalStaticEffect::ManaGrant { selector, ability }) if selector.filter.card_types.is_empty() && selector.controller == PlayerScope::You && matches!(ability.effects.as_slice(), [OracleEffect::Mana(yield_)] if yield_.colorless == 1))
    );
    assert!(
        matches!(static_effect("Artifact spells you cast cost {1} less to cast."), OracleStaticEffect::SpellCostReduction(reduction) if reduction.amount == 1 && reduction.filter == OracleSpellFilter::Types(vec![OracleSpellType::Artifact]))
    );
    assert!(matches!(
        static_effect("You may play an additional land on each of your turns."),
        OracleStaticEffect::AdditionalLandDrop(1)
    ));
    assert!(matches!(
        static_effect("If there are 20 or more tower counters on this artifact, you win the game."),
        OracleStaticEffect::WinsAtCounters(20)
    ));
    let unsupported =
        static_effect("As long as you control a creature, this permanent has flying.");
    assert!(
        matches!(unsupported, OracleStaticEffect::Unsupported(ref text) if text.to_ascii_lowercase().contains("as long as")),
        "unexpected static node: {unsupported:?}"
    );
}

/// Preserve incomplete or unknown ability statements without guessing effects.
#[test]
fn preserves_unsupported_ability_shapes_without_inventing_effects() {
    let activated = parse_oracle_text("{T} Draw a card", &[]);
    let [OracleAbility::Unsupported(activated)] = activated.abilities.as_slice() else {
        panic!("expected incomplete activation to remain unsupported");
    };
    assert_eq!(activated.kind, UnsupportedAbilityKind::Activated);
    assert_eq!(activated.source, "{T} Draw a card");
    let triggered = parse_oracle_text("Whenever a permanent does something", &[]);
    let [OracleAbility::Unsupported(triggered)] = triggered.abilities.as_slice() else {
        panic!("expected incomplete trigger to remain unsupported");
    };
    assert_eq!(triggered.kind, UnsupportedAbilityKind::Triggered);
    let unknown = parse_oracle_text("Venture into the dungeon.", &[]);
    let [OracleAbility::Unsupported(unknown)] = unknown.abilities.as_slice() else {
        panic!("expected unknown statement to remain unsupported");
    };
    assert_eq!(unknown.kind, UnsupportedAbilityKind::Unknown);
}

/// Preserve combined Saga chapters and unsupported chapter text.
#[test]
fn parses_combined_saga_chapters_in_order_and_keeps_unsupported_text() {
    let card = parse_oracle_text("I, II — Draw a card.\nIII — Venture into the dungeon.", &[]);
    let chapters = card.saga_chapters().collect::<Vec<&OracleSagaChapter>>();
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].chapters, [1, 2]);
    assert!(matches!(
        chapters[0].effects.as_slice(),
        [OracleEffect::Draw(1)]
    ));
    assert_eq!(chapters[1].chapters, [3]);
    assert!(
        matches!(chapters[1].effects.as_slice(), [OracleEffect::Unsupported(text)] if text.to_ascii_lowercase().contains("venture"))
    );
}
