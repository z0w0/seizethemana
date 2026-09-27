//! Parse Oracle text and lower supported data into simulator card fields.
//!
//! Unsupported text stays inert. The simulator documents current limits in
//! its output assumptions.

use super::model::{Ability, SimCard, Tier};
use crate::db::CardRow;

/// Lower a parsed Oracle card and its metadata into simulator card data.
#[path = "oracle_parse/card_lower.rs"]
mod card_lower;
/// Parse supported cast effects and costs from Oracle text.
#[path = "oracle_parse/cast_riders.rs"]
mod cast_riders;
/// Parse static keyword and interaction flags from Oracle data.
#[path = "oracle_parse/static_flags.rs"]
mod static_flags;

pub use super::parse_cost::{
    parse_cost as parse_oracle_cost, parse_cost_faces as parse_oracle_cost_faces,
};
use super::parse_cycle::{cycling_cost, cycling_life, landcycling_type};
pub use super::parse_land::parse_tap_generic as parse_oracle_tap;
use super::parse_land::{enters_tapped, parse_enter_counters};
pub use super::parse_land::{
    parse_gates as parse_oracle_gates, parse_tap_filtered as parse_oracle_tap_filtered,
    parse_tap_yield as parse_oracle_tap_yield,
};

/// Parse one Oracle activation into the simulator's ability model.
pub fn parse_oracle_ability(segment: &str) -> Option<Ability> {
    super::oracle_parser::parse_oracle_activated_ability(segment)?.to_runtime()
}

/// Parse station tiers and the station-card flag from Oracle text.
///
/// The reminder text lists striations as `N+ |` segments; each segment's
/// abilities belong to that tier. The animation threshold comes from the
/// "It's an artifact creature at N+" note in the station reminder. Planets
/// never animate because they have no power and toughness box.
pub fn parse_oracle_station_tiers(oracle_text: &str, type_line: &str) -> (Vec<Tier>, bool) {
    let is_station_card = type_line.contains("Spacecraft") || type_line.contains("Planet");
    let mut tiers: Vec<Tier> = Vec::new();
    for segment in oracle_text.split('\n') {
        let seg = segment.trim();
        let Some((head, rest)) = seg.split_once('|') else {
            continue;
        };
        let head_trim = head.trim().trim_end_matches('+').trim();
        let Ok(n) = head_trim.parse::<u32>() else {
            continue;
        };
        let mut tier = Tier {
            at: n,
            animate: false,
            abilities: Vec::new(),
        };
        for part in rest.split('\n') {
            if let Some(ability) = parse_oracle_ability(part.trim()) {
                tier.abilities.push(ability);
            }
        }
        tiers.push(tier);
    }
    if let Some(animate_at) = animate_threshold(oracle_text) {
        if let Some(tier) = tiers.iter_mut().find(|tier| tier.at == animate_at) {
            tier.animate = true;
        } else {
            tiers.push(Tier {
                at: animate_at,
                animate: true,
                abilities: Vec::new(),
            });
        }
    }
    tiers.sort_by_key(|tier| tier.at);
    (tiers, is_station_card)
}

/// Find the station animation threshold from its reminder text.
fn animate_threshold(oracle_text: &str) -> Option<u32> {
    let lower = oracle_text.to_ascii_lowercase();
    let index = lower.find("artifact creature at ")?;
    let tail = &lower[index + "artifact creature at ".len()..];
    let number: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    number.parse::<u32>().ok()
}

/// Match targeted damage spells that can remove a creature or permanent.
///
/// This excludes player-only burn, triggers, sweeps, and activated abilities.
pub fn parse_oracle_damage_removal_shape(oracle_text: &str) -> bool {
    oracle_text.split(['.', '\n', ',']).any(|segment| {
        let lower = segment.trim().to_ascii_lowercase();
        if lower.starts_with("whenever") || lower.starts_with("when ") || lower.contains(": ") {
            return false;
        }
        lower
            .split_once(" deals ")
            .and_then(|(_, tail)| {
                let (amount, after) = tail.split_once(" damage ")?;
                let n: u32 = amount
                    .trim()
                    .split(' ')
                    .next()
                    .and_then(|word| word.parse().ok())
                    .unwrap_or(0);
                if n < 2 {
                    return Some(false);
                }
                // Any-target and divided damage can hit a creature.
                let player_only = after.starts_with("to target player")
                    || after.starts_with("to target opponent")
                    || after.starts_with("to each opponent")
                    || after.starts_with("to each player")
                    || after.starts_with("to each creature")
                    || after.starts_with("to each permanent");
                let qualifies = after.contains("target") || after.starts_with("to any target");
                Some(qualifies && !player_only)
            })
            .unwrap_or(false)
    })
}

/// Parse a card row into Oracle syntax before lowering it to simulator data.
pub fn parse_oracle_card(row: &CardRow) -> super::oracle_ast::OracleCard {
    let keyword_names = parse_card_keywords(&row.keywords);
    super::oracle_parser::parse_oracle_text(&row.oracle_text, &keyword_names)
}

/// Read keyword arrays from storage and whitespace lists from test fixtures.
fn parse_card_keywords(source: &str) -> Vec<String> {
    let source = source.trim();
    if source.starts_with('[') {
        serde_json::from_str(source).expect("stored Scryfall keywords must be a JSON array")
    } else {
        source
            .split(|character: char| character == ',' || character.is_whitespace())
            .map(str::trim)
            .filter(|keyword| !keyword.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// Parse a card row and build the simulator model for that card.
pub fn parse_sim_card(row: &CardRow) -> SimCard {
    let oracle = parse_oracle_card(row);
    card_lower::lower_oracle_card(row, &oracle)
}

/// Match an effect that gives a permanent X counters as it enters.
fn enters_with_x_counters(text: &str) -> bool {
    (text.contains("enters with x") || text.contains("the battlefield with x"))
        && text.contains("counters")
}

/// Find a draw effect that scales with a permanent count.
fn scaling_draw_match(text: &str) -> Option<super::model::DrawMatch> {
    if !(text.contains("draw") && text.contains("for each")) {
        return None;
    }
    if text.contains("for each enchantment you control") {
        Some(super::model::DrawMatch::Enchantments)
    } else if text.contains("for each artifact you control") {
        Some(super::model::DrawMatch::Artifacts)
    } else if text.contains("for each land you control") {
        Some(super::model::DrawMatch::Lands)
    } else if text.contains("for each creature you control") {
        Some(super::model::DrawMatch::Creatures)
    } else {
        None
    }
}
