use super::*;
use crate::deck::simulator::game::{CardRef, TokenKind, new_perm_with, new_token_perm};
use crate::deck::simulator::model::{Equipment, Format, SimCard};
use crate::deck::simulator::scenario_test_support::state;

/// Build runtime characteristics without passing the expected rule through the parser.
fn typed_card(card_types: &[&str], subtypes: &[&str]) -> SimCard {
    SimCard {
        card_types: card_types.iter().map(|kind| (*kind).to_string()).collect(),
        subtypes: subtypes.iter().map(|kind| (*kind).to_string()).collect(),
        is_creature: card_types.contains(&"creature"),
        is_land: card_types.contains(&"land"),
        is_artifact: card_types.contains(&"artifact"),
        is_planeswalker: card_types.contains(&"planeswalker"),
        is_permanent: true,
        ..Default::default()
    }
}

/// Keep main-deck and command-zone card definitions independent in predicate tests.
fn typed_deck(cards: Vec<SimCard>, commanders: Vec<SimCard>) -> SimDeck {
    SimDeck {
        cards,
        commanders,
        companion: None,
        format: Format::Constructed,
        rules: crate::deck::simulator::format::rules_for("constructed"),
    }
}

/// Build complete runtime qualities without using the Oracle quality parser.
fn quality(card_types: &[&str], subtypes: &[&str]) -> BeholdSpec {
    BeholdSpec {
        card_types: card_types.iter().map(|kind| (*kind).to_string()).collect(),
        subtypes: subtypes.iter().map(|kind| (*kind).to_string()).collect(),
        count: 1,
    }
}

/// Hand reveals use the front-face metadata and do not consume the revealed card.
#[test]
fn hand_reveal_matches_only_the_complete_quality() {
    let deck = typed_deck(
        vec![
            typed_card(&["creature"], &["dragon"]),
            typed_card(&["kindred", "enchantment"], &["dragon"]),
            typed_card(&["creature"], &["gamma"]),
        ],
        Vec::new(),
    );
    let mut st = state(vec![1], Vec::new());
    assert!(available(&deck, &st, &quality(&[], &["dragon"]), None));
    assert!(!available(
        &deck,
        &st,
        &quality(&["creature"], &["dragon"]),
        None
    ));
    st.hand = vec![CardIdx(0)];
    assert!(available(
        &deck,
        &st,
        &quality(&["creature"], &["dragon"]),
        None
    ));
    assert!(!available(
        &deck,
        &st,
        &quality(&["creature"], &["gamma"]),
        None
    ));
    st.hand = vec![CardIdx(2)];
    assert!(available(
        &deck,
        &st,
        &quality(&["creature"], &["gamma"]),
        None
    ));
    assert_eq!(st.hand, [CardIdx(2)]);
}

/// A pending cast cannot pay its own Behold requirement while still in hand.
#[test]
fn pending_cast_and_entering_land_are_excluded_from_hand_choices() {
    let deck = typed_deck(
        vec![
            typed_card(&["creature"], &["elf"]),
            typed_card(&["land"], &[]),
            typed_card(&["creature"], &["elf"]),
        ],
        Vec::new(),
    );
    let mut st = state(vec![0, 1], Vec::new());
    assert!(!available(
        &deck,
        &st,
        &quality(&[], &["elf"]),
        Some(CardIdx(0))
    ));
    assert!(!available(
        &deck,
        &st,
        &quality(&["land"], &[]),
        Some(CardIdx(1))
    ));
    st.hand.push(CardIdx(2));
    assert!(available(
        &deck,
        &st,
        &quality(&[], &["elf"]),
        Some(CardIdx(0))
    ));
    assert_eq!(st.hand.pop(), Some(CardIdx(2)));
    st.battlefield
        .push(new_perm_with(7, &deck, CardIdx(2), 1, true));
    assert!(available(
        &deck,
        &st,
        &quality(&[], &["elf"]),
        Some(CardIdx(0))
    ));
}

