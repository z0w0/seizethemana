// Format and Commander-bracket legality for decks.
//
// The checks here are deterministic: everything computable from stored card
// data (deck size, copy limits, commander rules, format legality, Game
// Changer counts). Bracket signals that need judgment — tutor density,
// extra turns, mass land destruction, combo speed — are not computed; the
// command prints a checklist of what to review instead.
//
// Deck shape assumptions: a `// COMMANDER` section names the commander(s);
// a `// SIDEBOARD` section holds sideboard cards for 60-card formats (or
// the commander upgrade kit); a `// MAYBEBOARD` section holds loose
// candidates. Both bench sections check per-card legality and, in
// commander, color identity.

use super::grammar::Deck;
use super::stats::{is_basic_land, is_unlimited_copies};
use crate::db::CardRow;

use std::collections::HashMap;

#[path = "bracket_scan.rs"]
mod bracket_scan;
pub use bracket_scan::{bracket_note, game_changer_limit, scan_bracket_signals};

/// Formats `deck legal` knows. Keys match Scryfall legality names so
/// `legalities` lookups are direct.
pub const KNOWN_FORMATS: &[&str] = &[
    "standard",
    "pioneer",
    "modern",
    "legacy",
    "vintage",
    "commander",
    "pauper",
    "brawl",
    "oathbreaker",
];

/// Formats with a singleton deck rule. Each is built around a commander.
const SINGLETON_FORMATS: &[&str] = &["commander", "brawl", "oathbreaker"];

/// Commander deck size, commander included.
const COMMANDER_SIZE: i64 = 100;

/// Minimum maindeck size for a 60-card constructed format.
const CONSTRUCTED_MIN: i64 = 60;

/// Maximum sideboard size for constructed formats.
const SIDEBOARD_MAX: i64 = 15;

/// Default per-name copy limit outside singleton formats.
const MAX_COPIES: i64 = 4;

/// Exact maindeck size (including commander) per singleton format.
fn singleton_deck_size(format: &str) -> i64 {
    match format {
        "brawl" => 60,
        "oathbreaker" => 59,
        _ => COMMANDER_SIZE,
    }
}

/// Deck-size target for a deck by shape: a deck whose only maindeck
/// section is COMMANDER is brawl-shaped (60); a COMMANDER plus DECK
/// split is a full commander deck (100); anything else backfills to 60.
/// The shared target for `--backfill-basics` so a backfilled deck never
/// fails the size check `deck legal` applies.
pub fn singleton_size_for_deck(deck: &Deck) -> i64 {
    let has_commander = deck.section_index("COMMANDER").is_some();
    let has_deck = deck.section_index("DECK").is_some();
    match (has_commander, has_deck) {
        (true, false) => singleton_deck_size("brawl"),
        (true, true) => COMMANDER_SIZE,
        _ => CONSTRUCTED_MIN,
    }
}

/// One rule violation, with the cards that broke it.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    /// Rule name, e.g. "copy limit" or "commander color identity".
    pub rule: String,
    /// Card names involved; ordering is per-rule (some rules sort and
    /// dedup, others preserve deck order).
    pub cards: Vec<String>,
    /// Human explanation with the numbers.
    pub detail: String,
}

/// A check the CLI cannot decide; the reader validates these by hand.
#[derive(Debug, Clone, PartialEq)]
pub struct BracketNote {
    /// One line per check. Lines that start with "PASS", "CHECK", or
    /// "ADVISE" carry a verdict; anything else stays a manual note.
    pub checks: Vec<String>,
}

/// Result of guessing the deck's format from its sections.
#[derive(Debug, Clone, PartialEq)]
pub enum InferredFormat {
    /// A `// COMMANDER` section exists: commander.
    Commander,
    /// No commander section: a generic 60-card constructed deck.
    Constructed,
}

/// Guess the format from deck shape.
pub fn infer_format(deck: &Deck) -> InferredFormat {
    if deck.section_index("COMMANDER").is_some() {
        InferredFormat::Commander
    } else {
        InferredFormat::Constructed
    }
}

