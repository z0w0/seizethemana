// Format and Commander-bracket legality for decks.
//
// The checks here are deterministic: everything computable from stored card
// data (deck size, copy limits, commander rules, format legality, Game
// Changer counts). Bracket signals that need judgment — tutor density,
// extra turns, mass land destruction, combo speed — are not computed; the
// command prints a checklist of what to review instead.
//
// Deck shape assumptions: a `// COMMANDER` section names the commander(s);
// a `// SIDEBOARD` section holds sideboard cards for 60-card formats.

use super::grammar::Deck;
use super::stats::{is_basic_land, is_unlimited_copies};
use crate::db::CardRow;

use std::collections::HashMap;

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

/// Formats with a singleton deck rule.
const SINGLETON_FORMATS: &[&str] = &["commander", "brawl", "oathbreaker"];

/// Formats built around a commander (100-card singleton deck).
const COMMANDER_FORMATS: &[&str] = &["commander", "brawl", "oathbreaker"];

/// Commander deck size, commander included.
const COMMANDER_SIZE: i64 = 100;

/// Minimum maindeck size for a 60-card constructed format.
const CONSTRUCTED_MIN: i64 = 60;

/// Maximum sideboard size for constructed formats.
const SIDEBOARD_MAX: i64 = 15;

/// Default per-name copy limit outside singleton formats.
const MAX_COPIES: i64 = 4;

/// One rule violation, with the cards that broke it.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    /// Rule name, e.g. "copy limit" or "commander color identity".
    pub rule: String,
    /// Card names involved, in deck order.
    pub cards: Vec<String>,
    /// Human explanation with the numbers.
    pub detail: String,
}

/// A check the CLI cannot decide; the reader validates these by hand.
#[derive(Debug, Clone, PartialEq)]
pub struct BracketNote {
    /// One line per check. Lines that start with "PASS" or "CHECK" carry a
    /// verdict; anything else stays a manual note.
    pub checks: Vec<String>,
}

/// Result of guessing the deck's format from its sections.
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

/// Count copies per card name across all sections (sideboard included).
fn copies_by_name(deck: &Deck) -> Vec<(String, i64)> {
    copies_in_sections(deck, |_| true)
}

/// Count copies per card name outside SIDEBOARD sections.
///
/// For commander-style formats the sideboard is a wishlist, not a legal
/// zone, so rules that bind the deck itself read this count.
fn maindeck_copies_by_name(deck: &Deck) -> Vec<(String, i64)> {
    copies_in_sections(deck, |s| !s.eq_ignore_ascii_case("SIDEBOARD"))
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
                    || c.oracle_text.contains("Partner with")
                    || c.oracle_text.contains("Friends forever")
            });
            if both_partner {
                None
            } else {
                Some(Violation {
                    rule: "commander".into(),
                    cards: names.to_vec(),
                    detail: "two commanders need Partner, 'Partner with', or 'Friends forever'"
                        .into(),
                })
            }
        }
        n => Some(Violation {
            rule: "commander".into(),
            cards: names.to_vec(),
            detail: format!("{n} cards in the COMMANDER section; exactly 1 (or 2 with Partner)"),
        }),
    }
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

/// Color identity letters for a stored card ("WU"), colorless as "".
fn color_identity(card: &CardRow) -> String {
    serde_json::from_str::<Vec<String>>(&card.color_identity)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| c.chars().next())
        .collect()
}

/// True when every identity color of `card` is in the commander's identity.
fn identity_ok(card: &CardRow, commander_identity: &str) -> bool {
    color_identity(card)
        .chars()
        .all(|c| commander_identity.contains(c))
}

