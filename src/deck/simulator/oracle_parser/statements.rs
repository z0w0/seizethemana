//! Splitting and classification for Oracle ability statements.

use super::super::oracle_ast::*;
use super::activation::is_activation_start;
use super::effects::parse_oracle_number_word;
use super::keywords::parse_oracle_keyword;

/// Split Oracle paragraphs into statements while keeping clauses intact.
pub(super) fn oracle_segments(oracle_text: &str, keywords: &[KeywordAbility]) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    for line in oracle_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if current.is_empty() {
            current = line.to_string();
        } else if starts_new_statement(&current, line, keywords) {
            segments.push(current);
            current = line.to_string();
        } else {
            current.push(' ');
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// Decide whether a line begins a new ability instead of continuing one.
fn starts_new_statement(current: &str, next: &str, keywords: &[KeywordAbility]) -> bool {
    let lower_next = next.to_ascii_lowercase();
    let stripped = strip_ability_word(next);
    if is_trigger_start(stripped)
        || is_activation_start(next)
        || super::triggers::is_saga_header(next)
        || lower_next.contains("station (")
        || lower_next.contains('|')
        || super::keywords::standalone_keyword(next, keywords).is_some()
        || is_static_statement(next)
    {
        return true;
    }
    let lower_current = current.to_ascii_lowercase();
    if is_trigger_start(strip_ability_word(current))
        || is_activation_start(current)
        || lower_current.starts_with("choose one")
        || lower_current.starts_with("choose two")
    {
        return false;
    }
    is_spell_statement(next) || is_static_statement(next)
}

/// Return true for a triggered-ability prefix.
pub(super) fn is_trigger_start(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("when ")
        || lower.starts_with("whenever ")
        || lower.starts_with("at the beginning")
}

/// Classify syntax that did not match a complete ability grammar.
pub(super) fn unsupported_kind(source: &str) -> UnsupportedAbilityKind {
    if is_activation_start(source) {
        UnsupportedAbilityKind::Activated
    } else if is_trigger_start(source) {
        UnsupportedAbilityKind::Triggered
    } else {
        UnsupportedAbilityKind::Unknown
    }
}

/// Strip an ability-word label before parsing the rules statement.
pub(super) fn strip_ability_word(source: &str) -> &str {
    let Some((label, statement)) = source.split_once(" — ") else {
        return source;
    };
    let label = label.trim();
    let single_word = !label.contains(' ');
    let known = matches!(
        label,
        "Constellation"
            | "Landfall"
            | "Raid"
            | "Revolt"
            | "Battle cry"
            | "Spectacle"
            | "Alliance"
            | "Training"
            | "Heroic"
            | "Inspired"
            | "Delirium"
            | "Magecraft"
            | "Descend"
            | "Forge"
            | "Commit"
            | "Will"
            | "Spellcraft"
            | "Start your engines"
            | "Renown"
            | "Afflict"
            | "Battalion"
            | "Bloodrush"
            | "Channel"
            | "Conspire"
    );
    if (single_word || known)
        && (statement.starts_with("When")
            || statement.starts_with("Whenever")
            || statement.starts_with("At the beginning")
            || statement.starts_with('{')
            || statement.starts_with(['−', '–', '+', '-']))
    {
        statement.trim()
    } else {
        source
    }
}

/// Parse known static-effect forms and preserve other static rules.
pub(super) fn parse_oracle_static(source: &str) -> StaticAbility {
    let lower = source.to_ascii_lowercase();
    let effect = if lower.contains("additional land") {
        StaticEffect::AdditionalLandDrop
    } else if lower.contains("doesn't untap during your untap step")
        || lower.contains("does not untap during your untap step")
    {
        StaticEffect::DoesntUntap
    } else if lower.contains("or more") && lower.contains("counter") && lower.contains("win") {
        let counters = lower
            .split_once(" or more")
            .and_then(|(before, _)| before.split_whitespace().last())
            .and_then(parse_oracle_number_word)
            .unwrap_or(u32::MAX);
        StaticEffect::WinThreshold(counters)
    } else if let Some((power, toughness)) =
        super::super::parse_equipment::parse_creature_buff(&lower)
    {
        StaticEffect::CreatureBuff { power, toughness }
    } else if lower.contains("you control have") && lower.contains("{t}: add ") {
        let target = if lower.contains("creatures you control") {
            StaticTarget::CreaturesYouControl
        } else if lower.contains("lands you control") {
            StaticTarget::LandsYouControl
        } else {
            StaticTarget::PermanentsYouControl
        };
        super::super::parse_land::parse_tap_yield(&lower.replace('"', ""))
            .map(|yield_| StaticEffect::ManaGrant { target, yield_ })
            .unwrap_or_else(|| StaticEffect::Unsupported(source.to_string()))
    } else if let Some((target, keyword)) = parse_keyword_grant(source, &lower) {
        StaticEffect::KeywordGrant { target, keyword }
    } else if let Some(effect) = parse_cost_reduction(source, &lower) {
        effect
    } else {
        StaticEffect::Unsupported(source.to_string())
    };
    StaticAbility {
        effects: vec![effect],
    }
}

/// Parse a flat spell-cost reduction. Board-scaled discounts ("for each",
/// "where X is") and unknown amounts stay unsupported.
fn parse_cost_reduction(source: &str, lower: &str) -> Option<StaticEffect> {
    if !lower.contains("less to cast") || lower.contains("for each") || lower.contains("where x is")
    {
        return None;
    }
    let amount = (lower
        .split_once("cost {")
        .or_else(|| lower.split_once("costs {"))?)
    .1
    .split('}')
    .next()?
    .trim()
    .parse::<u32>()
    .ok();
    match amount {
        Some(amount) => Some(StaticEffect::CostReduction {
            amount,
            spell_class: lower.split(" cost").next().unwrap_or("spells").to_string(),
        }),
        None => Some(StaticEffect::Unsupported(source.to_string())),
    }
}

/// Parse a static keyword grant for this card, creatures, lands, spells,
/// or permanents.
fn parse_keyword_grant(source: &str, lower: &str) -> Option<(StaticTarget, KeywordAbility)> {
    let (target, keyword) =
        if lower.starts_with("this creature has ") || lower.starts_with("this permanent has ") {
            (StaticTarget::Source, source.split_once(" has ")?.1)
        } else if lower.starts_with("creatures you control have ") {
            (
                StaticTarget::CreaturesYouControl,
                source.split_once(" have ")?.1,
            )
        } else if lower.starts_with("lands you control have ") {
            (
                StaticTarget::LandsYouControl,
                source.split_once(" have ")?.1,
            )
        } else if lower.starts_with("spells you cast have ") {
            (StaticTarget::SpellsYouCast, source.split_once(" have ")?.1)
        } else if lower.starts_with("permanents you control have ") {
            (
                StaticTarget::PermanentsYouControl,
                source.split_once(" have ")?.1,
            )
        } else {
            return None;
        };
    Some((
        target,
        parse_oracle_keyword(keyword.trim().trim_end_matches('.')),
    ))
}

/// Identify static rules without a triggered or activated prefix.
pub(super) fn is_static_statement(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    [
        "as this ",
        "as long as ",
        "creatures you control ",
        "lands you control ",
        "nonland spells ",
        "artifact spells ",
        "spells you cast ",
        "permanents you control ",
        "each creature ",
        "each land ",
        "if there are ",
        "if this card is in your opening hand",
        "you may play an additional land",
        "doesn't untap during your untap step",
        "you may begin the game",
        "this creature has ",
        "this permanent has ",
        "this spell costs ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || lower.contains("you control have {t}:")
        || lower.contains("doesn't untap during your untap step")
        || lower.contains("does not untap during your untap step")
}

/// Identify a one-shot spell statement or resolution clause.
pub(super) fn is_spell_statement(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    [
        "as an additional cost to cast this spell",
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
        "each player ",
        "each opponent ",
        "target ",
        "exile ",
        "destroy ",
        "put ",
        "deal ",
        "deals ",
        "you lose ",
        "take an extra turn",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || lower.contains(" deals ") && lower.contains(" damage to ")
}
