//! Land-shape and tap-yield parsing for the simulator: tap yields, verge
//! gates, enters-tapped, enter counters, and spend restrictions.

use super::super::model::{
    BasicLandType, EnterCounters, ManaColor, ManaYield, Scale, SearchCardType, SearchDestination,
    SearchSpec, SpendRestriction,
};
use super::super::oracle_ast::{
    AbilityRestriction, ActivationCost, ActivationTarget, CostObject, OracleAbility,
    OracleActivatedAbility, OracleEffect, OracleFetch, OracleLand, OracleLandEntryCondition,
};

/// Parse one "add" clause into a tap yield. Juxtaposed symbols with no
/// "or" produce fixed simultaneous pips (Jegantha's `{W}{U}{B}{R}{G}`);
/// "or" between symbols and "one mana of any color" prose are choices.
pub fn parse_tap_yield(text: &str) -> Option<ManaYield> {
    let lower = text.replace('’', "'").to_ascii_lowercase();
    // Opponent-dependent production keeps its flag for generic-only payment.
    let opponent = lower.contains("opponent");
    let mut yield_ = ManaYield {
        opponent_any: opponent,
        cannot_pay_generic: lower.contains("this mana can't be spent to pay generic mana costs"),
        restriction: spend_restriction(&lower),
        ..ManaYield::default()
    };
    if lower.contains("spend this mana only") && yield_.restriction.is_none() {
        return None;
    }
    let production = lower.split_once("add ")?.1.split('.').next()?;
    if production.contains(" among ") && production.contains("mana of any") {
        if production
            != "one mana of any color among legendary creatures and planeswalkers you control"
        {
            return None;
        }
        yield_.any_pips = 1;
        yield_.legendary_colors = true;
        return Some(yield_);
    }
    // Any-color amounts: "one mana of any color" (1), "N mana of any one
    // color" (N), "N mana in any combination of colors" (N).
    if lower.contains("any combination of colors") || lower.contains("mana of any one color") {
        let (amount, colors) = production.split_once(" mana ")?;
        if !matches!(colors, "of any one color" | "in any combination of colors") {
            return None;
        }
        yield_.any_pips = parse_mana_amount(amount)?;
        yield_.one_color = colors == "of any one color";
        return Some(yield_);
    }
    // Kinnan-class triggers: "add one mana of any type that permanent
    // produced". One any-color pip, matched to the tapped source's output
    // by the runtime.
    if lower.contains("any type that permanent produced") {
        yield_.any_pips = 1;
        yield_.choice = [false; 5];
        return Some(yield_);
    }
    if lower.contains("one mana of any color") {
        if !matches!(
            production,
            "one mana of any color"
                | "one mana of any color in your commander's color identity"
                | "one mana of any color that a land an opponent controls could produce"
        ) {
            return None;
        }
        if production == "one mana of any color in your commander's color identity" {
            yield_.commander_pips = 1;
        } else {
            yield_.any_pips = 1;
        }
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
        yield_.one_color = true;
        return Some(yield_);
    }
    if production.contains("for each") || production.contains("equal to") {
        return None;
    }
    parse_mana_symbols(production, &mut yield_)?;
    if yield_.total() == 0 {
        return None;
    }
    Some(yield_)
}

/// Keep single-symbol alternatives separate from fixed simultaneous mana bundles.
fn parse_mana_symbols(production: &str, yield_: &mut ManaYield) -> Option<()> {
    if production.contains(" or ") {
        for clause in production
            .split(" or ")
            .flat_map(|clause| clause.split(','))
            .map(str::trim)
            .filter(|clause| !clause.is_empty())
        {
            let symbol = clause.strip_prefix('{')?.strip_suffix('}')?;
            if symbol.len() != 1 {
                return None;
            }
            let color = symbol.chars().next()?.to_ascii_uppercase();
            if color == 'C' {
                yield_.choice_colorless = true;
            } else {
                yield_.choice[ManaColor::from_symbol(color)?.index()] = true;
            }
        }
    } else {
        let mut remaining = production.trim();
        while !remaining.is_empty() {
            let (symbol, rest) = remaining.strip_prefix('{')?.split_once('}')?;
            if symbol.len() != 1 {
                return None;
            }
            let color = symbol.chars().next()?.to_ascii_uppercase();
            if color == 'C' {
                yield_.colorless += 1;
            } else {
                yield_.fixed[ManaColor::from_symbol(color)?.index()] += 1;
            }
            remaining = rest.trim();
        }
    }
    Some(())
}

