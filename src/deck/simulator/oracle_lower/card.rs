//! Lower one card row and its parsed Oracle syntax into simulator data.

use super::super::super::stats::is_land;
use super::super::model::{Cost, SimCard, SimSpellData, SimStriation};
use super::super::oracle_ast::{OracleAbility, OracleCard, OracleKeywordName, OracleStaticEffect};
use super::cast_riders::{parse_cast_riders, parse_min_cost};
use super::static_flags::parse_static_flags;
use crate::db::CardRow;

/// Facts shared by the card-lowering steps.
struct CardShape {
    land: bool,
    is_mdfc_spell: bool,
    cast_face: bool,
    mythic: bool,
    cost: Cost,
}

/// Station tiers and non-tier abilities lowered for one card.
struct CardAbilities {
    striations: Vec<SimStriation>,
    is_station_card: bool,
    is_saga: bool,
}

/// Lower parsed Oracle syntax and card metadata into one simulated card.
pub(super) fn lower_oracle_card(row: &CardRow, oracle: &OracleCard) -> SimCard {
    let text = row.oracle_text.to_ascii_lowercase();
    let type_line = row.type_line.to_string();
    let shape = card_shape(row);
    let abilities = card_abilities(oracle, &type_line, &shape);
    let mut card = SimCard::default();

    lower_identity_fields(&mut card, row, oracle, &type_line, &shape, &abilities);
    lower_land_fields(&mut card, oracle, shape.land);
    lower_cast_fields(&mut card, row, oracle, &type_line, &shape);
    lower_classification_fields(&mut card, row, &text, &shape, abilities.is_station_card);
    lower_static_fields(&mut card, oracle, &type_line);
    lower_oracle_detail_fields(&mut card, row, oracle, &shape);

    card
}

/// Determine whether the row is a land, a modal land/spell, or a cast face.
fn card_shape(row: &CardRow) -> CardShape {
    let land = is_land(row);
    // Land/spell MDFC: one face is a Land, the other a castable spell.
    let is_mdfc_spell = row
        .type_line
        .split(" // ")
        .any(|face| face.split('—').next().unwrap_or("").contains("Land"))
        && row
            .type_line
            .split(" // ")
            .any(|face| !face.split('—').next().unwrap_or("").contains("Land"));
    // MDFC spell faces keep their cast cost; plain lands cost nothing.
    let cost = if land && !is_mdfc_spell {
        Cost::default()
    } else {
        super::super::oracle_parser::cost::parse_cost_faces(&row.mana_cost)
    };
    CardShape {
        land,
        is_mdfc_spell,
        cast_face: !land || is_mdfc_spell,
        mythic: row.rarity == "mythic",
        cost,
    }
}

/// Lower station tiers, Saga chapters, and runtime abilities for a card.
fn card_abilities(oracle: &OracleCard, type_line: &str, shape: &CardShape) -> CardAbilities {
    let abilities = oracle.runtime_abilities();
    let has_station_text = oracle.stations.has_station_ability;
    let mut striations = if has_station_text {
        super::lower_striations(&oracle.stations)
    } else {
        Vec::new()
    };
    let is_saga = type_line.contains("Saga") && !shape.land;
    if !abilities.is_empty() {
        striations.insert(
            0,
            SimStriation {
                at: 0,
                animate: false,
                abilities,
            },
        );
    }
    for (id, ability) in striations
        .iter_mut()
        .flat_map(|striation| striation.abilities.iter_mut())
        .enumerate()
    {
        ability.id = id;
    }

    CardAbilities {
        striations,
        is_station_card: has_station_text && oracle.stations.is_station_card,
        is_saga,
    }
}

