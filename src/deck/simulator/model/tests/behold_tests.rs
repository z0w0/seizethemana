use super::*;

/// Every required type and subtype must belong to the same exposed object.
#[test]
fn combined_quality_requires_all_characteristics() {
    let spec = BeholdSpec {
        card_types: vec!["creature".into()],
        subtypes: vec!["dragon".into()],
        count: 1,
    };
    assert!(spec.matches_characteristics(&["creature".into()], &["dragon".into()]));
    assert!(!spec.matches_characteristics(&["enchantment".into()], &["dragon".into()]));
    assert!(!spec.matches_characteristics(&["creature".into()], &["elf".into()]));
    assert!(!spec.matches_characteristics(&["creature".into()], &[]));
}

/// Jace means a subtype, not a card name or an arbitrary planeswalker.
#[test]
fn jace_planeswalker_quality_retains_both_constraints() {
    let spec = BeholdSpec {
        card_types: vec!["planeswalker".into()],
        subtypes: vec!["jace".into()],
        count: 1,
    };
    assert!(spec.matches_characteristics(
        &["legendary".into(), "planeswalker".into()],
        &["jace".into()],
    ));
    assert!(!spec.matches_characteristics(&["planeswalker".into()], &["sarkhan".into()]));
    assert!(!spec.matches_characteristics(&["creature".into()], &["jace".into()]));
}
