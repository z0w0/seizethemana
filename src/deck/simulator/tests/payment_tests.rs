use super::*;
use crate::deck::simulator::game::new_perm_with;
use crate::deck::simulator::game_mana::{
    ManaLot, add_yield, add_yield_turns, add_yield_turns_unscaled, cast_restrictions, pay_cost,
    pay_restricted_cost, payable,
};
use crate::deck::simulator::model::{CardIdx, Format, SimCard, SimDeck};
use crate::deck::simulator::oracle_parser::{cost::parse_cost, land::parse_tap_yield};

/// Build one produced mana lot with exact allowed mana types.
fn lot(types: u8, restriction: Option<SpendRestriction>) -> ManaLot {
    ManaLot {
        amount: 1,
        types,
        restriction,
        cannot_pay_generic: false,
        one_color: false,
    }
}

#[test]
/// Prevent a two-color source from gaining other mana types.
fn green_blue_choice_cannot_pay_red_or_colorless() {
    let yield_ = parse_tap_yield("Add {G} or {U}.").expect("supported choice");
    let mut pool = ManaPool::default();
    add_yield(&yield_, &mut pool);
    for cost in ["{G}", "{U}", "{1}", "{G/U}"] {
        assert!(payable(&parse_cost(cost), &pool), "{cost}");
    }
    for cost in ["{R}", "{C}", "{W/B}", "{G}{U}"] {
        let before = pool.clone();
        assert!(!pay_cost(&parse_cost(cost), &mut pool), "{cost}");
        assert_eq!(pool, before);
    }
}

#[test]
/// Count a colored-or-colorless choice as one source output.
fn colorless_choice_is_one_mana_not_two() {
    let yield_ = parse_tap_yield("Add {C} or {G}.").expect("supported choice");
    assert_eq!(yield_.total(), 1);
    let mut pool = ManaPool::default();
    add_yield(&yield_, &mut pool);
    assert_eq!(pool.total(), 1);
    assert!(payable(&parse_cost("{C}"), &pool));
    assert!(payable(&parse_cost("{G}"), &pool));
    assert!(!payable(&parse_cost("{C}{G}"), &pool));
}

#[test]
/// Keep generic-cost resources out of hybrid color payments.
fn hybrid_requires_a_printed_component_color() {
    let mut pool = ManaPool {
        colorless: 1,
        generic_credit: 1,
        fixed: [0, 0, 0, 1, 0],
        ..ManaPool::default()
    };
    assert!(!pay_cost(&parse_cost("{W/U}"), &mut pool));
    pool.fixed[1] = 1;
    assert!(pay_cost(&parse_cost("{W/U}"), &mut pool));
    assert_eq!(pool.fixed, [0, 0, 0, 1, 0]);
    assert_eq!(pool.colorless, 1);
    assert_eq!(pool.generic_credit, 1);
}

#[test]
/// Accept a legal payment that requires changing an earlier assignment.
fn matching_reroutes_an_earlier_color_choice() {
    let mut pool = ManaPool {
        lots: vec![lot(0b00011, None), lot(0b00101, None)],
        ..ManaPool::default()
    };
    assert!(pay_cost(&parse_cost("{W}{U}"), &mut pool));
    assert_eq!(pool.total(), 0);
}

#[test]
/// Preserve both restrictions on a fixed-color restricted source.
fn restricted_fixed_mana_keeps_its_color_and_class() {
    let yield_ = parse_tap_yield("Add {U}. Spend this mana only to cast an artifact spell.")
        .expect("supported restricted mana");
    let mut pool = ManaPool::default();
    add_yield(&yield_, &mut pool);
    assert!(!payable(&parse_cost("{U}"), &pool));
    assert!(payment_plan(&parse_cost("{U}"), &pool, &[SpendRestriction::Artifact]).is_some());
    assert!(payment_plan(&parse_cost("{R}"), &pool, &[SpendRestriction::Artifact]).is_none());
    assert!(payment_plan(&parse_cost("{U}"), &pool, &[SpendRestriction::Creature]).is_none());
    assert!(pay_restricted_cost(
        &parse_cost("{U}"),
        &mut pool,
        &[SpendRestriction::Artifact],
    ));
    assert_eq!(pool.total(), 0);
}

#[test]
/// Allow real colorless mana within its supported cast restriction.
fn restricted_colorless_can_pay_explicit_colorless() {
    let yield_ = parse_tap_yield("Add {C}{C}. Spend this mana only to cast a creature spell.")
        .expect("supported restricted colorless mana");
    let mut pool = ManaPool::default();
    add_yield(&yield_, &mut pool);
    assert!(!payable(&parse_cost("{C}"), &pool));
    assert!(pay_restricted_cost(
        &parse_cost("{C}{1}"),
        &mut pool,
        &[SpendRestriction::Creature],
    ));
    assert_eq!(pool.total(), 0);
}

