use super::*;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::game::{Activation, new_perm_with};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_effects::activation::{
    pick_best_activation, resolve_activation, spend_leftover,
};
use crate::deck::simulator::game_mana::{pay_cost, payable};
use crate::deck::simulator::game_run::mana::{
    activate_mana_mode, build_pool, tap_for_nonspell_payment,
};
use crate::deck::simulator::game_run::{TurnCensus, run_turn};
use crate::deck::simulator::model::{CardIdx, SimAbilityCondition};
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use crate::deck::simulator::turn_loop_tests::cast;

/// Every painland identity comes from the pinned source capture.
const PAINLANDS: [(&str, [&str; 2]); 10] = [
    ("Adarkar Wastes", ["{W}", "{U}"]),
    ("Underground River", ["{U}", "{B}"]),
    ("Sulfurous Springs", ["{B}", "{R}"]),
    ("Karplusan Forest", ["{R}", "{G}"]),
    ("Brushland", ["{G}", "{W}"]),
    ("Caves of Koilos", ["{W}", "{B}"]),
    ("Shivan Reef", ["{U}", "{R}"]),
    ("Llanowar Wastes", ["{B}", "{G}"]),
    ("Battlefield Forge", ["{R}", "{W}"]),
    ("Yavimaya Coast", ["{G}", "{U}"]),
];

/// Put one known land on the battlefield without changing its captured card data.
pub(super) fn with_land(cards: &SimDeck, hand: Vec<usize>) -> GameState {
    let mut st = state(hand, Vec::new());
    st.battlefield
        .push(new_perm_with(1, cards, CardIdx(0), 0, false));
    st.next_uid = 1;
    st
}

/// All ten source identities keep painless colorless and generic payments.
#[test]
fn captured_painlands_pay_colorless_and_generic_without_damage() {
    for (name, _) in PAINLANDS {
        for cost in ["{C}", "{1}"] {
            let cards = deck(&[named_row(name), row("Spell", cost, "Sorcery", "")]);
            let mut st = with_land(&cards, vec![1]);
            let mut pool = build_pool(&cards, &mut st, 1);

            assert_eq!(pool.total(), 1, "{name} {cost}");
            assert_eq!(pool.colorless, 1, "{name} {cost}");
            assert!(st.battlefield[0].tapped);
            assert_eq!(st.life, 20);
            assert_eq!(st.life_paid, 0);
            assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);
            assert!(st.hand.is_empty());
            assert_eq!(pool.total(), 0);
        }
    }
}

/// Each printed painland color resolves one damage effect, not a life payment.
#[test]
fn captured_painlands_pay_each_printed_color_with_one_damage() {
    for (name, colors) in PAINLANDS {
        for cost in colors {
            let cards = deck(&[named_row(name), row("Spell", cost, "Sorcery", "")]);
            let mut st = with_land(&cards, vec![1]);
            let mut pool = build_pool(&cards, &mut st, 1);

            assert!(payable(&parse_cost(cost), &pool), "{name} {cost}");
            assert!(!payable(&parse_cost("{C}"), &pool));
            assert_eq!(pool.total(), 1);
            assert_eq!(st.life, 19, "{name} {cost}");
            assert_eq!(st.life_paid, 0);
            assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);
            assert_eq!(st.life, 19);
            assert_eq!(pool.total(), 0);
        }
    }
}

/// A wrong-color or two-symbol spell does not justify painful mana production.
#[test]
fn captured_painlands_never_invent_a_color_or_second_tap() {
    for (name, colors) in PAINLANDS {
        let wrong = ["{W}", "{U}", "{B}", "{R}", "{G}"]
            .into_iter()
            .find(|cost| !colors.contains(cost))
            .expect("each painland lacks three colors");
        for cost in [wrong.to_string(), format!("{}{}", colors[0], colors[1])] {
            let cards = deck(&[named_row(name), row("Spell", &cost, "Sorcery", "")]);
            let mut st = with_land(&cards, vec![1]);
            let mut pool = build_pool(&cards, &mut st, 1);
            let before = pool.clone();

            assert!(!payable(&parse_cost(&cost), &pool), "{name} {cost}");
            assert_eq!(cast(&cards, &mut st, &mut pool), [0.0]);
            let source = st.battlefield[0].clone();
            assert!(!activate_mana_mode(&cards, &mut st, &source, &mut pool, 1));
            assert_eq!(pool, before);
            assert_eq!(st.life, 20);
            assert_eq!(st.life_paid, 0);
            assert_eq!(st.hand, [CardIdx(1)]);
        }
    }
}