/// True when the deck plays as commander: an explicit `--format
/// commander` pin, or no pin at all and the deck has a `// COMMANDER`
/// section. A pinned non-commander format is never commander. The one
/// shared predicate for every format-aware branch.
pub fn is_commander(deck: &Deck, pinned_format: Option<&str>) -> bool {
    match pinned_format {
        Some(fmt) => fmt.eq_ignore_ascii_case("commander"),
        None => infer_format(deck) == InferredFormat::Commander,
    }
}

/// Count copies per card name across all sections (sideboard and
/// maybeboard included).
pub(super) fn copies_by_name(deck: &Deck) -> Vec<(String, i64)> {
    copies_in_sections(deck, |_| true)
}

/// Count copies per card name outside SIDEBOARD and MAYBEBOARD sections.
///
/// Size rules, copy limits, bracket counts, and color-identity offenders
/// read this count: the sideboard is the upgrade kit and the maybeboard
/// holds candidates, neither a legal zone.
pub(super) fn maindeck_copies_by_name(deck: &Deck) -> Vec<(String, i64)> {
    copies_in_sections(deck, |s| !super::grammar::is_bench_section(s))
}

/// Count copies per card name outside MAYBEBOARD sections.
///
/// Constructed copy limits read this count: the 4-copy rule spans the
/// maindeck and sideboard (BO3 swaps included), but maybeboard cards
/// are loose candidates and never count.
pub(super) fn playable_copies_by_name(deck: &Deck) -> Vec<(String, i64)> {
    copies_in_sections(deck, |s| !super::grammar::is_maybeboard_section(s))
}

/// Count copies per card name across SIDEBOARD and MAYBEBOARD sections
/// (the legal deck's bench).
pub(super) fn bench_copies_by_name(deck: &Deck) -> Vec<(String, i64)> {
    copies_in_sections(deck, super::grammar::is_bench_section)
}

/// Count copies per card name across the sections the filter keeps.
fn copies_in_sections(deck: &Deck, keep: impl Fn(&str) -> bool) -> Vec<(String, i64)> {
    let mut counts: Vec<(String, i64)> = Vec::new();
    for entry in deck
        .sections
        .iter()
        .filter(|(s, _)| keep(s))
        .flat_map(|(_, e)| e.iter())
    {
        if let Some(count) = counts
            .iter_mut()
            .find(|(n, _)| *n == entry.name)
            .map(|(_, c)| c)
        {
            *count += entry.quantity;
        } else {
            counts.push((entry.name.clone(), entry.quantity));
        }
    }
    counts
}

/// Names listed in the deck's COMMANDER section (order preserved).
fn commander_names(deck: &Deck) -> Vec<String> {
    deck.section_index("COMMANDER")
        .map(|i| deck.sections[i].1.iter().map(|e| e.name.clone()).collect())
        .unwrap_or_default()
}

/// Keywords are stored as a JSON array; partner status lives there and in
/// the type line ("Partner", "Partner with", "Friends forever").
fn keyword_has_partner(keywords: &str) -> bool {
    let list: Vec<String> = serde_json::from_str(keywords).unwrap_or_default();
    list.iter().any(|k| k == "Partner")
}

