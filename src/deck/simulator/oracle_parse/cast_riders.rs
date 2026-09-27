//! Parse one-shot effects and extra cast costs from Oracle data.

use super::super::model::{Cost, TapYield};
use crate::db::CardRow;

/// One-shot effects on cast, parsed from oracle text. Split cards zero
/// the riders (the cast pays the cheaper face).
pub(super) struct CastRiders {
    pub(super) mana_on_cast: Option<TapYield>,
    pub(super) draws_on_cast: u32,
    pub(super) life_gain_on_cast: u32,
    pub(super) alternative_cast_cost: Option<super::super::model::AlternativeCastCost>,
    pub(super) reveal_rule: Option<super::super::model::RevealRule>,
    pub(super) tokens_on_cast: u32,
    pub(super) scry_on_cast: u32,
    pub(super) surveils_on_cast: u32,
    pub(super) extra_turns_on_cast: bool,
    pub(super) drain_on_cast: u32,
    pub(super) mills_on_enter: u32,
    pub(super) wheel_on_cast: bool,
    pub(super) additional_cost_bodies: u32,
    pub(super) additional_cost_discards: u32,
    pub(super) additional_cost_life: u32,
    pub(super) counters_on_cast: u32,
    pub(super) x_class: Option<super::super::model::XClass>,
    pub(super) mana_per_cast: Option<TapYield>,
    pub(super) kicker: Option<Cost>,
    pub(super) search_after_sacrifice: bool,
    pub(super) graveyard_creature_exchange: bool,
    pub(super) grants_flashback: bool,
    pub(super) grants_escape: bool,
    pub(super) cycling_cost: Option<Cost>,
    pub(super) cycling_life: u32,
    pub(super) landcycling_type: Option<char>,
}

/// Parse an exile-from-hand alternate casting cost and its optional payoff.
fn alternative_cast_cost(text: &str) -> Option<super::super::model::AlternativeCastCost> {
    if !text.contains("rather than pay this spell's mana cost") || !text.contains("exile ") {
        return None;
    }
    let color = [
        ("white", 'W'),
        ("blue", 'U'),
        ("black", 'B'),
        ("red", 'R'),
        ("green", 'G'),
    ]
    .into_iter()
    .find_map(|(word, color)| text.contains(word).then_some(color));
    Some(super::super::model::AlternativeCastCost {
        filter: super::super::model::CardFilter { color },
        count: super::super::model::amount_after(text, "exile "),
        payoff: if text.contains("gain x life") {
            super::super::model::AlternativeCostPayoff::GainLifeEqualToExiledManaValue
        } else {
            super::super::model::AlternativeCostPayoff::None
        },
    })
}

/// Parse a repeated reveal effect that charges life equal to mana value.
fn reveal_rule(text: &str) -> Option<super::super::model::RevealRule> {
    (text.contains("reveal the top card of your library")
        && text.contains("put that card into your hand")
        && text.contains("lose life equal to its mana value")
        && text.contains("repeat this process"))
    .then_some(super::super::model::RevealRule {
        destination: super::super::model::RevealDestination::Hand,
        life_loss: super::super::model::RevealLifeLoss::ManaValue,
    })
}

/// Collect one-shot effects from a spell or its self-cast trigger.
fn cast_effects(
    oracle: &super::super::oracle_ast::OracleCard,
) -> Vec<&super::super::oracle_ast::OracleEffect> {
    let mut effects = Vec::new();
    for ability in &oracle.abilities {
        match ability {
            super::super::oracle_ast::OracleAbility::Spell(spell) => effects.extend(&spell.effects),
            super::super::oracle_ast::OracleAbility::Triggered(trigger)
                if matches!(
                    &trigger.event,
                    super::super::oracle_ast::TriggerEvent::CastsSpell { this_spell: true }
                ) =>
            {
                effects.extend(&trigger.effects);
            }
            _ => {}
        }
    }
    effects
}

/// Sum the draw count carried by parsed spell effects.
fn effect_draw_count(effect: &super::super::oracle_ast::OracleEffect) -> u32 {
    match effect {
        super::super::oracle_ast::OracleEffect::Draw(count)
        | super::super::oracle_ast::OracleEffect::Loot(count) => *count,
        super::super::oracle_ast::OracleEffect::DrawAndMinusCounter => 1,
        _ => 0,
    }
}

