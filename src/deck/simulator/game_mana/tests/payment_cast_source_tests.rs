use super::*;
use crate::deck::simulator::oracle_parser::cost::parse_cost;

/// One permanent cannot tap for mana and convoke the same spell.
#[test]
fn mana_and_convoke_modes_share_one_source_capacity() {
    let mut costs = [[None; 6]];
    costs[0][4] = Some(0);
    let taps = [Some(2)];
    assert!(
        payment_plan_with_cast_sources(
            &parse_cost("{G}{U}"),
            &ManaPool::default(),
            &[],
            &costs,
            &taps,
        )
        .is_none()
    );
    let (_, used) = payment_plan_with_cast_sources(
        &parse_cost("{U}"),
        &ManaPool::default(),
        &[],
        &costs,
        &taps,
    )
    .expect("convoke pays blue without activating the green mana mode");
    assert!(used.is_empty());
    let (_, used) = payment_plan_with_cast_sources(
        &parse_cost("{G}"),
        &ManaPool::default(),
        &[],
        &costs,
        &taps,
    )
    .expect("the actual green mode can pay instead of convoking");
    assert_eq!(used, [(0, 16)]);
}

/// Virtual generic payment is not actual colorless mana, including when bit 5 is set.
#[test]
fn combined_cast_sources_keep_virtual_colorless_rules_unchanged() {
    for mask in [0, 32, 64] {
        let costs = [[None; 6]];
        let taps = [Some(mask)];
        assert!(
            payment_plan_with_cast_sources(
                &parse_cost("{C}"),
                &ManaPool::default(),
                &[],
                &costs,
                &taps,
            )
            .is_none()
        );
        let (_, used) = payment_plan_with_cast_sources(
            &parse_cost("{1}"),
            &ManaPool::default(),
            &[],
            &costs,
            &taps,
        )
        .expect("a generic virtual payment does not need a colored mask");
        assert!(used.is_empty());
    }
    let mut costs = [[None; 6]];
    costs[0][5] = Some(0);
    let (_, used) = payment_plan_with_cast_sources(
        &parse_cost("{C}"),
        &ManaPool::default(),
        &[],
        &costs,
        &[Some(32)],
    )
    .expect("the separate actual colorless mode can pay a colorless symbol");
    assert_eq!(used, [(0, 32)]);
}

/// A partial virtual payment can combine with one necessary painful mana activation.
#[test]
fn partial_virtual_payment_adds_only_the_missing_actual_mana() {
    let mut costs = [[None; 6]; 2];
    costs[0][1] = Some(1);
    costs[0][5] = Some(0);
    let (_, used) = payment_plan_with_cast_sources(
        &parse_cost("{U}{U}"),
        &ManaPool::default(),
        &[],
        &costs,
        &[None, Some(2)],
    )
    .expect("one convoking creature and one painful blue mode pay two blue symbols");
    assert_eq!(used, [(0, 2)]);
    assert!(
        payment_plan_with_cast_sources(
            &parse_cost("{U}{U}{U}"),
            &ManaPool::default(),
            &[],
            &costs,
            &[None, Some(2)],
        )
        .is_none()
    );
}