/// Set identity and type fields that come directly from card metadata.
fn lower_identity_fields(
    card: &mut SimCard,
    row: &CardRow,
    oracle: &OracleCard,
    type_line: &str,
    shape: &CardShape,
    abilities: &CardAbilities,
) {
    card.name.clone_from(&row.name);
    card.cost.clone_from(&shape.cost);
    card.mana_value = row.cmc.max(0.0) as u32;
    card.is_basic_land = type_line.contains("Basic");
    card.is_land = shape.land;
    card.is_planeswalker = type_line.contains("Planeswalker");
    card.has_mana_cost = !row.mana_cost.trim().is_empty();
    card.is_creature = type_line.contains("Creature");
    card.is_human = card.is_creature && type_line.contains("Human");
    card.is_instant_or_sorcery = type_line.contains("Instant") || type_line.contains("Sorcery");
    card.is_permanent = type_line.split("//").any(|face| {
        face.split('—').next().is_some_and(|head| {
            [
                "Creature",
                "Artifact",
                "Enchantment",
                "Land",
                "Planeswalker",
                "Battle",
            ]
            .iter()
            .any(|kind| head.contains(kind))
        })
    });
    card.exile_on_resolve = oracle.spell_data.exiles_on_resolve;
    card.is_legendary = type_line.contains("Legendary");
    card.is_artifact = type_line.contains("Artifact");
    card.is_station_card = abilities.is_station_card;
    card.enter_counters = oracle.static_data.enter_counters;
    card.is_saga = abilities.is_saga;
    card.saga = if abilities.is_saga {
        super::super::oracle_lower::lower_saga_chapters(oracle)
    } else {
        super::super::model::SimSaga::default()
    };
    card.is_enchantment = type_line.contains("Enchantment") && !type_line.contains("Aura");
    card.is_mdfc_spell = shape.is_mdfc_spell;
    card.mythic = shape.mythic;
    card.crew = crew_cost(oracle);
    card.striations.clone_from(&abilities.striations);
}

/// Parse crew's numeric keyword argument (CR 702.122a). The value comes
/// from the typed `Crew` keyword; a bare "Crew" without a number defaults
/// to 1.
fn crew_cost(oracle: &OracleCard) -> Option<u32> {
    oracle
        .has_keyword(&OracleKeywordName::Crew)
        .then(|| oracle.keyword_number(&OracleKeywordName::Crew).unwrap_or(1))
}

/// Set land-entry, fetch, gate, and starting-game fields.
fn lower_land_fields(card: &mut SimCard, oracle: &OracleCard, land: bool) {
    // Unless conditions that self-solve early stay untapped early.
    card.enters_tapped = land && oracle.land.enters_tapped;
    card.gate_types = if land {
        oracle.land.gates.clone()
    } else {
        Vec::new()
    };
    card.opens_in_play = oracle.static_data.starts_on_battlefield;
    card.life_to_untap = if land { oracle.land.life_to_untap } else { 0 };
    let fetch = if land {
        oracle.land.fetch.as_ref()
    } else {
        None
    };
    card.fetch_life_cost = fetch.map_or(0, |data| data.life_cost);
    card.is_fetch_land = fetch.is_some();
    card.fetch_target_types = fetch.map_or_else(Vec::new, |data| data.target_types.clone());
    card.fetch_basic_only = fetch.is_some_and(|data| data.basic_only);
    card.fetch_enters_tapped = land && oracle.land.fetch_enters_tapped;
}

/// Parse cast spell data, cost reductions, and split-card casting rules.
fn lower_cast_fields(
    card: &mut SimCard,
    row: &CardRow,
    oracle: &OracleCard,
    type_line: &str,
    shape: &CardShape,
) {
    (card.min_cost, card.battlefield_discount) = parse_min_cost(oracle, &shape.cost);
    lower_self_cost_reduction(card, oracle);
    let tap = mana_tap_yield(oracle).or_else(|| {
        oracle
            .land
            .produces_any_color
            .then_some(super::super::model::ManaYield {
                any_pips: 1,
                ..super::super::model::ManaYield::default()
            })
    });
    let mut riders = parse_cast_riders(row, shape.cast_face, tap.is_none(), oracle);
    let is_adventure = type_line
        .split(" // ")
        .any(|face| face.split('—').next().unwrap_or("").contains("Adventure"));
    let is_transform = oracle.has_keyword(&OracleKeywordName::Transform);
    let is_split = !shape.is_mdfc_spell
        && !is_adventure
        && !is_transform
        && (row.oracle_text.contains("//") || row.mana_cost.contains("//"));
    suppress_split_riders(&mut riders, is_split);

    card.tap = tap;
    card.spell_data = riders;
}

