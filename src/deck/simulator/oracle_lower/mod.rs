//! Lower Oracle syntax nodes into simulator runtime data.

use super::model::{
    SimAbility, SimAbilityCondition, SimAbilityKind, SimActivation, SimActivationCost,
    SimActivationRestriction, SimCard, SimEffect, SimEventSubject, SimStriation, SimTrigger,
};
use super::oracle_ast::{
    AbilityRestriction, ActivationCost, ActivationTarget, CostObject, ObjectSubject, OracleAbility,
    OracleActivatedAbility, OracleCard, OracleEffect, OracleStaticAbility, OracleStaticEffect,
    OracleStations, OracleTriggerEvent, OracleTriggeredAbility, PlayerScope,
};
use crate::db::CardRow;

/// Lower card-row fields and AST nodes into the executable card model.
mod card;
/// Lower one-shot spell data from parsed syntax.
mod cast_riders;
/// Lower keyword AST nodes into runtime card data.
mod keyword_lower;
/// Lower interaction and static card diagnostics.
mod static_flags;
use keyword_lower::lower_keywords;

/// Build simulator card data from one stored card row.
pub fn parse_sim_card(row: &CardRow) -> SimCard {
    let oracle = super::oracle_parser::parse_oracle_card(row);
    card::lower_oracle_card(row, &oracle)
}

/// Lower parsed station striations to runtime abilities.
pub(super) fn lower_striations(stations: &OracleStations) -> Vec<SimStriation> {
    stations
        .striations
        .iter()
        .map(|striation| SimStriation {
            at: striation.at,
            animate: striation.animate,
            abilities: striation
                .abilities
                .iter()
                .filter_map(|ability| ability.to_runtime_all().into_iter().next())
                .collect(),
        })
        .collect()
}

impl OracleCard {
    /// Lower abilities the current game model can execute.
    pub(super) fn runtime_abilities(&self) -> Vec<SimAbility> {
        self.abilities
            .iter()
            .flat_map(OracleAbility::to_runtime)
            .collect()
    }
}

impl OracleAbility {
    /// Lower every executable effect of an ability node into the
    /// simulator's turn-loop model. A triggered node lowers one entry
    /// per supported effect ("draw a card and create a Treasure token"
    /// fires both), so a multi-effect resolution loses nothing.
    pub(super) fn to_runtime(&self) -> Vec<SimAbility> {
        match self {
            Self::Activated(ability) => ability.to_runtime_all(),
            Self::Triggered(ability) => ability.to_runtime_all(),
            Self::Static(ability) => ability.to_runtime().into_iter().collect(),
            Self::Spell(_) | Self::SagaChapter(_) | Self::Unsupported(_) => Vec::new(),
        }
    }
}

impl OracleActivatedAbility {
    /// Lower a supported activation to one runtime ability with ordered
    /// effects and one shared cost.
    pub(super) fn to_runtime_all(&self) -> Vec<SimAbility> {
        if self.mana_per_spell_cast {
            return Vec::new();
        }
        if self.is_mana_ability
            && self.effects.iter().any(|effect| {
                matches!(effect, OracleEffect::Unsupported(text)
                    if text.to_ascii_lowercase().contains("spend this mana only")
                        || text.to_ascii_lowercase().contains("this mana can't be spent"))
            })
            && !self.effects.iter().any(|effect| {
                matches!(effect, OracleEffect::Mana(yield_)
                    if yield_.restriction.is_some() || yield_.cannot_pay_generic)
            })
        {
            return Vec::new();
        }
        if self
            .restrictions
            .iter()
            .any(|restriction| matches!(restriction, AbilityRestriction::Condition(_)))
            || !self.costs.iter().all(activation_cost_is_supported)
        {
            return Vec::new();
        }
        let effects = self
            .effects
            .iter()
            .filter_map(OracleEffect::to_runtime)
            .collect::<Vec<_>>();
        let Some(effect) = effects.first().cloned() else {
            return Vec::new();
        };
        vec![SimAbility {
            id: 0,
            kind: if self.is_mana_ability {
                SimAbilityKind::ManaActivated
            } else {
                SimAbilityKind::Activated
            },
            trigger: SimTrigger::Never,
            effect,
            effects,
            activation: Some(lower_activation(self)),
            once_per_turn: false,
            condition: None,
            event_subject: SimEventSubject::Any,
        }]
    }
}