/// The ordinary turn pipeline makes the same painless and painful choices.
#[test]
fn captured_painland_modes_reach_normal_full_turn_casts() {
    for (name, colors) in PAINLANDS {
        for (cost, life) in [("{C}", 20), ("{1}", 20), (colors[0], 19), (colors[1], 19)] {
            let cards = deck(&[named_row(name), row("Spell", cost, "Sorcery", "")]);
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

            assert!(st.completion.is_none(), "{name} {cost}");
            assert!(st.hand.is_empty(), "{name} {cost}");
            assert_eq!(st.life, life, "{name} {cost}");
            assert_eq!(st.life_paid, 0);
            assert_eq!(census.mana_spent, [1.0]);
        }
    }
}

/// No visible eligible cast means every captured painland uses its harmless mode.
#[test]
fn captured_painlands_do_not_hurt_the_player_for_unused_capacity() {
    for (name, _) in PAINLANDS {
        let cards = deck(&[named_row(name)]);
        let mut st = with_land(&cards, Vec::new());
        let pool = build_pool(&cards, &mut st, 1);
        assert_eq!(pool.colorless, 1, "{name}");
        assert_eq!(pool.total(), 1);
        assert_eq!(st.life, 20);
        assert_eq!(st.life_paid, 0);
    }
}

/// All native simple modes remain available without relying on the source's name.
#[test]
fn equal_effect_modes_keep_both_colors_on_one_physical_land() {
    for cost in ["{W}", "{U}"] {
        let mut cards = deck(&[
            row("Two modes", "", "Land", "{T}: Add {W}.\n{T}: Add {U}."),
            row("Spell", cost, "Sorcery", ""),
        ]);
        cards.cards[0].name = "Changed after lowering".to_string();
        let mut st = with_land(&cards, vec![1]);
        let mut pool = build_pool(&cards, &mut st, 1);

        assert_eq!(pool.total(), 1);
        assert!(payable(&parse_cost("{W}"), &pool));
        assert!(payable(&parse_cost("{U}"), &pool));
        assert!(!payable(&parse_cost("{W}{U}"), &pool));
        assert!(pay_cost(&parse_cost(cost), &mut pool));
        assert_eq!(pool.total(), 0);
    }
}

/// Each typed condition gates only its own mode and never removes the first mode.
#[test]
fn typed_condition_on_second_mode_keeps_first_color() {
    for unlocked in [false, true] {
        let mut cards = deck(&[
            row(
                "Conditional modes",
                "",
                "Land",
                "{T}: Add {W}.\n{T}: Add {U}.",
            ),
            row("Artifact", "{9}", "Artifact", ""),
        ]);
        cards.cards[0].striations[0].abilities[1].condition = Some(SimAbilityCondition::Metalcraft);
        let mut st = with_land(&cards, Vec::new());
        if unlocked {
            for uid in 2..=4 {
                st.battlefield
                    .push(new_perm_with(uid, &cards, CardIdx(1), 0, false));
            }
            st.next_uid = 4;
        }
        let pool = build_pool(&cards, &mut st, 1);
        assert_eq!(pool.total(), 1);
        assert!(payable(&parse_cost("{W}"), &pool));
        assert_eq!(payable(&parse_cost("{U}"), &pool), unlocked);
    }
}

/// A captured Verge retains its first color when its other mode unlocks.
#[test]
fn captured_verge_keeps_both_colors_only_when_its_gate_is_met() {
    for (land_type, unlocked) in [
        ("Land — Plains", true),
        ("Land — Island", true),
        ("Land — Forest", false),
    ] {
        for cost in ["{W}", "{U}"] {
            let cards = deck(&[
                named_row("Floodfarm Verge"),
                row("Gate land", "", land_type, ""),
                row("Spell", cost, "Sorcery", ""),
            ]);
            let mut st = with_land(&cards, vec![2]);
            st.battlefield
                .push(new_perm_with(2, &cards, CardIdx(1), 0, true));
            st.next_uid = 2;
            let mut pool = build_pool(&cards, &mut st, 1);

            assert_eq!(pool.total(), 1);
            assert!(payable(&parse_cost("{W}"), &pool));
            assert_eq!(payable(&parse_cost("{U}"), &pool), unlocked);
            assert_eq!(
                cast(&cards, &mut st, &mut pool),
                [if cost == "{W}" || unlocked { 1.0 } else { 0.0 }]
            );
            assert_eq!(st.life, 20);
        }
    }
}

