// Land-shape and tap-yield parsing for the simulator: tap yields, verge
// gates, enters-tapped, enter counters, and spend restrictions, split
// from parse.rs to keep files small.

use super::model::{Restriction, Scale, TapYield};
use crate::db::CardRow;

/// Parse one "add" clause into a tap yield. Juxtaposed symbols with no
/// "or" produce fixed simultaneous pips (Jegantha's `{W}{U}{B}{R}{G}`);
/// "or" between symbols and "one mana of any color" prose are choices.
pub fn parse_tap_yield(text: &str) -> Option<TapYield> {
    let lower = text.to_ascii_lowercase();
    // Opponent-dependent production ("any color that a land an opponent
    // controls could produce") reads as best-case any-color from turn 2:
    // the goldfish has no opponents, but a real table does, and the sim
    // is best-case everywhere else.
    let opponent = lower.contains("opponent");
    let mut yield_ = TapYield {
        opponent_any: opponent,
        ..TapYield::default()
    };
    // Any-color amounts: "one mana of any color" (1), "N mana of any one
    // color" (N), "N mana in any combination of colors" (N).
    let number_words: &[(&str, u32)] = &[
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
    ];
    if lower.contains("any combination of colors") || lower.contains("mana of any one color") {
        // The amount precedes "mana": "Add three mana of any one color".
        let mut amount = 1u32;
        for (word, n) in number_words {
            if lower.contains(&format!("add {word} mana"))
                || lower.contains(&format!(", {word} mana"))
                || lower.contains(&format!(" {word} mana"))
            {
                amount = *n;
                break;
            }
        }
        yield_.any_pips = amount.max(1);
        yield_.choice = [false; 5];
        return Some(yield_);
    }
    // Conditional any-color: "one mana of any color among X you control"
    // (Mox Amber, Plaza of Heroes). Parsed as ColorsPresent scaling: the
    // output grows with the matching permanents on the battlefield.
    if lower.contains("one mana of any color among") && lower.contains("you control") {
        yield_.scaling = Some(Scale::ColorsPresent);
        return Some(yield_);
    }
    if lower.contains("one mana of any color") {
        yield_.any_pips = 1;
        yield_.choice = [false; 5];
        return Some(yield_);
    }
    // Scaling producers: "for each color among permanents you control".
    if lower.contains("for each color among permanents you control") {
        yield_.scaling = Some(Scale::ColorsPresent);
        return Some(yield_);
    }
    // Per-counter producers: "Add one mana of that color for each charge
    // counter on this" (Astral Cornucopia). One activation = one
    // any-color pip per counter, resolved at activation.
    if lower.contains("for each charge counter") && lower.contains("mana") {
        yield_.scaling = Some(Scale::PerChargeCounter);
        yield_.any_pips = 1;
        return Some(yield_);
    }
    if lower.contains(" or ") {
        for clause in lower.split(" or ") {
            for symbol in clause.split(['{', '}']).filter(|s| !s.is_empty()) {
                let upper = symbol.to_ascii_uppercase();
                if upper.len() == 1 {
                    let ch = upper.chars().next().unwrap_or(' ');
                    if let Some(idx) = super::model::COLORS.iter().position(|c| *c == ch) {
                        yield_.choice[idx] = true;
                    } else if ch == 'C' {
                        yield_.colorless += 1;
                    }
                }
            }
        }
    } else {
        // No "or": juxtaposed symbols are one simultaneous set.
        for symbol in lower.split(['{', '}']).filter(|s| !s.is_empty()) {
            let upper = symbol.to_ascii_uppercase();
            if upper.len() == 1 {
                let ch = upper.chars().next().unwrap_or(' ');
                if let Some(idx) = super::model::COLORS.iter().position(|c| *c == ch) {
                    yield_.fixed[idx] += 1;
                } else if ch == 'C' {
                    yield_.colorless += 1;
                }
            }
        }
    }
    if yield_.total() == 0 {
        return None;
    }
    Some(yield_)
}

/// Tap yield from `{T}: Add …` segments, merged across abilities (a
/// permanent taps once; later abilities merge as the union of colors).
/// Lands whose oracle grants them a basic type ("This land is the chosen
/// type") tap for that type's color without an explicit add clause.
pub fn parse_tap(row: &CardRow) -> Option<TapYield> {
    parse_tap_generic(row)
}

/// Tap yield with the per-cast engine's own tap clause ("add one mana
/// for each spell you've cast") dropped, so the engine mode and a plain
/// one-mana tap do not double count on the same permanent.
pub fn parse_tap_filtered(row: &CardRow) -> Option<TapYield> {
    let oracle: String = row
        .oracle_text
        .lines()
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            !(l.contains("spell you've cast") && l.contains("add "))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let filtered = CardRow {
        oracle_text: oracle,
        ..row.clone()
    };
    parse_tap_generic(&filtered)
}

