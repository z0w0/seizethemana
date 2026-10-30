use super::*;
use crate::deck::simulator::game::{CardRef, new_token_perm};
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::turn_loop_tests::{cast, deck, row, state};

/// Entry draws affect the next cast choice, rather than a later sweep.
#[test]
fn entry_draws_settle_before_the_next_cast_choice() {
    let cards = deck(&[
        row(
            "Entry draw",
            "{0}",
            "Enchantment",
            "When this permanent enters, draw a card.",
        ),
        row("Follow-up", "{U}", "Sorcery", ""),
        row("Free ritual", "{0}", "Instant", "Add {U}."),
    ]);
    let mut st = state(vec![0, 1], vec![2]);
    let mut pool = ManaPool::default();

    assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);
    assert!(st.hand.is_empty());
    assert_eq!(st.graveyard, [CardIdx(2), CardIdx(1)]);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(st.spells_cast_this_turn, 3);
}

/// A permanent's cast listener is not on the battlefield during its own cast.
#[test]
fn cast_draw_source_does_not_trigger_for_its_own_cast() {
    let cards = deck(&[
        row(
            "Cast draw",
            "{0}",
            "Enchantment",
            "Whenever you cast a spell, draw a card.",
        ),
        row("Undrawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![1]);

    cast(&cards, &mut st, &mut ManaPool::default());

    assert!(st.hand.is_empty());
    assert_eq!(st.library, [CardIdx(1)]);
    assert_eq!(st.seen, 0);
}

/// A cast draw resolves above the spell that discards the hand.
#[test]
fn cast_trigger_resolves_before_the_causing_wheel() {
    let mut rows = vec![
        row(
            "Cast draw",
            "{0}",
            "Enchantment",
            "Whenever you cast a spell, draw a card.",
        ),
        row(
            "Wheel",
            "{0}",
            "Sorcery",
            "Each player discards their hand, then draws seven cards.",
        ),
    ];
    rows.extend((0..8).map(|_| row("Filler", "{9}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut st = state(vec![1], (2..10).collect());
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.next_uid = 1;

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(st.graveyard, [CardIdx(9), CardIdx(1)]);
    assert_eq!(st.hand, (2..9).rev().map(CardIdx).collect::<Vec<_>>());
    assert!(st.library.is_empty());
}

/// Every effect in a cast trigger resolves once and in Oracle order.
#[test]
fn cast_trigger_resolves_its_complete_effect_sequence() {
    let cards = deck(&[
        row(
            "Cast draw",
            "{0}",
            "Enchantment",
            "Whenever you cast a spell, draw a card and gain two life.",
        ),
        row("Spell", "{0}", "Sorcery", ""),
        row("Drawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![1], vec![2]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.next_uid = 1;

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.life, 22);
    assert_eq!(st.life_gained, 2);
}

/// A resolved spell is available to delve before another spell is chosen.
#[test]
fn resolved_spell_reaches_the_graveyard_before_the_next_cast() {
    let cards = deck(&[
        row("Free spell", "{0}", "Sorcery", ""),
        row("Delve spell", "{1}", "Sorcery", "Delve"),
    ]);
    let mut st = state(vec![0, 1], vec![]);

    cast(&cards, &mut st, &mut ManaPool::default());

    assert!(st.hand.is_empty());
    assert_eq!(st.exile, [CardIdx(0)]);
    assert_eq!(st.graveyard, [CardIdx(1)]);
}

/// Entry counter assignment must retain a planeswalker's printed loyalty.
#[test]
fn cast_planeswalker_keeps_its_starting_loyalty() {
    let mut walker = row(
        "Walker",
        "{0}",
        "Planeswalker",
        "+1: Create a 2/2 creature token.",
    );
    walker.loyalty = Some("4".into());
    let cards = deck(&[walker]);
    let mut st = state(vec![0], vec![]);

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].counters.loyalty, 4);
}

/// Flashback pays Phyrexian life once, then exiles the resolving spell once.
#[test]
fn flashback_pays_phyrexian_life_once() {
    let mut cards = deck(&[row("Flashback spell", "{U}", "Instant", "")]);
    cards.cards[0].spell_data.own_flashback = Some(parse_cost("{U/P}"));
    let mut st = state(vec![], vec![]);
    st.graveyard.push(CardIdx(0));

    cast_graveyard_spells(
        &cards,
        &mut st,
        &mut ManaPool::default(),
        1,
        &mut [0.0],
        &mut Vec::new(),
    );

    assert_eq!(st.life, 18);
    assert_eq!(st.life_paid, 2);
    assert_eq!(st.exile, [CardIdx(0)]);
    assert!(st.graveyard.is_empty());
}

/// Escape does not apply flashback's exile replacement to an instant.
#[test]
fn escaped_instant_returns_to_the_graveyard() {
    let mut cards = deck(&[
        row("Escape spell", "{U}", "Instant", ""),
        row("Fodder one", "{9}", "Sorcery", ""),
        row("Fodder two", "{9}", "Sorcery", ""),
        row("Fodder three", "{9}", "Sorcery", ""),
    ]);
    cards.cards[0].spell_data.own_escape = Some(parse_cost("{1}"));
    let mut st = state(vec![], vec![]);
    st.graveyard = (0..4).map(CardIdx).collect();
    let mut pool = ManaPool {
        colorless: 1,
        ..ManaPool::default()
    };

    cast_graveyard_spells(&cards, &mut st, &mut pool, 1, &mut [0.0], &mut Vec::new());

    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.exile, [CardIdx(1), CardIdx(2), CardIdx(3)]);
    assert_eq!(st.replay_casts, 1);
}

/// A tapped token is a legal creature sacrifice for an additional cast cost.
#[test]
fn additional_sacrifice_consumes_a_tapped_creature_token() {
    let cards = deck(&[row(
        "Sacrifice spell",
        "{0}",
        "Sorcery",
        "As an additional cost to cast this spell, sacrifice a creature.",
    )]);
    let mut st = state(vec![0], vec![]);
    let mut token = new_token_perm(1, 0);
    token.tapped = true;
    st.battlefield.push(token);
    st.next_uid = 1;

    cast(&cards, &mut st, &mut ManaPool::default());

    assert!(st.battlefield.is_empty());
    assert!(st.hand.is_empty());
    assert_eq!(st.graveyard, [CardIdx(0)]);
}

/// A returned exchange card leaves exile, and tapped creatures are sacrificed.
#[test]
fn graveyard_exchange_preserves_one_zone_per_card() {
    let cards = deck(&[
        row(
            "Exchange",
            "{0}",
            "Sorcery",
            "Each player exiles all creature cards from their graveyard, then sacrifices all creatures they control, then puts all cards they exiled this way onto the battlefield.",
        ),
        row("Returned", "{9}", "Creature", ""),
        row("Sacrificed", "{9}", "Creature", ""),
    ]);
    let mut st = state(vec![0], vec![]);
    st.graveyard.push(CardIdx(1));
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(2), 0, true));
    st.next_uid = 1;

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(st.graveyard, [CardIdx(2), CardIdx(0)]);
    assert!(st.exile.is_empty());
}

/// A subtype cast filter must reject a near-matching creature spell.
#[test]
fn cast_filter_accepts_elf_and_rejects_goblin() {
    let cards = deck(&[
        row(
            "Elf draw",
            "{0}",
            "Enchantment",
            "Whenever you cast an Elf spell, draw a card.",
        ),
        row("Elf", "{0}", "Creature — Elf", ""),
        row("Goblin", "{0}", "Creature — Goblin", ""),
        row("Drawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![1, 2], vec![3]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.next_uid = 1;

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(st.seen, 1);
    assert_eq!(st.hand, [CardIdx(3)]);
}

/// An optional trigger payment must be paid before its reward resolves.
#[test]
fn cast_trigger_mana_payment_is_required() {
    let mut cards = deck(&[
        row(
            "Cast draw",
            "{0}",
            "Enchantment",
            "Whenever you cast a spell, draw a card.",
        ),
        row("Spell", "{0}", "Sorcery", ""),
        row("Drawn", "{9}", "Sorcery", ""),
    ]);
    cards.cards[0].striations[0].abilities[0].resolution_payment =
        Some(crate::deck::simulator::model::SimResolutionPayment::PayMana(parse_cost("{1}")));
    for available in [0, 1] {
        let mut st = state(vec![1], vec![2]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
        st.next_uid = 1;
        let mut pool = ManaPool {
            colorless: available,
            ..Default::default()
        };

        cast(&cards, &mut st, &mut pool);

        assert_eq!(st.seen, available);
        assert_eq!(pool.total(), 0);
        assert_eq!(st.graveyard, [CardIdx(1)]);
    }
}
