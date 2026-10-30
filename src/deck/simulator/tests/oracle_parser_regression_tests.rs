//! Regression tests for Oracle parsing and safe lowering.

use super::game::ManaPool;
use super::model::{AlternativeCostPayoff, SearchDestination};
use super::oracle_ast::OracleKeywordName;
use super::oracle_ast::{OracleAbility, OracleEffect as AstEffect};
use super::oracle_parser::parse_oracle_text;
use super::turn_loop_tests::{cast, deck, row, state};

/// Read a search effect from one activated ability.
fn activated_effect(text: &str) -> AstEffect {
    let card = parse_oracle_text(&format!("{{T}}: {text}"), &[]);
    let [OracleAbility::Activated(ability)] = card.abilities.as_slice() else {
        panic!("expected one activated ability for {text:?}");
    };
    ability.effects.first().expect("activated effect").clone()
}

/// Exact mana-value tutors find a matching target and leave a near miss.
#[test]
fn exact_mana_value_search_accepts_only_matching_targets() {
    let cards = deck(&[
        row(
            "Tutor",
            "{1}",
            "Sorcery",
            "Search your library for an artifact card with mana value 2, put that card into your hand, then shuffle.",
        ),
        row("Two", "{2}", "Artifact", ""),
        row("Three", "{3}", "Artifact", ""),
    ]);
    let mut st = state(vec![0], vec![1, 2]);
    let mut pool = ManaPool {
        flexible: 1,
        ..Default::default()
    };

    cast(&cards, &mut st, &mut pool);

    assert_eq!(st.hand, [super::model::CardIdx(1)]);
    assert_eq!(st.library, [super::model::CardIdx(2)]);
}

/// Source-dependent and unsupported selectors do not become open searches.
#[test]
fn unsupported_search_selectors_remain_inert() {
    for text in [
        "Search your library for a legendary permanent card with mana value less than this creature's power, put it onto the battlefield, then shuffle.",
        "Search your library for a legendary permanent card, put it onto the battlefield, then shuffle.",
        "Search your library for a snow creature card, put it into your hand, then shuffle.",
        "Search your library for a red or green creature card, put it into your hand, then shuffle.",
    ] {
        assert!(matches!(activated_effect(text), AstEffect::Unsupported(_)));
    }
}

/// Tutor destinations stay on top of the library with linked life loss.
#[test]
fn library_top_tutors_keep_destination_and_drawback() {
    for text in [
        "Search your library for a card, then shuffle and put that card on top of your library. You lose 2 life.",
        "Search your library for an instant or sorcery card, reveal it, then shuffle and put it on top of your library.",
    ] {
        let card = parse_oracle_text(text, &[]);
        let [OracleAbility::Spell(spell)] = card.abilities.as_slice() else {
            panic!("expected spell for {text:?}");
        };
        assert!(matches!(
            spell.effects.first(),
            Some(AstEffect::Search(spec)) if spec.destination == SearchDestination::LibraryTop && spec.shuffle
        ));
        if text.contains("lose 2 life") {
            assert!(matches!(
                spell.effects.get(1),
                Some(AstEffect::LoseLife { amount: 2, .. })
            ));
        }
    }
}

/// A single-result top placement cannot silently move a later shuffle before that placement.
#[test]
fn library_top_search_with_a_trailing_shuffle_blocks_linked_rewards() {
    for text in [
        "Search your library for a card, put that card on top of your library, then shuffle. You gain 2 life.",
        "Search your library for an instant or sorcery card, reveal it, put it on top of your library, then shuffle. You gain 2 life.",
    ] {
        let card = parse_oracle_text(text, &[]);
        let [OracleAbility::Spell(spell)] = card.abilities.as_slice() else {
            panic!("the search and reward belong to one spell");
        };
        assert!(matches!(
            spell.effects.as_slice(),
            [AstEffect::Unsupported(_), AstEffect::GainLife(2)]
        ));
        assert!(super::oracle_lower::lower_effect_sequence(&spell.effects).is_none());
        let lowered = super::oracle_lower::parse_sim_card(&row(
            "Rejected top search",
            "{1}",
            "Sorcery",
            text,
        ));
        assert!(lowered.spell_data.effects.is_empty());
        assert_eq!(lowered.spell_data.life_gain_on_cast, 0);
    }
}