/// Convert every supported activation cost and restriction into one
/// typed runtime bundle.
fn lower_activation(ability: &OracleActivatedAbility) -> SimActivation {
    let costs = ability
        .costs
        .iter()
        .filter_map(|cost| match cost {
            ActivationCost::Mana(cost) => Some(SimActivationCost::Mana(*cost)),
            ActivationCost::Tap(ActivationTarget::Source) => Some(SimActivationCost::TapSource),
            ActivationCost::Untap(ActivationTarget::Source) => Some(SimActivationCost::UntapSource),
            ActivationCost::Sacrifice {
                count,
                object: CostObject::Creature,
            } => Some(SimActivationCost::SacrificeCreatures(*count)),
            ActivationCost::Sacrifice {
                object: CostObject::Source,
                ..
            } => Some(SimActivationCost::SacrificeSource),
            ActivationCost::PayLife(amount) => Some(SimActivationCost::PayLife(*amount)),
            ActivationCost::Energy(amount) => Some(SimActivationCost::Energy(*amount)),
            ActivationCost::RemoveCounter { count, .. } => {
                Some(SimActivationCost::RemoveChargeCounters(*count))
            }
            ActivationCost::Loyalty(amount) => Some(SimActivationCost::Loyalty(*amount)),
            ActivationCost::Tap(ActivationTarget::AnotherObject)
            | ActivationCost::Untap(ActivationTarget::AnotherObject)
            | ActivationCost::Sacrifice { .. }
            | ActivationCost::Discard(_)
            | ActivationCost::Other(_) => None,
        })
        .collect();
    let restrictions = ability
        .restrictions
        .iter()
        .filter_map(|restriction| match restriction {
            AbilityRestriction::OncePerTurn => Some(SimActivationRestriction::OncePerTurn),
            AbilityRestriction::OncePerGame => Some(SimActivationRestriction::OncePerGame),
            AbilityRestriction::PowerUp => Some(SimActivationRestriction::PowerUp),
            AbilityRestriction::SorcerySpeed | AbilityRestriction::Condition(_) => None,
        })
        .collect();
    SimActivation {
        costs,
        restrictions,
    }
}

/// True when the runtime can pay this activation cost without ignoring
/// part of the payment.
fn activation_cost_is_supported(cost: &ActivationCost) -> bool {
    match cost {
        ActivationCost::Mana(_) | ActivationCost::PayLife(_) | ActivationCost::Energy(_) => true,
        ActivationCost::Tap(ActivationTarget::Source)
        | ActivationCost::Untap(ActivationTarget::Source) => true,
        ActivationCost::Sacrifice {
            count: 1,
            object: CostObject::Creature | CostObject::Source,
        } => true,
        ActivationCost::RemoveCounter { kind, count: 1 } => {
            kind.as_deref().is_some_and(|name| name == "charge")
        }
        ActivationCost::Loyalty(_) => true,
        ActivationCost::Tap(ActivationTarget::AnotherObject)
        | ActivationCost::Untap(ActivationTarget::AnotherObject)
        | ActivationCost::Sacrifice { .. }
        | ActivationCost::Discard(_)
        | ActivationCost::RemoveCounter { .. }
        | ActivationCost::Other(_) => false,
    }
}

impl OracleStaticAbility {
    /// Lower the counter-threshold rule that the upkeep census can check.
    pub(super) fn to_runtime(&self) -> Option<SimAbility> {
        let counters = self.effects.iter().find_map(|effect| match effect {
            OracleStaticEffect::WinsAtCounters(counters) => Some(*counters),
            _ => None,
        })?;
        Some(SimAbility {
            kind: SimAbilityKind::Static,
            trigger: SimTrigger::Upkeep,
            effect: SimEffect::WinThreshold { counters },
            ..SimAbility::default()
        })
    }
}

impl OracleTriggerEvent {
    /// Map the parsed event to a simulator trigger when its timing is modeled.
    pub(super) fn to_runtime(&self) -> Option<SimTrigger> {
        match self {
            Self::Enters(_) => Some(SimTrigger::Enters),
            Self::LandEnters(PlayerScope::You) => Some(SimTrigger::LandEnters),
            Self::BeginningOfUpkeep(PlayerScope::You) => Some(SimTrigger::Upkeep),
            Self::BeginningOfEndStep(PlayerScope::You) => Some(SimTrigger::EndStep),
            Self::BeginningOfPrecombatMain(PlayerScope::You) => Some(SimTrigger::PrecombatMain),
            Self::Attacks(_) => Some(SimTrigger::Attacks),
            Self::PlayerAttacks => Some(SimTrigger::PlayerAttacks),
            Self::CombatDamageToPlayer(_) => Some(SimTrigger::CombatDamage),
            Self::CastsSpell { this_spell: false } => Some(SimTrigger::SpellCast),
            Self::Dies(_) => Some(SimTrigger::Dies),
            Self::TappedForMana => Some(SimTrigger::TappedForMana),
            // "Whenever the Ring tempts you" (CR 701.54d) fires when the
            // Ring-tempts counter moves.
            Self::RingTempts => Some(SimTrigger::RingTempts),
            // A state trigger on a counter threshold normalizes to the
            // upkeep win check: the census reads counters, not events.
            Self::WinsAtCounters { .. } => Some(SimTrigger::Upkeep),
            Self::CastsSpell { this_spell: true }
            | Self::BeginningOfUpkeep(_)
            | Self::BeginningOfEndStep(_)
            | Self::BeginningOfPrecombatMain(_)
            | Self::BeginningOfOther { .. }
            | Self::LandEnters(PlayerScope::Opponent | PlayerScope::Any)
            | Self::Other(_) => None,
        }
    }

