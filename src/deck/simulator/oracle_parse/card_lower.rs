//! Lower one card row and its parsed Oracle syntax into simulator data.

use super::super::super::stats::is_land;
use super::super::model::{CastRiders, Cost, SimCard, Tier};
use super::super::oracle_ast::{KeywordName, OracleAbility, OracleCard, StaticEffect};
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
    station_tiers: Vec<Tier>,
    is_station_card: bool,
    is_saga: bool,
}

/// Lower parsed Oracle syntax and card metadata into one simulated card.
pub(super) fn lower_oracle_card(row: &CardRow, oracle: &OracleCard) -> SimCard {
    let text = row.oracle_text.to_ascii_lowercase();
    let type_line = row.type_line.to_string();
    let shape = card_shape(row);
    let abilities = card_abilities(row, oracle, &text, &type_line, &shape);
    let mut card = SimCard::default();

    lower_identity_fields(
        &mut card, row, oracle, &text, &type_line, &shape, &abilities,
    );
    lower_land_fields(&mut card, row, &text, shape.land);
    lower_cast_fields(&mut card, row, oracle, &text, &type_line, &shape);
    lower_classification_fields(&mut card, row, &text, &shape, abilities.is_station_card);
    lower_static_fields(&mut card, row, oracle, &text, &type_line, &shape);
    lower_oracle_detail_fields(&mut card, row, oracle, &text, &shape);

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
        super::parse_oracle_cost_faces(&row.mana_cost)
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
fn card_abilities(
    row: &CardRow,
    oracle: &OracleCard,
    text: &str,
    type_line: &str,
    shape: &CardShape,
) -> CardAbilities {
    // Station tiers (Spacecraft and Planet cards).
    let (tiers, is_station_card) = if text.contains("station") {
        super::parse_oracle_station_tiers(&row.oracle_text, &row.type_line)
    } else {
        (Vec::new(), false)
    };

    let abilities = oracle.runtime_abilities();
    let mut station_tiers = tiers;
    let is_saga = type_line.contains("Saga") && !shape.land;
    if !abilities.is_empty() {
        station_tiers.insert(
            0,
            Tier {
                at: 0,
                animate: false,
                abilities,
            },
        );
    }

    CardAbilities {
        station_tiers,
        is_station_card,
        is_saga,
    }
}

/// Set identity and type fields that come directly from card metadata.
fn lower_identity_fields(
    card: &mut SimCard,
    row: &CardRow,
    oracle: &OracleCard,
    text: &str,
    type_line: &str,
    shape: &CardShape,
    abilities: &CardAbilities,
) {
    card.name.clone_from(&row.name);
    card.cost.clone_from(&shape.cost);
    card.mana_value = row.cmc.max(0.0) as u32;
    card.is_basic_land = type_line.contains("Basic");
    card.has_mana_cost = !row.mana_cost.trim().is_empty();
    card.is_creature = type_line.contains("Creature");
    card.is_human = card.is_creature && type_line.contains("Human");
    card.is_instant_or_sorcery = type_line.contains("Instant") || type_line.contains("Sorcery");
    card.exile_on_resolve = text.contains("exile this spell")
        || text.contains(&format!("exile {}", row.name.to_ascii_lowercase()));
    card.is_legendary = type_line.contains("Legendary");
    card.is_artifact = type_line.contains("Artifact");
    card.is_station_card = abilities.is_station_card;
    card.enter_counters = super::parse_enter_counters(text);
    card.is_saga = abilities.is_saga;
    card.saga = if abilities.is_saga {
        super::super::oracle_lower::lower_saga_chapters(oracle)
    } else {
        super::super::model::SagaData::default()
    };
    card.is_enchantment = type_line.contains("Enchantment") && !type_line.contains("Aura");
    card.is_mdfc_spell = shape.is_mdfc_spell;
    card.mythic = shape.mythic;
    card.crew = crew_cost(text, oracle);
    card.station_tiers.clone_from(&abilities.station_tiers);
}

/// Parse crew's numeric keyword argument or its reminder-text value.
fn crew_cost(text: &str, oracle: &OracleCard) -> Option<u32> {
    if oracle.has_keyword(&KeywordName::Crew) {
        Some(
            oracle
                .keyword_number(&KeywordName::Crew)
                .or_else(|| {
                    text.find("crew ").and_then(|i| {
                        text[i + 5..]
                            .chars()
                            .take_while(|c| c.is_ascii_digit())
                            .collect::<String>()
                            .parse::<u32>()
                            .ok()
                    })
                })
                .unwrap_or(1),
        )
    } else {
        None
    }
}

/// Set land-entry, fetch, gate, and starting-game fields.
fn lower_land_fields(card: &mut SimCard, row: &CardRow, text: &str, land: bool) {
    // Unless conditions that self-solve early stay untapped early.
    card.enters_tapped = land && super::enters_tapped(text);
    card.gate_types = if land {
        super::parse_oracle_gates(&row.oracle_text)
    } else {
        Vec::new()
    };
    card.opens_in_play = text.contains("begin the game with it on the battlefield");
    card.life_to_untap = if land {
        super::super::parse_land::life_to_untap(text)
    } else {
        0
    };

    let fetch_text = text.replace(&row.name.to_ascii_lowercase(), "this land");
    let fetch_search = if land {
        super::super::parse_land::fetch_search(&fetch_text)
    } else {
        None
    };
    card.fetch_life_cost = if land {
        super::super::parse_land::fetch_life_cost(&fetch_text)
    } else {
        0
    };
    card.is_fetch_land = fetch_search.is_some();
    card.fetch_target_types = fetch_search.map(|(types, _)| types).unwrap_or_default();
    card.fetch_basic_only = fetch_search.is_some_and(|(_, basic_only)| basic_only);
    card.fetch_enters_tapped = land && text.contains("put it onto the battlefield tapped");
}

/// Parse cast riders, cost reductions, and split-card casting rules.
fn lower_cast_fields(
    card: &mut SimCard,
    row: &CardRow,
    oracle: &OracleCard,
    text: &str,
    type_line: &str,
    shape: &CardShape,
) {
    (card.min_cost, card.board_discount) = parse_min_cost(text, &shape.cost);
    lower_self_cost_reduction(card, oracle);
    let tap = super::parse_oracle_tap(row);
    let mut riders = parse_cast_riders(row, text, shape.cast_face, tap.is_none(), oracle);
    let is_adventure = type_line
        .split(" // ")
        .any(|face| face.split('—').next().unwrap_or("").contains("Adventure"));
    let is_transform = oracle.has_keyword(&KeywordName::Transform);
    let is_split = !shape.is_mdfc_spell
        && !is_adventure
        && !is_transform
        && (row.oracle_text.contains("//") || row.mana_cost.contains("//"));
    suppress_split_riders(&mut riders, is_split);

    let tap = if riders.mana_per_cast.is_some() {
        super::parse_oracle_tap_filtered(row)
    } else {
        tap
    };
    card.tap = tap;
    card.riders = riders;
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
                    StaticEffect::CostReduction {
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
    if reduction > 0 && !card.board_discount {
        card.min_cost.generic = card.min_cost.generic.saturating_sub(reduction);
    }
}

/// Clear on-cast credits for a split card, whose cheaper face is cast.
fn suppress_split_riders(riders: &mut CastRiders, is_split: bool) {
    if is_split {
        riders.draws_on_cast = 0;
        riders.life_gain_on_cast = 0;
        riders.alternative_cast_cost = None;
        riders.reveal_rule = None;
        riders.mana_on_cast = None;
        riders.tokens_on_cast = 0;
        riders.drain_on_cast = 0;
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
        &card.riders.mana_on_cast,
        if shape.is_mdfc_spell {
            false
        } else {
            shape.land
        },
        is_station_card,
    );
    for (i, ch) in super::super::model::COLORS.iter().enumerate() {
        card.colors[i] = row.colors.contains(*ch) || row.mana_cost.contains(*ch);
    }
    for (i, subtype) in ["Plains", "Island", "Swamp", "Mountain", "Forest"]
        .into_iter()
        .enumerate()
    {
        card.land_types[i] = row.type_line.contains(subtype);
    }
}

/// Apply parsed keywords, interactions, mana grants, and equipment data.
fn lower_static_fields(
    card: &mut SimCard,
    row: &CardRow,
    oracle: &OracleCard,
    text: &str,
    type_line: &str,
    shape: &CardShape,
) {
    let flags = parse_static_flags(row, text, type_line, shape.cast_face, oracle);
    card.flags = flags;
}

/// Lower library, combat, and scaling fields that are not cast riders.
fn lower_oracle_detail_fields(
    card: &mut SimCard,
    row: &CardRow,
    oracle: &OracleCard,
    text: &str,
    shape: &CardShape,
) {
    card.mills_opponent = super::super::model::mills_opponent(text);
    card.printed_power = row
        .power
        .as_deref()
        .and_then(|power| power.trim_end_matches('*').parse::<u32>().ok());
    card.printed_toughness = row
        .toughness
        .as_deref()
        .and_then(|value| value.trim_end_matches('*').parse::<u32>().ok());
    card.has_cascade = oracle.has_keyword(&KeywordName::Cascade)
        || text.contains("cascade")
        || text.contains("battle-cascade");
    card.starting_loyalty = row
        .loyalty
        .as_deref()
        .and_then(|loyalty| loyalty.trim().parse::<u32>().ok());
    card.dredge = dredge_amount(text, oracle);
    card.library_graveyard_trigger = library_graveyard_trigger(text);
    card.sacrifices_for_mana = row.oracle_text.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains(": add ")
            && lower.contains("sacrifice")
            && (lower.contains(&row.name.to_ascii_lowercase())
                || lower.contains("sacrifice this permanent"))
    });
    card.has_undying = oracle.has_keyword(&KeywordName::Undying) || text.contains("undying");
    card.buffs_board_on_enter = text.contains("+x/+x")
        && (text.contains("where x is") || text.contains("equal to the number"));
    card.counters_are_power = super::enters_with_x_counters(text);
    card.draws_per_matching = if shape.land {
        None
    } else {
        super::scaling_draw_match(text)
    };
}

/// Parse the number of cards milled by a Dredge replacement.
fn dredge_amount(text: &str, oracle: &OracleCard) -> Option<u32> {
    oracle.keyword_number(&KeywordName::Dredge).or_else(|| {
        text.find("dredge ")
            .and_then(|start| {
                text[start + "dredge ".len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse::<u32>()
                    .ok()
            })
            .filter(|amount| *amount > 0)
    })
}

/// Parse the effect that triggers when this card moves from library to graveyard.
fn library_graveyard_trigger(text: &str) -> Option<super::super::model::LibraryGraveyardTrigger> {
    text.split(['\n', '.'])
        .find(|line| line.contains("put into your graveyard from your library"))
        .and_then(|line| {
            if line.contains("put it onto the battlefield") {
                Some(super::super::model::LibraryGraveyardTrigger::ReturnToBattlefield)
            } else if line.contains("you gain") {
                Some(super::super::model::amount_after(line, "loses "))
                    .filter(|amount| *amount > 0)
                    .map(super::super::model::LibraryGraveyardTrigger::DrainAndGain)
            } else {
                None
            }
        })
}
