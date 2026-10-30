//! Parsing for supported spell, trigger, activation, and Saga effects.

use super::super::model::XClass;
use super::super::oracle_ast::OracleEffect;

/// Search selectors, destinations, and exact mana-value relations.
mod search;
use search::parse_oracle_search_spec;

/// Parse supported effects from Oracle text and preserve unknown clauses.
pub(super) fn parse_oracle_effects(source: &str) -> Vec<OracleEffect> {
    parse_spell_effects(source, None)
}

/// Retain X markers only when the owning spell has a supported X payment class.
pub(super) fn parse_spell_effects(source: &str, x_class: Option<XClass>) -> Vec<OracleEffect> {
    parse_effects(source, x_class, false, None)
}

/// Tie variable source charge counters to the activation's chosen-X payment.
pub(super) fn parse_activation_effects(source: &str, has_x: bool) -> Vec<OracleEffect> {
    parse_effects(source, None, has_x, None)
}

/// Keep event-linked quantities tied to the trigger that supplies their value.
pub(super) fn parse_trigger_effects(
    source: &str,
    event: &super::super::oracle_ast::OracleTriggerEvent,
) -> Vec<OracleEffect> {
    parse_effects(source, None, false, Some(event))
}

/// Keep effect order and the owning spell or activation's variable amounts.
fn parse_effects(
    source: &str,
    x_class: Option<XClass>,
    activation_x: bool,
    event: Option<&super::super::oracle_ast::OracleTriggerEvent>,
) -> Vec<OracleEffect> {
    // Reminder text (parenthesized) clarifies a keyword; it is not rules
    // text for the resolution, so drop it before effect parsing. This
    // keeps "it explores. (Reveal the top card …)" from reading as a
    // search effect.
    let source = &strip_reminder_text(source);
    let lower = source.to_ascii_lowercase();
    if let Some(effect) = super::behold::parse_effect(source) {
        return vec![effect];
    }
    if let Some(effects) = super::selection::parse_top_selection(source)
        .or_else(|| super::selection::parse_draw_life(source))
    {
        return effects;
    }
    if lower.contains("at the beginning of the next end step") {
        return vec![OracleEffect::Unsupported(source.to_string())];
    }
    if lower.contains("secretly chooses") {
        return vec![OracleEffect::Unsupported(source.to_string())];
    }
    if has_unmodeled_resolution_branch(&lower)
        || lower.contains(" unless ")
        || lower.contains("you may pay ")
    {
        return vec![OracleEffect::Unsupported(source.to_string())];
    }
    if is_search_instruction(&lower) {
        return parse_search_effects(source, x_class);
    }
    if let Some(effect) = parse_compound_effect(source, &lower) {
        return effect;
    }
    split_effect_clauses(source)
        .iter()
        .map(|clause| {
            parse_x_effect(clause, x_class)
                .or_else(|| source_charge_counters(&clause.to_ascii_lowercase(), activation_x))
                .or_else(|| super::tribal_effects::parse_effect(clause, event))
                .unwrap_or_else(|| parse_oracle_effect_clause(clause))
        })
        .collect()
}

/// Reject resolution branches before splitting can turn them into separate rewards.
fn has_unmodeled_resolution_branch(lower: &str) -> bool {
    // Typed trigger conditions and payments are removed before effect parsing.
    // Activation restrictions remain as annotations with their own typed gates.
    lower
        .split(['.', ';'])
        .filter(|clause| !clause.trim_start().starts_with("activate only "))
        .flat_map(|clause| clause.split(|character: char| !character.is_ascii_alphabetic()))
        .any(|word| matches!(word, "if" | "instead"))
}