/// The shared tap parser.
fn parse_tap_generic(row: &CardRow) -> Option<TapYield> {
    // Basic lands wrap the oracle text in parens: "({T}: Add {G}.)".
    let oracle = row.oracle_text.trim_start_matches('(');
    let segments: Vec<&str> = oracle.split(['\n', '.']).map(str::trim).collect();
    // Type-granted lands ("This land is the chosen type"): no add clause,
    // but the chosen basic type is the player's choice each game, so the
    // tap reads as one mana of any color.
    let lower_all = oracle.to_ascii_lowercase();
    if segments.iter().all(|s| {
        !(s.to_ascii_lowercase().starts_with("{t}")
            || s.to_ascii_lowercase().contains(": add"))
    } && (lower_all.contains("this land is the chosen type")
        || lower_all.contains("this land is every basic land type")))
    {
        return Some(TapYield {
            any_pips: 1,
            ..TapYield::default()
        });
    }
    // Station tier lines ("12+ | {U}, {T}: Add …") belong to a tier, not
    // to the base card; parse_tap must not double-count them.
    let tier_lines: Vec<usize> = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.split_once('|').is_some_and(|(head, _)| {
                head.trim()
                    .trim_end_matches('+')
                    .trim()
                    .parse::<u32>()
                    .is_ok()
            })
        })
        .map(|(i, _)| i)
        .collect();
    let mut merged: Option<TapYield> = None;
    for (si, seg) in segments.iter().enumerate() {
        // Skip tier segments and any segment a tier marker precedes.
        if tier_lines.contains(&si)
            || tier_lines
                .iter()
                .any(|t| *t > si && segments[si..*t].iter().all(|s| s.is_empty()))
        {
            continue;
        }
        let lower = seg.to_ascii_lowercase();
        if !lower.starts_with("{t}") && !lower.contains(": add") {
            // Scaling adds read as bare sentences after a "{T}: Choose a
            // color" segment (Astral Cornucopia); accept an "add"-leading
            // continuation only when the previous segment carried the tap.
            let continuation = lower.starts_with("add ")
                && si > 0
                && segments[si - 1].to_ascii_lowercase().starts_with("{t}");
            if !continuation {
                continue;
            }
        }
        let body = match seg.split_once(':') {
            Some((_, body)) => body,
            // A bare "Add …" continuation sentence has no colon; its whole
            // text is the yield.
            None if lower.starts_with("add ") => seg,
            None => continue,
        };
        if let Some(y) = parse_tap_yield(body) {
            // A gated mode ("Activate only if you control …" in the
            // segment itself or the next one) joins as a choice color,
            // not a fixed pip: it does not produce every turn.
            let gated = lower.contains("activate only if")
                || segments.get(si + 1).is_some_and(|next| {
                    tier_lines.contains(&(si + 1))
                        || next.to_ascii_lowercase().contains("activate only if")
                });
            // Spend restriction: "spend this mana only to cast …" (creature
            // spells: Secluded Courtyard; a legendary spell: Plaza of
            // Heroes; artifact spells: Steelswarm Operator; instant and
            // sorcery spells: Hydro-Channeler). The clause follows the add
            // segment as its own sentence.
            let restriction_window: String = segments[si..]
                .iter()
                .take(3)
                .map(|s| s.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let restriction = spend_restriction(&restriction_window);
            let y = if gated {
                let mut choice_only = y.clone();
                choice_only.choice = y.fixed.map(|p| p > 0);
                choice_only.fixed = [0; 5];
                choice_only
            } else {
                y
            };
            let mut y = y;
            y.restriction = restriction;
            merged = Some(match merged.take() {
                None => y,
                Some(prev) => {
                    // Several tap abilities on one permanent: one tap
                    // yields one mana of any reachable color. A restricted
                    // mode restricts the merged tap (the unrestricted
                    // colorless mode produces nothing of value).
                    let mut m = TapYield {
                        alternatives: true,
                        restriction: prev.restriction.or(y.restriction),
                        opponent_any: prev.opponent_any || y.opponent_any,
                        ..TapYield::default()
                    };
                    m.any_pips = (prev.any_pips + y.any_pips).max(1);
                    for i in 0..5 {
                        m.choice[i] =
                            prev.choice[i] || y.choice[i] || prev.fixed[i] > 0 || y.fixed[i] > 0;
                    }
                    m.colorless = u32::from(prev.colorless > 0 || y.colorless > 0);
                    m
                }
            });
        }
    }
    merged
}

