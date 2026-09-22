use crate::deck::url_fetch::{DeckSource, parse_url};

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
