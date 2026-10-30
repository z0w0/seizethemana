use super::*;
use crate::db::CardRow;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::game::{CardRef, ManaPool};
use crate::deck::simulator::game_effects::apply_effect_at;
use crate::deck::simulator::model::SimEffect;
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use crate::deck::simulator::turn_loop_tests::cast;
use rand::RngCore;

/// Compare physical card identities across all modeled zones.
fn physical_cards(st: &GameState) -> Vec<CardIdx> {
    let mut cards: Vec<_> = st
        .library
        .iter()
        .chain(&st.hand)
        .chain(&st.graveyard)
        .chain(&st.exile)
        .copied()
        .chain(
            st.battlefield
                .iter()
                .filter_map(|permanent| match permanent.card {
                    CardRef::Deck(index) => Some(index),
                    CardRef::Commander { .. } | CardRef::Token(_) => None,
                }),
        )
        .collect();
    cards.sort_unstable_by_key(|index| index.0);
    cards
}

/// Read the executable plan produced from the row's complete Oracle source.
fn compiled_search(cards: &SimDeck) -> &MultiSearchSpec {
    let [SimEffect::MultiSearch(spec)] = cards.cards[0].spell_data.effects.as_slice() else {
        panic!("a supported source lowers to exactly one typed multiple-result search");
    };
    spec
}

/// Captured split searches use one physical result per destination and shuffle once on failure too.
#[test]
fn captured_split_searches_resolve_zero_one_or_two_legal_results() {
    for name in ["Cultivate", "Kodama's Reach"] {
        for matches in 0..=2 {
            let cards = deck(&[
                named_row(name),
                row("Forest", "", "Basic Land — Forest", ""),
                row("Plains", "", "Basic Land — Plains", ""),
                row("Nonbasic Forest", "", "Land — Forest Swamp", ""),
                row(
                    "Spell // Forest back",
                    "{9} // ",
                    "Sorcery // Land — Forest",
                    "",
                ),
                row("A", "{9}", "Sorcery", ""),
                row("B", "{9}", "Sorcery", ""),
                row("C", "{9}", "Sorcery", ""),
            ]);
            let mut library: Vec<_> = (3..8).collect();
            library.extend(1..=matches);
            let mut st = state(vec![0], library);
            let physical_before = physical_cards(&st);
            let mut expected_rng = st.library_rng.clone();
            let mut expected_library = (3..8).map(CardIdx).collect::<Vec<_>>();
            shuffle(&mut expected_library, &mut expected_rng);
            let mut pool = ManaPool {
                flexible: 3,
                ..Default::default()
            };

            assert_eq!(compiled_search(&cards).max_cards, 2);
            let spent = cast(&cards, &mut st, &mut pool);

            assert_eq!(spent, [3.0]);
            assert_eq!(st.graveyard, [CardIdx(0)]);
            assert_eq!(st.battlefield.len(), matches.min(1));
            assert_eq!(st.hand.len(), matches.saturating_sub(1));
            assert!(st.battlefield.iter().all(|permanent| permanent.tapped));
            assert_eq!(st.library, expected_library, "{name}: {matches}");
            assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
            assert_eq!(physical_cards(&st), physical_before);
        }
    }
}

