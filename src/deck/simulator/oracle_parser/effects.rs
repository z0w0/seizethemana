//! Parsing for supported spell, trigger, activation, and Saga effects.

use super::super::model::{
    BasicLandType, ManaColor, SearchCardType, SearchDestination, SearchSpec,
};
use super::super::oracle_ast::OracleEffect;
use super::super::oracle_parser::{amount_after, draw_amount};

/// Parse supported search words into explicit card constraints.
fn parse_oracle_search_spec(text: &str) -> SearchSpec {
    let mana_value = search_mana_value(text);
    let card_type = search_card_type(text);
    let land_types = if matches!(
        card_type,
        Some(SearchCardType::Land | SearchCardType::BasicLand)
    ) {
        let types = BasicLandType::ALL.map(|land| text.contains(&land.name().to_ascii_lowercase()));
        if types.iter().any(|allowed| *allowed) {
            types
        } else {
            [true; 5]
        }
    } else {
        [false; 5]
    };
    SearchSpec {
        card_type,
        color: search_color(text),
        colorless: text.contains("colorless"),
        mana_value: if text.contains("exactly") {
            mana_value
        } else {
            None
        },
        max_mana_value: if text.contains("or less") || text.contains("less") {
            mana_value
        } else {
            None
        },
        min_mana_value: if text.contains("or more") || text.contains("or greater") {
            mana_value
        } else {
            None
        },
        destination: search_destination(text),
        optional: text.contains("you may search")
            || text.contains("you may put")
            || text.contains("up to one"),
        top_count: search_top_count(text),
        non_human: text.contains("non-human"),
        land_types,
        basic_land_only: matches!(card_type, Some(SearchCardType::BasicLand)),
    }
}

/// Find the card type named by a supported search phrase.
fn search_card_type(text: &str) -> Option<SearchCardType> {
    if text.contains("basic land") {
        Some(SearchCardType::BasicLand)
    } else if text.contains("artifact or enchantment") {
        Some(SearchCardType::ArtifactOrEnchantment)
    } else if text.contains("instant or sorcery") {
        Some(SearchCardType::InstantSorcery)
    } else if text.contains("creature") {
        Some(SearchCardType::Creature)
    } else if text.contains("planeswalker") {
        Some(SearchCardType::Planeswalker)
    } else if text.contains("enchantment") {
        Some(SearchCardType::Enchantment)
    } else if text.contains("artifact") {
        Some(SearchCardType::Artifact)
    } else if text.contains("permanent") {
        Some(SearchCardType::Permanent)
    } else if text.contains("land")
        || BasicLandType::ALL
            .iter()
            .any(|kind| text.contains(&kind.name().to_ascii_lowercase()))
    {
        Some(SearchCardType::Land)
    } else {
        None
    }
}

/// Find the first supported color named by a search phrase.
fn search_color(text: &str) -> Option<ManaColor> {
    [
        ("white", ManaColor::White),
        ("blue", ManaColor::Blue),
        ("black", ManaColor::Black),
        ("red", ManaColor::Red),
        ("green", ManaColor::Green),
    ]
    .into_iter()
    .find_map(|(word, symbol)| text.contains(word).then_some(symbol))
}

/// Parse a mana value following the words "mana value".
fn search_mana_value(text: &str) -> Option<u32> {
    text.split("mana value ").nth(1).and_then(|tail| {
        tail.split_whitespace().find_map(|value| {
            value
                .trim_matches(|character: char| !character.is_ascii_digit())
                .parse::<u32>()
                .ok()
        })
    })
}

/// Find the destination named by a supported search phrase.
fn search_destination(text: &str) -> SearchDestination {
    if text.contains("on top of") {
        SearchDestination::LibraryTop
    } else if text.contains("exile it") || text.contains("exile that card") {
        SearchDestination::Exile
    } else if text.contains("onto the battlefield tapped") {
        SearchDestination::BattlefieldTapped
    } else if text.contains("onto the battlefield") {
        SearchDestination::Battlefield
    } else {
        SearchDestination::Hand
    }
}

/// Parse a supported count after the words "top".
fn search_top_count(text: &str) -> Option<usize> {
    text.split("top ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|word| match word {
            "five" => Some(5),
            "four" => Some(4),
            "three" => Some(3),
            "two" => Some(2),
            "one" => Some(1),
            digits => digits.parse().ok(),
        })
}

