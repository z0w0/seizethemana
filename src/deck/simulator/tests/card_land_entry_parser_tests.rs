use super::*;

/// Exact entry wording keeps number words and numerals in typed data.
#[test]
fn land_entry_parser_preserves_supported_thresholds() {
    use crate::deck::simulator::model::LandEntryCondition as Runtime;
    use crate::deck::simulator::oracle_ast::OracleLandEntryCondition as LandEntryCondition;
    use crate::deck::simulator::oracle_parser::parse_oracle_card;
    for (text, expected) in [
        (
            "This land enters tapped unless you control two or fewer other lands.",
            LandEntryCondition::OtherLandsAtMost(2),
        ),
        (
            "This land enters tapped unless you control 0 or fewer other lands.",
            LandEntryCondition::OtherLandsAtMost(0),
        ),
        (
            "This land enters tapped unless you control two or more other lands.",
            LandEntryCondition::OtherLandsAtLeast(2),
        ),
        (
            "This land enters tapped unless you control 7 or more other lands.",
            LandEntryCondition::OtherLandsAtLeast(7),
        ),
        (
            "This land enters tapped unless you have three or more opponents.",
            LandEntryCondition::OpponentsAtLeast(3),
        ),
        (
            "This land enters tapped unless you have 12 or more opponents.",
            LandEntryCondition::OpponentsAtLeast(12),
        ),
    ] {
        let source = row("Renamed conditional source", "", "Land", text);
        let oracle = parse_oracle_card(&source);
        assert_eq!(
            oracle.land.untapped_entry_condition,
            Some(expected),
            "{text}"
        );
        let cards = deck(&[source]);
        let runtime = match expected {
            LandEntryCondition::OtherLandsAtMost(count) => Runtime::OtherLandsAtMost(count),
            LandEntryCondition::OtherLandsAtLeast(count) => Runtime::OtherLandsAtLeast(count),
            LandEntryCondition::OpponentsAtLeast(count) => Runtime::OpponentsAtLeast(count),
            LandEntryCondition::ControlsLandTypes(types) => Runtime::ControlsLandTypes(types),
        };
        assert_eq!(cards.cards[0].untapped_entry_condition, Some(runtime));
        assert!(
            cards.cards[0].enters_tapped,
            "the static field cannot promise untapped entry for every state"
        );
    }
}

/// Extra requirements cannot leave a supported fragment that grants untapped entry.
#[test]
fn land_entry_parser_rejects_combined_and_unknown_conditions() {
    use crate::deck::simulator::oracle_parser::parse_oracle_card;
    for text in [
        "This land enters tapped unless you control two or fewer other lands and an Elf.",
        "This land enters tapped unless you control two or fewer other lands or have two or more opponents.",
        "This land enters tapped unless you have two or more opponents and control a Forest.",
        "This land enters tapped unless you control two or fewer other lands or you may pay 2 life.",
        "This land enters tapped unless you control exactly two other lands.",
        "This land enters tapped unless you control 4294967296 or more other lands.",
        "This land enters tapped unless you control two or fewer other lands. This land enters tapped unless you have two or more opponents.",
        "Whenever a land enters tapped unless you control two or fewer other lands, draw a card.",
    ] {
        let source = row("Conditional safety control", "", "Land", text);
        let oracle = parse_oracle_card(&source);
        assert_eq!(oracle.land.untapped_entry_condition, None, "{text}");
        assert!(oracle.land.enters_tapped, "{text}");
        assert_eq!(oracle.land.life_to_untap, 0, "{text}");
    }
}
