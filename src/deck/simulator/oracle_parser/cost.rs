//! Convert Scryfall mana-cost strings into simulator Cost values.

use super::super::model::{Cost, ManaColor};

/// Parse a Scryfall mana-cost string (`{2}{W}{W}`, `{W/U}`, `{1}{B/P}`,
/// multi-face `{2}{B} // {B}`) into a cost. Multi-face costs union the
/// faces. Hybrid symbols retain their permitted colors.
pub fn parse_cost(mana_cost: &str) -> Cost {
    let mut cost = Cost::default();
    for part in mana_cost.split('{').skip(1) {
        let Some((symbol, _)) = part.split_once('}') else {
            cost.unsupported = true;
            continue;
        };
        let upper = symbol.trim().to_ascii_uppercase();
        if upper.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(amount) = upper.parse::<u32>()
                && let Some(total) = cost.generic.checked_add(amount)
            {
                cost.generic = total;
            } else {
                cost.unsupported = true;
            }
            continue;
        }
        if upper == "X" {
            if let Some(count) = cost.x_symbols.checked_add(1) {
                cost.x_symbols = count;
            } else {
                cost.unsupported = true;
            }
            continue;
        }
        if upper == "S" {
            cost.snow += 1;
            continue;
        }
        // Colorless pip (`{C}`): paid with colorless mana only (CR 107.4a).
        if upper == "C" {
            cost.colorless += 1;
            continue;
        }
        parse_colored_symbol(&upper, &mut cost);
    }
    cost
}

/// Preserve the legal choices of a colored mana symbol.
fn parse_colored_symbol(symbol: &str, cost: &mut Cost) {
    let color_index = |letter: u8| ManaColor::from_symbol(char::from(letter)).map(ManaColor::index);
    let count = match symbol.as_bytes() {
        [color] => color_index(*color).map(|i| &mut cost.pips[i]),
        [color, b'/', b'P'] => color_index(*color).map(|i| &mut cost.phyrexian[i]),
        [b'2', b'/', color] => color_index(*color).map(|i| &mut cost.twobrid[i]),
        [first, b'/', second] => color_index(*first)
            .zip(color_index(*second))
            .map(|(first, second)| &mut cost.hybrid[(1 << first) | (1 << second)]),
        _ => None,
    };
    if let Some(count) = count
        && let Some(next) = count.checked_add(1)
    {
        *count = next;
    } else {
        cost.unsupported = true;
    }
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
    let symbols: String = cleaned
        .split('{')
        .skip(1)
        .filter_map(|part| {
            part.split_once('}').and_then(|(symbol, _)| {
                (!matches!(symbol.to_ascii_uppercase().as_str(), "T" | "Q"))
                    .then(|| format!("{{{symbol}}}"))
            })
        })
        .collect();
    parse_cost(&symbols)
}
