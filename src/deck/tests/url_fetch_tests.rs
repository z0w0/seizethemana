use super::super::url_fetch::archidekt_card;
use crate::deck::url_fetch::{DeckSource, deck_from_grouped, parse_url};

#[test]
fn parses_archidekt_urls() {
    assert_eq!(
        parse_url("https://archidekt.com/decks/42069/My_Deck"),
        Some((DeckSource::Archidekt, "42069".to_string()))
    );
    // Query strings and trailing paths do not leak into the id.
    assert_eq!(
        parse_url("https://www.archidekt.com/decks/42069?foo=bar"),
        Some((DeckSource::Archidekt, "42069".to_string()))
    );
}

#[test]
fn rejects_unsupported_and_malformed() {
    // Scryfall has no public deck API; Moxfield's API rejects bots.
    assert!(parse_url("https://scryfall.com/decks/abc-123").is_none());
    assert!(parse_url("https://www.moxfield.com/decks/some-slug").is_none());
    assert!(parse_url("https://tappedout.net/mtg-decks/x/").is_none());
    assert!(parse_url("not a url").is_none());
    assert!(parse_url("https://archidekt.com/binders/x").is_none());
    assert!(parse_url("https://archidekt.com/decks/").is_none());
}

/// The user agent identifies the tool and version.
#[test]
fn user_agent_names_the_tool() {
    assert!(crate::deck::url_fetch::USER_AGENT.starts_with("seizethemana/"));
}

/// One Archidekt card row: the commander category wins, sideboard and
/// maybeboard follow, everything else lands in DECK.
#[test]
fn archidekt_row_categories_route_sections() {
    let row = |cats: &[&str], foil: Option<bool>, qty: Option<i64>| {
        let cats: Vec<serde_json::Value> = cats
            .iter()
            .map(|c| serde_json::Value::String((*c).to_string()))
            .collect();
        serde_json::json!({
            "quantity": qty.unwrap_or(2),
            "foil": foil.unwrap_or(false),
            "card": {
                "oracleCard": {"name": "Frog Wizard"},
                "edition": {"editioncode": "tsr", "number": "266"}
            },
            "categories": cats
        })
    };
    let commander = super::super::url_fetch::archidekt_card(&row(&["Commander"], None, None))
        .expect("commander row parses");
    assert_eq!(commander.0, "COMMANDER");
    let side = archidekt_card(&row(&["Sideboard"], None, None)).expect("sideboard row parses");
    assert_eq!(side.0, "SIDEBOARD");
    let maybe = archidekt_card(&row(&["Maybeboard"], None, None)).expect("maybeboard row parses");
    assert_eq!(maybe.0, "MAYBEBOARD");
    let plain = archidekt_card(&row(&["Maindeck"], None, None)).expect("plain row parses");
    assert_eq!(plain.0, "DECK");
    // Fields carry through: quantity, foil flag, set code, collector number.
    assert_eq!(commander.1.quantity, 2);
    assert_eq!(commander.1.name, "Frog Wizard");
    assert_eq!(commander.1.set_code.as_deref(), Some("tsr"));
    assert_eq!(commander.1.collector_number.as_deref(), Some("266"));
    let foiled = archidekt_card(&row(&[], Some(true), None)).expect("foil row parses");
    assert!(foiled.1.foil);
    // Missing fields fall back: no name drops the row entirely.
    assert!(
        archidekt_card(&serde_json::json!({
            "quantity": 1, "card": {"edition": {}}, "categories": []
        }))
        .is_none()
    );
}

/// Grouped entries land in one section each, first-seen order preserved.
#[test]
fn deck_from_grouped_orders_sections() {
    use crate::deck::grammar::DeckEntry;
    let entry = |name: &str| {
        (
            "DECK".to_string(),
            super::super::url_fetch::FetchedEntry {
                name: name.to_string(),
                set_code: None,
                collector_number: None,
                foil: false,
                quantity: 1,
            },
        )
    };
    let (s, e) = (
        "COMMANDER".to_string(),
        super::super::url_fetch::FetchedEntry {
            name: "Frog".to_string(),
            set_code: None,
            collector_number: None,
            foil: false,
            quantity: 1,
        },
    );
    let mut deck = deck_from_grouped(vec![entry("Bolt"), (s, e), entry("Fog")]);
    let names: Vec<String> = deck
        .section_entries_mut("DECK")
        .iter()
        .map(|d: &DeckEntry| d.name.clone())
        .collect();
    assert_eq!(names, vec!["Bolt", "Fog"]);
    assert_eq!(deck.section_entries_mut("COMMANDER").len(), 1);
}
