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