/// Zero is the runtime X marker only for the owning spell's recognized effect.
fn parse_x_effect(source: &str, x_class: Option<XClass>) -> Option<OracleEffect> {
    let lower = source.to_ascii_lowercase();
    let body = lower.trim();
    match x_class? {
        XClass::Draw
            if matches!(
                body,
                "draw x cards" | "you draw x cards" | "you may draw x cards"
            ) =>
        {
            Some(OracleEffect::Draw(0))
        }
        XClass::Mill if body == "mill x cards" => Some(OracleEffect::Mill(0)),
        XClass::Drain
            if matches!(
                body,
                "target player loses x life"
                    | "target opponent loses x life"
                    | "each opponent loses x life"
            ) =>
        {
            Some(OracleEffect::LoseLife {
                amount: 0,
                scope: life_loss_scope(body),
            })
        }
        XClass::Drain
            if body
                .split_once(" deals x damage ")
                .is_some_and(|(_, target)| {
                    matches!(
                        target,
                        "to target player" | "to target opponent" | "to each opponent"
                    )
                }) =>
        {
            Some(OracleEffect::Damage {
                amount: 0,
                target: super::super::oracle_ast::DamageTarget::Player,
                scope: life_loss_scope(body),
            })
        }
        class @ (XClass::Tokens | XClass::Treasures)
            if body.starts_with("create x ")
                && body.contains("token")
                && (class == XClass::Treasures) == body.contains("treasure token")
                && !["for each", "equal to", "that many", " and "]
                    .iter()
                    .any(|text| body.contains(text)) =>
        {
            Some(if body.contains("treasure token") {
                OracleEffect::CreateTreasureTokens(0)
            } else {
                OracleEffect::CreateTokens(0)
            })
        }
        _ => None,
    }
}

