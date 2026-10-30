use super::*;

/// Each selector uses the front card's normalized type data.
#[test]
fn card_types_match_without_using_merged_face_flags() {
    for kind in [
        SpellCostCardType::Artifact,
        SpellCostCardType::Battle,
        SpellCostCardType::Creature,
        SpellCostCardType::Enchantment,
        SpellCostCardType::Instant,
        SpellCostCardType::Planeswalker,
        SpellCostCardType::Sorcery,
    ] {
        let filter = SpellCostFilter::Types(vec![kind]);
        let mut card = SimCard {
            card_types: vec![kind.name().into()],
            ..Default::default()
        };
        assert!(filter.matches(&card));
        card.card_types = vec!["land".into()];
        card.is_artifact = true;
        card.is_creature = true;
        card.is_enchantment = true;
        card.is_instant_or_sorcery = true;
        card.is_planeswalker = true;
        assert!(!filter.matches(&card));
    }
}

/// A union selects any listed type and does not require all listed types.
#[test]
fn type_union_includes_auras_and_artifact_creatures() {
    let filter = SpellCostFilter::Types(vec![
        SpellCostCardType::Artifact,
        SpellCostCardType::Enchantment,
    ]);
    for types in [vec!["artifact", "creature"], vec!["enchantment"]] {
        let card = SimCard {
            card_types: types.into_iter().map(str::to_string).collect(),
            is_enchantment: false,
            ..Default::default()
        };
        assert!(filter.matches(&card));
    }
    assert!(!filter.matches(&SimCard::default()));
    assert!(!SpellCostFilter::Types(Vec::new()).matches(&SimCard::default()));
}

/// Unknown front-face colors never borrow a hidden face's merged colors.
#[test]
fn color_union_requires_known_front_face_colors() {
    let filter = SpellCostFilter::Colors(vec![ManaColor::Red, ManaColor::Green]);
    let mut card = SimCard {
        colors: [true; 5],
        color_identity: [true; 5],
        search_colors: None,
        ..Default::default()
    };
    assert!(!filter.matches(&card));
    for known in [
        [false, false, false, true, false],
        [false, false, false, false, true],
        [false, false, false, true, true],
    ] {
        card.search_colors = Some(known);
        assert!(filter.matches(&card));
    }
    card.search_colors = Some([false, true, false, false, false]);
    assert!(!filter.matches(&card));
    card.search_colors = Some([false; 5]);
    assert!(!filter.matches(&card));
    assert!(!SpellCostFilter::Colors(Vec::new()).matches(&card));
    assert!(SpellCostFilter::Any.matches(&card));
}