/// Commander count per deck rules: exactly one commander, or exactly two
/// when both carry a partner-style keyword.
fn commander_legal(names: &[String], cards: &HashMap<String, CardRow>) -> Option<Violation> {
    match names.len() {
        1 => {
            let card = cards.get(&names[0]);
            match card {
                Some(card) if is_commander_type(card) => None,
                Some(card) => Some(Violation {
                    rule: "commander".into(),
                    cards: vec![card.name.clone()],
                    detail: format!(
                        "{} must be a legendary creature, planeswalker, or legendary \
                         Vehicle/Spacecraft with a power/toughness box to command",
                        card.name
                    ),
                }),
                None => None, // unknown card reported by the unknown-name check
            }
        }
        2 => {
            let partners: Vec<&CardRow> = names.iter().filter_map(|n| cards.get(n)).collect();
            if partners.len() < 2 {
                return Some(Violation {
                    rule: "commander".into(),
                    cards: names.to_vec(),
                    detail: "two commanders given but one is not a known card".into(),
                });
            }
            let both_partner = partners.iter().all(|c| {
                c.type_line.contains("Partner")
                    || keyword_has_partner(&c.keywords)
                    || c.oracle_text.contains("Partner with")
                    || c.oracle_text.contains("Friends forever")
            });
            if both_partner {
                None
            } else {
                // A "Choose a Background" commander pairs with exactly one
                // Background enchantment as a second commander.
                let background_pair = {
                    let chooses = partners
                        .iter()
                        .any(|c| c.oracle_text.contains("Choose a Background"));
                    let backgrounds = partners
                        .iter()
                        .filter(|c| c.type_line.contains("Background"))
                        .count();
                    chooses && backgrounds == 1
                };
                if background_pair {
                    None
                } else {
                    Some(Violation {
                        rule: "commander".into(),
                        cards: names.to_vec(),
                        detail: "two commanders need Partner, 'Partner with', \
                                 'Friends forever', or a 'Choose a Background' pair"
                            .into(),
                    })
                }
            }
        }
        n => Some(Violation {
            rule: "commander".into(),
            cards: names.to_vec(),
            detail: format!("{n} cards in the COMMANDER section; exactly 1 (or 2 with Partner)"),
        }),
    }
}

/// Legality state for one format from a stored `legalities` JSON map.
///
/// The shared parser for every format gate (legal, suggest, cuts):
/// `None` when the map is malformed or the format key is missing.
pub fn legality_in(legalities_json: &str, format: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(legalities_json)
        .ok()
        .and_then(|m| m.get(format).and_then(|v| v.as_str().map(String::from)))
}

/// The full `legalities` JSON map, empty when malformed (multi-format
/// gates that probe several keys read this).
pub fn legality_in_map(
    legalities_json: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str(legalities_json).ok()
}

/// True when the card can lead a commander deck.
///
/// Since Edge of Eternities (2025), legendary Vehicles and Spacecraft with a
/// printed power/toughness box are also legal commanders.
pub fn is_commander_type(card: &CardRow) -> bool {
    card.type_line.contains("Legendary Creature")
        || card.type_line.contains("Legendary Planeswalker")
        || (card.type_line.contains("Legendary")
            && (card.type_line.contains("Vehicle") || card.type_line.contains("Spacecraft"))
            && card.power.is_some()
            && card.toughness.is_some())
}

/// Color identity letters from a stored identity JSON array ("WU").
///
/// The one parser for every identity check (legal, suggest, cuts): colorless
/// parses to "" and malformed JSON to "" as well.
pub fn identity_letters(color_identity_json: &str) -> String {
    serde_json::from_str::<Vec<String>>(color_identity_json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| c.chars().next())
        .collect()
}

/// Color identity letters for a stored card ("WU"), colorless as "".
fn color_identity(card: &CardRow) -> String {
    identity_letters(&card.color_identity)
}

/// True when every identity color of `card` is in the commander's identity.
fn identity_ok(card: &CardRow, commander_identity: &str) -> bool {
    color_identity(card)
        .chars()
        .all(|c| commander_identity.contains(c))
}

/// Violations for commander-style singleton formats: exact deck size, the
/// singleton copy limit, commander rules, and color identity (maindeck and
/// bench, against the combined identity of a resolved commander or
/// Partner pair).
fn singleton_violations(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    format: &str,
    maindeck_counts: &[(String, i64)],
) -> Vec<Violation> {
    let mut violations = Vec::new();
    // Deck size: each singleton format carries its own exact count
    // (commander 100 including commander, brawl 60, oathbreaker 59).
    // Sideboard cards do not count (in commander it is a wishlist, not
    // a legal sideboard).
    let maindeck_total = deck.maindeck_total();
    let expected = singleton_deck_size(format);
    if maindeck_total != expected {
        violations.push(Violation {
            rule: "deck size".into(),
            cards: Vec::new(),
            detail: format!("{maindeck_total} cards; {format} decks are exactly {expected}"),
        });
    }
    // Copy limit: singleton, so more than one copy is illegal;
    // unlimited-copy oracle text and basics excepted. Sideboard
    // copies are a wishlist, not extra maindeck copies, so they do
    // not count here.
    let limit_violations: Vec<String> = maindeck_counts
        .iter()
        .filter(|(name, qty)| {
            *qty > 1
                && cards
                    .get(name)
                    .is_some_and(|c| !is_basic_land(c) && !is_unlimited_copies(c))
        })
        .map(|(name, qty)| format!("{name} ×{qty}"))
        .collect();
    if !limit_violations.is_empty() {
        violations.push(Violation {
            rule: "copy limit".into(),
            cards: limit_violations,
            detail: "more than 1 copy (singleton; basics and 'any number' cards excepted)".into(),
        });
    }
    violations.extend(commander_identity_violations(deck, cards, maindeck_counts));
    violations
}

