//! Empty-cost payment equivalence, resource cleanup, and matcher work checks.

use super::super::ManaLot;
use super::*;

/// Virtual resources include generic-only, colored, and masked-out type choices.
const VIRTUAL_SOURCE_CASES: [&[u8]; 4] = [
    &[],
    &[0],
    &[0b00011],
    &[COLORLESS, COLORED, 0, 0b10000000, u8::MAX],
];

/// Prepare payment through the original matcher, without the empty-cost shortcut.
fn slow_payment_plan(
    cost: &Cost,
    pool: &ManaPool,
    classes: &[SpendRestriction],
    virtual_sources: &[u8],
) -> Option<(ManaPayment, Vec<usize>)> {
    if cost.snow > 0 || cost.unsupported || cost.hybrid[0] > 0 {
        return None;
    }
    let mut supplies = supplies(pool, classes)?;
    supplies.splice(
        0..0,
        virtual_sources
            .iter()
            .enumerate()
            .map(|(index, types)| Supply {
                amount: 1,
                types: *types & COLORED,
                generic: true,
                origin: Origin::Virtual(index),
                one_color: false,
            }),
    );
    let mut chosen = *cost;
    chosen.twobrid = [0; 5];
    choose_twobrid(cost, &chosen, &mut supplies, pool, 0)
}

/// Compare every prepared pool field, virtual source identity, and committed result.
fn assert_same_payment(
    expected: Option<(ManaPayment, Vec<usize>)>,
    actual: Option<(ManaPayment, Vec<usize>)>,
    pool: &ManaPool,
) -> bool {
    match (expected, actual) {
        (Some((expected, expected_used)), Some((actual, actual_used))) => {
            assert_eq!(expected.before, *pool);
            assert_eq!(actual.before, expected.before);
            assert_eq!(actual.after, expected.after);
            assert_eq!(actual_used, expected_used);
            let mut committed = pool.clone();
            assert!(actual.commit(&mut committed));
            assert_eq!(committed, expected.after);
            true
        }
        (None, None) => false,
        (Some(_), None) => panic!("payment rejected a pool accepted by the slow matcher: {pool:?}"),
        (None, Some(_)) => panic!("payment accepted a pool rejected by the slow matcher: {pool:?}"),
    }
}

/// Check empty payment against the slow matcher and require zero matching work.
fn assert_empty_matches_slow(
    pool: &ManaPool,
    classes: &[SpendRestriction],
    virtual_sources: &[u8],
) -> bool {
    let before = pool.clone();
    let cost = Cost::default();
    let expected = slow_payment_plan(&cost, pool, classes, virtual_sources);
    MATCHER_CALLS.with(|calls| calls.set(0));
    let actual = payment_plan_with_virtual_sources(&cost, pool, classes, virtual_sources);
    assert_eq!(MATCHER_CALLS.with(|calls| calls.get()), 0, "{pool:?}");
    assert_eq!(*pool, before);
    if let Some((_, used)) = &actual {
        assert!(used.is_empty());
    }
    assert_same_payment(expected, actual, pool)
}

/// Exercise each cast class alone and in every supported combination.
fn cast_class_sets() -> Vec<Vec<SpendRestriction>> {
    let classes = [
        SpendRestriction::Creature,
        SpendRestriction::Artifact,
        SpendRestriction::Legendary,
        SpendRestriction::InstantSorcery,
    ];
    (0usize..16)
        .map(|mask| {
            classes
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, class)| *class)
                .collect()
        })
        .collect()
}

/// Vary all scalar resources, fixed colors, and valid nongeneric portions.
fn generated_pool(seed: usize) -> ManaPool {
    let amounts = [0, 1, 3, u32::MAX];
    let fixed = std::array::from_fn(|color| amounts[(seed + color) % 4]);
    let fixed_no_generic = std::array::from_fn(|color| match (seed / 4 + color) % 3 {
        0 => 0,
        1 => fixed[color] / 2,
        _ => fixed[color],
    });
    ManaPool {
        fixed,
        fixed_no_generic,
        flexible: amounts[seed % 4],
        colorless: amounts[(seed / 2) % 4],
        generic_credit: amounts[(seed / 4) % 4],
        treasures: amounts[(seed / 8) % 4],
        lots: Vec::new(),
    }
}