/// Lower supported spell effects into one-shot card fields.
pub(super) fn parse_cast_riders(
    row: &CardRow,
    text: &str,
    cast_face: bool,
    tap_is_none: bool,
    oracle: &super::super::oracle_ast::OracleCard,
) -> CastRiders {
    let effects = if cast_face {
        cast_effects(oracle)
    } else {
        Vec::new()
    };
    let mana_on_cast = if tap_is_none {
        effects.iter().find_map(|effect| match effect {
            super::super::oracle_ast::OracleEffect::Mana(yield_) => Some(yield_.clone()),
            _ => None,
        })
    } else {
        None
    };
    let draws_on_cast = effects.iter().map(|effect| effect_draw_count(effect)).sum();
    let life_gain_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::GainLife(amount) => total + amount,
        _ => total,
    });
    let tokens_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Tokens(amount) => total + amount,
        _ => total,
    });
    let scry_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Look {
            count,
            surveil: false,
        } => total + count,
        _ => total,
    });
    let surveils_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Look {
            count,
            surveil: true,
        } => total + count,
        _ => total,
    });
    let extra_turns_on_cast = effects
        .iter()
        .any(|effect| matches!(effect, super::super::oracle_ast::OracleEffect::ExtraTurn));
    let drain_on_cast = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Drain(amount) => total + amount,
        _ => total,
    });
    let mills_on_enter = effects.iter().fold(0, |total, effect| match effect {
        super::super::oracle_ast::OracleEffect::Mill(amount) => total + amount,
        _ => total,
    });
    let wheel_on_cast = effects
        .iter()
        .any(|effect| matches!(effect, super::super::oracle_ast::OracleEffect::Wheel));
    // X-cost effect class: "target player loses X life" (Drain), "draw X
    // cards" (Draw), "mill X" (Mill), "create X … creature tokens"
    // (Tokens). Oracle text writes the X bare ("loses X life"), so the
    // check is the mana cost carrying {X} plus the effect word.
    let has_x_cost = row.mana_cost.to_ascii_uppercase().contains("{X}");
    let x_class = if cast_face && has_x_cost {
        x_cost_class(text)
    } else {
        None
    };
    // Repeatable per-cast mana: "add {N} for each spell you've cast this
    // turn". Fires per spell cast while the host is untapped.
    let mana_per_cast = if cast_face
        && text.contains("add ")
        && (text.contains("for each spell you've cast")
            || text.contains("spell you've cast this turn"))
    {
        row.oracle_text
            .split(['.', '\n'])
            .map(str::trim)
            .find_map(|seg| {
                let lower = seg.to_ascii_lowercase();
                (lower.contains("spell you've cast") && lower.contains("add "))
                    .then(|| super::parse_oracle_tap_yield(seg))
                    .flatten()
            })
    } else {
        None
    };
    CastRiders {
        mana_on_cast,
        draws_on_cast,
        life_gain_on_cast,
        alternative_cast_cost: cast_face.then(|| alternative_cast_cost(text)).flatten(),
        reveal_rule: cast_face.then(|| reveal_rule(text)).flatten(),
        tokens_on_cast,
        scry_on_cast,
        surveils_on_cast,
        extra_turns_on_cast,
        drain_on_cast,
        mills_on_enter,
        wheel_on_cast,
        additional_cost_bodies: additional_cost_bodies_shape(text),
        additional_cost_discards: additional_cost_discards_shape(text),
        additional_cost_life: additional_cost_life_shape(text),
        counters_on_cast: super::super::parse_land::charge_counters_on_cast(text),
        x_class,
        mana_per_cast,
        kicker: kicker_cost(text),
        search_after_sacrifice: cast_face
            && text.contains("sacrificed creature's mana value")
            && text.contains("search your library for a creature card"),
        graveyard_creature_exchange: cast_face
            && text.contains("exiles all creature cards from their graveyard")
            && text.contains("sacrifices all creatures they control")
            && text.contains("puts all cards they exiled this way onto the battlefield"),
        grants_flashback: cast_face
            && (text.contains("instant and sorcery cards in your graveyard gain flashback")
                || text.contains("instant and sorcery card in your graveyard gains flashback")),
        grants_escape: cast_face
            && (text.contains("nonland cards in your graveyard have escape")
                || text.contains("each nonland card in your graveyard has escape"))
            && text.contains("exile three other cards from your graveyard"),
        cycling_cost: super::cycling_cost(text),
        cycling_life: super::cycling_life(text),
        landcycling_type: super::landcycling_type(text),
    }
}

