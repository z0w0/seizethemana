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

/// Count copies per card name across all sections (sideboard included).
pub(super) fn copies_by_name(deck: &Deck) -> Vec<(String, i64)> {
    copies_in_sections(deck, |_| true)
}

/// Count copies per card name outside SIDEBOARD sections.
///
/// For commander-style formats the sideboard is a wishlist, not a legal
/// zone, so rules that bind the deck itself read this count.
pub(super) fn maindeck_copies_by_name(deck: &Deck) -> Vec<(String, i64)> {
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
        // Deck size: each singleton format carries its own exact count
        // (commander 100 including commander, brawl 60, oathbreaker 59).
        // Sideboard cards do not count (in commander it is a wishlist, not
        // a legal sideboard).
        let expected = singleton_deck_size(format.unwrap_or(""));
        if maindeck_total != expected {
            violations.push(Violation {
                rule: "deck size".into(),
                cards: Vec::new(),
                detail: format!(
                    "{maindeck_total} cards; {} decks are exactly {expected}",
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
        if !commander_section.is_empty() {
            if let Some(v) = commander_legal(&commander_section, cards) {
                violations.push(v);
            }
            // Color identity of every other card must sit inside the
            // commander's. Maindeck only: sideboard copies are a
            // wishlist and never count as violations. Skip the check
            // entirely when no commander resolved: an unknown name is
            // already reported as an unknown card, and guessing ""
            // identity would flag every colored card.
            let commanders_resolved = commander_section.len() == 1
                && cards
                    .get(&commander_section[0])
                    .is_some_and(is_commander_type);
            let identity = commander_section
                .iter()
                .filter_map(|n| cards.get(n))
                .map(color_identity)
                .collect::<String>();
            let offenders: Vec<String> = if !commanders_resolved {
                Vec::new()
            } else {
                maindeck_counts
                    .iter()
                    .filter(|(name, _)| {
                        !commander_section.contains(name)
                            && cards.get(name).is_some_and(|c| !identity_ok(c, &identity))
                    })
                    .map(|(name, _)| name.clone())
                    .collect()
            };
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
        if let Some(limit) = limit {
            let capped = format!(
                "{} Game Changers; the bracket-{bracket} hard cap is {limit} (Game Changer count is the hard bracket rule)",
                changers.len()
            );
            if changers.len() > limit as usize {
                violations.push(Violation {
                    rule: "game changers".into(),
                    cards: changers.clone(),
                    detail: capped,
                });
            }
        }
        // The sideboard is the upgrade kit: surface its Game Changers so a
        // reader previewing a bracket bump can see what comes along. Every
        // SIDEBOARD section counts (imports may keep more than one).
        let mut sideboard_changers: Vec<String> = Vec::new();
        for (name, entries) in &deck.sections {
            if !name.eq_ignore_ascii_case("SIDEBOARD") {
                continue;
            }
            sideboard_changers.extend(
                entries
                    .iter()
                    .filter(|e| {
                        cards
                            .get(&e.name)
                            .is_some_and(|c| c.game_changer == Some(true))
                    })
                    .map(|e| e.name.clone()),
            );
        }
        sideboard_changers.sort();
        sideboard_changers.dedup();
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
    }

    (violations, advisories)
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

/// Oracle-text signals for one bracket: library searchers, extra turns,
/// mass land denial, and alternate wins.
///
/// Returns verdict lines ("PASS"/"CHECK"/"ADVISE" prefixes) plus advisory
/// notes naming the matched cards. Counts are official bracket guidance,
/// not hard rules.
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
    // Library searchers, split by how the official bracket guidance treats
    // them. Hard tutors are one-shot spells ("search your library for a
    // card"): they fetch combo pieces and are the class the best-of list
    // sits on — several are Game Changers, so the GC count does the real
    // work at bracket 3. Soft searchers are ETB/activated effects with
    // restrictions ("into your hand", mana-value caps, sacrifice costs):
    // utility, not combo delivery.
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
    let mut searchers = scan("search your library for");
    searchers.extend(scan("search your library and/or graveyard"));
    searchers.sort();
    searchers.dedup();
    // Hard tutor: a spell (instant/sorcery) whose search is the card's
    // whole job — a one-shot tutor. Soft: ETB/activated searchers and
    // one-shots with utility twists (a "put it into your hand" clause on
    // a spell is still a tutor; an activated "sacrifice an artifact:"
    // cost, an MV cap, or a battlefield-reveal shape is not).
    let is_hard_tutor = |name: &str| -> bool {
        let Some(card) = cards.get(name) else {
            return false;
        };
        let text = card.oracle_text.to_lowercase();
        let spell = card.type_line.contains("Instant") || card.type_line.contains("Sorcery");
        let activated_or_etb = text.contains("sacrifice an artifact")
            || text.contains("mana value equal to")
            || text.contains("when ")
            || text.contains("whenever ")
            || text.contains(", {t}")
            || text.contains("reveal cards from the top");
        spell && !activated_or_etb
    };
    let hard: Vec<String> = searchers
        .iter()
        .map(|name| (*name).clone())
        .filter(|name| !is_land_ramp(name) && is_hard_tutor(name))
        .collect();
    let soft: Vec<String> = searchers
        .iter()
        .map(|name| (*name).clone())
        .filter(|name| !is_land_ramp(name) && !is_hard_tutor(name))
        .collect();
    let ramp = searchers
        .iter()
        .map(|name| (*name).clone())
        .filter(|name| is_land_ramp(name))
        .collect::<std::collections::BTreeSet<_>>();
    // Advisory counts are official guidance, not hard rules: tutors are
    // "sparse" in brackets 1-2, and bracket 3's only hard cap is the
    // Game Changer allowance (the best tutors are on that list).
    match (bracket, hard.len() + soft.len()) {
        (1 | 2, 0) => out.push("PASS library search: none found".to_string()),
        (1 | 2, n) => out.push(format!(
            "CHECK library search: {} card(s) search the library (official guidance: tutors should be sparse; no tutors for combo pieces): {}",
            n,
            {
                let mut names = hard.clone();
                names.extend(soft.clone());
                names.join(", ")
            }
        )),
        (3, n) if n > 0 => out.push(format!(
            "ADVISE library search: {} card(s) search the library (advisory; the bracket-3 hard cap is 3 Game Changers, which includes the best tutors): {}",
            n,
            {
                let mut names = hard.clone();
                names.extend(soft.clone());
                names.join(", ")
            }
        )),
        _ => {}
    }
    if !hard.is_empty() {
        out.push(format!(
            "note hard tutors: {} card(s) are one-shot search spells (combo delivery): {}",
            hard.len(),
            hard.join(", ")
        ));
    }
    if !soft.is_empty() {
        out.push(format!(
            "note soft searchers: {} card(s) are ETB/activated/restricted searchers (utility): {}",
            soft.len(),
            soft.join(", ")
        ));
    }
    if !ramp.is_empty() {
        out.push(format!(
            "note ramp: {} card(s) search for lands (ramp, not combo tutors): {}",
            ramp.len(),
            ramp.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    // Extra turns: official wording is "low quantities, not chained".
    let extra_turns = scan("extra turn");
    match (bracket, extra_turns.len()) {
        (1..=3, 0) => out.push("PASS extra turns: none found".to_string()),
        (1..=3, n) => out.push(format!(
            "CHECK extra turns: {} card(s) grant an extra turn (official guidance: low quantities, not chained in succession): {}",
            n,
            extra_turns.join(", ")
        )),
        _ => {}
    }
    // Mass land denial: officially "should not be expected anywhere in
    // brackets 1-3". The needles cover the standard wordings: destroy,
    // exile, bounce-all, and untap-lock.
    let mld_needles = [
        "destroy all lands",
        "destroy all non",
        "exile all lands",
        "return all lands",
        "lands don't untap",
        "lands you control don't untap",
        "doesn't untap lands",
    ];
    // "destroy all non" (Ruination-class) needs a land word nearby, but
    // "nonland permanents" (a nonland sweeper) does not count: exclude any
    // "nonland" hit and require the word "land" with a word boundary.
    let mld: std::collections::BTreeSet<String> = mld_needles
        .iter()
        .flat_map(|needle| scan(needle))
        .filter(|name| {
            cards.get(name).is_some_and(|c| {
                let text = c.oracle_text.to_lowercase();
                !text.contains("nonland")
                    && text
                        .split(|c: char| !c.is_alphabetic())
                        .any(|word| word == "land" || word == "lands")
            })
        })
        .collect();
    match (bracket, mld.len()) {
        (1..=3, 0) => out.push("PASS mass land destruction: none found".to_string()),
        (1..=3, _) => out.push(format!(
            "CHECK mass land destruction: {} card(s) deny several lands (official rule: none in brackets 1-3): {}",
            mld.len(),
            mld.into_iter().collect::<Vec<_>>().join(", ")
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
            "no mass land denial",
            "extra turns only in low quantities, not chained",
            "tutors should be sparse",
            "no Game Changers (hard cap)",
        ],
        3 => &[
            "at most 3 Game Changers (hard cap)",
            "no mass land denial",
            "no intentional early-game two-card infinite combos",
            "extra turns only in low quantities, not chained",
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