/// Commander-rule and color-identity violations for a deck with a
/// COMMANDER section (called only for singleton formats).
fn commander_identity_violations(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    maindeck_counts: &[(String, i64)],
) -> Vec<Violation> {
    let commander_section = commander_names(deck);
    if commander_section.is_empty() {
        return Vec::new();
    }
    let mut violations = Vec::new();
    if let Some(v) = commander_legal(&commander_section, cards) {
        violations.push(v);
    }
    // A resolved commander (or both halves of a Partner pair) gates the
    // identity check. An unresolved name is already reported as unknown;
    // guessing "" identity would flag every colored card.
    let commanders_resolved = commander_section
        .iter()
        .all(|name| cards.get(name).is_some_and(is_commander_type));
    if !commanders_resolved {
        return violations;
    }
    let identity = commander_section
        .iter()
        .filter_map(|n| cards.get(n))
        .map(color_identity)
        .collect::<String>();
    let identity_offenders = |names: &[(String, i64)]| -> Vec<String> {
        names
            .iter()
            .filter(|(name, _)| {
                !commander_section.contains(name)
                    && cards.get(name).is_some_and(|c| !identity_ok(c, &identity))
            })
            .map(|(name, _)| name.clone())
            .collect()
    };
    let offenders = identity_offenders(maindeck_counts);
    if !offenders.is_empty() {
        violations.push(Violation {
            rule: "commander color identity".into(),
            cards: offenders,
            detail: format!("cards fall outside the commander's color identity ({identity})"),
        });
    }
    let bench_offenders = identity_offenders(&bench_copies_by_name(deck));
    if !bench_offenders.is_empty() {
        violations.push(Violation {
            rule: "bench color identity".into(),
            cards: bench_offenders,
            detail: format!(
                "sideboard/maybeboard cards outside the commander's color identity ({identity})"
            ),
        });
    }
    violations
}

/// Violations for 60-card constructed formats: maindeck minimum, sideboard
/// cap, and the 4-copy limit spanning maindeck + sideboard.
fn constructed_violations(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    format: Option<&str>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let maindeck = deck.maindeck_total();
    if maindeck < CONSTRUCTED_MIN {
        violations.push(Violation {
            rule: "deck size".into(),
            cards: Vec::new(),
            detail: format!(
                "{maindeck} maindeck cards; {} needs at least {CONSTRUCTED_MIN}",
                format.unwrap_or("this format")
            ),
        });
    }
    let sideboard = deck.sideboard_total();
    if sideboard > SIDEBOARD_MAX {
        violations.push(Violation {
            rule: "sideboard size".into(),
            cards: Vec::new(),
            detail: format!("{sideboard} sideboard cards; maximum {SIDEBOARD_MAX}"),
        });
    }
    // Copy limit spans maindeck + sideboard (BO3 swaps included); the
    // maybeboard is exempt (loose candidates, not part of the deck).
    let limit_violations: Vec<String> = playable_copies_by_name(deck)
        .iter()
        .filter(|(name, qty)| {
            *qty > MAX_COPIES
                && cards
                    .get(name)
                    .is_some_and(|c| !is_basic_land(c) && !is_unlimited_copies(c))
        })
        .map(|(name, qty)| format!("{name} ×{qty}"))
        .collect();
    if !limit_violations.is_empty() {
        violations.push(Violation {
            rule: "copy limit".into(),
            cards: limit_violations,
            detail: format!(
                "more than {MAX_COPIES} copies (basics and 'any number' cards excepted)"
            ),
        });
    }
    violations
}