#[test]
/// Keep the opponent-color approximation distinct from actual mana types.
fn generic_credit_is_neither_colored_nor_colorless_mana() {
    let yield_ = parse_tap_yield(
        "Add one mana of any color that a land an opponent controls could produce.",
    )
    .expect("opponent-dependent source");
    let mut pool = ManaPool::default();
    add_yield_turns_unscaled(&yield_, &mut pool, 1);
    assert_eq!(pool.total(), 0);
    add_yield_turns_unscaled(&yield_, &mut pool, 2);
    assert_eq!(pool.generic_credit, 1);
    assert_eq!(pool.colorless, 0);
    for cost in ["{C}", "{G}", "{G/U}"] {
        assert!(!payable(&parse_cost(cost), &pool));
    }
    assert!(pay_cost(&parse_cost("{1}"), &mut pool));
    assert_eq!(pool.total(), 0);
}

#[test]
/// Honor a generic-payment ban without banning colored hybrid payments.
fn nongeneric_mana_covers_colored_symbols_but_not_generic() {
    let yield_ = parse_tap_yield("Add {W}{U}. This mana can't be spent to pay generic mana costs.")
        .expect("fixed nongeneric mana");
    let mut pool = ManaPool::default();
    add_yield(&yield_, &mut pool);
    assert!(!payable(&parse_cost("{1}"), &pool));
    assert!(pay_cost(&parse_cost("{W/U}{W/U}"), &mut pool));
    assert_eq!(pool.total(), 0);
    assert_eq!(pool.fixed_no_generic, [0; 5]);
}

#[test]
/// Reject incomplete payments and plans invalidated by a pool change.
fn failed_or_stale_plan_leaves_every_mana_resource_unchanged() {
    let mut pool = ManaPool {
        flexible: 1,
        lots: vec![lot(1, Some(SpendRestriction::Creature))],
        ..ManaPool::default()
    };
    let before = pool.clone();
    assert!(!pay_restricted_cost(
        &parse_cost("{W}{U}{C}"),
        &mut pool,
        &[SpendRestriction::Creature],
    ));
    assert_eq!(pool, before);
    let plan = payment_plan(&parse_cost("{1}"), &pool, &[]).expect("one available mana");
    pool.flexible += 1;
    let changed = pool.clone();
    assert!(!plan.commit(&mut pool));
    assert_eq!(pool, changed);
}

/// Keep colored mana available for the next spell when generic-only credit works.
#[test]
fn generic_payment_spends_credit_then_colorless_before_colored_mana() {
    let mut pool = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        generic_credit: 1,
        colorless: 1,
        ..ManaPool::default()
    };
    assert!(pay_cost(&parse_cost("{1}"), &mut pool));
    assert_eq!(pool.fixed[0], 1);
    assert_eq!(pool.generic_credit, 0);
    assert_eq!(pool.colorless, 1);
    assert!(pay_cost(&parse_cost("{1}"), &mut pool));
    assert_eq!(pool.fixed[0], 1);
    assert_eq!(pool.colorless, 0);
    assert!(pay_cost(&parse_cost("{W}"), &mut pool));
}

/// Reserve real colorless pips before spending spare mana on generic costs.
#[test]
fn generic_payment_does_not_take_mana_needed_for_other_symbols() {
    let mut pool = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        colorless: 1,
        ..ManaPool::default()
    };
    assert!(pay_cost(&parse_cost("{C}{1}"), &mut pool));
    assert_eq!(pool.total(), 0);
}

/// Use card types rather than a strategy label for spend restrictions.
#[test]
fn instant_sorcery_restriction_uses_the_actual_spell_type() {
    let spell = SimCard {
        is_instant_or_sorcery: true,
        ..SimCard::default()
    };
    assert!(cast_restrictions(&spell).contains(&SpendRestriction::InstantSorcery));
    let mut creature = SimCard {
        is_creature: true,
        ..SimCard::default()
    };
    creature.flags.is_interaction = true;
    assert!(!cast_restrictions(&creature).contains(&SpendRestriction::InstantSorcery));
    assert!(cast_restrictions(&creature).contains(&SpendRestriction::Creature));
}

