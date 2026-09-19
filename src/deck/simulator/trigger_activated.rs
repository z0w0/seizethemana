// Activated-ability and cast-shape parsing for the simulator, split
// from triggers.rs to keep files small.

use super::model::Ability;
use super::parse::parse_ability;

/// Activated abilities on plain cards ("{T}: Draw a card",
/// "{1}, {T}: …", "−3: …", "{2}, Sacrifice a creature: …"). The
/// station-tier path handles tiered activations; this covers the rest.
pub(super) fn activated_trigger(seg: &str, lower: &str, out: &mut Vec<Ability>) -> bool {
    // Activated abilities on plain cards ("{T}: Draw a card",
    // "{1}, {T}: …", "−3: …", "{2}, Sacrifice a creature: …"). The
    // station-tier path handles tiered activations; this covers the
    // rest. Loyalty activations start with a minus sign. Banked
    // activations ("Remove a charge counter …: Add …") start with a
    // remove clause and consume a counter per fire.
    if (seg.starts_with('{')
        || lower.starts_with("tap:")
        || lower.starts_with('−')
        || lower.starts_with('-')
        || lower.starts_with('+')
        || lower.starts_with("sacrifice")
        || lower.starts_with("remove a charge counter"))
        && let Some(ab) = parse_ability(seg.trim())
    {
        out.push(ab);
        return true;
    }
    false
}

/// True when the segment reads as an enters-the-battlefield shape even
/// without the strict "when … enters" prefix (sagas, cast triggers).
pub(super) fn etb_shape(lower: &str) -> bool {
    lower.starts_with("at the beginning of your end step") || lower.starts_with("when you cast")
}

/// The best trigger guess for a segment by its opening words.
pub(super) fn trigger_for(lower: &str) -> super::model::Trigger {
    if lower.starts_with("whenever ") && lower.contains("attack") {
        super::model::Trigger::OnAttack
    } else if lower.contains("deals combat damage") {
        super::model::Trigger::OnCombatDamage
    } else if lower.starts_with("at the beginning of your upkeep")
        || lower.starts_with("at the beginning of your end step")
    {
        super::model::Trigger::OnUpkeep
    } else if lower.starts_with("whenever you cast") || lower.starts_with("when you cast") {
        super::model::Trigger::OnCastSpell
    } else if lower.contains("enters") {
        super::model::Trigger::OnEnter
    } else {
        super::model::Trigger::OnUpkeep
    }
}

/// Mill amount from text ("mill three cards", "mill 10"). Scans every
/// "mill " occurrence so card names ("Mill Fiend") do not swallow the
/// real clause.
pub fn mill_amount(text: &str) -> u32 {
    let mut best = 0;
    let mut from = 0;
    while let Some(rel) = text[from..].find("mill ") {
        let start = from + rel + 5;
        let tail = &text[start..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        let found = if !digits.is_empty() {
            digits.parse::<u32>().unwrap_or(0)
        } else {
            let mut hit = 0;
            for (word, n) in [
                ("seven", 7u32),
                ("six", 6),
                ("five", 5),
                ("four", 4),
                ("three", 3),
                ("two", 2),
                ("one", 1),
            ] {
                if tail.starts_with(word) {
                    hit = n;
                    break;
                }
            }
            hit
        };
        best = best.max(found);
        from = start;
    }
    best
}