/// Lower source-tap mana modes while retaining each mode in the Oracle AST.
fn mana_tap_yield(oracle: &OracleCard) -> Option<super::super::model::ManaYield> {
    use super::super::oracle_ast::{
        AbilityRestriction, ActivationCost, ActivationTarget, CostObject, OracleAbility,
        OracleEffect,
    };
    let modes: Vec<_> = oracle
        .abilities
        .iter()
        .filter_map(|ability| match ability {
            OracleAbility::Activated(ability)
                if ability.is_mana_ability
                    && !ability.mana_per_spell_cast
                    && ability.costs.iter().all(|cost| {
                        matches!(
                            cost,
                            ActivationCost::Mana(_)
                                | ActivationCost::Tap(ActivationTarget::Source)
                                | ActivationCost::Sacrifice {
                                    count: 1,
                                    object: CostObject::Creature | CostObject::Source,
                                }
                                | ActivationCost::PayLife(_)
                                | ActivationCost::Energy(_)
                                | ActivationCost::RemoveCounter {
                                    kind: Some(_),
                                    count: 1,
                                }
                        )
                    })
                    && ability.restrictions.iter().all(|restriction| {
                        matches!(restriction, AbilityRestriction::OncePerTurn)
                            || matches!(restriction, AbilityRestriction::Condition(text)
                                if text.contains("metalcraft")
                                    || text.contains("three or more artifacts"))
                    }) =>
            {
                if ability.effects.iter().any(|effect| {
                    matches!(effect, OracleEffect::Unsupported(text)
                        if text.to_ascii_lowercase().contains("spend this mana only")
                            && ![
                                "creature spell",
                                "artifact spell",
                                "legendary spell",
                                "instant or sorcery spell",
                            ]
                            .iter()
                            .any(|supported| text.to_ascii_lowercase().contains(supported)))
                }) {
                    return None;
                }
                ability.effects.iter().find_map(|effect| match effect {
                    OracleEffect::Mana(yield_) | OracleEffect::ManaPerCounter(yield_) => {
                        Some(yield_.clone())
                    }
                    _ => None,
                })
            }
            _ => None,
        })
        .collect();
    let [single] = modes.as_slice() else {
        if modes.is_empty() {
            return None;
        }
        let mut combined = super::super::model::ManaYield {
            alternatives: true,
            restriction: modes.iter().find_map(|mode| mode.restriction),
            opponent_any: modes.iter().any(|mode| mode.opponent_any),
            ..super::super::model::ManaYield::default()
        };
        combined.any_pips = u32::from(modes.iter().any(|mode| mode.any_pips > 0));
        combined.colorless = u32::from(modes.iter().any(|mode| mode.colorless > 0));
        for mode in &modes {
            for (index, choice) in combined.choice.iter_mut().enumerate() {
                *choice |= mode.choice[index] || mode.fixed[index] > 0;
            }
            combined.scaling = combined.scaling.or(mode.scaling);
        }
        return Some(combined);
    };
    Some(single.clone())
}

/// Apply a self cost reduction ("this spell costs {N} less to cast") to
/// the card's own floor. Grant reductions ("artifact spells you cast cost
/// {1} less") discount other spells and stay inert; affinity already cut
/// the floor, so its board-scaled path wins.
fn lower_self_cost_reduction(card: &mut SimCard, oracle: &OracleCard) {
    let reduction = oracle
        .abilities
        .iter()
        .filter_map(|ability| match ability {
            OracleAbility::Static(ability) => {
                ability.effects.iter().find_map(|effect| match effect {
                    OracleStaticEffect::CostReduction {
                        amount,
                        spell_class,
                    } if spell_class.starts_with("this spell") => Some(*amount),
                    _ => None,
                })
            }
            _ => None,
        })
        .max()
        .unwrap_or(0);
    if reduction > 0 && !card.battlefield_discount {
        card.min_cost.generic = card.min_cost.generic.saturating_sub(reduction);
    }
}

/// Clear on-cast credits for a split card, whose cheaper face is cast.
fn suppress_split_riders(spell_data: &mut SimSpellData, is_split: bool) {
    if is_split {
        spell_data.draws_on_cast = 0;
        spell_data.life_gain_on_cast = 0;
        spell_data.alternative_cast_cost = None;
        spell_data.reveal_rule = None;
        spell_data.mana_on_cast = None;
        spell_data.tokens_on_cast = 0;
        spell_data.treasures_on_cast = 0;
        spell_data.life_loss_on_resolve = 0;
        spell_data.damage_on_resolve = 0;
    }
}

