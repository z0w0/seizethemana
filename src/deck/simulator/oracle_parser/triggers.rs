//! Parsing for triggered abilities and Saga chapter statements.

use super::super::oracle_ast::*;
use super::statements::{is_trigger_start, strip_ability_word};

/// Parse a triggered event from the ability's opening words.
pub(super) fn parse_oracle_trigger_event(source: &str) -> Option<TriggerEvent> {
    let statement = strip_ability_word(source).trim();
    let lower = statement.to_ascii_lowercase();
    if !is_trigger_start(statement) || !lower.contains(',') {
        return None;
    }
    if lower.starts_with("at the beginning") {
        return parse_beginning_of_step(&lower);
    }
    let event_text = lower
        .split_once(',')
        .map_or(lower.as_str(), |(event, _)| event);
    if event_text.contains("opponent") || event_text.contains("attacks you") {
        return Some(TriggerEvent::Other(event_prefix(event_text).to_string()));
    }
    if lower.starts_with("when you cast this spell") {
        return Some(TriggerEvent::CastsSpell { this_spell: true });
    }
    if lower.contains("deals combat damage to a player")
        || lower.contains("deals combat damage to an opponent")
    {
        return Some(TriggerEvent::CombatDamageToPlayer(object_subject(&lower)));
    }
    if lower.contains("tap a nonland permanent for mana") {
        return Some(TriggerEvent::TappedForMana);
    }
    if (lower.starts_with("when you cast") || lower.starts_with("whenever you cast"))
        && lower.contains("spell")
    {
        return Some(TriggerEvent::CastsSpell { this_spell: false });
    }
    if lower.contains("land") && lower.contains("enters") && lower.contains("you control") {
        return Some(TriggerEvent::LandEnters(PlayerScope::You));
    }
    if lower.contains("attack") {
        return Some(TriggerEvent::Attacks(object_subject(&lower)));
    }
    if lower.contains("dies") || lower.contains("put into a graveyard") {
        return Some(TriggerEvent::Dies(object_subject(&lower)));
    }
    if lower.contains("enters") {
        return Some(TriggerEvent::Enters(object_subject(&lower)));
    }
    Some(TriggerEvent::Other(event_prefix(&lower).to_string()))
}

/// Parse the turn step and player for a beginning-of-step trigger.
fn parse_beginning_of_step(lower: &str) -> Option<TriggerEvent> {
    let (step, player) = if lower.contains("your upkeep") {
        (TurnStep::Upkeep, PlayerScope::You)
    } else if lower.contains("your end step") {
        (TurnStep::EndStep, PlayerScope::You)
    } else if lower.contains("your first main phase") {
        (TurnStep::FirstMainPhase, PlayerScope::You)
    } else if lower.contains("opponent") {
        (TurnStep::Other, PlayerScope::Opponent)
    } else {
        (TurnStep::Other, PlayerScope::Any)
    };
    Some(TriggerEvent::BeginningOfStep { step, player })
}

/// Choose the most specific event subject named in a trigger clause.
fn object_subject(lower: &str) -> ObjectSubject {
    if lower.contains("this ") {
        ObjectSubject::ThisPermanent
    } else if lower.contains("another ") {
        ObjectSubject::AnotherPermanent
    } else if lower.contains("land you control") {
        ObjectSubject::ControlledLand
    } else {
        ObjectSubject::Any
    }
}

/// Keep the trigger phrase separate from its resolution text.
fn event_prefix(lower: &str) -> &str {
    lower.find(',').map_or(lower, |comma| lower[..comma].trim())
}

/// Preserve legacy trigger-parser effects as syntax nodes before lowering.
pub(super) fn parse_oracle_triggered(source: String, event: TriggerEvent) -> TriggeredAbility {
    let resolution = trigger_resolution(&source);
    let effects = super::effects::parse_oracle_effects(resolution);
    let lower = source.to_ascii_lowercase();
    let once_per_turn = lower.contains("only once each turn")
        || lower.contains("only once each of your turns")
        || lower.contains("triggers only once each turn");
    // Intervening "if" clause (CR 603.4): text between the event's
    // comma and the resolution that starts with "if". The clause stays
    // part of the resolution text, so trimming it changes nothing the
    // effect parser reads twice.
    let condition = trigger_resolution(&source)
        .split_once(". ")
        .map_or(trigger_resolution(&source), |(first, _)| first)
        .strip_prefix("if ")
        .map(|body| body.trim_end_matches('.').trim().to_string());
    TriggeredAbility {
        event,
        effects,
        once_per_turn,
        condition,
    }
}

/// Separate a trigger event from the resolution clause after its comma.
fn trigger_resolution(source: &str) -> &str {
    let statement = strip_ability_word(source);
    if is_trigger_start(statement) {
        statement
            .split_once(',')
            .map_or(statement, |(_, resolution)| resolution.trim())
    } else {
        source
    }
}

/// Parse a Saga chapter line and any combined chapter numbers.
pub(super) fn parse_oracle_saga_chapter(source: &str) -> Option<SagaChapter> {
    let (symbol, body) = source
        .split_once('—')
        .or_else(|| source.split_once(" - "))?;
    let chapters = symbol
        .trim()
        .trim_end_matches('.')
        .split([',', ' '])
        .filter_map(roman_chapter)
        .collect::<Vec<_>>();
    if chapters.is_empty() {
        return None;
    }
    Some(SagaChapter {
        chapters,
        effects: super::effects::parse_oracle_effects(body),
    })
}

/// Parse one Roman-numeral Saga chapter marker.
fn roman_chapter(symbol: &str) -> Option<u32> {
    match symbol.trim() {
        "I" => Some(1),
        "II" => Some(2),
        "III" => Some(3),
        "IV" => Some(4),
        "V" => Some(5),
        _ => None,
    }
}

/// Return true when an Oracle segment begins with a Saga chapter marker.
pub(super) fn is_saga_header(source: &str) -> bool {
    source
        .split_once('—')
        .or_else(|| source.split_once(" - "))
        .is_some_and(|(symbol, _)| {
            symbol
                .trim()
                .split([',', ' '])
                .any(|part| roman_chapter(part).is_some())
        })
}
