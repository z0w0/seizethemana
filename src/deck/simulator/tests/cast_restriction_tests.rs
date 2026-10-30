//! Tests for mana cast restrictions.
use super::game::ManaPool;
use super::game_mana::{
    ManaLot, cast_restrictions, pay_restricted_cost, usable_for_classes, usable_for_noncreature,
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

/// Add any-color mana with one cast-class restriction.
fn restricted(pool: &mut ManaPool, amount: u32, restriction: SpendRestriction) {
    pool.lots.push(ManaLot {
        amount,
        types: 31,
        restriction: Some(restriction),
        cannot_pay_generic: false,
        one_color: false,
    });
}

/// Count the mana left for one restricted cast class.
fn restricted_amount(pool: &ManaPool, restriction: SpendRestriction) -> u32 {
    pool.lots
        .iter()
        .filter(|lot| lot.restriction == Some(restriction))
        .map(|lot| lot.amount)
        .sum()
}

#[test]
fn usable_for_counts_general_plus_own_bucket() {
    let mut pool = pool_general(1, 2, 1);
    restricted(&mut pool, 3, SpendRestriction::Legendary);
    restricted(&mut pool, 4, SpendRestriction::Creature);
    restricted(&mut pool, 5, SpendRestriction::Artifact);
    restricted(&mut pool, 6, SpendRestriction::InstantSorcery);
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
    restricted(&mut pool, 2, SpendRestriction::Legendary);
    // {3} total: bucket pays 2, general covers 1.
    pool.colorless = 1;
    pay_restricted_cost(
        &parse_cost("{3}"),
        &mut pool,
        &[SpendRestriction::Legendary],
    );
    assert_eq!(restricted_amount(&pool, SpendRestriction::Legendary), 0);
    assert_eq!(pool.colorless, 0);
    assert_eq!(pool.total(), 0);
}

#[test]
fn restricted_payment_preserves_colored_mana_when_colorless_covers_generic() {
    let mut pool = pool_general(0, 0, 2);
    restricted(&mut pool, 3, SpendRestriction::Legendary);
    pay_restricted_cost(
        &parse_cost("{2}"),
        &mut pool,
        &[SpendRestriction::Legendary],
    );
    assert_eq!(restricted_amount(&pool, SpendRestriction::Legendary), 3);
    assert_eq!(pool.colorless, 0);
}

#[test]
fn restricted_payment_covers_pips_without_double_pay() {
    let mut pool = pool_general(0, 0, 0);
    restricted(&mut pool, 3, SpendRestriction::Legendary);
    // {W}{W}: the bucket's chosen-color mana covers both pips; the
    // general pool must not pay them a second time.
    pay_restricted_cost(
        &parse_cost("{W}{W}"),
        &mut pool,
        &[SpendRestriction::Legendary],
    );
    assert_eq!(restricted_amount(&pool, SpendRestriction::Legendary), 1);
    assert_eq!(pool.total(), 1);
}

#[test]
fn restricted_payment_splits_between_bucket_and_general_pips() {
    let mut pool = pool_general(2, 0, 0);
    restricted(&mut pool, 1, SpendRestriction::InstantSorcery);
    // {W}{W}: one pip from the bucket, one from the two fixed white.
    pay_restricted_cost(
        &parse_cost("{W}{W}"),
        &mut pool,
        &[SpendRestriction::InstantSorcery],
    );
    assert_eq!(
        restricted_amount(&pool, SpendRestriction::InstantSorcery),
        0
    );
    assert_eq!(pool.fixed[0], 1);
    assert_eq!(pool.total(), 1);
}

#[test]
fn restricted_payment_ignores_life_paid_phyrexian_pips() {
    let mut pool = pool_general(0, 0, 0);
    restricted(&mut pool, 2, SpendRestriction::Artifact);
    // {1}{B/P} owes 1 mana (the pip pays 2 life) + 2 life: the bucket
    // drains 1, not the full printed 2.
    pay_restricted_cost(
        &parse_cost("{1}{B/P}"),
        &mut pool,
        &[SpendRestriction::Artifact],
    );
    assert_eq!(restricted_amount(&pool, SpendRestriction::Artifact), 1);
    assert_eq!(pool.total(), 1);
}

#[test]
fn restricted_bucket_mana_cannot_pay_colorless_pips() {
    // Bucket mana is colored ("one color of the source's choice"), so it
    // cannot pay a colorless pip (CR 107.4c). The bucket pays the green
    // pip; the general colorless pool pays the colorless pip.
    let mut pool = pool_general(0, 0, 1);
    restricted(&mut pool, 2, SpendRestriction::Creature);
    pay_restricted_cost(
        &parse_cost("{C}{G}"),
        &mut pool,
        &[SpendRestriction::Creature],
    );
    assert_eq!(
        restricted_amount(&pool, SpendRestriction::Creature),
        1,
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
    restricted(&mut pool, 5, SpendRestriction::Creature);
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
    restricted(&mut pool, 1, SpendRestriction::Creature);
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
