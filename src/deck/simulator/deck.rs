//! Deck construction for the simulator: joins parsed deck text to card
//! rows and converts each entry into a `SimCard` via `parse`.

use super::model::{Format, Role, SimCard, SimDeck};
use super::oracle_lower::parse_sim_card;
use crate::db::CardRow;
use crate::deck::grammar::is_bench_section;
use std::collections::HashMap;

/// Convert one deck entry into a simulated card. `is_commander` marks the
/// command-zone card; a commander with a repeatable draw trigger counts as
/// a draw engine from the turn after it is cast.
fn make_sim_card(
    entry: &crate::deck::grammar::DeckEntry,
    cards: &HashMap<String, CardRow>,
) -> SimCard {
    match cards.get(&entry.name) {
        Some(card) => parse_sim_card(card),
        None => SimCard {
            name: entry.name.clone(),
            role: Role::Other,
            ..SimCard::default()
        },
    }
}

/// Build a `SimDeck` from parsed deck text joined to card rows.
///
/// Entries without oracle data still become zero-cost cards so they count
/// in totals; their diagnostics are limited. The sideboard never enters
/// the library: it is a wishlist, not a legal zone.
pub fn build_sim_deck(
    deck: &crate::deck::grammar::Deck,
    cards: &HashMap<String, CardRow>,
    format_key: Option<&str>,
) -> SimDeck {
    let has_commanders = deck.section_index("COMMANDER").is_some();
    let rules = match format_key {
        Some(key) => super::format::rules_for(key),
        None => super::format::rules_inferred(has_commanders),
    };
    let format = rules.shape;
    let commander_names: Vec<String> = match format {
        Format::Commander => deck
            .section_index("COMMANDER")
            .map(|i| deck.sections[i].1.iter().map(|e| e.name.clone()).collect())
            .unwrap_or_default(),
        Format::Constructed => Vec::new(),
    };
    let mut commanders = Vec::new();
    for name in &commander_names {
        let entry = crate::deck::grammar::DeckEntry {
            quantity: 1,
            name: name.clone(),
            set_code: None,
            collector_number: None,
            foil: false,
        };
        let mut sim = make_sim_card(&entry, cards);
        sim.role = Role::Wincon;
        commanders.push(sim);
    }
    let mut library = Vec::new();
    // Library = every section except COMMANDER and the bench
    // (SIDEBOARD/MAYBEBOARD). Checking the section source directly avoids
    // dropping a DECK copy whose name also appears in a bench section.
    for (section, entries) in &deck.sections {
        if section.eq_ignore_ascii_case("COMMANDER") || is_bench_section(section) {
            continue;
        }
        for entry in entries {
            if commander_names.contains(&entry.name) {
                continue;
            }
            for _ in 0..entry.quantity {
                library.push(make_sim_card(entry, cards));
            }
        }
    }
    // The companion lives in `cards` but never in the library: the
    // fetch step moves it to hand once {3} is paid (CR 702.139a).
    let companion = companion_card(deck, cards, &mut library);
    SimDeck {
        companion,
        cards: library,
        commanders,
        format,
        rules,
    }
}

/// The deck's companion (CR 702.139) as an index into `cards`.
///
/// A deck may reveal at most one companion (CR 103.2b), so the first
/// bench card with the Companion keyword wins. The condition is not
/// validated: the sim assumes a legal companion, documented in the
/// output assumptions. The card joins `cards` so casts can reference it,
/// but never the library.
fn companion_card(
    deck: &crate::deck::grammar::Deck,
    cards: &HashMap<String, CardRow>,
    library: &mut Vec<SimCard>,
) -> Option<super::model::CardIdx> {
    let entry = deck
        .sections
        .iter()
        .filter(|(section, _)| is_bench_section(section))
        .flat_map(|(_, entries)| entries)
        .find(|entry| {
            cards
                .get(&entry.name)
                .is_some_and(|card| parse_sim_card(card).has_companion)
        })?;
    let sim = make_sim_card(entry, cards);
    let idx = super::model::CardIdx(library.len() as u32);
    library.push(sim);
    Some(idx)
}

/// The deck's format key: `commander` when a COMMANDER section exists,
/// else `constructed` (the same inference `build_sim_deck` uses).
pub fn infer_format_key(deck: &crate::deck::grammar::Deck) -> String {
    let has_commanders = deck.section_index("COMMANDER").is_some();
    super::format::rules_inferred(has_commanders)
        .key
        .to_string()
}

/// An explicit `--format` may override the inferred one. A constructed run
/// of a commander deck shuffles the whole list (rough approximation).
pub fn apply_format_override(deck: &mut SimDeck, format: &str) -> bool {
    let rules = super::format::rules_for(format);
    if rules.shape == Format::Commander {
        return deck.format == Format::Commander;
    }
    if deck.format == Format::Commander {
        let mut merged = deck.commanders.clone();
        merged.extend(deck.cards.clone());
        *deck = SimDeck {
            companion: None,
            cards: merged,
            commanders: Vec::new(),
            format: Format::Constructed,
            rules,
        };
    } else {
        deck.rules = rules;
    }
    true
}
