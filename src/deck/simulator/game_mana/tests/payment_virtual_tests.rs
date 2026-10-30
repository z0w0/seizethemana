use super::*;
use crate::deck::simulator::game_mana::ManaLot;
use crate::deck::simulator::oracle_parser::cost::parse_cost;

/// Matching reroutes an earlier virtual color choice and spends real generic mana.
#[test]
fn virtual_sources_report_the_exact_used_indices() {
    let mut pool = ManaPool {
        colorless: 1,
        ..ManaPool::default()
    };
    let (payment, mut used) = payment_plan_with_virtual_sources(
        &parse_cost("{1}{W}{U}"),
        &pool,
        &[],
        &[0b00011, 0b00001],
    )
    .expect("two matching creatures and one generic source pay the spell");
    used.sort_unstable();

    assert_eq!(used, [0, 1]);
    assert!(payment.commit(&mut pool));
    assert_eq!(pool.colorless, 0);
}

/// A virtual source cannot pay two symbols or any explicit colorless symbol.
#[test]
fn virtual_sources_have_one_use_and_no_colorless_type() {
    for cost in ["{W}{U}", "{1}{W}", "{C}"] {
        let pool = ManaPool::default();
        assert!(
            payment_plan_with_virtual_sources(&parse_cost(cost), &pool, &[], &[0b111111]).is_none(),
            "{cost}"
        );
        assert_eq!(pool.total(), 0);
    }
}

/// Improvise-style sources only pay generic symbols, including twobrid generic choices.
#[test]
fn generic_virtual_sources_do_not_gain_colors() {
    let pool = ManaPool::default();
    for cost in ["{W}", "{W/U}", "{C}"] {
        assert!(
            payment_plan_with_virtual_sources(&parse_cost(cost), &pool, &[], &[0, 0]).is_none(),
            "{cost}"
        );
    }
    let (_, used) = payment_plan_with_virtual_sources(&parse_cost("{2/W}"), &pool, &[], &[0, 0])
        .expect("two generic resources pay the generic twobrid choice");
    assert_eq!(used, [0, 1]);
}

/// Unused virtual sources do not appear among the committed resource identities.
#[test]
fn virtual_payment_reports_only_consumed_sources() {
    let pool = ManaPool::default();
    let (_, used) = payment_plan_with_virtual_sources(
        &parse_cost("{U}"),
        &pool,
        &[],
        &[0b00001, 0b00010, 0b00100],
    )
    .expect("one blue source pays one blue symbol");
    assert_eq!(used, [1]);
}

/// Virtual sources do not loosen the restrictions on actual produced mana.
#[test]
fn virtual_payment_keeps_real_mana_cast_restrictions() {
    let pool = ManaPool {
        lots: vec![ManaLot {
            amount: 1,
            types: 0b00010,
            restriction: Some(SpendRestriction::Artifact),
            cannot_pay_generic: false,
            one_color: false,
        }],
        ..ManaPool::default()
    };
    let cost = parse_cost("{1}{U}");
    assert!(payment_plan_with_virtual_sources(&cost, &pool, &[], &[0]).is_none());
    assert!(
        payment_plan_with_virtual_sources(&cost, &pool, &[SpendRestriction::Artifact], &[0])
            .is_some()
    );
}

/// Many flexible creatures need one capacitated match, not color-choice enumeration.
#[test]
fn many_virtual_sources_use_one_exact_match() {
    let mut sources = vec![0b00011; 40];
    sources.extend([0b00001; 40]);
    let cost = Cost {
        pips: [40, 40, 0, 0, 0],
        ..Cost::default()
    };
    MATCHER_CALLS.with(|calls| calls.set(0));

    let (_, used) = payment_plan_with_virtual_sources(&cost, &ManaPool::default(), &[], &sources)
        .expect("forty white and forty white-blue creatures cover the fixed pips");

    assert_eq!(used.len(), 80);
    assert_eq!(MATCHER_CALLS.with(|calls| calls.get()), 1);
}