/// A tapped, summoning-sick permanent remains a legal nonconsuming Behold choice.
#[test]
fn controlled_permanent_needs_neither_untapped_status_nor_haste() {
    let deck = typed_deck(vec![typed_card(&["creature"], &["elf"])], Vec::new());
    let mut st = state(Vec::new(), Vec::new());
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, true));
    assert!(st.battlefield[0].summoning_sick);
    assert!(available(&deck, &st, &quality(&[], &["elf"]), None));
    assert!(st.battlefield[0].tapped);
    assert!(st.battlefield[0].summoning_sick);
}

/// A cast commander has its real quality, but a commander still in its zone does not.
#[test]
fn only_a_controlled_commander_is_available() {
    let deck = typed_deck(Vec::new(), vec![typed_card(&["creature"], &["dragon"])]);
    let mut st = state(Vec::new(), Vec::new());
    let spec = quality(&["creature"], &["dragon"]);
    assert!(!available(&deck, &st, &spec, None));
    let mut commander = new_token_perm(4, 1);
    commander.card = CardRef::Commander { slot: 0 };
    commander.tapped = true;
    st.battlefield.push(commander);
    assert!(available(&deck, &st, &spec, None));
    assert!(!available(&deck, &st, &quality(&[], &["elf"]), None));
}

/// Library, graveyard, and exile cards do not provide Behold choices.
#[test]
fn other_zones_and_card_names_do_not_match() {
    let mut card = typed_card(&["planeswalker"], &["sarkhan"]);
    card.name = "Jace".into();
    let deck = typed_deck(
        vec![
            card,
            typed_card(&["planeswalker"], &["jace"]),
            typed_card(&["planeswalker"], &["jace"]),
            typed_card(&["planeswalker"], &["jace"]),
        ],
        Vec::new(),
    );
    let mut st = state(vec![0], vec![1]);
    st.graveyard.push(CardIdx(2));
    st.exile.push(CardIdx(3));
    assert!(!available(&deck, &st, &quality(&[], &["jace"]), None));
    st.hand.push(
        st.library
            .pop()
            .expect("the test starts with one library card"),
    );
    assert!(available(
        &deck,
        &st,
        &quality(&["planeswalker"], &["jace"]),
        None
    ));
}

/// Generic token bodies do not acquire an assumed creature subtype.
#[test]
fn generic_tokens_have_only_known_qualities() {
    let deck = typed_deck(Vec::new(), Vec::new());
    let mut st = state(Vec::new(), Vec::new());
    st.battlefield.push(new_token_perm(1, 1));
    assert!(available(&deck, &st, &quality(&["creature"], &[]), None));
    for subtype in ["elf", "dragon", "soldier", "warrior", "jace"] {
        assert!(
            !available(&deck, &st, &quality(&[], &[subtype]), None),
            "{subtype}"
        );
    }
}

/// Explicit token kinds expose their known subtype and card type only.
#[test]
fn known_token_kinds_can_satisfy_their_exact_qualities() {
    let deck = typed_deck(Vec::new(), Vec::new());
    let mut st = state(Vec::new(), Vec::new());
    for (kind, card_type, subtype) in [
        (TokenKind::Spirit, "creature", "spirit"),
        (TokenKind::Army, "creature", "army"),
        (TokenKind::Jace, "planeswalker", "jace"),
    ] {
        let mut token = new_token_perm(1, 1);
        token.card = CardRef::Token(kind);
        st.battlefield = vec![token];
        assert!(available(
            &deck,
            &st,
            &quality(&[card_type], &[subtype]),
            None
        ));
        assert!(!available(&deck, &st, &quality(&[], &["dragon"]), None));
    }
    assert!(!available(
        &deck,
        &st,
        &quality(&["creature"], &["jace"]),
        None
    ));
}

