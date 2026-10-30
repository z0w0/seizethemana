use super::mana_modes_tests::with_land;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::game::{GameState, ManaPool, new_perm_with};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_mana::ManaLot;
use crate::deck::simulator::game_run::mana::{build_pool, tap_new_rocks};
use crate::deck::simulator::game_run::{TurnCensus, run_turn};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use crate::deck::simulator::turn_loop_tests::cast;

/// Give a blue-green creature convoke eligibility and an optional green mana ability.
fn convoke_board(cost: &str, dork: bool) -> (SimDeck, GameState) {
    let mut body = row(
        "Blue creature",
        "{G}",
        "Creature",
        if dork { "{T}: Add {G}." } else { "" },
    );
    body.colors = "[\"U\"]".into();
    let cards = deck(&[
        named_row("Adarkar Wastes"),
        body,
        row("Convoke spell", cost, "Sorcery", "Convoke"),
    ]);
    let mut st = with_land(&cards, vec![2]);
    let mut creature = new_perm_with(2, &cards, CardIdx(1), 0, false);
    creature.summoning_sick = false;
    st.battlefield.push(creature);
    st.next_uid = 2;
    (cards, st)
}

/// Existing pool reroutes must not tap an otherwise unneeded mana creature.
#[test]
fn a_free_residual_pool_payment_keeps_the_dork_untapped() {
    let cards = deck(&[
        row("Blue dork", "{U}", "Creature", "{T}: Add {U}."),
        row("Spell", "{W}{U}", "Sorcery", ""),
    ]);
    let mut st = state(vec![1], Vec::new());
    let mut dork = new_perm_with(1, &cards, CardIdx(0), 0, false);
    dork.summoning_sick = false;
    st.battlefield.push(dork);
    st.next_uid = 1;
    let mut pool = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        lots: vec![ManaLot {
            amount: 1,
            types: 3,
            restriction: None,
            cannot_pay_generic: false,
            one_color: false,
        }],
        ..Default::default()
    };
    let before = pool.clone();
    tap_new_rocks(&cards, &mut st, &mut pool, 1);
    assert!(!st.battlefield[0].tapped);
    assert_eq!(pool, before);
    assert_eq!(cast(&cards, &mut st, &mut pool), [2.0]);
    assert!(!st.battlefield[0].tapped);
    assert_eq!(pool.total(), 0);
}

/// An already-payable convoke spell does not need painful colored mana.
#[test]
fn convoke_avoids_unused_painful_mana_in_pool_building_and_full_turns() {
    let (cards, mut st) = convoke_board("{U}", false);
    st.battlefield[1].summoning_sick = true;
    st.battlefield[1].entered_turn = 1;
    let mut pool = build_pool(&cards, &mut st, 1);
    assert_eq!(pool.colorless, 1);
    assert_eq!(st.life, 20);
    assert!(!st.battlefield[1].tapped);
    assert_eq!(cast(&cards, &mut st, &mut pool), [0.0]);
    assert!(st.battlefield[1].tapped);
    assert_eq!(pool.colorless, 1);
    assert_eq!(st.life, 20);
    assert!(st.hand.is_empty());

    let (cards, mut st) = convoke_board("{U}", false);
    let mut census = TurnCensus::new(1, cards.cards.len());
    run_turn(
        &cards,
        &mut st,
        &mut census,
        &CommanderProfile::new(&cards),
        &mut Vec::new(),
        1,
        false,
    );
    assert!(st.hand.is_empty());
    assert_eq!(st.life, 20);
    assert_eq!(st.life_paid, 0);
    assert_eq!(census.mana_spent, [0.0]);
    assert_eq!(census.mana_remaining, [1.0]);
    assert!(st.completion.is_none());
}

/// Partial convoke combines with exactly one necessary painful blue activation.
#[test]
fn partial_convoke_and_painful_mana_form_one_complete_payment() {
    let (cards, mut st) = convoke_board("{U}{U}", false);
    let mut census = TurnCensus::new(1, cards.cards.len());
    run_turn(
        &cards,
        &mut st,
        &mut census,
        &CommanderProfile::new(&cards),
        &mut Vec::new(),
        1,
        false,
    );
    assert!(st.hand.is_empty());
    assert!(st.battlefield.iter().all(|source| source.tapped));
    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 0);
    assert_eq!(census.mana_spent, [1.0]);
    assert_eq!(census.mana_remaining, [0.0]);
    assert!(st.completion.is_none());
}

