// Keyword-shaped helpers for the simulator's oracle text: static buffs,
// equipment stats, and the "+N" amount grammar. Split from parse.rs to
// keep files small.

use super::model::Equipment;

/// Static creature buff amount: "creatures you control get +2/+2".
/// Only full-board buffs count (the sim applies them deck-wide).
pub(super) fn parse_creature_buff(text: &str) -> Option<(i32, i32)> {
    let rel = text
        .find("creatures you control get +")
        .or_else(|| text.find("creatures you control have +"))?;
    let tail = &text[rel..];
    let plus_pos = tail.find('+')?;
    let (p, rest) = parse_plus_n(&tail[plus_pos..])?;
    let t = rest
        .find("+/")
        .and_then(|i| parse_plus_n(&rest[i + 1..]).map(|(t, _)| t))
        .unwrap_or(0);
    Some((p, t))
}

/// Parse a leading "+N" from a string, returning (n, remainder).
fn parse_plus_n(s: &str) -> Option<(i32, &str)> {
    let rest = &s[1..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: i32 = digits.parse().ok()?;
    let used = 1 + digits.len();
    Some((n, &s[used..]))
}

/// Equipment stats: (equip cost, equipped-creature buff, death draws).
pub(super) fn parse_equipment(text: &str) -> Option<Equipment> {
    // Buff: "Equipped creature gets +1/-1" / "gets +1/+2".
    let buff = {
        let marker = "equipped creature gets ";
        let i = text.find(marker)?;
        let tail = &text[i + marker.len()..];
        let (p, rest) = parse_plus_n(tail)?;
        let t = rest
            .find("+/")
            .and_then(|i2| parse_plus_n(&rest[i2 + 1..]).map(|(t, _)| t));
        (p, t.unwrap_or(0))
    };
    // Equip cost: "Equip {1}" / "Equip {2}".
    let cost = text
        .find("equip ")
        .and_then(|i| {
            text[i + 6..]
                .trim_start_matches(['{', '}', ' '])
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .ok()
        })
        .filter(|c| *c > 0)
        .unwrap_or(0);
    // Death draws: "Whenever equipped creature dies, draw N".
    let death_draws = if text.contains("equipped creature dies") && text.contains("draw") {
        super::model::draw_amount(text).max(1)
    } else {
        0
    };
    Some(Equipment {
        cost,
        buff,
        death_draws,
    })
}
