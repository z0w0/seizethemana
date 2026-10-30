use crate::deck::io_external::{DeckFormat, detect, parse_external, render};

const MOXFIELD: &str =
    "//Deck\n2x Rhystic Study (scg) 105 *F*\n1x Island (mh3) 274\n\n//Sideboard\n1x Fog\n";
const ARCHIDEKT: &str = "// Maindeck\n2x Rhystic Study [scg] 105 *F*\n1x Island [mh3] 274\n";
const ARENA: &str = "Commander\n1 Breya\n\nDeck\n2 Island (mh3) 274\n1 Bolt\n\nSideboard\n1 Fog\n";
const NAMES: &str = "2 Island\n1 Bolt\n";

#[test]
fn detects_each_format() {
    assert_eq!(detect(MOXFIELD), DeckFormat::Moxfield);
    assert_eq!(detect(ARCHIDEKT), DeckFormat::Archidekt);
    assert_eq!(detect(ARENA), DeckFormat::Arena);
    assert_eq!(
        detect(NAMES),
        DeckFormat::ManaBox,
        "names falls back to the native grammar"
    );
    assert_eq!(
        detect("// COMMANDER\n1 Breya (MH3) 372 *F*\n\n// DECK\n2 Island (SOS) 274\n"),
        DeckFormat::ManaBox
    );
}

#[test]
fn native_export_auto_detects_deck_and_sideboard_sections() {
    use crate::deck::Deck;

    let deck_only = Deck::parse("// DECK\n2 Lightning Bolt\n").unwrap();
    let text = render(&deck_only, DeckFormat::ManaBox);
    assert_eq!(detect(&text), DeckFormat::ManaBox);
    assert_eq!(parse_external(&text, detect(&text)).unwrap(), deck_only);

    let with_sideboard = Deck::parse("// DECK\n2 Lightning Bolt\n// SIDEBOARD\n1 Fog\n").unwrap();
    let text = render(&with_sideboard, DeckFormat::ManaBox);
    assert_eq!(detect(&text), DeckFormat::ManaBox);
    let imported = parse_external(&text, detect(&text)).unwrap();
    assert_eq!(imported, with_sideboard);
    assert_eq!(imported.sideboard_total(), 1);

    let with_commander = Deck::parse(
        "// COMMANDER\n1 Breya (MH3) 372 *F*\n// DECK\n2 Island (MH3) 274\n// SIDEBOARD\n1 Fog\n// MAYBEBOARD\n1 Lightning Bolt\n",
    ).unwrap();
    let text = render(&with_commander, DeckFormat::ManaBox);
    assert_eq!(detect(&text), DeckFormat::ManaBox);
    assert_eq!(
        parse_external(&text, detect(&text)).unwrap(),
        with_commander
    );
}

#[test]
fn parses_moxfield_shape() {
    let deck = parse_external(MOXFIELD, DeckFormat::Moxfield).unwrap();
    assert_eq!(deck.section_index("DECK"), Some(0));
    assert_eq!(deck.total(), 4);
    let entries = &deck.sections[0].1;
    assert_eq!(entries[0].name, "Rhystic Study");
    assert_eq!(entries[0].quantity, 2);
    assert_eq!(entries[0].set_code.as_deref(), Some("SCG"));
    assert_eq!(entries[0].collector_number.as_deref(), Some("105"));
    assert!(entries[0].foil);
    assert_eq!(entries[1].set_code.as_deref(), Some("MH3"));
}

#[test]
fn parses_archidekt_brackets() {
    let deck = parse_external(ARCHIDEKT, DeckFormat::Archidekt).unwrap();
    assert_eq!(deck.total(), 3);
    let entries = &deck.sections[0].1;
    assert_eq!(entries[0].set_code.as_deref(), Some("SCG"));
    assert_eq!(entries[1].set_code.as_deref(), Some("MH3"));
    assert!(entries[0].foil);
}

#[test]
fn parses_arena_headers() {
    let deck = parse_external(ARENA, DeckFormat::Arena).unwrap();
    assert_eq!(deck.section_index("COMMANDER"), Some(0));
    assert_eq!(deck.section_index("DECK"), Some(1));
    assert_eq!(deck.section_index("SIDEBOARD"), Some(2));
    assert_eq!(deck.total(), 5);
}