/// Deferring a painland prevents needless damage when a later source pays the spell.
#[test]
fn ordinary_sources_and_treasures_are_known_before_painland_choices() {
    for treasure in [false, true] {
        let cards = deck(&[
            named_row("Adarkar Wastes"),
            row("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
            row("Spell", "{U}", "Sorcery", ""),
        ]);
        let mut st = with_land(&cards, vec![2]);
        if treasure {
            st.treasure_bank = 1;
        } else {
            st.battlefield
                .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
            st.next_uid = 2;
        }
        let mut pool = build_pool(&cards, &mut st, 1);
        assert_eq!(st.life, 20);
        assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);
        assert_eq!(st.life, 20);
        assert_eq!(st.life_paid, 0);
    }
}

/// The target matcher considers all deferred physical lands together.
#[test]
fn two_painlands_can_pay_different_required_colors_once_each() {
    let cards = deck(&[
        named_row("Adarkar Wastes"),
        named_row("Caves of Koilos"),
        row("Spell", "{W}{U}", "Sorcery", ""),
    ]);
    let mut st = with_land(&cards, vec![2]);
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    st.next_uid = 2;
    let mut pool = build_pool(&cards, &mut st, 1);
    assert!(payable(&parse_cost("{W}{U}"), &pool));
    assert_eq!(pool.total(), 2);
    assert_eq!(st.life, 18);
    assert_eq!(cast(&cards, &mut st, &mut pool), [2.0]);
    assert_eq!(pool.total(), 0);
    assert_eq!(st.life_paid, 0);
}

/// Unsupported and unpaid mandatory costs never justify a painful tap.
#[test]
fn ineligible_spell_additional_costs_do_not_choose_painful_mana() {
    for failure in [
        "unsupported",
        "sacrifice",
        "discard",
        "life",
        "target",
        "missing_cost",
    ] {
        let mut cards = deck(&[
            named_row("Adarkar Wastes"),
            row("Spell", "{U}", "Sorcery", ""),
        ]);
        let card = &mut cards.cards[1];
        match failure {
            "unsupported" => card.spell_data.unsupported_additional_cost = true,
            "sacrifice" => card.spell_data.additional_cost_creatures = 1,
            "discard" => card.spell_data.additional_cost_discards = 1,
            "life" => card.spell_data.additional_cost_life = 20,
            "target" => {
                card.spell_data.effects = vec![SimEffect::ReturnFromGraveyard {
                    to_hand: false,
                    count: 1,
                }]
            }
            "missing_cost" => card.has_mana_cost = false,
            _ => unreachable!("the test lists every failure"),
        }
        let mut st = with_land(&cards, vec![1]);
        let pool = build_pool(&cards, &mut st, 1);
        assert_eq!(pool.colorless, 1, "{failure}");
        assert_eq!(st.life, 20, "{failure}");
        assert_eq!(st.life_paid, 0);
    }
}

/// Mana ability damage must leave enough life for the cast's separate life charge.
#[test]
fn planner_preserves_life_for_phyrexian_and_additional_costs() {
    for life in [2, 3, 4] {
        let cards = deck(&[
            named_row("Adarkar Wastes"),
            row("Spell", "{U}{W/P}", "Sorcery", ""),
        ]);
        let mut st = with_land(&cards, vec![1]);
        st.life = life;
        let pool = build_pool(&cards, &mut st, 1);
        assert_eq!(st.life, if life >= 3 { life - 1 } else { life });
        assert_eq!(payable(&parse_cost("{U}"), &pool), life >= 3);
        assert_eq!(st.life_paid, 0);
    }
}

/// Generic nonspell payments choose a painland's painless output.
#[test]
fn nonspell_payment_never_merges_painful_colored_output() {
    let cards = deck(&[named_row("Adarkar Wastes")]);
    let mut st = with_land(&cards, Vec::new());
    let source = st.battlefield[0].clone();
    let mut pool = ManaPool::default();
    assert!(tap_for_nonspell_payment(
        &cards, &mut st, &source, &mut pool, 1
    ));
    assert_eq!(pool.colorless, 1);
    assert!(!payable(&parse_cost("{U}"), &pool));
    assert_eq!(st.life, 20);
    assert_eq!(st.life_paid, 0);
}

