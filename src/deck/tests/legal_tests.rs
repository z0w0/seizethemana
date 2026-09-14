use super::*;

/// Test card row; `identity` is a plain letters string ("WUB") stored
/// as a JSON array like the oracle does.
fn card(name: &str, type_line: &str, identity: &str, text: &str) -> CardRow {
    let identity_json = serde_json::to_string(
        &identity
            .chars()
            .map(|c| c.to_string())
            .collect::<Vec<String>>(),
    )
    .unwrap();
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: String::new(),
        cmc: 0.0,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: identity_json,
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: r#"{"commander":"legal","modern":"legal"}"#.into(),
        set_code: "TST".into(),
        collector_number: "1".into(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

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
            .any(|c| c.starts_with("CHECK tutors:") && c.contains("Demonic Tutor"))
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
            .any(|c| c.starts_with("PASS") && c.contains("tutors"))
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
        .find(|c| c.starts_with("CHECK tutors:"))
        .expect("tutor check present");
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

#[test]
fn infer_format_from_sections() {
    let mut deck = Deck::parse("// COMMANDER\n1 Breya\n").unwrap();
    assert!(matches!(infer_format(&deck), InferredFormat::Commander));
    deck = Deck::parse("// DECK\n4 Bolt\n").unwrap();
    assert!(matches!(infer_format(&deck), InferredFormat::Constructed));
}

#[test]
fn commander_count_and_partner_rules() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card(
            "Breya",
            "Legendary Creature — Human Artificer",
            "WUB",
            "Partner with Bruse Tarl",
        ),
    );
    cards.insert(
        "Bruse".to_string(),
        card(
            "Bruse",
            "Legendary Creature — Human",
            "RW",
            "Partner with Breya",
        ),
    );
    cards.insert(
        "Solo".to_string(),
        card("Solo", "Legendary Creature — Cat", "W", ""),
    );
    cards.insert(
        "NotLegendary".to_string(),
        card("NotLegendary", "Creature — Cat", "W", ""),
    );
    // Two partners: legal.
    assert!(commander_legal(&["Breya".into(), "Bruse".into()], &cards).is_none());
    // One legendary: legal.
    assert!(commander_legal(&["Solo".to_string()], &cards).is_none());
    // Non-legendary commander: violation.
    let v = commander_legal(&["NotLegendary".to_string()], &cards).unwrap();
    assert_eq!(v.rule, "commander");
    // Two without partner: violation.
    assert!(commander_legal(&["Solo".to_string(), "Breya".to_string()], &cards).is_some());
    // Three commanders: violation.
    assert!(
        commander_legal(
            &["Solo".to_string(), "Breya".to_string(), "Bruse".to_string()],
            &cards
        )
        .is_some()
    );
}

#[test]
fn spacecraft_with_pt_box_commands() {
    let mut cards = HashMap::new();
    let mut ig_station = card(
        "IGS",
        "Legendary Artifact — Spacecraft",
        "WUBRG",
        "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft.)",
    );
    ig_station.power = Some("7".into());
    ig_station.toughness = Some("15".into());
    cards.insert("IGS".to_string(), ig_station);
    cards.insert("Bolt".to_string(), card("Bolt", "Instant", "R", "Deal 3."));
    let deck = Deck::parse("// COMMANDER\n1 IGS\n// DECK\n1 Bolt\n").unwrap();
    // Legal commander type; identity WUBRG covers Bolt's R. The tiny
    // deck still trips deck size, so check that no commander violation
    // appears.
    let violations = check(&deck, &cards, Some("commander"), None);
    assert!(
        violations.iter().all(|v| v.rule != "commander"),
        "Spacecraft with P/T must command: {violations:?}"
    );

    // Same type line without a P/T box: not a commander.
    let mut no_pt = card("IGS", "Legendary Artifact — Spacecraft", "WUBRG", "");
    no_pt.power = None;
    no_pt.toughness = None;
    cards.insert("IGS".to_string(), no_pt);
    let v = check(&deck, &cards, Some("commander"), None)
        .iter()
        .find(|v| v.rule == "commander")
        .expect("Spacecraft without P/T must not command")
        .clone();
    assert_eq!(v.cards, vec!["IGS".to_string()]);
    assert!(v.detail.contains("Vehicle/Spacecraft"));
}

