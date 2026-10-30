//! Mana-yield grammar regressions for conditional color sets and colorless choices.

use super::*;

/// Graveyard conditions and near-matching battlefield conditions remain unsupported.
#[test]
fn conditional_color_sets_fail_closed() {
    for text in [
        "Add one mana of any color among legendary creature cards in your graveyard.",
        "Add one mana of any color among legendary creatures you control.",
        "Add one mana of any color among legendary creatures and planeswalkers you control in your graveyard.",
        "Add two mana of any one color among creature cards in your graveyard.",
        "Add one mana of any color that a creature card in your graveyard could produce.",
    ] {
        assert!(parse_tap_yield(text).is_none(), "{text}");
    }
}

/// The supported legendary battlefield condition retains its runtime gate.
#[test]
fn legendary_battlefield_colors_keep_their_condition() {
    let mana = parse_tap_yield(
        "Add one mana of any color among legendary creatures and planeswalkers you control.",
    )
    .expect("supported battlefield colors");
    assert_eq!(mana.any_pips, 1);
    assert!(mana.legendary_colors);
}

/// Supported unconditional and opponent-dependent sources keep distinct semantics.
#[test]
fn supported_rainbow_mana_is_not_lost() {
    for text in [
        "Add one mana of any color.",
        "Add one mana of any color in your commander's color identity.",
        "Add one mana of any color in your commander’s color identity.",
        "Add two mana of any one color.",
        "Add two mana in any combination of colors.",
    ] {
        assert!(parse_tap_yield(text).is_some(), "{text}");
    }
    let opponent = parse_tap_yield(
        "Add one mana of any color that a land an opponent controls could produce.",
    )
    .expect("opponent-dependent production");
    assert!(opponent.opponent_any);
}

/// Equivalent apostrophes do not remove or broaden the written identity clause.
#[test]
fn commander_identity_punctuation_keeps_the_complete_mana_phrase() {
    for apostrophe in ["'", "’"] {
        let text =
            format!("Add one mana of any color in your commander{apostrophe}s color identity.");
        let mana = parse_tap_yield(&text).expect("identity-limited mana wording");
        assert_eq!(mana.commander_pips, 1);
        assert_eq!(mana.any_pips, 0);
        assert!(!mana.opponent_any);
        let unsupported = format!(
            "Add one mana of any color in your commander{apostrophe}s color identity unless an opponent pays {{1}}."
        );
        assert!(parse_tap_yield(&unsupported).is_none());
    }
}

/// Colorless is one of the source choices, not an extra simultaneous pip.
#[test]
fn colorless_alternative_is_one_mana() {
    let mana = parse_tap_yield("Add {C} or {W}.").expect("supported choice");
    assert_eq!(mana.total(), 1);
    assert!(mana.choice_colorless);
    assert!(mana.choice[0]);
    assert_eq!(mana.colorless, 0);
}

/// Commas separate single-mana alternatives, including the Oxford-comma form.
#[test]
fn comma_separated_color_choices_are_one_mana() {
    for text in [
        "Add {W}, {U}, or {B}.",
        "Add {W}, {U} or {B}.",
        "Add {G}, {W}, or {U}.",
        "Add {C}, {W}, or {U}.",
    ] {
        let mana = parse_tap_yield(text).expect("single-mana alternatives");
        assert_eq!(mana.total(), 1, "{text}");
        assert_eq!(mana.fixed, [0; 5]);
        assert_eq!(mana.colorless, 0);
    }
    let tri_land = parse_tap_yield("Add {W}, {U}, or {B}.").expect("tri-land colors");
    assert_eq!(tri_land.choice, [true, true, true, false, false]);
    let hierarch = parse_tap_yield("Add {G}, {W}, or {U}.").expect("Hierarch colors");
    assert_eq!(hierarch.choice, [true, true, false, false, true]);
}

/// A multi-symbol choice cannot become several independent color choices.
#[test]
fn unsupported_alternative_bundles_remain_inactive() {
    for text in [
        "Add {W}{U} or {B}{G}.",
        "Add {W}, {U}{B}, or {G}.",
        "Add {W}{W}, {U}, or {B}.",
    ] {
        assert!(parse_tap_yield(text).is_none(), "{text}");
    }
    let fixed = parse_tap_yield("Add {W}{U}{B}.").expect("fixed bundle");
    assert_eq!(fixed.fixed, [1, 1, 1, 0, 0]);
    assert_eq!(fixed.total(), 3);
}

/// Counter-scaled mana keeps one shared color choice for the whole activation.
#[test]
fn charge_counter_mana_keeps_one_color() {
    let mana = parse_tap_yield(
        "Choose a color. Add one mana of that color for each charge counter on this artifact.",
    )
    .expect("counter-scaled mana");
    assert_eq!(mana.scaling, Some(Scale::PerChargeCounter));
    assert_eq!(mana.any_pips, 1);
    assert!(mana.one_color);
    let colors =
        parse_tap_yield("For each color among permanents you control, add one mana of that color.")
            .expect("supported permanent-color scaling");
    assert_eq!(colors.scaling, Some(Scale::ColorsPresent));
}

/// Only complete controlled-land conditions become typed gates.
#[test]
fn controlled_land_types_require_the_complete_supported_condition() {
    for (condition, expected) in [
        ("you control a Plains", [true, false, false, false, false]),
        ("you control an Island", [false, true, false, false, false]),
        (
            "you control a Plains or an Island",
            [true, true, false, false, false],
        ),
        (
            "you control a Swamp or a Forest",
            [false, false, true, false, true],
        ),
    ] {
        assert_eq!(parse_controlled_land_types(condition), Some(expected));
    }
    for condition in [
        "you control a Plains and an Island",
        "you control another Plains or an Island",
        "you control a Plains or another Island",
        "you control a Plains or an Island and an Elf",
        "you control a Plains or an Island or a Swamp",
        "you control an Elf or a Forest",
        "you control a Forest card in your graveyard",
        "you control a nonbasic Forest",
        "you control a Forest and have 10 or more life",
        "you control a Island",
        "you control ",
    ] {
        assert_eq!(parse_controlled_land_types(condition), None, "{condition}");
    }
}