/// Forest searches may select nonbasic Forests but never a Mountain or a spell's land back.
#[test]
fn captured_forest_search_enters_two_matching_front_faces_untapped() {
    let cards = deck(&[
        named_row("Skyshroud Claim"),
        row("Forest", "", "Basic Land — Forest", ""),
        row("Forest dual", "", "Land — Forest Swamp", ""),
        row("Mountain", "", "Basic Land — Mountain", ""),
        row(
            "Spell // Forest back",
            "{9} // ",
            "Sorcery // Land — Forest",
            "",
        ),
        row("Spell", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![1, 2, 3, 4, 5]);
    let before = physical_cards(&st);
    let mut pool = ManaPool {
        flexible: 4,
        ..Default::default()
    };

    cast(&cards, &mut st, &mut pool);

    let found: Vec<_> = st
        .battlefield
        .iter()
        .map(|permanent| permanent.card)
        .collect();
    assert_eq!(found.len(), 2);
    assert!(found.contains(&CardRef::Deck(CardIdx(1))));
    assert!(found.contains(&CardRef::Deck(CardIdx(2))));
    assert!(st.battlefield.iter().all(|permanent| !permanent.tapped));
    assert!(st.hand.is_empty());
    assert!(st.library.contains(&CardIdx(3)));
    assert!(st.library.contains(&CardIdx(4)));
    assert_eq!(physical_cards(&st), before);
}

/// Card names do not grant the search rule or change its printed filter.
#[test]
fn renamed_captured_searches_resolve_from_their_oracle_text() {
    for name in ["Cultivate", "Kodama's Reach", "Skyshroud Claim"] {
        let mut source = named_row(name);
        source.name = "Renamed land search".into();
        let cards = deck(&[
            source,
            row("Forest A", "", "Basic Land — Forest", ""),
            row("Forest B", "", "Basic Land — Forest", ""),
        ]);
        let mut st = state(Vec::new(), vec![1, 2]);
        let before = physical_cards(&st);
        let effect = &cards.cards[0].spell_data.effects[0];

        apply_effect_at(&cards, effect, &mut st, 1, false, None);

        assert_eq!(st.battlefield.len() + st.hand.len(), 2, "{name}");
        assert!(st.library.is_empty());
        assert_eq!(physical_cards(&st), before);
    }
}

/// A changed printed count controls both the total results and each destination's capacity.
#[test]
fn fixed_three_result_search_honors_each_destination_capacity() {
    let mut source = named_row("Cultivate");
    source.oracle_text = source
        .oracle_text
        .replace("up to two", "three")
        .replace("put one", "put two");
    let cards = deck(&[
        source,
        row("Basic A", "", "Basic Land — Forest", ""),
        row("Basic B", "", "Basic Land — Island", ""),
        row("Basic C", "", "Basic Land — Swamp", ""),
        row("Basic D", "", "Basic Land — Plains", ""),
    ]);
    let mut st = state(Vec::new(), vec![1, 2, 3, 4]);
    let before = physical_cards(&st);
    let spec = compiled_search(&cards);

    assert_eq!((spec.min_cards, spec.max_cards), (3, 3));
    apply_effect_at(
        &cards,
        &SimEffect::MultiSearch(spec.clone()),
        &mut st,
        1,
        false,
        None,
    );

    assert_eq!(st.battlefield.len(), 2);
    assert_eq!(st.hand.len(), 1);
    assert_eq!(st.library.len(), 1);
    assert_eq!(physical_cards(&st), before);
}

/// A fixed hidden-zone search still performs the possible part when fewer matches exist.
#[test]
fn fixed_result_search_can_find_fewer_cards_than_its_printed_count() {
    let mut source = named_row("Skyshroud Claim");
    source.oracle_text = source.oracle_text.replace("up to two", "two");
    let cards = deck(&[
        source,
        row("Forest", "", "Basic Land — Forest", ""),
        row("Mountain", "", "Basic Land — Mountain", ""),
    ]);
    let mut st = state(Vec::new(), vec![1, 2]);

    assert_eq!(compiled_search(&cards).min_cards, 2);
    apply_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        None,
    );

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(st.library, [CardIdx(2)]);
}

/// Forced tapped entry never spends optional shock-land life, even for a nonbasic result.
#[test]
fn forced_tapped_search_uses_the_common_entry_life_rule() {
    let mut source = named_row("Skyshroud Claim");
    source.oracle_text = source
        .oracle_text
        .replace("onto the battlefield", "onto the battlefield tapped");
    let cards = deck(&[
        source,
        row(
            "Shock Forest",
            "",
            "Land — Forest Island",
            "As this land enters, you may pay 2 life. If you don't, it enters tapped.",
        ),
    ]);
    let mut st = state(Vec::new(), vec![1]);
    assert_eq!(cards.cards[1].life_to_untap, 2);

    apply_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        None,
    );

    assert!(st.battlefield[0].tapped);
    assert_eq!(st.life, 20);
    assert_eq!(st.life_paid, 0);
}

/// The card's own tapped-entry rule applies when the search does not force tapped entry.
#[test]
fn untapped_search_destination_keeps_the_found_cards_printed_entry_rule() {
    let cards = deck(&[
        named_row("Skyshroud Claim"),
        row(
            "Tapped Forest",
            "",
            "Land — Forest",
            "This land enters tapped.",
        ),
        row("Untapped Forest", "", "Land — Forest", ""),
    ]);
    let mut st = state(Vec::new(), vec![1, 2]);

    apply_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        None,
    );

    let tapped = st
        .battlefield
        .iter()
        .find(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
        .expect("the tapped Forest enters");
    let untapped = st
        .battlefield
        .iter()
        .find(|permanent| permanent.card == CardRef::Deck(CardIdx(2)))
        .expect("the untapped Forest enters");
    assert!(tapped.tapped);
    assert!(!untapped.tapped);
}