/// A rejected painful-only activation cannot mutate the pool, life, or source.
#[test]
fn failed_painful_only_attempt_is_atomic() {
    let cards = deck(&[
        row(
            "Painful land",
            "",
            "Land",
            "{T}: Add {U}. This land deals 1 damage to you.",
        ),
        row("Unpayable", "{U}{U}", "Sorcery", ""),
    ]);
    let mut st = with_land(&cards, vec![1]);
    let source = st.battlefield[0].clone();
    let mut pool = ManaPool::default();
    assert!(!activate_mana_mode(&cards, &mut st, &source, &mut pool, 1));
    assert_eq!(pool, ManaPool::default());
    assert!(!st.battlefield[0].tapped);
    assert_eq!(st.life, 20);
    assert_eq!(st.life_paid, 0);
    assert_eq!(st.hand, [CardIdx(1)]);
    assert!(st.activated_this_turn.is_empty());
    assert!(st.activated_once.is_empty());
}

/// A full turn cannot pay a two-mana spell by hurting the player for one mana.
#[test]
fn failed_painful_only_cast_stays_atomic_through_the_full_turn() {
    let cards = deck(&[
        row(
            "Painful land",
            "",
            "Land",
            "{T}: Add {U}. This land deals 1 damage to you.",
        ),
        row("Unpayable", "{U}{U}", "Sorcery", ""),
    ]);
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
    assert!(st.completion.is_none());
    assert!(!st.battlefield[0].tapped);
    assert_eq!(st.life, 20);
    assert_eq!(st.life_paid, 0);
    assert_eq!(st.hand, [CardIdx(1)]);
    assert_eq!(census.mana_spent, [0.0]);
}

/// Neither general activation selection nor direct resolution can bypass the planner.
#[test]
fn rejected_painful_only_modes_cannot_use_the_general_activation_path() {
    for cost in [None, Some("{W}"), Some("{U}{U}")] {
        let cards = deck(&[
            row(
                "Painful land",
                "",
                "Land",
                "{T}: Add {U}. This land deals 1 damage to you.",
            ),
            row("Unpayable", cost.unwrap_or("{U}"), "Sorcery", ""),
        ]);
        let mut st = with_land(&cards, if cost.is_some() { vec![1] } else { Vec::new() });
        let hand_before = st.hand.clone();
        let mut pool = build_pool(&cards, &mut st, 1);
        assert_eq!(pool, ManaPool::default());
        assert!(!st.battlefield[0].tapped);
        let source = st.battlefield[0].clone();
        assert!(!activate_mana_mode(&cards, &mut st, &source, &mut pool, 1));
        assert!(!tap_for_nonspell_payment(
            &cards, &mut st, &source, &mut pool, 1
        ));
        assert!(pick_best_activation(&cards, &st, &pool, 1).is_none());
        let activation = Activation {
            pos: 0,
            uid: 1,
            ability: cards.cards[0].striations[0].abilities[0].clone(),
            ability_cost: Cost::default(),
            paid_x: 0,
            draws: 0,
            sacrifice_uid: None,
            target_uid: None,
        };
        resolve_activation(&cards, &mut st, &mut pool, 1, &activation);
        spend_leftover(&cards, &mut st, &mut pool, 1);

        assert_eq!(pool, ManaPool::default());
        assert!(!st.battlefield[0].tapped);
        assert_eq!(st.life, 20);
        assert_eq!(st.life_paid, 0);
        assert_eq!(st.hand, hand_before);
        assert!(st.activated_this_turn.is_empty());
        assert!(st.activated_once.is_empty());
        assert!(st.completion.is_none());
    }
}

/// Blocking the general activation bypass does not block a complete planned cast.
#[test]
fn useful_painful_only_modes_still_pay_normal_full_turn_casts() {
    for cost in ["{U}", "{1}"] {
        let cards = deck(&[
            row(
                "Painful land",
                "",
                "Land",
                "{T}: Add {U}. This land deals 1 damage to you.",
            ),
            row("Payable", cost, "Sorcery", ""),
        ]);
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

        assert!(st.completion.is_none());
        assert!(st.battlefield[0].tapped);
        assert_eq!(st.life, 19);
        assert_eq!(st.life_paid, 0);
        assert!(st.hand.is_empty());
        assert_eq!(census.mana_spent, [1.0]);
    }
}

