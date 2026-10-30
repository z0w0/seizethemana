//! Grammar parser for the supported subset of Magic Oracle text.

/// Parse activated-ability costs and timing restrictions.
mod activation;
/// Parse complete Behold choices, costs, and conditional instructions.
mod behold;
/// Parse mana costs and mana yields from Oracle wording.
pub(crate) mod cost;
/// Parse complete flat generic spell-cost reductions.
mod cost_reduction;
/// Parse effects and their supported clause shapes.
mod effects;
/// Parse equipment and creature power/toughness clauses.
pub(crate) mod equipment;
/// Add parsed keyword abilities that have no full Oracle line.
mod keyword_synthesis;
/// Parse standalone keyword abilities.
pub(crate) mod keywords;
/// Parse land entries, search abilities, and mana modes.
pub(crate) mod land;
/// Parse searches that assign more than one result.
mod multi_search;
/// Parse complete draw, life-loss, and top-of-library choices.
mod selection;
/// Split Oracle text and classify ability statements.
mod statements;
/// Parse station reminder striations.
mod stations;
/// Number-word and numeral extraction from Oracle text.
mod text;
/// Parse triggered abilities and Saga chapter statements.
mod triggers;

use super::model::{DrawMatch, ManaColor};
use super::oracle_ast::*;
use crate::db::CardRow;

use keywords::{add_keyword, keyword_line_list, parse_oracle_keyword, standalone_keyword};

/// Shared battlefield selectors, written costs, and scoped static grammar.
mod tribal;
/// Complete creature-token, scaled-mana, and selected-untap grammar.
mod tribal_effects;
use statements::{
    is_spell_statement, is_static_statement, oracle_segments, strip_ability_word, unsupported_kind,
};
use triggers::{parse_oracle_saga_chapter, parse_oracle_trigger_event, parse_oracle_triggered};

/// Keep text-number helpers available to the parser grammar modules.
pub(super) use text::amount_after;
/// Expose the bounded text helper only for its existing unit tests.
#[cfg(test)]
pub(super) use text::draw_amount;

/// Parse one activated-ability statement into typed syntax nodes.
pub(super) use activation::parse_oracle_activated_ability;

/// Parse raw text without source-name metadata for grammar tests.
#[cfg(test)]
pub fn parse_oracle_text(oracle_text: &str, keywords: &[String]) -> OracleCard {
    parse_oracle_source(oracle_text, keywords, None, None)
}

/// Parse original ability segments while keeping their source provenance.
fn parse_oracle_source(
    oracle_text: &str,
    keywords: &[String],
    name: Option<&str>,
    x_class: Option<super::model::XClass>,
) -> OracleCard {
    let mut card = OracleCard {
        keywords: keywords
            .iter()
            .map(|keyword| parse_oracle_keyword(keyword))
            .collect(),
        abilities: Vec::new(),
        ability_sources: Vec::new(),
        spell_data: OracleSpellData::default(),
        static_data: OracleStaticData::default(),
        land: OracleLand::default(),
        stations: OracleStations::default(),
        library_graveyard_trigger: None,
    };

    let mut source_cursor = 0;
    for source in oracle_segments(oracle_text, &card.keywords) {
        let original = ability_source(oracle_text, &source, &mut source_cursor);
        let source = name.map_or_else(
            || source.clone(),
            |name| normalize_source_name(&source, name),
        );
        let keyword_entries = keyword_line_list(&source, &card.keywords).or_else(|| {
            standalone_keyword(&source, &card.keywords)
                .map(|keyword| vec![parse_oracle_keyword(&keyword)])
        });
        if let Some(keywords) = keyword_entries {
            let unsupported_warp = keywords
                .iter()
                .any(|keyword| keyword.name == OracleKeywordName::Warp);
            for keyword in keywords {
                add_keyword(&mut card.keywords, keyword);
            }
            if unsupported_warp {
                card.ability_sources.push(original);
                card.abilities
                    .push(OracleAbility::Unsupported(OracleUnsupported {
                        kind: UnsupportedAbilityKind::Unknown,
                        source,
                    }));
            }
            continue;
        }
        card.ability_sources.push(original);
        if let Some(chapter) = parse_oracle_saga_chapter(&source) {
            card.abilities.push(OracleAbility::SagaChapter(chapter));
        } else if let Some(ability) = parse_oracle_activated_ability(strip_ability_word(&source)) {
            card.abilities.push(OracleAbility::Activated(ability));
        } else if let Some(event) = parse_oracle_trigger_event(&source) {
            card.abilities
                .push(OracleAbility::Triggered(parse_oracle_triggered(
                    source, event,
                )));
        } else if is_static_statement(&source) {
            card.abilities
                .push(OracleAbility::Static(statements::parse_oracle_static(
                    &source,
                )));
        } else if is_spell_statement(&source) {
            card.abilities
                .push(OracleAbility::Spell(OracleSpellAbility {
                    effects: parse_spell_resolution(&source, x_class),
                }));
        } else {
            card.abilities
                .push(OracleAbility::Unsupported(OracleUnsupported {
                    kind: unsupported_kind(&source),
                    source,
                }));
        }
    }
    card
}