/// All simultaneously entering lands observe the full entry batch.
#[test]
fn simultaneous_results_capture_each_others_land_entry_events() {
    let cards = deck(&[
        named_row("Skyshroud Claim"),
        row(
            "Forest observer A",
            "",
            "Land Creature — Forest Dryad",
            "Whenever a land you control enters, draw a card.",
        ),
        row(
            "Forest observer B",
            "",
            "Land Creature — Forest Dryad",
            "Whenever a land you control enters, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
        row("C", "{9}", "Sorcery", ""),
        row("D", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![3, 4, 5, 6, 1, 2]);
    let before = physical_cards(&st);

    apply_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        None,
    );

    assert_eq!(st.battlefield.len(), 2);
    assert_eq!(st.hand.len(), 4);
    assert!(st.library.is_empty());
    assert!(!st.failed_draw);
    assert_eq!(physical_cards(&st), before);
}

/// An entry trigger cannot draw before the search's later hand placement finishes.
#[test]
fn split_search_finishes_placement_and_shuffle_before_entry_triggers_resolve() {
    let cards = deck(&[
        named_row("Cultivate"),
        row("Plains", "", "Basic Land — Plains", ""),
        row(
            "Forest with entry",
            "",
            "Basic Land — Forest",
            "When this land enters, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
        row("C", "{9}", "Sorcery", ""),
        row("D", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![3, 4, 5, 6, 1, 2]);
    let before = physical_cards(&st);
    let mut expected_rng = st.library_rng.clone();
    let mut expected_library = vec![CardIdx(3), CardIdx(4), CardIdx(5), CardIdx(6)];
    shuffle(&mut expected_library, &mut expected_rng);
    let expected_draw = expected_library
        .pop()
        .expect("four cards remain after the search");

    apply_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        None,
    );

    assert_eq!(st.hand, [CardIdx(1), expected_draw]);
    assert_eq!(st.library, expected_library);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
    assert_eq!(physical_cards(&st), before);
}

/// A search without shuffle neither changes the remaining order nor advances library randomness.
#[test]
fn multiple_result_search_without_shuffle_preserves_remaining_order_and_rng() {
    let mut source = named_row("Skyshroud Claim");
    source.oracle_text = source.oracle_text.replace(", then shuffle", "");
    let cards = deck(&[
        source,
        row("Forest A", "", "Land — Forest", ""),
        row("Spell A", "{9}", "Sorcery", ""),
        row("Forest B", "", "Land — Forest", ""),
        row("Spell B", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![1, 2, 3, 4]);
    let mut expected_rng = st.library_rng.clone();

    assert!(!compiled_search(&cards).shuffle);
    apply_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        None,
    );

    assert_eq!(st.library, [CardIdx(2), CardIdx(4)]);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
}

/// Found cards return to the library before a printed trailing shuffle, not after it.
#[test]
fn top_search_shuffles_its_found_cards_with_the_rest_of_the_library() {
    for renamed in [false, true] {
        for shuffle_after in [false, true] {
            let mut source = named_row("Skyshroud Claim");
            source.oracle_text = source
                .oracle_text
                .replace("onto the battlefield", "on top of your library");
            if !shuffle_after {
                source.oracle_text = source.oracle_text.replace(", then shuffle", "");
            }
            if renamed {
                source.name = "Renamed top search".into();
            }
            assert_top_search_order(source, shuffle_after);
        }
    }
}

/// Exercise parsing, lowering, paid casting, and the exact library random stream together.
fn assert_top_search_order(source: CardRow, shuffle_after: bool) {
    let cards = deck(&[
        source,
        row("Forest A", "", "Land — Forest", ""),
        row("Forest B", "", "Land — Forest", ""),
        row("A", "{99}", "Sorcery", ""),
        row("B", "{99}", "Sorcery", ""),
        row("C", "{99}", "Sorcery", ""),
        row("D", "{99}", "Sorcery", ""),
        row("E", "{99}", "Sorcery", ""),
        row("F", "{99}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![3, 4, 5, 6, 7, 8, 1, 2]);
    let before = physical_cards(&st);
    let mut expected_rng = st.library_rng.clone();
    let mut expected_library = vec![
        CardIdx(3),
        CardIdx(4),
        CardIdx(5),
        CardIdx(6),
        CardIdx(7),
        CardIdx(8),
        CardIdx(2),
        CardIdx(1),
    ];
    if shuffle_after {
        shuffle(&mut expected_library, &mut expected_rng);
    }
    let spec = compiled_search(&cards);
    assert_eq!(
        spec.destinations[0].destination,
        SearchDestination::LibraryTop
    );
    assert_eq!(spec.shuffle, shuffle_after);
    let mut pool = ManaPool {
        flexible: 4,
        ..Default::default()
    };

    assert_eq!(cast(&cards, &mut st, &mut pool), [4.0]);

    assert_eq!(st.library, expected_library);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert!(st.hand.is_empty() && st.battlefield.is_empty());
    assert_eq!(physical_cards(&st), before);
}

/// Mutate a captured rule into one compound placement with mixed battlefield destinations.
fn mixed_battlefield_source(forced_first: bool, renamed: bool) -> CardRow {
    let mut source = named_row("Cultivate");
    let placement = if forced_first {
        "put one onto the battlefield tapped and the other onto the battlefield"
    } else {
        "put one onto the battlefield and the other onto the battlefield tapped"
    };
    source.oracle_text = source
        .oracle_text
        .replace("basic land cards", "land cards")
        .replace(
            "put one onto the battlefield tapped and the other into your hand",
            placement,
        );
    if renamed {
        source.name = "Renamed mixed entry search".into();
    }
    source
}

/// Both mixed-destination observers see both entries, regardless of which slot forces tapped entry.
#[test]
fn mixed_battlefield_destinations_share_one_entry_event() {
    for renamed in [false, true] {
        for forced_first in [false, true] {
            let cards = deck(&[
                mixed_battlefield_source(forced_first, renamed),
                row(
                    "Forest observer A",
                    "",
                    "Land Creature — Forest Dryad",
                    "Whenever a land you control enters, draw a card.",
                ),
                row(
                    "Forest observer B",
                    "",
                    "Land Creature — Forest Dryad",
                    "Whenever a land you control enters, draw a card.",
                ),
                row("A", "{99}", "Sorcery", ""),
                row("B", "{99}", "Sorcery", ""),
                row("C", "{99}", "Sorcery", ""),
                row("D", "{99}", "Sorcery", ""),
            ]);
            let mut st = state(vec![0], vec![3, 4, 5, 6, 1, 2]);
            let before = physical_cards(&st);
            let mut pool = ManaPool {
                flexible: 3,
                ..Default::default()
            };

            assert_eq!(cast(&cards, &mut st, &mut pool), [3.0]);

            assert_eq!(st.battlefield.len(), 2);
            assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(2)));
            assert_eq!(st.battlefield[1].card, CardRef::Deck(CardIdx(1)));
            assert_eq!(st.battlefield[0].tapped, forced_first);
            assert_eq!(st.battlefield[1].tapped, !forced_first);
            assert_eq!(st.hand.len(), 4);
            assert!(st.library.is_empty());
            assert!(!st.failed_draw && st.completion.is_none());
            assert_eq!(physical_cards(&st), before);
        }
    }
}

/// A check land cannot use a Forest that enters in the same compound placement.
#[test]
fn mixed_battlefield_entry_checks_only_the_pre_entry_board() {
    for renamed in [false, true] {
        for existing_forest in [false, true] {
            let cards = deck(&[
                mixed_battlefield_source(true, renamed),
                row(
                    "Check land",
                    "",
                    "Land",
                    "This land enters tapped unless you control a Forest or an Island.\n{T}: Add {G} or {U}.",
                ),
                row("New Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
                row(
                    "Existing Forest",
                    "",
                    "Basic Land — Forest",
                    "{T}: Add {G}.",
                ),
            ]);
            assert!(matches!(cards.cards[1].untapped_entry_condition,
                Some(crate::deck::simulator::model::LandEntryCondition::ControlsLandTypes(types))
                if types == [false, true, false, false, true]));
            let mut st = state(vec![0], vec![1, 2]);
            if existing_forest {
                st.battlefield
                    .push(crate::deck::simulator::game::new_perm_with(
                        100,
                        &cards,
                        CardIdx(3),
                        0,
                        false,
                    ));
                st.next_uid = 101;
            }
            let before = physical_cards(&st);
            let mut pool = ManaPool {
                flexible: 3,
                ..Default::default()
            };

            assert_eq!(cast(&cards, &mut st, &mut pool), [3.0]);

            let check_land = st
                .battlefield
                .iter()
                .find(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
                .expect("the check land enters");
            let new_forest = st
                .battlefield
                .iter()
                .find(|permanent| permanent.card == CardRef::Deck(CardIdx(2)))
                .expect("the searched Forest enters");
            assert_eq!(check_land.tapped, !existing_forest);
            assert!(new_forest.tapped);
            assert_eq!(st.life_paid, 0);
            assert_eq!(physical_cards(&st), before);
        }
    }
}
