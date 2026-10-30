use super::*;

#[test]
fn nonland_mana_trigger_adds_one_produced_mana() {
    let rows = [
        row(
            "Mana trigger",
            "{G}{U}",
            "Creature — Human Druid",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced. Gain 1 life.",
        ),
        row("Three mana rock", "{3}", "Artifact", "{T}: Add {C}{C}{C}."),
    ];
    let deck = deck(&rows);
    assert!(deck.cards[0].flags.bonus_mana_on_nonland_tap);
    let mut st = state(vec![], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        0,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    st.battlefield.push(super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(1),
        1,
        false,
    ));

    let pool = build_pool(&deck, &mut st, 1);

    assert_eq!(pool.total(), 4);
    assert!(super::game_mana::payment_plan(&parse_cost("{C}{C}{C}{C}"), &pool, &[]).is_some());
    assert!(st.battlefield[1].tapped);
}

#[test]
fn metalcraft_mana_source_needs_three_artifacts() {
    let rows = [
        row(
            "Conditional rock",
            "{0}",
            "Artifact",
            "Metalcraft — {T}: Add one mana of any color. Activate only if you control three or more artifacts.",
        ),
        row("Artifact two", "{0}", "Artifact", ""),
        row("Artifact three", "{0}", "Artifact", ""),
        row("Land", "", "Basic Land — Island", "{T}: Add {U}."),
    ];
    let deck = deck(&rows);
    assert!(deck.cards[0].flags.requires_metalcraft);
    for (artifact_count, include_land) in [(1, true), (2, false), (3, false)] {
        let mut st = state(vec![], vec![]);
        for card in 0..artifact_count {
            st.battlefield.push(super::game::new_perm_with(
                card as u32,
                &deck,
                CardIdx(card as u32),
                1,
                false,
            ));
        }
        if include_land {
            st.battlefield.push(super::game::new_perm_with(
                1,
                &deck,
                crate::deck::simulator::model::CardIdx(3),
                1,
                false,
            ));
        }
        let pool = build_pool(&deck, &mut st, 1);
        assert_eq!(pool.total(), u32::from(include_land || artifact_count == 3));
        assert_eq!(st.battlefield[0].tapped, artifact_count == 3);
    }

    let mut active = state(vec![], vec![]);
    for index in 0..3 {
        active.battlefield.push(new_perm_with(
            index as u32,
            &deck,
            crate::deck::simulator::model::CardIdx(index as u32),
            1,
            false,
        ));
    }
    let pool = build_pool(&deck, &mut active, 1);
    assert_eq!(pool.total(), 1);
    active.battlefield.retain(|permanent| permanent.uid != 2);
    for permanent in &mut active.battlefield {
        permanent.tapped = false;
    }
    let pool = build_pool(&deck, &mut active, 2);
    assert_eq!(pool.total(), 0, "Mox Opal stops after an artifact leaves");
    assert!(!active.battlefield[0].tapped);
}

#[test]
fn opponent_dependent_mana_is_generic_only_from_turn_two() {
    let deck = deck(&[row(
        "Fellwar Stone",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
    )]);
    let mut st = state(vec![], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        0,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));

    let turn_one_pool = build_pool(&deck, &mut st, 1);
    assert_eq!(turn_one_pool.total(), 0);
    assert!(!st.battlefield[0].tapped);

    let turn_two_pool = build_pool(&deck, &mut st, 2);
    assert_eq!(turn_two_pool.colorless, 0);
    assert_eq!(turn_two_pool.generic_credit, 1);
    assert!(!super::game_mana::payable(
        &parse_cost("{C}"),
        &turn_two_pool
    ));
    assert!(st.battlefield[0].tapped);
    assert!(!super::game_mana::pips_ok(
        &super::model::Cost {
            pips: [0, 1, 0, 0, 0],
            ..super::model::Cost::default()
        },
        &turn_two_pool,
    ));
}

#[test]
fn reusable_mana_mode_does_not_pay_a_separate_sacrifice_mode() {
    let source = row(
        "Two-mode mana artifact",
        "{2}",
        "Artifact",
        "{T}: Add {C}{C}.\nSacrifice this artifact: Add {C}.",
    );
    let deck = deck(&[source]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, false));

    let pool = build_pool(&deck, &mut st, 1);

    assert_eq!(pool.colorless, 2);
    assert!(st.graveyard.is_empty());
    assert_eq!(st.battlefield.len(), 1);
    assert!(st.battlefield[0].tapped);
}

#[test]
fn mana_mode_must_pay_its_own_mana_cost_before_tapping() {
    let source = row(
        "Paid mana artifact",
        "{2}",
        "Artifact",
        "{1}, {T}: Add {C}.",
    );
    let deck = deck(&[source]);
    let mut unpaid = state(vec![], vec![]);
    unpaid
        .battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, false));
    let empty_pool = build_pool(&deck, &mut unpaid, 1);
    assert_eq!(empty_pool.total(), 0);
    assert!(!unpaid.battlefield[0].tapped);

    let mut paid = state(vec![], vec![]);
    paid.battlefield
        .push(new_perm_with(2, &deck, CardIdx(0), 1, false));
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };
    let source = paid.battlefield[0].clone();
    super::game_run::mana::activate_mana_mode(&deck, &mut paid, &source, &mut pool, 1);
    assert_eq!(pool.colorless, 1);
    assert_eq!(pool.flexible, 0);
    assert!(paid.battlefield[0].tapped);
}

