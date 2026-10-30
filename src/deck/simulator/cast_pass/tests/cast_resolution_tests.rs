use super::*;
use crate::deck::simulator::game::{CardRef, new_perm_with};
use crate::deck::simulator::model::LibraryGraveyardTrigger;
use crate::deck::simulator::turn_loop_tests::{cast, deck, row, state};

/// X is fixed before affinity reaches the generic-cost floor.
#[test]
fn affinity_applies_after_x_and_counts_only_matching_artifacts() {
    for (mana_cost, affinity, resource_type, generic, x, paid_generic) in [
        ("{X}{G}", true, "Artifact", 0, 3, 0),
        ("{X}{G}", false, "Artifact", 0, 0, 0),
        ("{X}{G}", true, "Creature", 0, 0, 0),
        ("{X}{X}{G}", true, "Artifact", 0, 1, 0),
        ("{X}{X}{G}", true, "Artifact", 1, 2, 1),
        ("{2}{X}{G}", true, "Artifact", 0, 1, 0),
        ("{5}{X}{G}", true, "Artifact", 2, 0, 2),
    ] {
        let cards = deck(&[
            row(
                "Variable spell",
                mana_cost,
                "Sorcery",
                if affinity {
                    "Affinity for artifacts"
                } else {
                    ""
                },
            ),
            row("Resource", "{9}", resource_type, ""),
        ]);
        let mut st = state(vec![0], vec![]);
        for uid in 1..=3 {
            st.battlefield
                .push(new_perm_with(uid, &cards, CardIdx(1), 0, false));
        }
        let pool = ManaPool {
            colorless: generic,
            fixed: [0, 0, 0, 0, 1],
            ..ManaPool::default()
        };

        let (chosen_x, cost) = super::super::payment::choose_x_cost(
            &cards,
            &st,
            &pool,
            CardIdx(0),
            cards.cards[0].min_cost,
        )
        .expect("the fixed part of the spell is payable");

        assert_eq!(
            chosen_x, x,
            "{mana_cost}, affinity={affinity}, {resource_type}"
        );
        assert_eq!(cost.generic, paid_generic);
        assert_eq!(cost.x_symbols, 0);
        assert!(st.battlefield.iter().all(|permanent| !permanent.tapped));
        assert_eq!(pool.total(), generic + 1);
    }
}

/// The hand-cast path passes raw X costs rather than a reduced cost floor.
#[test]
fn affinity_x_cast_uses_the_full_reduction_without_generic_mana() {
    let cards = deck(&[
        row(
            "Affinity draw",
            "{X}{G}",
            "Sorcery",
            "Affinity for artifacts\nDraw X cards.",
        ),
        row("First artifact", "{9}", "Artifact", ""),
        row("Second artifact", "{9}", "Artifact", ""),
        row("Third artifact", "{9}", "Artifact", ""),
        row("First draw", "{9}", "Sorcery", ""),
        row("Second draw", "{9}", "Sorcery", ""),
        row("Third draw", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![4, 5, 6]);
    for index in 1..=3 {
        st.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    st.next_uid = 3;
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 0, 1],
        ..ManaPool::default()
    };

    assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);

    assert_eq!(st.hand, [CardIdx(6), CardIdx(5), CardIdx(4)]);
    assert_eq!(pool.total(), 0);
    assert_eq!(st.graveyard, [CardIdx(0)]);
}

