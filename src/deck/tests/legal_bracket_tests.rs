use super::*;

#[test]
fn game_changer_limit_matches_brackets() {
    assert_eq!(game_changer_limit(1), Some(0));
    assert_eq!(game_changer_limit(2), Some(0));
    assert_eq!(game_changer_limit(3), Some(3));
    assert_eq!(game_changer_limit(4), None);
    assert_eq!(game_changer_limit(5), None);
}

#[test]
fn bracket_notes_carry_a_checklist() {
    let note = bracket_note(2).expect("bracket 2 has a checklist");
    assert!(note.checks.iter().any(|c| c.contains("combos")));
    assert!(bracket_note(5).is_none(), "bracket 5 has no checklist");
}

#[test]
fn bracket_scan_flags_tutors_and_extra_turns() {
    let cards: HashMap<String, CardRow> = [
        card(
            "Demonic Tutor",
            "Sorcery",
            "B",
            "Search your library for a card, put it into your hand.",
        ),
        card(
            "Time Warp",
            "Sorcery",
            "U",
            "Take an extra turn after this one.",
        ),
        card(
            "Obliterate",
            "Sorcery",
            "R",
            "Destroy all lands, all artifacts.",
        ),
        card("Bear", "Creature — Bear", "G", "Just a bear."),
    ]
    .into_iter()
    .map(|c| (c.name.clone(), c))
    .collect();
    let deck = Deck::parse(
        "// COMMANDER\n1 Bear\n\n// DECK\n1 Demonic Tutor\n1 Time Warp\n1 Obliterate\n",
    )
    .unwrap();
    let checks = scan_bracket_signals(&deck, &cards, 2);
    assert!(
        checks
            .iter()
            .any(|c| c.starts_with("CHECK library search:") && c.contains("Demonic Tutor"))
    );
    assert!(
        checks
            .iter()
            .any(|c| c.starts_with("CHECK extra turns") && c.contains("Time Warp"))
    );
    assert!(
        checks
            .iter()
            .any(|c| c.starts_with("CHECK mass land destruction"))
    );
    // The clean card's section passes.
    assert!(
        !checks
            .iter()
            .any(|c| c.starts_with("PASS") && c.contains("library search"))
    );
}

#[test]
fn bracket_scan_passes_clean_decks() {
    let cards: HashMap<String, CardRow> = [card("Bear", "Creature — Bear", "G", "Vanilla.")]
        .into_iter()
        .map(|c| (c.name.clone(), c))
        .collect();
    let deck = Deck::parse("// COMMANDER\n1 Bear\n\n// DECK\n10 Forest\n").unwrap();
    let checks = scan_bracket_signals(&deck, &cards, 2);
    assert!(checks.iter().all(|c| c.starts_with("PASS ")));
    assert_eq!(checks.len(), 4);
}

#[test]
fn bracket_scan_splits_land_ramp_from_tutors() {
    let cards: HashMap<String, CardRow> = [
        card(
            "Demonic Tutor",
            "Sorcery",
            "B",
            "Search your library for a card, put it into your hand.",
        ),
        card(
            "Cultivate",
            "Sorcery",
            "G",
            "Search your library for up to two basic lands, put one onto the battlefield tapped and the other into your hand, then shuffle.",
        ),
        card(
            "Farseek",
            "Sorcery",
            "G",
            "Search your library for a Plains, Island, Swamp, Mountain, or Forest card, put it onto the battlefield tapped, then shuffle.",
        ),
        card(
            "Myriad Landscape",
            "Land",
            "",
            "{T}: Add {C}.\n{2}, {T}: Search your library for a basic land that shares a land type with this land, put it onto the battlefield tapped, then shuffle.",
        ),
        card("Bear", "Creature — Bear", "G", "Just a bear."),
    ]
    .into_iter()
    .map(|c| (c.name.clone(), c))
    .collect();
    let deck = Deck::parse(
        "// COMMANDER\n1 Bear\n\n// DECK\n1 Demonic Tutor\n1 Cultivate\n1 Farseek\n1 Myriad Landscape\n",
    )
    .unwrap();
    let checks = scan_bracket_signals(&deck, &cards, 2);
    // Only the nonland tutor trips the CHECK.
    let tutors = checks
        .iter()
        .find(|c| c.starts_with("CHECK library search:"))
        .expect("library search check present");
    assert!(tutors.contains("Demonic Tutor"));
    assert!(!tutors.contains("Cultivate"));
    assert!(!tutors.contains("Farseek"));
    // Land ramp gets its own note line, listed once each.
    let ramp = checks
        .iter()
        .find(|c| c.starts_with("note ramp:"))
        .expect("ramp note present");
    assert!(ramp.contains("Cultivate"));
    assert!(ramp.contains("Farseek"));
    assert!(ramp.contains("Myriad Landscape"));
    assert!(!ramp.contains("Demonic Tutor"));
}