/// Parse supported effects from Oracle text and preserve unknown clauses.
pub(super) fn parse_oracle_effects(source: &str) -> Vec<OracleEffect> {
    // Reminder text (parenthesized) clarifies a keyword; it is not rules
    // text for the resolution, so drop it before effect parsing. This
    // keeps "it explores. (Reveal the top card …)" from reading as a
    // search effect.
    let source = &strip_reminder_text(source);
    let lower = source.to_ascii_lowercase();
    if lower.contains("at the beginning of the next end step") {
        return vec![OracleEffect::Unsupported(source.to_string())];
    }
    if let Some(effect) = parse_compound_effect(source, &lower) {
        return effect;
    }
    split_effect_clauses(source)
        .iter()
        .map(|clause| parse_oracle_effect_clause(clause))
        .collect()
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
            super::land::parse_tap_yield(lower)
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
        return Some(vec![if lower.contains("you may draw") {
            OracleEffect::Draw(draw_amount(lower).max(1))
        } else {
            OracleEffect::DrawThenDiscard(draw_amount(lower).max(1))
        }]);
    }
    if lower.contains("draw") && lower.contains("-1/-1 counter") {
        return Some(vec![OracleEffect::DrawAndMinusCounter]);
    }
    let is_search = lower.contains("search your library")
        || lower.contains("look at the top") && lower.contains("from among them")
        || lower.contains("reveal the top")
            && (lower.contains("from among them") || lower.contains("into your hand"));
    is_search.then(|| vec![OracleEffect::Search(parse_oracle_search_spec(lower))])
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
    if lower.starts_with("untap this ") {
        Some(OracleEffect::UntapSource)
    } else if let Some(energy) = energy_gain(lower) {
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
        Some(parse_token_effect(lower))
    } else if lower.starts_with("mill ")
        || lower.contains(", mill ")
        || lower.starts_with("target player mills ")
        || lower.starts_with("target opponent mills ")
        || lower.starts_with("each opponent mills ")
    {
        Some(OracleEffect::Mill(mill_amount(lower)))
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
        Some(OracleEffect::ReturnFromGraveyard {
            to_hand: lower.contains("to your hand"),
            count: parse_number_after(lower, "return").unwrap_or(1),
        })
    } else if lower.contains("take an extra turn") || lower.contains("takes an extra turn") {
        Some(OracleEffect::ExtraTurn)
    } else if lower.contains("surveil") {
        Some(OracleEffect::Surveil(amount_after(lower, "surveil").max(1)))
    } else if lower.contains("scry") {
        Some(OracleEffect::Scry(amount_after(lower, "scry")))
    } else if lower.contains("add ") {
        Some(
            super::land::parse_tap_yield(&lower.replace('"', ""))
                .map(OracleEffect::Mana)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else if lower.contains("draw") || lower.contains("investigate") {
        Some(OracleEffect::Draw(draw_amount(lower).max(1)))
    } else if lower.contains("gain") && lower.contains("life") {
        Some(OracleEffect::GainLife(
            parse_number_after(lower, "gain").unwrap_or(1),
        ))
    } else {
        None
    }
}

/// Parse life and counter effects from a clause.
fn parse_life_and_counter_effect(_source: &str, lower: &str) -> Option<OracleEffect> {
    if lower.contains("or more") && lower.contains("counter") && lower.contains("win") {
        Some(OracleEffect::WinsAtCounters(
            parse_number_before(lower, "or more").unwrap_or(u32::MAX),
        ))
    } else if lower.contains("counter") && lower.contains("put ") {
        let count = if lower.contains("put x ") {
            0
        } else {
            parse_number_after(lower, "put").unwrap_or(1)
        };
        Some(OracleEffect::PutChargeCounters(count))
    } else if (lower.contains("loses") || lower.contains(" lose ")) && lower.contains("life") {
        let amount = parse_number_after(lower, "loses")
            .or_else(|| parse_number_after(lower, "lose"))
            .unwrap_or(1);
        Some(OracleEffect::LoseLife {
            amount,
            scope: life_loss_scope(lower),
        })
    } else {
        None
    }
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
    } else {
        // Unqualified "you lose N life" is a self-cost, not a drain; the
        // conservative read of any other phrasing is one opponent.
        LifeLossScope::TargetPlayer
    }
}

/// Parse player-targeted damage as drain; other damage stays unsupported.
fn parse_damage_effect(_source: &str, lower: &str) -> OracleEffect {
    // The amount follows "deals" as a word or a digit ("deals 2 damage").
    let amount = lower
        .split_once("deals ")
        .and_then(|(_, tail)| {
            let tail = tail.trim_start();
            let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits
                .parse::<u32>()
                .ok()
                .or_else(|| parse_oracle_number_word(tail))
        })
        .unwrap_or(1);
    let target = if lower.contains("damage to target player")
        || lower.contains("damage to target opponent")
        || lower.contains("damage to each opponent")
        || lower.contains("damage to each player")
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

/// Count generated tokens, with a fixed cap for unsupported scaling forms.
fn token_amount(lower: &str) -> u32 {
    if lower.contains("for each") {
        return 8;
    }
    let Some((_, tail)) = lower.split_once("create") else {
        return 2;
    };
    parse_oracle_number_word(tail.trim_start())
        .map(|amount| amount.min(8))
        .filter(|amount| *amount > 0)
        .unwrap_or_else(|| {
            if tail.trim_start().starts_with("a ") || tail.trim_start().starts_with("an ") {
                1
            } else {
                2
            }
        })
}

/// Token creation scoped by the "for each" clause. "For each opponent"
/// creates that many tokens for every opponent (three in the commander
/// family); the count is capped at the creature cap. Any other scaling
/// ("for each creature you control") keeps the bounded best-case 8.
fn parse_token_effect(lower: &str) -> OracleEffect {
    if lower.contains("for each opponent") {
        // Per-opponent counts are small in practice ("for each opponent,
        // create a 1/1 … token"); read the amount after "create".
        let per_opponent = lower
            .split_once("create")
            .and_then(|(_, tail)| parse_oracle_number_word(tail.trim_start()))
            .unwrap_or(1)
            .clamp(1, 4);
        OracleEffect::CreateTokensPerOpponent { per_opponent }
    } else if lower.contains("treasure token") {
        OracleEffect::CreateTreasureTokens(token_amount(lower))
    } else {
        OracleEffect::CreateTokens(token_amount(lower))
    }
}

/// Count cards milled from a digit or number word after "mill".
fn mill_amount(text: &str) -> u32 {
    let mut best = 0;
    let mut remaining = text;
    while let Some((position, marker)) = ["mills ", "mill "]
        .into_iter()
        .filter_map(|marker| remaining.find(marker).map(|position| (position, marker)))
        .min_by_key(|(position, _)| *position)
    {
        let tail = &remaining[position + marker.len()..];
        let amount = parse_oracle_number_word(tail).unwrap_or(0);
        best = best.max(amount);
        remaining = tail;
        if remaining.is_empty() {
            break;
        }
    }
    best
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
