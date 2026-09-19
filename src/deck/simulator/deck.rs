// Deck construction for the simulator: joins parsed deck text to card
// rows and converts each entry into a `SimCard` via `parse`.

use super::model::{Ability, Effect, Format, Role, SimCard, SimDeck, Tier, Trigger};
use super::parse::parse_sim_card;
use crate::db::CardRow;
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

/// The commander's engine tier: draw 1 per turn while on the battlefield
/// when its oracle shows an unconditional repeatable draw — upkeep or
/// end-step triggers. Attack-gated draws ("Whenever N attacks, draw…")
/// stay off: they need animation and combat, which the normal attack path
/// models.
fn commander_engine_tier(cmd: &CardRow) -> Option<Tier> {
    let lower = cmd.oracle_text.to_ascii_lowercase();
    let engine = (lower.starts_with("at the beginning of your upkeep")
        || lower.starts_with("at the beginning of your end step"))
        && (lower.contains("draw") || lower.contains("investigate"));
    engine.then_some(Tier {
        at: 0,
        animate: false,
        abilities: vec![Ability {
            trigger: Trigger::OnUpkeep,
            effect: Effect::Draw(1),
            ..Ability::default()
        }],
    })
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
        if let Some(card) = cards.get(name)
            // The synthetic draw tier only fills the gap when the real
            // parse produced no OnUpkeep draw already (otherwise both
            // fire and upkeep draws double).
            && sim.abilities().all(|a| {
                !(a.trigger == Trigger::OnUpkeep && matches!(a.effect, Effect::Draw(_)))
            })
            && let Some(tier) = commander_engine_tier(card)
        {
            sim.station_tiers.insert(0, tier);
        }
        commanders.push(sim);
    }
    let mut library = Vec::new();
    // Library = every section except COMMANDER and SIDEBOARD. Checking the
    // section source directly avoids dropping a DECK copy whose name also
    // appears in the sideboard wishlist.
    for (section, entries) in &deck.sections {
        if section.eq_ignore_ascii_case("COMMANDER") || section.eq_ignore_ascii_case("SIDEBOARD") {
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
    SimDeck {
        cards: library,
        commanders,
        format,
        rules,
    }
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
