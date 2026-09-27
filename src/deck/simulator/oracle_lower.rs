//! Lower Oracle syntax nodes into simulator runtime data.

use super::model::{Ability, AbilityTiming, Effect};
use super::oracle_ast::{
    AbilityRestriction, ActivatedAbility, ActivationCost, ActivationTarget, CostObject,
    OracleAbility, OracleCard, OracleEffect, PlayerScope, StaticAbility, StaticEffect,
    TriggerEvent, TriggeredAbility, TurnStep,
};

impl OracleCard {
    /// Lower abilities the current game model can execute.
    pub(super) fn runtime_abilities(&self) -> Vec<Ability> {
        self.abilities
            .iter()
            .filter_map(OracleAbility::to_runtime)
            .collect()
    }
}

impl OracleAbility {
    /// Lower executable ability nodes into the simulator's turn-loop model.
    pub(super) fn to_runtime(&self) -> Option<Ability> {
        match self {
            Self::Activated(ability) => ability.to_runtime(),
            Self::Triggered(ability) => ability.to_runtime(),
            Self::Static(ability) => ability.to_runtime(),
            Self::Spell(_) | Self::SagaChapter(_) | Self::Unsupported(_) => None,
        }
    }
}

impl ActivatedAbility {
    /// Lower a supported activation to the existing simulator model.
    pub(super) fn to_runtime(&self) -> Option<Ability> {
        if self
            .restrictions
            .iter()
            .any(|restriction| matches!(restriction, AbilityRestriction::Condition(_)))
        {
            return None;
        }
        let effect = self.effects.iter().find_map(OracleEffect::to_runtime)?;
        let mana_cost = self.costs.iter().find_map(|cost| match cost {
            ActivationCost::Mana(cost) => Some(cost.clone()),
            _ => None,
        });
        let loyalty_cost = self.costs.iter().find_map(|cost| match cost {
            ActivationCost::Loyalty(amount) if *amount < 0 => Some(amount.unsigned_abs()),
            _ => None,
        });
        let loyalty_gain = self.costs.iter().find_map(|cost| match cost {
            ActivationCost::Loyalty(amount) if *amount > 0 => Some(*amount as u32),
            _ => None,
        });
        Some(Ability {
            trigger: AbilityTiming::Activated,
            cost: mana_cost.unwrap_or_default(),
            effect,
            condition: None,
            taps: self
                .costs
                .iter()
                .any(|cost| matches!(cost, ActivationCost::Tap(ActivationTarget::Source))),
            uses_counters: self
                .costs
                .iter()
                .any(|cost| matches!(cost, ActivationCost::RemoveCounter { .. })),
            sacrifice_bodies: self
                .costs
                .iter()
                .find_map(|cost| match cost {
                    ActivationCost::Sacrifice {
                        count,
                        object: CostObject::Creature,
                    } => Some(*count),
                    _ => None,
                })
                .unwrap_or(0),
            once_per_turn: self
                .restrictions
                .iter()
                .any(|restriction| matches!(restriction, AbilityRestriction::OncePerTurn)),
            loyalty_cost: loyalty_cost.unwrap_or(0),
            loyalty_gain: loyalty_gain.unwrap_or(0),
            life_cost: self
                .costs
                .iter()
                .find_map(|cost| match cost {
                    ActivationCost::PayLife(amount) => Some(*amount),
                    _ => None,
                })
                .unwrap_or(0),
        })
    }
}

impl StaticAbility {
    /// Lower the counter-threshold rule that the upkeep census can check.
    pub(super) fn to_runtime(&self) -> Option<Ability> {
        let counters = self.effects.iter().find_map(|effect| match effect {
            StaticEffect::WinThreshold(counters) => Some(*counters),
            _ => None,
        })?;
        Some(Ability {
            trigger: AbilityTiming::OnUpkeep,
            effect: Effect::WinThreshold { counters },
            ..Ability::default()
        })
    }
}