/// Parse one card row into typed Oracle syntax and card-level Oracle data.
///
/// Unsupported nodes keep their source wording for support reporting. Parsed
/// syntax alone does not establish runtime support.
pub fn parse_oracle_card(row: &CardRow) -> OracleCard {
    let keyword_names = parse_card_keywords(&row.keywords);
    let spell_data = parse_spell_data(&row.oracle_text, &row.name, &row.mana_cost);
    let mut card = parse_oracle_source(
        &row.oracle_text,
        &keyword_names,
        Some(&row.name),
        spell_data.x_class,
    );
    card.spell_data = spell_data;
    card.spell_data.search_after_sacrifice = linked_sacrifice_search(&card);
    card.static_data = parse_static_data(
        &row.oracle_text,
        &card.abilities,
        &card.keywords,
        &row.type_line,
    );
    card.land = land::parse_land_data(&row.oracle_text, &card.abilities);
    card.stations = stations::parse_stations(&row.oracle_text, &row.type_line);
    card.library_graveyard_trigger = parse_library_graveyard_trigger(&row.oracle_text);
    keyword_synthesis::synthesize_keyword_abilities(&mut card);
    card
}

/// Link one spell-resolution search to its required creature sacrifice cost.
fn linked_sacrifice_search(card: &OracleCard) -> Option<super::model::SacrificeSearch> {
    if card.spell_data.unsupported_additional_cost
        || card.spell_data.additional_cost?.sacrifice_creatures != 1
    {
        return None;
    }
    let mut rules = card
        .abilities
        .iter()
        .flat_map(|ability| match ability {
            OracleAbility::Spell(spell) => spell.effects.as_slice(),
            _ => &[],
        })
        .filter_map(|effect| match effect {
            OracleEffect::SacrificeSearch(rule) => Some(*rule),
            _ => None,
        });
    let rule = rules.next()?;
    rules.next().is_none().then_some(rule)
}

/// Keep exact original text even when a segment joins several source lines.
fn ability_source(text: &str, segment: &str, cursor: &mut usize) -> OracleAbilitySource {
    let mut end = *cursor;
    let mut start = None;
    for word in segment.split_whitespace() {
        let Some(offset) = text[end..].find(word) else {
            return OracleAbilitySource {
                text: segment.to_string(),
                span: None,
            };
        };
        let position = end + offset;
        if start.is_some() && !text[end..position].chars().all(char::is_whitespace) {
            return OracleAbilitySource {
                text: segment.to_string(),
                span: None,
            };
        }
        start.get_or_insert(position);
        end = position + word.len();
    }
    let span = start.map(|start| OracleSourceSpan { start, end });
    *cursor = end;
    OracleAbilitySource {
        text: span.map_or_else(
            || segment.to_string(),
            |span| text[span.start..span.end].to_string(),
        ),
        span,
    }
}

/// Normalize full and abbreviated source names without granting mechanics.
fn normalize_source_name(source: &str, name: &str) -> String {
    let mut normalized = source.to_string();
    let short_name = name.split_once(',').map(|(short, _)| short);
    for candidate in std::iter::once(name).chain(short_name) {
        if candidate.is_empty() {
            continue;
        }
        let lower = normalized.to_ascii_lowercase();
        let candidate = candidate.to_ascii_lowercase();
        for (start, _) in lower
            .match_indices(&candidate)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let end = start + candidate.len();
            let boundary = |ch: char| !ch.is_alphanumeric() && ch != '_';
            if normalized[..start].chars().next_back().is_none_or(boundary)
                && normalized[end..].chars().next().is_none_or(boundary)
                && is_source_reference(&normalized[..start], &normalized[end..])
            {
                normalized.replace_range(start..end, "this permanent");
            }
        }
    }
    normalized
}