/// Keep all resource fields and mixed zero-amount lots present in one fixture.
fn populated_pool() -> ManaPool {
    ManaPool {
        fixed: [3, 4, 5, 6, 7],
        fixed_no_generic: [0, 1, 2, 3, 4],
        flexible: 3,
        colorless: 4,
        generic_credit: 5,
        treasures: 6,
        lots: vec![
            ManaLot {
                amount: 3,
                types: COLORED | COLORLESS,
                restriction: None,
                cannot_pay_generic: false,
                one_color: true,
            },
            ManaLot {
                amount: 0,
                types: 0,
                restriction: Some(SpendRestriction::Artifact),
                cannot_pay_generic: true,
                one_color: true,
            },
            ManaLot {
                amount: 2,
                types: 0b00110,
                restriction: Some(SpendRestriction::Legendary),
                cannot_pay_generic: true,
                one_color: false,
            },
            ManaLot {
                amount: 0,
                types: COLORLESS,
                restriction: None,
                cannot_pay_generic: false,
                one_color: false,
            },
        ],
    }
}

/// Include every restriction and spend rule for empty, typed, and untyped lots.
fn generated_lots() -> Vec<ManaLot> {
    let mut lots = Vec::new();
    for amount in [0, 1, 3] {
        for types in [0, 1, 3, 16, COLORED, COLORLESS, COLORED | COLORLESS] {
            for restriction in [
                None,
                Some(SpendRestriction::Creature),
                Some(SpendRestriction::Artifact),
                Some(SpendRestriction::Legendary),
                Some(SpendRestriction::InstantSorcery),
            ] {
                for cannot_pay_generic in [false, true] {
                    for one_color in [false, true] {
                        lots.push(ManaLot {
                            amount,
                            types,
                            restriction,
                            cannot_pay_generic,
                            one_color,
                        });
                    }
                }
            }
        }
    }
    lots
}

