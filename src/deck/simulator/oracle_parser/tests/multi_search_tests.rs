use super::*;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::oracle_ast::{OracleAbility, OracleEffect};
use crate::deck::simulator::oracle_parser::parse_oracle_card;

/// Captured split searches retain both result destinations and their shared reveal.
#[test]
fn captured_split_searches_keep_the_complete_plan() {
    for name in ["Cultivate", "Kodama's Reach"] {
        let source = named_row(name);
        let plan = parse_multi_search(&source.oracle_text).expect("captured split search parses");
        assert_eq!(plan.card_type, SearchCardType::BasicLand);
        assert_eq!(plan.land_types, [false; 5]);
        assert_eq!((plan.min_cards, plan.max_cards), (0, 2));
        assert!(plan.reveal);
        assert!(plan.shuffle);
        assert_eq!(
            plan.destinations,
            [
                OracleSearchPlacement {
                    destination: SearchDestination::BattlefieldTapped,
                    max_cards: 1,
                },
                OracleSearchPlacement {
                    destination: SearchDestination::Hand,
                    max_cards: 1,
                },
            ]
        );
    }
}

/// Captured Forest searches do not acquire a Basic or tapped-entry restriction.
#[test]
fn captured_forest_search_keeps_its_subtype_and_untapped_destination() {
    let source = named_row("Skyshroud Claim");
    let plan = parse_multi_search(&source.oracle_text).expect("captured Forest search parses");
    assert_eq!(plan.card_type, SearchCardType::Land);
    assert_eq!(plan.land_types, [false, false, false, false, true]);
    assert_eq!((plan.min_cards, plan.max_cards), (0, 2));
    assert!(!plan.reveal);
    assert!(plan.shuffle);
    assert_eq!(
        plan.destinations,
        [OracleSearchPlacement {
            destination: SearchDestination::Battlefield,
            max_cards: 2,
        }]
    );
}

/// The full card parser selects the same rule when the card name changes.
#[test]
fn renamed_sources_use_the_same_typed_search_rule() {
    for name in ["Cultivate", "Kodama's Reach", "Skyshroud Claim"] {
        let mut source = named_row(name);
        let expected = parse_multi_search(&source.oracle_text).expect("captured search parses");
        source.name = "Renamed land search".into();
        let parsed = parse_oracle_card(&source);
        let plan = parsed.abilities.iter().find_map(|ability| match ability {
            OracleAbility::Spell(spell) => spell.effects.iter().find_map(|effect| match effect {
                OracleEffect::MultiSearch(plan) => Some(plan),
                _ => None,
            }),
            _ => None,
        });
        assert_eq!(plan, Some(&expected), "{name}");
    }
}

/// Fixed counts and written destination counts are independent of the source card.
#[test]
fn changed_counts_retain_the_full_split_capacity() {
    let source = named_row("Cultivate");
    let changed = source
        .oracle_text
        .replace("up to two", "three")
        .replace("put one", "put two");
    let plan = parse_multi_search(&changed).expect("fixed three-card search parses");
    assert_eq!((plan.min_cards, plan.max_cards), (3, 3));
    assert_eq!(plan.destinations[0].max_cards, 2);
    assert_eq!(plan.destinations[1].max_cards, 1);
    assert_eq!(
        plan.destinations[0].destination,
        SearchDestination::BattlefieldTapped
    );
    assert_eq!(plan.destinations[1].destination, SearchDestination::Hand);
}

/// Other land types and explicit count words use the same complete grammar.
#[test]
fn subtype_alternatives_and_group_references_keep_their_counts() {
    let source = named_row("Skyshroud Claim");
    let changed = source
        .oracle_text
        .replace("up to two Forest", "3 basic Plains, Island, or Forest")
        .replace("put them", "put those cards")
        .replace("onto the battlefield", "into your hand");
    let plan = parse_multi_search(&changed).expect("typed alternatives parse");
    assert_eq!(plan.card_type, SearchCardType::BasicLand);
    assert_eq!(plan.land_types, [true, true, false, false, true]);
    assert_eq!((plan.min_cards, plan.max_cards), (3, 3));
    assert_eq!(plan.destinations[0].max_cards, 3);
    assert_eq!(plan.destinations[0].destination, SearchDestination::Hand);
}

/// A search without a printed shuffle must not gain one during parsing.
#[test]
fn search_without_shuffle_keeps_its_original_library_order_rule() {
    let source = named_row("Skyshroud Claim");
    let changed = source.oracle_text.replace(", then shuffle", "");
    let plan = parse_multi_search(&changed).expect("search without a shuffle parses");
    assert!(!plan.shuffle);
}

