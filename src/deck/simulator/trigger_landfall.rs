// Landfall trigger family and token-count parsing for the simulator,
// split from triggers.rs to keep files small.

use super::model::{Ability, Effect, draw_amount};

/// Landfall trigger family: "Whenever a land you control enters, X" /
/// "Landfall — Whenever a land enters, X". Draw, tokens, mana, ramp
/// (extra land), and drain riders map; +1/+1 counters ride the combat
/// power path via the `landfall` flag.
pub(super) fn landfall_trigger(lower: &str, out: &mut Vec<Ability>) -> bool {
    let landfall = lower.contains("landfall")
        || (lower.contains("whenever a land") && lower.contains("enters"));
    if !landfall || lower.contains("opponent") {
        return false;
    }
    if (lower.contains("add one mana") || lower.contains("add {")) && !lower.contains("draw") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::ExtraLand,
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("draw") && !lower.contains("discard") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::Draw(draw_amount(lower).max(1)),
            ..Ability::default()
        });
        return true;
    }
    if lower.contains("create") && lower.contains("token") {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::Tokens(token_amount(lower)),
            ..Ability::default()
        });
        return true;
    }
    if (lower.contains("search") || lower.contains("put the top"))
        && lower.contains("land")
        && lower.contains("onto the battlefield")
    {
        out.push(Ability {
            trigger: super::model::Trigger::OnEnter,
            effect: Effect::ExtraLand,
            ..Ability::default()
        });
        return true;
    }
    false
}

/// Token count from "create N …token(s)" / "for each X you control"
/// scaling. Word and numeral amounts; "for each" shapes cap at 8 (go-wide
/// boards stay bounded). Bare "create a token" stays 1; legacy 2-pip
/// default applies when the amount is unreadable ("create two 1/1"
/// = 2).
pub fn token_amount(lower: &str) -> u32 {
    if lower.contains("for each") {
        return 8;
    }
    let Some(idx) = lower.find("create") else {
        return 2;
    };
    let tail = &lower[idx + 6..];
    let digits: String = tail
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if let Ok(n) = digits.parse::<u32>()
        && n > 0
    {
        return n.min(8);
    }
    for (word, n) in [
        ("one", 1u32),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
        ("seven", 7),
        ("eight", 8),
    ] {
        if tail.trim_start().starts_with(word) {
            return n;
        }
    }
    // "a token", "that many tokens": 1 when singular, else the legacy 2.
    if tail.trim_start().starts_with("a ") || tail.trim_start().starts_with("an ") {
        return 1;
    }
    2
}
