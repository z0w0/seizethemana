//! Independent spell payments from at most two known land drops.

use super::game::ManaPool;
use super::game_mana::{add_yield, cast_restrictions, payment_plan, phyrexian_life_charge};
use super::model::{
    BasicLandType, CardIdx, LifeLossScope, ManaYield, Role, SimAbilityCondition, SimCard, SimDeck,
    SimEffect,
};
use super::oracle_ast::{OracleAbility, OracleCard};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use crate::db::CardRow;
use crate::deck::mana_audit::{land_can_tap_on_entry, unsupported_land_entry};
use std::collections::HashMap;

/// Bounded payment evidence without future draws or nonland mana sources.
#[derive(Debug, Default)]
pub(crate) struct KnownHandPayments {
    /// Spells that can each pay their own mana cost by turn two.
    pub payable_spells: usize,
    /// Some missing data or source requirements were outside this check.
    pub skipped_requirements: bool,
}

/// One physical land and its mutually exclusive, supported tap modes.
struct KnownLand {
    index: CardIdx,
    card: SimCard,
    modes: Vec<LandMode>,
    can_tap_on_entry: bool,
}

/// One complete tap, with its life cost or self-directed damage.
struct LandMode {
    mana: ManaYield,
    life: u32,
    required_land_types: Option<[bool; 5]>,
}

impl LandMode {
    /// Check the activation gate against the lands in the chosen play order.
    fn is_available(&self, land_types: [bool; 5]) -> bool {
        self.required_land_types.is_none_or(|required| {
            required
                .into_iter()
                .zip(land_types)
                .any(|(required, controlled)| required && controlled)
        })
    }
}

/// Count independent payments without assuming hidden cards or reusing an MDFC.
pub(crate) fn known_hand_payments(
    deck: &SimDeck,
    hand: &[CardIdx],
    cards: &HashMap<String, CardRow>,
) -> KnownHandPayments {
    let mut result = KnownHandPayments::default();
    let commander_colors = commander_colors(deck, cards);
    let active_opponents = super::opponents::OpponentTable::for_format(deck.format)
        .seats
        .iter()
        .filter(|seat| seat.active)
        .count() as u32;
    let lands: Vec<_> = hand
        .iter()
        .filter(|index| deck[**index].role == Role::Land || deck[**index].is_mdfc_spell)
        .filter_map(|index| {
            let Some(row) = cards.get(&deck[*index].name) else {
                result.skipped_requirements = true;
                return None;
            };
            known_land(
                *index,
                row,
                commander_colors,
                &mut result.skipped_requirements,
            )
        })
        .collect();
    for index in hand {
        let spell = &deck[*index];
        if spell.opens_in_play || spell.role == Role::Land && !spell.is_mdfc_spell {
            continue;
        }
        let Some(row) = cards.get(&spell.name) else {
            result.skipped_requirements = true;
            continue;
        };
        if row.mana_cost.trim().is_empty() || !spell.has_mana_cost {
            continue;
        }
        if spell.cost.unsupported || spell.cost.snow > 0 {
            result.skipped_requirements = true;
            continue;
        }
        if payable_from_lands(
            *index,
            spell,
            &lands,
            deck.rules.starting_life,
            active_opponents,
        ) {
            result.payable_spells += 1;
        }
    }
    result
}

/// Keep entry rules and land-face text separate from the MDFC's spell face.
fn known_land(
    index: CardIdx,
    row: &CardRow,
    commander_colors: Option<[bool; 5]>,
    skipped: &mut bool,
) -> Option<KnownLand> {
    let Some(face) = land_face(row) else {
        *skipped = true;
        return None;
    };
    if unsupported_land_entry(&face) {
        *skipped = true;
        return None;
    }
    let oracle = parse_oracle_card(&face);
    let land = parse_sim_card(&face);
    let mut modes = land_modes(&oracle, commander_colors, skipped);
    for kind in BasicLandType::ALL {
        if land.land_types[kind.index()] {
            let mut mana = ManaYield::default();
            mana.fixed[kind.index()] = 1;
            modes.push(LandMode {
                mana,
                life: 0,
                required_land_types: None,
            });
        }
    }
    Some(KnownLand {
        index,
        card: land,
        modes,
        can_tap_on_entry: land_can_tap_on_entry(&face),
    })
}

/// Select the known land face rather than reading mana from both faces.
fn land_face(row: &CardRow) -> Option<CardRow> {
    let (face_index, type_line) = row.type_line.split(" // ").enumerate().find(|(_, face)| {
        face.split('—')
            .next()
            .is_some_and(|head| head.contains("Land"))
    })?;
    let mut face = row.clone();
    face.type_line = type_line.to_string();
    face.mana_cost.clear();
    face.cmc = 0.0;
    if row.type_line.contains(" // ") {
        face.oracle_text = row
            .oracle_text
            .split("//")
            .nth(face_index)?
            .trim()
            .to_string();
        face.keywords = "[]".to_string();
    }
    Some(face)
}

