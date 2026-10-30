//! Hand-built search rules retain exact filters, counts, and destination capacities.

use super::*;
use crate::deck::simulator::model::{ManaColor, SearchCardType, SearchDestination, SearchSpec};
use crate::deck::simulator::oracle_ast::search::{OracleMultiSearch, OracleSearchPlacement};

/// Exact instant and sorcery selectors remain distinct from their supported union.
#[test]
fn typed_search_lowering_retains_exact_spell_types_and_all_constraints() {
    for card_type in [
        SearchCardType::Instant,
        SearchCardType::Sorcery,
        SearchCardType::InstantSorcery,
    ] {
        let effect = OracleEffect::Search(SearchSpec {
            card_type: Some(card_type),
            color: Some(ManaColor::Green),
            max_mana_value: Some(3),
            destination: SearchDestination::LibraryTop,
            optional: true,
            top_count: Some(5),
            shuffle: true,
            ..SearchSpec::default()
        });
        for lowered in [
            effect.to_runtime().expect("the typed search lowers"),
            lower_effect_sequence(&[effect])
                .expect("the complete search sequence lowers")
                .remove(0),
        ] {
            let SimEffect::Search(actual) = lowered else {
                panic!("a typed search stays a search");
            };
            assert_eq!(actual.card_type, Some(card_type));
            assert_eq!(actual.color, Some(ManaColor::Green));
            assert!(!actual.colorless);
            assert_eq!(actual.mana_value, None);
            assert_eq!(actual.max_mana_value, Some(3));
            assert_eq!(actual.min_mana_value, None);
            assert_eq!(actual.destination, SearchDestination::LibraryTop);
            assert!(actual.optional && actual.shuffle);
            assert_eq!(actual.top_count, Some(5));
            assert!(!actual.non_human && !actual.basic_land_only);
            assert_eq!(actual.land_types, [false; 5]);
        }
    }
}

/// A required Basic supertype survives independently of the chosen Forest subtype.
#[test]
fn typed_basic_subtype_search_retains_both_land_constraints() {
    let effect = OracleEffect::Search(SearchSpec {
        card_type: Some(SearchCardType::BasicLand),
        basic_land_only: true,
        land_types: [false, false, false, false, true],
        destination: SearchDestination::BattlefieldTapped,
        shuffle: true,
        ..SearchSpec::default()
    });
    let SimEffect::Search(actual) = effect.to_runtime().expect("the typed land search lowers")
    else {
        panic!("the typed land search remains one search");
    };
    assert_eq!(actual.card_type, Some(SearchCardType::BasicLand));
    assert!(actual.basic_land_only);
    assert_eq!(actual.land_types, [false, false, false, false, true]);
    assert_eq!(actual.destination, SearchDestination::BattlefieldTapped);
    assert!(actual.shuffle);
    assert_eq!(actual.color, None);
    assert!(!actual.colorless && !actual.optional && !actual.non_human);
    assert_eq!(actual.mana_value, None);
    assert_eq!(actual.max_mana_value, None);
    assert_eq!(actual.min_mana_value, None);
    assert_eq!(actual.top_count, None);
}

/// State an independent two-destination plan for accepted and rejected lowering checks.
fn two_destination_search() -> OracleMultiSearch {
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

/// Both destinations and a later reward survive complete multiple-result lowering.
#[test]
fn typed_two_destination_search_retains_the_whole_effect_sequence() {
    let effect = OracleEffect::MultiSearch(two_destination_search());
    let SimEffect::MultiSearch(actual) = effect.to_runtime().expect("the complete plan lowers")
    else {
        panic!("the plan remains a multiple-result search");
    };
    assert_two_destination_search(&actual);
    let lowered = lower_effect_sequence(&[effect, OracleEffect::GainLife(3)])
        .expect("both the complete search and its later reward lower");
    let [SimEffect::MultiSearch(actual), SimEffect::GainLife(3)] = lowered.as_slice() else {
        panic!("the complete search must precede its linked reward");
    };
    assert_two_destination_search(actual);
}

/// Check every runtime field against the independently written two-destination rule.
fn assert_two_destination_search(actual: &crate::deck::simulator::model::MultiSearchSpec) {
    assert_eq!(actual.card_type, SearchCardType::BasicLand);
    assert_eq!(actual.land_types, [false; 5]);
    assert_eq!((actual.min_cards, actual.max_cards), (0, 2));
    assert!(actual.reveal && actual.shuffle);
    assert_eq!(actual.destinations.len(), 2);
    assert_eq!(
        actual.destinations[0].destination,
        SearchDestination::BattlefieldTapped
    );
    assert_eq!(actual.destinations[0].max_cards, 1);
    assert_eq!(actual.destinations[1].destination, SearchDestination::Hand);
    assert_eq!(actual.destinations[1].max_cards, 1);
}

/// Invalid result bounds cannot leave a partial search or its later reward executable.
#[test]
fn invalid_two_destination_search_blocks_the_whole_effect_sequence() {
    let mut insufficient_capacity = two_destination_search();
    insufficient_capacity.max_cards = 3;
    let mut reversed_bounds = two_destination_search();
    reversed_bounds.min_cards = 3;
    let mut no_destinations = two_destination_search();
    no_destinations.destinations.clear();
    for plan in [insufficient_capacity, reversed_bounds, no_destinations] {
        let effect = OracleEffect::MultiSearch(plan);
        assert!(effect.to_runtime().is_none());
        assert!(lower_effect_sequence(&[effect, OracleEffect::GainLife(3)]).is_none());
    }
}
