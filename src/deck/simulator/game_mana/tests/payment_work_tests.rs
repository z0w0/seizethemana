use super::*;
use crate::deck::simulator::game_mana::ManaLot;
use crate::deck::simulator::oracle_parser::cost::parse_cost;

/// Large impossible alternatives stop before entering the unpriced matcher.
#[test]
fn huge_twobrid_shortages_skip_matching_for_both_payment_routes() {
    let cost = Cost {
        twobrid: [100; 5],
        ..Cost::default()
    };
    let pool = ManaPool::default();
    MATCHER_CALLS.with(|calls| calls.set(0));
    assert!(payment_plan(&cost, &pool, &[]).is_none());
    assert_eq!(MATCHER_CALLS.with(|calls| calls.get()), 0);
    assert!(payment_plan_with_cast_sources(&cost, &pool, &[], &[], &[]).is_none());
    assert_eq!(pool, ManaPool::default());
}

/// Each colored twobrid half needs one mana, not its printed mana value of two.
#[test]
fn large_colored_twobrid_payment_keeps_every_color_available() {
    let cost = Cost {
        twobrid: [100; 5],
        ..Cost::default()
    };
    let mut pool = ManaPool {
        fixed: [100; 5],
        ..ManaPool::default()
    };
    let payment = payment_plan(&cost, &pool, &[]).expect("one matching mana pays each twobrid");
    let (priced, used) = payment_plan_with_cast_sources(&cost, &pool, &[], &[], &[])
        .expect("priced matching keeps the same colored twobrid payment");
    assert!(used.is_empty());
    assert_eq!(priced.after, payment.after);
    assert!(payment.commit(&mut pool));
    assert_eq!(pool, ManaPool::default());
}

/// Generic alternatives still spend two mana when their colored halves cannot be paid.
#[test]
fn generic_twobrid_payment_keeps_its_full_mana_cost() {
    let cost = parse_cost("{2/W}{2/U}");
    for amount in [2, 3, 4] {
        let mut pool = ManaPool {
            generic_credit: amount,
            ..ManaPool::default()
        };
        let before = pool.clone();
        let payment = payment_plan(&cost, &pool, &[]);
        let priced = payment_plan_with_cast_sources(&cost, &pool, &[], &[], &[]);
        assert_eq!(
            payment.as_ref().map(|payment| &payment.after),
            priced.as_ref().map(|(payment, _)| &payment.after)
        );
        if let Some((_, used)) = priced {
            assert!(used.is_empty());
        }
        match payment {
            Some(payment) => {
                assert_eq!(amount, 4);
                assert!(payment.commit(&mut pool));
                assert_eq!(pool, ManaPool::default());
            }
            None => {
                assert!(amount < 4);
                assert_eq!(pool, before);
            }
        }
    }
}

/// Ineligible lots cannot keep an impossible alternative search running.
#[test]
fn capacity_counts_only_mana_eligible_for_the_cast_class() {
    let pool = ManaPool {
        lots: vec![ManaLot {
            amount: 500,
            types: COLORED,
            restriction: Some(SpendRestriction::Artifact),
            cannot_pay_generic: false,
            one_color: false,
        }],
        ..ManaPool::default()
    };
    let huge = Cost {
        twobrid: [100; 5],
        ..Cost::default()
    };
    MATCHER_CALLS.with(|calls| calls.set(0));
    assert!(payment_plan(&huge, &pool, &[]).is_none());
    assert_eq!(MATCHER_CALLS.with(|calls| calls.get()), 0);
    assert!(payment_plan_with_cast_sources(&huge, &pool, &[], &[], &[]).is_none());
    let mut after = pool.clone();
    let payment = payment_plan(&parse_cost("{2/W}"), &pool, &[SpendRestriction::Artifact])
        .expect("an eligible lot can pay the colored half");
    assert!(payment.commit(&mut after));
    assert_eq!(after.lots[0].amount, 499);
    assert_eq!(pool.lots[0].amount, 500);
}

/// Virtual sources contribute one unit each without becoming produced mana.
#[test]
fn virtual_capacity_keeps_colored_and_generic_twobrid_payments() {
    let pool = ManaPool::default();
    for (raw, sources) in [("{2/W}{U}", &[1, 2][..]), ("{2/W}{2/U}", &[0, 0, 0, 0][..])] {
        let (payment, used) =
            payment_plan_with_virtual_sources(&parse_cost(raw), &pool, &[], sources)
                .expect("matching virtual resources cover the complete payment");
        assert_eq!(used, (0..sources.len()).collect::<Vec<_>>());
        let mut after = pool.clone();
        assert!(payment.commit(&mut after));
        assert_eq!(after, pool);
    }
}

/// A shared mana and cast-tap slot cannot contribute two units to the lower bound.
#[test]
fn priced_capacity_counts_each_physical_source_once() {
    let pool = ManaPool::default();
    let mut costs = [[None; 6]; 2];
    costs[0][0] = Some(0);
    costs[1][1] = Some(1);
    let taps = [Some(1), None];
    let cost = parse_cost("{2/W}{U}");
    assert!(payment_plan_with_cast_sources(&cost, &pool, &[], &costs[..1], &taps[..1]).is_none());
    let (payment, used) = payment_plan_with_cast_sources(&cost, &pool, &[], &costs, &taps)
        .expect("one virtual white payment and one actual blue activation cover the cost");
    assert_eq!(used, [(1, 2)]);
    assert_eq!(payment.source_cost, (1, 1));
    let mut after = pool.clone();
    assert!(payment.commit(&mut after));
    assert_eq!(after, pool);
}