/// Per-card format legality: banned or not_legal fails. Skipped when the
/// format is unknown (no legality key to test).
fn format_legality_violations(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    format: &str,
) -> Vec<Violation> {
    let format_key = format.to_ascii_lowercase();
    let mut violations = Vec::new();
    let mut not_legal: Vec<String> = Vec::new();
    let mut banned: Vec<String> = Vec::new();
    let mut restricted: Vec<(String, i64)> = Vec::new();
    for (name, qty) in copies_by_name(deck) {
        if let Some(card) = cards.get(name.as_str()) {
            match legality_in(&card.legalities, &format_key).as_deref() {
                Some("legal") => {}
                // Restricted cards are capped at one copy (Vintage rule);
                // legality itself passes here, the copy count is checked
                // below.
                Some("restricted") => restricted.push((name, qty)),
                Some("banned") => banned.push(name),
                _ => not_legal.push(name),
            }
        }
    }
    let over_copies: Vec<String> = restricted
        .iter()
        .filter(|(_, qty)| *qty > 1)
        .map(|(name, qty)| format!("{name} ×{qty}"))
        .collect();
    if !over_copies.is_empty() {
        violations.push(Violation {
            rule: "restricted copy limit".into(),
            cards: over_copies,
            detail: "restricted cards are limited to one copy".into(),
        });
    }
    let restricted_names: Vec<String> = restricted.into_iter().map(|(name, _)| name).collect();
    if !restricted_names.is_empty() {
        violations.push(Violation {
            rule: "restricted".into(),
            cards: restricted_names,
            detail: format!("restricted in {format}"),
        });
    }
    if !banned.is_empty() {
        violations.push(Violation {
            rule: "banned".into(),
            cards: banned,
            detail: format!("banned in {format}"),
        });
    }
    if !not_legal.is_empty() {
        violations.push(Violation {
            rule: "not legal".into(),
            cards: not_legal,
            detail: format!("not legal in {format}"),
        });
    }
    violations
}

/// Bracket check: the Game Changer hard cap (a violation when exceeded)
/// plus a sideboard advisory naming Game Changers waiting in the upgrade
/// kit. Returns `(violations, advisories)`.
fn bracket_game_changer_check(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    maindeck_counts: &[(String, i64)],
    bracket: u8,
) -> (Vec<Violation>, Vec<String>) {
    let changers: Vec<String> = maindeck_counts
        .iter()
        .filter(|(name, _)| cards.get(name).is_some_and(is_game_changer))
        .map(|(name, _)| name.clone())
        .collect();
    let limit = game_changer_limit(bracket);
    let mut violations = Vec::new();
    if let Some(limit) = limit
        && changers.len() > limit as usize
    {
        violations.push(Violation {
            rule: "game changers".into(),
            cards: changers.clone(),
            detail: format!(
                "{} Game Changers; the bracket-{bracket} hard cap is {limit} (Game Changer count is the hard bracket rule)",
                changers.len()
            ),
        });
    }
    // The sideboard is the upgrade kit: surface its Game Changers so a
    // reader previewing a bracket bump can see what comes along. Every
    // SIDEBOARD section counts (imports may keep more than one). The
    // maybeboard never joins the deck as-is, so it stays out.
    let mut sideboard_changers: Vec<String> = Vec::new();
    for (name, entries) in &deck.sections {
        if !super::grammar::is_sideboard_section(name) {
            continue;
        }
        sideboard_changers.extend(
            entries
                .iter()
                .filter(|e| cards.get(&e.name).is_some_and(is_game_changer))
                .map(|e| e.name.clone()),
        );
    }
    sideboard_changers.sort();
    sideboard_changers.dedup();
    let mut advisories = Vec::new();
    if !sideboard_changers.is_empty() {
        let names = sideboard_changers.join(", ");
        match limit {
            Some(l) => advisories.push(format!(
                "ADVISE sideboard: {} sideboard Game Changer(s) not counted toward the bracket-{bracket} cap of {l}: {}",
                sideboard_changers.len(),
                names
            )),
            None => advisories.push(format!(
                "ADVISE sideboard: {} sideboard Game Changer(s) (uncapped at bracket {bracket}): {}",
                sideboard_changers.len(),
                names
            )),
        }
    }
    (violations, advisories)
}