/// Regression cases for supported mana wording and nearby unsupported conditions.
#[cfg(test)]
#[path = "tests/land_tests.rs"]
mod land_tests;

/// Read a printed mana quantity without inventing a default amount.
fn parse_mana_amount(word: &str) -> Option<u32> {
    match word {
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        _ => word.parse().ok(),
    }
}

/// Parse land-entry rules and fetch abilities from their owning Oracle statements.
pub(crate) fn parse_land_data(oracle_text: &str, abilities: &[OracleAbility]) -> OracleLand {
    let lower = oracle_text.to_ascii_lowercase();
    let fetch_data = abilities.iter().find_map(|ability| match ability {
        OracleAbility::Activated(ability) => native_fetch_data(ability),
        _ => None,
    });
    let gates = abilities
        .iter()
        .filter_map(|ability| match ability {
            OracleAbility::Activated(ability) => {
                ability.restrictions.iter().find_map(|restriction| {
                    let types = match restriction {
                        AbilityRestriction::ControlsLandTypes(types) => *types,
                        AbilityRestriction::Condition(condition) => {
                            parse_controlled_land_types(condition)?
                        }
                        _ => return None,
                    };
                    Some(
                        BasicLandType::ALL
                            .into_iter()
                            .filter(|kind| types[kind.index()])
                            .collect::<Vec<_>>(),
                    )
                })
            }
            _ => None,
        })
        .flatten()
        .collect();
    let fetch_enters_tapped = fetch_data.as_ref().is_some_and(|(_, tapped)| *tapped);
    let entry_text = lower
        .split(['\n', '.'])
        .filter(|part| part.contains("enters"))
        .collect::<Vec<_>>()
        .join(".");
    OracleLand {
        behold_untapped_entry: super::statements::oracle_segments(oracle_text, &[])
            .iter()
            .find_map(|source| super::behold::land_entry_condition(source)),
        enters_tapped: enters_tapped(&entry_text),
        untapped_entry_condition: untapped_entry_condition(&entry_text),
        life_to_untap: life_to_untap(&entry_text),
        fetch: fetch_data.map(|(fetch, _)| fetch),
        gates,
        fetch_enters_tapped,
        produces_any_color: lower.contains("this land is the chosen type")
            || lower.contains("this land is every basic land type"),
    }
}

/// Keep fetch metadata only when it represents the complete activation.
fn native_fetch_data(ability: &OracleActivatedAbility) -> Option<(OracleFetch, bool)> {
    if !ability.restrictions.is_empty() || ability.is_mana_ability || ability.mana_per_spell_cast {
        return None;
    }
    let (spec, behold_untap) = match ability.effects.as_slice() {
        [OracleEffect::Search(spec)] => (spec, None),
        [OracleEffect::SearchThenBeholdUntap { search, spec }] if spec.count == 1 => {
            (search, Some(spec.clone()))
        }
        _ => return None,
    };
    if !native_fetch_search(spec) {
        return None;
    }
    let life_cost = native_fetch_life_cost(&ability.costs)?;
    Some((
        OracleFetch {
            behold_untap,
            target_types: BasicLandType::ALL
                .into_iter()
                .enumerate()
                .filter_map(|(index, kind)| spec.land_types[index].then_some(kind))
                .collect(),
            basic_only: spec.basic_land_only || spec.card_type == Some(SearchCardType::BasicLand),
            life_cost,
        },
        spec.destination == SearchDestination::BattlefieldTapped,
    ))
}

