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
    if event_text == "whenever equipped creature dies"
        || event_text == "when equipped creature dies"
    {
        return Some(OracleTriggerEvent::EquippedCreatureDies);
    }
    if let Some((_, second)) = event_text.split_once(" enters or ") {
        let subject = object_subject(event_text);
        let second = match second {
            "dies" => Some(OracleTriggerEvent::Dies(subject)),
            "attacks" => Some(OracleTriggerEvent::Attacks(subject)),
            _ => None,
        };
        if let Some(second) = second {
            return Some(OracleTriggerEvent::AnyOf(vec![
                OracleTriggerEvent::Enters(subject),
                second,
            ]));
        }
    }
    if event_text == "when you cast this spell" {
        return Some(OracleTriggerEvent::CastsSpell { this_spell: true });
    }
    // "Whenever the Ring tempts you, …" (CR 701.54d). The event text
    // names the Ring, not an opponent, so it must precede the generic
    // opponent-scope fallback.
    if event_text.contains("the ring tempts you") {
        return Some(OracleTriggerEvent::RingTempts);
    }
    if event_text.contains("deals combat damage to a player")
        || event_text.contains("deals combat damage to an opponent")
    {
        return Some(OracleTriggerEvent::CombatDamageToPlayer(object_subject(
            event_text,
        )));
    }
    if event_text.starts_with("whenever an opponent casts ") {
        return Some(OracleTriggerEvent::OpponentSpellCast {
            noncreature: event_text.contains("noncreature spell"),
            first: event_text.contains("their first ") && event_text.ends_with("each turn"),
        });
    }
    if event_text == "whenever an opponent draws a card" {
        return Some(OracleTriggerEvent::OpponentDraw);
    }
    if event_text.contains("opponent") || event_text.contains("attacks you") {
        return Some(OracleTriggerEvent::Other(
            event_prefix(event_text).to_string(),
        ));
    }
    if event_text.contains("tap a nonland permanent for mana") {
        return Some(OracleTriggerEvent::TappedForMana);
    }
    if (event_text.starts_with("when you cast") || event_text.starts_with("whenever you cast"))
        && event_text.contains("spell")
    {
        return Some(OracleTriggerEvent::CastsSpell { this_spell: false });
    }
    if event_text.contains("land")
        && event_text.contains("enters")
        && (event_text.contains("you control") || event_text.contains("under your control"))
    {
        return Some(OracleTriggerEvent::LandEnters(PlayerScope::You));
    }
    if event_text.contains("attack") {
        if event_text.contains("you attack") || event_text.contains("one or more creatures attack")
        {
            return Some(OracleTriggerEvent::PlayerAttacks);
        }
        return Some(OracleTriggerEvent::Attacks(object_subject(event_text)));
    }
    if event_text.contains("dies") || event_text.contains("put into a graveyard") {
        return Some(OracleTriggerEvent::Dies(object_subject(event_text)));
    }
    if event_text.contains("enters") {
        return Some(OracleTriggerEvent::Enters(object_subject(event_text)));
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
    if !event_text.contains("charge counters on it") {
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

/// Parse trigger conditions and effects into syntax nodes for lowering.
pub(super) fn parse_oracle_triggered(
    source: String,
    event: OracleTriggerEvent,
) -> OracleTriggeredAbility {
    let lower = source.to_ascii_lowercase();
    let once_per_turn = lower.contains("only once each turn")
        || lower.contains("only once each of your turns")
        || lower.contains("triggers only once each turn");
    let resolution = trigger_resolution(&source);
    let (condition, resolution) = intervening_condition(resolution);
    let (resolution_payment, resolution) = resolution_payment(resolution);
    let effects = super::effects::parse_trigger_effects(resolution, &event);
    let is_mana_ability = matches!(event, OracleTriggerEvent::TappedForMana)
        && !lower.contains("target")
        && effects.iter().any(|effect| {
            matches!(
                effect,
                OracleEffect::Mana(_) | OracleEffect::ManaPerCounter(_)
            )
        })
        && !effects.iter().any(is_non_mana_effect);
    OracleTriggeredAbility {
        is_mana_ability,
        effects,
        once_per_turn,
        condition,
        event_filter: parse_event_filter(&source, &event),
        resolution_payment,
        event,
    }
}

/// Retain an intervening-if condition without parsing it as a free effect.
fn intervening_condition(resolution: &str) -> (Option<String>, &str) {
    if resolution.to_ascii_lowercase().starts_with("if ")
        && let Some((condition, body)) = resolution[3..].split_once(',')
    {
        return (Some(condition.trim().to_string()), body.trim());
    }
    (None, resolution)
}

/// Separate supported optional payments from the reward they control.
fn resolution_payment(resolution: &str) -> (Option<OracleResolutionPayment>, &str) {
    if matches!(
        super::behold::parse_effect(resolution),
        Some(OracleEffect::Behold { .. } | OracleEffect::SearchThenBeholdUntap { .. })
    ) {
        return (None, resolution);
    }
    let lower = resolution.to_ascii_lowercase();
    if let Some(index) = lower.find(" unless ") {
        let tax = lower[index + " unless ".len()..].trim_end_matches('.');
        let amount = tax
            .strip_prefix("that player pays ")
            .or_else(|| tax.strip_prefix("they pay "));
        let payment = amount
            .and_then(opponent_tax_amount)
            .map(OracleResolutionPayment::OpponentTax)
            .unwrap_or_else(|| OracleResolutionPayment::Unsupported(tax.to_string()));
        return (Some(payment), resolution[..index].trim());
    }
    for (prefix, controller) in [("you may pay ", true), ("that player may pay ", false)] {
        if !lower.starts_with(prefix) {
            continue;
        }
        let Some((amount, remainder)) = lower[prefix.len()..].split_once('.') else {
            return (
                Some(OracleResolutionPayment::Unsupported(resolution.to_string())),
                resolution,
            );
        };
        let effect_prefix = if controller {
            "if you do, "
        } else {
            "if the player doesn't, "
        };
        let body = remainder.trim_start();
        let body = body.strip_prefix(effect_prefix).or_else(|| {
            (!controller)
                .then(|| body.strip_prefix("if they don't, "))
                .flatten()
        });
        if let Some(body) = body {
            let payment = if controller {
                fixed_payment_cost(amount).map(OracleResolutionPayment::PayMana)
            } else {
                opponent_tax_amount(amount).map(OracleResolutionPayment::OpponentTax)
            };
            let offset = resolution.len() - body.len();
            return (
                Some(
                    payment.unwrap_or_else(|| {
                        OracleResolutionPayment::Unsupported(amount.to_string())
                    }),
                ),
                &resolution[offset..],
            );
        }
        return (
            Some(OracleResolutionPayment::Unsupported(resolution.to_string())),
            resolution,
        );
    }
    if lower.contains("pay ") || lower.contains("if you do") || lower.contains("if you don't") {
        return (
            Some(OracleResolutionPayment::Unsupported(resolution.to_string())),
            resolution,
        );
    }
    (None, resolution)
}

/// Parse a complete fixed mana payment without accepting variable symbols.
fn fixed_payment_cost(amount: &str) -> Option<super::super::model::Cost> {
    let amount = amount.trim();
    if !amount.starts_with('{')
        || !amount.ends_with('}')
        || !amount
            .chars()
            .all(|ch| "{}wubrgcs/p0123456789".contains(ch))
    {
        return None;
    }
    let cost = super::cost::parse_activation_cost(amount);
    (!cost.unsupported).then_some(cost)
}

/// Read a fixed generic tax or a tax equal to the source's power.
fn opponent_tax_amount(amount: &str) -> Option<OracleTaxAmount> {
    if amount.trim() == "{x}, where x is this permanent's power" {
        return Some(OracleTaxAmount::SourcePower);
    }
    let amount = amount.trim().strip_prefix('{')?.strip_suffix('}')?;
    let generic = amount.parse::<u32>().ok()?;
    Some(OracleTaxAmount::Mana(super::super::model::Cost {
        generic,
        ..Default::default()
    }))
}

/// Preserve the entire object filter or mark its unmodeled wording inert.
fn parse_event_filter(source: &str, event: &OracleTriggerEvent) -> OracleEventFilter {
    let lower = strip_ability_word(source).to_ascii_lowercase();
    let event_text = event_prefix(&lower);
    let mut filter = OracleEventFilter::default();
    let object = match event {
        OracleTriggerEvent::CastsSpell { this_spell: false } => event_text
            .strip_prefix("whenever you cast ")
            .or_else(|| event_text.strip_prefix("when you cast ")),
        OracleTriggerEvent::OpponentSpellCast { first, .. } => {
            filter.first_each_turn = *first;
            event_text.strip_prefix("whenever an opponent casts ")
        }
        OracleTriggerEvent::LandEnters(PlayerScope::You) => {
            let body = event_text
                .strip_prefix("whenever ")
                .or_else(|| event_text.strip_prefix("when "));
            if !matches!(
                body,
                Some(
                    "a land you control enters"
                        | "a land you control enters the battlefield"
                        | "a land enters under your control"
                        | "a land enters the battlefield under your control"
                )
            ) {
                filter.unsupported = Some(event_text.to_string());
            }
            return filter;
        }
        OracleTriggerEvent::PlayerAttacks => {
            if !matches!(event_text, "whenever you attack" | "when you attack") {
                filter.unsupported = Some(event_text.to_string());
            }
            return filter;
        }
        OracleTriggerEvent::AnyOf(_)
        | OracleTriggerEvent::Enters(_)
        | OracleTriggerEvent::Dies(_)
        | OracleTriggerEvent::Attacks(_)
        | OracleTriggerEvent::CombatDamageToPlayer(_) => object_filter_phrase(event_text, event),
        _ => return filter,
    };
    let Some(mut object) = object else {
        filter.unsupported = Some(event_text.to_string());
        return filter;
    };
    if matches!(
        object,
        "this"
            | "this permanent"
            | "this creature"
            | "this artifact"
            | "this enchantment"
            | "this land"
    ) {
        return filter;
    }
    if let Some((head, tail)) = object.split_once(" spell") {
        if !matches!(tail, "" | " each turn") {
            filter.unsupported = Some(event_text.to_string());
        }
        object = head;
    }
    object = object.trim_end_matches(" you control");
    append_object_filter(&mut filter, object, event_text);
    filter
}

/// Share type and subtype requirements between events and battlefield selectors.
pub(super) fn append_object_filter(
    filter: &mut OracleEventFilter,
    object: &str,
    unsupported: &str,
) {
    for word in object.split_whitespace() {
        match word {
            "a" | "an" | "another" | "their" | "permanent" => {}
            "first" if filter.first_each_turn => {}
            "nontoken" => filter.nontoken = true,
            "noncreature" => filter.noncreature = true,
            "artifact" | "creature" | "enchantment" | "instant" | "sorcery" | "land"
            | "planeswalker" | "battle" | "legendary" => filter.card_types.push(word.to_string()),
            "or" | "and" | "with" | "one" | "more" | "other" | "first" | "second" | "white"
            | "blue" | "black" | "red" | "green" | "colorless" | "multicolored" | "historic"
            | "from" | "nonland" | "colored" | "modified" | "tapped" | "untapped" | "enchanted"
            | "equipped" | "attacking" | "your" => {
                filter.unsupported = Some(unsupported.to_string())
            }
            subtype if subtype.chars().all(char::is_alphabetic) => {
                filter.subtypes.push(subtype.to_string())
            }
            _ => filter.unsupported = Some(unsupported.to_string()),
        }
    }
}

/// Validate the full object event so suffix restrictions cannot disappear.
fn object_filter_phrase<'a>(event_text: &'a str, event: &OracleTriggerEvent) -> Option<&'a str> {
    let body = event_text
        .strip_prefix("whenever ")
        .or_else(|| event_text.strip_prefix("when "))?;
    let (index, marker) = [" enters", " dies", " attacks", " deals", " is put"]
        .into_iter()
        .filter_map(|marker| body.find(marker).map(|index| (index, marker)))
        .min_by_key(|(index, _)| *index)?;
    let object = &body[..index];
    let suffix = &body[index + 1..];
    let supported = match event {
        OracleTriggerEvent::AnyOf(events) => {
            matches!(
                events.as_slice(),
                [
                    OracleTriggerEvent::Enters(ObjectSubject::ThisPermanent),
                    OracleTriggerEvent::Dies(ObjectSubject::ThisPermanent)
                        | OracleTriggerEvent::Attacks(ObjectSubject::ThisPermanent)
                ]
            ) && matches!(suffix, "enters or dies" | "enters or attacks")
        }
        OracleTriggerEvent::Enters(ObjectSubject::ThisPermanent) => {
            matches!(suffix, "enters" | "enters the battlefield")
        }
        OracleTriggerEvent::Enters(_) => {
            matches!(
                suffix,
                "enters under your control" | "enters the battlefield under your control"
            ) || object.ends_with(" you control")
                && matches!(suffix, "enters" | "enters the battlefield")
        }
        OracleTriggerEvent::Dies(_) => {
            marker == " dies" && suffix == "dies"
                || matches!(
                    suffix,
                    "is put into a graveyard from the battlefield"
                        | "is put into a graveyard from battlefield"
                )
        }
        OracleTriggerEvent::Attacks(_) => suffix == "attacks",
        OracleTriggerEvent::CombatDamageToPlayer(_) => matches!(
            suffix,
            "deals combat damage to a player" | "deals combat damage to an opponent"
        ),
        _ => false,
    };
    supported.then_some(object)
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