/// A copy's library-return and draw triggers settle before the next copy mills.
#[test]
fn storm_copies_settle_triggers_between_mill_resolutions() {
    for prior_spells in [0, 1] {
        let mut cards = deck(&[
            row("Storm mill", "{0}", "Sorcery", "Storm\nMill a card."),
            row(
                "A",
                "{9}",
                "Creature",
                "When this creature enters, draw a card.",
            ),
            row("B", "{9}", "Sorcery", ""),
            row("C", "{9}", "Sorcery", ""),
        ]);
        cards.cards[1].library_graveyard_trigger =
            Some(LibraryGraveyardTrigger::ReturnToBattlefield);
        let mut st = state(vec![0], vec![3, 2, 1]);
        st.spells_cast_this_turn = prior_spells;

        cast(&cards, &mut st, &mut ManaPool::default());

        assert_eq!(st.hand, [CardIdx(2)], "the first resolution's ETB draws B");
        assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
        assert_eq!(st.milled_self, prior_spells + 1);
        assert_eq!(
            st.library,
            if prior_spells == 0 {
                vec![CardIdx(3)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            st.graveyard
                .iter()
                .filter(|index| **index == CardIdx(0))
                .count(),
            1
        );
        assert_eq!(st.graveyard.contains(&CardIdx(3)), prior_spells == 1);
    }
}

/// The physical original reaches the graveyard only at its own resolution boundary.
#[test]
fn original_spell_destination_is_not_applied_to_a_storm_copy() {
    for prior_spells in [0, 1] {
        let mut cards = deck(&[
            row("Storm mill", "{0}", "Sorcery", "Storm\nMill a card."),
            row(
                "Returned creature",
                "{9}",
                "Creature",
                "When this creature enters, return target card from your graveyard to your hand.",
            ),
            row("Next mill", "{9}", "Sorcery", ""),
        ]);
        cards.cards[1].library_graveyard_trigger =
            Some(LibraryGraveyardTrigger::ReturnToBattlefield);
        let mut st = state(vec![0], vec![2, 1]);
        st.spells_cast_this_turn = prior_spells;
        let targets = prepare_spell_targets(&cards, &st, &cards.cards[0])
            .expect("the mill spell has no required targets");

        resolve_cast(
            &cards,
            &mut st,
            &mut ManaPool::default(),
            1,
            CardIdx(0),
            &cards.cards[0].min_cost,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut 0,
            &mut Vec::new(),
            true,
            CastPayment::Mana,
            targets,
        );

        if prior_spells == 0 {
            assert_eq!(
                st.hand,
                [CardIdx(0)],
                "the original is a legal target after it resolves"
            );
            assert!(st.graveyard.is_empty());
        } else {
            assert!(
                st.hand.is_empty(),
                "the copy cannot put the original in the graveyard early"
            );
            assert_eq!(st.graveyard, [CardIdx(2), CardIdx(0)]);
        }
    }
}

/// Every storm copy retains the original target identities and rechecks legality.
#[test]
fn later_storm_resolution_fizzles_after_the_copy_returns_its_only_target() {
    let cards = deck(&[
        row(
            "Storm return",
            "{0}",
            "Sorcery",
            "Storm\nReturn target creature card from your graveyard to the battlefield. Gain three life.",
        ),
        row("Target", "{9}", "Creature", ""),
    ]);
    let mut st = state(vec![0], vec![]);
    st.graveyard.push(CardIdx(1));
    st.spells_cast_this_turn = 1;

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(
        st.life_gained, 3,
        "the original's illegal target stops its life gain"
    );
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.graveyard, [CardIdx(0)]);
}

/// Spending the sole target as delve stops all resolution riders, not only the return.
#[test]
fn all_illegal_targets_skip_effects_and_linked_flashback_permission() {
    let mut cards = deck(&[
        row(
            "Delve return",
            "{1}",
            "Sorcery",
            "Delve\nReturn target creature card from your graveyard to the battlefield. Gain three life.",
        ),
        row("Target", "{9}", "Creature", ""),
        row("Other graveyard spell", "{9}", "Instant", ""),
    ]);
    cards.cards[0].spell_data.grants_flashback = true;
    let mut st = state(vec![0], vec![]);
    st.graveyard = vec![CardIdx(2), CardIdx(1)];

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(st.life_gained, 0);
    assert!(st.flashback_permissions.is_empty());
    assert!(st.battlefield.is_empty());
    assert_eq!(st.exile, [CardIdx(1)]);
    assert_eq!(st.graveyard, [CardIdx(2), CardIdx(0)]);
}

/// One remaining legal target lets the spell resolve without replacing the illegal target.
#[test]
fn partial_target_legality_preserves_the_remaining_effects() {
    let mut cards = deck(&[
        row("Partial return", "{1}", "Sorcery", "Delve"),
        row("First target", "{9}", "Creature", ""),
        row("Second target", "{9}", "Creature", ""),
    ]);
    cards.cards[0].spell_data.effects = vec![
        SimEffect::ReturnFromGraveyard {
            to_hand: false,
            count: 2,
        },
        SimEffect::GainLife(3),
    ];
    let mut st = state(vec![0], vec![]);
    st.graveyard = vec![CardIdx(1), CardIdx(2)];

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(st.life_gained, 3);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(st.exile, [CardIdx(2)]);
    assert_eq!(st.graveyard, [CardIdx(0)]);
}

/// A watcher in a simultaneous reveal batch sees creatures revealed before it.
#[test]
fn revealed_permanents_capture_enters_triggers_after_the_full_batch_enters() {
    for (watcher_cost, expected_gain) in [("{0}", 1), ("{3}", 0)] {
        let cards = deck(&[
            row(
                "Wave",
                "{X}{G}",
                "Sorcery",
                "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
            ),
            row("Creature", "{0}", "Creature", ""),
            row(
                "Watcher",
                watcher_cost,
                "Enchantment",
                "Whenever a creature enters under your control, gain one life.",
            ),
        ]);
        let mut st = state(vec![0], vec![2, 1]);
        let mut pool = ManaPool {
            colorless: 2,
            fixed: [0, 0, 0, 0, 1],
            ..ManaPool::default()
        };

        cast(&cards, &mut st, &mut pool);

        assert_eq!(st.life_gained, expected_gain);
        assert_eq!(st.battlefield.len(), if expected_gain == 1 { 2 } else { 1 });
        assert!(st.library.is_empty());
    }
}

/// A paid X above eight reveals the full X-card batch.
#[test]
fn reveal_permanents_does_not_cap_paid_x_at_eight() {
    let mut rows = vec![row(
        "Wave",
        "{X}{G}",
        "Sorcery",
        "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
    )];
    rows.extend((0..9).map(|_| row("Artifact", "{0}", "Artifact", "")));
    let cards = deck(&rows);
    let mut st = state(vec![0], (1..=9).collect());
    let mut pool = ManaPool {
        colorless: 9,
        fixed: [0, 0, 0, 0, 1],
        ..ManaPool::default()
    };

    cast(&cards, &mut st, &mut pool);

    assert_eq!(st.battlefield.len(), 9);
    assert_eq!(st.seen, 9);
    assert!(st.library.is_empty());
    assert_eq!(st.graveyard, [CardIdx(0)]);
}
