//! Parsing for supported spell, trigger, activation, and Saga effects.

use super::super::model::{SearchCardType, SearchDestination, SearchSpec};
use super::super::oracle_ast::OracleEffect;
use super::super::oracle_parser::{amount_after, draw_amount};

/// Parse supported search words into explicit card constraints.
fn parse_oracle_search_spec(text: &str) -> SearchSpec {
    let mana_value = search_mana_value(text);
    SearchSpec {
        card_type: search_card_type(text),
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
    } else if text.contains("land") {
        Some(SearchCardType::Land)
    } else {
        None
    }
}

/// Find the first supported color named by a search phrase.
fn search_color(text: &str) -> Option<char> {
    [
        ("white", 'W'),
        ("blue", 'U'),
        ("black", 'B'),
        ("red", 'R'),
        ("green", 'G'),
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

/// Parse compound effect forms that take precedence over clause splitting.
fn parse_compound_effect(source: &str, lower: &str) -> Option<Vec<OracleEffect>> {
    if lower.contains("remove all charge counters")
        && lower.contains("for each charge counter removed")
        && lower.contains("add ")
    {
        return Some(vec![
            super::super::parse_land::parse_tap_yield(lower)
                .map(OracleEffect::ManaPerCounter)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        ]);
    }
    if lower.contains("exile") && lower.contains("return it to the battlefield") {
        return Some(vec![OracleEffect::Blink]);
    }
    if lower.contains("each player") && lower.contains("discards") && lower.contains("draws") {
        return Some(vec![OracleEffect::Wheel]);
    }
    if lower.contains("as an additional cost to cast this spell") {
        return None;
    }
    if lower.contains("draw") && lower.contains("discard") {
        return Some(vec![if lower.contains("you may draw") {
            OracleEffect::Draw(draw_amount(lower).max(1))
        } else {
            OracleEffect::Loot(draw_amount(lower).max(1))
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
        "put ",
        "untap ",
        "exile ",
        "take an extra turn",
        "become the monarch",
        "deal ",
        "destroy ",
        "counter ",
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
            if lower.contains("deals") && lower.contains("damage to") {
                parse_damage_effect(source, &lower)
            } else {
                OracleEffect::Unsupported(source.to_string())
            }
        })
}

/// Parse effects that change permanents or create tokens.
fn parse_permanent_effect(source: &str, lower: &str) -> Option<OracleEffect> {
    if lower.starts_with("untap this ") {
        Some(OracleEffect::UntapSelf)
    } else if lower.contains("remove all charge counters") && lower.contains("add ") {
        Some(
            super::super::parse_land::parse_tap_yield(&lower.replace('"', ""))
                .map(OracleEffect::ManaPerCounter)
                .unwrap_or_else(|| OracleEffect::Unsupported(source.to_string())),
        )
    } else if lower.contains("additional land") {
        Some(OracleEffect::ExtraLand)
    } else if lower.contains("exile") && lower.contains("return it to the battlefield") {
        Some(OracleEffect::Blink)
    } else if lower.contains("you become the monarch") {
        Some(OracleEffect::Monarch)
    } else if lower.contains("create") && lower.contains("token") {
        Some(OracleEffect::Tokens(token_amount(lower)))
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
        Some(OracleEffect::Look {
            count: amount_after(lower, "surveil").max(1),
            surveil: true,
        })
    } else if lower.contains("scry") {
        Some(OracleEffect::Look {
            count: amount_after(lower, "scry"),
            surveil: false,
        })
    } else if lower.contains("add ") {
        Some(
            super::super::parse_land::parse_tap_yield(&lower.replace('"', ""))
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
        Some(OracleEffect::WinThreshold(
            parse_number_before(lower, "or more").unwrap_or(u32::MAX),
        ))
    } else if lower.contains("counter") && lower.contains("put ") {
        let count = if lower.contains("put x ") {
            0
        } else {
            parse_number_after(lower, "put").unwrap_or(1)
        };
        Some(OracleEffect::Counters(count))
    } else if (lower.contains("loses") || lower.contains(" lose ")) && lower.contains("life") {
        Some(OracleEffect::Drain(
            parse_number_after(lower, "loses")
                .or_else(|| parse_number_after(lower, "lose"))
                .unwrap_or(1),
        ))
    } else {
        None
    }
}

/// Parse player-targeted damage as drain; other damage stays unsupported.
fn parse_damage_effect(source: &str, lower: &str) -> OracleEffect {
    let player_target = lower.contains("damage to target player")
        || lower.contains("damage to target opponent")
        || lower.contains("damage to each opponent")
        || lower.contains("damage to each player");
    if !player_target {
        return OracleEffect::Unsupported(source.to_string());
    }
    let amount = lower
        .split_once("deals ")
        .and_then(|(_, tail)| parse_oracle_number_word(tail))
        .unwrap_or(1);
    OracleEffect::Drain(amount)
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
        digits => digits.parse().ok(),
    }
}
