//! Tests for mana cast restrictions.
use super::game::ManaPool;
use super::game_mana::{
    cast_restrictions, pay_restricted_cost, usable_for_classes, usable_for_noncreature,
};
use super::model::SpendRestriction;
use super::oracle_parser::cost::parse_cost;

fn pool_general(fixed_w: u32, flexible: u32, colorless: u32) -> ManaPool {
    let mut pool = ManaPool::default();
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
    assert_eq!(
        usable_for_classes(&pool, &[SpendRestriction::Legendary]),
        1 + 2 + 1 + 3
    );
    assert_eq!(
        usable_for_classes(&pool, &[SpendRestriction::Creature]),
        1 + 2 + 1 + 4
    );
    assert_eq!(
        usable_for_classes(&pool, &[SpendRestriction::Artifact]),
        1 + 2 + 1 + 5
    );
    assert_eq!(
        usable_for_classes(&pool, &[SpendRestriction::InstantSorcery]),
        1 + 2 + 1 + 6
    );
}

#[test]
fn restricted_payment_spends_bucket_first() {
    let mut pool = pool_general(0, 0, 0);
    pool.legendary_only = 2;
    // {3} total: bucket pays 2, general covers 1.
    pool.colorless = 1;
    pay_restricted_cost(
        &parse_cost("{3}"),
        &mut pool,
        &[SpendRestriction::Legendary],
    );
    assert_eq!(pool.legendary_only, 0);
    assert_eq!(pool.colorless, 0);
    assert_eq!(pool.total(), 0);
}

#[test]
fn restricted_payment_leaves_unused_bucket() {
    let mut pool = pool_general(0, 0, 2);
    pool.legendary_only = 3;
    // {2}: bucket pays 2, one stays for a later legendary cast.
    pay_restricted_cost(
        &parse_cost("{2}"),
        &mut pool,
        &[SpendRestriction::Legendary],
    );
    assert_eq!(pool.legendary_only, 1);
    assert_eq!(pool.colorless, 2);
}

#[test]
fn restricted_payment_covers_pips_without_double_pay() {
    let mut pool = pool_general(0, 0, 0);
    pool.legendary_only = 3;
    // {W}{W}: the bucket's chosen-color mana covers both pips; the
    // general pool must not pay them a second time.
    pay_restricted_cost(
        &parse_cost("{W}{W}"),
        &mut pool,
        &[SpendRestriction::Legendary],
    );
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
        &[SpendRestriction::InstantSorcery],
    );
    assert_eq!(pool.instant_sorcery_only, 0);
    assert_eq!(pool.fixed[0], 1);
    assert_eq!(pool.total(), 1);
}

#[test]
fn restricted_payment_ignores_life_paid_phyrexian_pips() {
    let mut pool = pool_general(0, 0, 0);
    pool.artifact_only = 2;
    // {1}{B/P} owes 1 mana (the pip pays 2 life) + 2 life: the bucket
    // drains 1, not the full printed 2.
    pay_restricted_cost(
        &parse_cost("{1}{B/P}"),
        &mut pool,
        &[SpendRestriction::Artifact],
    );
    assert_eq!(pool.artifact_only, 1);
    assert_eq!(pool.total(), 1);
}

#[test]
fn restricted_bucket_mana_cannot_pay_colorless_pips() {
    // Bucket mana is colored ("one color of the source's choice"), so it
    // cannot pay a colorless pip (CR 107.4c). The bucket pays the green
    // pip; the general colorless pool pays the colorless pip.
    let mut pool = pool_general(0, 0, 1);
    pool.creature_only = 2;
    pay_restricted_cost(
        &parse_cost("{C}{G}"),
        &mut pool,
        &[SpendRestriction::Creature],
    );
    assert_eq!(
        pool.creature_only, 1,
        "colored bucket mana pays the green pip, not the colorless pip"
    );
    assert_eq!(
        pool.colorless, 0,
        "the colorless pip drained the colorless pool"
    );
}

#[test]
fn buckets_pay_only_their_own_class_across_casts() {
    // A creature-only bucket cannot fund an artifact cast, even alone.
    let mut pool = pool_general(0, 0, 0);
    pool.creature_only = 5;
    assert!(usable_for_classes(&pool, &[SpendRestriction::Artifact]) < 1);
    // And an unrestricted cast reaches none of any bucket.
    assert_eq!(usable_for_noncreature(&pool), 0);
}

#[test]
fn artifact_creature_cast_draws_from_both_classes() {
    // An artifact creature belongs to both classes: creature-restricted
    // mana (Secluded Courtyard) and artifact-restricted mana may pay
    // for it.
    let mut pool = pool_general(0, 0, 0);
    pool.creature_only = 1;
    let classes = cast_restrictions(&artifact_creature_card());
    assert!(
        classes.contains(&SpendRestriction::Creature)
            && classes.contains(&SpendRestriction::Artifact),
        "an artifact creature carries both cast classes: {:?}",
        classes
    );
    assert!(
        usable_for_classes(&pool, &classes) >= 1,
        "creature-only mana pays for an artifact creature"
    );
    // A plain artifact never gains the creature class.
    let mut plain = artifact_creature_card();
    plain.is_creature = false;
    let classes = cast_restrictions(&plain);
    assert_eq!(classes, vec![SpendRestriction::Artifact]);
    assert_eq!(usable_for_classes(&pool, &classes), 0);
}

fn artifact_creature_card() -> super::model::SimCard {
    super::model::SimCard {
        name: "Test Ballista".into(),
        is_creature: true,
        is_artifact: true,
        ..super::model::SimCard::default()
    }
}
