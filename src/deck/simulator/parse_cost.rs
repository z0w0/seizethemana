// Mana-cost parsing for the simulator: Scryfall cost strings → Cost
// values, split from parse.rs to keep files small.

use super::model::Cost;

/// Parse a Scryfall mana-cost string (`{2}{W}{W}`, `{W/U}`, `{1}{B/P}`,
/// multi-face `{2}{B} // {B}`) into a cost. Multi-face costs union the
/// faces; hybrid pips become flexible pips.
pub fn parse_cost(mana_cost: &str) -> Cost {
    let mut cost = Cost::default();
    for symbol in mana_cost.split(['{', '}']).filter(|s| !s.is_empty()) {
        if symbol == "//" {
            continue;
        }
        let upper = symbol.to_ascii_uppercase();
        if upper.chars().all(|c| c.is_ascii_digit()) {
            cost.generic += upper.parse::<u32>().unwrap_or(0);
            continue;
        }
        if upper == "X" || upper == "S" {
            cost.generic += 1;
            continue;
        }
        let colored: Vec<usize> = upper
            .chars()
            .filter(|c| *c != '/' && *c != 'P')
            .filter_map(|c| super::model::COLORS.iter().position(|w| *w == c))
            .collect();
        if colored.len() == 1 {
            cost.pips[colored[0]] += 1;
        } else if colored.len() > 1 {
            cost.flex_pips += 1;
        }
    }
    cost
}

/// The cheapest castable cost across a card's faces: split cards
/// ("Dusk // Dawn", "Bramble Familiar // Fetch Quest") are one card the
/// deck casts by choosing a face, so the model pays the cheaper face, not
/// the face sum. MDFC spell faces ("Bramble Familiar // Fetch Quest" again:
/// one face is a land) and modal faces follow the same rule. Empty faces
/// (MDFC land side) cost nothing as a land and never gate a cast.
pub fn parse_cost_faces(mana_cost: &str) -> Cost {
    let faces: Vec<&str> = mana_cost.split(" // ").collect();
    if faces.len() <= 1 {
        return parse_cost(mana_cost);
    }
    let nonempty: Vec<Cost> = faces
        .iter()
        .filter(|face| !face.trim().is_empty())
        .map(|face| parse_cost(face))
        .collect();
    if nonempty.len() <= 1 {
        return nonempty.first().cloned().unwrap_or_default();
    }
    nonempty
        .into_iter()
        .min_by_key(|cost| cost.total())
        .unwrap_or_default()
}

/// Parse an activation cost prefix ("{1}, {T}", "−3", "{0}").
pub fn parse_activation_cost(text: &str) -> Cost {
    // Planeswalker loyalty costs ("-3", "0", "−7") cost no mana.
    let cleaned = text.replace(['−', '–'], "-");
    if cleaned.trim_start().starts_with('-')
        || cleaned
            .trim()
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-')
    {
        return Cost::default();
    }
    parse_cost(&cleaned)
}
