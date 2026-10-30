use super::*;
use crate::deck::simulator::model::Cost;
use crate::deck::simulator::oracle_ast::{KeywordArgument, OracleKeyword, OracleKeywordName};
use crate::deck::simulator::oracle_parser::{parse_oracle_card, parse_oracle_text};
use crate::deck::simulator::scenario_test_support::row;

/// State the modeled red Escape cost without using the source parser.
fn escape_mana() -> Cost {
    Cost {
        pips: [0, 0, 0, 2, 0],
        ..Cost::default()
    }
}

/// Lower a hand-built Escape payment independently of keyword parsing.
fn lower_escape(arguments: &[KeywordArgument]) -> Option<Cost> {
    let source = row("Escape lowering control", "{3}{R}{R}", "Creature", "");
    let oracle = OracleCard {
        keywords: vec![OracleKeyword {
            name: OracleKeywordName::Escape,
            arguments: arguments.to_vec(),
        }],
        abilities: Vec::new(),
        ability_sources: Vec::new(),
        spell_data: Default::default(),
        static_data: Default::default(),
        land: Default::default(),
        stations: Default::default(),
        library_graveyard_trigger: None,
    };
    let card = super::card::lower_oracle_card(&source, &oracle);
    assert_eq!(
        card.cost,
        Cost {
            generic: 3,
            ..escape_mana()
        }
    );
    assert!(!card.spell_data.unsupported_additional_cost);
    card.spell_data.own_escape
}

/// One complete mana payment and exactly three other cards are the supported route.
#[test]
fn typed_escape_accepts_only_the_complete_three_card_payment() {
    for cost in [escape_mana(), Cost::default()] {
        for arguments in [
            vec![KeywordArgument::Mana(cost), KeywordArgument::EscapeCards(3)],
            vec![KeywordArgument::EscapeCards(3), KeywordArgument::Mana(cost)],
        ] {
            assert_eq!(lower_escape(&arguments), Some(cost));
        }
    }
}

/// Extra life, numeric, unknown, and separate mana payments cannot be omitted.
#[test]
fn typed_escape_rejects_every_extra_payment_in_any_order() {
    for extra in [
        KeywordArgument::Life(1),
        KeywordArgument::Number(1),
        KeywordArgument::Other("Discard a card".into()),
        KeywordArgument::Mana(Cost {
            generic: 1,
            ..Cost::default()
        }),
    ] {
        for position in 0..=2 {
            let mut arguments = vec![
                KeywordArgument::Mana(escape_mana()),
                KeywordArgument::EscapeCards(3),
            ];
            arguments.insert(position, extra.clone());
            assert_eq!(lower_escape(&arguments), None, "{arguments:?}");
        }
    }
}

/// Missing, duplicate, zero-card, and eight-card costs do not grant Escape.
#[test]
fn typed_escape_rejects_missing_duplicate_and_wrong_count_payments() {
    for arguments in [
        vec![],
        vec![KeywordArgument::Mana(escape_mana())],
        vec![KeywordArgument::EscapeCards(3)],
        vec![
            KeywordArgument::Mana(escape_mana()),
            KeywordArgument::EscapeCards(0),
        ],
        vec![
            KeywordArgument::Mana(escape_mana()),
            KeywordArgument::EscapeCards(8),
        ],
        vec![
            KeywordArgument::Mana(escape_mana()),
            KeywordArgument::Mana(escape_mana()),
            KeywordArgument::EscapeCards(3),
        ],
        vec![
            KeywordArgument::Mana(escape_mana()),
            KeywordArgument::EscapeCards(3),
            KeywordArgument::EscapeCards(3),
        ],
        vec![
            KeywordArgument::Mana(escape_mana()),
            KeywordArgument::EscapeCards(3),
            KeywordArgument::EscapeCards(8),
        ],
    ] {
        assert_eq!(lower_escape(&arguments), None, "{arguments:?}");
    }
}

/// Hand-built mana data cannot bypass unsupported-symbol or snow-payment rules.
#[test]
fn typed_escape_rejects_unsupported_mana_payments() {
    let mut empty_hybrid = escape_mana();
    empty_hybrid.hybrid[0] = 1;
    for cost in [
        Cost {
            unsupported: true,
            ..escape_mana()
        },
        Cost {
            snow: 1,
            ..escape_mana()
        },
        empty_hybrid,
    ] {
        assert_eq!(
            lower_escape(&[KeywordArgument::Mana(cost), KeywordArgument::EscapeCards(3)]),
            None
        );
    }
}