/// Extra costs and effects cannot become free choices in a merged mana lot.
#[test]
fn simple_mode_rejects_extra_costs_and_effects() {
    let cards = deck(&[named_row("Adarkar Wastes")]);
    let st = with_land(&cards, Vec::new());
    let base = cards.cards[0].striations[0]
        .abilities
        .iter()
        .find(|ability| matches!(ability.effect_sequence(), [SimEffect::Mana(_)]))
        .expect("the source includes its painless colorless mode");
    for cost in [
        SimActivationCost::PayLife(1),
        SimActivationCost::Mana(parse_cost("{1}")),
    ] {
        let mut changed = base.clone();
        changed
            .activation
            .as_mut()
            .expect("mana ability has activation costs")
            .costs
            .push(cost);
        assert!(simple_mode(&cards, &st, &changed).is_none());
    }
    for effect in [
        SimEffect::Draw(1),
        SimEffect::LoseLife {
            amount: 1,
            scope: LifeLossScope::SelfOnly,
        },
    ] {
        let mut changed = base.clone();
        changed.effects = vec![base.effect.clone(), effect];
        assert!(simple_mode(&cards, &st, &changed).is_none());
    }
}

/// A mode with extra costs still pays them through the normal typed ability path.
#[test]
fn paid_extra_mode_never_becomes_a_free_color_choice() {
    let cards = deck(&[row(
        "Paid mode",
        "",
        "Land",
        "{T}: Add {C}.\n{T}, Pay 1 life: Add {U}.",
    )]);
    let mut st = with_land(&cards, Vec::new());
    assert!(simple_source(&cards, &st, &st.battlefield[0]).is_none());
    let pool = build_pool(&cards, &mut st, 1);
    assert!(payable(&parse_cost("{U}"), &pool));
    assert!(!payable(&parse_cost("{C}"), &pool));
    assert_eq!(pool.total(), 1);
    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 1);
}

/// Merging color alternatives cannot silently erase a native mode's extra life gain.
#[test]
fn extra_effect_mode_keeps_its_effect_in_ordinary_pool_building() {
    let cards = deck(&[row(
        "Extra effect",
        "",
        "Land",
        "{T}: Add {C}.\n{T}: Add {U}. You gain 1 life.",
    )]);
    let mut st = with_land(&cards, Vec::new());
    assert!(simple_source(&cards, &st, &st.battlefield[0]).is_none());
    let pool = build_pool(&cards, &mut st, 1);
    assert!(payable(&parse_cost("{U}"), &pool));
    assert!(!payable(&parse_cost("{C}"), &pool));
    assert_eq!(st.life, 21);
    assert_eq!(st.life_gained, 1);
    assert_eq!(pool.total(), 1);
}

/// The bounded policy cannot retap an already-floated painless source after a draw.
#[test]
fn painless_float_does_not_gain_free_colored_mana_for_a_later_draw() {
    let cards = deck(&[
        named_row("Adarkar Wastes"),
        row("Cantrip", "{1}", "Sorcery", "Draw a card."),
        row("Drawn spell", "{U}", "Sorcery", ""),
    ]);
    let mut st = with_land(&cards, vec![1]);
    st.library.push(CardIdx(2));
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
    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.life, 20);
    assert_eq!(st.life_paid, 0);
    assert_eq!(census.mana_spent, [1.0]);
}

/// Commander mana and recast tax use the same complete payment check.
#[test]
fn commander_cost_can_justify_pain_but_unpayable_recast_tax_cannot() {
    for casts in [0, 1] {
        let mut cards = deck(&[named_row("Adarkar Wastes")]);
        cards.commanders = deck(&[row("Commander", "{U}", "Legendary Creature", "")]).cards;
        let mut st = with_land(&cards, Vec::new());
        st.commander_casts = vec![casts];
        let pool = build_pool(&cards, &mut st, 1);
        assert_eq!(st.life, if casts == 0 { 19 } else { 20 });
        assert_eq!(payable(&parse_cost("{U}"), &pool), casts == 0);
        assert_eq!(st.life_paid, 0);
    }
}

/// A real command-zone cast consumes the planned colored output once.
#[test]
fn full_turn_commander_cast_resolves_pain_once() {
    let mut cards = deck(&[named_row("Adarkar Wastes")]);
    cards.commanders = deck(&[row("Commander", "{U}", "Legendary Creature", "")]).cards;
    let mut st = with_land(&cards, Vec::new());
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
    assert_eq!(st.commander_casts, [1]);
    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 0);
    assert_eq!(census.mana_spent, [1.0]);
}