#[test]
fn fixed_mana_restricted_from_generic_costs_keeps_its_colors() {
    let deck = deck(&[row(
        "Five-color source",
        "{5}",
        "Legendary Creature — Elk",
        "{T}: Add {W}{U}{B}{R}{G}. This mana can't be spent to pay generic mana costs.",
    )]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 0, false));
    st.battlefield[0].summoning_sick = false;
    let source = st.battlefield[0].clone();
    let mut pool = ManaPool::default();

    assert!(super::game_run::mana::activate_mana_mode(
        &deck, &mut st, &source, &mut pool, 1
    ));

    let colored = super::model::Cost {
        pips: [1; 5],
        ..super::model::Cost::default()
    };
    let generic = super::model::Cost {
        generic: 1,
        ..super::model::Cost::default()
    };
    assert_eq!(pool.fixed, [1; 5]);
    assert_eq!(super::game_mana::usable_for_noncreature(&pool), 0);
    assert!(super::game_mana::pips_ok(&colored, &pool));
    assert!(!super::game_mana::pips_ok(&generic, &pool));
    super::game_mana::pay_cost(&colored, &mut pool);
    assert_eq!(pool.total(), 0);
}

#[test]
fn mana_ability_resolves_its_other_nonlibrary_effects() {
    let source = row(
        "Life-gain mana artifact",
        "{2}",
        "Artifact",
        "{T}: Add {G}. Gain 1 life.",
    );
    let deck = deck(&[source]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, false));

    let pool = build_pool(&deck, &mut st, 1);

    assert_eq!(pool.fixed[super::model::ManaColor::Green.index()], 1);
    assert_eq!(st.life, 21);
    assert_eq!(st.life_gained, 1);
}

#[test]
fn triggered_mana_ability_resolves_after_a_nonland_mana_tap() {
    let deck = deck(&[
        row("Green rock", "{1}", "Artifact", "{T}: Add {G}."),
        row(
            "Mana trigger",
            "{1}",
            "Enchantment",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced. Gain 1 life.",
        ),
        row("Island", "", "Basic Land — Island", "{T}: Add {U}."),
    ]);
    assert!(deck.cards[1].unlocked_abilities(0).any(|ability| {
        ability.kind == super::model::SimAbilityKind::ManaTriggered
            && ability
                .effect_sequence()
                .iter()
                .any(|effect| matches!(effect, SimEffect::GainLife(1)))
    }));
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, false));
    st.battlefield
        .push(new_perm_with(2, &deck, CardIdx(1), 1, false));

    let pool = build_pool(&deck, &mut st, 1);

    assert_eq!(pool.total(), 2);
    assert!(super::game_mana::payment_plan(&parse_cost("{G}{G}"), &pool, &[]).is_some());
    assert!(super::game_mana::payment_plan(&parse_cost("{G}{U}"), &pool, &[]).is_none());
    assert_eq!(st.life, 21);
    assert!(st.battlefield[0].tapped);

    let mut land_tap = state(vec![], vec![]);
    land_tap
        .battlefield
        .push(new_perm_with(3, &deck, CardIdx(2), 1, false));
    land_tap
        .battlefield
        .push(new_perm_with(4, &deck, CardIdx(1), 1, false));
    let pool = build_pool(&deck, &mut land_tap, 1);
    assert_eq!(pool.fixed[super::model::ManaColor::Blue.index()], 1);
    assert_eq!(land_tap.life, 20);
}

#[test]
fn duplicate_land_permanents_satisfy_each_others_gate() {
    let mut deck = deck(&[row("Island Gate", "", "Land — Island", "")]);
    deck.cards[0].gate_types = vec![super::model::BasicLandType::Island];
    deck.cards[0].tap = Some(super::model::ManaYield {
        fixed: [0, 1, 1, 0, 0],
        ..super::model::ManaYield::default()
    });
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, false));
    st.battlefield
        .push(new_perm_with(2, &deck, CardIdx(0), 1, false));

    let pool = build_pool(&deck, &mut st, 1);

    assert_eq!(pool.fixed[super::model::ManaColor::Blue.index()], 2);
    assert_eq!(pool.fixed[super::model::ManaColor::Black.index()], 2);
}

#[test]
fn mana_mode_keeps_its_own_spend_restriction() {
    let deck = deck(&[
        row(
            "Restricted mana artifact",
            "{2}",
            "Artifact",
            "{T}: Add {G}{G}. Spend this mana only to cast creature spells.\nSacrifice this artifact: Add {C}.",
        ),
        row("Creature spell", "{1}{G}", "Creature — Bear", ""),
    ]);
    let mut st = state(vec![1], vec![]);
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, false));

    let pool = build_pool(&deck, &mut st, 1);

    assert_eq!(pool.total(), 2);
    assert_eq!(super::game_mana::usable_for_noncreature(&pool), 0);
    assert_eq!(
        super::game_mana::usable_for_classes(&pool, &[super::model::SpendRestriction::Creature],),
        2
    );
    assert!(st.graveyard.is_empty());
    assert!(st.battlefield[0].tapped);
}
