use super::*;
use crate::deck::simulator::game_mana::ManaLot;
use crate::deck::simulator::oracle_parser::cost::parse_cost;

/// Make one physical source with explicit costs for each allowed mana type.
fn priced(types: &[(usize, u32)]) -> [Option<u32>; 6] {
    let mut costs = [None; 6];
    for (color, cost) in types {
        costs[*color] = Some(*cost);
    }
    costs
}

/// Measure only the actual physical modes returned by a prepared payment.
fn source_cost(costs: &[[Option<u32>; 6]], used: &[(usize, u8)]) -> u64 {
    used.iter()
        .map(|(source, types)| {
            u64::from(
                costs[*source][types.trailing_zeros() as usize]
                    .expect("a selected actual type has a cost"),
            )
        })
        .sum()
}

/// Choosing the cheaper source does not depend on its battlefield position.
#[test]
fn source_cost_matching_prefers_the_cheapest_damage_in_both_orders() {
    let cheap = priced(&[(1, 1), (5, 0)]);
    let expensive = priced(&[(1, 3), (5, 0)]);
    for costs in [[cheap, expensive], [expensive, cheap]] {
        let (_, used) =
            payment_plan_with_source_costs(&parse_cost("{U}"), &ManaPool::default(), &[], &costs)
                .expect("a blue physical source can pay");
        assert_eq!(used.len(), 1);
        assert_eq!(source_cost(&costs, &used), 1);
        assert_eq!(used[0].1, 2);
    }
}

/// Residual paths can move an earlier assignment instead of losing a required color.
#[test]
fn source_cost_matching_reroutes_overlapping_color_choices() {
    let costs = [priced(&[(0, 0), (1, 0)]), priced(&[(0, 1)])];
    let (_, used) =
        payment_plan_with_source_costs(&parse_cost("{W}{U}"), &ManaPool::default(), &[], &costs)
            .expect("the free flexible source pays blue and the other source pays white");
    assert_eq!(used, [(0, 2), (1, 1)]);
    assert_eq!(source_cost(&costs, &used), 1);
}

/// Generic payments choose painless types while colored payments use the cheaper source.
#[test]
fn source_cost_matching_prices_actual_types_not_just_sources() {
    let costs = [priced(&[(1, 3), (5, 0)]), priced(&[(1, 1), (5, 0)])];
    let (_, used) =
        payment_plan_with_source_costs(&parse_cost("{1}{U}"), &ManaPool::default(), &[], &costs)
            .expect("one colorless and one blue mode pay the complete spell");
    assert_eq!(used, [(0, 32), (1, 2)]);
    assert_eq!(source_cost(&costs, &used), 1);
    for cost in ["{C}", "{1}"] {
        let (_, used) =
            payment_plan_with_source_costs(&parse_cost(cost), &ManaPool::default(), &[], &costs)
                .expect("a painless colorless mode is available");
        assert_eq!(source_cost(&costs, &used), 0);
        assert!(used.iter().all(|(_, types)| *types == 32));
    }
}

/// Twobrid symbol choices compare damage instead of stopping at the first payable half.
#[test]
fn source_cost_matching_can_prefer_two_existing_generic_mana() {
    let mut pool = ManaPool {
        colorless: 2,
        ..Default::default()
    };
    let (payment, used) =
        payment_plan_with_source_costs(&parse_cost("{2/U}"), &pool, &[], &[priced(&[(1, 3)])])
            .expect("two produced colorless mana avoid a painful blue mode");
    assert!(used.is_empty());
    assert!(payment.commit(&mut pool));
    assert_eq!(pool.total(), 0);
}

/// A multi-mana single-color lot cannot become a free mixed-color bundle.
#[test]
fn source_cost_matching_preserves_single_color_pool_bundles() {
    let pool = ManaPool {
        lots: vec![ManaLot {
            amount: 2,
            types: 3,
            restriction: None,
            cannot_pay_generic: false,
            one_color: true,
        }],
        ..Default::default()
    };
    let costs = [priced(&[(0, 1), (1, 1)])];
    let (payment, used) = payment_plan_with_source_costs(&parse_cost("{W}{U}"), &pool, &[], &costs)
        .expect("one lot color and one separate physical source can pay");
    assert_eq!(source_cost(&costs, &used), 1);
    let mut after = pool;
    assert!(payment.commit(&mut after));
    assert_eq!(after.total(), 1);
    assert_eq!(after.lots[0].types.count_ones(), 1);
}