/// Reject an impossible colorless payment before enumerating bundle colors.
#[test]
fn many_colored_bundles_reject_colorless_with_one_match() {
    let mut bundle = lot(COLORED, None);
    bundle.amount = 3;
    bundle.one_color = true;
    let mut pool = ManaPool {
        lots: vec![bundle; 9],
        ..ManaPool::default()
    };
    let before = pool.clone();
    MATCHER_CALLS.with(|calls| calls.set(0));
    assert!(!pay_cost(&parse_cost("{C}"), &mut pool));
    assert_eq!(pool, before);
    MATCHER_CALLS.with(|calls| assert_eq!(calls.get(), 1));
}

/// Reject relaxed-but-impossible payments without permuting identical sources.
#[test]
fn identical_bundles_prune_partial_states_and_color_permutations() {
    let mut bundle = lot(COLORED, None);
    bundle.amount = 3;
    bundle.one_color = true;
    let mut pool = ManaPool {
        lots: vec![bundle; 9],
        ..ManaPool::default()
    };
    let before = pool.clone();
    let cost = Cost {
        pips: [14, 13, 0, 0, 0],
        ..Cost::default()
    };
    MATCHER_CALLS.with(|calls| calls.set(0));
    assert!(!pay_cost(&cost, &mut pool));
    assert_eq!(pool, before);
    MATCHER_CALLS.with(|calls| assert!(calls.get() <= 128, "{} matches", calls.get()));
}

#[test]
/// Test both twobrid choices, including competition for matching colors.
fn twobrid_accepts_one_matching_color_or_two_generic() {
    let cost = parse_cost("{2/W}");
    let mut white = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        ..ManaPool::default()
    };
    assert!(pay_cost(&cost, &mut white));
    assert_eq!(white.total(), 0);
    let mut blue = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        generic_credit: 1,
        ..ManaPool::default()
    };
    assert!(pay_cost(&cost, &mut blue));
    assert_eq!(blue.total(), 0);
    assert!(!payable(
        &cost,
        &ManaPool {
            colorless: 1,
            ..ManaPool::default()
        }
    ));
    let mut reserved_white = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        colorless: 2,
        ..ManaPool::default()
    };
    assert!(pay_cost(&parse_cost("{W}{2/W}"), &mut reserved_white));
    assert_eq!(reserved_white.total(), 0);
}

#[test]
/// Retain a source's common chosen color after a partial payment.
fn multi_mana_single_color_choice_cannot_split_colors() {
    let yield_ = parse_tap_yield("Add three mana of any one color.").expect("single-color mana");
    let mut pool = ManaPool::default();
    add_yield(&yield_, &mut pool);
    assert_eq!(pool.total(), 3);
    assert!(payable(&parse_cost("{U}{U}{U}"), &pool));
    assert!(!payable(&parse_cost("{U}{G}"), &pool));
    assert!(pay_cost(&parse_cost("{U}"), &mut pool));
    assert_eq!(pool.total(), 2);
    assert!(!payable(&parse_cost("{G}"), &pool));
    assert!(payable(&parse_cost("{U}{U}"), &pool));
}

#[test]
/// Distinguish independent color choices from a single-color bundle.
fn arbitrary_color_combinations_can_split_colors() {
    let yield_ = parse_tap_yield("Add three mana in any combination of colors.")
        .expect("independent color choices");
    let mut pool = ManaPool::default();
    add_yield(&yield_, &mut pool);
    assert!(pay_cost(&parse_cost("{U}{G}{R}"), &mut pool));
    assert_eq!(pool.total(), 0);
}

#[test]
/// Keep unsupported costs and source wording from granting free payment.
fn unsupported_snow_symbols_and_bundle_choices_fail_closed() {
    let pool = ManaPool {
        flexible: 10,
        ..ManaPool::default()
    };
    assert!(!payable(&parse_cost("{S}"), &pool));
    assert!(!payable(&parse_cost("{G/U/P}"), &pool));
    assert!(!payable(&parse_cost("{unknown}"), &pool));
    assert!(parse_tap_yield("Add {G}{G} or {U}{U}.").is_none());
    assert!(parse_tap_yield("Add some mana of any one color.").is_none());
    assert!(
        parse_tap_yield("Add {U}. Spend this mana only to cast artifact creature spells.")
            .is_none()
    );
    assert!(
        parse_tap_yield(
            "Add {G}. Spend this mana only to cast creature spells of the chosen type."
        )
        .is_none()
    );
}

/// Make a battlefield from explicit card traits for conditional mana tests.
fn battlefield(cards: Vec<SimCard>) -> (SimDeck, Vec<crate::deck::simulator::game::Permanent>) {
    let deck = SimDeck {
        cards,
        commanders: Vec::new(),
        companion: None,
        format: Format::Constructed,
        rules: crate::deck::simulator::format::rules_for("constructed"),
    };
    let permanents = (0..deck.cards.len())
        .map(|index| new_perm_with(index as u32, &deck, CardIdx(index as u32), 1, false))
        .collect();
    (deck, permanents)
}

