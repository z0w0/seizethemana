use super::mana_modes_tests::with_land;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::game::new_perm_with;
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_mana::payable;
use crate::deck::simulator::game_run::mana::{build_pool, tap_dorks_for_mana, tap_new_rocks};
use crate::deck::simulator::game_run::{TurnCensus, run_turn};
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::scenario_test_support::{deck, row};
use crate::deck::simulator::turn_loop_tests::cast;

/// Keyword and unavailable alternate costs do not forbid ordinary mana payment.
#[test]
fn base_mana_payments_remain_available_with_cast_resource_keywords() {
    for mechanic in [
        "Convoke",
        "Improvise",
        "Delve",
        "You may exile one blue card from your hand rather than pay this spell's mana cost.",
    ] {
        for (cost, accepted) in [("{U}", true), ("{U}{U}", false)] {
            let cards = deck(&[
                named_row("Adarkar Wastes"),
                row("Spell", cost, "Sorcery", mechanic),
            ]);
            let card = &cards.cards[1];
            assert!(
                card.keyword_abilities.convoke
                    || card.keyword_abilities.improvise
                    || card.keyword_abilities.delve
                    || card.spell_data.alternative_cast_cost.is_some()
            );
            let mut st = with_land(&cards, vec![1]);
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

            assert!(st.completion.is_none(), "{mechanic} {cost}");
            assert_eq!(st.hand.is_empty(), accepted, "{mechanic} {cost}");
            assert_eq!(st.life, if accepted { 19 } else { 20 });
            assert_eq!(st.life_paid, 0);
            assert_eq!(census.mana_spent, [if accepted { 1.0 } else { 0.0 }]);
        }
    }
}

/// A cast that will use a free pitch cost does not need painful printed-cost mana.
#[test]
fn an_available_pitch_cost_does_not_cause_unused_pain() {
    let mut cards = deck(&[
        named_row("Adarkar Wastes"),
        row(
            "Pitch spell",
            "{U}",
            "Sorcery",
            "You may exile one blue card from your hand rather than pay this spell's mana cost.",
        ),
        row("Pitch card", "{9}", "Sorcery", ""),
    ]);
    cards.cards[2].colors[1] = true;
    let mut st = with_land(&cards, vec![1, 2]);
    let mut pool = build_pool(&cards, &mut st, 1);
    assert_eq!(pool.colorless, 1);
    assert_eq!(st.life, 20);
    assert_eq!(cast(&cards, &mut st, &mut pool), [0.0]);
    assert_eq!(st.exile, [CardIdx(2)]);
    assert!(st.hand.is_empty());
    assert_eq!(st.life, 20);
}

/// Ready dork mana participates before a land's irreversible painless float.
#[test]
fn ready_green_dork_and_painland_pay_a_complete_blue_spell() {
    for reverse in [false, true] {
        let cards = deck(&[
            named_row("Adarkar Wastes"),
            row("Green dork", "{G}", "Creature — Elf", "{T}: Add {G}."),
            row("Spell", "{1}{U}", "Sorcery", ""),
        ]);
        let mut st = with_land(&cards, vec![2]);
        let mut dork = new_perm_with(2, &cards, CardIdx(1), 0, false);
        dork.summoning_sick = false;
        st.battlefield.push(dork);
        st.next_uid = 2;
        if reverse {
            st.battlefield.reverse();
        }
        let mut pool = build_pool(&cards, &mut st, 1);

        assert!(payable(&parse_cost("{1}{U}"), &pool));
        assert_eq!(pool.total(), 2);
        assert_eq!(st.life, 19);
        assert!(st.battlefield.iter().all(|source| source.tapped));
        tap_dorks_for_mana(&cards, &mut st, &mut pool, 1);
        tap_new_rocks(&cards, &mut st, &mut pool, 1);
        assert_eq!(pool.total(), 2);
        assert_eq!(st.life, 19);
        assert_eq!(cast(&cards, &mut st, &mut pool), [2.0]);
        assert!(st.hand.is_empty());
        assert_eq!(pool.total(), 0);
        assert_eq!(st.life_paid, 0);
        assert_eq!(build_pool(&cards, &mut st, 1).total(), 0);
    }
}

/// The normal turn sees an established dork before selecting the land's output.
#[test]
fn full_turn_cast_uses_ready_dork_and_painland_once() {
    let cards = deck(&[
        named_row("Adarkar Wastes"),
        row("Green dork", "{G}", "Creature — Elf", "{T}: Add {G}."),
        row("Spell", "{1}{U}", "Sorcery", ""),
    ]);
    let mut st = with_land(&cards, vec![2]);
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    st.next_uid = 2;
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

    assert!(st.completion.is_none());
    assert!(st.hand.is_empty());
    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 0);
    assert_eq!(census.mana_spent, [2.0]);
}

