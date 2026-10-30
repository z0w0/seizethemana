use super::*;

/// A name-only move keeps every source print field when it removes the line.
#[test]
fn name_only_move_preserves_full_source_entry() {
    let mut deck = Deck::parse("// DECK\n1 Sol Ring (CMM) 396 *F*\n").expect("parse deck");
    let source = deck.entries().next().expect("source entry").clone();
    let op = parse_op("move", "1 Sol Ring to:sideboard").expect("parse move");

    let summary = apply_ops(&mut deck, &[op]).expect("move card");

    assert_eq!(summary.moved, 1);
    assert!(summary.missing.is_empty());
    assert!(deck.section_entries_mut("DECK").is_empty());
    assert_eq!(deck.section_entries_mut("SIDEBOARD"), &[source]);
    assert_eq!(deck.total(), 1);
}

/// Partial moves keep the same printing and finish on both remaining lines.
#[test]
fn partial_move_preserves_source_entry_and_actual_quantity() {
    let mut deck = Deck::parse("// DECK\n3 Sol Ring (CMM) 396 *F*\n").expect("parse deck");
    let mut moved = deck.entries().next().expect("source entry").clone();
    moved.quantity = 2;
    let mut remaining = moved.clone();
    remaining.quantity = 1;
    let op = parse_op("move", "2 Sol Ring to:sideboard").expect("parse move");

    let summary = apply_ops(&mut deck, &[op]).expect("move card");

    assert_eq!(summary.moved, 2);
    assert_eq!(deck.section_entries_mut("DECK"), &[remaining]);
    assert_eq!(deck.section_entries_mut("SIDEBOARD"), &[moved]);
    assert_eq!(deck.total(), 3);
}

/// An oversized move reports and transfers only the copies held by the source.
#[test]
fn oversized_move_preserves_source_print_and_clamps_quantity() {
    let mut deck = Deck::parse("// DECK\n1 Sol Ring (CMM) 396 *F*\n").expect("parse deck");
    let source = deck.entries().next().expect("source entry").clone();
    let op = parse_op("move", "3 Sol Ring to:sideboard").expect("parse move");

    let summary = apply_ops(&mut deck, &[op]).expect("move card");

    assert_eq!(summary.moved, 1);
    assert_eq!(deck.section_entries_mut("SIDEBOARD"), &[source]);
    assert_eq!(deck.total(), 1);
}

/// Unqualified moves retain the entry found outside the default source section.
#[test]
fn move_from_elsewhere_preserves_source_entry() {
    let mut deck =
        Deck::parse("// DECK\n// SIDEBOARD\n2 Sol Ring (CMM) 396 *F*\n").expect("parse deck");
    let mut source = deck.entries().next().expect("source entry").clone();
    source.quantity = 1;
    let op = parse_op("move", "1 Sol Ring to:maybeboard").expect("parse move");

    let summary = apply_ops(&mut deck, &[op]).expect("move card");

    assert_eq!(summary.moved, 1);
    assert_eq!(
        summary.relocated,
        vec![("Sol Ring".to_string(), "SIDEBOARD".to_string())]
    );
    assert_eq!(
        deck.section_entries_mut("SIDEBOARD").as_slice(),
        std::slice::from_ref(&source)
    );
    assert_eq!(deck.section_entries_mut("MAYBEBOARD"), &[source]);
    assert_eq!(deck.total(), 2);
}

/// An explicit section does not replace source print data with command data.
#[test]
fn section_qualified_move_preserves_source_entry() {
    let mut deck =
        Deck::parse("// DECK\n// SIDEBOARD\n1 Sol Ring (CMM) 396 *F*\n").expect("parse deck");
    let source = deck.entries().next().expect("source entry").clone();
    let op = parse_op("move", "sideboard:1 Sol Ring to:deck").expect("parse move");

    let summary = apply_ops(&mut deck, &[op]).expect("move card");

    assert_eq!(summary.moved, 1);
    assert!(summary.relocated.is_empty());
    assert_eq!(deck.section_entries_mut("DECK"), &[source]);
    assert!(deck.section_entries_mut("SIDEBOARD").is_empty());
}

/// Exact print matching can ignore set-code case without changing stored text.
#[test]
fn print_qualified_move_keeps_original_set_code_case() {
    let mut deck = Deck::parse("// DECK\n1 Sol Ring (CMM) 396 *F*\n").expect("parse deck");
    let source = deck.entries().next().expect("source entry").clone();
    let mut requested = source.clone();
    requested.set_code = Some("cmm".into());
    let op = DeckOp::Move {
        from: None,
        to: "SIDEBOARD".into(),
        entry: requested,
    };

    let summary = apply_ops(&mut deck, &[op]).expect("move card");

    assert_eq!(summary.moved, 1);
    assert_eq!(deck.section_entries_mut("SIDEBOARD"), &[source]);
}

/// Destination entries with another printing or finish must stay separate.
#[test]
fn move_does_not_merge_different_destination_identities() {
    for destination in [
        "2 Sol Ring (LTC) 284 *F*",
        "2 Sol Ring (CMM) 397 *F*",
        "2 Sol Ring (CMM) 396",
        "2 Sol Ring",
    ] {
        let mut deck = Deck::parse(&format!(
            "// DECK\n1 Sol Ring (CMM) 396 *F*\n// SIDEBOARD\n{destination}\n"
        ))
        .expect("parse deck");
        let source = deck.entries().next().expect("source entry").clone();
        let existing = deck.section_entries_mut("SIDEBOARD")[0].clone();
        let op = parse_op("move", "1 Sol Ring to:sideboard").expect("parse move");

        let summary = apply_ops(&mut deck, &[op]).expect("move card");

        assert_eq!(summary.moved, 1);
        assert_eq!(
            deck.section_entries_mut("SIDEBOARD"),
            &[existing, source],
            "destination: {destination}"
        );
        assert_eq!(deck.total(), 3);
    }
}

/// A source without print data must not gain a known destination printing.
#[test]
fn move_keeps_unprinted_source_separate_from_printed_destination() {
    let mut deck = Deck::parse("// DECK\n1 Sol Ring\n// SIDEBOARD\n2 Sol Ring (CMM) 396 *F*\n")
        .expect("parse deck");
    let source = deck.entries().next().expect("source entry").clone();
    let existing = deck.section_entries_mut("SIDEBOARD")[0].clone();
    let op = parse_op("move", "1 Sol Ring to:sideboard").expect("parse move");

    let summary = apply_ops(&mut deck, &[op]).expect("move card");

    assert_eq!(summary.moved, 1);
    assert_eq!(deck.section_entries_mut("SIDEBOARD"), &[existing, source]);
    assert_eq!(deck.total(), 3);
}

/// Moves add quantities when the destination has the same print identity.
#[test]
fn move_merges_matching_destination_print() {
    let mut deck =
        Deck::parse("// DECK\n3 Sol Ring (CMM) 396 *F*\n// SIDEBOARD\n2 Sol Ring (CMM) 396 *F*\n")
            .expect("parse deck");
    let mut moved = deck.entries().next().expect("source entry").clone();
    moved.quantity = 4;
    let op = parse_op("move", "2 Sol Ring to:sideboard").expect("parse move");

    let summary = apply_ops(&mut deck, &[op]).expect("move card");

    assert_eq!(summary.moved, 2);
    assert_eq!(deck.section_entries_mut("SIDEBOARD"), &[moved]);
    assert_eq!(deck.section_entries_mut("DECK")[0].quantity, 1);
    assert_eq!(deck.total(), 5);
}