#[test]
fn vehicle_commander_rules() {
    let mut cards = HashMap::new();
    let mut vehicle = card("Parhelion", "Legendary Artifact — Vehicle", "W", "Crew 4");
    vehicle.power = Some("4".into());
    vehicle.toughness = Some("6".into());
    cards.insert("Parhelion".to_string(), vehicle);
    cards.insert("Bolt".to_string(), card("Bolt", "Instant", "R", "Deal 3."));

    // Legendary Vehicle with P/T commands (deck size aside).
    let deck = Deck::parse("// COMMANDER\n1 Parhelion\n// DECK\n1 Bolt\n").unwrap();
    let violations = check(&deck, &cards, Some("commander"), None);
    assert!(
        violations.iter().all(|v| v.rule != "commander"),
        "Vehicle with P/T must command: {violations:?}"
    );

    // A non-legendary Vehicle never commands.
    let mut plain = card("Parhelion", "Artifact — Vehicle", "W", "Crew 4");
    plain.power = Some("4".into());
    plain.toughness = Some("6".into());
    cards.insert("Parhelion".to_string(), plain);
    let v = check(&deck, &cards, Some("commander"), None)
        .iter()
        .find(|v| v.rule == "commander")
        .expect("non-legendary Vehicle must not command")
        .clone();
    assert_eq!(v.cards, vec!["Parhelion".to_string()]);

    // Color identity still applies to the Spacecraft/Vehicle commander.
    let mut ubs_ship = card("Ship", "Legendary Artifact — Spacecraft", "UB", "");
    ubs_ship.power = Some("2".into());
    ubs_ship.toughness = Some("2".into());
    cards.insert("Ship".to_string(), ubs_ship);
    let deck = Deck::parse("// COMMANDER\n1 Ship\n// DECK\n1 Bolt\n").unwrap();
    let identity_v = check(&deck, &cards, Some("commander"), None)
        .into_iter()
        .find(|v| v.rule == "commander color identity")
        .expect("R pips fall outside UB identity");
    assert_eq!(identity_v.cards, vec!["Bolt".to_string()]);
}

#[test]
fn color_identity_subset_check() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card("Breya", "Legendary Creature — Human", "WUB", ""),
    );
    cards.insert("Bolt".to_string(), card("Bolt", "Instant", "R", "Deal 3."));
    cards.insert(
        "Birds".to_string(),
        card("Birds", "Creature — Bird", "WU", ""),
    );
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n1 Bolt\n1 Birds\n").unwrap();
    let violations = check(&deck, &cards, Some("commander"), None);
    let identity_v = violations
        .iter()
        .find(|v| v.rule == "commander color identity")
        .expect("Bolt is outside WUB identity");
    assert_eq!(identity_v.cards, vec!["Bolt"]);
}

#[test]
fn singleton_limit_excepts_basics_and_oracle_text() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card("Breya", "Legendary Creature — Human", "WUB", ""),
    );
    cards.insert(
        "Plains".to_string(),
        card("Plains", "Basic Land — Plains", "", ""),
    );
    cards.insert(
        "Rats".to_string(),
        card(
            "Rats",
            "Creature — Rat",
            "B",
            "A deck can have any number of cards named Relentless Rats.",
        ),
    );
    let deck =
        Deck::parse("// COMMANDER\n1 Breya\n// DECK\n30 Plains\n1 Rats\n1 Sol Ring\n").unwrap();
    cards.insert(
        "Sol Ring".to_string(),
        card("Sol Ring", "Artifact", "", "{T}: Add {C}{C}."),
    );
    let violations = check(&deck, &cards, Some("commander"), None);
    // 30 Plains and 1 Rats pass; no singleton violation at all.
    assert!(
        violations.iter().all(|v| v.rule != "singleton"),
        "singleton violations: {violations:?}"
    );
    // Deck size is 32, not 100: that violation exists.
    assert!(violations.iter().any(|v| v.rule == "deck size"));
}

#[test]
fn commander_size_ignores_sideboard() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card("Breya", "Legendary Creature — Human", "WUB", ""),
    );
    cards.insert("Bolt".to_string(), card("Bolt", "Instant", "R", "Deal 3."));
    // 100 cards maindeck (commander + 99) plus 3 sideboard cards: legal
    // size. The sideboard is a commander wishlist, not a legal zone.
    let deck =
        Deck::parse("// COMMANDER\n1 Breya\n// DECK\n99 Bolt\n// SIDEBOARD\n2 Bolt\n1 Bolt\n")
            .unwrap();
    let violations = check(&deck, &cards, Some("commander"), None);
    assert!(
        violations.iter().all(|v| v.rule != "deck size"),
        "sideboard must not count toward commander size: {violations:?}"
    );
    // 99 maindeck + 0 sideboard is one short: still a violation.
    let short = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n98 Bolt\n").unwrap();
    let violations = check(&short, &cards, Some("commander"), None);
    assert!(violations.iter().any(|v| v.rule == "deck size"));
    // The summary line separates the sideboard from the deck count.
    let s = summary_line(&deck, &cards);
    assert!(s.contains("100 cards"), "summary was {s}");
    assert!(s.contains("+ 3 sideboard"), "summary was {s}");
}

