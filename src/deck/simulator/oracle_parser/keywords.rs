//! Parsing for keyword names and their typed parameters.

use super::super::oracle_ast::{KeywordAbility, KeywordArgument, KeywordName};

/// The rules category of a known keyword.
///
/// CR 702 keyword abilities grant ongoing capabilities; CR 701 keyword
/// actions change game state; ability words (CR 702.200+) label triggers
/// and never appear as a `KeywordName`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeywordCategory {
    /// A CR 702 keyword ability.
    Ability,
    /// A CR 701 keyword action (e.g. Transform, 701.27).
    Action,
}

/// One table entry: the Oracle spelling and its parsed variant.
#[derive(Debug)]
pub(crate) struct KeywordEntry {
    /// Lowercase Oracle spelling of the keyword head.
    pub(crate) text: &'static str,
    /// The parsed keyword variant.
    pub(crate) name: KeywordName,
    /// The rules category of the keyword.
    pub(crate) category: KeywordCategory,
}

/// Every known keyword, one source of truth for spelling lookup, name
/// mapping, and head-length matching. Entries are longest-first where a
/// prefix collision exists ("basic landcycling" before "landcycling").
pub(crate) const KEYWORDS: &[KeywordEntry] = &[
    KeywordEntry {
        text: "flying",
        name: KeywordName::Flying,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "haste",
        name: KeywordName::Haste,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "double strike",
        name: KeywordName::DoubleStrike,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "first strike",
        name: KeywordName::FirstStrike,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "prowess",
        name: KeywordName::Prowess,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "trample",
        name: KeywordName::Trample,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "menace",
        name: KeywordName::Menace,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "flash",
        name: KeywordName::Flash,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "undying",
        name: KeywordName::Undying,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "cascade",
        name: KeywordName::Cascade,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "dredge",
        name: KeywordName::Dredge,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "ward",
        name: KeywordName::Ward,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "deathtouch",
        name: KeywordName::Deathtouch,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "lifelink",
        name: KeywordName::Lifelink,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "vigilance",
        name: KeywordName::Vigilance,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "reach",
        name: KeywordName::Reach,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "defender",
        name: KeywordName::Defender,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "indestructible",
        name: KeywordName::Indestructible,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "hexproof",
        name: KeywordName::Hexproof,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "protection",
        name: KeywordName::Protection,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "crew",
        name: KeywordName::Crew,
        category: KeywordCategory::Action,
    },
    KeywordEntry {
        text: "transform",
        name: KeywordName::Transform,
        category: KeywordCategory::Action,
    },
    KeywordEntry {
        text: "cycling",
        name: KeywordName::Cycling,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "basic landcycling",
        name: KeywordName::BasicLandcycling,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "landcycling",
        name: KeywordName::Landcycling,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "kicker",
        name: KeywordName::Kicker,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "flashback",
        name: KeywordName::Flashback,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "escape",
        name: KeywordName::Escape,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "affinity",
        name: KeywordName::Affinity,
        category: KeywordCategory::Ability,
    },
    KeywordEntry {
        text: "improvise",
        name: KeywordName::Improvise,
        category: KeywordCategory::Ability,
    },
];

/// Parse a keyword entry and retain its typed parameters.
pub(super) fn parse_oracle_keyword(source: &str) -> KeywordAbility {
    let trimmed = source.trim();
    let delimiter = keyword_head_end(trimmed);
    let name_text = trimmed[..delimiter]
        .trim()
        .trim_end_matches(['-', '—'])
        .trim();
    let name = keyword_name(name_text);
    let mut arguments = Vec::new();
    let tail = &trimmed[delimiter..];
    for parameter in tail
        .split([',', '—'])
        .map(str::trim)
        .filter(|parameter| !parameter.is_empty())
    {
        // Mana symbols (`{2}`) parse as their numeric value; only the
        // plain numeric tail carries meaning in the runtime model.
        if let Ok(value) = parameter
            .trim_matches(|c| c == '{' || c == '}')
            .parse::<u32>()
        {
            arguments.push(KeywordArgument::Number(value));
        }
    }
    KeywordAbility { name, arguments }
}

/// Convert a keyword spelling into a known keyword variant or an open-set name.
fn keyword_name(name: &str) -> KeywordName {
    let lower = name.trim().to_ascii_lowercase();
    KEYWORDS
        .iter()
        .find(|entry| entry.text == lower)
        .map(|entry| entry.name.clone())
        .unwrap_or(KeywordName::Other(name.to_string()))
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
pub(super) fn standalone_keyword(
    source: &str,
    known_keywords: &[KeywordAbility],
) -> Option<String> {
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
    let ability_word = candidate.split_once('—').is_some_and(|(_, remainder)| {
        let remainder = remainder.trim().to_ascii_lowercase();
        remainder.starts_with("when ")
            || remainder.starts_with("whenever ")
            || remainder.starts_with("at the beginning")
    });
    let known = !matches!(parsed, KeywordName::Other(_))
        || known_keywords.iter().any(|keyword| keyword.name == parsed);
    (known && !ability_word && !candidate.contains(':')).then(|| candidate.to_string())
}

/// Recognize a comma-separated line of keyword abilities, such as
/// "Flying, lifelink" or "// Reach, trample". Every part must parse as a
/// standalone keyword; otherwise the line is not a keyword list.
pub(super) fn keyword_line_list(
    source: &str,
    known_keywords: &[KeywordAbility],
) -> Option<Vec<KeywordAbility>> {
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
pub(super) fn add_keyword(keywords: &mut Vec<KeywordAbility>, keyword: KeywordAbility) {
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