/// Two physical sources cannot fund a three-mana spell or two missing blue pips.
#[test]
fn ready_dork_capacity_does_not_justify_an_incomplete_painful_payment() {
    for cost in ["{2}{U}", "{U}{U}"] {
        let cards = deck(&[
            named_row("Adarkar Wastes"),
            row("Green dork", "{G}", "Creature — Elf", "{T}: Add {G}."),
            row("Unpayable", cost, "Sorcery", ""),
        ]);
        let mut st = with_land(&cards, vec![2]);
        let mut dork = new_perm_with(2, &cards, CardIdx(1), 0, false);
        dork.summoning_sick = false;
        st.battlefield.push(dork);
        st.next_uid = 2;
        let mut pool = build_pool(&cards, &mut st, 1);

        assert_eq!(pool.colorless, 1);
        assert!(!st.battlefield[1].tapped);
        assert_eq!(st.life, 20);
        assert_eq!(cast(&cards, &mut st, &mut pool), [0.0]);
        assert_eq!(st.hand, [CardIdx(2)]);
        assert_eq!(st.life_paid, 0);
        assert!(st.completion.is_none());
    }
}

/// Tapped, sick, and newly entered non-haste creatures cannot fund a painful choice.
#[test]
fn ready_creature_choices_preserve_sickness_haste_and_source_exclusivity() {
    for (entered, sick, haste, tapped, accepted) in [
        (0, false, false, false, true),
        (0, true, false, false, false),
        (1, false, false, false, false),
        (1, true, true, false, true),
        (0, false, true, true, false),
    ] {
        let cards = deck(&[
            named_row("Adarkar Wastes"),
            row(
                "Dork",
                "{G}",
                "Creature",
                if haste {
                    "Haste\n{T}: Add {G}."
                } else {
                    "{T}: Add {G}."
                },
            ),
            row("Spell", "{1}{U}", "Sorcery", ""),
        ]);
        let mut st = with_land(&cards, vec![2]);
        let mut dork = new_perm_with(2, &cards, CardIdx(1), entered, tapped);
        dork.summoning_sick = sick;
        st.battlefield.push(dork);
        st.next_uid = 2;
        let pool = build_pool(&cards, &mut st, 1);

        assert_eq!(payable(&parse_cost("{1}{U}"), &pool), accepted);
        assert_eq!(pool.total(), if accepted { 2 } else { 1 });
        assert_eq!(st.life, if accepted { 19 } else { 20 });
        assert_eq!(st.life_paid, 0);
        assert_eq!(st.battlefield[1].tapped, tapped || accepted);
    }
}

/// A painless dork can avoid land damage, while an unneeded body stays untapped.
#[test]
fn creature_sources_avoid_pain_without_tapping_unused_bodies() {
    for (cost, dork_mana, dork_used) in [("{U}", "{U}", true), ("{1}", "{G}", false)] {
        let cards = deck(&[
            named_row("Adarkar Wastes"),
            row(
                "Dork",
                "{1}",
                "Creature",
                &format!("{{T}}: Add {dork_mana}."),
            ),
            row("Spell", cost, "Sorcery", ""),
        ]);
        let mut st = with_land(&cards, vec![2]);
        let mut dork = new_perm_with(2, &cards, CardIdx(1), 0, false);
        dork.summoning_sick = false;
        st.battlefield.push(dork);
        st.next_uid = 2;
        let mut pool = build_pool(&cards, &mut st, 1);
        assert_eq!(st.life, 20);
        assert_eq!(st.battlefield[1].tapped, dork_used);
        assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);
        assert_eq!(st.life, 20);
    }
}

/// Separate granted mana can fund the spell without inheriting native mode limits.
#[test]
fn ready_creature_grants_participate_in_the_payment_plan() {
    let cards = deck(&[
        named_row("Adarkar Wastes"),
        row("Body", "{1}", "Creature", ""),
        row(
            "Mana grant",
            "{9}",
            "Enchantment",
            "Creatures you control have \"{T}: Add one mana of any color.\"",
        ),
        row("Spell", "{1}{U}", "Sorcery", ""),
    ]);
    let mut st = with_land(&cards, vec![3]);
    let mut body = new_perm_with(2, &cards, CardIdx(1), 0, false);
    body.summoning_sick = false;
    st.battlefield.push(body);
    st.battlefield
        .push(new_perm_with(3, &cards, CardIdx(2), 0, false));
    st.next_uid = 3;
    let mut pool = build_pool(&cards, &mut st, 1);
    assert_eq!(st.life, 20);
    assert!(st.battlefield[1].tapped);
    assert_eq!(cast(&cards, &mut st, &mut pool), [2.0]);
    assert_eq!(pool.total(), 0);
    assert_eq!(st.life_paid, 0);
}