#[test]
fn parses_names_lines() {
    let deck = parse_external(NAMES, DeckFormat::Names).unwrap();
    assert_eq!(deck.total(), 3);
    assert!(deck.sections.iter().all(|(s, _)| s == "DECK"));
    assert!(deck.sections[0].1.iter().all(|e| e.set_code.is_none()));
}

#[test]
fn names_with_brackets_stay_names() {
    // A card name containing brackets must not lose them when the bracket
    // content is not set-shaped (too long for a set code).
    let deck = parse_external("1x Crocodile [piranha] Plant", DeckFormat::Archidekt).unwrap();
    let entry = &deck.sections[0].1[0];
    assert_eq!(entry.set_code, None, "[piranha] is too long for a set code");
    assert_eq!(entry.name, "Crocodile [piranha] Plant");

    // A bracket tail at a valid set shape does split.
    let deck = parse_external("1x Crocodile [pir] Plant", DeckFormat::Archidekt).unwrap();
    let entry = &deck.sections[0].1[0];
    assert_eq!(entry.set_code.as_deref(), Some("PIR"));
    assert_eq!(entry.collector_number.as_deref(), Some("Plant"));
}

#[test]
fn renders_each_format() {
    let deck = parse_external(MOXFIELD, DeckFormat::Moxfield).unwrap();
    let moxfield = render(&deck, DeckFormat::Moxfield);
    assert!(moxfield.contains("2x Rhystic Study (scg) 105 *F*"));
    let archidekt = render(&deck, DeckFormat::Archidekt);
    assert!(archidekt.contains("2x Rhystic Study [scg] 105 *F*"));
    let names = render(&deck, DeckFormat::Names);
    assert!(names.contains("2 Rhystic Study"));
    let arena = render(&deck, DeckFormat::Arena);
    assert!(arena.contains("Deck\n"));
}

#[test]
fn moxfield_round_trips() {
    let deck = parse_external(MOXFIELD, DeckFormat::Moxfield).unwrap();
    let text = render(&deck, DeckFormat::Moxfield);
    let reparsed = parse_external(&text, DeckFormat::Moxfield).unwrap();
    assert_eq!(deck, reparsed);
}

#[test]
fn archidekt_round_trips() {
    let deck = parse_external(ARCHIDEKT, DeckFormat::Archidekt).unwrap();
    let text = render(&deck, DeckFormat::Archidekt);
    let reparsed = parse_external(&text, DeckFormat::Archidekt).unwrap();
    assert_eq!(deck, reparsed);
}

#[test]
fn format_keys_parse() {
    for key in DeckFormat::all_keys() {
        assert!(DeckFormat::parse(key).is_some(), "{key}");
    }
    assert!(DeckFormat::parse("nope").is_none());
}

/// `# Maindeck`-style headers map through the known vocabulary; unknown
/// `#` lines are comments, not errors.
#[test]
fn hash_headers_and_comments() {
    let deck = parse_external(
        "# Maindeck\n2x Island [mh3] 1\n# a plain comment\n",
        DeckFormat::Archidekt,
    )
    .unwrap();
    assert_eq!(deck.section_index("DECK"), Some(0));
    assert_eq!(deck.total(), 2);
    // Arena ignores `#` lines entirely.
    let deck = parse_external("Deck\n2 Island\n# comment\n", DeckFormat::Arena).unwrap();
    assert_eq!(deck.total(), 2);
}

/// Every import format must use the native quantity limit.
#[test]
fn all_formats_share_entry_quantity_limits() {
    let formats = [
        (DeckFormat::ManaBox, "// DECK\n", ""),
        (DeckFormat::Moxfield, "//Deck\n", "x"),
        (DeckFormat::Archidekt, "// Maindeck\n", "x"),
        (DeckFormat::Arena, "Deck\n", ""),
        (DeckFormat::Names, "", ""),
    ];
    for (format, header, suffix) in formats {
        for quantity in [i64::MIN, -1, 0, 10_000, i64::MAX] {
            let text = format!("{header}{quantity}{suffix} Island\n");
            assert!(parse_external(&text, format).is_err(), "{format:?}: {text}");
        }
        let text = format!("{header}9999{suffix} Island\n");
        let deck = parse_external(&text, format).expect("maximum allowed quantity");
        assert_eq!(deck.total(), crate::deck::grammar::MAX_ENTRY_QUANTITY);
        assert_eq!(
            crate::deck::Deck::parse(&deck.to_text()).expect("readable native deck"),
            deck
        );
    }
}