/// Run every deterministic check.
///
/// `format` is `Some` for a known format (per-card legality is checked) and
/// `None` for an inferred deck whose real format is unknown (structural
/// checks only, since there is no legality key to test against).
/// `cards` maps card names to stored rows; names absent from the map are
/// skipped by metadata checks (they are reported separately as unknown).
pub fn check(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    format: Option<&str>,
    bracket: Option<u8>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
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
            cards: unknown.clone(),
            detail: "these names are not in the oracle; legality cannot be checked".into(),
        });
    }

    let counts = copies_by_name(deck);
    // Sideboard copies never count toward the deck size. Singleton formats
    // (commander etc.) have no sideboard; 60-card formats subtract them for
    // the maindeck minimum.
    let sideboard = deck.sideboard_total();
    let maindeck_total = deck.maindeck_total();
    let commander_section = commander_names(deck);
    let maindeck_counts = maindeck_copies_by_name(deck);

    if SINGLETON_FORMATS.contains(&format.unwrap_or("")) {
        // Deck size: commander formats need exactly 100 including commander.
        // Sideboard cards do not count (in commander it is a wishlist, not
        // a legal sideboard).
        if maindeck_total != COMMANDER_SIZE {
            violations.push(Violation {
                rule: "deck size".into(),
                cards: Vec::new(),
                detail: format!(
                    "{maindeck_total} cards; {} decks are exactly {COMMANDER_SIZE}",
                    format.unwrap_or("this format")
                ),
            });
        }
        // Copy limit: singleton; unlimited-copy oracle text and basics
        // excepted. Sideboard copies are a wishlist, not extra maindeck
        // copies, so they do not count here.
        let limit_violations: Vec<String> = maindeck_counts
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
        // Commander rules only apply to commander-style formats with a
        // COMMANDER section.
        if COMMANDER_FORMATS.contains(&format.unwrap_or("")) && !commander_section.is_empty() {
            if let Some(v) = commander_legal(&commander_section, cards) {
                violations.push(v);
            }
            // Color identity of every other card must sit inside the
            // commander's.
            let identity = commander_section
                .iter()
                .filter_map(|n| cards.get(n))
                .map(color_identity)
                .collect::<String>();
            let offenders: Vec<String> = counts
                .iter()
                .filter(|(name, _)| {
                    !commander_section.contains(name)
                        && cards.get(name).is_some_and(|c| !identity_ok(c, &identity))
                })
                .map(|(name, _)| name.clone())
                .collect();
            if !offenders.is_empty() {
                violations.push(Violation {
                    rule: "commander color identity".into(),
                    cards: offenders,
                    detail: format!(
                        "cards fall outside the commander's color identity ({identity})"
                    ),
                });
            }
        }
    } else {
        // 60-card constructed: maindeck >= 60, sideboard <= 15, max 4 copies.
        let maindeck = maindeck_total;
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
        if sideboard > SIDEBOARD_MAX {
            violations.push(Violation {
                rule: "sideboard size".into(),
                cards: Vec::new(),
                detail: format!("{sideboard} sideboard cards; maximum {SIDEBOARD_MAX}"),
            });
        }
        let limit_violations: Vec<String> = counts
            .iter()
            .filter(|(name, qty)| {
                *qty > MAX_COPIES && cards.get(name).is_some_and(|c| !is_unlimited_copies(c))
            })
            .map(|(name, qty)| format!("{name} ×{qty}"))
            .collect();
        if !limit_violations.is_empty() {
            violations.push(Violation {
                rule: "copy limit".into(),
                cards: limit_violations,
                detail: format!(
                    "more than {MAX_COPIES} copies (cards with 'any number' oracle text excepted)"
                ),
            });
        }
    }

    // Per-card format legality: banned or not_legal fails. Skipped when the
    // format is unknown (no legality key to test).
    if let Some(format) = format {
        let format_key = format.to_ascii_lowercase();
        let mut not_legal: Vec<String> = Vec::new();
        let mut banned: Vec<String> = Vec::new();
        for (name, _) in &counts {
            if let Some(card) = cards.get(name) {
                let state = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
                    &card.legalities,
                )
                .ok()
                .and_then(|m| {
                    m.get(&format_key)
                        .and_then(|v| v.as_str().map(String::from))
                });
                match state.as_deref() {
                    Some("legal") | Some("restricted") => {}
                    Some("banned") => banned.push(name.clone()),
                    _ => not_legal.push(name.clone()),
                }
            }
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
    }

    // Bracket: game-changer count is the deterministic part. In commander
    // the sideboard is a wishlist, so its Game Changers never count toward
    // the bracket allowance.
    if let (Some(bracket), Some(format)) = (bracket, format)
        && SINGLETON_FORMATS.contains(&format)
    {
        let changers: Vec<String> = maindeck_counts
            .iter()
            .filter(|(name, _)| {
                cards
                    .get(name)
                    .is_some_and(|c| c.game_changer == Some(true))
            })
            .map(|(name, _)| name.clone())
            .collect();
        let limit = game_changer_limit(bracket);
        if let Some(limit) = limit
            && changers.len() > limit as usize
        {
            violations.push(Violation {
                rule: "game changers".into(),
                cards: changers.clone(),
                detail: format!(
                    "{} Game Changers; bracket {bracket} allows at most {limit}",
                    changers.len()
                ),
            });
        }
    }

    violations
}

/// Game Changer allowance per Commander bracket: none for 1–2, at most 3 for
/// bracket 3, unlimited (None) for 4–5.
fn game_changer_limit(bracket: u8) -> Option<u8> {
    match bracket {
        1 | 2 => Some(0),
        3 => Some(3),
        _ => None,
    }
}