/// Current source order cannot force the player to choose a lethal expensive mode.
#[test]
fn minimum_damage_choices_work_in_both_orders_at_low_and_high_life() {
    for reverse in [false, true] {
        for life in [3, 20] {
            for cost in ["{U}", "{1}{U}"] {
                let cards = deck(&[
                    row(
                        "Cheap",
                        "",
                        "Land",
                        "{T}: Add {C}.\n{T}: Add {U}. This land deals 1 damage to you.",
                    ),
                    row(
                        "Expensive",
                        "",
                        "Land",
                        "{T}: Add {C}.\n{T}: Add {U}. This land deals 3 damage to you.",
                    ),
                    row("Spell", cost, "Sorcery", ""),
                ]);
                let mut st = with_land(&cards, vec![2]);
                st.life = life;
                st.battlefield
                    .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
                st.next_uid = 2;
                if reverse {
                    st.battlefield.reverse();
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

                assert!(st.completion.is_none());
                assert!(st.hand.is_empty());
                assert_eq!(st.life, life - 1);
                assert_eq!(st.life_paid, 0);
                assert_eq!(census.mana_spent, [if cost == "{U}" { 1.0 } else { 2.0 }]);
            }
        }
    }
}

/// The price belongs to each actual mode, not one price for the entire source.
#[test]
fn minimum_damage_choices_can_assign_different_cheap_colors() {
    let cards = deck(&[
        row(
            "Cheap white",
            "",
            "Land",
            "{T}: Add {C}.\n{T}: Add {W}. This land deals 1 damage to you.\n{T}: Add {U}. This land deals 3 damage to you.",
        ),
        row(
            "Cheap blue",
            "",
            "Land",
            "{T}: Add {C}.\n{T}: Add {U}. This land deals 1 damage to you.\n{T}: Add {W}. This land deals 3 damage to you.",
        ),
        row("Spell", "{W}{U}", "Sorcery", ""),
    ]);
    let mut st = with_land(&cards, vec![2]);
    st.life = 3;
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    st.next_uid = 2;
    let mut pool = build_pool(&cards, &mut st, 1);
    assert_eq!(st.life, 1);
    assert_eq!(st.life_paid, 0);
    assert_eq!(cast(&cards, &mut st, &mut pool), [2.0]);
    assert!(st.hand.is_empty());
    assert_eq!(pool.total(), 0);
}

/// Behold's surcharge must be payable before a land deals damage for colored mana.
#[test]
fn visible_hand_costs_include_behold_before_generic_reductions() {
    for reduction in [false, true] {
        let cards = deck(&[
            named_row("Adarkar Wastes"),
            row(
                "Pending Elf",
                "{U}",
                "Creature — Elf",
                "As an additional cost to cast this spell, behold an Elf or pay {1}.",
            ),
            row(
                "Reducer",
                "{9}",
                "Artifact",
                "Spells you cast cost {1} less to cast.",
            ),
        ]);
        let mut st = with_land(&cards, vec![1]);
        if reduction {
            st.battlefield
                .push(new_perm_with(2, &cards, CardIdx(2), 0, false));
            st.next_uid = 2;
        }
        let mut pool = build_pool(&cards, &mut st, 1);
        assert_eq!(st.life, if reduction { 19 } else { 20 });
        assert_eq!(
            cast(&cards, &mut st, &mut pool),
            [if reduction { 1.0 } else { 0.0 }]
        );
        assert_eq!(st.hand.is_empty(), reduction);
        assert_eq!(st.life_paid, 0);
    }
}

/// Commander tax and Behold use the shared current-state cost calculation.
#[test]
fn visible_commander_costs_include_tax_and_behold_before_reductions() {
    for (reduction, accepted) in [(2, false), (3, true)] {
        let mut cards = deck(&[
            named_row("Adarkar Wastes"),
            row(
                "Reducer",
                "{9}",
                "Artifact",
                &format!("Spells you cast cost {{{reduction}}} less to cast."),
            ),
        ]);
        cards.commanders = deck(&[row(
            "Commander",
            "{U}",
            "Legendary Creature — Elf",
            "As an additional cost to cast this spell, behold an Elf or pay {1}.",
        )])
        .cards;
        let mut st = with_land(&cards, Vec::new());
        st.commander_casts = vec![1];
        st.battlefield
            .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
        st.next_uid = 2;
        let mut census = TurnCensus::with_commander_count(1, cards.cards.len(), 1);
        run_turn(
            &cards,
            &mut st,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut Vec::new(),
            1,
            false,
        );

        assert!(st.completion.is_none());
        assert_eq!(st.commander_casts, [if accepted { 2 } else { 1 }]);
        assert_eq!(st.life, if accepted { 19 } else { 20 });
        assert_eq!(st.life_paid, 0);
        assert_eq!(census.mana_spent, [if accepted { 1.0 } else { 0.0 }]);
    }
}
