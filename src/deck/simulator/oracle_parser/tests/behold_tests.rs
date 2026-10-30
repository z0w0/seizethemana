use super::*;
use crate::deck::simulator::model::SearchCardType;

/// Quality parsing keeps subtype and card-type requirements separate.
#[test]
fn qualities_preserve_the_complete_written_conjunction() {
    for (source, card_types, subtypes) in [
        ("an Elf", Vec::new(), vec!["elf"]),
        ("a Dragon", Vec::new(), vec!["dragon"]),
        ("a Dragon creature", vec!["creature"], vec!["dragon"]),
        ("a Gamma creature", vec!["creature"], vec!["gamma"]),
        ("a Jace planeswalker", vec!["planeswalker"], vec!["jace"]),
        ("a planeswalker", vec!["planeswalker"], Vec::new()),
    ] {
        let spec = parse_quality(source).expect("the complete quality has a typed form");
        assert_eq!(spec.card_types, card_types, "{source}");
        assert_eq!(spec.subtypes, subtypes, "{source}");
        assert_eq!(spec.count, 1, "{source}");
    }
}

/// Counted wording stays counted even though runtime selection cannot yet execute it.
#[test]
fn counted_qualities_do_not_become_singular_choices() {
    for (source, subtype, count) in [
        ("three Elementals", "elemental", 3),
        ("two Elves", "elf", 2),
        ("two Dragons", "dragon", 2),
    ] {
        let spec = parse_quality(source).expect("a fixed count can be retained in the AST");
        assert_eq!(spec.subtypes, [subtype]);
        assert_eq!(spec.count, count);
    }
}

/// Extra restrictions cannot silently disappear from the parsed quality.
#[test]
fn qualities_reject_nearby_unsupported_grammar() {
    for source in [
        "a Dragon or an Elf",
        "a Dragon you control",
        "a red Dragon",
        "a Dragon with flying",
        "a nonland creature",
        "a card",
        "a creature of the chosen type",
        "a Dragon and exile it",
        "zero Dragons",
        "",
    ] {
        assert!(parse_quality(source).is_none(), "{source}");
    }
}

/// Every supported cost keeps its exact generic payment and Behold subtype.
#[test]
fn additional_cost_alternatives_accept_the_printed_payment_forms() {
    for (quality, subtype, generic) in [
        ("a Dragon", "dragon", 1),
        ("a Jace", "jace", 1),
        ("a Kithkin", "kithkin", 2),
        ("an Elf", "elf", 2),
        ("a Goblin", "goblin", 2),
        ("a Merfolk", "merfolk", 2),
        ("an Elemental", "elemental", 2),
    ] {
        let source = format!(
            "As an additional cost to cast this spell, behold {quality} or pay {{{generic}}}. (To behold, choose or reveal a matching card.)"
        );
        let alternative = additional_cost_alternative(&source)
            .expect("the complete singular alternative is supported");
        assert_eq!(alternative.spec.subtypes, [subtype]);
        assert_eq!(alternative.spec.count, 1);
        assert_eq!(alternative.generic, generic);
    }
    let source = "As an additional cost to cast this spell, behold a Dragon or pay {1}. Target creature gets -3/-3 until end of turn.";
    assert_eq!(
        additional_cost_alternative(source)
            .expect("a following resolution does not change the cost")
            .generic,
        1
    );
}

/// Required exile, chosen types, and optional cast rewards are not free alternatives.
#[test]
fn additional_cost_alternatives_reject_unsupported_payments() {
    for source in [
        "As an additional cost to cast this spell, behold an Elf and exile it.",
        "As an additional cost to cast this spell, you may behold a Dragon.",
        "As an additional cost to cast this spell, behold a Dragon or pay {R}.",
        "As an additional cost to cast this spell, behold a Dragon or pay {1}{1}.",
        "As an additional cost to cast this spell, behold a Dragon or pay {1} and discard a card.",
        "As an additional cost to cast this spell, you may choose a creature type and behold two creatures of that type.",
        "Flashback—{1}{R}, Behold three Elementals.",
    ] {
        assert!(additional_cost_alternative(source).is_none(), "{source}");
    }
}

