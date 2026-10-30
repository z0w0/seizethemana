use super::*;
use crate::deck::simulator::oracle_parser::cost::parse_cost;

/// Physical choices retain explicit colorless mana without changing convoke rules.
#[test]
fn physical_source_choices_keep_the_sixth_mana_type() {
    let pool = ManaPool::default();
    for (cost, expected) in [("{C}", 32), ("{U}", 2), ("{1}", 32)] {
        let (_, chosen) =
            payment_plan_with_source_choices(&parse_cost(cost), &pool, &[], &[0b100010])
                .expect("one physical source can produce the needed type");
        assert_eq!(chosen, [(0, expected)], "{cost}");
    }
    assert!(payment_plan_with_virtual_sources(&parse_cost("{C}"), &pool, &[], &[32]).is_none());
}

/// One physical source can pay exactly one symbol, even when it offers many types.
#[test]
fn physical_source_choices_cannot_pay_two_symbols() {
    let pool = ManaPool::default();
    for cost in ["{W}{U}", "{C}{U}", "{1}{U}", "{2}"] {
        assert!(
            payment_plan_with_source_choices(&parse_cost(cost), &pool, &[], &[63],).is_none(),
            "{cost}"
        );
        assert_eq!(pool, ManaPool::default());
    }
}

/// Rerouting preserves both source identity and each actual chosen mana type.
#[test]
fn physical_source_choices_reroute_overlapping_colors() {
    let (_, chosen) = payment_plan_with_source_choices(
        &parse_cost("{W}{U}{C}"),
        &ManaPool::default(),
        &[],
        &[3, 1, 32],
    )
    .expect("the flexible source pays blue and the white-only source pays white");
    assert_eq!(chosen, [(0, 2), (1, 1), (2, 32)]);
}

/// Already-produced mana avoids an unnecessary physical activation.
#[test]
fn physical_source_choices_use_existing_pool_first() {
    let pool = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        ..Default::default()
    };
    let (_, chosen) = payment_plan_with_source_choices(&parse_cost("{U}"), &pool, &[], &[2])
        .expect("the produced blue mana already pays the spell");
    assert!(chosen.is_empty());
    assert_eq!(pool.fixed[1], 1);
}

/// A source that has no legal mode cannot pay even a generic symbol.
#[test]
fn empty_physical_source_choices_do_not_create_generic_mana() {
    for source in [0, 64, 128] {
        assert!(
            payment_plan_with_source_choices(
                &parse_cost("{1}"),
                &ManaPool::default(),
                &[],
                &[source],
            )
            .is_none()
        );
    }
    assert!(
        payment_plan_with_virtual_sources(&parse_cost("{1}"), &ManaPool::default(), &[], &[0],)
            .is_some()
    );
}

/// A stale physical-source plan cannot consume an altered pool.
#[test]
fn physical_source_payment_commit_is_atomic() {
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let (payment, chosen) =
        payment_plan_with_source_choices(&parse_cost("{1}{U}"), &pool, &[], &[2])
            .expect("one produced mana and one physical source pay the spell");
    assert_eq!(chosen, [(0, 2)]);
    pool.colorless += 1;
    let before = pool.clone();
    assert!(!payment.commit(&mut pool));
    assert_eq!(pool, before);
}

/// Hybrid and twobrid costs share the same one-source accounting.
#[test]
fn physical_source_choices_keep_alternative_cost_symbols() {
    for cost in ["{W/U}", "{2/U}"] {
        let (_, chosen) =
            payment_plan_with_source_choices(&parse_cost(cost), &ManaPool::default(), &[], &[2])
                .expect("one blue source pays the colored half");
        assert_eq!(chosen, [(0, 2)]);
    }
    assert!(
        payment_plan_with_source_choices(&parse_cost("{2/U}"), &ManaPool::default(), &[], &[32],)
            .is_none()
    );
}