/// The parser retains a required life payment before, between, or after other costs.
#[test]
fn parsed_escape_retains_extra_life_in_every_payment_order() {
    for tail in [
        "Pay 1 life, {R}{R}, Exile three other cards from your graveyard",
        "{R}{R}, Pay 1 life, Exile three other cards from your graveyard",
        "{R}{R}, Exile three other cards from your graveyard, Pay 1 life",
    ] {
        let text = format!("Escape—{tail}.");
        let oracle = parse_oracle_text(&text, &[]);
        let arguments = &oracle.keywords[0].arguments;
        assert_eq!(arguments.len(), 3, "{text}");
        assert!(
            arguments
                .iter()
                .any(|argument| matches!(argument, KeywordArgument::Life(1))),
            "{text}"
        );
        assert!(
            arguments.iter().any(
                |argument| matches!(argument, KeywordArgument::Mana(cost) if *cost == escape_mana())
            ),
            "{text}"
        );
        assert!(
            arguments
                .iter()
                .any(|argument| matches!(argument, KeywordArgument::EscapeCards(3))),
            "{text}"
        );
        assert_eq!(lower_escape(arguments), None, "{text}");
    }
}

/// Unsupported selectors and payment leftovers stay visible instead of becoming partial costs.
#[test]
fn parsed_escape_retains_unknown_costs_and_qualifiers() {
    for payment in [
        "Discard a card",
        "Exile three other creature cards from your graveyard",
        "Exile three tapped other cards from your graveyard",
        "Exile three or more other cards from your graveyard",
        "Exile three! other cards from your graveyard",
        "Exile X other cards from your graveyard",
        "Exile -3 other cards from your graveyard",
        "Exile 4294967296 other cards from your graveyard",
        "Exile three other cards from your graveyard at random",
        "{R}{R} and pay 1 life",
        "{R}{R} {1}",
        "{R}{R",
        "Pay 1 life unless you control a creature",
    ] {
        let text =
            format!("Escape—{{R}}{{R}}, {payment}, Exile three other cards from your graveyard.");
        let oracle = parse_oracle_text(&text, &[]);
        let arguments = &oracle.keywords[0].arguments;
        assert!(
            arguments.iter().any(
                |argument| matches!(argument, KeywordArgument::Other(source) if source == payment)
            ),
            "{text}: {arguments:?}"
        );
        assert_eq!(lower_escape(arguments), None, "{text}");
    }
}

/// Unsupported first payments remain typed even before any recognizable mana cost.
#[test]
fn parsed_escape_preserves_unknown_first_payments_and_unsupported_mana() {
    let oracle = parse_oracle_text(
        "Escape—Sacrifice a creature, {R}{R}, Exile three other cards from your graveyard.",
        &[],
    );
    let arguments = &oracle.keywords[0].arguments;
    assert!(
        matches!(&arguments[0], KeywordArgument::Other(source) if source == "Sacrifice a creature")
    );
    assert_eq!(lower_escape(arguments), None);

    let oracle = parse_oracle_text(
        "Escape—{R}{Q}, Exile three other cards from your graveyard.",
        &[],
    );
    let arguments = &oracle.keywords[0].arguments;
    assert!(matches!(&arguments[0], KeywordArgument::Mana(cost) if cost.unsupported));
    assert_eq!(lower_escape(arguments), None);
}

/// A later mana fragment cannot hide an unknown payment before it.
#[test]
fn parsed_escape_retains_leading_unknown_payment_without_scanning_ahead() {
    for payment in ["Discard a card", "Remove a counter", "Pay a debt"] {
        let oracle = parse_oracle_text(
            &format!("Escape—{payment}, {{R}}{{R}}, Exile three other cards from your graveyard."),
            &[],
        );
        let arguments = &oracle.keywords[0].arguments;
        assert!(matches!(arguments.as_slice(), [
            KeywordArgument::Other(source),
            KeywordArgument::Mana(cost),
            KeywordArgument::EscapeCards(3),
        ] if source == payment && *cost == escape_mana()));
        assert_eq!(lower_escape(arguments), None);
    }
}

/// Parsing preserves exact exile counts and every separate mana or exile clause.
#[test]
fn parsed_escape_counts_and_duplicate_payments_stay_distinct() {
    for (count, expected, supported) in [
        ("three", 3, true),
        ("3", 3, true),
        ("0", 0, false),
        ("eight", 8, false),
    ] {
        let source = row(
            "Escape count control",
            "{3}{R}{R}",
            "Creature",
            &format!("Escape—{{R}}{{R}}, Exile {count} other cards from your graveyard."),
        );
        let oracle = parse_oracle_card(&source);
        let arguments = &oracle.keywords[0].arguments;
        assert_eq!(arguments.len(), 2);
        assert!(matches!(arguments[1], KeywordArgument::EscapeCards(value) if value == expected));
        assert_eq!(
            parse_sim_card(&source).spell_data.own_escape,
            supported.then_some(escape_mana())
        );
    }
    for tail in [
        "{R}{R}, {1}, Exile three other cards from your graveyard",
        "{R}{R}, Exile three other cards from your graveyard, Exile three other cards from your graveyard",
    ] {
        let oracle = parse_oracle_text(&format!("Escape—{tail}."), &[]);
        assert_eq!(oracle.keywords[0].arguments.len(), 3);
        assert_eq!(lower_escape(&oracle.keywords[0].arguments), None);
    }
}