/// Retain complete free-tap modes with supported life costs and land-type gates.
fn land_modes(
    oracle: &OracleCard,
    commander_colors: Option<[bool; 5]>,
    skipped: &mut bool,
) -> Vec<LandMode> {
    let mut modes = Vec::new();
    for ability in &oracle.abilities {
        let OracleAbility::Activated(activated) = ability else {
            continue;
        };
        if !activated.is_mana_ability {
            continue;
        }
        let runtime = ability.to_runtime();
        if runtime.is_empty() {
            *skipped = true;
        }
        for ability in runtime {
            let activation = ability
                .activation
                .as_ref()
                .expect("lowered activated abilities retain their costs");
            let required_land_types = match ability.condition {
                None => None,
                Some(SimAbilityCondition::ControlsLandTypes(types)) => Some(types),
                _ => {
                    *skipped = true;
                    continue;
                }
            };
            if !activation.taps_source()
                || !activation.costs.iter().all(|cost| {
                    matches!(
                        cost,
                        super::model::SimActivationCost::TapSource
                            | super::model::SimActivationCost::PayLife(_)
                    )
                })
            {
                *skipped = true;
                continue;
            }
            let Some(mut mode) = land_mode(ability.effect_sequence(), activation.life_payment())
            else {
                *skipped = true;
                continue;
            };
            mode.required_land_types = required_land_types;
            if mode.mana.commander_pips > 0
                && !restrict_commander_colors(&mut mode.mana, commander_colors)
            {
                *skipped = true;
                continue;
            }
            modes.push(mode);
        }
    }
    modes
}

/// Use declared identities, including partner colors and off-cost color symbols.
fn commander_colors(deck: &SimDeck, cards: &HashMap<String, CardRow>) -> Option<[bool; 5]> {
    if deck.commanders.is_empty() {
        return None;
    }
    let mut colors = [false; 5];
    for commander in &deck.commanders {
        let row = cards.get(&commander.name)?;
        let identity: Vec<String> = serde_json::from_str(&row.color_identity).ok()?;
        for color in identity {
            let index = ["W", "U", "B", "R", "G"]
                .iter()
                .position(|letter| *letter == color)?;
            colors[index] = true;
        }
    }
    Some(colors)
}

/// Resolve a typed identity pip to the commander's known colors.
fn restrict_commander_colors(mana: &mut ManaYield, colors: Option<[bool; 5]>) -> bool {
    let Some(colors) = colors else { return false };
    if mana.commander_pips != 1
        || mana.any_pips > 0
        || mana.choice.iter().any(|color| *color)
        || mana.choice_colorless
    {
        return false;
    }
    mana.commander_pips = 0;
    mana.choice = colors;
    true
}

/// Preserve life costs and reject effects that need another game resource.
fn land_mode(effects: &[SimEffect], mut life: u32) -> Option<LandMode> {
    let mut mana = None;
    for effect in effects {
        match effect {
            SimEffect::Mana(yield_)
                if mana.is_none()
                    && !yield_.opponent_any
                    && !yield_.legendary_colors
                    && yield_.scaling.is_none() =>
            {
                mana = Some(yield_.clone());
            }
            SimEffect::LoseLife {
                amount,
                scope: LifeLossScope::SelfOnly,
            }
            | SimEffect::Damage {
                amount,
                scope: LifeLossScope::SelfOnly,
            } => life += amount,
            _ => return None,
        }
    }
    Some(LandMode {
        mana: mana?,
        life,
        required_land_types: None,
    })
}

/// Try one land and every legal order of two different land cards.
fn payable_from_lands(
    index: CardIdx,
    spell: &SimCard,
    lands: &[KnownLand],
    life: i32,
    active_opponents: u32,
) -> bool {
    let life = u32::try_from(life).expect("format starting life is nonnegative");
    if pays(spell, &ManaPool::default(), life) {
        return true;
    }
    for first in lands.iter().filter(|land| land.index != index) {
        if pays_with_land_modes(spell, first, None, first.card.land_types, life) {
            return true;
        }
        for second in lands
            .iter()
            .filter(|land| land.index != index && land.index != first.index)
        {
            let types =
                std::array::from_fn(|i| first.card.land_types[i] || second.card.land_types[i]);
            if pays_with_land_modes(spell, first, None, types, life) {
                return true;
            }
            let second_ready = second.can_tap_on_entry
                && second.card.life_to_untap < life
                && !second.card.entry_is_tapped_with_land_types(
                    1,
                    active_opponents,
                    first.card.land_types,
                );
            let entry_life = if second_ready {
                second.card.life_to_untap
            } else {
                0
            };
            if pays_with_land_modes(
                spell,
                first,
                second_ready.then_some(second),
                types,
                life - entry_life,
            ) {
                return true;
            }
        }
    }
    false
}

/// Spend at most one tap from each physical land after the chosen entries.
fn pays_with_land_modes(
    spell: &SimCard,
    first: &KnownLand,
    second: Option<&KnownLand>,
    land_types: [bool; 5],
    life: u32,
) -> bool {
    for first_mode in first
        .modes
        .iter()
        .filter(|mode| mode.is_available(land_types))
        .map(Some)
        .chain(std::iter::once(None))
    {
        for second_mode in second
            .into_iter()
            .flat_map(|land| &land.modes)
            .filter(|mode| mode.is_available(land_types))
            .map(Some)
            .chain(std::iter::once(None))
        {
            let mut pool = ManaPool::default();
            let mut paid_life = 0;
            for mode in [first_mode, second_mode].into_iter().flatten() {
                add_yield(&mode.mana, &mut pool);
                paid_life += mode.life;
            }
            if paid_life < life && pays(spell, &pool, life - paid_life) {
                return true;
            }
        }
    }
    false
}

/// Use the simulator's symbol matching and cast-class restrictions.
fn pays(spell: &SimCard, pool: &ManaPool, life: u32) -> bool {
    phyrexian_life_charge(&spell.cost) <= life
        && payment_plan(&spell.cost, pool, &cast_restrictions(spell)).is_some()
}
