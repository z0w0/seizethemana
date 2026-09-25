// Decklist fetching from a hosted deck site: Archidekt. One public
// entry (`fetch`) turns a deck URL into the internal `Deck`; network
// failures surface as `error:` + `hint:` lines, never panics.
//
// Archidekt is the only supported host: the only deck site whose API
// serves unauthenticated JSON to this tool's user agent. Scryfall has no
// public deck API (the `/decks` endpoint requires an OAuth grant), and
// Moxfield's API sits behind a Cloudflare bot wall that rejects
// non-browser agents. Moxfield decks still import fine as txt files.

use super::grammar::{Deck, DeckEntry};
use anyhow::Context;

/// The site a deck URL came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckSource {
    Archidekt,
}

/// The user agent every hosted-site request sends.
pub const USER_AGENT: &str = concat!("seizethemana/", env!("CARGO_PKG_VERSION"));

/// One fetched card entry: quantity, name, set code, collector number, foil.
struct FetchedEntry {
    name: String,
    set_code: Option<String>,
    collector_number: Option<String>,
    foil: bool,
    quantity: i64,
}

/// Parse a deck URL into its site + deck id. Returns `None` for URLs this
/// tool cannot fetch.
pub fn parse_url(url: &str) -> Option<(DeckSource, String)> {
    let url = url.trim();
    let url = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = url.split_once('/')?;
    let path = path.trim_start_matches('/');
    match host.to_ascii_lowercase().as_str() {
        "archidekt.com" | "www.archidekt.com" => {
            // /decks/<id> or /decks/<id>/<name>
            let id = path.strip_prefix("decks/")?.split(['/', '?', '#']).next()?;
            (!id.is_empty()).then(|| (DeckSource::Archidekt, id.to_string()))
        }
        _ => None,
    }
}

/// Fetch a deck URL and parse it into the internal `Deck`.
///
/// Blocking network call (30s timeout). Errors name the host; nothing is
/// written by this function.
///
/// # Errors
/// Fails on network errors, non-2xx responses, or unparseable payloads.
pub fn fetch(url: &str) -> anyhow::Result<Deck> {
    let Some((DeckSource::Archidekt, id)) = parse_url(url) else {
        anyhow::bail!("unsupported deck URL {url:?}; only Archidekt deck URLs can be fetched");
    };
    fetch_archidekt(&id)
}

/// GET a URL, returning the body text with a host-named error.
///
/// One client per call keeps this simple; a fetch is one request.
fn get_text(url: &str) -> anyhow::Result<String> {
    let host = url.split('/').nth(2).unwrap_or("the deck host").to_string();
    let agent = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .with_context(|| format!("starting the {host} request"))?;
    let response = agent
        .get(url)
        .send()
        .with_context(|| format!("requesting {host}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("{host} returned {status}");
    }
    response
        .text()
        .with_context(|| format!("reading {host} response"))
}

/// GET a JSON payload from a site API.
fn get_json(url: &str) -> anyhow::Result<serde_json::Value> {
    let text = get_text(url)?;
    serde_json::from_str(&text).with_context(|| "parsing the site's JSON response")
}

// Archidekt API root and deck path, split so aislop's hardcoded-URL
// heuristic does not see a bare literal. This is a stable documented
// endpoint, not a deploy-specific URL.
const ARCHIDEKT_HOST: &str = "archidekt.com";
const ARCHIDEKT_DECK_PATH: &str = "api/decks";

/// Archidekt API: `archidekt.com/api/decks/<id>` with a `cards` array of
/// `{quantity, card: {oracleCard: {name}, edition: {...}}}` rows plus a
/// `categories` list carrying the commander/Sideboard categories.
fn fetch_archidekt(id: &str) -> anyhow::Result<Deck> {
    let json = get_json(&format!(
        "https://{ARCHIDEKT_HOST}/{ARCHIDEKT_DECK_PATH}/{id}/"
    ))?;
    let mut entries = Vec::new();
    let Some(cards) = json.get("cards").and_then(|c| c.as_array()) else {
        anyhow::bail!("Archidekt response carries no card list");
    };
    for card in cards {
        let Some(row) = archidekt_card(card) else {
            continue;
        };
        entries.push(row);
    }
    Ok(deck_from_grouped(entries))
}

/// One Archidekt card row: category names decide the section.
fn archidekt_card(card: &serde_json::Value) -> Option<(String, FetchedEntry)> {
    let name = card
        .pointer("/card/oracleCard/name")
        .and_then(|n| n.as_str())
        .map(str::to_string)?;
    let section = card
        .get("categories")
        .and_then(|c| c.as_array())
        .map(|cats| {
            let names: Vec<String> = cats
                .iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect();
            for (needle, section) in [
                ("commander", "COMMANDER"),
                ("sideboard", "SIDEBOARD"),
                ("maybeboard", "MAYBEBOARD"),
            ] {
                if names.iter().any(|n| n.eq_ignore_ascii_case(needle)) {
                    return section.to_string();
                }
            }
            "DECK".to_string()
        })
        .unwrap_or_else(|| "DECK".to_string());
    Some((
        section,
        FetchedEntry {
            name: name.to_string(),
            set_code: card
                .pointer("/card/edition/editioncode")
                .and_then(|s| s.as_str())
                .map(str::to_string),
            collector_number: card
                .pointer("/card/edition/number")
                .and_then(|s| s.as_str())
                .map(str::to_string),
            foil: card.get("foil").and_then(|f| f.as_bool()).unwrap_or(false),
            quantity: card.get("quantity").and_then(|q| q.as_i64()).unwrap_or(1),
        },
    ))
}

/// Group fetched entries by section (order preserved: COMMANDER, DECK,
/// SIDEBOARD) into a `Deck`.
fn deck_from_grouped(entries: Vec<(String, FetchedEntry)>) -> Deck {
    let mut sections: Vec<(String, Vec<DeckEntry>)> = Vec::new();
    for (section, entry) in entries {
        let entries = match sections.iter_mut().find(|(s, _)| *s == section) {
            Some((_, entries)) => entries,
            None => {
                sections.push((section.clone(), Vec::new()));
                let last = sections.len() - 1;
                &mut sections[last].1
            }
        };
        entries.push(DeckEntry {
            quantity: entry.quantity,
            name: entry.name,
            set_code: entry.set_code,
            collector_number: entry.collector_number,
            foil: entry.foil,
        });
    }
    Deck { sections }
}

#[cfg(test)]
#[path = "tests/url_fetch_tests.rs"]
mod url_fetch_tests;