/// An optional resolution reward remains nested under its own Behold instruction.
#[test]
fn conditional_rewards_keep_the_gate_and_typed_effects() {
    let parsed = conditional_reward(
        "You may behold a Dragon. If you do, create a Treasure token. (Choose a Dragon you control or reveal a Dragon card from your hand.)",
    )
    .expect("the complete optional Behold has a conditional node");
    let OracleEffect::Behold { spec, effects } = parsed else {
        panic!("the Treasure reward must remain gated");
    };
    assert_eq!(spec.subtypes, ["dragon"]);
    assert!(matches!(
        effects.as_slice(),
        [OracleEffect::CreateTreasureTokens(1)]
    ));
}

/// A known Behold gate does not make an unknown dependent effect executable.
#[test]
fn conditional_unknown_rewards_remain_explicitly_unsupported() {
    for source in [
        "You may behold a planeswalker. If you do, it perpetually gains a loyalty ability.",
        "You may behold a Dragon. If you do, that permanent gains \"Create a Treasure token.\"",
        "You may behold a Dragon. If you do, create a Treasure token and exile the beheld card.",
    ] {
        let parsed = conditional_reward(source).expect("the Behold gate itself has typed syntax");
        let OracleEffect::Behold { effects, .. } = parsed else {
            panic!("the unsupported effect must remain within the Behold gate");
        };
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, OracleEffect::Unsupported(_)))
        );
    }
}

/// Elven Passage's search and optional untap form one result-linked operation.
#[test]
fn fetch_untap_keeps_the_original_search_and_quality() {
    let source = "Search your library for a basic land card, put it onto the battlefield tapped, then shuffle. You may behold an Elf. If you do, untap that land.";
    let parsed = fetch_untap_annotation(source).expect("the entire linked fetch has a typed form");
    let OracleEffect::SearchThenBeholdUntap { search, spec } = parsed else {
        panic!("the untap must remain attached to its own search result");
    };
    assert_eq!(search.card_type, Some(SearchCardType::BasicLand));
    assert_eq!(search.destination, SearchDestination::BattlefieldTapped);
    assert!(search.basic_land_only);
    assert!(search.shuffle);
    assert_eq!(spec.subtypes, ["elf"]);
}

/// Other objects, extra rewards, and nonbattlefield results are not this fetch form.
#[test]
fn fetch_untap_rejects_nearby_unlinked_instructions() {
    for source in [
        "Search your library for a basic land card, put it into your hand, then shuffle. You may behold an Elf. If you do, untap that land.",
        "Search your library for a basic land card, put it onto the battlefield tapped, then shuffle. You may behold an Elf. If you do, untap target land.",
        "Search your library for a basic land card, put it onto the battlefield tapped, then shuffle. You may behold an Elf. If you do, untap that land and draw a card.",
    ] {
        assert!(fetch_untap_annotation(source).is_none(), "{source}");
        assert!(
            matches!(parse_effect(source), Some(OracleEffect::Unsupported(_))),
            "{source}"
        );
    }
}

/// The Sanctum's optional Behold replacement is distinct from a triggered effect.
#[test]
fn land_entry_condition_requires_the_full_failure_clause() {
    let source = "As this land enters, you may behold a Jace. If you don't, this land enters tapped. (To behold a Jace, choose a Jace you control or reveal a Jace card from your hand.)";
    let spec = land_entry_condition(source).expect("the complete entry replacement is supported");
    assert_eq!(spec.subtypes, ["jace"]);
    assert_eq!(spec.count, 1);
    for source in [
        "As this land enters, you may behold a Jace. If you don't, sacrifice it.",
        "As this land enters, you may behold a Jace. If you do, this land enters tapped.",
        "As this land enters, you may behold a Jace. If you don't, this land enters tapped and you lose 1 life.",
    ] {
        assert!(land_entry_condition(source).is_none(), "{source}");
    }
}

/// Unhandled mandatory and cast-linked wording cannot fall through to free effects.
#[test]
fn resolution_claims_unsupported_behold_wording_atomically() {
    for source in [
        "Behold an Elf and exile it. Draw a card.",
        "You may behold a Dragon. If you don't, create a Treasure token.",
        "As an additional cost to cast this spell, you may behold a Hero. Destroy target tapped creature. If a Hero was beheld, scry 2.",
        "Flashback—{1}{R}, Behold three Elementals.",
        "Choose a creature type and behold two creatures of that type.",
    ] {
        assert!(
            matches!(parse_effect(source), Some(OracleEffect::Unsupported(_))),
            "{source}"
        );
    }
    assert!(parse_effect("Draw a card.").is_none());
    assert!(parse_effect("Beholder deals 2 damage to each opponent.").is_none());
}