/// Phyrexian life remains outside both produced-mana and physical-source matching.
#[test]
fn capacity_does_not_count_phyrexian_symbols_as_mana() {
    for raw in ["{W/P}{U/P}", "{W/P}{U/P}{2/U}"] {
        let cost = parse_cost(raw);
        let pool = ManaPool {
            fixed: [0, u32::from(cost.twobrid[1]), 0, 0, 0],
            ..ManaPool::default()
        };
        let payment = payment_plan(&cost, &pool, &[])
            .expect("callers pay Phyrexian life outside the mana payment");
        let (priced, used) = payment_plan_with_cast_sources(&cost, &pool, &[], &[], &[])
            .expect("priced matching also leaves Phyrexian life to callers");
        assert!(used.is_empty());
        assert_eq!(payment.after, ManaPool::default());
        assert_eq!(priced.after, payment.after);
    }
}

/// Wide totals preserve payments whose combined demand exceeds one u32 field.
#[test]
fn capacity_uses_wide_totals_for_both_supply_and_demand() {
    let pool = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        generic_credit: u32::MAX,
        ..ManaPool::default()
    };
    let mut cost = Cost {
        generic: u32::MAX,
        pips: [1, 0, 0, 0, 0],
        ..Cost::default()
    };
    let payment = payment_plan(&cost, &pool, &[])
        .expect("separate supply groups cover more than u32::MAX total mana");
    assert_eq!(payment.after, ManaPool::default());
    let (priced, used) = payment_plan_with_cast_sources(&cost, &pool, &[], &[], &[])
        .expect("priced matching also uses wide total demand");
    assert!(used.is_empty());
    assert_eq!(priced.after, payment.after);
    cost.pips[0] = 2;
    MATCHER_CALLS.with(|calls| calls.set(0));
    assert!(payment_plan(&cost, &pool, &[]).is_none());
    assert_eq!(MATCHER_CALLS.with(|calls| calls.get()), 0);
}

/// Every fixed mana group contributes to the lower bound without using printed mana value.
#[test]
fn capacity_includes_generic_colored_hybrid_colorless_and_twobrid_demand() {
    let cost = parse_cost("{1}{W}{W/U}{C}{2/W}");
    let short = ManaPool {
        generic_credit: 4,
        ..ManaPool::default()
    };
    MATCHER_CALLS.with(|calls| calls.set(0));
    assert!(payment_plan(&cost, &short, &[]).is_none());
    assert_eq!(MATCHER_CALLS.with(|calls| calls.get()), 0);
    let pool = ManaPool {
        fixed: [3, 0, 0, 0, 0],
        generic_credit: 1,
        colorless: 1,
        ..ManaPool::default()
    };
    let payment = payment_plan(&cost, &pool, &[])
        .expect("five eligible mana cover all five required symbol groups");
    assert_eq!(payment.after, ManaPool::default());
}

/// Enough total capacity does not grant missing colors, generic eligibility, or split bundles.
#[test]
fn capacity_is_only_a_rejection_check() {
    let cases = [
        (
            "{C}",
            ManaPool {
                flexible: 1,
                ..ManaPool::default()
            },
        ),
        (
            "{W/U}",
            ManaPool {
                generic_credit: 1,
                ..ManaPool::default()
            },
        ),
        (
            "{1}",
            ManaPool {
                fixed: [1, 0, 0, 0, 0],
                fixed_no_generic: [1, 0, 0, 0, 0],
                ..ManaPool::default()
            },
        ),
        (
            "{W}{U}",
            ManaPool {
                lots: vec![ManaLot {
                    amount: 2,
                    types: 3,
                    restriction: None,
                    cannot_pay_generic: false,
                    one_color: true,
                }],
                ..ManaPool::default()
            },
        ),
    ];
    for (raw, pool) in cases {
        let cost = parse_cost(raw);
        let eligible = supplies(&pool, &[]).expect("valid pool fields");
        assert!(has_minimum_capacity(&cost, &eligible));
        assert!(payment_plan(&cost, &pool, &[]).is_none(), "{raw}");
        assert!(
            payment_plan_with_cast_sources(&cost, &pool, &[], &[], &[]).is_none(),
            "{raw}"
        );
    }
}

/// Reused visited flags must not hide a remaining supply from the next demand.
#[test]
fn matching_scratch_resets_between_demands_and_augmentations() {
    let pool = ManaPool {
        flexible: 3,
        colorless: 1,
        ..ManaPool::default()
    };
    let payment = payment_plan(&parse_cost("{W}{U}{2}"), &pool, &[])
        .expect("one flexible supply can serve separate demands and partial augmentations");
    assert_eq!(payment.after, ManaPool::default());
}

/// Reused shortest-path buffers cannot retain unreachable predecessors or distances.
#[test]
fn shortest_path_scratch_resets_after_residual_capacity_changes() {
    let mut flow = CostFlow {
        edges: vec![Vec::new(); 4],
    };
    flow.add_edge(0, 1, 1, (0, 0));
    flow.add_edge(1, 2, 1, (1, 1));
    let mut distance = vec![Some((-1, -1)); 4];
    let mut previous = vec![Some((0, 0)); 4];
    flow.shortest_paths(&mut distance, &mut previous);
    assert_eq!(distance, [Some((0, 0)), Some((0, 0)), Some((1, 1)), None]);
    assert_eq!(previous, [None, Some((0, 0)), Some((1, 1)), None]);
    assert!(flow.fund(2, 1).is_some());
    flow.shortest_paths(&mut distance, &mut previous);
    assert_eq!(distance, [Some((0, 0)), None, None, None]);
    assert_eq!(previous, [None; 4]);
}