/// Remove parenthesized reminder text from an effect sentence.
fn strip_reminder_text(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut depth = 0u32;
    for ch in source.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Parse compound effect forms that take precedence over clause splitting.
fn parse_compound_effect(source: &str, lower: &str) -> Option<Vec<OracleEffect>> {
    // The One Ring class: the add-counter-and-draw-per-counter sentence
    // is one resolution, so it must beat the clause splitter.
    if lower.contains("put a burden counter")
        && lower.contains("draw a card for each burden counter")
    {
        return Some(vec![OracleEffect::AddBurdenCounter]);
    }
    if lower.contains("lose") && lower.contains("life for each burden counter") {
        return Some(vec![OracleEffect::BurdenLifeLoss]);
    }
    if lower.contains("remove all charge counters")
        && lower.contains("for each charge counter removed")
        && lower.contains("add ")
    {
        return Some(vec![
            banked_mana_yield(lower)
                .map(OracleEffect::ManaPerCounter)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        ]);
    }
    if lower.contains("exile") && lower.contains("return it to the battlefield") {
        return Some(vec![OracleEffect::ExileThenReturn]);
    }
    if lower.contains("each player") && lower.contains("discards") && lower.contains("draws") {
        return Some(vec![OracleEffect::DiscardHandThenDraw]);
    }
    if lower.contains("as an additional cost to cast this spell") {
        return None;
    }
    if lower.contains("draw") && lower.contains("discard") {
        return Some(vec![draw_discard_effect(source, lower)]);
    }
    if lower.contains("draw") && lower.contains("-1/-1 counter") {
        let body = lower.trim_end_matches('.');
        return Some(vec![if matches!(
            body,
            "draw a card and put a -1/-1 counter on this creature"
                | "put a -1/-1 counter on this creature, then draw a card"
                | "put a -1/-1 counter on up to one target creature and draw a card"
                | "draw a card, then put a -1/-1 counter on up to one target creature"
        ) {
            OracleEffect::DrawAndMinusCounter
        } else {
            OracleEffect::Unsupported(source.to_string())
        }]);
    }
    None
}

/// Recognize a search head without consuming later payment or reward clauses.
fn is_search_instruction(lower: &str) -> bool {
    lower.starts_with("search your library")
        || lower.starts_with("you may search your library")
        || lower.starts_with("look at the top") && lower.contains("from among them")
        || lower.starts_with("reveal the top")
            && (lower.contains("from among them") || lower.contains("into your hand"))
}

/// Keep search destinations together while retaining every following effect.
fn parse_search_effects(source: &str, x_class: Option<XClass>) -> Vec<OracleEffect> {
    let lower = source.to_ascii_lowercase();
    let mut end = source.find('.').map_or(source.len(), |index| index + 1);
    while end < source.len() {
        let rest = lower[end..].trim_start();
        if !is_search_continuation(rest) {
            break;
        }
        let start = source.len() - rest.len();
        end = source[start..]
            .find('.')
            .map_or(source.len(), |index| start + index + 1);
    }
    if let Some(plan) = parse_multi_search(&lower[..end]) {
        let mut effects = vec![OracleEffect::MultiSearch(plan)];
        let remainder = source[end..].trim();
        if !remainder.is_empty() {
            effects.extend(parse_spell_effects(remainder, x_class));
        }
        return effects;
    }
    let consequence = [", then ", " and then ", ", ", " and "]
        .into_iter()
        .flat_map(|connector| {
            lower[..end]
                .match_indices(connector)
                .map(move |(index, _)| (index, connector.len()))
        })
        .filter(|(index, length)| {
            let tail = lower[index + length..end].trim_start();
            is_effect_clause(tail) && !is_search_continuation(tail)
        })
        .min_by_key(|(index, _)| *index);
    let (instruction_end, remainder_start) =
        consequence.map_or((end, end), |(index, length)| (index, index + length));
    let instruction = &lower[..instruction_end];
    let search = if instruction.contains("sacrificed creature") {
        parse_sacrifice_search(instruction)
            .map(OracleEffect::SacrificeSearch)
            .unwrap_or_else(|| OracleEffect::Unsupported(source[..instruction_end].to_string()))
    } else {
        parse_oracle_search_spec(instruction)
            .map(OracleEffect::Search)
            .unwrap_or_else(|| OracleEffect::Unsupported(source[..instruction_end].to_string()))
    };
    let mut effects = vec![search];
    let remainder = source[remainder_start..].trim();
    if !remainder.is_empty() {
        effects.extend(parse_spell_effects(remainder, x_class));
    }
    effects
}

/// Parse every result and destination before accepting a multiple-result search.
fn parse_multi_search(
    instruction: &str,
) -> Option<super::super::oracle_ast::search::OracleMultiSearch> {
    super::multi_search::parse_multi_search(instruction)
}

/// Keep the exact mana-value relation and entry counter on linked sacrifice searches.
pub(super) fn parse_sacrifice_search(source: &str) -> Option<super::super::model::SacrificeSearch> {
    let lower = source.to_ascii_lowercase().replace('’', "'");
    let (_, search) =
        lower.split_once("search your library for a creature card with mana value ")?;
    let search = search.split('.').next()?.trim();
    let (relation, destination) = search.split_once(", put ")?;
    let (amount, allow_lower_mana_value) = if let Some(amount) = relation.strip_prefix("equal to ")
    {
        (amount, false)
    } else {
        (relation.strip_prefix("x or less, where x is ")?, true)
    };
    let amount = amount.strip_suffix(" plus the sacrificed creature's mana value")?;
    if amount.split_whitespace().count() != 1 {
        return None;
    }
    let mana_value_offset = parse_oracle_number_word(amount)?;
    let entry = destination
        .strip_prefix("that card ")
        .or_else(|| destination.strip_prefix("it "))?
        .strip_suffix(", then shuffle")?;
    let enters_with_plus1_counter = match entry {
        "onto the battlefield" => false,
        "onto the battlefield with an additional +1/+1 counter on it" => true,
        _ => return None,
    };
    Some(super::super::model::SacrificeSearch {
        shuffle: lower.contains("shuffle"),
        mana_value_offset,
        allow_lower_mana_value,
        enters_with_plus1_counter,
    })
}

/// Search destination, reveal, and shuffle sentences belong to the search.
fn is_search_continuation(lower: &str) -> bool {
    if lower.starts_with("you may put ") {
        return lower.contains("from among");
    }
    [
        "put it ",
        "put that card ",
        "put those cards ",
        "put the rest ",
        "reveal it",
        "reveal that card",
        "reveal those cards",
        "exile it",
        "exile that card",
        "then shuffle",
        "shuffle",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

/// Parse source-counter removal separately from its per-counter mana amount.
fn banked_mana_yield(lower: &str) -> Option<super::super::model::ManaYield> {
    let (removal, mana) = lower.split_once(". ")?;
    if !matches!(
        removal,
        "remove all charge counters from this artifact"
            | "remove all charge counters from this permanent"
    ) {
        return None;
    }
    let (production, remainder) = mana.split_once(" for each charge counter removed")?;
    if !matches!(remainder.trim_end_matches('.'), "" | " this way") {
        return None;
    }
    super::land::parse_tap_yield(production)
}

/// Keep loot counts and any extra linked instructions from disappearing.
fn draw_discard_effect(source: &str, lower: &str) -> OracleEffect {
    let body = lower.trim_end_matches('.');
    let pair = [", then ", " and then ", " and ", ". "]
        .into_iter()
        .find_map(|connector| body.split_once(connector));
    if let Some((draw, discard)) = pair
        && let Some(draw_count) = explicit_draw_amount(draw)
        && let Some(discard) = discard.strip_prefix("discard ")
        && let Some((amount, cards)) = discard.split_once(' ')
        && matches!(cards, "card" | "cards")
        && let Some(discard_count) = parse_oracle_number_word(amount)
    {
        return if draw_count == discard_count {
            OracleEffect::DrawThenDiscard(draw_count)
        } else {
            OracleEffect::DrawDiscard {
                draw: draw_count,
                discard: discard_count,
            }
        };
    }
    OracleEffect::Unsupported(source.to_string())
}

/// Split a statement only at connectors between two effect clauses.
fn split_effect_clauses(source: &str) -> Vec<&str> {
    let mut clauses = Vec::new();
    for sentence in source
        .split(['.', ';'])
        .map(str::trim)
        .filter(|sentence| !sentence.is_empty())
    {
        let mut remainder = sentence;
        loop {
            let connector = [", then ", " and then ", " then ", " and "]
                .into_iter()
                .find_map(|connector| {
                    remainder.find(connector).and_then(|index| {
                        let left = remainder[..index].trim();
                        let right = remainder[index + connector.len()..].trim();
                        (is_effect_clause(left) && is_effect_clause(right))
                            .then_some((index, connector))
                    })
                });
            let Some((index, connector)) = connector else {
                clauses.push(remainder);
                break;
            };
            clauses.push(remainder[..index].trim());
            remainder = remainder[index + connector.len()..].trim();
        }
    }
    if clauses.is_empty() {
        clauses.push(source);
    }
    clauses
}

/// Return true when a clause begins with a supported effect action.
fn is_effect_clause(source: &str) -> bool {
    let lower = source.trim().to_ascii_lowercase();
    [
        "draw ",
        "scry ",
        "surveil ",
        "mill ",
        "create ",
        "add ",
        "search ",
        "return ",
        "you may return ",
        "gain ",
        "you gain ",
        "you draw ",
        "you may draw ",
        "lose ",
        "you lose ",
        "each opponent loses ",
        "each player loses ",
        "target player loses ",
        "put ",
        "untap ",
        "exile ",
        "take an extra turn",
        "become the monarch",
        "deal ",
        "destroy ",
        "counter ",
        "sacrifice ",
        "discard ",
        "venture ",
        "pay ",
        "amass ",
        "explore",
        "connive",
        "the ring tempts you",
        "empower jace",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

/// Parse one effect clause, retaining unsupported wording verbatim.
fn parse_oracle_effect_clause(source: &str) -> OracleEffect {
    let lower = source.trim().to_ascii_lowercase();
    if is_search_instruction(&lower) {
        return parse_oracle_search_spec(&lower)
            .map(OracleEffect::Search)
            .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string()));
    }
    parse_permanent_effect(source, &lower)
        .or_else(|| parse_card_effect(source, &lower))
        .or_else(|| parse_life_and_counter_effect(source, &lower))
        .unwrap_or_else(|| {
            if lower.contains("deals")
                && (lower.contains("damage to") || lower.contains("damage divided"))
            {
                parse_damage_effect(source, &lower)
            } else {
                OracleEffect::Unsupported(source.to_string())
            }
        })
}

/// Parse effects that change permanents or create tokens.
fn parse_permanent_effect(source: &str, lower: &str) -> Option<OracleEffect> {
    if let Some(energy) = energy_gain(lower) {
        Some(OracleEffect::Energy(energy))
    } else if lower.contains("proliferate") {
        Some(OracleEffect::Proliferate)
    } else if let Some(amount) = amass_keyword(lower) {
        Some(OracleEffect::Amass(amount))
    } else if lower.starts_with("amass ") {
        // A bare "amass Orcs N" clause with no parseable number still
        // raises the Army by one (the per-resolution floor).
        Some(OracleEffect::Amass(amass_amount(lower).unwrap_or(1)))
    } else if lower.contains("the ring tempts you") {
        Some(OracleEffect::RingTempts)
    } else if lower.contains("empower jace") {
        Some(OracleEffect::EmpowerJace(
            keyword_number(lower, "empower jace").unwrap_or(1),
        ))
    } else if lower.starts_with("explore")
        || lower.contains(" explores")
        || lower.contains("explores.")
    {
        Some(OracleEffect::Explore)
    } else if lower.contains("connive") {
        Some(OracleEffect::Connive(
            keyword_number(lower, "connive").unwrap_or(1),
        ))
    } else if lower.contains("put a burden counter")
        && lower.contains("draw a card for each burden counter")
    {
        // The One Ring class (CR 122 counters): the activation adds one
        // burden counter, then draws for the new total.
        Some(OracleEffect::AddBurdenCounter)
    } else if lower.contains("lose 1 life for each burden counter")
        || (lower.contains("lose") && lower.contains("life for each burden counter"))
    {
        Some(OracleEffect::BurdenLifeLoss)
    } else if lower.contains("remove all charge counters") && lower.contains("add ") {
        Some(
            super::land::parse_tap_yield(&lower.replace('"', ""))
                .map(OracleEffect::ManaPerCounter)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else if lower.contains("additional land") {
        Some(OracleEffect::AdditionalLandPlay)
    } else if lower.contains("exile") && lower.contains("return it to the battlefield") {
        Some(OracleEffect::ExileThenReturn)
    } else if lower.contains("you become the monarch") {
        Some(OracleEffect::Monarch)
    } else if lower.contains("create") && lower.contains("token") {
        Some(parse_token_effect(source, lower))
    } else if lower.starts_with("mill ")
        || lower.contains(", mill ")
        || lower.starts_with("target player mills ")
        || lower.starts_with("target opponent mills ")
        || lower.starts_with("each opponent mills ")
    {
        Some(
            mill_amount(lower)
                .map(OracleEffect::Mill)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else {
        None
    }
}

/// Read a numeric keyword head from a clause ("amass Orcs 2").
fn keyword_number(lower: &str, head: &str) -> Option<u32> {
    let start = lower.find(head)?;
    let tail = &lower[start + head.len()..];
    let digits: String = tail
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if let Ok(n) = digits.parse::<u32>() {
        return (n > 0).then_some(n);
    }
    tail.split_whitespace()
        .find_map(|word| word.trim_matches(['.', ',']).parse::<u32>().ok())
        .filter(|n| *n > 0)
}

/// Read the amount of an "amass <subtype> N" clause without a keyword
/// node (used when the clause appears as a bare resolution).
fn amass_amount(lower: &str) -> Option<u32> {
    keyword_number(lower, "amass")
}

/// Recognize the amass keyword-action head when the parser kept it as a
/// keyword entry.
fn amass_keyword(lower: &str) -> Option<u32> {
    (lower.starts_with("amass ") || lower.contains(", amass "))
        .then(|| amass_amount(lower))
        .flatten()
}

/// Parse effects that draw, move, or filter cards.
fn parse_card_effect(source: &str, lower: &str) -> Option<OracleEffect> {
    if lower.contains("from your graveyard") && lower.contains("return") {
        Some(graveyard_return_effect(source, lower))
    } else if lower.contains("take an extra turn") || lower.contains("takes an extra turn") {
        Some(OracleEffect::ExtraTurn)
    } else if lower.contains("surveil") {
        Some(
            fixed_selection_count(lower, "surveil")
                .map(OracleEffect::Surveil)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else if lower.contains("scry") {
        Some(
            fixed_selection_count(lower, "scry")
                .map(OracleEffect::Scry)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else if lower.contains("add ") {
        Some(
            super::land::parse_tap_yield(&lower.replace('"', ""))
                .map(OracleEffect::Mana)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else if lower.contains("draw") || lower.contains("investigate") {
        Some(
            explicit_draw_amount(lower)
                .map(OracleEffect::Draw)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else if lower.contains("gain") && lower.contains("life") {
        if lower.contains("for each") || lower.contains("for every") {
            return Some(OracleEffect::Unsupported(source.to_string()));
        }
        Some(
            parse_number_after(lower, "gain")
                .map(OracleEffect::GainLife)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else {
        None
    }
}

/// Keep library choices on the controller and require a complete fixed count.
fn fixed_selection_count(lower: &str, keyword: &str) -> Option<u32> {
    let body = lower.trim().trim_end_matches('.');
    let body = body
        .strip_prefix("you may ")
        .or_else(|| body.strip_prefix("you "))
        .unwrap_or(body);
    let amount = body.strip_prefix(keyword)?.strip_prefix(' ')?;
    if amount.split_whitespace().count() != 1 {
        return None;
    }
    parse_oracle_number_word(amount)
}

/// Accept only explicit targeted returns whose card class and destination are modeled.
fn graveyard_return_effect(source: &str, lower: &str) -> OracleEffect {
    let body = lower
        .strip_prefix("you may ")
        .unwrap_or(lower)
        .trim_end_matches('.');
    let to_hand = match body {
        "return target card from your graveyard to your hand" => true,
        "return target creature card from your graveyard to the battlefield" => false,
        _ => return OracleEffect::Unsupported(source.to_string()),
    };
    OracleEffect::ReturnFromGraveyard { to_hand, count: 1 }
}

/// Read an unconditional controller draw with a complete explicit count.
fn explicit_draw_amount(lower: &str) -> Option<u32> {
    let body = lower
        .strip_prefix("you may ")
        .or_else(|| lower.strip_prefix("you "))
        .unwrap_or(lower);
    let amount = body.strip_prefix("draw ")?;
    let (word, cards) = amount.split_once(' ')?;
    if !matches!(cards.trim(), "card" | "cards") {
        return None;
    }
    parse_oracle_number_word(word)
}

/// Parse life and counter effects from a clause.
fn parse_life_and_counter_effect(source: &str, lower: &str) -> Option<OracleEffect> {
    if lower.contains("or more") && lower.contains("counter") && lower.contains("win") {
        Some(OracleEffect::WinsAtCounters(
            parse_number_before(lower, "or more").unwrap_or(u32::MAX),
        ))
    } else if lower.contains("counter") && lower.contains("put ") {
        Some(OracleEffect::Unsupported(source.to_string()))
    } else if (lower.contains("loses") || lower.contains(" lose ")) && lower.contains("life") {
        Some(
            fixed_life_loss(lower).unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else {
        None
    }
}

/// Require a complete fixed life-loss instruction and an explicit player scope.
fn fixed_life_loss(lower: &str) -> Option<OracleEffect> {
    use super::super::model::LifeLossScope;
    let body = lower.trim().trim_end_matches('.');
    let (amount, scope) = [
        ("you lose ", LifeLossScope::SelfOnly),
        ("lose ", LifeLossScope::SelfOnly),
        ("each opponent loses ", LifeLossScope::EachOpponent),
        ("each player loses ", LifeLossScope::EachPlayer),
        ("target player loses ", LifeLossScope::TargetPlayer),
        ("target opponent loses ", LifeLossScope::TargetPlayer),
        ("that player loses ", LifeLossScope::TargetPlayer),
    ]
    .into_iter()
    .find_map(|(prefix, scope)| body.strip_prefix(prefix).map(|tail| (tail, scope)))?;
    let amount = amount.strip_suffix(" life")?;
    if amount.split_whitespace().count() != 1 {
        return None;
    }
    Some(OracleEffect::LoseLife {
        amount: parse_oracle_number_word(amount)?,
        scope,
    })
}

/// Parse a complete charge-counter instruction for the source permanent only.
fn source_charge_counters(lower: &str, activation_x: bool) -> Option<OracleEffect> {
    let body = lower
        .trim()
        .strip_prefix("you may ")
        .unwrap_or(lower.trim());
    let (amount, destination) = body.strip_prefix("put ")?.split_once(" charge counter")?;
    let destination = destination.strip_prefix('s').unwrap_or(destination);
    if !matches!(
        destination,
        " on this artifact" | " on this creature" | " on this permanent"
    ) || amount.split_whitespace().count() != 1
    {
        return None;
    }
    let count = if amount == "x" && activation_x {
        0
    } else {
        parse_oracle_number_word(amount).filter(|count| *count > 0)?
    };
    Some(OracleEffect::PutChargeCounters(count))
}

/// Who a life-loss or damage clause reaches (CR 119.3). "Each opponent"
/// and "each player" use the format's opponent multiplier; "target
/// player"/"target opponent" affect exactly one player.
fn life_loss_scope(lower: &str) -> super::super::model::LifeLossScope {
    use super::super::model::LifeLossScope;
    if lower.contains("each opponent") || lower.contains("each player") {
        if lower.contains("each player") && !lower.contains("each opponent") {
            LifeLossScope::EachPlayer
        } else {
            LifeLossScope::EachOpponent
        }
    } else if lower.contains("target player") || lower.contains("target opponent") {
        LifeLossScope::TargetPlayer
    } else if lower.contains("you lose") || lower.ends_with(" damage to you") {
        LifeLossScope::SelfOnly
    } else {
        // Any other phrasing ("this creature loses N life" is not a
        // player life loss) reads as one opponent drain.
        LifeLossScope::TargetPlayer
    }
}

/// Parse player-targeted damage as drain; other damage stays unsupported.
fn parse_damage_effect(source: &str, lower: &str) -> OracleEffect {
    // The amount follows "deals" as a word or a digit ("deals 2 damage").
    let Some(amount) = lower.split_once("deals ").and_then(|(_, tail)| {
        let tail = tail.trim_start();
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits
            .parse::<u32>()
            .ok()
            .or_else(|| parse_oracle_number_word(tail))
    }) else {
        return OracleEffect::Unsupported(source.to_string());
    };
    let target = if lower.contains("damage to target player")
        || lower.contains("damage to target opponent")
        || lower.contains("damage to each opponent")
        || lower.contains("damage to each player")
        || lower.ends_with(" damage to you")
    {
        super::super::oracle_ast::DamageTarget::Player
    } else if lower.contains("damage to target battle") {
        super::super::oracle_ast::DamageTarget::Battle
    } else if lower.contains("damage to any target")
        || lower.contains("damage divided as you choose") && lower.contains("targets")
    {
        super::super::oracle_ast::DamageTarget::AnyTarget
    } else if lower.contains("damage to target creature")
        || lower.contains("damage to target planeswalker")
    {
        super::super::oracle_ast::DamageTarget::Permanent
    } else {
        super::super::oracle_ast::DamageTarget::Other
    };
    OracleEffect::Damage {
        amount,
        target,
        scope: life_loss_scope(lower),
    }
}

/// Parse a number word immediately after a phrase.
fn parse_number_after(text: &str, phrase: &str) -> Option<u32> {
    text.split_once(phrase)
        .and_then(|(_, tail)| parse_oracle_number_word(tail.trim_start()))
}

/// Parse the number immediately before a relationship phrase.
fn parse_number_before(text: &str, phrase: &str) -> Option<u32> {
    let before = text.split_once(phrase)?.0;
    before
        .split_whitespace()
        .last()
        .and_then(parse_oracle_number_word)
}

/// Read an explicit token count without inventing variable quantities.
fn token_amount(lower: &str) -> Option<u32> {
    if lower.contains("for each") || lower.contains("that many") || lower.contains("equal to") {
        return None;
    }
    let (_, tail) = lower.split_once("create")?;
    parse_oracle_number_word(tail.trim_start()).filter(|count| *count > 0)
}

/// Token creation scoped by the "for each" clause. "For each opponent"
/// creates that many tokens for every opponent (three in the commander
/// family). Other variable counts remain explicit unsupported wording.
fn parse_token_effect(source: &str, lower: &str) -> OracleEffect {
    if lower.contains("for each opponent") {
        // Per-opponent counts are small in practice ("for each opponent,
        // create a 1/1 … token"); read the amount after "create".
        return lower
            .split_once("create")
            .and_then(|(_, tail)| parse_oracle_number_word(tail.trim_start()))
            .map(|per_opponent| OracleEffect::CreateTokensPerOpponent { per_opponent })
            .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string()));
    }
    let Some(amount) = token_amount(lower) else {
        return OracleEffect::Unsupported(source.to_string());
    };
    if lower.contains("treasure token") {
        OracleEffect::CreateTreasureTokens(amount)
    } else {
        OracleEffect::CreateTokens(amount)
    }
}

/// Count cards milled from a digit or number word after "mill".
fn mill_amount(text: &str) -> Option<u32> {
    let (position, marker) = ["mills ", "mill "]
        .into_iter()
        .filter_map(|marker| text.find(marker).map(|position| (position, marker)))
        .min_by_key(|(position, _)| *position)?;
    parse_oracle_number_word(&text[position + marker.len()..])
}

/// Parse an energy gain: "you get {E}{E}", "you get three {E} (…)", or
/// "you get six {E} (…)". A symbol run counts its symbols; a numeric
/// word before "{E}" supplies the count.
fn energy_gain(lower: &str) -> Option<u32> {
    if !lower.contains("{e}") || !lower.contains("get ") {
        return None;
    }
    let tail = lower.split_once("get ")?.1;
    let symbols = tail.matches("{e}").count() as u32;
    if symbols > 1 {
        return Some(symbols);
    }
    if symbols == 0 {
        return None;
    }
    Some(parse_oracle_number_word(tail).unwrap_or(1).max(1))
}

/// Parse a digit or common number word at the start of a clause.
pub(super) fn parse_oracle_number_word(text: &str) -> Option<u32> {
    let word = text
        .split_whitespace()
        .next()?
        .trim_matches(|character: char| !character.is_ascii_alphanumeric());
    match word {
        "a" | "an" | "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        "eleven" => Some(11),
        "twelve" => Some(12),
        "thirteen" => Some(13),
        "fourteen" => Some(14),
        "fifteen" => Some(15),
        "sixteen" => Some(16),
        "seventeen" => Some(17),
        "eighteen" => Some(18),
        "nineteen" => Some(19),
        "twenty" => Some(20),
        "thirty" => Some(30),
        "forty" => Some(40),
        "fifty" => Some(50),
        "hundred" => Some(100),
        digits => digits.parse().ok(),
    }
}