#[test]
/// Separate Mox Amber's output quantity from its eligible color choices.
fn mox_amber_produces_one_mana_only_from_eligible_colors() {
    let yield_ = parse_tap_yield(
        "Add one mana of any color among legendary creatures and planeswalkers you control.",
    )
    .expect("legendary creature or planeswalker colors");
    let (deck, board) = battlefield(vec![
        SimCard {
            is_legendary: true,
            is_creature: true,
            colors: [true, true, false, false, false],
            ..SimCard::default()
        },
        SimCard {
            is_legendary: true,
            is_planeswalker: true,
            colors: [false, false, false, false, true],
            ..SimCard::default()
        },
        SimCard {
            is_creature: true,
            colors: [false, false, false, true, false],
            ..SimCard::default()
        },
        SimCard {
            is_legendary: true,
            is_artifact: true,
            colors: [false, false, true, false, false],
            ..SimCard::default()
        },
    ]);
    let mut pool = ManaPool::default();
    add_yield_turns(&deck, &yield_, &mut pool, 2, &board);
    assert_eq!(pool.total(), 1);
    for cost in ["{W}", "{U}", "{G}"] {
        assert!(payable(&parse_cost(cost), &pool));
    }
    for cost in ["{B}", "{R}", "{C}", "{W}{U}"] {
        assert!(!payable(&parse_cost(cost), &pool));
    }
    let mut ineligible = ManaPool::default();
    add_yield_turns(&deck, &yield_, &mut ineligible, 2, &board[2..]);
    assert_eq!(ineligible.total(), 0);
}

#[test]
/// Preserve each color produced by a supported color-counting source.
fn colors_present_produces_fixed_present_colors() {
    let yield_ =
        parse_tap_yield("For each color among permanents you control, add one mana of that color.")
            .expect("permanent color scaling");
    let (deck, board) = battlefield(vec![SimCard {
        colors: [true, false, false, false, true],
        ..SimCard::default()
    }]);
    let mut pool = ManaPool::default();
    add_yield_turns(&deck, &yield_, &mut pool, 2, &board);
    assert_eq!(pool.fixed, [1, 0, 0, 0, 1]);
    assert_eq!(pool.flexible, 0);
    assert!(!payable(&parse_cost("{U}"), &pool));
}

/// Try every individual assignment without using the production matcher.
fn reference_payment(demands: &[u8], mana: &[(u8, bool)]) -> bool {
    let Some((&demand, rest)) = demands.split_first() else {
        return true;
    };
    for (index, &(types, generic)) in mana.iter().enumerate() {
        if if demand == 0 {
            !generic
        } else {
            types & demand == 0
        } {
            continue;
        }
        let mut remaining = mana.to_vec();
        remaining.remove(index);
        if reference_payment(rest, &remaining) {
            return true;
        }
    }
    false
}

#[test]
/// Compare matching and resource consumption against all small assignments.
fn small_payment_cases_match_an_independent_exhaustive_solver() {
    let costs: &[(&str, &[u8])] = &[
        ("{W}", &[1]),
        ("{U}", &[2]),
        ("{C}", &[32]),
        ("{W}{U}", &[1, 2]),
        ("{W/U}{U/B}", &[3, 6]),
        ("{W}{W/U}{1}", &[1, 3, 0]),
        ("{C}{W/U}{1}", &[32, 3, 0]),
        ("{3}", &[0, 0, 0]),
        ("{W}{U}{B}{1}", &[1, 2, 4, 0]),
    ];
    let masks = [1, 2, 3, 4, 5, 6, 7, 32];
    for first in masks {
        for second in masks {
            for third in masks {
                for no_generic in [false, true] {
                    let mut lots = vec![lot(first, None), lot(second, None), lot(third, None)];
                    lots[0].cannot_pay_generic = no_generic;
                    let pool = ManaPool {
                        lots,
                        generic_credit: 1,
                        ..ManaPool::default()
                    };
                    let mana = [
                        (first, !no_generic),
                        (second, true),
                        (third, true),
                        (0, true),
                    ];
                    for &(text, demands) in costs {
                        let expected = reference_payment(demands, &mana);
                        let mut after = pool.clone();
                        assert_eq!(
                            pay_cost(&parse_cost(text), &mut after),
                            expected,
                            "{text}, {first}/{second}/{third}, no_generic={no_generic}"
                        );
                        if expected {
                            assert_eq!(after.total(), pool.total() - demands.len() as u32);
                        } else {
                            assert_eq!(after, pool);
                        }
                    }
                }
            }
        }
    }
}
