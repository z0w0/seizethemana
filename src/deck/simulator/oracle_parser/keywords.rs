//! Parsing for keyword names and their typed parameters.

use super::super::model::BasicLandType;
use super::super::oracle_ast::{KeywordArgument, OracleKeyword, OracleKeywordName};

/// One table entry: the Oracle spelling and its parsed variant.
#[derive(Debug)]
pub(crate) struct KeywordEntry {
    /// Lowercase Oracle spelling of the keyword head.
    pub(crate) text: &'static str,
    /// The parsed keyword variant.
    pub(crate) name: OracleKeywordName,
}

/// Every known keyword, one source of truth for spelling lookup, name
/// mapping, and head-length matching. Entries are longest-first where a
/// prefix collision exists ("basic landcycling" before "landcycling").
pub(crate) const KEYWORDS: &[KeywordEntry] = &[
    KeywordEntry {
        text: "flying",
        name: OracleKeywordName::Flying,
    },
    KeywordEntry {
        text: "haste",
        name: OracleKeywordName::Haste,
    },
    KeywordEntry {
        text: "double strike",
        name: OracleKeywordName::DoubleStrike,
    },
    KeywordEntry {
        text: "first strike",
        name: OracleKeywordName::FirstStrike,
    },
    KeywordEntry {
        text: "prowess",
        name: OracleKeywordName::Prowess,
    },
    KeywordEntry {
        text: "trample",
        name: OracleKeywordName::Trample,
    },
    KeywordEntry {
        text: "menace",
        name: OracleKeywordName::Menace,
    },
    KeywordEntry {
        text: "flash",
        name: OracleKeywordName::Flash,
    },
    KeywordEntry {
        text: "undying",
        name: OracleKeywordName::Undying,
    },
    KeywordEntry {
        text: "cascade",
        name: OracleKeywordName::Cascade,
    },
    KeywordEntry {
        text: "dredge",
        name: OracleKeywordName::Dredge,
    },
    KeywordEntry {
        text: "ward",
        name: OracleKeywordName::Ward,
    },
    KeywordEntry {
        text: "deathtouch",
        name: OracleKeywordName::Deathtouch,
    },
    KeywordEntry {
        text: "lifelink",
        name: OracleKeywordName::Lifelink,
    },
    KeywordEntry {
        text: "vigilance",
        name: OracleKeywordName::Vigilance,
    },
    KeywordEntry {
        text: "reach",
        name: OracleKeywordName::Reach,
    },
    KeywordEntry {
        text: "defender",
        name: OracleKeywordName::Defender,
    },
    KeywordEntry {
        text: "indestructible",
        name: OracleKeywordName::Indestructible,
    },
    KeywordEntry {
        text: "hexproof",
        name: OracleKeywordName::Hexproof,
    },
    KeywordEntry {
        text: "protection",
        name: OracleKeywordName::Protection,
    },
    KeywordEntry {
        text: "crew",
        name: OracleKeywordName::Crew,
    },
    KeywordEntry {
        text: "transform",
        name: OracleKeywordName::Transform,
    },
    KeywordEntry {
        text: "cycling",
        name: OracleKeywordName::Cycling,
    },
    KeywordEntry {
        text: "basic landcycling",
        name: OracleKeywordName::BasicLandcycling,
    },
    KeywordEntry {
        text: "landcycling",
        name: OracleKeywordName::Landcycling,
    },
    KeywordEntry {
        text: "plainscycling",
        name: OracleKeywordName::Landcycling,
    },
    KeywordEntry {
        text: "islandcycling",
        name: OracleKeywordName::Landcycling,
    },
    KeywordEntry {
        text: "swampcycling",
        name: OracleKeywordName::Landcycling,
    },
    KeywordEntry {
        text: "mountaincycling",
        name: OracleKeywordName::Landcycling,
    },
    KeywordEntry {
        text: "forestcycling",
        name: OracleKeywordName::Landcycling,
    },
    KeywordEntry {
        text: "kicker",
        name: OracleKeywordName::Kicker,
    },
    KeywordEntry {
        text: "flashback",
        name: OracleKeywordName::Flashback,
    },
    KeywordEntry {
        text: "escape",
        name: OracleKeywordName::Escape,
    },
    KeywordEntry {
        text: "affinity",
        name: OracleKeywordName::Affinity,
    },
    KeywordEntry {
        text: "improvise",
        name: OracleKeywordName::Improvise,
    },
    KeywordEntry {
        text: "companion",
        name: OracleKeywordName::Companion,
    },
    KeywordEntry {
        text: "warp",
        name: OracleKeywordName::Warp,
    },
    KeywordEntry {
        text: "read ahead",
        name: OracleKeywordName::ReadAhead,
    },
    KeywordEntry {
        text: "reconfigure",
        name: OracleKeywordName::Reconfigure,
    },
    KeywordEntry {
        text: "morph",
        name: OracleKeywordName::Morph,
    },
    KeywordEntry {
        text: "megamorph",
        name: OracleKeywordName::Megamorph,
    },
    KeywordEntry {
        text: "disguise",
        name: OracleKeywordName::Disguise,
    },
    KeywordEntry {
        text: "manifest",
        name: OracleKeywordName::Manifest,
    },
    KeywordEntry {
        text: "proliferate",
        name: OracleKeywordName::Proliferate,
    },
    KeywordEntry {
        text: "mobilize",
        name: OracleKeywordName::Mobilize,
    },
    KeywordEntry {
        text: "saddle",
        name: OracleKeywordName::Saddle,
    },
    KeywordEntry {
        text: "storm",
        name: OracleKeywordName::Storm,
    },
    KeywordEntry {
        text: "convoke",
        name: OracleKeywordName::Convoke,
    },
    KeywordEntry {
        text: "delve",
        name: OracleKeywordName::Delve,
    },
    KeywordEntry {
        text: "offspring",
        name: OracleKeywordName::Offspring,
    },
    KeywordEntry {
        text: "plot",
        name: OracleKeywordName::Plot,
    },
    KeywordEntry {
        text: "living metal",
        name: OracleKeywordName::LivingMetal,
    },
    KeywordEntry {
        text: "afterlife",
        name: OracleKeywordName::Afterlife,
    },
    KeywordEntry {
        text: "power-up",
        name: OracleKeywordName::PowerUp,
    },
    KeywordEntry {
        text: "teamwork",
        name: OracleKeywordName::Teamwork,
    },
    KeywordEntry {
        text: "exhaust",
        name: OracleKeywordName::Exhaust,
    },
    KeywordEntry {
        text: "amass",
        name: OracleKeywordName::Amass,
    },
    KeywordEntry {
        text: "explore",
        name: OracleKeywordName::Explore,
    },
    KeywordEntry {
        text: "connive",
        name: OracleKeywordName::Connive,
    },
    KeywordEntry {
        text: "empower jace",
        name: OracleKeywordName::EmpowerJace,
    },
];

