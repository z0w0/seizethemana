// Commander bracket signal scans: Game Changer allowance, oracle-text
// scans for tutors/extra turns/mass land denial/alternate wins, and the
// bracket checklist. Split from `legal.rs` to keep each file small.

use super::super::grammar::Deck;
use super::{BracketNote, maindeck_copies_by_name};
use crate::db::CardRow;

use std::collections::HashMap;

/// Game Changer allowance per Commander bracket: none for 1–2, at most 3 for
/// bracket 3, unlimited (None) for 4–5.
pub fn game_changer_limit(bracket: u8) -> Option<u8> {
    match bracket {
        1 | 2 => Some(0),
        3 => Some(3),
        _ => None,
    }
}

/// Searcher classification for the bracket scan: names that search the
/// library split into hard tutors (one-shot search spells), soft searchers
/// (ETB/activated/restricted effects), and land ramp.
struct SearchScan {
    hard: Vec<String>,
    soft: Vec<String>,
    /// BTreeSet: deduped, sorted for the note line.
    ramp: std::collections::BTreeSet<String>,
}

/// Classify the deck's library searchers for the bracket scan.
///
/// Hard tutor: a spell (instant/sorcery) whose search is the card's whole
/// job. Soft: ETB/activated searchers and one-shots with utility twists.
/// Land ramp searches for basic lands and never counts as a tutor.
fn classify_searchers(maindeck: &[(String, i64)], cards: &HashMap<String, CardRow>) -> SearchScan {
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
    SearchScan {
        hard: searchers
            .iter()
            .filter(|name| !is_land_ramp(name) && is_hard_tutor(name))
            .cloned()
            .collect(),
        soft: searchers
            .iter()
            .filter(|name| !is_land_ramp(name) && !is_hard_tutor(name))
            .cloned()
            .collect(),
        ramp: searchers
            .iter()
            .filter(|name| is_land_ramp(name))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
    }
}

/// Verdict + note lines for the searcher classification, per bracket.
fn searcher_verdicts(scan: &SearchScan, bracket: u8) -> Vec<String> {
    let mut out = Vec::new();
    // Advisory counts are official guidance, not hard rules: tutors are
    // "sparse" in brackets 1-2, and bracket 3's only hard cap is the
    // Game Changer allowance (the best tutors are on that list).
    let all_names = {
        let mut names = scan.hard.clone();
        names.extend(scan.soft.clone());
        names.join(", ")
    };
    match (bracket, scan.hard.len() + scan.soft.len()) {
        (1 | 2, 0) => out.push("PASS library search: none found".to_string()),
        (1 | 2, n) => out.push(format!(
            "CHECK library search: {n} card(s) search the library (official guidance: tutors should be sparse; no tutors for combo pieces): {all_names}"
        )),
        (3, 0) => out.push("ADVISE library search: none found".to_string()),
        (3, n) if n > 0 => out.push(format!(
            "ADVISE library search: {n} card(s) search the library (advisory; the bracket-3 hard cap is 3 Game Changers, which includes the best tutors): {all_names}"
        )),
        _ => {}
    }
    if !scan.hard.is_empty() {
        out.push(format!(
            "note hard tutors: {} card(s) are one-shot search spells (combo delivery): {}",
            scan.hard.len(),
            scan.hard.join(", ")
        ));
    }
    if !scan.soft.is_empty() {
        out.push(format!(
            "note soft searchers: {} card(s) are ETB/activated/restricted searchers (utility): {}",
            scan.soft.len(),
            scan.soft.join(", ")
        ));
    }
    if !scan.ramp.is_empty() {
        out.push(format!(
            "note ramp: {} card(s) search for lands (ramp, not combo tutors): {}",
            scan.ramp.len(),
            scan.ramp.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    out
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

    let mut out = searcher_verdicts(&classify_searchers(&maindeck, cards), bracket);
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
    // "nonland permanents" (a nonland sweeper) does not count: exclude
    // per sentence, so a modal card with a nonland sweeper and a real
    // land-denial mode still scans on its MLD sentence.
    let mld: std::collections::BTreeSet<String> = mld_needles
        .iter()
        .flat_map(|needle| scan(needle).into_iter().map(move |name| (needle, name)))
        .filter(|(needle, name)| {
            cards.get(name).is_some_and(|c| {
                let text = c.oracle_text.to_lowercase();
                text.split(['.', '\n']).any(|sentence| {
                    sentence.contains(*needle)
                        && !sentence.contains("nonland")
                        && sentence
                            .split(|c: char| !c.is_alphabetic())
                            .any(|word| word == "land" || word == "lands")
                })
            })
        })
        .map(|(_, name)| name)
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
        (3, 0) => out.push(
            "PASS alternate wins: no 'you win the game' text found".to_string(),
        ),
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
