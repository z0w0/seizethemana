//! Tests for format rules and mulligan policy.
use crate::deck::simulator::format::{MulliganPolicy, rules_for, rules_inferred};
use crate::deck::simulator::model::Format;

#[test]
fn table_covers_every_known_format() {
    for key in crate::deck::legal::KNOWN_FORMATS {
        assert_eq!(rules_for(key).key, *key, "missing rules for {key}");
    }
}

#[test]
fn unknown_key_falls_back_to_constructed() {
    let rules = rules_for("kitchen-table");
    assert_eq!(rules.key, "constructed");
    assert_eq!(rules.shape, Format::Constructed);
    assert_eq!(rules.default_turns, 8);
}

#[test]
fn commander_family_keeps_free_redraw_band() {
    for key in ["commander", "brawl", "oathbreaker"] {
        let rules = rules_for(key);
        assert_eq!(rules.shape, Format::Commander);
        assert_eq!(rules.default_turns, 10);
        assert!(
            matches!(
                rules.mulligan,
                MulliganPolicy::FreeRedraw { land_band: (2, 6) }
            ),
            "{key} lost its redraw band"
        );
    }
}

#[test]
fn starting_life_matches_the_rules() {
    // CR 103.4c: commander starts at 40. CR 903.12f: brawl starts at 25.
    // Oathbreaker and 60-card constructed start at 20.
    assert_eq!(rules_for("commander").starting_life, 40);
    assert_eq!(rules_for("brawl").starting_life, 25);
    assert_eq!(rules_for("oathbreaker").starting_life, 20);
    for key in [
        "standard", "pioneer", "modern", "legacy", "vintage", "pauper",
    ] {
        assert_eq!(rules_for(key).starting_life, 20, "{key} life");
    }
    // The lethal census races opponents' total life.
    assert_eq!(rules_for("commander").life_target(), 120.0);
    assert_eq!(rules_for("brawl").life_target(), 75.0);
    assert_eq!(rules_for("modern").life_target(), 20.0);
}

#[test]
fn sixty_card_formats_use_london() {
    for key in [
        "standard", "pioneer", "modern", "legacy", "vintage", "pauper",
    ] {
        let rules = rules_for(key);
        assert_eq!(rules.shape, Format::Constructed);
        assert_eq!(rules.default_turns, 8);
        assert!(
            matches!(rules.mulligan, MulliganPolicy::London),
            "{key} lost its London policy"
        );
    }
}

#[test]
fn inference_follows_commander_section() {
    assert_eq!(rules_inferred(true).key, "commander");
    assert_eq!(rules_inferred(false).key, "constructed");
}