/// Parse a keyword entry and retain its typed parameters.
pub(super) fn parse_oracle_keyword(source: &str) -> OracleKeyword {
    let trimmed = source.trim();
    let delimiter = keyword_head_end(trimmed);
    let name_text = trimmed[..delimiter]
        .trim()
        .trim_end_matches(['-', '—'])
        .trim();
    let name = keyword_name(name_text);
    let mut arguments = Vec::new();
    let tail = &trimmed[delimiter..];
    // A specific landcycling spelling ("Forestcycling", "Plainscycling",
    // …) lowers to the shared Landcycling variant plus the named basic
    // land type argument.
    if matches!(name, OracleKeywordName::Landcycling)
        && let Some(land) = basic_land_type(name_text)
    {
        arguments.push(KeywordArgument::BasicLandType(land));
    }
    // A cycling life payment ("Cycling—Pay 2 life") rides the keyword
    // tail as a "pay N life" clause (CR 702.29).
    let lower_tail = tail.to_ascii_lowercase();
    if let Some(pay) = lower_tail.find("pay ")
        && let Some(amount) = lower_tail[pay + "pay ".len()..]
            .split_whitespace()
            .next()
            .and_then(|word| word.parse::<u32>().ok())
        && lower_tail[pay + "pay ".len()..].contains("life")
    {
        arguments.push(KeywordArgument::Life(amount));
    }
    // Cost keywords take a full mana cost, even an all-digit one
    // ("Cycling {2}" is a cost, not a count).
    let cost_keyword = matches!(
        name,
        OracleKeywordName::Offspring
            | OracleKeywordName::Plot
            | OracleKeywordName::Cycling
            | OracleKeywordName::BasicLandcycling
            | OracleKeywordName::Landcycling
            | OracleKeywordName::Kicker
            | OracleKeywordName::Flashback
            | OracleKeywordName::Escape
            | OracleKeywordName::Morph
            | OracleKeywordName::Megamorph
            | OracleKeywordName::Disguise
            | OracleKeywordName::Warp
    );
    // A brace group is a parameter. A pure-number group ("Crew {2}")
    // parses as a plain number unless the keyword's parameter is a cost;
    // a mixed group ("Kicker {1}{R}") keeps the whole mana cost.
    let brace_groups: Vec<&str> = tail
        .split('{')
        .skip(1)
        .filter_map(|group| group.split('}').next())
        .collect();
    let mixed_cost = brace_groups
        .iter()
        .any(|symbol| !symbol.chars().all(|c| c.is_ascii_digit()) && !symbol.trim().is_empty());
    if (mixed_cost || cost_keyword) && tail.contains('{') {
        if let Some(brace) = tail.find('{') {
            let symbols: String = tail[brace..]
                .chars()
                .take_while(|c| *c == '{' || *c == '}' || c.is_ascii_alphanumeric() || *c == '/')
                .collect();
            let cost = super::cost::parse_cost(&symbols);
            if cost.total() > 0 {
                arguments.push(KeywordArgument::Mana(cost));
            }
        }
    } else if let Some(symbol) = brace_groups.first()
        && let Ok(value) = symbol.parse::<u32>()
    {
        arguments.push(KeywordArgument::Number(value));
    }
    // Amass carries its count after the subtype ("Amass Orcs 2"): read
    // the last bare number of the tail.
    if matches!(name, OracleKeywordName::Amass) {
        let value = tail
            .split_whitespace()
            .filter_map(|word| word.trim_matches('.').parse::<u32>().ok())
            .next_back();
        if let Some(value) = value {
            arguments.push(KeywordArgument::Number(value));
        }
    }
    // A bare numeric tail ("Mobilize 2", "Crew 3", "Dredge 5"), only
    // when no parameter was found yet.
    if arguments.is_empty() {
        for parameter in tail
            .split([',', '—'])
            .map(str::trim)
            .filter(|parameter| !parameter.is_empty())
        {
            if let Ok(value) = parameter.parse::<u32>() {
                arguments.push(KeywordArgument::Number(value));
            }
        }
    }
    OracleKeyword { name, arguments }
}

