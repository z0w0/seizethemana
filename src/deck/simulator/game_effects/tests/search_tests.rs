use super::*;
use crate::deck::simulator::turn_loop_tests::{cast, deck, row, state};
use rand::RngCore;

/// A Forest search cannot take a Mountain merely because it is a land.
#[test]
fn forest_search_keeps_nonmatching_lands_in_library() {
    let cards = deck(&[
        row(
            "Forest search",
            "{1}{G}",
            "Sorcery",
            "Search your library for a Forest card, put that card onto the battlefield, then shuffle.",
        ),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Mountain", "", "Basic Land — Mountain", "{T}: Add {R}."),
    ]);
    let mut st = state(vec![0], vec![1, 2]);
    let mut pool = ManaPool {
        flexible: 2,
        ..Default::default()
    };
    cast(&cards, &mut st, &mut pool);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(
        st.battlefield[0].card,
        super::super::game::CardRef::Deck(CardIdx(1))
    );
    assert_eq!(st.library, [CardIdx(2)]);
}

/// A basic-only search rejects a nonbasic Forest and a spell's land back face.
#[test]
fn land_filters_use_the_library_face_and_basic_supertype() {
    let cards = deck(&[
        row("Forest", "", "Basic Land — Forest", ""),
        row("Dual", "", "Land — Forest Swamp", ""),
        row(
            "Spell // Forest back",
            "{1} // ",
            "Sorcery // Land — Forest",
            "Draw a card. // {T}: Add {G}.",
        ),
    ]);
    let spec = SearchSpec {
        basic_land_only: true,
        land_types: [false, false, false, false, true],
        ..Default::default()
    };
    assert!(search_matches(&cards, CardIdx(0), &spec));
    assert!(!search_matches(&cards, CardIdx(1), &spec));
    assert!(!search_matches(&cards, CardIdx(2), &spec));
}

/// A top-of-library tutor shuffles the rest before placing its selected card.
#[test]
fn library_top_search_places_its_result_after_the_shuffle() {
    let cards = deck(&[
        row("Creature", "{1}", "Creature", ""),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
        row("C", "{9}", "Sorcery", ""),
        row("D", "{9}", "Sorcery", ""),
        row("E", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![0, 1, 2, 3, 4, 5]);
    let mut expected_rng = st.library_rng.clone();
    let mut expected = vec![CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4), CardIdx(5)];
    crate::deck::simulator::rng::shuffle(&mut expected, &mut expected_rng);
    expected.push(CardIdx(0));
    let spec = SearchSpec {
        card_type: Some(SearchCardType::Creature),
        destination: SearchDestination::LibraryTop,
        shuffle: true,
        ..Default::default()
    };

    apply_effect_at(&cards, &SimEffect::Search(spec), &mut st, 1, false, None);

    assert_eq!(st.library, expected);
    assert_eq!(st.library.last(), Some(&CardIdx(0)));
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
}

/// A search without an Oracle shuffle preserves library order and random state.
#[test]
fn search_without_shuffle_does_not_consume_the_library_stream() {
    let cards = deck(&[
        row("Creature", "{1}", "Creature", ""),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![0, 1, 2]);
    let mut expected_rng = st.library_rng.clone();
    let spec = SearchSpec {
        card_type: Some(SearchCardType::Creature),
        ..Default::default()
    };

    apply_effect_at(&cards, &SimEffect::Search(spec), &mut st, 1, false, None);

    assert_eq!(st.library, [CardIdx(1), CardIdx(2)]);
    assert_eq!(st.hand, [CardIdx(0)]);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
}

/// A committed search still shuffles when its controller finds no legal card.
#[test]
fn committed_search_with_no_match_still_obeys_the_shuffle_instruction() {
    let cards = deck(&[
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
        row("C", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![0, 1, 2]);
    let mut expected_rng = st.library_rng.clone();
    let mut expected = st.library.clone();
    crate::deck::simulator::rng::shuffle(&mut expected, &mut expected_rng);
    let spec = SearchSpec {
        card_type: Some(SearchCardType::Creature),
        shuffle: true,
        optional: true,
        ..Default::default()
    };

    apply_effect_at(&cards, &SimEffect::Search(spec), &mut st, 1, false, None);

    assert_eq!(st.library, expected);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
    assert!(st.hand.is_empty());
}

/// The compiled explicit shuffle instruction reaches committed search execution.
#[test]
fn oracle_search_carries_its_explicit_shuffle_flag() {
    let cards = deck(&[row(
        "Tutor",
        "{1}",
        "Sorcery",
        "Search your library for a creature card, then shuffle and put that card on top of your library.",
    )]);
    let SimEffect::Search(spec) = &cards.cards[0].spell_data.effects[0] else {
        panic!("supported search lowers to a typed search effect");
    };
    assert!(spec.shuffle);
}

/// Either the search instruction or the card can require tapped entry.
#[test]
fn battlefield_search_preserves_printed_and_instruction_tapped_entry() {
    for (text, destination, tapped) in [
        ("", SearchDestination::Battlefield, false),
        (
            "This artifact enters tapped.",
            SearchDestination::Battlefield,
            true,
        ),
        ("", SearchDestination::BattlefieldTapped, true),
    ] {
        let cards = deck(&[row("Artifact", "{1}", "Artifact", text)]);
        let mut st = state(Vec::new(), vec![0]);
        let spec = SearchSpec {
            destination,
            ..Default::default()
        };

        apply_effect_at(&cards, &SimEffect::Search(spec), &mut st, 1, false, None);

        assert_eq!(st.battlefield[0].tapped, tapped);
        assert!(st.library.is_empty());
    }
}