/// Fixed multiple-result counts retain every destination capacity and a later supported reward.
#[test]
fn multiple_result_counts_keep_complete_searches_and_linked_rewards() {
    for (count, maximum, minimum) in [
        ("two", 2, 2),
        ("2", 2, 2),
        ("three", 3, 3),
        ("3", 3, 3),
        ("up to two", 2, 0),
        ("up to three", 3, 0),
    ] {
        let source = row(
            "Multiple-result count control",
            "{G}",
            "Sorcery",
            &format!(
                "Search your library for {count} basic land cards, put them onto the battlefield tapped, then shuffle. You gain 2 life."
            ),
        );
        let oracle = super::oracle_parser::parse_oracle_card(&source);
        let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
            panic!("the complete search and its reward remain one spell");
        };
        let [AstEffect::MultiSearch(plan), AstEffect::GainLife(2)] = spell.effects.as_slice()
        else {
            panic!("fixed land counts must keep the complete multiple-result rule");
        };
        assert_eq!(plan.card_type, super::model::SearchCardType::BasicLand);
        assert_eq!(plan.land_types, [false; 5]);
        assert_eq!((plan.min_cards, plan.max_cards), (minimum, maximum));
        assert!(!plan.reveal && plan.shuffle);
        assert_eq!(plan.destinations.len(), 1);
        assert_eq!(
            plan.destinations[0].destination,
            SearchDestination::BattlefieldTapped
        );
        assert_eq!(plan.destinations[0].max_cards, maximum);
        let lowered = super::oracle_lower::lower_effect_sequence(&spell.effects)
            .expect("all fixed counts lower");
        let [
            super::model::SimEffect::MultiSearch(actual),
            super::model::SimEffect::GainLife(2),
        ] = lowered.as_slice()
        else {
            panic!("the complete runtime search must precede its reward");
        };
        assert_eq!(actual.card_type, plan.card_type);
        assert_eq!(actual.land_types, plan.land_types);
        assert_eq!((actual.min_cards, actual.max_cards), (minimum, maximum));
        assert_eq!((actual.reveal, actual.shuffle), (plan.reveal, plan.shuffle));
        assert_eq!(actual.destinations.len(), 1);
        assert_eq!(
            actual.destinations[0].destination,
            SearchDestination::BattlefieldTapped
        );
        assert_eq!(actual.destinations[0].max_cards, maximum);
    }
}

/// Escape only lowers when the printed exile count matches runtime payment.
#[test]
fn escape_payment_requires_the_complete_supported_count() {
    let ox = row(
        "Ox of Agonas",
        "{3}{R}{R}",
        "Creature",
        "Escape—{3}{R}{R}, Exile eight other cards from your graveyard.",
    );
    let lowered_ox = super::oracle_lower::parse_sim_card(&ox);
    assert!(lowered_ox.spell_data.own_escape.is_none());

    let supported = parse_oracle_text(
        "Escape—{2}{B}, Exile three other cards from your graveyard.",
        &[],
    );
    assert_eq!(
        supported.keyword_number(&OracleKeywordName::Escape),
        None,
        "the exile count is not confused with a mana parameter"
    );
    assert!(
        supported.keywords[0]
            .arguments
            .iter()
            .any(|argument| matches!(argument, super::oracle_ast::KeywordArgument::EscapeCards(3)))
    );
    assert!(
        super::oracle_lower::parse_sim_card(&row(
            "Supported Escape",
            "{3}{B}",
            "Creature",
            "Escape—{2}{B}, Exile three other cards from your graveyard.",
        ))
        .spell_data
        .own_escape
        .is_some()
    );
}

/// Unsupported alternative payments do not remove the ordinary cast route.
#[test]
fn alternative_cost_with_unmodeled_life_payment_is_disabled() {
    let lowered = super::oracle_lower::parse_sim_card(&row(
        "Alternative",
        "{3}{U}",
        "Sorcery",
        "You may pay 1 life and exile a blue card from your hand rather than pay this spell's mana cost. Draw a card.",
    ));
    assert_eq!(lowered.min_cost.total(), 4);
    assert!(lowered.spell_data.alternative_cast_cost.is_none());

    let supported = super::oracle_lower::parse_sim_card(&row(
        "Supported alternative",
        "{3}{U}",
        "Sorcery",
        "You may exile a blue card from your hand rather than pay this spell's mana cost.",
    ));
    assert!(
        matches!(
            supported.spell_data.alternative_cast_cost,
            Some(cost) if cost.count == 1
                && cost.filter.color == Some(super::model::ManaColor::Blue)
                && cost.payoff == AlternativeCostPayoff::None
        ),
        "unexpected alternative cost: {:?}",
        supported.spell_data.alternative_cast_cost
    );
}

/// Scaled life gain stays inert when the parser cannot model its selector.
#[test]
fn life_gain_per_attacking_elf_is_not_lowered_as_one_life() {
    let parsed = parse_oracle_text(
        "Whenever an Elf you control attacks, you gain 1 life for each attacking Elf you control.",
        &[],
    );
    let [OracleAbility::Triggered(trigger)] = parsed.abilities.as_slice() else {
        panic!("expected an attack trigger");
    };
    assert!(matches!(
        trigger.effects.as_slice(),
        [AstEffect::Unsupported(_)]
    ));
    assert!(trigger.to_runtime_all().is_empty());
}