#[test]
fn game_changer_count_violates_bracket_3() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card("Breya", "Legendary Creature — Human", "WUB", ""),
    );
    cards.insert(
        "Plains".to_string(),
        card("Plains", "Basic Land — Plains", "", ""),
    );
    for (i, name) in ["GC One", "GC Two", "GC Three", "GC Four"]
        .iter()
        .enumerate()
    {
        let mut gc = card(name, "Instant", "U", "Strong effect.");
        gc.game_changer = Some(true);
        gc.legalities = r#"{"commander":"legal"}"#.into();
        let _ = i;
        cards.insert(name.to_string(), gc);
    }
    // 4 Game Changers + commander + 95 Plains = 100 cards.
    let deck = Deck::parse(
        "// COMMANDER\n1 Breya\n// DECK\n95 Plains\n1 GC One\n1 GC Two\n1 GC Three\n1 GC Four\n",
    )
    .unwrap();
    let violations = check(&deck, &cards, Some("commander"), Some(3));
    let gc_v = violations
        .iter()
        .find(|v| v.rule == "game changers")
        .expect("4 Game Changers exceed bracket 3's limit of 3");
    assert_eq!(gc_v.cards.len(), 4);
    // Bracket 4 allows any number.
    let violations = check(&deck, &cards, Some("commander"), Some(4));
    assert!(violations.iter().all(|v| v.rule != "game changers"));
}

#[test]
fn sideboard_game_changers_ignore_bracket_count() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card("Breya", "Legendary Creature — Human", "WUB", ""),
    );
    cards.insert(
        "Plains".to_string(),
        card("Plains", "Basic Land — Plains", "", ""),
    );
    for name in ["GC One", "GC Two", "GC Three", "GC Four"] {
        let mut gc = card(name, "Instant", "U", "Strong effect.");
        gc.game_changer = Some(true);
        gc.legalities = r#"{"commander":"legal"}"#.into();
        cards.insert(name.to_string(), gc);
    }
    // 3 maindeck Game Changers (at the bracket-3 limit) plus 2 more in
    // the sideboard. The sideboard is a wishlist, so the deck is legal.
    let deck = Deck::parse(
        "// COMMANDER\n1 Breya\n// DECK\n95 Plains\n1 GC One\n1 GC Two\n1 GC Three\n\
         // SIDEBOARD\n1 GC Four\n1 GC Four\n",
    )
    .unwrap();
    let violations = check(&deck, &cards, Some("commander"), Some(3));
    assert!(
        violations.iter().all(|v| v.rule != "game changers"),
        "sideboard Game Changers must not count: {violations:?}"
    );
    // The no-bracket checklist reports maindeck Game Changers only.
    let checklist = game_changer_checklist(&deck, &cards);
    assert!(checklist[0].contains("GC One"));
    assert!(
        !checklist[0].contains("GC Four"),
        "checklist was {checklist:?}"
    );
}

#[test]
fn banned_cards_fail_the_format() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card("Breya", "Legendary Creature — Human", "WUB", ""),
    );
    cards.insert(
        "Plains".to_string(),
        card("Plains", "Basic Land — Plains", "", ""),
    );
    let mut banned_card = card("Banned Card", "Instant", "W", "Text.");
    banned_card.legalities = r#"{"commander":"banned"}"#.into();
    cards.insert("Banned Card".to_string(), banned_card);
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n99 Plains\n1 Banned Card\n").unwrap();
    let violations = check(&deck, &cards, Some("commander"), None);
    assert!(violations.iter().any(|v| v.rule == "banned"));
}

#[test]
fn unknown_names_are_reported() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card("Breya", "Legendary Creature — Human", "WUB", ""),
    );
    cards.insert(
        "Plains".to_string(),
        card("Plains", "Basic Land — Plains", "", ""),
    );
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n99 Plains\n1 Ghost Card\n").unwrap();
    let violations = check(&deck, &cards, Some("commander"), None);
    let v = violations
        .iter()
        .find(|v| v.rule == "unknown cards")
        .expect("unknown names reported");
    assert_eq!(v.cards, vec!["Ghost Card"]);
}

#[test]
fn unknown_names_are_deduped() {
    let mut cards = HashMap::new();
    cards.insert(
        "Breya".to_string(),
        card("Breya", "Legendary Creature — Human", "WUB", ""),
    );
    cards.insert(
        "Plains".to_string(),
        card("Plains", "Basic Land — Plains", "", ""),
    );
    let deck = Deck::parse("// COMMANDER\n1 Breya\n// DECK\n90 Plains\n3 Ghost Card\n").unwrap();
    let violations = check(&deck, &cards, Some("commander"), None);
    let v = violations
        .iter()
        .find(|v| v.rule == "unknown cards")
        .expect("unknown names reported");
    // Three copies of the unknown name list it once.
    assert_eq!(v.cards, vec!["Ghost Card"]);
}

#[test]
fn constructed_checks_sizes_and_copies() {
    let mut cards = HashMap::new();
    cards.insert("Bolt".to_string(), card("Bolt", "Instant", "R", "Deal 3."));
    // 61 cards: 60 maindeck (5 Bolt × 12 = 60) plus a 5-card sideboard.
    let deck = Deck::parse("// DECK\n10 Bolt\n1 Bolt\n// SIDEBOARD\n2 Bolt\n").unwrap();
    let violations = check(&deck, &cards, Some("modern"), None);
    // 13 Bolt copies trip the copy limit.
    assert!(violations.iter().any(|v| v.rule == "copy limit"));
    // Maindeck 11 + sideboard 2 = 13 total, under 60: deck size violation.
    assert!(violations.iter().any(|v| v.rule == "deck size"));
}
