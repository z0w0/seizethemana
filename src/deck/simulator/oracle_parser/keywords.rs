//! Parsing for keyword names and their typed parameters.

use super::super::oracle_ast::{KeywordAbility, KeywordArgument, KeywordName};

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
    match name.trim().to_ascii_lowercase().as_str() {
        "flying" => KeywordName::Flying,
        "haste" => KeywordName::Haste,
        "double strike" => KeywordName::DoubleStrike,
        "prowess" => KeywordName::Prowess,
        "trample" => KeywordName::Trample,
        "menace" => KeywordName::Menace,
        "flash" => KeywordName::Flash,
        "undying" => KeywordName::Undying,
        "cascade" => KeywordName::Cascade,
        "crew" => KeywordName::Crew,
        "cycling" => KeywordName::Cycling,
        "basic landcycling" => KeywordName::BasicLandcycling,
        "landcycling" => KeywordName::Landcycling,
        "dredge" => KeywordName::Dredge,
        "kicker" => KeywordName::Kicker,
        "flashback" => KeywordName::Flashback,
        "escape" => KeywordName::Escape,
        "transform" => KeywordName::Transform,
        "ward" => KeywordName::Ward,
        "first strike" => KeywordName::FirstStrike,
        "deathtouch" => KeywordName::Deathtouch,
        "lifelink" => KeywordName::Lifelink,
        "vigilance" => KeywordName::Vigilance,
        "reach" => KeywordName::Reach,
        "defender" => KeywordName::Defender,
        "indestructible" => KeywordName::Indestructible,
        "hexproof" => KeywordName::Hexproof,
        "protection" => KeywordName::Protection,
        "affinity" => KeywordName::Affinity,
        "improvise" => KeywordName::Improvise,
        _ => KeywordName::Other(name.to_string()),
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
fn known_keyword_head_end(text: &str) -> Option<usize> {
    [
        "basic landcycling",
        "double strike",
        "first strike",
        "landcycling",
        "indestructible",
        "protection",
        "deathtouch",
        "vigilance",
        "flying",
        "haste",
        "prowess",
        "trample",
        "menace",
        "flash",
        "undying",
        "cascade",
        "crew",
        "cycling",
        "dredge",
        "kicker",
        "flashback",
        "escape",
        "transform",
        "ward",
        "lifelink",
        "reach",
        "defender",
        "hexproof",
        "affinity",
        "improvise",
    ]
    .into_iter()
    .find(|name| text == *name || text.starts_with(&format!("{name} ")))
    .map(str::len)
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
