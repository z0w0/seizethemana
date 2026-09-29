//! Parse static buffs, equipment stats, and numeric keyword parameters.

use super::super::model::Equipment;

/// Static creature buff amount: "creatures you control get +2/+2".
/// Only full-board buffs count (the sim applies them deck-wide).
pub(super) fn parse_creature_buff(text: &str) -> Option<(i32, i32)> {
    let rel = text
        .find("creatures you control get +")
        .or_else(|| text.find("creatures you control have +"))?;
    let tail = &text[rel..];
    let plus_pos = tail.find('+')?;
    parse_power_toughness(&tail[plus_pos..])
}

/// Parse signed power and toughness adjustments from `+N/-N` text.
fn parse_power_toughness(text: &str) -> Option<(i32, i32)> {
    let (power, remainder) = parse_signed_amount(text)?;
    let toughness_text = remainder.strip_prefix('/')?;
    let (toughness, _) = parse_signed_amount(toughness_text)?;
    Some((power, toughness))
}

/// Parse a signed integer and return the unconsumed text.
fn parse_signed_amount(text: &str) -> Option<(i32, &str)> {
    let (sign, digits) = match text.chars().next()? {
        '+' => (1, &text[1..]),
        '-' | '−' => (-1, &text[1..]),
        _ => return None,
    };
    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
    let value = digits.parse::<i32>().ok()? * sign;
    Some((value, &text[1 + digits.len()..]))
}

/// Equipment stats: equip or reconfigure cost, buff, and the
/// reconfigure flag.
pub(in crate::deck::simulator) fn parse_equipment(text: &str) -> Option<Equipment> {
    // Buff: "Equipped creature gets +1/-1" / "gets +1/+2".
    let buff = {
        let marker = "equipped creature gets ";
        let i = text.find(marker)?;
        let tail = &text[i + marker.len()..];
        parse_power_toughness(tail)?
    };
    // Reconfigure (CR 702.151): "Reconfigure—Pay {2} or {E}{E}{E}."
    // The generic mana option is the modeled cost.
    let reconfigure = text.contains("reconfigure");
    let reconfigure_cost = text
        .split_once("reconfigure")
        .and_then(|(_, tail)| tail.split_once('{'))
        .and_then(|(_, tail)| tail.split('}').next())
        .and_then(|digits| digits.trim().parse::<u32>().ok())
        .filter(|cost| *cost > 0);
    // Equip cost: "Equip {1}" / "Equip {2}".
    let equip_cost = text
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
        .filter(|c| *c > 0);
    let cost = equip_cost.or(reconfigure_cost).unwrap_or(0);
    Some(Equipment {
        cost,
        buff,
        reconfigure,
    })
}

/// Parse the temporary power and toughness bonus that applies while saddled.
pub(super) fn parse_saddled_buff(text: &str) -> Option<(i32, i32)> {
    if !text.contains("saddled") {
        return None;
    }
    let sentence = text
        .split(['.', '\n'])
        .find(|line| line.contains("saddled") && line.contains("gets +"))?;
    let tail = sentence.split("gets +").nth(1)?;
    let power: String = tail
        .trim_start_matches('+')
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '-')
        .collect();
    let toughness = tail.split_once('/')?.1;
    let toughness: String = toughness
        .trim_start_matches('+')
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '-')
        .collect();
    Some((power.parse().ok()?, toughness.parse().ok()?))
}