/// Run every deterministic check against one deck.
///
/// `format` is `Some` for a known format (per-card legality is checked) and
/// `None` for an inferred deck whose real format is unknown (structural
/// checks only, since there is no legality key to test against).
/// `cards` maps card names to stored rows; names absent from the map are
/// skipped by metadata checks (they are reported separately as unknown).
///
/// Returns `(violations, advisories)`: violations are hard failures
/// (unknown cards, copy limits, size, commander rules, Game Changer cap);
/// advisories are informational notes (e.g. Game Changers waiting in the
/// sideboard) that never affect the exit code.
pub fn check(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    format: Option<&str>,
    bracket: Option<u8>,
) -> (Vec<Violation>, Vec<String>) {
    let mut violations = Vec::new();
    let mut advisories: Vec<String> = Vec::new();
    let mut unknown: Vec<String> = deck
        .entries()
        .map(|e| e.name.clone())
        .filter(|n| !cards.contains_key(n))
        .collect();
    unknown.sort();
    unknown.dedup();
    if !unknown.is_empty() {
        violations.push(Violation {
            rule: "unknown cards".into(),
            cards: unknown,
            detail: "these names are not in the oracle; legality cannot be checked".into(),
        });
    }

    let maindeck_counts = maindeck_copies_by_name(deck);

    if SINGLETON_FORMATS.contains(&format.unwrap_or("")) {
        let format = format.unwrap_or("");
        violations.extend(singleton_violations(deck, cards, format, &maindeck_counts));
    } else {
        violations.extend(constructed_violations(deck, cards, format));
    }

    // Per-card format legality: banned or not_legal fails.
    if let Some(format) = format {
        violations.extend(format_legality_violations(deck, cards, format));
    }

    // Bracket: game-changer count is the deterministic part. In commander
    // the sideboard is a wishlist, so its Game Changers never count toward
    // the bracket allowance.
    if let (Some(bracket), Some(format)) = (bracket, format)
        && SINGLETON_FORMATS.contains(&format)
    {
        let (bracket_violations, bracket_advisories) =
            bracket_game_changer_check(deck, cards, &maindeck_counts, bracket);
        violations.extend(bracket_violations);
        advisories.extend(bracket_advisories);
    }

    (violations, advisories)
}

/// Build the deck-size summary line.
///
/// Counts the maindeck (bench sections excluded); the sideboard and
/// maybeboard are noted separately when present.
pub fn summary_line(deck: &Deck, cards: &HashMap<String, CardRow>) -> String {
    let counts: Vec<(String, i64)> = deck
        .sections
        .iter()
        .filter(|(s, _)| !super::grammar::is_bench_section(s))
        .flat_map(|(_, e)| e.iter())
        .fold(Vec::new(), |mut acc, entry| {
            if let Some((_, c)) = acc.iter_mut().find(|(n, _)| *n == entry.name) {
                *c += entry.quantity;
            } else {
                acc.push((entry.name.clone(), entry.quantity));
            }
            acc
        });
    let total: i64 = counts.iter().map(|(_, q)| q).sum();
    let unique = counts.len();
    let basics: i64 = counts
        .iter()
        .filter(|(name, _)| cards.get(name).is_some_and(is_basic_land))
        .map(|(_, q)| q)
        .sum();
    let sideboard = deck.sideboard_total();
    let maybeboard = deck.maybeboard_total();
    let mut base = format!("{total} cards, {unique} unique ({basics} basic-land copies)");
    if sideboard > 0 {
        base.push_str(&format!(" + {sideboard} sideboard"));
    }
    if maybeboard > 0 {
        base.push_str(&format!(" + {maybeboard} maybeboard"));
    }
    base
}

