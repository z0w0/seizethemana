//! Land-shape and tap-yield parsing for the simulator: tap yields, verge
//! gates, enters-tapped, enter counters, and spend restrictions.

use super::super::model::{
    BasicLandType, EnterCounters, ManaColor, ManaYield, Scale, SpendRestriction,
};
use super::super::oracle_ast::{
    AbilityRestriction, ActivationCost, CostObject, OracleAbility, OracleEffect, OracleFetch,
    OracleLand,
};

/// Parse one "add" clause into a tap yield. Juxtaposed symbols with no
/// "or" produce fixed simultaneous pips (Jegantha's `{W}{U}{B}{R}{G}`);
/// "or" between symbols and "one mana of any color" prose are choices.
pub fn parse_tap_yield(text: &str) -> Option<ManaYield> {
    let lower = text.to_ascii_lowercase();
    // Opponent-dependent production keeps its flag for generic-only payment.
    let opponent = lower.contains("opponent");
    let mut yield_ = ManaYield {
        opponent_any: opponent,
        cannot_pay_generic: lower.contains("this mana can't be spent to pay generic mana costs"),
        ..ManaYield::default()
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
    // Kinnan-class triggers: "add one mana of any type that permanent
    // produced". One any-color pip, matched to the tapped source's output
    // by the runtime.
    if lower.contains("any type that permanent produced") {
        yield_.any_pips = 1;
        yield_.choice = [false; 5];
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
                    if let Some(idx) = ManaColor::from_symbol(ch).map(ManaColor::index) {
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
                if let Some(idx) = ManaColor::from_symbol(ch).map(ManaColor::index) {
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

/// Parse land-entry rules and fetch abilities from their owning Oracle statements.
pub(crate) fn parse_land_data(oracle_text: &str, abilities: &[OracleAbility]) -> OracleLand {
    let lower = oracle_text.to_ascii_lowercase();
    let fetch_ability = abilities.iter().find_map(|ability| match ability {
        OracleAbility::Activated(ability)
            if ability.costs.iter().any(|cost| {
                matches!(
                    cost,
                    ActivationCost::Sacrifice {
                        object: CostObject::Source,
                        ..
                    }
                )
            }) =>
        {
            ability.effects.iter().find_map(|effect| match effect {
                OracleEffect::Search(spec)
                    if matches!(
                        spec.card_type,
                        Some(
                            super::super::model::SearchCardType::Land
                                | super::super::model::SearchCardType::BasicLand
                        )
                    ) =>
                {
                    Some((ability, spec))
                }
                _ => None,
            })
        }
        _ => None,
    });
    let fetch = fetch_ability.map(|(ability, spec)| OracleFetch {
        target_types: BasicLandType::ALL
            .into_iter()
            .enumerate()
            .filter_map(|(index, kind)| spec.land_types[index].then_some(kind))
            .collect(),
        basic_only: spec.basic_land_only,
        life_cost: ability
            .costs
            .iter()
            .find_map(|cost| match cost {
                ActivationCost::PayLife(amount) => Some(*amount),
                _ => None,
            })
            .unwrap_or(0),
    });
    let gates = abilities
        .iter()
        .filter_map(|ability| match ability {
            OracleAbility::Activated(ability) => {
                ability.restrictions.iter().find_map(|restriction| {
                    let AbilityRestriction::Condition(condition) = restriction else {
                        return None;
                    };
                    Some(
                        BasicLandType::ALL
                            .into_iter()
                            .filter(|kind| condition.contains(&kind.name().to_ascii_lowercase()))
                            .collect::<Vec<_>>(),
                    )
                })
            }
            _ => None,
        })
        .flatten()
        .collect();
    let fetch_enters_tapped = fetch_ability.is_some_and(|(_, spec)| {
        spec.destination == super::super::model::SearchDestination::BattlefieldTapped
    });
    let entry_text = lower
        .split(['\n', '.'])
        .filter(|part| part.contains("enters"))
        .collect::<Vec<_>>()
        .join(".");
    OracleLand {
        enters_tapped: enters_tapped(&entry_text),
        life_to_untap: life_to_untap(&entry_text),
        fetch,
        gates,
        fetch_enters_tapped,
        produces_any_color: lower.contains("this land is the chosen type")
            || lower.contains("this land is every basic land type"),
    }
}

/// Spend restriction from a "spend this mana only to cast …" window:
/// creature, legendary, artifact, or instant-and-sorcery spells.
pub(super) fn spend_restriction(window: &str) -> Option<SpendRestriction> {
    if !window.contains("only to cast") {
        return None;
    }
    if window.contains("creature") {
        Some(SpendRestriction::Creature)
    } else if window.contains("legendary") {
        Some(SpendRestriction::Legendary)
    } else if window.contains("artifact") {
        Some(SpendRestriction::Artifact)
    } else if window.contains("instant and sorcery") || window.contains("instant or sorcery") {
        Some(SpendRestriction::InstantSorcery)
    } else {
        None
    }
}

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

/// Life a player may pay as a land enters to have it enter untapped.
pub fn life_to_untap(text: &str) -> u32 {
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