impl TriggerEvent {
    /// Map the parsed event to a simulator trigger when its timing is modeled.
    pub(super) fn to_runtime(&self) -> Option<AbilityTiming> {
        match self {
            Self::Enters(_) => Some(AbilityTiming::OnEnter),
            Self::LandEnters(PlayerScope::You) => Some(AbilityTiming::OnLandfall),
            Self::BeginningOfStep {
                step: TurnStep::Upkeep,
                player: PlayerScope::You,
            } => Some(AbilityTiming::OnUpkeep),
            Self::BeginningOfStep {
                step: TurnStep::EndStep,
                player: PlayerScope::You,
            } => Some(AbilityTiming::OnEndStep),
            Self::BeginningOfStep {
                step: TurnStep::FirstMainPhase,
                player: PlayerScope::You,
            } => Some(AbilityTiming::PrecombatMainPhase),
            Self::Attacks(_) => Some(AbilityTiming::OnAttack),
            Self::CombatDamageToPlayer(_) => Some(AbilityTiming::OnCombatDamage),
            Self::CastsSpell { this_spell: false } => Some(AbilityTiming::OnCastSpell),
            Self::Dies(_) => Some(AbilityTiming::OnDeath),
            Self::CastsSpell { this_spell: true }
            | Self::TappedForMana
            | Self::BeginningOfStep { .. }
            | Self::LandEnters(PlayerScope::Opponent | PlayerScope::Any)
            | Self::Other(_) => None,
        }
    }
}

impl TriggeredAbility {
    /// Lower a supported trigger to the simulator's turn-loop model.
    pub(super) fn to_runtime(&self) -> Option<Ability> {
        let trigger = self.event.to_runtime()?;
        let effect = self
            .effects
            .iter()
            .find(|effect| matches!(effect, OracleEffect::Draw(_) | OracleEffect::Loot(_)))
            .and_then(OracleEffect::to_runtime)
            .or_else(|| self.effects.iter().find_map(OracleEffect::to_runtime))?;
        Some(Ability {
            trigger,
            effect,
            once_per_turn: self.once_per_turn,
            condition: self.condition.clone(),
            ..Ability::default()
        })
    }
}

impl OracleEffect {
    /// Convert a supported syntax effect to the executable game model.
    pub(super) fn to_runtime(&self) -> Option<Effect> {
        Some(match self {
            Self::Draw(count) => Effect::Draw(*count),
            Self::Loot(count) => Effect::Loot(*count),
            Self::DrawAndMinusCounter => Effect::DrawAndMinusCounter,
            Self::GainLife(amount) => Effect::GainLife(*amount),
            Self::Search(spec) => Effect::Search(*spec),
            Self::Mana(yield_) => Effect::Mana(yield_.clone()),
            Self::UntapSelf => Effect::UntapSelf,
            Self::ManaPerCounter(yield_) => Effect::ManaPerCounter(yield_.clone()),
            Self::Tokens(count) => Effect::Tokens(*count),
            Self::Counters(count) => Effect::Counters(*count),
            Self::ExtraLand => Effect::ExtraLand,
            Self::Blink => Effect::Blink,
            Self::Monarch => Effect::Monarch,
            Self::Mill(count) => Effect::Mill(*count),
            Self::ReturnFromGraveyard { to_hand, count } => Effect::ReturnFromGraveyard {
                to_hand: *to_hand,
                count: *count,
            },
            Self::Wheel => Effect::Wheel,
            Self::ExtraTurn => Effect::ExtraTurn,
            Self::Look {
                count,
                surveil: false,
            } => Effect::Scry(*count),
            Self::Look {
                count,
                surveil: true,
            } => Effect::Surveil(*count),
            Self::Drain(amount) => Effect::Drain(*amount),
            Self::WinThreshold(counters) => Effect::WinThreshold {
                counters: *counters,
            },
            Self::Unsupported(_) => return None,
        })
    }
}

/// Convert parsed Saga chapters to the ordered effects consumed by the
/// simulator. Chapter I lands at index 0; combined chapter symbols
/// ("II, III") fill every listed slot with the same effect.
pub(super) fn lower_saga_chapters(oracle: &OracleCard) -> super::model::SagaData {
    let mut effects = Vec::new();
    for chapter in oracle.saga_chapters() {
        let Some(last) = chapter.chapters.iter().max().copied() else {
            continue;
        };
        let effect = chapter
            .effects
            .iter()
            .find_map(OracleEffect::to_runtime)
            .unwrap_or(Effect::None);
        while effects.len() < last as usize {
            effects.push(Effect::None);
        }
        for ordinal in &chapter.chapters {
            if let Some(slot) = effects.get_mut(*ordinal as usize - 1) {
                *slot = effect.clone();
            }
        }
    }
    super::model::SagaData { chapters: effects }
}