/// True when a stored card is on the Game Changer list.
pub fn is_game_changer(card: &CardRow) -> bool {
    card.game_changer == Some(true)
}

/// Entry point for `stm deck legal <name>`.
///
/// Exit 0 when the deck passes every deterministic check, exit 1 when it
/// does not (JSON still prints the full report either way). A missing deck
/// file is the shared deck-not-found error: exit 1 with a "create it
/// first" hint from the dispatcher.
pub fn legal(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    format: Option<&str>,
    bracket: Option<u8>,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = super::store::load_deck(paths, name)?;

    // Resolve the format: explicit flag wins, otherwise infer from sections
    // and say so.
    let (format, assumed) = match format {
        Some(f) => {
            let f = f.to_ascii_lowercase();
            if !KNOWN_FORMATS.contains(&f.as_str()) {
                out.error(&format!("unknown format {f:?}"));
                out.hint(&format!("known formats: {}", KNOWN_FORMATS.join(", ")));
                return Ok(crate::cli::codes::USAGE);
            }
            (f, false)
        }
        None => match infer_format(&deck) {
            InferredFormat::Commander => ("commander".to_string(), true),
            InferredFormat::Constructed => ("constructed".to_string(), true),
        },
    };
    let is_commander_format = SINGLETON_FORMATS.contains(&format.as_str());
    if bracket.is_some() && !is_commander_format {
        out.warning(&format!(
            "--bracket applies to commander-style formats; ignored for {format}"
        ));
    }

    // Stored rows for every deck entry name.
    let cards = super::stats::lookup_names(conn, &deck)?;

    // "constructed" is the inferred no-format case: structural checks only.
    let check_format: Option<&str> = if format == "constructed" {
        None
    } else {
        Some(&format)
    };
    let (violations, check_advisories) = check(&deck, &cards, check_format, bracket);

    // Notes: bracket checklist with oracle-text scan verdicts when given;
    // otherwise, for commander decks, list the deck's Game Changers so the
    // reader can pick a bracket.
    let note = if is_commander_format {
        match bracket {
            Some(bracket) => {
                let mut checks = bracket_note(bracket).map(|n| n.checks).unwrap_or_default();
                checks.extend(scan_bracket_signals(&deck, &cards, bracket));
                Some(BracketNote { checks })
            }
            None => Some(BracketNote {
                checks: super::bracket::game_changer_checklist(&deck, &cards),
            }),
        }
    } else if format == "constructed" {
        Some(BracketNote {
            checks: vec![
                "pass --format to check per-card legality for a specific format".to_string(),
            ],
        })
    } else {
        None
    };

    let legal = violations.is_empty();
    let summary = summary_line(&deck, &cards);

    // Advisories: bracket judgment calls the official rules leave to the
    // table. Only `ADVISE ` verdicts (the `ℹ` lines) land here; `CHECK `
    // verdicts stay hard rules and surface in `violations`/`notes`.
    let advisories: Vec<String> = {
        let mut merged = check_advisories;
        merged.extend(
            note.as_ref()
                .map(|n| {
                    n.checks
                        .iter()
                        .filter(|c| c.starts_with("ADVISE "))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );
        merged
    };

    if json {
        let violations: Vec<serde_json::Value> = violations
            .iter()
            .map(|v| {
                serde_json::json!({
                    "rule": v.rule,
                    "cards": v.cards,
                    "detail": v.detail,
                })
            })
            .collect();
        let notes: Vec<String> = note.map(|n| n.checks).unwrap_or_default();
        let v = serde_json::json!({
            "name": name,
            "format": format,
            "format_assumed": assumed,
            "bracket": bracket,
            "legal": legal,
            "violations": violations,
            "advisories": advisories,
            "notes": notes,
            "summary": summary,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        super::bracket::print_report(
            out,
            name,
            &format,
            assumed,
            bracket,
            legal,
            &violations,
            &note,
            &summary,
        );
    }
    Ok(if legal {
        crate::cli::codes::OK
    } else {
        crate::cli::codes::ERROR
    })
}
#[cfg(test)]
#[path = "tests/legal_tests.rs"]
mod legal_tests;