/// Convert a keyword spelling into a known keyword variant or an open-set name.
fn keyword_name(name: &str) -> OracleKeywordName {
    let lower = name.trim().to_ascii_lowercase();
    KEYWORDS
        .iter()
        .find(|entry| entry.text == lower)
        .map(|entry| entry.name.clone())
        .unwrap_or(OracleKeywordName::Other(name.to_string()))
}

/// The basic land type named by a specific landcycling spelling.
fn basic_land_type(name_text: &str) -> Option<BasicLandType> {
    let lower = name_text.to_ascii_lowercase();
    match lower.as_str() {
        "plainscycling" => Some(BasicLandType::Plains),
        "islandcycling" => Some(BasicLandType::Island),
        "swampcycling" => Some(BasicLandType::Swamp),
        "mountaincycling" => Some(BasicLandType::Mountain),
        "forestcycling" => Some(BasicLandType::Forest),
        _ => None,
    }
}

/// Find the end of a known keyword name before its typed parameters.
fn keyword_head_end(source: &str) -> usize {
    let lower = source.to_ascii_lowercase();
    let delimiter = lower
        .find('{')
        .or_else(|| lower.find('—'))
        .unwrap_or(lower.len());
    let text = lower[..delimiter].trim_end_matches(['-', '—']).trim_end();
    known_keyword_head_end(text)
        .or_else(|| numbered_keyword_head_end(text))
        .unwrap_or(delimiter)
}

