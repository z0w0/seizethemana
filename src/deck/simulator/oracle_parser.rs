//! Grammar parser for the supported subset of Magic Oracle text.

/// Parse activated-ability costs and timing restrictions.
mod activation;
/// Parse mana costs and mana yields from Oracle wording.
pub(crate) mod cost;
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

use effects::parse_oracle_effects;
use keywords::{add_keyword, keyword_line_list, parse_oracle_keyword, standalone_keyword};
use statements::{
    is_spell_statement, is_static_statement, is_trigger_start, oracle_segments, strip_ability_word,
    unsupported_kind,
};
use triggers::{parse_oracle_saga_chapter, parse_oracle_trigger_event, parse_oracle_triggered};

/// Keep text-number helpers available to the parser grammar modules.
pub(super) use text::{amount_after, draw_amount};

/// Parse one activated-ability statement into typed syntax nodes.
pub(super) use activation::parse_oracle_activated_ability;

/// Parse Oracle text and its card-data keyword list into typed syntax nodes.
pub fn parse_oracle_text(oracle_text: &str, keywords: &[String]) -> OracleCard {
    let mut card = OracleCard {
        keywords: keywords
            .iter()
            .map(|keyword| parse_oracle_keyword(keyword))
            .collect(),
        abilities: Vec::new(),
        spell_data: OracleSpellData::default(),
        static_data: OracleStaticData::default(),
        land: OracleLand::default(),
        stations: OracleStations::default(),
        library_graveyard_trigger: None,
    };

    for source in oracle_segments(oracle_text, &card.keywords) {
        if let Some(keywords) = keyword_line_list(&source, &card.keywords) {
            for keyword in keywords {
                add_keyword(&mut card.keywords, keyword);
            }
            continue;
        }
        if let Some(keyword) = standalone_keyword(&source, &card.keywords) {
            add_keyword(&mut card.keywords, parse_oracle_keyword(&keyword));
            continue;
        }
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
                    effects: parse_oracle_effects(&source),
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
pub fn parse_oracle_card(row: &CardRow) -> OracleCard {
    let keyword_names = parse_card_keywords(&row.keywords);
    let parser_text = normalize_land_source_name(&row.oracle_text, &row.name);
    let parser_text = normalize_trigger_source_name(&parser_text, &row.name);
    let mut card = parse_oracle_text(&parser_text, &keyword_names);
    card.spell_data = parse_spell_data(&row.oracle_text, &row.name, &row.mana_cost);
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

/// Normalize a card's printed name only in its own land-fetch cost.
fn normalize_land_source_name(oracle_text: &str, name: &str) -> String {
    oracle_text
        .lines()
        .map(|line| {
            if !line.to_ascii_lowercase().contains("search your library") {
                return line.to_string();
            }
            let lower = line.to_ascii_lowercase();
            let phrase = format!("sacrifice {}", name.to_ascii_lowercase());
            let Some(index) = lower.find(&phrase) else {
                return line.to_string();
            };
            let mut normalized = line.to_string();
            normalized.replace_range(index..index + phrase.len(), "Sacrifice this land");
            normalized
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace the source card's printed name with "this" in trigger event
/// clauses so event subjects keep their object relationship.
fn normalize_trigger_source_name(oracle_text: &str, name: &str) -> String {
    let name = name.to_ascii_lowercase();
    oracle_text
        .lines()
        .map(|line| {
            let Some(event_end) = line.find(',') else {
                return line.to_string();
            };
            if !is_trigger_start(line) {
                return line.to_string();
            }
            let lower_event = line[..event_end].to_ascii_lowercase();
            let Some(position) = lower_event.find(&name) else {
                return line.to_string();
            };
            format!(
                "{}this{}",
                &line[..position],
                &line[position + name.len()..]
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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

/// Parse the card's extra cast costs and one-shot spell flags into typed
/// `OracleSpellData`. This is the single text pass for the spell fields
/// `cast_riders` and `card_lower` used to re-scan raw text for.
pub(super) fn parse_spell_data(oracle_text: &str, name: &str, mana_cost: &str) -> OracleSpellData {
    let name_lower = name.to_ascii_lowercase();
    let clauses: Vec<String> = statements::oracle_segments(oracle_text, &[])
        .into_iter()
        .map(|clause| clause.to_ascii_lowercase())
        .collect();
    let has_clause = |matches: &dyn Fn(&str) -> bool| clauses.iter().any(|clause| matches(clause));
    OracleSpellData {
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
        search_after_sacrifice: has_clause(&|clause| {
            clause.contains("sacrificed creature's mana value")
                && clause.contains("search your library for a creature card")
        }),
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
        counters_on_cast: clauses
            .iter()
            .filter(|clause| clause.contains("put ") && clause.contains("charge counter"))
            .map(|clause| charge_counter_amount(clause))
            .max()
            .unwrap_or(0),
    }
}

/// Parse a charge-counter count from one cast-effect clause.
fn charge_counter_amount(clause: &str) -> u32 {
    let Some((_, tail)) = clause.split_once("put ") else {
        return 0;
    };
    tail.trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| {
            [
                ("five", 5),
                ("four", 4),
                ("three", 3),
                ("two", 2),
                ("one", 1),
            ]
            .into_iter()
            .find_map(|(word, count)| tail.starts_with(word).then_some(count))
            .unwrap_or(0)
        })
}

/// Parse the card's static whole-card flags into typed `OracleStaticData`.
/// This is the single text pass for the fields `static_flags` and
/// `card_lower` used to re-scan raw text for.
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
    OracleStaticData {
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
        landfall: abilities.iter().any(|ability| {
            matches!(
                ability,
                OracleAbility::Triggered(trigger)
                    if matches!(trigger.event, OracleTriggerEvent::LandEnters(_))
            )
        }),
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
        buffs_battlefield_on_entry: clauses.iter().any(|clause| {
            clause.contains("+x/+x")
                && (clause.contains("where x is") || clause.contains("equal to the number"))
        }),
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
    if !lower.contains("additional cost") {
        return None;
    }
    let sacrifice_creatures =
        if lower.contains("sacrifice a creature") || lower.contains("sacrifice any number") {
            1
        } else {
            0
        };
    let discard_cards = additional_cost_discards(lower);
    let pay_life = additional_cost_life(lower);
    (sacrifice_creatures + discard_cards + pay_life > 0).then_some(OracleAdditionalCost {
        sacrifice_creatures,
        discard_cards,
        pay_life,
    })
}

/// Count cards required by an additional-cost discard clause.
fn additional_cost_discards(lower: &str) -> u32 {
    let Some(discard) = lower
        .split("additional cost")
        .nth(1)
        .and_then(|clause| clause.split("discard ").nth(1))
    else {
        return 0;
    };
    for (word, count) in [("two", 2), ("three", 3), ("four", 4), ("a card", 1)] {
        if discard.starts_with(word) {
            return count;
        }
    }
    discard
        .split_whitespace()
        .next()
        .and_then(|count| count.parse::<u32>().ok())
        .unwrap_or(0)
}

/// Parse the life payment from an additional-cost clause.
fn additional_cost_life(lower: &str) -> u32 {
    if !lower.contains("additional cost") {
        return 0;
    }
    lower
        .split("pay ")
        .nth(1)
        .and_then(|rest| rest.split([' ', '.', ',']).next().map(str::to_string))
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0)
}

/// Parse an exile-from-hand alternative cast cost (CR 118.9).
fn parse_alternative_cost(lower: &str) -> Option<super::model::AlternativeCastCost> {
    if !lower.contains("rather than pay this spell's mana cost") || !lower.contains("exile ") {
        return None;
    }
    let color = [
        ("white", ManaColor::White),
        ("blue", ManaColor::Blue),
        ("black", ManaColor::Black),
        ("red", ManaColor::Red),
        ("green", ManaColor::Green),
    ]
    .into_iter()
    .find_map(|(word, color)| lower.contains(word).then_some(color));
    Some(super::model::AlternativeCastCost {
        filter: super::model::CardFilter { color },
        count: amount_after(lower, "exile "),
        payoff: if lower.contains("gain x life") {
            super::model::AlternativeCostPayoff::GainLifeEqualToExiledManaValue
        } else {
            super::model::AlternativeCostPayoff::None
        },
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