/// Spend restriction from a "spend this mana only to cast …" window:
/// creature, legendary, artifact, or instant-and-sorcery spells.
pub(super) fn spend_restriction(window: &str) -> Option<Restriction> {
    if !window.contains("only to cast") {
        return None;
    }
    if window.contains("creature") {
        Some(Restriction::Creature)
    } else if window.contains("legendary") {
        Some(Restriction::Legendary)
    } else if window.contains("artifact") {
        Some(Restriction::Artifact)
    } else if window.contains("instant and sorcery") || window.contains("instant or sorcery") {
        Some(Restriction::InstantSorcery)
    } else {
        None
    }
}

/// Gate colors of a verge-style land: the types listed after the second
/// `{T}:` ability's "Activate only if you control …".
pub fn parse_gates(oracle_text: &str) -> Vec<&'static str> {
    let text = oracle_text.to_ascii_lowercase();
    // The gate clause names basic types; any type mentioned after
    // "Activate only if" is gated (the ungated mode is the first).
    let Some(idx) = text.find("activate only if") else {
        return Vec::new();
    };
    let gate_text = &text[idx..];
    let mut gates = Vec::new();
    for (word, kind) in [
        ("plains", "Plains"),
        ("island", "Island"),
        ("swamp", "Swamp"),
        ("mountain", "Mountain"),
        ("forest", "Forest"),
    ] {
        if gate_text.contains(&format!("control a {word}"))
            || gate_text.contains(&format!("control an {word}"))
            // "control a Swamp or a Mountain": the second type follows an
            // "or a" clause.
            || gate_text.contains(&format!("or a {word}"))
            || gate_text.contains(&format!("or an {word}"))
        {
            gates.push(kind);
        }
    }
    gates
}

/// Enters-tapped oracle check for lands.
/// Enters-tapped oracle check for lands. Best-case reading: the shock-dual
/// life-payment clause ("you may pay 2 life") stays untapped; unconditional
/// "enters tapped" texts are tapped.
pub fn enters_tapped(text: &str) -> bool {
    // Shock duals and MDFC "you may pay 3 life" lands: the sim's
    // best-case agent pays any printed life.
    if text.contains("you may pay 2 life")
        || text.contains("unless you pay 2 life")
        || text.contains("you may pay 3 life")
        || text.contains("unless you pay 3 life")
    {
        return false;
    }
    // "Enters tapped unless …" conditions that self-solve early
    // ("unless you control two or fewer other lands", first turns) are
    // treated untapped; other unless-conditions as tapped.
    if text.contains("enters tapped unless") {
        return !text.contains("two or fewer other lands")
            && !text.contains("it's your first, second, or third turn");
    }
    text.contains("enters tapped") || text.contains("enters the battlefield tapped")
}

/// Sentinel from [`parse_enter_counters`]: the card "enters with X
/// charge counters" — the cast leftover converts to counters at entry.
pub const X_ENTRY_COUNTERS: u32 = u32::MAX;

/// Charge counters the card enters with ("enters with three charge
/// counters on it" → 3). The search stays inside the same sentence so a
/// later "{2}, {T}" activation does not leak a number. "Enters with X
/// charge counters" returns the [`X_ENTRY_COUNTERS`] sentinel.
pub fn parse_enter_counters(text: &str) -> u32 {
    // "Enters with X charge counters": the cast leftover converts to
    // counters (Astral Cornucopia). Checked before Sunburst, which the
    // reminder text of the same card carries.
    if text.contains("enters with x charge counters") {
        return X_ENTRY_COUNTERS;
    }
    // Sunburst (best case): two colors paid on-curve → 2 counters.
    if text.contains("sunburst") {
        return 2;
    }
    if !text.contains("enters with") {
        return 0;
    }
    let Some(idx) = text.find("enters with") else {
        return 0;
    };
    let tail: &str = text[idx..].split(['.', '\n', ',']).next().unwrap_or("");
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if !digits.is_empty() {
        return digits.parse::<u32>().unwrap_or(0);
    }
    for (word, n) in [
        ("one", 1u32),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
    ] {
        if tail.contains(&format!("{word} ")) {
            return n;
        }
    }
    0
}

/// "Put N charge counters" on cast (Drill Too Deep).
pub fn charge_counters_on_cast(text: &str) -> u32 {
    if !text.contains("charge counter") {
        return 0;
    }
    let Some(idx) = text.find("put ") else {
        return 0;
    };
    let tail = &text[idx + 4..];
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        return digits.parse::<u32>().unwrap_or(0);
    }
    for (word, n) in [
        ("five", 5u32),
        ("four", 4),
        ("three", 3),
        ("two", 2),
        ("one", 1),
    ] {
        if tail.starts_with(word) {
            return n;
        }
    }
    0
}