/// Face-down permanents keep the public creature type and lose printed types and subtypes.
#[test]
fn face_down_cards_do_not_expose_printed_qualities() {
    let deck = typed_deck(
        vec![typed_card(&["artifact", "creature"], &["elf"])],
        Vec::new(),
    );
    let mut st = state(Vec::new(), Vec::new());
    let mut permanent = new_perm_with(1, &deck, CardIdx(0), 1, false);
    permanent.face_down = true;
    st.battlefield.push(permanent);
    assert!(available(&deck, &st, &quality(&["creature"], &[]), None));
    assert!(!available(&deck, &st, &quality(&["artifact"], &[]), None));
    assert!(!available(&deck, &st, &quality(&[], &["elf"]), None));
    st.battlefield[0].face_down = false;
    assert!(available(
        &deck,
        &st,
        &quality(&["artifact"], &["elf"]),
        None
    ));
}

/// A spell front and a played land face never expose both faces at once.
#[test]
fn mdfc_land_face_suppresses_the_hidden_spell_characteristics() {
    let mut card = typed_card(&["creature"], &["elf"]);
    card.is_mdfc_spell = true;
    card.is_land = true;
    let deck = typed_deck(vec![card], Vec::new());
    let mut st = state(vec![0], Vec::new());
    assert!(available(
        &deck,
        &st,
        &quality(&["creature"], &["elf"]),
        None
    ));
    assert!(!available(&deck, &st, &quality(&["land"], &[]), None));
    st.hand.clear();
    let mut permanent = new_perm_with(1, &deck, CardIdx(0), 1, false);
    permanent.land_face = true;
    st.battlefield.push(permanent);
    assert!(available(&deck, &st, &quality(&["land"], &[]), None));
    assert!(!available(&deck, &st, &quality(&["creature"], &[]), None));
    assert!(!available(&deck, &st, &quality(&[], &["elf"]), None));
    st.battlefield[0].land_face = false;
    assert!(available(
        &deck,
        &st,
        &quality(&["creature"], &["elf"]),
        None
    ));
}

/// Legacy union flags do not make a hidden creature face revealable from hand.
#[test]
fn hand_qualities_ignore_hidden_mdfc_faces() {
    let mut card = typed_card(&["sorcery"], &[]);
    card.is_mdfc_spell = true;
    card.is_land = true;
    card.is_creature = true;
    let deck = typed_deck(vec![card], Vec::new());
    let st = state(vec![0], Vec::new());
    assert!(!available(&deck, &st, &quality(&["creature"], &[]), None));
    assert!(!available(&deck, &st, &quality(&[], &["elf"]), None));
}

/// An exposed land front keeps its known subtype, but that subtype stays hidden on its other face.
#[test]
fn land_front_qualities_follow_the_exposed_face() {
    let mut card = typed_card(&["land"], &["cave"]);
    card.is_mdfc_spell = true;
    let deck = typed_deck(vec![card], Vec::new());
    let mut st = state(Vec::new(), Vec::new());
    let mut permanent = new_perm_with(1, &deck, CardIdx(0), 1, false);
    permanent.land_face = true;
    st.battlefield.push(permanent);
    assert!(available(&deck, &st, &quality(&["land"], &["cave"]), None));
    st.battlefield[0].land_face = false;
    assert!(!available(&deck, &st, &quality(&[], &["cave"]), None));
}

/// Current creature status includes animation and excludes attached reconfigure gear.
#[test]
fn combined_quality_checks_the_current_creature_type() {
    let vehicle = typed_card(&["artifact"], &["vehicle"]);
    let mut gear = typed_card(&["artifact", "creature"], &["elf"]);
    gear.flags.equipment = Some(Equipment {
        cost: 1,
        buff: (1, 1),
        reconfigure: true,
    });
    let deck = typed_deck(vec![vehicle, gear], Vec::new());
    let mut st = state(Vec::new(), Vec::new());
    let mut permanent = new_perm_with(1, &deck, CardIdx(0), 1, false);
    permanent.animated = true;
    st.battlefield.push(permanent);
    assert!(available(
        &deck,
        &st,
        &quality(&["creature"], &["vehicle"]),
        None
    ));
    st.battlefield = vec![new_perm_with(2, &deck, CardIdx(1), 1, false)];
    st.battlefield[0].equipped = true;
    assert!(!available(
        &deck,
        &st,
        &quality(&["creature"], &["elf"]),
        None
    ));
    assert!(available(
        &deck,
        &st,
        &quality(&["artifact"], &["elf"]),
        None
    ));
}