/// Scan the deck's oracle text for the bracket's judgment-call signals.
///
/// Deterministic text search: tutors, extra turns, mass land destruction,
/// and upkeep/end-step win-enabler lines. Verdicts are PASS (no hits) or
/// CHECK (hits found, named). The Game Changer count is checked elsewhere.
pub fn scan_bracket_signals(
    deck: &Deck,
    cards: &HashMap<String, CardRow>,
    bracket: u8,
) -> Vec<String> {
    let maindeck = maindeck_copies_by_name(deck);
    let scan = |needle: &str| -> Vec<String> {
        maindeck
            .iter()
            .filter(|(name, _)| {
                cards
                    .get(name)
                    .is_some_and(|c| c.oracle_text.to_lowercase().contains(needle))
            })
            .map(|(name, _)| name.clone())
            .collect()
    };

    let mut out = Vec::new();
    // Tutors. Cards that search for basic lands are ramp, not tutors: they
    // get their own note line so the tutor verdict stays readable.
    let is_land_ramp = |name: &str| -> bool {
        let Some(card) = cards.get(name) else {
            return false;
        };
        card.oracle_text
            .to_lowercase()
            .split('.')
            .filter(|sentence| sentence.contains("search your library"))
            .any(|sentence| {
                sentence.contains("basic land")
                    || sentence.contains("land card")
                    || sentence.contains("plains")
                    || sentence.contains("island")
                    || sentence.contains("swamp")
                    || sentence.contains("mountain")
                    || sentence.contains("forest")
            })
    };
    let tutors: Vec<String> = scan("search your library for")
        .into_iter()
        .filter(|name| !is_land_ramp(name))
        .collect();
    let ramp = scan("search your library for")
        .into_iter()
        .filter(|name| is_land_ramp(name))
        .collect::<std::collections::BTreeSet<_>>();
    match (bracket, tutors.len()) {
        (1 | 2, 0) => out.push("PASS tutors: none found".to_string()),
        (1 | 2, n) => out.push(format!(
            "CHECK tutors: {} card(s) search the library (bracket 1-2 wants none for combo pieces): {}",
            n,
            tutors.join(", ")
        )),
        (3, n) if n > 3 => out.push(format!(
            "CHECK tutors: {} found (bracket 3 allows at most 3): {}",
            n,
            tutors.join(", ")
        )),
        (3, n) => out.push(format!("PASS tutors: {} found, within the bracket-3 allowance of 3", n)),
        _ => {}
    }
    if !ramp.is_empty() {
        out.push(format!(
            "note ramp: {} card(s) search for lands (ramp, not combo tutors): {}",
            ramp.len(),
            ramp.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    // Extra turns.
    let extra_turns = scan("extra turn");
    match (bracket, extra_turns.len()) {
        (1..=3, 0) => out.push("PASS extra turns: none found".to_string()),
        (1..=3, n) => out.push(format!(
            "CHECK extra turns: {} card(s) grant an extra turn: {}",
            n,
            extra_turns.join(", ")
        )),
        _ => {}
    }
    // Mass land destruction.
    let mld = scan("destroy all lands");
    match (bracket, mld.len()) {
        (1..=3, 0) => out.push("PASS mass land destruction: none found".to_string()),
        (1..=3, n) => out.push(format!(
            "CHECK mass land destruction: {} card(s) destroy all lands: {}",
            n,
            mld.join(", ")
        )),
        _ => {}
    }
    // Two-card combo markers (proxy, not proof): "you win the game".
    let alt_wins = scan("you win the game");
    match (bracket, alt_wins.len()) {
        (1 | 2, 0) => out.push("PASS alternate wins: no 'you win the game' text found".to_string()),
        (1 | 2, n) => out.push(format!(
            "CHECK alternate wins: {} card(s) can win the game outright (verify no early two-card combo): {}",
            n,
            alt_wins.join(", ")
        )),
        (3, n) if n > 0 => out.push(format!(
            "CHECK alternate wins: {} card(s) win the game outright (must only fire late): {}",
            n,
            alt_wins.join(", ")
        )),
        _ => {}
    }
    out
}

/// The non-deterministic checklist for a bracket, tailored to what that
/// bracket asks players to avoid, with oracle-text scan verdicts where the
/// CLI can decide. Brackets 4–5 carry no checklist.
pub fn bracket_note(bracket: u8) -> Option<BracketNote> {
    let checks: &[&str] = match bracket {
        1 | 2 => &[
            "no two-card combos that end the game early",
            "no mass land destruction",
            "no extra turns before turn 7",
            "no tutors for combo pieces",
            "no Game Changers",
        ],
        3 => &[
            "at most 3 tutors, and they should not fetch combo pieces",
            "no mass land destruction",
            "no extra turns before turn 7",
            "two-card combos should only win in the late game",
        ],
        _ => &[],
    };
    if checks.is_empty() {
        None
    } else {
        Some(BracketNote {
            checks: checks.iter().map(|c| (*c).to_string()).collect(),
        })
    }
}

/// Build the deck-size summary line.
///
/// Counts the maindeck (sideboard excluded); the sideboard is noted
/// separately when present (in commander it is a wishlist for extra
/// deckbuilding advice, not a legal sideboard).
pub fn summary_line(deck: &Deck, cards: &HashMap<String, CardRow>) -> String {
    let counts: Vec<(String, i64)> = deck
        .sections
        .iter()
        .filter(|(s, _)| !s.eq_ignore_ascii_case("SIDEBOARD"))
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
    let base = format!("{total} cards, {unique} unique ({basics} basic-land copies)");
    if sideboard > 0 {
        format!("{base} + {sideboard} sideboard")
    } else {
        base
    }
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
    let is_commander_format = SINGLETON_FORMATS.contains(&format.as_str())
        && COMMANDER_FORMATS.contains(&format.as_str());
    if bracket.is_some() && !is_commander_format {
        out.warning(&format!(
            "--bracket applies to commander-style formats; ignored for {format}"
        ));
    }

    // Stored rows for every deck entry name.
    let cards = super::stats::lookup_names(conn, &deck);

    // "constructed" is the inferred no-format case: structural checks only.
    let check_format: Option<&str> = if format == "constructed" {
        None
    } else {
        Some(&format)
    };
    let violations = check(&deck, &cards, check_format, bracket);

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
                checks: game_changer_checklist(&deck, &cards),
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
            "notes": notes,
            "summary": summary,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print_report(
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

/// Checklist items listing the deck's own Game Changers, so the reader can
/// decide a bracket without re-querying every card.
fn game_changer_checklist(deck: &Deck, cards: &HashMap<String, CardRow>) -> Vec<String> {
    let changers = game_changer_names(deck, cards);
    if changers.is_empty() {
        vec!["this deck has no Game Changers".to_string()]
    } else {
        vec![format!(
            "Game Changers in this deck: {}",
            changers.join(", ")
        )]
    }
}

/// Names of the deck's maindeck Game Changers, in first-seen deck order.
///
/// Sideboard Game Changers are excluded: the sideboard is a commander
/// wishlist, not part of the deck.
pub fn game_changer_names(deck: &Deck, cards: &HashMap<String, CardRow>) -> Vec<String> {
    maindeck_copies_by_name(deck)
        .into_iter()
        .filter(|(name, _)| cards.get(name).is_some_and(is_game_changer))
        .map(|(name, _)| name)
        .collect()
}

/// Human report on stdout.
///
/// Violations print as `error: <rule>: <detail>` with the offending card
/// names indented below; the non-deterministic checklist prints as a
/// `note:` block. Both are results on stdout, not stderr diagnostics —
/// the whole block is the command's answer.
#[allow(clippy::too_many_arguments)]
fn print_report(
    out: &crate::output::Output,
    name: &str,
    format: &str,
    assumed: bool,
    bracket: Option<u8>,
    legal: bool,
    violations: &[Violation],
    note: &Option<BracketNote>,
    summary: &str,
) {
    let styles = out.styles();
    let mut format_line = format.to_string();
    if assumed {
        format_line.push_str(" (assumed; pass --format to override)");
    }
    println!(
        "{}  {}  {}",
        styles.header(name),
        styles.dim(&format_line),
        styles.dim(summary),
    );
    if let Some(bracket) = bracket {
        println!("  {} {bracket}", styles.dim("bracket"));
    }
    println!();
    if legal {
        println!("{}", styles.success("legal"));
    } else {
        println!(
            "{}",
            styles.error(&format!(
                "not legal ({} violation{})",
                violations.len(),
                if violations.len() == 1 { "" } else { "s" }
            ))
        );
        for v in violations {
            println!("  {}", styles.error(&format!("{}: {}", v.rule, v.detail)));
            for card in &v.cards {
                println!("    {card}");
            }
        }
    }
    if let Some(note) = note {
        println!();
        let has_verdicts = note
            .checks
            .iter()
            .any(|c| c.starts_with("PASS ") || c.starts_with("CHECK "));
        if has_verdicts {
            println!("{}", styles.note("bracket checks:"));
            for check in &note.checks {
                if let Some(rest) = check.strip_prefix("PASS ") {
                    println!("  {} {}", styles.success("✓"), rest);
                } else if let Some(rest) = check.strip_prefix("CHECK ") {
                    println!("  {} {}", styles.warning("!"), rest);
                } else {
                    println!("  - {check}");
                }
            }
        } else {
            println!("{}", styles.note("not checked automatically:"));
            for check in &note.checks {
                println!("  - {check}");
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/legal_tests.rs"]
mod legal_tests;