/// A dork can make mana or convoke, but the same UID cannot do both.
#[test]
fn a_convoke_dork_has_one_shared_physical_payment_slot() {
    for (cost, accepted) in [("{G}{U}", true), ("{G}{U}{U}", false)] {
        let (cards, mut st) = convoke_board(cost, true);
        assert_eq!(cards.cards[1].colors, [false, true, false, false, true]);
        let mut pool = build_pool(&cards, &mut st, 1);
        assert_eq!(st.life, if accepted { 19 } else { 20 }, "{cost}");
        assert!(st.battlefield[0].tapped, "{cost}: one land tap");
        assert!(
            !st.battlefield[1].tapped,
            "{cost}: reserve the body for convoke"
        );
        assert_eq!(pool.total(), 1, "{cost}: no extra dork activation");
        assert_eq!(pool.colorless, u32::from(!accepted));
        assert_eq!(
            pool.fixed[4], 0,
            "{cost}: green is a virtual payment, not produced mana"
        );
        assert_eq!(st.hand, [CardIdx(2)]);
        assert!(st.graveyard.is_empty() && st.exile.is_empty());
        assert_eq!(
            cast(&cards, &mut st, &mut pool),
            [if accepted { 1.0 } else { 0.0 }],
            "{cost}"
        );
        assert_eq!(
            st.battlefield[1].tapped, accepted,
            "{cost}: exactly one convoke tap"
        );
        assert_eq!(
            st.battlefield
                .iter()
                .map(|source| source.uid)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(pool.total(), u32::from(!accepted));
        assert_eq!(
            st.hand,
            if accepted {
                Vec::new()
            } else {
                vec![CardIdx(2)]
            }
        );
        assert_eq!(
            st.graveyard,
            if accepted {
                vec![CardIdx(2)]
            } else {
                Vec::new()
            }
        );
        assert!(st.exile.is_empty() && st.library.is_empty());
        assert_eq!(st.life, if accepted { 19 } else { 20 });
        assert_eq!(st.life_paid, 0);
        assert!(st.completion.is_none());
    }
}

/// Improvise and delve pay only the generic part while painful mana pays missing blue.
#[test]
fn partial_improvise_and_delve_use_the_actual_cast_resource_rules() {
    for mechanic in ["Improvise", "Delve"] {
        let cards = deck(&[
            named_row("Adarkar Wastes"),
            row("Spell", "{1}{U}", "Sorcery", mechanic),
            row("Resource", "{9}", "Artifact", ""),
        ]);
        let mut st = with_land(&cards, vec![1]);
        if mechanic == "Delve" {
            st.graveyard.push(CardIdx(2));
        } else {
            st.battlefield
                .push(new_perm_with(2, &cards, CardIdx(2), 0, false));
            st.next_uid = 2;
        }
        let mut census = TurnCensus::new(1, cards.cards.len());
        run_turn(
            &cards,
            &mut st,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut Vec::new(),
            1,
            false,
        );
        assert!(st.hand.is_empty(), "{mechanic}");
        assert_eq!(st.life, 19);
        assert_eq!(st.life_paid, 0);
        assert_eq!(census.mana_spent, [1.0]);
        assert_eq!(census.mana_remaining, [0.0]);
        assert!(st.completion.is_none());
        if mechanic == "Delve" {
            assert_eq!(st.exile, [CardIdx(2)]);
        } else {
            assert!(st.battlefield[1].tapped);
        }
    }
}

/// Fully available generic cast resources cannot justify a painful-only mana tap.
#[test]
fn already_payable_improvise_and_delve_do_not_float_unused_painful_mana() {
    for mechanic in ["Improvise", "Delve"] {
        let cards = deck(&[
            row(
                "Painful only",
                "",
                "Land",
                "{T}: Add {U}. This land deals 1 damage to you.",
            ),
            row("Spell", "{1}", "Sorcery", mechanic),
            row("Resource", "{9}", "Artifact", ""),
        ]);
        let mut st = with_land(&cards, vec![1]);
        if mechanic == "Delve" {
            st.graveyard.push(CardIdx(2));
        } else {
            st.battlefield
                .push(new_perm_with(2, &cards, CardIdx(2), 0, false));
            st.next_uid = 2;
        }
        let mut census = TurnCensus::new(1, cards.cards.len());
        run_turn(
            &cards,
            &mut st,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut Vec::new(),
            1,
            false,
        );
        assert!(st.hand.is_empty(), "{mechanic}");
        assert!(!st.battlefield[0].tapped);
        assert_eq!(st.life, 20);
        assert_eq!(st.life_paid, 0);
        assert_eq!(census.mana_spent, [0.0]);
        assert_eq!(census.mana_remaining, [0.0]);
        assert!(st.completion.is_none());
    }
}

/// A pitch option still owes its mandatory Behold mana and does not consume cards while planning.
#[test]
fn pitch_targets_include_behold_mana_before_selecting_a_painful_mode() {
    for behold_available in [false, true] {
        let mut pitch = row(
            "Pitch card",
            "{9}",
            if behold_available {
                "Creature — Elf"
            } else {
                "Sorcery"
            },
            "",
        );
        pitch.colors = "[\"U\"]".into();
        let cards = deck(&[
            row(
                "Painful only",
                "",
                "Land",
                "{T}: Add {U}. This land deals 1 damage to you.",
            ),
            row(
                "Pitch spell",
                "{9}{U}",
                "Sorcery",
                "You may exile one blue card from your hand rather than pay this spell's mana cost.\nAs an additional cost to cast this spell, behold an Elf or pay {1}.",
            ),
            pitch,
        ]);
        let mut st = with_land(&cards, vec![1, 2]);
        let before = st.hand.clone();
        let pool = build_pool(&cards, &mut st, 1);
        assert_eq!(st.hand, before);
        assert!(st.exile.is_empty() && st.graveyard.is_empty());
        assert_eq!(st.life, if behold_available { 20 } else { 19 });
        assert_eq!(pool.total(), u32::from(!behold_available));

        let mut st = with_land(&cards, vec![1, 2]);
        let mut census = TurnCensus::new(1, cards.cards.len());
        run_turn(
            &cards,
            &mut st,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut Vec::new(),
            1,
            false,
        );
        assert!(st.hand.is_empty());
        assert_eq!(st.exile, [CardIdx(2)]);
        assert_eq!(st.graveyard, [CardIdx(1)]);
        assert_eq!(st.life, if behold_available { 20 } else { 19 });
        assert_eq!(st.life_paid, 0);
        assert_eq!(
            census.mana_spent,
            [if behold_available { 0.0 } else { 1.0 }]
        );
        assert!(st.completion.is_none());
    }
}

/// Pitch cards and mandatory discard cards remain distinct in the planner's target gate.
#[test]
fn a_pitch_target_cannot_reuse_its_pitch_card_for_an_additional_discard() {
    for spare in [false, true] {
        let mut pitch = row("Pitch card", "{9}", "Sorcery", "");
        pitch.colors = "[\"U\"]".into();
        let mut cards = deck(&[
            row(
                "Painful only",
                "",
                "Land",
                "{T}: Add {U}. This land deals 1 damage to you.",
            ),
            row(
                "Pitch spell",
                "{9}{U}",
                "Sorcery",
                "You may exile one blue card from your hand rather than pay this spell's mana cost.\nAs an additional cost to cast this spell, behold an Elf or pay {1}.",
            ),
            pitch,
            row("Discard card", "{99}", "Artifact", ""),
        ]);
        cards.cards[1].spell_data.additional_cost_discards = 1;
        let hand = if spare { vec![1, 2, 3] } else { vec![1, 2] };
        let mut st = with_land(&cards, hand);
        let mut census = TurnCensus::new(1, cards.cards.len());
        run_turn(
            &cards,
            &mut st,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut Vec::new(),
            1,
            false,
        );
        assert_eq!(st.life, if spare { 19 } else { 20 });
        assert_eq!(st.life_paid, 0);
        assert_eq!(st.battlefield[0].tapped, spare);
        assert_eq!(st.hand.is_empty(), spare);
        assert_eq!(census.mana_spent, [if spare { 1.0 } else { 0.0 }]);
        if spare {
            assert_eq!(st.exile, [CardIdx(2)]);
            assert_eq!(st.graveyard, [CardIdx(3), CardIdx(1)]);
        } else {
            assert_eq!(st.hand, [CardIdx(1), CardIdx(2)]);
            assert!(st.exile.is_empty() && st.graveyard.is_empty());
        }
        assert!(st.completion.is_none());
    }
}