/// Preserve all pool fields without entering the matcher, including large counts.
#[test]
fn empty_cost_matches_slow_for_generated_pool_fields() {
    let class_sets = cast_class_sets();
    let mut cases = 0;
    for pool in std::iter::once(ManaPool::default()).chain((0..64).map(generated_pool)) {
        for classes in &class_sets {
            for sources in VIRTUAL_SOURCE_CASES {
                assert!(assert_empty_matches_slow(&pool, classes, sources));
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 4_160);
}

/// Keep bundle validity and cast eligibility identical to the slow matcher.
#[test]
fn empty_cost_matches_slow_for_generated_lots_and_classes() {
    let class_sets = cast_class_sets();
    let mut outcomes = [0; 2];
    for lot in generated_lots() {
        let mut pool = populated_pool();
        pool.lots.push(lot);
        for classes in &class_sets {
            for sources in VIRTUAL_SOURCE_CASES {
                let accepted = assert_empty_matches_slow(&pool, classes, sources);
                outcomes[usize::from(accepted)] += 1;
            }
        }
    }
    assert_eq!(outcomes, [768, 26_112]);
}

/// Reject invalid fixed-mana subsets before granting an empty payment.
#[test]
fn empty_cost_rejects_malformed_nongeneric_fixed_mana() {
    for base in [ManaPool::default(), populated_pool()] {
        for color in 0..5 {
            let mut pool = base.clone();
            pool.fixed_no_generic[color] = pool.fixed[color] + 1;
            for sources in VIRTUAL_SOURCE_CASES {
                assert!(!assert_empty_matches_slow(&pool, &[], sources));
            }
        }
    }
}

/// Remove only zero-amount lots, including ineligible and invalid empty bundles.
#[test]
fn empty_payment_commits_exact_cleanup_without_spending_resources() {
    let mut pool = populated_pool();
    let before = pool.clone();
    let mut expected = before.clone();
    expected.lots = vec![before.lots[0].clone(), before.lots[2].clone()];
    let (payment, used) =
        payment_plan_with_virtual_sources(&Cost::default(), &pool, &[], &[0, COLORED, COLORLESS])
            .expect("empty payment ignores zero-amount untyped bundles");

    assert_eq!(pool, before);
    assert_eq!(payment.before, before);
    assert_eq!(payment.after, expected);
    assert!(used.is_empty());
    assert!(payment.commit(&mut pool));
    assert_eq!(pool, expected);
    assert!(pool.lots[0].one_color);
    assert_eq!(pool.lots[0].types, COLORED | COLORLESS);
}

/// Reject a stale empty plan without applying any cleanup or other pool change.
fn assert_stale_empty_plan(pool: &ManaPool, mut stale: ManaPool) {
    let (payment, used) =
        payment_plan_with_virtual_sources(&Cost::default(), pool, &[], &[COLORED])
            .expect("the unchanged pool accepts an empty payment");
    let before_commit = stale.clone();
    assert_ne!(stale, *pool);
    assert!(used.is_empty());
    assert!(!payment.commit(&mut stale));
    assert_eq!(stale, before_commit);
}

/// Require every resource field and every lot attribute to match preparation.
#[test]
fn empty_payment_rejects_stale_scalar_resources_and_lots() {
    let pool = populated_pool();
    for color in 0..5 {
        let mut changed = pool.clone();
        changed.fixed[color] += 1;
        assert_stale_empty_plan(&pool, changed);
        let mut changed = pool.clone();
        changed.fixed_no_generic[color] += 1;
        assert_stale_empty_plan(&pool, changed);
    }
    let changes: [fn(&mut ManaPool); 12] = [
        |pool| pool.flexible += 1,
        |pool| pool.colorless += 1,
        |pool| pool.generic_credit += 1,
        |pool| pool.treasures += 1,
        |pool| pool.lots[0].amount += 1,
        |pool| pool.lots[0].types ^= 1,
        |pool| pool.lots[0].restriction = Some(SpendRestriction::Creature),
        |pool| pool.lots[0].cannot_pay_generic = true,
        |pool| pool.lots[0].one_color = false,
        |pool| pool.lots.swap(0, 2),
        |pool| {
            pool.lots.remove(1);
        },
        |pool| pool.lots.push(pool.lots[1].clone()),
    ];
    for change in changes {
        let mut changed = pool.clone();
        change(&mut changed);
        assert_stale_empty_plan(&pool, changed);
    }
}

/// Reject unsupported, snow, and invalid hybrid costs before any matching work.
#[test]
fn unsupported_cost_guards_do_not_grant_empty_payment() {
    let mut hybrid_zero = Cost::default();
    hybrid_zero.hybrid[0] = 1;
    let costs = [
        Cost {
            unsupported: true,
            ..Cost::default()
        },
        Cost {
            snow: 1,
            ..Cost::default()
        },
        hybrid_zero,
    ];
    for cost in costs {
        assert_ne!(cost, Cost::default());
        for pool in [ManaPool::default(), populated_pool()] {
            for sources in VIRTUAL_SOURCE_CASES {
                MATCHER_CALLS.with(|calls| calls.set(0));
                assert!(payment_plan_with_virtual_sources(&cost, &pool, &[], sources).is_none());
                assert_eq!(MATCHER_CALLS.with(|calls| calls.get()), 0);
            }
        }
    }
    assert_eq!(costs[0].total(), 0);
}

/// Cover nondefault mana demands, unresolved X, and Phyrexian metadata.
fn nondefault_costs() -> Vec<Cost> {
    let mut costs = vec![
        Cost {
            generic: 1,
            ..Cost::default()
        },
        Cost {
            colorless: 1,
            ..Cost::default()
        },
        Cost {
            x_symbols: 1,
            ..Cost::default()
        },
    ];
    for color in 0..5 {
        let mut cost = Cost::default();
        cost.pips[color] = 1;
        costs.push(cost);
        let mut cost = Cost::default();
        cost.twobrid[color] = 1;
        costs.push(cost);
        let mut cost = Cost::default();
        cost.phyrexian[color] = 1;
        costs.push(cost);
    }
    for mask in 1..32 {
        let mut cost = Cost::default();
        cost.hybrid[mask] = 1;
        costs.push(cost);
    }
    costs
}

/// Reject capacity shortages without matching and preserve all reference results.
#[test]
fn nondefault_costs_match_reference_or_reject_capacity_shortages() {
    let costs = nondefault_costs();
    assert_eq!(costs.len(), 49);
    assert_eq!(costs[2].total(), 0);
    for cost in costs {
        assert_ne!(cost, Cost::default());
        for pool in [ManaPool::default(), populated_pool()] {
            for sources in VIRTUAL_SOURCE_CASES {
                let before = pool.clone();
                let expected = slow_payment_plan(&cost, &pool, &[], sources);
                MATCHER_CALLS.with(|calls| calls.set(0));
                let actual = payment_plan_with_virtual_sources(&cost, &pool, &[], sources);
                let requires_mana = cost.generic > 0
                    || cost.colorless > 0
                    || cost
                        .pips
                        .iter()
                        .chain(&cost.hybrid)
                        .chain(&cost.twobrid)
                        .any(|count| *count > 0);
                let calls = MATCHER_CALLS.with(|calls| calls.get());
                if requires_mana && pool == ManaPool::default() && sources.is_empty() {
                    assert_eq!(calls, 0, "{cost:?}");
                } else {
                    assert!(calls > 0, "{cost:?}");
                }
                assert_eq!(pool, before);
                assert_same_payment(expected, actual, &pool);
            }
        }
    }
}