/// The X-cost effect class for an {X} spell, or None when the effect
/// does not parse to a modeled X shape.
fn x_cost_class(text: &str) -> Option<super::super::model::XClass> {
    if (text.contains("loses x life")
        || text.contains("each opponent loses x")
        || text.contains("deals x damage"))
        && (text.contains("target player") || text.contains("opponent"))
    {
        Some(super::super::model::XClass::Drain)
    } else if text.contains("draw x") || text.contains("draws x") {
        Some(super::super::model::XClass::Draw)
    } else if text.contains("mill x") {
        Some(super::super::model::XClass::Mill)
    } else if text.contains("create x") && text.contains("token") {
        Some(super::super::model::XClass::Tokens)
    } else if text.contains("reveal the top x")
        && text.contains("permanent")
        && (text.contains("put any number") || text.contains("onto the battlefield"))
    {
        Some(super::super::model::XClass::RevealPermanents)
    } else if super::enters_with_x_counters(text) {
        Some(super::super::model::XClass::Counters)
    } else {
        None
    }
}

/// Match an additional cost that sacrifices a creature.
fn additional_cost_bodies_shape(text: &str) -> u32 {
    if text.contains("additional cost")
        && (text.contains("sacrifice a creature") || text.contains("sacrifice any number"))
    {
        1
    } else {
        0
    }
}

/// Count cards required by an additional-cost discard clause.
fn additional_cost_discards_shape(text: &str) -> u32 {
    let Some(discard) = text
        .split("additional cost")
        .nth(1)
        .and_then(|clause| clause.split("discard ").nth(1))
    else {
        return 0;
    };
    for (word, count) in [("two", 2), ("three", 3), ("four", 4), ("a card", 1)] {
        if discard.starts_with(word) {
            return count;
        }
    }
    discard
        .split_whitespace()
        .next()
        .and_then(|count| count.parse::<u32>().ok())
        .unwrap_or(0)
}

/// Parse the life payment from an additional-cost clause.
fn additional_cost_life_shape(text: &str) -> u32 {
    if !text.contains("additional cost") {
        return 0;
    }
    text.split("pay ")
        .nth(1)
        .and_then(|rest| rest.split([' ', '.', ',']).next().map(str::to_string))
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0)
}

/// Parse the full kicker or multikicker cost, including colored pips.
fn kicker_cost(text: &str) -> Option<Cost> {
    let rest = text.split("kicker ").nth(1)?;
    let head = rest.split(['(', '.', '\n', ',']).next()?.trim();
    let cost = super::parse_oracle_cost(head);
    (cost.total() > 0).then_some(cost)
}

/// Parse a cast-time reduction as a minimum cost and a board-scaling flag.
pub(super) fn parse_min_cost(text: &str, cost: &Cost) -> (Cost, bool) {
    let mut min_cost = cost.clone();
    let mut board_discount = false;
    if text.contains("warp ") {
        let warp = text
            .split("warp ")
            .nth(1)
            .and_then(|rest| rest.split(['(', '.', '\n']).next())
            .map(str::trim)
            .map(super::parse_oracle_cost);
        if let Some(warp) = warp
            && warp.total() < min_cost.total()
        {
            min_cost = warp;
        }
    }
    // Improvise and affinity scale with the artifact board: the parse-time
    // floor cuts 2 generic and the runtime adds one pip back per 4
    // artifacts (game_mana::effective_min_cost).
    if text.contains("improvise") || text.contains("affinity") {
        min_cost.generic = min_cost.generic.saturating_sub(2);
        board_discount = true;
    }
    (min_cost, board_discount)
}
