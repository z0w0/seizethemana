use super::*;
use crate::deck::simulator::model::{SearchCardType, SearchDestination};

/// Build typed requirements without using the parser as a test oracle.
fn elf_spec(count: u32) -> OracleBeholdSpec {
    OracleBeholdSpec {
        card_types: Vec::new(),
        subtypes: vec!["elf".into()],
        count,
    }
}

/// Lowering copies the full type conjunction into a distinct runtime type.
#[test]
fn lower_spec_retains_card_types_subtypes_and_count() {
    let ast = OracleBeholdSpec {
        card_types: vec!["creature".into()],
        subtypes: vec!["gamma".into()],
        count: 1,
    };
    let runtime = lower_spec(&ast).expect("a singular typed quality is supported");
    assert_eq!(runtime.card_types, ["creature"]);
    assert_eq!(runtime.subtypes, ["gamma"]);
    assert_eq!(runtime.count, 1);
}

/// Unsupported distinct-choice counts never become one free Behold choice.
#[test]
fn lower_spec_rejects_empty_zero_and_multiple_choices() {
    for count in [0, 2, 3] {
        assert!(lower_spec(&elf_spec(count)).is_none());
    }
    assert!(
        lower_spec(&OracleBeholdSpec {
            card_types: Vec::new(),
            subtypes: Vec::new(),
            count: 1,
        })
        .is_none()
    );
}

/// The generic alternative is an extra payment, not a discount to printed cost.
#[test]
fn lower_alternative_retains_the_mana_payment() {
    let cost = OracleBeholdAlternative {
        spec: elf_spec(1),
        generic: 2,
    };
    let runtime = lower_alternative(&cost).expect("singular Behold-or-pay is supported");
    assert_eq!(runtime.generic, 2);
    assert_eq!(runtime.spec.subtypes, ["elf"]);
    assert!(
        lower_alternative(&OracleBeholdAlternative {
            spec: elf_spec(3),
            generic: 2,
        })
        .is_none()
    );
}

/// A supported reward remains inside its Behold gate.
#[test]
fn lower_conditional_keeps_the_gate_and_ordered_rewards() {
    let runtime = lower_conditional(
        &elf_spec(1),
        &[
            OracleEffect::CreateTreasureTokens(1),
            OracleEffect::GainLife(2),
        ],
    )
    .expect("complete fixed rewards lower");
    let SimEffect::Behold { spec, effects } = runtime else {
        panic!("the reward must remain conditional");
    };
    assert_eq!(spec.subtypes, ["elf"]);
    assert!(matches!(
        effects.as_slice(),
        [SimEffect::Treasures(1), SimEffect::GainLife(2)]
    ));
}

/// A later unmodeled instruction blocks every reward in the conditional branch.
#[test]
fn lower_conditional_rejects_partial_or_multiple_choice_rewards() {
    assert!(
        lower_conditional(
            &elf_spec(1),
            &[
                OracleEffect::CreateTreasureTokens(1),
                OracleEffect::Unsupported("Exile the beheld card.".into()),
            ],
        )
        .is_none()
    );
    assert!(lower_conditional(&elf_spec(3), &[OracleEffect::GainLife(2)]).is_none());
}

/// A linked untap carries the complete original search rather than a global flag.
#[test]
fn lower_search_then_untap_retains_the_search_constraints() {
    let search = SearchSpec {
        card_type: Some(SearchCardType::BasicLand),
        basic_land_only: true,
        destination: SearchDestination::BattlefieldTapped,
        shuffle: true,
        ..Default::default()
    };
    let runtime = lower_search_then_untap(&search, &elf_spec(1))
        .expect("a tapped battlefield result can be conditionally untapped");
    let SimEffect::SearchThenBeholdUntap { search, spec } = runtime else {
        panic!("the search result must remain linked");
    };
    assert_eq!(search.card_type, Some(SearchCardType::BasicLand));
    assert!(search.basic_land_only);
    assert!(search.shuffle);
    assert_eq!(search.destination, SearchDestination::BattlefieldTapped);
    assert_eq!(spec.subtypes, ["elf"]);
}

/// A hand result cannot be used as a battlefield untap target.
#[test]
fn lower_search_then_untap_rejects_other_destinations_and_counts() {
    assert!(lower_search_then_untap(&SearchSpec::default(), &elf_spec(1)).is_none());
    let search = SearchSpec {
        destination: SearchDestination::BattlefieldTapped,
        ..Default::default()
    };
    assert!(lower_search_then_untap(&search, &elf_spec(3)).is_none());
}