/// Classify the card's role and printed colors after its tap data is known.
fn lower_classification_fields(
    card: &mut SimCard,
    row: &CardRow,
    text: &str,
    shape: &CardShape,
    is_station_card: bool,
) {
    card.role = super::super::role_classify::classify(
        row,
        text,
        &card.tap,
        &card.spell_data.mana_on_cast,
        if shape.is_mdfc_spell {
            false
        } else {
            shape.land
        },
        is_station_card,
    );
    for color in super::super::model::COLORS {
        card.colors[color.index()] =
            row.colors.contains(color.symbol()) || row.mana_cost.contains(color.symbol());
    }
    for (i, subtype) in ["Plains", "Island", "Swamp", "Mountain", "Forest"]
        .into_iter()
        .enumerate()
    {
        card.land_types[i] = row.type_line.contains(subtype);
    }
}

/// Apply parsed keywords, interactions, mana grants, and equipment data.
fn lower_static_fields(card: &mut SimCard, oracle: &OracleCard, type_line: &str) {
    let flags = parse_static_flags(type_line, oracle);
    card.flags = flags;
    card.printed_keywords = printed_keywords(oracle);
    card.has_companion = oracle.has_keyword(&OracleKeywordName::Companion);
    card.morph_cost = morph_cost(oracle);
    card.read_ahead = oracle.has_keyword(&OracleKeywordName::ReadAhead);
}

/// Lower parsed Oracle keyword abilities into the runtime keyword set.
fn printed_keywords(oracle: &OracleCard) -> super::super::model::KeywordSet {
    let mut set = super::super::model::KeywordSet::new();
    for keyword in &oracle.keywords {
        if let Some(runtime) = super::static_flags::runtime_keyword(&keyword.name) {
            set.insert(runtime);
        }
    }
    set
}

/// The face-down turn-up cost from Morph, Megamorph, or Disguise
/// (CR 702.37/702.168). The cost is a typed mana argument on the keyword.
fn morph_cost(oracle: &OracleCard) -> Option<Cost> {
    [
        OracleKeywordName::Morph,
        OracleKeywordName::Megamorph,
        OracleKeywordName::Disguise,
    ]
    .iter()
    .find_map(|name| oracle.keyword_cost(name))
}

/// Lower library, combat, and scaling fields that are not cast spell data.
fn lower_oracle_detail_fields(
    card: &mut SimCard,
    row: &CardRow,
    oracle: &OracleCard,
    shape: &CardShape,
) {
    card.spell_data.extra_land_drops_on_cast = oracle
        .abilities
        .iter()
        .filter_map(|ability| match ability {
            super::super::oracle_ast::OracleAbility::Spell(spell) => Some(&spell.effects),
            _ => None,
        })
        .flatten()
        .filter(|effect| {
            matches!(
                effect,
                super::super::oracle_ast::OracleEffect::AdditionalLandPlay
            )
        })
        .count() as u32;
    card.mills_opponent = oracle.static_data.mills_opponent;
    card.keyword_abilities = super::lower_keywords(oracle);
    card.printed_power = row
        .power
        .as_deref()
        .and_then(|power| power.trim_end_matches('*').parse::<u32>().ok());
    card.printed_toughness = row
        .toughness
        .as_deref()
        .and_then(|value| value.trim_end_matches('*').parse::<u32>().ok());
    card.has_cascade = oracle.has_keyword(&OracleKeywordName::Cascade);
    card.starting_loyalty = row
        .loyalty
        .as_deref()
        .and_then(|loyalty| loyalty.trim().parse::<u32>().ok());
    card.dredge = dredge_amount(oracle);
    card.library_graveyard_trigger = oracle.library_graveyard_trigger.map(|effect| match effect {
        super::super::oracle_ast::OracleLibraryGraveyardEffect::ReturnToBattlefield => {
            super::super::model::LibraryGraveyardTrigger::ReturnToBattlefield
        }
        super::super::oracle_ast::OracleLibraryGraveyardEffect::DrainAndGain(amount) => {
            super::super::model::LibraryGraveyardTrigger::DrainAndGain(amount)
        }
    });
    card.has_undying = oracle.has_keyword(&OracleKeywordName::Undying);
    card.buffs_battlefield_on_entry = oracle.static_data.buffs_battlefield_on_entry;
    card.counters_are_power = matches!(
        oracle.static_data.enter_counters,
        super::super::model::EnterCounters::XPlus1
    );
    card.draws_per_matching = if shape.land {
        None
    } else {
        oracle.static_data.draw_per_controlled_permanent
    };
}

/// Parse the number of cards milled by a Dredge replacement (CR 702.52).
fn dredge_amount(oracle: &OracleCard) -> Option<u32> {
    oracle
        .keyword_number(&OracleKeywordName::Dredge)
        .filter(|amount| *amount > 0)
}
