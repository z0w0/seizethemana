use super::game::Pool;
use super::game_mana::{pay_restricted_cost, usable_for_noncreature};
use super::model::Restriction;
use super::parse_cost::parse_cost;

fn pool_general(fixed_w: u32, flexible: u32, colorless: u32) -> Pool {
    let mut pool = Pool::default();
    pool.fixed[0] = fixed_w;
    pool.flexible = flexible;
    pool.colorless = colorless;
    pool
}

#[test]
fn usable_for_counts_general_plus_own_bucket() {
    let mut pool = pool_general(1, 2, 1);
    pool.legendary_only = 3;
    pool.creature_only = 4;
    pool.artifact_only = 5;
    pool.instant_sorcery_only = 6;
    // Each class reaches its own bucket, never another class's.
    assert_eq!(pool.usable_for(Restriction::Legendary), 1 + 2 + 1 + 3);
    assert_eq!(pool.usable_for(Restriction::Creature), 1 + 2 + 1 + 4);
    assert_eq!(pool.usable_for(Restriction::Artifact), 1 + 2 + 1 + 5);
    assert_eq!(pool.usable_for(Restriction::InstantSorcery), 1 + 2 + 1 + 6);
}

#[test]
fn restricted_payment_spends_bucket_first() {
    let mut pool = pool_general(0, 0, 0);
    pool.legendary_only = 2;
    // {3} total: bucket pays 2, general covers 1.
    pool.colorless = 1;
    pay_restricted_cost(&parse_cost("{3}"), &mut pool, Restriction::Legendary);
    assert_eq!(pool.legendary_only, 0);
    assert_eq!(pool.colorless, 0);
    assert_eq!(pool.total(), 0);
}

#[test]
fn restricted_payment_leaves_unused_bucket() {
    let mut pool = pool_general(0, 0, 2);
    pool.legendary_only = 3;
    // {2}: bucket pays 2, one stays for a later legendary cast.
    pay_restricted_cost(&parse_cost("{2}"), &mut pool, Restriction::Legendary);
    assert_eq!(pool.legendary_only, 1);
    assert_eq!(pool.colorless, 2);
}

#[test]
fn restricted_payment_covers_pips_without_double_pay() {
    let mut pool = pool_general(0, 0, 0);
    pool.legendary_only = 3;
    // {W}{W}: the bucket's chosen-color mana covers both pips; the
    // general pool must not pay them a second time.
    pay_restricted_cost(&parse_cost("{W}{W}"), &mut pool, Restriction::Legendary);
    assert_eq!(pool.legendary_only, 1);
    assert_eq!(pool.total(), 1);
}

#[test]
fn restricted_payment_splits_between_bucket_and_general_pips() {
    let mut pool = pool_general(2, 0, 0);
    pool.instant_sorcery_only = 1;
    // {W}{W}: one pip from the bucket, one from the two fixed white.
    pay_restricted_cost(
        &parse_cost("{W}{W}"),
        &mut pool,
        Restriction::InstantSorcery,
    );
    assert_eq!(pool.instant_sorcery_only, 0);
    assert_eq!(pool.fixed[0], 1);
    assert_eq!(pool.total(), 1);
}

#[test]
fn buckets_pay_only_their_own_class_across_casts() {
    // A creature-only bucket cannot fund an artifact cast, even alone.
    let mut pool = pool_general(0, 0, 0);
    pool.creature_only = 5;
    assert!(pool.usable_for(Restriction::Artifact) < 1);
    // And an unrestricted cast reaches none of any bucket.
    assert_eq!(usable_for_noncreature(&pool), 0);
}
