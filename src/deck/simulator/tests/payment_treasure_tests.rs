use super::*;
use crate::deck::simulator::game_mana::{
    ManaLot, pay_cost, pay_restricted_cost, usable_for_classes, usable_for_noncreature,
};

/// Preparation keeps tokens banked; commit spends only the required count.
#[test]
fn treasure_payments_preserve_unused_tokens_without_floating_mana() {
    let mut pool = ManaPool {
        treasures: 3,
        ..ManaPool::default()
    };
    assert_eq!(pool.total(), 3);
    assert_eq!(pool.mana_total(), 0);
    let before = pool.clone();
    let generic = Cost {
        generic: 1,
        ..Cost::default()
    };
    let payment = payment_plan(&generic, &pool, &[]).expect("one Treasure pays generic mana");
    assert_eq!(pool, before);
    assert!(payment.commit(&mut pool));
    assert_eq!(
        pool,
        ManaPool {
            treasures: 2,
            ..ManaPool::default()
        }
    );

    let blue = Cost {
        pips: [0, 1, 0, 0, 0],
        ..Cost::default()
    };
    assert!(pay_cost(&blue, &mut pool));
    assert_eq!(
        pool,
        ManaPool {
            treasures: 1,
            ..ManaPool::default()
        }
    );
    assert_eq!(pool.total(), 1);
    assert_eq!(pool.mana_total(), 0);
}

/// Banked Treasures cannot cover explicit colorless or an unpaid remainder.
#[test]
fn failed_treasure_payments_leave_the_bank_unchanged() {
    let mut pool = ManaPool {
        treasures: 3,
        ..ManaPool::default()
    };
    let before = pool.clone();
    for cost in [
        Cost {
            colorless: 1,
            ..Cost::default()
        },
        Cost {
            generic: 4,
            ..Cost::default()
        },
        Cost {
            colorless: 1,
            generic: 2,
            ..Cost::default()
        },
    ] {
        assert!(!pay_cost(&cost, &mut pool));
        assert_eq!(pool, before);
    }
    assert!(pay_cost(&Cost::default(), &mut pool));
    assert_eq!(pool, before);
}

/// Each sacrificed token can choose its own color.
#[test]
fn treasure_tokens_pay_independent_colored_pips() {
    let mut pool = ManaPool {
        treasures: 5,
        ..ManaPool::default()
    };
    assert!(pay_cost(
        &Cost {
            pips: [1; 5],
            ..Cost::default()
        },
        &mut pool
    ));
    assert_eq!(pool, ManaPool::default());
}

/// Spend generic credit and produced mana before sacrificing banked tokens.
#[test]
fn existing_mana_and_generic_credit_preserve_treasures() {
    let mut pool = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        colorless: 1,
        generic_credit: 1,
        treasures: 3,
        ..ManaPool::default()
    };
    assert_eq!(pool.total(), 6);
    assert_eq!(pool.mana_total(), 3);
    assert!(pay_cost(
        &Cost {
            generic: 2,
            ..Cost::default()
        },
        &mut pool
    ));
    assert_eq!(pool.generic_credit, 0);
    assert_eq!(pool.colorless, 0);
    assert_eq!(pool.treasures, 3);
    assert_eq!(pool.mana_total(), 1);
    assert!(pay_cost(
        &Cost {
            pips: [0, 1, 0, 0, 0],
            ..Cost::default()
        },
        &mut pool
    ));
    assert_eq!(
        pool,
        ManaPool {
            treasures: 3,
            ..ManaPool::default()
        }
    );
}

/// Restricted lots keep their own types and classes beside unrestricted Treasures.
#[test]
fn restricted_lots_do_not_turn_treasures_into_colorless_mana() {
    let artifact_blue = ManaLot {
        amount: 2,
        types: 2,
        restriction: Some(SpendRestriction::Artifact),
        cannot_pay_generic: false,
        one_color: false,
    };
    let creature_colorless = ManaLot {
        amount: 1,
        types: COLORLESS,
        restriction: Some(SpendRestriction::Creature),
        cannot_pay_generic: false,
        one_color: false,
    };
    let mut pool = ManaPool {
        treasures: 3,
        lots: vec![artifact_blue.clone(), creature_colorless],
        ..ManaPool::default()
    };
    let cost = Cost {
        colorless: 1,
        pips: [0, 1, 0, 0, 0],
        ..Cost::default()
    };
    let before = pool.clone();
    assert!(!pay_restricted_cost(
        &cost,
        &mut pool,
        &[SpendRestriction::Artifact]
    ));
    assert_eq!(pool, before);
    assert!(pay_restricted_cost(
        &cost,
        &mut pool,
        &[SpendRestriction::Creature]
    ));
    assert_eq!(
        pool,
        ManaPool {
            treasures: 2,
            lots: vec![artifact_blue],
            ..ManaPool::default()
        }
    );
    assert_eq!(pool.mana_total(), 2);
    assert_eq!(usable_for_noncreature(&pool), 2);
    assert_eq!(usable_for_classes(&pool, &[SpendRestriction::Artifact]), 4);
}

/// A token-bank change invalidates a prepared payment instead of spending twice.
#[test]
fn treasure_bank_changes_reject_stale_payment_plans() {
    let mut pool = ManaPool {
        treasures: 3,
        ..ManaPool::default()
    };
    let cost = Cost {
        generic: 1,
        ..Cost::default()
    };
    let payment = payment_plan(&cost, &pool, &[]).expect("one Treasure is available");
    pool.treasures -= 1;
    let changed = pool.clone();
    assert!(!payment.commit(&mut pool));
    assert_eq!(pool, changed);
}