    /// Preserve the object relationship named by the parsed trigger.
    fn event_subject(&self) -> SimEventSubject {
        let subject = match self {
            Self::Enters(subject)
            | Self::Attacks(subject)
            | Self::CombatDamageToPlayer(subject)
            | Self::Dies(subject) => *subject,
            _ => return SimEventSubject::Any,
        };
        match subject {
            ObjectSubject::ThisPermanent => SimEventSubject::This,
            ObjectSubject::AnotherPermanent => SimEventSubject::Another,
            ObjectSubject::ControlledLand | ObjectSubject::Any => SimEventSubject::Any,
        }
    }
}

impl OracleTriggeredAbility {
    /// Lower a trigger to one runtime ability with ordered effects.
    pub(super) fn to_runtime_all(&self) -> Vec<SimAbility> {
        // A state trigger on a counter threshold ("When [this] has N or
        // more [kind] counters on it, you win the game.") is its own
        // win check; the resolution text repeats the win.
        let win_threshold = match self.event {
            OracleTriggerEvent::WinsAtCounters { counters } => Some((counters, SimTrigger::Upkeep)),
            _ => None,
        }
        .or_else(|| {
            self.effects.iter().find_map(|effect| match effect {
                OracleEffect::WinsAtCounters(counters) => Some((*counters, SimTrigger::Upkeep)),
                _ => None,
            })
        });
        if let Some((counters, trigger)) = win_threshold {
            return vec![SimAbility {
                trigger,
                effect: SimEffect::WinThreshold { counters },
                once_per_turn: self.once_per_turn,
                condition: None,
                event_subject: self.event.event_subject(),
                ..SimAbility::default()
            }];
        }
        let condition = match self.condition.as_deref() {
            Some(text) => match condition_from(text) {
                Some(condition) => Some(condition),
                None => return Vec::new(),
            },
            None => None,
        };
        let Some(trigger) = self.event.to_runtime() else {
            return Vec::new();
        };
        let effects = self
            .effects
            .iter()
            .filter_map(OracleEffect::to_runtime)
            .collect::<Vec<_>>();
        let Some(effect) = effects.first().cloned() else {
            return Vec::new();
        };
        vec![SimAbility {
            kind: if self.is_mana_ability {
                SimAbilityKind::ManaTriggered
            } else {
                SimAbilityKind::Triggered
            },
            trigger,
            effect,
            effects,
            once_per_turn: self.once_per_turn,
            condition,
            event_subject: self.event.event_subject(),
            ..SimAbility::default()
        }]
    }
}

/// Map a raw intervening-"if" clause (CR 603.4) onto a modeled
/// condition. Unsupported conditions make the whole trigger inert.
fn condition_from(condition: &str) -> Option<SimAbilityCondition> {
    let text = condition.to_ascii_lowercase();
    if text.contains(" and ") || text.contains(",") {
        return None;
    }
    // Descend (CR 701.48-adjacent): "N or more permanent cards in your
    // graveyard" or the ability word "descend N".
    if let Some(rest) = text.strip_prefix("descend ") {
        let n = condition_number(rest)?;
        return Some(SimAbilityCondition::Descend(n));
    }
    if let Some(rest) = text
        .strip_prefix("you have ")
        .or_else(|| text.strip_prefix("there are "))
        && let Some((amount, "")) = rest.split_once(" or more permanent cards in your graveyard")
    {
        let n = condition_number(amount)?;
        return Some(SimAbilityCondition::Descend(n));
    }
    if matches!(text.trim(), "you have threshold" | "threshold") {
        return Some(SimAbilityCondition::Threshold);
    }
    if matches!(text.trim(), "you attacked this turn" | "you attacked") {
        return Some(SimAbilityCondition::Raid);
    }
    if matches!(
        text.trim(),
        "you control a creature with power 4 or greater"
    ) {
        return Some(SimAbilityCondition::Ferocious);
    }
    if matches!(
        text.trim(),
        "you have metalcraft" | "you control three or more artifacts"
    ) {
        return Some(SimAbilityCondition::Metalcraft);
    }
    None
}