/// Native fetch costs require one source tap, one source sacrifice, and at most one life payment.
fn native_fetch_life_cost(costs: &[ActivationCost]) -> Option<u32> {
    let mut taps_source = false;
    let mut sacrifices_source = false;
    let mut life_cost = None;
    for cost in costs {
        match cost {
            ActivationCost::Tap(ActivationTarget::Source) if !taps_source => taps_source = true,
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Source,
                exclude_source: false,
            } if !sacrifices_source => sacrifices_source = true,
            ActivationCost::PayLife(amount) if life_cost.is_none() => life_cost = Some(*amount),
            _ => return None,
        }
    }
    (taps_source && sacrifices_source).then_some(life_cost.unwrap_or(0))
}

/// Native metadata preserves only full-library land searches with a printed shuffle.
fn native_fetch_search(spec: &SearchSpec) -> bool {
    matches!(
        spec,
        SearchSpec {
            card_type: Some(SearchCardType::Land | SearchCardType::BasicLand),
            color: None,
            colorless: false,
            mana_value: None,
            max_mana_value: None,
            min_mana_value: None,
            destination: SearchDestination::Battlefield | SearchDestination::BattlefieldTapped,
            optional: false,
            top_count: None,
            shuffle: true,
            non_human: false,
            land_types: _,
            basic_land_only: _,
        }
    )
}

/// Spend restriction from a "spend this mana only to cast …" window:
/// creature, legendary, artifact, or instant-and-sorcery spells.
pub(in crate::deck::simulator) fn spend_restriction(window: &str) -> Option<SpendRestriction> {
    let tail = window.split_once("spend this mana only to cast ")?.1;
    let class = tail.split(['.', '\n']).next()?.trim();
    match class {
        "a creature spell" | "creature spells" => Some(SpendRestriction::Creature),
        "a legendary spell" | "legendary spells" => Some(SpendRestriction::Legendary),
        "an artifact spell" | "artifact spells" => Some(SpendRestriction::Artifact),
        "an instant or sorcery spell"
        | "instant or sorcery spells"
        | "instant and sorcery spells" => Some(SpendRestriction::InstantSorcery),
        _ => None,
    }
}

/// Keep conditional entry tapped until its supported state or life payment is checked.
pub fn enters_tapped(text: &str) -> bool {
    life_to_untap(text) == 0
        && (text.contains("enters tapped") || text.contains("enters the battlefield tapped"))
}

/// Parse one complete supported condition without accepting extra required clauses.
fn untapped_entry_condition(text: &str) -> Option<OracleLandEntryCondition> {
    let mut clauses = text
        .split(['.', '\n'])
        .map(str::trim)
        .filter(|clause| clause.contains("enters"));
    let clause = clauses.next()?.trim_start_matches("//").trim();
    if clauses.next().is_some() {
        return None;
    }
    let condition = clause.strip_prefix("this land enters tapped unless ")?;
    if let Some(types) = parse_controlled_land_types(condition) {
        return Some(OracleLandEntryCondition::ControlsLandTypes(types));
    }
    if let Some(count) = condition.strip_prefix("you control ") {
        if let Some(count) = count.strip_suffix(" or fewer other lands") {
            return Some(OracleLandEntryCondition::OtherLandsAtMost(entry_count(
                count,
            )?));
        }
        if let Some(count) = count.strip_suffix(" or more other lands") {
            return Some(OracleLandEntryCondition::OtherLandsAtLeast(entry_count(
                count,
            )?));
        }
    }
    let count = condition
        .strip_prefix("you have ")?
        .strip_suffix(" or more opponents")?;
    Some(OracleLandEntryCondition::OpponentsAtLeast(entry_count(
        count,
    )?))
}

