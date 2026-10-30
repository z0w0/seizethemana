//! Full printing and ownership details for a human decklist row.

use super::{FinishPrices, entry_unit_price, set_name_for};
use crate::deck::{grammar::DeckEntry, ownership::SlotOwnership, stats};
use crate::output::{GlyphKind, Styles};
use std::collections::HashMap;

/// Print one entry without clipping its name, printing, or ownership details.
pub(super) fn print(
    styles: &Styles,
    conn: &rusqlite::Connection,
    entry: &DeckEntry,
    slots: &HashMap<String, SlotOwnership>,
    cards_by_name: &HashMap<String, crate::db::CardRow>,
    prices: &HashMap<String, FinishPrices>,
    set_names: &mut HashMap<String, String>,
) {
    let basic = cards_by_name
        .get(&entry.name)
        .is_some_and(stats::is_tracked_basic);
    let slot = slots.get(&entry.name);
    let owned = slot.map(|slot| slot.in_deck + slot.in_binder).unwrap_or(0);
    let elsewhere = slot.map(|slot| slot.held_elsewhere).unwrap_or(0);
    let line = entry.to_line();
    let name_len = format!("{} ", entry.quantity).len() + entry.name.len();
    let rest = line
        .get(name_len..)
        .expect("deck line starts with its quantity and name")
        .trim();
    let owned_display = if basic {
        styles.dim("(assumed available)")
    } else {
        let kind = if owned >= entry.quantity {
            GlyphKind::Good
        } else if owned > 0 {
            GlyphKind::Warn
        } else {
            GlyphKind::Bad
        };
        styles.glyph(&format!("({owned} available by name)"), kind)
    };
    let set_note = match &entry.set_code {
        Some(code) => {
            let full = set_names
                .entry(code.clone())
                .or_insert_with(|| set_name_for(conn, code).unwrap_or_default());
            if full.is_empty() {
                String::new()
            } else {
                format!(" ({full})")
            }
        }
        None => String::new(),
    };
    let elsewhere = if elsewhere > 0 && !basic {
        styles.dim(&format!(" ({elsewhere} in other decks)"))
    } else {
        String::new()
    };
    let price_note = match entry_unit_price(entry, prices) {
        Some(unit) if !basic => format!(" {} each", styles.money(unit)),
        None if !basic => format!(" {}", styles.dim("unpriced")),
        _ => String::new(),
    };
    crate::output::print_wrapped(&format!(
        "  {:>2} {} {}{}  {owned_display}{elsewhere}{price_note}",
        entry.quantity,
        styles.card_name(&entry.name),
        styles.dim(rest),
        styles.dim(&set_note),
    ));
}
