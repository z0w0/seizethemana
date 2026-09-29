//! Parsing for triggered abilities and Saga chapter statements.

use super::super::oracle_ast::*;
use super::effects::parse_oracle_number_word;
use super::statements::{is_trigger_start, strip_ability_word};

/// Parse a triggered event from the ability's opening words.
pub(super) fn parse_oracle_trigger_event(source: &str) -> Option<OracleTriggerEvent> {
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
    if lower.starts_with("when you cast this spell") {
        return Some(OracleTriggerEvent::CastsSpell { this_spell: true });
    }
    // "Whenever the Ring tempts you, …" (CR 701.54d). The event text
    // names the Ring, not an opponent, so it must precede the generic
    // opponent-scope fallback.
    if event_text.contains("the ring tempts you") {
        return Some(OracleTriggerEvent::RingTempts);
    }
    if lower.contains("deals combat damage to a player")
        || lower.contains("deals combat damage to an opponent")
    {
        return Some(OracleTriggerEvent::CombatDamageToPlayer(object_subject(
            &lower,
        )));
    }
    // Opponent-scoped events stay inert; the combat-damage check above
    // must come first ("deals combat damage to an opponent" is a
    // combat-damage trigger, not a generic opponent trigger).
    if event_text.contains("opponent") || event_text.contains("attacks you") {
        return Some(OracleTriggerEvent::Other(
            event_prefix(event_text).to_string(),
        ));
    }
    if lower.contains("tap a nonland permanent for mana") {
        return Some(OracleTriggerEvent::TappedForMana);
    }
    if (lower.starts_with("when you cast") || lower.starts_with("whenever you cast"))
        && lower.contains("spell")
    {
        return Some(OracleTriggerEvent::CastsSpell { this_spell: false });
    }
    if lower.contains("land") && lower.contains("enters") && lower.contains("you control") {
        return Some(OracleTriggerEvent::LandEnters(PlayerScope::You));
    }
    if lower.contains("attack") {
        if event_text.contains("you attack") || event_text.contains("one or more creatures attack")
        {
            return Some(OracleTriggerEvent::PlayerAttacks);
        }
        return Some(OracleTriggerEvent::Attacks(object_subject(&lower)));
    }
    if lower.contains("dies") || lower.contains("put into a graveyard") {
        return Some(OracleTriggerEvent::Dies(object_subject(&lower)));
    }
    if lower.contains("enters") {
        return Some(OracleTriggerEvent::Enters(object_subject(&lower)));
    }
    // A state trigger on a counter threshold: "When [this] has N or
    // more [kind] counters on it, you win the game." (Darksteel Reactor
    // class). The threshold number rides in the event text; the
    // resolution must win, so counter-placement triggers ("Whenever one
    // or more counters are put on this permanent, draw a card.") stay
    // out of the win check.
    if let Some(counters) = state_trigger_threshold(&lower, event_text) {
        return Some(OracleTriggerEvent::WinsAtCounters { counters });
    }
    Some(OracleTriggerEvent::Other(event_prefix(&lower).to_string()))
}

/// The counter threshold of a win-state trigger ("When this artifact
/// has twenty or more charge counters on it, you win the game."): the
/// event names "counters on it" and the resolution wins.
fn state_trigger_threshold(lower: &str, event_text: &str) -> Option<u32> {
    if !event_text.contains("counter") || !event_text.contains("counters on it") {
        return None;
    }
    if !lower.contains("you win") {
        return None;
    }
    let before = event_text
        .split_once(" or more")
        .map_or(event_text, |(before, _)| before);
    let amount = before
        .split_whitespace()
        .last()?
        .trim_end_matches(['.', ',', ':']);
    parse_oracle_number_word(amount).or_else(|| amount.parse::<u32>().ok())
}

/// Parse the phase or step and player for a beginning-of-phase trigger
/// (CR 603.2b). Only the upkeep step, the end step, and the precombat
/// main phase carry modeled timings; every other phase or step name is
/// kept as text.
fn parse_beginning_of_step(lower: &str) -> Option<OracleTriggerEvent> {
    if lower.contains("your upkeep") {
        return Some(OracleTriggerEvent::BeginningOfUpkeep(PlayerScope::You));
    }
    if lower.contains("your end step") {
        return Some(OracleTriggerEvent::BeginningOfEndStep(PlayerScope::You));
    }
    if lower.contains("your first main phase") {
        return Some(OracleTriggerEvent::BeginningOfPrecombatMain(
            PlayerScope::You,
        ));
    }
    let player = if lower.contains("opponent") {
        PlayerScope::Opponent
    } else if lower.contains("your ") {
        PlayerScope::You
    } else {
        PlayerScope::Any
    };
    Some(OracleTriggerEvent::BeginningOfOther {
        phase: beginning_phase_name(lower),
        player,
    })
}

/// The phase or step name after "at the beginning of": the ownership
/// phrase ("each opponent's", "your") and the turn qualifier drop, so
/// "each opponent's upkeep" reads "upkeep" and "your combat phase on
/// your turn" reads "combat phase".
fn beginning_phase_name(lower: &str) -> String {
    let rest = lower.trim_start_matches("at the beginning of ").trim();
    let end = rest.find([',', '(']).unwrap_or(rest.len());
    let name = rest[..end]
        .trim()
        .trim_start_matches("each ")
        .trim_start_matches("the ")
        .trim_start_matches("your ");
    let name = name.split_once("'s ").map_or(name, |(_, rest)| rest);
    let name = name.split_once("’s ").map_or(name, |(_, rest)| rest);
    let mut name = name.trim().to_string();
    for marker in [
        " on your turn",
        " of your turn",
        " on that turn",
        " of that turn",
    ] {
        if let Some(index) = name.rfind(marker) {
            name.truncate(index);
        }
    }
    name
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
pub(super) fn parse_oracle_triggered(
    source: String,
    event: OracleTriggerEvent,
) -> OracleTriggeredAbility {
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
    let resolution = trigger_resolution(&source);
    let condition = resolution
        .strip_prefix("if ")
        .and_then(|body| body.split_once(',').map(|(condition, _)| condition))
        .map(|condition| condition.trim().trim_end_matches('.').to_string());
    let is_mana_ability = matches!(event, OracleTriggerEvent::TappedForMana)
        && !lower.contains("target")
        && effects.iter().any(|effect| {
            matches!(
                effect,
                OracleEffect::Mana(_) | OracleEffect::ManaPerCounter(_)
            )
        })
        && !effects.iter().any(|effect| match effect {
            OracleEffect::Search(_)
            | OracleEffect::Draw(_)
            | OracleEffect::DrawThenDiscard(_)
            | OracleEffect::DrawAndMinusCounter
            | OracleEffect::Scry(_)
            | OracleEffect::Surveil(_)
            | OracleEffect::Mill(_) => true,
            OracleEffect::Unsupported(text) => text.to_ascii_lowercase().contains("library"),
            _ => false,
        });
    OracleTriggeredAbility {
        event,
        is_mana_ability,
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
pub(super) fn parse_oracle_saga_chapter(source: &str) -> Option<OracleSagaChapter> {
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
    Some(OracleSagaChapter {
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