/// Find the end of a recognized keyword name before its parameters.
///
/// The table drives the match: a hit whose text equals the head or whose
/// head starts with the entry plus a space marks the name's length.
fn known_keyword_head_end(text: &str) -> Option<usize> {
    KEYWORDS
        .iter()
        .find(|entry| text == entry.text || text.starts_with(&format!("{} ", entry.text)))
        .map(|entry| entry.text.len())
}

/// Find the end of an unknown keyword followed by a numeric parameter.
fn numbered_keyword_head_end(text: &str) -> Option<usize> {
    text.rfind(' ').filter(|space| {
        text[space + 1..]
            .chars()
            .all(|character| character.is_ascii_digit())
    })
}

/// Recognize an Oracle line that consists only of a keyword ability.
pub(super) fn standalone_keyword(source: &str, known_keywords: &[OracleKeyword]) -> Option<String> {
    let value = source.trim().trim_end_matches('.').trim();
    let candidate = value
        .split_once('(')
        .map_or(value, |(keyword, _)| keyword.trim());
    let name_end = keyword_head_end(candidate);
    let name = candidate[..name_end]
        .trim()
        .trim_end_matches(['-', '—'])
        .trim();
    let parsed = keyword_name(name);
    // The remainder after the keyword head must be only its parameter
    // (a number, a mana cost, or a subtype plus number). Anything else
    // means the line is a spell or ability sentence that merely starts
    // with a keyword word ("Storm Drain deals 2 damage …").
    let tail = candidate[name_end..]
        .trim()
        .trim_start_matches(['—', '–', '-', ' '])
        .trim();
    let lower_tail = tail.to_ascii_lowercase();
    let parameter_only = tail.is_empty()
        || tail.split_whitespace().all(|word| {
            word.trim_matches(['{', '}', '.', ','])
                .parse::<u32>()
                .is_ok()
                || word.starts_with('{')
                || matches!(
                    name.to_ascii_lowercase().as_str(),
                    "amass"
                        | "saddle"
                        | "mobilize"
                        | "afterlife"
                        | "teamwork"
                        | "dredge"
                        | "crew"
                        | "connive"
                        | "empower jace"
                )
        })
        || tail.starts_with('{')
        || (lower_tail.starts_with("pay ") && lower_tail.contains("life"));
    let ability_word = candidate.split_once('—').is_some_and(|(_, remainder)| {
        let remainder = remainder.trim().to_ascii_lowercase();
        remainder.starts_with("when ")
            || remainder.starts_with("whenever ")
            || remainder.starts_with("at the beginning")
    });
    let known = !matches!(parsed, OracleKeywordName::Other(_))
        || known_keywords.iter().any(|keyword| keyword.name == parsed);
    (known && !ability_word && !candidate.contains(':') && parameter_only)
        .then(|| candidate.to_string())
}

/// Recognize a comma-separated line of keyword abilities, such as
/// "Flying, lifelink" or "// Reach, trample". Every part must parse as a
/// standalone keyword; otherwise the line is not a keyword list.
pub(super) fn keyword_line_list(
    source: &str,
    known_keywords: &[OracleKeyword],
) -> Option<Vec<OracleKeyword>> {
    let stripped = source
        .trim()
        .strip_prefix("//")
        .map(str::trim_start)
        .unwrap_or_else(|| source.trim());
    let parts: Vec<&str> = stripped.split(',').map(str::trim).collect();
    if parts.len() < 2 {
        return None;
    }
    let parsed = parts
        .iter()
        .map(|part| {
            standalone_keyword(part, known_keywords)
                .as_deref()
                .map(parse_oracle_keyword)
        })
        .collect::<Option<Vec<_>>>()?;
    (!parsed.is_empty()).then_some(parsed)
}

/// Add a keyword unless the same parsed name already exists.
pub(super) fn add_keyword(keywords: &mut Vec<OracleKeyword>, keyword: OracleKeyword) {
    if let Some(current) = keywords
        .iter_mut()
        .find(|current| current.name == keyword.name)
    {
        if current.arguments.is_empty() {
            current.arguments = keyword.arguments;
        }
    } else {
        keywords.push(keyword);
    }
}