/// Parse a complete one-type or two-type controlled-land condition.
pub(super) fn parse_controlled_land_types(text: &str) -> Option<[bool; 5]> {
    let lower = text.to_ascii_lowercase();
    let selector = lower.strip_prefix("you control ")?;
    let mut types = [false; 5];
    let mut selectors = selector.split(" or ");
    for selector in selectors.by_ref().take(2) {
        let kind = BasicLandType::ALL.into_iter().find(|kind| {
            let name = kind.name().to_ascii_lowercase();
            let article = if *kind == BasicLandType::Island {
                "an"
            } else {
                "a"
            };
            selector == format!("{article} {name}")
        })?;
        types[kind.index()] = true;
    }
    if selectors.next().is_some() {
        return None;
    }
    Some(types)
}

/// Preserve an exact number word or numeral instead of guessing a threshold.
fn entry_count(text: &str) -> Option<u32> {
    match text {
        "zero" => Some(0),
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        _ if text.bytes().all(|byte| byte.is_ascii_digit()) => text.parse().ok(),
        _ => None,
    }
}

/// Life a player may pay as a land enters to have it enter untapped.
pub fn life_to_untap(text: &str) -> u32 {
    if let Some((_, condition)) = text.split_once("tapped unless ")
        && !matches!(
            condition.split(['.', '\n']).next().map(str::trim),
            Some("you pay 2 life" | "you pay 3 life")
        )
    {
        return 0;
    }
    for amount in [2, 3] {
        let clause = format!("pay {amount} life");
        if text.contains(&clause)
            && (text.contains("you may pay") || text.contains("unless you pay"))
        {
            return amount;
        }
    }
    0
}

/// Counters a card enters with, parsed from Oracle text.
///
/// Charge counters, +1/+1 counters, and the "X" forms are distinguished
/// because each feeds a different simulator subsystem. The search stays
/// inside the same sentence so a later "{2}, {T}" activation does not
/// leak a number.
pub fn parse_enter_counters(text: &str) -> EnterCounters {
    // "Enters with X charge counters": the cast leftover converts to
    // counters. "+1/+1 counters" follows the same rule. Checked before
    // Sunburst, which the reminder text of the same card carries.
    if text.contains("enters with x charge counters") {
        return EnterCounters::XCharge;
    }
    if text.contains("enters with x +1/+1 counters")
        || text.contains("the battlefield with x +1/+1 counters")
    {
        return EnterCounters::XPlus1;
    }
    // Sunburst (best case): two colors paid on-curve → 2 counters.
    if text.contains("sunburst") {
        return EnterCounters::Charge(2);
    }
    let plus1 = text.contains("+1/+1 counter");
    let Some(idx) = text
        .find("enters with")
        .into_iter()
        .chain(text.find("battlefield with"))
        .min()
    else {
        return EnterCounters::None;
    };
    let tail = &text[idx..].split(['.', '\n', ',']).next().unwrap_or("");
    let amount = parse_counter_amount(tail);
    if amount == 0 {
        return EnterCounters::None;
    }
    if plus1 {
        EnterCounters::Plus1(amount)
    } else {
        EnterCounters::Charge(amount)
    }
}

/// The count in an enter-counters clause, from digits or a number word.
///
/// The number sits before the counter noun ("enters with two +1/+1
/// counters"), so the search stops at "counter" and reads only the words
/// before it; this keeps "+1/+1" from being read as the count.
fn parse_counter_amount(tail: &str) -> u32 {
    let before_counter = tail.split("counter").next().unwrap_or(tail);
    for word in before_counter.split_whitespace().rev() {
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if let Ok(value) = cleaned.parse::<u32>() {
            return value;
        }
        let value = match cleaned {
            "one" | "a" | "an" => 1,
            "two" => 2,
            "three" => 3,
            "four" => 4,
            "five" => 5,
            "six" => 6,
            _ => 0,
        };
        if value > 0 {
            return value;
        }
    }
    0
}