/// A stale priced payment cannot consume changed pool resources.
#[test]
fn source_cost_matching_keeps_atomic_pool_commit_and_single_use() {
    let costs = [priced(&[(1, 1), (5, 0)])];
    assert!(
        payment_plan_with_source_costs(&parse_cost("{U}{C}"), &ManaPool::default(), &[], &costs,)
            .is_none()
    );
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let (payment, _) = payment_plan_with_source_costs(&parse_cost("{1}{U}"), &pool, &[], &costs)
        .expect("one produced mana and one physical source pay");
    pool.colorless = 2;
    let before = pool.clone();
    assert!(!payment.commit(&mut pool));
    assert_eq!(pool, before);
}

/// Enumerate small physical assignments independently of the priced flow algorithm.
fn reference_minimum(
    cost: &Cost,
    pool: &ManaPool,
    costs: &[[Option<u32>; 6]],
    choices: &mut [u8],
    source: usize,
) -> Option<(u64, usize)> {
    if source == choices.len() {
        return payment_plan_with_source_choices(cost, pool, &[], choices)
            .map(|(_, used)| (source_cost(costs, &used), used.len()));
    }
    choices[source] = 0;
    let mut best = reference_minimum(cost, pool, costs, choices, source + 1);
    for (color, price) in costs[source].iter().enumerate() {
        if price.is_none() {
            continue;
        }
        choices[source] = 1 << color;
        if let Some(candidate) = reference_minimum(cost, pool, costs, choices, source + 1) {
            best = Some(best.map_or(candidate, |held| held.min(candidate)));
        }
    }
    choices[source] = 0;
    best
}

/// Compare mixed costs against exhaustive small assignments rather than fixed examples alone.
#[test]
fn source_cost_matching_agrees_with_exhaustive_physical_choices() {
    let costs = [
        priced(&[(0, 2), (1, 1), (5, 0)]),
        priced(&[(0, 0), (1, 3)]),
        priced(&[(1, 0), (5, 2)]),
    ];
    let mut pools = vec![
        ManaPool::default(),
        ManaPool {
            colorless: 1,
            ..Default::default()
        },
    ];
    pools.push(ManaPool {
        fixed: [1, 0, 0, 0, 0],
        lots: vec![ManaLot {
            amount: 1,
            types: 3,
            restriction: None,
            cannot_pay_generic: false,
            one_color: false,
        }],
        ..Default::default()
    });
    for pool in pools {
        for raw in [
            "{0}", "{W}", "{U}", "{C}", "{1}{U}", "{W}{U}", "{W/U}{C}", "{2/U}{W}", "{4}",
        ] {
            let cost = parse_cost(raw);
            let expected = reference_minimum(&cost, &pool, &costs, &mut [0; 3], 0);
            let actual = payment_plan_with_source_costs(&cost, &pool, &[], &costs)
                .map(|(_, used)| (source_cost(&costs, &used), used.len()));
            assert_eq!(actual, expected, "{raw} with pool {pool:?}");
        }
    }
}

/// Free residual rerouting must beat tapping another physical source.
#[test]
fn equal_damage_matching_uses_only_existing_pool_when_rerouting_is_enough() {
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
    let (payment, used) =
        payment_plan_with_source_costs(&parse_cost("{W}{U}"), &pool, &[], &[priced(&[(1, 0)])])
            .expect("the pool alone can reroute the flexible mana to blue");
    assert!(used.is_empty());
    assert!(payment.commit(&mut pool));
    assert_eq!(pool.total(), 0);
}

/// Avoiding damage takes priority over reducing the number of source activations.
#[test]
fn life_loss_has_priority_over_the_number_of_new_activations() {
    let costs = [priced(&[(1, 1)]), priced(&[(5, 0)]), priced(&[(5, 0)])];
    let (_, used) =
        payment_plan_with_source_costs(&parse_cost("{2/U}"), &ManaPool::default(), &[], &costs)
            .expect("two painless sources avoid one damaging activation");
    assert_eq!(source_cost(&costs, &used), 0);
    assert_eq!(used, [(1, 32), (2, 32)]);
}