/// Read a positive count from a supported intervening-if phrase.
fn condition_number(text: &str) -> Option<u32> {
    match text.trim() {
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        digits => digits.parse::<u32>().ok().filter(|count| *count > 0),
    }
}

impl OracleEffect {
    /// Convert a supported syntax effect to the executable game model.
    pub(super) fn to_runtime(&self) -> Option<SimEffect> {
        Some(match self {
            Self::Draw(count) => SimEffect::Draw(*count),
            Self::DrawThenDiscard(count) => SimEffect::DrawThenDiscard(*count),
            Self::DrawAndMinusCounter => SimEffect::DrawAndMinusCounter,
            Self::GainLife(amount) => SimEffect::GainLife(*amount),
            Self::Search(spec) => SimEffect::Search(*spec),
            Self::Mana(yield_) => SimEffect::Mana(yield_.clone()),
            Self::UntapSource => SimEffect::UntapSelf,
            Self::ManaPerCounter(yield_) => SimEffect::ManaPerCounter(yield_.clone()),
            Self::CreateTokens(count) => SimEffect::Tokens(*count),
            Self::CreateTreasureTokens(count) => SimEffect::Treasures(*count),
            Self::CreateTokensPerOpponent { per_opponent } => SimEffect::TokensEachOpponent {
                per_opponent: *per_opponent,
            },
            Self::PutChargeCounters(count) => SimEffect::Counters(*count),
            Self::AdditionalLandPlay => SimEffect::ExtraLand,
            Self::ExileThenReturn => SimEffect::ExileThenReturnSource,
            Self::Monarch => SimEffect::Monarch,
            Self::Mill(count) => SimEffect::Mill(*count),
            Self::ReturnFromGraveyard { to_hand, count } => SimEffect::ReturnFromGraveyard {
                to_hand: *to_hand,
                count: *count,
            },
            Self::DiscardHandThenDraw => SimEffect::DiscardHandThenDrawSeven,
            Self::ExtraTurn => SimEffect::ExtraTurn,
            Self::Scry(count) => SimEffect::Scry(*count),
            Self::Surveil(count) => SimEffect::Surveil(*count),
            Self::LoseLife { amount, scope } => SimEffect::LoseLife {
                amount: *amount,
                scope: *scope,
            },
            Self::Damage {
                amount,
                target: super::oracle_ast::DamageTarget::Player,
                scope,
            } => SimEffect::Damage {
                amount: *amount,
                scope: *scope,
            },
            Self::Damage { .. } => return None,
            Self::Energy(amount) => SimEffect::Energy(*amount),
            Self::Amass(amount) => SimEffect::Amass(*amount),
            Self::RingTempts => SimEffect::RingTempts,
            Self::EmpowerJace(amount) => SimEffect::EmpowerJace(*amount),
            Self::Explore => SimEffect::Explore,
            Self::Connive(count) => SimEffect::Connive(*count),
            Self::Mobilize(amount) => SimEffect::Mobilize(*amount),
            Self::Afterlife(amount) => SimEffect::Afterlife(*amount),
            Self::AddBurdenCounter => SimEffect::AddBurdenCounter,
            Self::BurdenLifeLoss => SimEffect::BurdenLifeLoss,
            Self::Proliferate => SimEffect::Proliferate,
            Self::WinsAtCounters(counters) => SimEffect::WinThreshold {
                counters: *counters,
            },
            Self::Unsupported(_) => return None,
        })
    }
}

/// Convert parsed Saga chapters to the ordered effects consumed by the
/// simulator. Chapter I lands at index 0; combined chapter symbols
/// ("II, III") fill every listed slot with the same effects. A chapter
/// may resolve several effects; each slot holds the full list.
pub(super) fn lower_saga_chapters(oracle: &OracleCard) -> super::model::SimSaga {
    let mut effects: Vec<Vec<SimEffect>> = Vec::new();
    for chapter in oracle.saga_chapters() {
        let Some(last) = chapter.chapters.iter().max().copied() else {
            continue;
        };
        let chapter_effects: Vec<SimEffect> = chapter
            .effects
            .iter()
            .filter_map(OracleEffect::to_runtime)
            .collect();
        while effects.len() < last as usize {
            effects.push(Vec::new());
        }
        for ordinal in &chapter.chapters {
            if let Some(slot) = effects.get_mut(*ordinal as usize - 1) {
                slot.clone_from(&chapter_effects);
            }
        }
    }
    super::model::SimSaga { chapters: effects }
}