/// Distinguish self references from ordinary words that happen to match a name.
fn is_source_reference(before: &str, after: &str) -> bool {
    let before = before.to_ascii_lowercase();
    let after = after.to_ascii_lowercase();
    [
        "when ",
        "whenever ",
        "sacrifice ",
        "untap ",
        "exile ",
        "on ",
        "from ",
    ]
    .iter()
    .any(|prefix| before.ends_with(prefix))
        || ["'s power", "'s toughness", "'s mana value"]
            .iter()
            .any(|suffix| after.starts_with(suffix))
        || before.is_empty()
            && [" enters", " deals", " has", " doesn't", " does not", " is "]
                .iter()
                .any(|suffix| after.starts_with(suffix))
}

/// Read keyword arrays from storage and whitespace lists from test fixtures.
fn parse_card_keywords(source: &str) -> Vec<String> {
    let source = source.trim();
    if source.starts_with('[') {
        serde_json::from_str(source).expect("stored Scryfall keywords must be a JSON array")
    } else {
        source
            .split(|character: char| character == ',' || character.is_whitespace())
            .map(str::trim)
            .filter(|keyword| !keyword.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// Parse extra cast costs and one-shot spell flags for explicit runtime lowering.
pub(super) fn parse_spell_data(oracle_text: &str, name: &str, mana_cost: &str) -> OracleSpellData {
    let name_lower = name.to_ascii_lowercase();
    let clauses: Vec<String> = statements::oracle_segments(oracle_text, &[])
        .into_iter()
        .map(|clause| clause.to_ascii_lowercase())
        .collect();
    let has_clause = |matches: &dyn Fn(&str) -> bool| clauses.iter().any(|clause| matches(clause));
    OracleSpellData {
        unsupported_additional_cost: clauses.iter().any(|clause| {
            clause.starts_with("as an additional cost to cast this spell")
                && parse_additional_cost(clause).is_none()
                && behold::additional_cost_alternative(clause)
                    .is_none_or(|alternative| alternative.spec.count != 1)
        }),
        behold_alternative: clauses
            .iter()
            .find_map(|clause| behold::additional_cost_alternative(clause)),
        additional_cost: clauses
            .iter()
            .find_map(|clause| parse_additional_cost(clause)),
        alternative_cost: clauses
            .iter()
            .find_map(|clause| parse_alternative_cost(clause)),
        reveal_rule: clauses.iter().find_map(|clause| parse_reveal_rule(clause)),
        exiles_on_resolve: has_clause(&|clause| {
            clause.contains("exile this spell") || clause.contains(&format!("exile {name_lower}"))
        }),
        search_after_sacrifice: None,
        graveyard_creature_exchange: has_clause(&|clause| {
            clause.contains("exiles all creature cards from their graveyard")
                && clause.contains("sacrifices all creatures they control")
                && clause.contains("puts all cards they exiled this way onto the battlefield")
        }),
        grants_flashback: has_clause(&|clause| {
            clause.contains("instant and sorcery cards in your graveyard gain flashback")
                || clause.contains("instant and sorcery card in your graveyard gains flashback")
        }),
        grants_escape: has_clause(&|clause| {
            (clause.contains("nonland cards in your graveyard have escape")
                || clause.contains("each nonland card in your graveyard has escape"))
                && clause.contains("exile three other cards from your graveyard")
        }),
        x_class: if mana_cost.to_ascii_uppercase().contains("{X}") {
            clauses.iter().find_map(|clause| parse_x_class(clause))
        } else {
            None
        },
    }
}

/// Parse whole-card static rules for explicit runtime lowering.
pub(super) fn parse_static_data(
    oracle_text: &str,
    abilities: &[OracleAbility],
    keywords: &[OracleKeyword],
    type_line: &str,
) -> OracleStaticData {
    let clauses: Vec<String> = statements::oracle_segments(oracle_text, &[])
        .into_iter()
        .map(|clause| clause.to_ascii_lowercase())
        .collect();
    let has_clause = |pattern: &str| clauses.iter().any(|clause| clause.contains(pattern));
    let has_removal_shape = clauses.iter().any(|clause| {
        clause.contains("destroy target")
            || clause.contains("exile target")
            || clause.contains("counter target")
            || clause.contains("return target")
                && (clause.contains("to its owner's hand")
                    || clause.contains("to their owner's hand"))
    });
    let has_damage_removal = abilities.iter().any(|ability| match ability {
        OracleAbility::Spell(spell) => spell.effects.iter().any(|effect| {
            matches!(
                effect,
                OracleEffect::Damage {
                    amount: 2..,
                    target: DamageTarget::Permanent | DamageTarget::AnyTarget,
                    ..
                }
            )
        }),
        _ => false,
    });
    let sweeps = (type_line.contains("Instant") || type_line.contains("Sorcery"))
        && clauses.iter().any(|clause| {
            clause.contains("destroy all")
                || clause.contains("exile all")
                || clause.contains("return all")
                || clause.contains("sacrifice all")
                || clause.contains("each creature") && clause.contains("-x/-x")
        });
    let cumulative_upkeep = cumulative_upkeep_cost(oracle_text);
    OracleStaticData {
        cumulative_upkeep,
        unsupported_cumulative_upkeep: oracle_text
            .to_ascii_lowercase()
            .contains("cumulative upkeep")
            && cumulative_upkeep.is_none(),
        starts_on_battlefield: has_clause("begin the game with it on the battlefield"),
        no_max_hand_size: has_clause("no maximum hand size"),
        draw_per_controlled_permanent: clauses.iter().find_map(|clause| scaling_draw_match(clause)),
        enter_counters: clauses
            .iter()
            .map(|clause| land::parse_enter_counters(clause))
            .find(|counters| {
                matches!(
                    counters,
                    super::model::EnterCounters::XCharge | super::model::EnterCounters::XPlus1
                )
            })
            .or_else(|| {
                clauses
                    .iter()
                    .map(|clause| land::parse_enter_counters(clause))
                    .find(|counters| *counters != super::model::EnterCounters::None)
            })
            .unwrap_or_default(),
        doesnt_untap: abilities.iter().any(|ability| {
            matches!(
                ability,
                OracleAbility::Static(static_ability)
                    if static_ability.effects.iter().any(|effect| {
                        matches!(effect, OracleStaticEffect::DoesntUntap)
                    })
            )
        }),
        has_flash: keywords
            .iter()
            .any(|keyword| keyword.name == OracleKeywordName::Flash),
        saddled_buff: clauses
            .iter()
            .find_map(|clause| equipment::parse_saddled_buff(clause)),
        equipment: type_line
            .contains("Equipment")
            .then(|| equipment::parse_equipment(&oracle_text.to_ascii_lowercase()))
            .flatten(),
        mills_opponent: clauses.iter().any(|clause| {
            clause.contains("target player mills")
                || clause.contains("target opponent") && clause.contains("mill")
                || clause.contains("each opponent mills")
        }),
        sweeps,
        is_interaction: (type_line.contains("Instant") || type_line.contains("Sorcery"))
            && (has_removal_shape || has_damage_removal || sweeps),
    }
}

/// Recognize generic-only cumulative upkeep; other payments stay unsupported.
fn cumulative_upkeep_cost(oracle_text: &str) -> Option<super::model::Cost> {
    oracle_text.lines().find_map(|line| {
        let lower = line.trim().to_ascii_lowercase();
        let amount = lower
            .strip_prefix("cumulative upkeep ")?
            .split('(')
            .next()?
            .trim();
        let generic = amount.strip_prefix('{')?.strip_suffix('}')?.parse().ok()?;
        Some(super::model::Cost {
            generic,
            ..Default::default()
        })
    })
}

/// Parse the action on a library-to-graveyard trigger from its statement.
fn parse_library_graveyard_trigger(oracle_text: &str) -> Option<OracleLibraryGraveyardEffect> {
    statements::oracle_segments(oracle_text, &[])
        .iter()
        .map(|clause| clause.to_ascii_lowercase())
        .find(|clause| clause.contains("put into your graveyard from your library"))
        .and_then(|clause| {
            if clause.contains("put it onto the battlefield") {
                Some(OracleLibraryGraveyardEffect::ReturnToBattlefield)
            } else if clause.contains("you gain") {
                let amount = amount_after(&clause, "loses ");
                (amount > 0).then_some(OracleLibraryGraveyardEffect::DrainAndGain(amount))
            } else {
                None
            }
        })
}

/// Match a draw effect that scales with a controlled permanent class.
fn scaling_draw_match(lower: &str) -> Option<DrawMatch> {
    if !(lower.contains("draw") && lower.contains("for each")) {
        return None;
    }
    if lower.contains("for each enchantment you control") {
        Some(DrawMatch::Enchantments)
    } else if lower.contains("for each artifact you control") {
        Some(DrawMatch::Artifacts)
    } else if lower.contains("for each land you control") {
        Some(DrawMatch::Lands)
    } else if lower.contains("for each creature you control") {
        Some(DrawMatch::Creatures)
    } else {
        None
    }
}

/// Parse an additional cast cost (CR 118.8) into its typed components.
fn parse_additional_cost(lower: &str) -> Option<OracleAdditionalCost> {
    let cost = lower
        .split('.')
        .next()?
        .strip_prefix("as an additional cost to cast this spell, ")?;
    let mut parsed = OracleAdditionalCost::default();
    for component in cost.split(" and ") {
        match component {
            "sacrifice a creature" => parsed.sacrifice_creatures += 1,
            "sacrifice an artifact or creature" => parsed.sacrifice_artifacts_or_creatures += 1,
            component if component.starts_with("discard ") => {
                let amount = component.strip_prefix("discard ")?;
                let (count, cards) = amount.split_once(' ')?;
                if !matches!(cards, "card" | "cards") {
                    return None;
                }
                parsed.discard_cards += effects::parse_oracle_number_word(count)?;
            }
            component if component.starts_with("pay ") => {
                let amount = component.strip_prefix("pay ")?.strip_suffix(" life")?;
                parsed.pay_life += effects::parse_oracle_number_word(amount)?;
            }
            _ => return None,
        }
    }
    Some(parsed)
}

/// Exclude a fully parsed cast cost without excluding any linked effect.
fn parse_spell_resolution(
    source: &str,
    x_class: Option<super::model::XClass>,
) -> Vec<OracleEffect> {
    if source
        .to_ascii_lowercase()
        .starts_with("as an additional cost to cast this spell")
    {
        if parse_additional_cost(&source.to_ascii_lowercase()).is_none()
            && behold::additional_cost_alternative(source).is_none()
        {
            return vec![OracleEffect::Unsupported(source.to_string())];
        }
        let resolution = source
            .split_once('.')
            .map_or("", |(_, resolution)| resolution.trim());
        return if resolution.is_empty() {
            Vec::new()
        } else {
            effects::parse_spell_effects(resolution, x_class)
        };
    }
    effects::parse_spell_effects(source, x_class)
}

/// Parse an exile-from-hand alternative cast cost (CR 118.9).
fn parse_alternative_cost(lower: &str) -> Option<super::model::AlternativeCastCost> {
    let cost = lower
        .trim_end_matches('.')
        .strip_prefix("you may exile ")?
        .strip_suffix(" from your hand rather than pay this spell's mana cost")?;
    let colors = [
        ("white", ManaColor::White),
        ("blue", ManaColor::Blue),
        ("black", ManaColor::Black),
        ("red", ManaColor::Red),
        ("green", ManaColor::Green),
    ];
    let mut parts = cost.split_whitespace();
    let count = effects::parse_oracle_number_word(parts.next()?)?;
    let next = parts.next()?;
    let color = colors
        .iter()
        .find_map(|(word, color)| (*word == next).then_some(*color));
    let card_word = if color.is_some() { parts.next()? } else { next };
    if !matches!(card_word, "card" | "cards") || parts.next().is_some() {
        return None;
    }
    Some(super::model::AlternativeCastCost {
        filter: super::model::CardFilter { color },
        count,
        payoff: super::model::AlternativeCostPayoff::None,
    })
}

/// Parse a repeated reveal effect that charges life equal to mana value.
fn parse_reveal_rule(lower: &str) -> Option<super::model::RevealRule> {
    (lower.contains("reveal the top card of your library")
        && lower.contains("put that card into your hand")
        && lower.contains("lose life equal to its mana value")
        && lower.contains("repeat this process"))
    .then_some(super::model::RevealRule {
        destination: super::model::RevealDestination::Hand,
        life_loss: super::model::RevealLifeLoss::ManaValue,
    })
}

/// The X-cost effect class for an `{X}` spell, or None when the effect
/// does not parse to a modeled X shape.
fn parse_x_class(lower: &str) -> Option<super::model::XClass> {
    use super::model::XClass;
    if (lower.contains("loses x life")
        || lower.contains("each opponent loses x")
        || lower.contains("deals x damage"))
        && (lower.contains("target player") || lower.contains("opponent"))
    {
        Some(XClass::Drain)
    } else if lower.contains("draw x") || lower.contains("draws x") {
        Some(XClass::Draw)
    } else if lower.contains("mill x") {
        Some(XClass::Mill)
    } else if lower.contains("create x") && lower.contains("treasure token") {
        Some(XClass::Treasures)
    } else if lower.contains("create x") && lower.contains("token") {
        Some(XClass::Tokens)
    } else if lower.contains("reveal the top x")
        && lower.contains("permanent")
        && (lower.contains("put any number") || lower.contains("onto the battlefield"))
    {
        Some(XClass::RevealPermanents)
    } else if (lower.contains("enters with x") || lower.contains("the battlefield with x"))
        && lower.contains("counters")
    {
        Some(XClass::Counters)
    } else {
        None
    }
}