/// Required source, controller, and selector restrictions cannot widen into a supported rule.
#[test]
fn captured_search_mutations_reject_unmodeled_requirements() {
    let source = named_row("Cultivate");
    for changed in [
        source.oracle_text.replace("your library", "your graveyard"),
        source
            .oracle_text
            .replace("your library", "target player's library"),
        source
            .oracle_text
            .replace("your library", "your library and graveyard"),
        source.oracle_text.replace("up to two", "up to X"),
        source.oracle_text.replace("up to two", "any number of"),
        source
            .oracle_text
            .replace("basic land cards", "basic land cards with different names"),
        source.oracle_text.replace(
            "basic land cards",
            "basic land cards with different basic land types",
        ),
        source
            .oracle_text
            .replace("basic land cards", "basic land cards named Forest"),
        source.oracle_text.replace(
            "basic land cards",
            "basic land cards with mana value X or less",
        ),
        source.oracle_text.replace(
            "battlefield tapped",
            "battlefield tapped under an opponent's control",
        ),
        source
            .oracle_text
            .replace("into your hand", "on the bottom of your library"),
        source
            .oracle_text
            .replace("then shuffle", "then shuffle and draw a card"),
        source.oracle_text.replace("put one", "put two"),
        source.oracle_text.replace("the other", "another two"),
    ] {
        assert!(parse_multi_search(&changed).is_none(), "{changed}");
    }
}

/// Forest alternatives do not stand for intersections or a variable result count.
#[test]
fn captured_forest_mutations_reject_unretained_filters_and_destinations() {
    let source = named_row("Skyshroud Claim");
    for changed in [
        source.oracle_text.replace("Forest", "Forest and Plains"),
        source.oracle_text.replace("Forest", "snow Forest"),
        source
            .oracle_text
            .replace("Forest cards", "Forest cards with different names"),
        source.oracle_text.replace("up to two", "X"),
        source.oracle_text.replace("put them", "put one"),
        source.oracle_text.replace(
            "onto the battlefield",
            "onto the battlefield under its owner's control",
        ),
        source.oracle_text.replace(
            "onto the battlefield",
            "onto the battlefield with a +1/+1 counter on each",
        ),
        source.oracle_text.replace(
            "then shuffle",
            "then shuffle your graveyard into your library",
        ),
    ] {
        assert!(parse_multi_search(&changed).is_none(), "{changed}");
    }
}

/// A top placement followed by shuffle retains both operations without reversing their order.
#[test]
fn top_placement_keeps_its_printed_trailing_shuffle() {
    let original = named_row("Skyshroud Claim");
    for renamed in [false, true] {
        let mut source = original.clone();
        source.oracle_text = source
            .oracle_text
            .replace("onto the battlefield", "on top of your library");
        if renamed {
            source.name = "Renamed top search".into();
        }
        let plan = parse_multi_search(&source.oracle_text).expect("top placement precedes shuffle");
        assert_eq!(plan.max_cards, 2);
        assert_eq!(plan.land_types, [false, false, false, false, true]);
        assert_eq!(plan.destinations.len(), 1);
        assert_eq!(
            plan.destinations[0].destination,
            SearchDestination::LibraryTop
        );
        assert_eq!(plan.destinations[0].max_cards, 2);
        assert!(plan.shuffle);
        let parsed = parse_oracle_card(&source);
        assert!(
            matches!(parsed.abilities.as_slice(), [OracleAbility::Spell(spell)]
            if matches!(spell.effects.as_slice(), [OracleEffect::MultiSearch(actual)] if *actual == plan))
        );
    }
}

/// One compound placement can put different results onto the battlefield with different tapped states.
#[test]
fn mixed_battlefield_placements_keep_both_entry_instructions() {
    let original = named_row("Cultivate");
    for renamed in [false, true] {
        let mut source = original.clone();
        source.oracle_text = source.oracle_text.replace(
            "and the other into your hand",
            "and the other onto the battlefield",
        );
        if renamed {
            source.name = "Renamed mixed entry search".into();
        }
        let plan = parse_multi_search(&source.oracle_text)
            .expect("one compound battlefield placement parses");
        assert_eq!(plan.destinations.len(), 2);
        assert_eq!(
            plan.destinations[0].destination,
            SearchDestination::BattlefieldTapped
        );
        assert_eq!(
            plan.destinations[1].destination,
            SearchDestination::Battlefield
        );
        assert!(
            plan.destinations
                .iter()
                .all(|placement| placement.max_cards == 1)
        );
        assert!(plan.shuffle);
        let parsed = parse_oracle_card(&source);
        assert!(
            matches!(parsed.abilities.as_slice(), [OracleAbility::Spell(spell)]
            if matches!(spell.effects.as_slice(), [OracleEffect::MultiSearch(actual)] if *actual == plan))
        );
    }
}

/// A shuffle before placement cannot reuse a plan that retains only a trailing shuffle.
#[test]
fn shuffle_before_multiple_result_placement_is_not_reordered() {
    let source = named_row("Skyshroud Claim");
    let changed = source.oracle_text.replace(
        ", put them onto the battlefield, then shuffle",
        ", then shuffle and put them on top of your library",
    );
    assert_ne!(changed, source.oracle_text);
    assert!(parse_multi_search(&changed).is_none());
}

/// Separate placement instructions need operation boundaries that the compound plan does not retain.
#[test]
fn separate_placement_verbs_do_not_collapse_into_one_entry_event() {
    let source = named_row("Cultivate");
    for replacement in ["and put the other", ", then put the other"] {
        let changed = source.oracle_text.replace("and the other", replacement);
        assert_ne!(changed, source.oracle_text);
        assert!(parse_multi_search(&changed).is_none(), "{changed}");
    }
}