/// A counted requirement stays unavailable even when several matching objects exist.
#[test]
fn multiple_choices_and_empty_qualities_are_not_singular_fallbacks() {
    let deck = typed_deck(
        vec![
            typed_card(&["creature"], &["elemental"]),
            typed_card(&["creature"], &["elemental"]),
        ],
        Vec::new(),
    );
    let mut st = state(vec![0], Vec::new());
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(1), 1, false));
    for count in [0, 2, 3] {
        let mut spec = quality(&[], &["elemental"]);
        spec.count = count;
        assert!(!available(&deck, &st, &spec, None));
    }
    assert!(!available(&deck, &st, &quality(&[], &[]), None));
}

/// Repeated Behold checks neither consume a candidate nor spend player resources.
#[test]
fn generic_alternative_checks_do_not_consume_any_resources() {
    let deck = typed_deck(
        vec![
            typed_card(&["creature"], &["elf"]),
            typed_card(&["creature"], &["elf"]),
        ],
        Vec::new(),
    );
    let mut st = state(vec![0], vec![1]);
    st.life = 17;
    st.treasure_bank = 3;
    let alternative = BeholdAlternative {
        spec: quality(&[], &["elf"]),
        generic: 2,
    };
    for _ in 0..3 {
        assert_eq!(generic_surcharge(&deck, &st, &alternative, None), 0);
        assert_eq!(
            generic_surcharge(&deck, &st, &alternative, Some(CardIdx(0))),
            2
        );
    }
    assert_eq!(st.hand, [CardIdx(0)]);
    assert_eq!(st.library, [CardIdx(1)]);
    assert!(st.graveyard.is_empty());
    assert!(st.exile.is_empty());
    assert_eq!(st.life, 17);
    assert_eq!(st.life_paid, 0);
    assert_eq!(st.treasure_bank, 3);
}

/// Result identity prevents the compound effect from untapping an unrelated land.
#[test]
fn linked_untap_uses_only_the_search_result_uid() {
    let deck = typed_deck(
        vec![
            typed_card(&["land"], &[]),
            typed_card(&["land"], &[]),
            typed_card(&["creature"], &["elf"]),
        ],
        Vec::new(),
    );
    let mut st = state(vec![2], Vec::new());
    st.battlefield
        .push(new_perm_with(10, &deck, CardIdx(0), 1, true));
    st.battlefield
        .push(new_perm_with(11, &deck, CardIdx(1), 1, true));
    let spec = quality(&[], &["elf"]);
    untap_search_result(&deck, &mut st, &spec, Some(11));
    assert!(st.battlefield[0].tapped);
    assert!(!st.battlefield[1].tapped);
    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.life_paid, 0);
}

/// Failed searches, removed results, and absent qualities never untap another object.
#[test]
fn linked_untap_does_nothing_without_its_result_and_choice() {
    let deck = typed_deck(
        vec![
            typed_card(&["land"], &[]),
            typed_card(&["creature"], &["elf"]),
        ],
        Vec::new(),
    );
    let mut st = state(vec![1], Vec::new());
    st.battlefield
        .push(new_perm_with(10, &deck, CardIdx(0), 1, true));
    let spec = quality(&[], &["elf"]);
    untap_search_result(&deck, &mut st, &spec, None);
    untap_search_result(&deck, &mut st, &spec, Some(11));
    assert!(st.battlefield[0].tapped);
    st.hand.clear();
    untap_search_result(&deck, &mut st, &spec, Some(10));
    assert!(st.battlefield[0].tapped);
}
