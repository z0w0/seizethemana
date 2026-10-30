//! Lower Oracle syntax nodes into simulator runtime data.

use super::model::{
    OpponentTax, SimAbility, SimAbilityCondition, SimAbilityKind, SimActivation, SimActivationCost,
    SimActivationRestriction, SimCard, SimEffect, SimEventFilter, SimEventSubject,
    SimResolutionPayment, SimStriation, SimTrigger,
};
use super::oracle_ast::{
    AbilityRestriction, ActivationCost, ActivationTarget, CostObject, ObjectSubject, OracleAbility,
    OracleActivatedAbility, OracleCard, OracleEffect, OracleResolutionPayment, OracleStaticAbility,
    OracleStaticEffect, OracleStations, OracleTaxAmount, OracleTriggerEvent,
    OracleTriggeredAbility, PlayerScope,
};
use crate::db::CardRow;

/// Explicit lowering for Behold qualities and linked effects.
mod behold;
/// Lower card-row fields and AST nodes into the executable card model.
mod card;
/// Lower one-shot spell data from parsed syntax.
mod cast_riders;
/// Live generic spell-cost reducers with complete selectors.
mod cost_reduction;
/// Lower keyword AST nodes into runtime card data.
mod keyword_lower;
/// Complete multiple-result search plans and placement bounds.
mod search;
/// Lower interaction and static card diagnostics.
mod static_flags;
use keyword_lower::lower_keywords;

/// Compiler regressions for preserved requirements and source provenance.
#[cfg(test)]
#[path = "tests/oracle_lower_tests.rs"]
mod oracle_lower_tests;

/// Independent lowering checks for complete fixed activation payments.
#[cfg(test)]
#[path = "tests/activation_payment_lower_tests.rs"]
mod activation_payment_lower_tests;

/// Hand-built effect lowering and pinned source integration controls.
#[cfg(test)]
#[path = "tests/card_lower_tests.rs"]
mod card_lower_tests;

/// Independent typed checks for complete Escape payment data.
#[cfg(test)]
#[path = "tests/escape_lower_tests.rs"]
mod escape_lower_tests;

/// Independent typed checks for search constraints and atomic rejection.
#[cfg(test)]
#[path = "tests/search_filter_lower_tests.rs"]
mod search_filter_lower_tests;

/// Atomic rejection of unsupported conditional token replacements.
#[cfg(test)]
#[path = "tests/token_replacement_lower_tests.rs"]
mod token_replacement_lower_tests;

/// Pinned Oracle regressions for complete cast costs and ordered consequences.
#[cfg(test)]
#[path = "tests/spell_runtime_tests.rs"]
mod spell_runtime_tests;

/// Pinned grammar controls for typed creature quantities and written payments.
#[cfg(test)]
#[path = "tests/tribal_parser_tests.rs"]
mod tribal_parser_tests;

/// Scoped lord, mana grant, untap, and equipment relationship grammar controls.
#[cfg(test)]
#[path = "tests/tribal_scope_tests.rs"]
mod tribal_scope_tests;

/// Build simulator card data from one stored card row.
pub fn parse_sim_card(row: &CardRow) -> SimCard {
    let oracle = super::oracle_parser::parse_oracle_card(row);
    let mut card = card::lower_oracle_card(row, &oracle);
    card.source_hash = super::support::card_source_hash(row);
    for (support, ability) in card.support.iter_mut().zip(&oracle.abilities) {
        if let Some(reason) = ability.unsupported_runtime_reason() {
            support.status = super::support::AbilitySupportStatus::Unsupported {
                reason: reason.to_string(),
            };
        }
    }
    card
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
            .filter(|ability| !self.static_data.unsupported_cumulative_upkeep
                || !matches!(ability, OracleAbility::Triggered(trigger)
                    if matches!(trigger.event, OracleTriggerEvent::OpponentDraw | OracleTriggerEvent::OpponentSpellCast { .. })))
            .flat_map(OracleAbility::to_runtime)
            .collect()
    }
}

impl OracleAbility {
    /// Explain typed requirements that must stay inert until the runtime can use them.
    pub(super) fn unsupported_runtime_reason(&self) -> Option<&'static str> {
        match self {
            Self::Activated(ability) => {
                if ability
                    .costs
                    .iter()
                    .any(|cost| matches!(cost, ActivationCost::Tribal(_)))
                {
                    return Some(
                        "Selected-permanent tap and return-to-hand costs are parsed but cannot be paid by the runtime.",
                    );
                }
                typed_effect_runtime_reason(&ability.effects)
            }
            Self::Triggered(ability) => {
                if ability.event == OracleTriggerEvent::EquippedCreatureDies {
                    return Some(
                        "The equipped-creature death relationship is parsed but has no runtime event dispatch.",
                    );
                }
                typed_effect_runtime_reason(&ability.effects)
            }
            Self::Static(ability) => ability.effects.iter().find_map(|effect| match effect {
                OracleStaticEffect::Tribal(
                    super::oracle_ast::tribal::OracleTribalStaticEffect::CreatureBuff { .. },
                ) => {
                    Some("Scoped creature bonuses are parsed but have no runtime selector support.")
                }
                OracleStaticEffect::Tribal(
                    super::oracle_ast::tribal::OracleTribalStaticEffect::ManaGrant { .. },
                ) => {
                    Some("Complete granted mana abilities are parsed but have no runtime lowering.")
                }
                _ => None,
            }),
            Self::Spell(ability) => typed_effect_runtime_reason(&ability.effects),
            Self::SagaChapter(ability) => typed_effect_runtime_reason(&ability.effects),
            _ => None,
        }
    }

    /// Lower a complete ability without dropping required costs or effects.
    pub(super) fn to_runtime(&self) -> Vec<SimAbility> {
        match self {
            Self::Activated(ability) => ability.to_runtime_all(),
            Self::Triggered(ability) => ability.to_runtime_all(),
            Self::Static(ability) => ability.to_runtime().into_iter().collect(),
            Self::Spell(_) | Self::SagaChapter(_) | Self::Unsupported(_) => Vec::new(),
        }
    }
}

/// Reject creature and mana rules that cannot execute their full effects.
fn typed_effect_runtime_reason(effects: &[OracleEffect]) -> Option<&'static str> {
    effects.iter().find_map(|effect| match effect {
        OracleEffect::Tribal(
            super::oracle_ast::tribal::OracleTribalEffect::CreateCreatureTokens {
                quantity: super::oracle_ast::tribal::OracleQuantity::Fixed(_),
                token,
            },
        ) if token.power == 1 && token.toughness == 1 => None,
        OracleEffect::Tribal(super::oracle_ast::tribal::OracleTribalEffect::CreateCreatureTokens { .. }) =>
            Some("Token quantities and creature characteristics are parsed but have no complete runtime token model."),
        OracleEffect::Tribal(super::oracle_ast::tribal::OracleTribalEffect::Mana { .. }) =>
            Some("Matching-permanent and source-power mana quantities are parsed but have no runtime lowering."),
        OracleEffect::Tribal(super::oracle_ast::tribal::OracleTribalEffect::Untap(_)) =>
            Some("Selected-permanent untap effects are parsed but have no runtime target selection."),
        _ => None,
    })
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
        let Some(condition) = activation_condition(self) else {
            return Vec::new();
        };
        if !self.costs.iter().all(activation_cost_is_supported) {
            return Vec::new();
        }
        let Some(activation) = lower_activation(self) else {
            return Vec::new();
        };
        let Some(effects) = lower_activation_effects(self) else {
            return Vec::new();
        };
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
            activation: Some(activation),
            once_per_turn: false,
            condition,
            event_subject: SimEventSubject::Any,
            ..SimAbility::default()
        }]
    }
}

/// Lower complete activation predicates without dropping an extra condition.
fn activation_condition(ability: &OracleActivatedAbility) -> Option<Option<SimAbilityCondition>> {
    let mut condition = None;
    for restriction in &ability.restrictions {
        if let AbilityRestriction::ControlsLandTypes(types) = restriction {
            if !ability.is_mana_ability || condition.is_some() {
                return None;
            }
            condition = Some(SimAbilityCondition::ControlsLandTypes(*types));
        }
        if let AbilityRestriction::Condition(text) = restriction {
            if !ability.is_mana_ability
                || condition.is_some()
                || condition_from(text) != Some(SimAbilityCondition::Metalcraft)
            {
                return None;
            }
            condition = Some(SimAbilityCondition::Metalcraft);
        }
    }
    Some(condition)
}

/// A typed Metalcraft condition accounts for its matching rule sentence.
pub(super) fn lower_activation_effects(ability: &OracleActivatedAbility) -> Option<Vec<SimEffect>> {
    let condition = activation_condition(ability)?;
    let effects: Vec<_> = ability.effects.iter().filter(|effect| {
        !matches!(effect, OracleEffect::Unsupported(text)
            if condition == Some(SimAbilityCondition::Metalcraft)
                && text.eq_ignore_ascii_case("activate only if you control three or more artifacts"))
    }).cloned().collect();
    lower_effect_sequence_with_fixed_tokens(&effects)
}

/// Sum fixed life payments for the runtime's single life-cost entry.
/// Repeated sacrifice and counter payments stay unsupported as complete costs.
/// Compound loyalty payments also stay inert.
fn lower_activation(ability: &OracleActivatedAbility) -> Option<SimActivation> {
    if ability.costs.len() != 1
        && ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Loyalty(_)))
    {
        return None;
    }
    if ability
        .costs
        .iter()
        .filter(|cost| matches!(cost, ActivationCost::RemoveCounter { .. }))
        .count()
        > 1
        || ability
            .costs
            .iter()
            .filter(|cost| matches!(cost, ActivationCost::Sacrifice { .. }))
            .count()
            > 1
    {
        return None;
    }
    let life_payment = activation_life_payment(&ability.costs)?;
    let mut life_lowered = false;
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
                exclude_source,
            } => Some(SimActivationCost::SacrificeCreatures {
                count: *count,
                exclude_source: *exclude_source,
            }),
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Source,
                exclude_source: false,
            } => Some(SimActivationCost::SacrificeSource),
            ActivationCost::PayLife(_) if !life_lowered => {
                life_lowered = true;
                Some(SimActivationCost::PayLife(life_payment))
            }
            ActivationCost::PayLife(_) => None,
            ActivationCost::Energy(amount) => Some(SimActivationCost::Energy(*amount)),
            ActivationCost::RemoveCounter { count, .. } => {
                Some(SimActivationCost::RemoveChargeCounters(*count))
            }
            ActivationCost::Loyalty(amount) => Some(SimActivationCost::Loyalty(*amount)),
            ActivationCost::Tap(ActivationTarget::AnotherObject)
            | ActivationCost::Untap(ActivationTarget::AnotherObject)
            | ActivationCost::Sacrifice { .. }
            | ActivationCost::Discard(_)
            | ActivationCost::Other(_)
            | ActivationCost::Tribal(_) => None,
        })
        .collect();
    let restrictions = ability
        .restrictions
        .iter()
        .filter_map(|restriction| match restriction {
            AbilityRestriction::OncePerTurn => Some(SimActivationRestriction::OncePerTurn),
            AbilityRestriction::OncePerGame => Some(SimActivationRestriction::OncePerGame),
            AbilityRestriction::PowerUp => Some(SimActivationRestriction::PowerUp),
            AbilityRestriction::SorcerySpeed
            | AbilityRestriction::Condition(_)
            | AbilityRestriction::ControlsLandTypes(_) => None,
        })
        .collect();
    Some(SimActivation {
        costs,
        restrictions,
    })
}

/// Keep the complete fixed and Phyrexian life charge within the runtime's signed life range.
fn activation_life_payment(costs: &[ActivationCost]) -> Option<u32> {
    let life_payment = costs.iter().try_fold(0_u32, |total, cost| match cost {
        ActivationCost::PayLife(amount) => total.checked_add(*amount),
        _ => Some(total),
    })?;
    let phyrexian_payment = costs
        .iter()
        .find_map(|cost| match cost {
            ActivationCost::Mana(cost) => Some(
                2 * cost
                    .phyrexian
                    .iter()
                    .map(|count| u32::from(*count))
                    .sum::<u32>(),
            ),
            _ => None,
        })
        .unwrap_or(0);
    (life_payment.checked_add(phyrexian_payment)? <= i32::MAX as u32).then_some(life_payment)
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
            object: CostObject::Creature,
            ..
        }
        | ActivationCost::Sacrifice {
            count: 1,
            object: CostObject::Source,
            exclude_source: false,
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
        | ActivationCost::Other(_)
        | ActivationCost::Tribal(_) => false,
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
            Self::OpponentDraw => Some(SimTrigger::OpponentDraw),
            Self::OpponentSpellCast {
                noncreature: false,
                first: false,
            } => Some(SimTrigger::OpponentSpellCast),
            Self::OpponentSpellCast {
                noncreature: true,
                first: false,
            } => Some(SimTrigger::OpponentNoncreatureCast),
            Self::OpponentSpellCast {
                noncreature: true,
                first: true,
            } => Some(SimTrigger::OpponentFirstNoncreatureCast),
            Self::OpponentSpellCast {
                noncreature: false,
                first: true,
            } => None,
            Self::Dies(_) => Some(SimTrigger::Dies),
            Self::TappedForMana => Some(SimTrigger::TappedForMana),
            // "Whenever the Ring tempts you" (CR 701.54d) fires when the
            // Ring-tempts counter moves.
            Self::RingTempts => Some(SimTrigger::RingTempts),
            // A state trigger on a counter threshold normalizes to the
            // upkeep win check: the census reads counters, not events.
            Self::WinsAtCounters { .. } => Some(SimTrigger::Upkeep),
            Self::EquippedCreatureDies
            | Self::AnyOf(_)
            | Self::CastsSpell { this_spell: true }
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
        if self.event_filter.unsupported.is_some() {
            return Vec::new();
        }
        if let OracleTriggerEvent::AnyOf(events) = &self.event {
            if self.once_per_turn {
                return Vec::new();
            }
            let mut abilities = Vec::new();
            for event in events {
                let mut branch = self.clone();
                branch.event = event.clone();
                let lowered = branch.to_runtime_all();
                if lowered.is_empty() {
                    return Vec::new();
                }
                abilities.extend(lowered);
            }
            return abilities;
        }
        if matches!(
            self.resolution_payment,
            Some(OracleResolutionPayment::PayMana(_))
        ) && !matches!(
            self.event,
            OracleTriggerEvent::CastsSpell { this_spell: false }
        ) {
            return Vec::new();
        }
        if matches!(self.event, OracleTriggerEvent::OpponentSpellCast { .. })
            && (!self.event_filter.card_types.is_empty() || !self.event_filter.subtypes.is_empty())
        {
            return Vec::new();
        }
        let Some((resolution_payment, opponent_tax)) =
            lower_resolution_payment(&self.resolution_payment)
        else {
            return Vec::new();
        };
        if let OracleTriggerEvent::WinsAtCounters { counters } = self.event {
            return self.lower_counter_threshold(counters).into_iter().collect();
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
        let Some(effects) = lower_effect_sequence_with_fixed_tokens(&self.effects) else {
            return Vec::new();
        };
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
            event_filter: SimEventFilter {
                card_types: self.event_filter.card_types.clone(),
                subtypes: self.event_filter.subtypes.clone(),
                noncreature: self.event_filter.noncreature,
                nontoken: self.event_filter.nontoken,
                first_each_turn: self.event_filter.first_each_turn,
            },
            resolution_payment,
            opponent_tax,
            ..SimAbility::default()
        }]
    }

    /// Lower a bare counter-threshold win without dropping other requirements.
    fn lower_counter_threshold(&self, counters: u32) -> Option<SimAbility> {
        if self.condition.is_some()
            || self.resolution_payment.is_some()
            || !matches!(self.effects.as_slice(), [OracleEffect::Unsupported(text)] if text.trim_end_matches('.').eq_ignore_ascii_case("you win the game"))
        {
            return None;
        }
        Some(SimAbility {
            trigger: SimTrigger::Upkeep,
            effect: SimEffect::WinThreshold { counters },
            once_per_turn: self.once_per_turn,
            event_subject: self.event.event_subject(),
            ..SimAbility::default()
        })
    }
}

/// Preserve the payer and full amount of each supported resolution payment.
fn lower_resolution_payment(
    payment: &Option<OracleResolutionPayment>,
) -> Option<(Option<SimResolutionPayment>, Option<OpponentTax>)> {
    match payment {
        Some(OracleResolutionPayment::PayMana(cost)) if !cost.unsupported => {
            Some((Some(SimResolutionPayment::PayMana(*cost)), None))
        }
        Some(OracleResolutionPayment::OpponentTax(OracleTaxAmount::Mana(cost)))
            if *cost
                == super::model::Cost {
                    generic: cost.generic,
                    ..Default::default()
                } =>
        {
            Some((None, Some(OpponentTax::Fixed(cost.generic))))
        }
        Some(OracleResolutionPayment::OpponentTax(OracleTaxAmount::SourcePower)) => {
            Some((None, Some(OpponentTax::SourcePower)))
        }
        Some(
            OracleResolutionPayment::Unsupported(_)
            | OracleResolutionPayment::OpponentTax(_)
            | OracleResolutionPayment::PayMana(_),
        ) => None,
        None => Some((None, None)),
    }
}

/// Lower a resolution atomically, except for already-modeled rule annotations.
pub(super) fn lower_spell_effect_sequence(
    oracle: &OracleCard,
    effects: &[OracleEffect],
) -> Option<Vec<SimEffect>> {
    let resolution: Vec<_> = effects
        .iter()
        .filter(|effect| !modeled_spell_annotation(oracle, effect))
        .cloned()
        .collect();
    lower_effect_sequence_with_fixed_tokens(&resolution)
}

/// Linked searches and self-exile resolve through their typed spell fields.
fn modeled_spell_annotation(oracle: &OracleCard, effect: &OracleEffect) -> bool {
    match effect {
        OracleEffect::SacrificeSearch(rule) => {
            oracle.spell_data.search_after_sacrifice == Some(*rule)
        }
        OracleEffect::Unsupported(text) => {
            oracle.spell_data.exiles_on_resolve
                && matches!(
                    text.trim_end_matches('.').to_ascii_lowercase().as_str(),
                    "exile this spell" | "exile this permanent"
                )
        }
        _ => false,
    }
}

/// Lower a resolution atomically, except for already-modeled rule annotations.
pub(super) fn lower_effect_sequence(effects: &[OracleEffect]) -> Option<Vec<SimEffect>> {
    let mut lowered = Vec::new();
    for effect in effects {
        if let OracleEffect::Unsupported(text) = effect
            && modeled_annotation(text, effects)
        {
            continue;
        }
        lowered.push(effect.to_runtime()?);
    }
    Some(lowered)
}

/// Lower fixed token counts where a one-shot or activated effect creates bodies.
pub(super) fn lower_effect_sequence_with_fixed_tokens(
    effects: &[OracleEffect],
) -> Option<Vec<SimEffect>> {
    let mut lowered = Vec::new();
    for effect in effects {
        if let OracleEffect::Unsupported(text) = effect
            && modeled_annotation(text, effects)
        {
            continue;
        }
        let runtime = match effect {
            OracleEffect::Tribal(
                super::oracle_ast::tribal::OracleTribalEffect::CreateCreatureTokens {
                    quantity: super::oracle_ast::tribal::OracleQuantity::Fixed(count),
                    token,
                },
            ) if token.power == 1 && token.toughness == 1 => SimEffect::Tokens(*count),
            _ => effect.to_runtime()?,
        };
        lowered.push(runtime);
    }
    Some(lowered)
}

/// Recognize text whose rule is retained by typed costs or mana restrictions.
fn modeled_annotation(text: &str, effects: &[OracleEffect]) -> bool {
    let text = text.trim().to_ascii_lowercase();
    if matches!(
        text.as_str(),
        "activate only as a sorcery"
            | "activate only once each turn"
            | "activate only once each game"
            | "this ability triggers only once each turn"
    ) {
        return true;
    }
    if text == "choose a color" {
        return effects.iter().any(|effect| {
            matches!(effect, OracleEffect::Mana(yield_)
                if yield_.one_color
                    && yield_.scaling == Some(super::model::Scale::PerChargeCounter))
        });
    }
    let restriction = super::oracle_parser::land::spend_restriction(&text);
    let nongeneric = text == "this mana can't be spent to pay generic mana costs";
    (restriction.is_some() || nongeneric)
        && effects.iter().any(|effect| {
            matches!(effect, OracleEffect::Mana(yield_) | OracleEffect::ManaPerCounter(yield_)
            if restriction.is_some() && yield_.restriction == restriction
                || nongeneric && yield_.cannot_pay_generic)
        })
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
            Self::Behold { spec, effects } => behold::lower_conditional(spec, effects)?,
            Self::SearchThenBeholdUntap { search, spec } => {
                behold::lower_search_then_untap(search, spec)?
            }
            Self::Tribal(_) => return None,
            Self::MultiSearch(search) => {
                SimEffect::MultiSearch(search::lower_multi_search(search)?)
            }
            Self::Draw(count) => SimEffect::Draw(*count),
            Self::LookAtTop { count, may_shuffle } => SimEffect::LookAtTop {
                count: *count,
                may_shuffle: *may_shuffle,
            },
            Self::PutHandOnTop(count) => SimEffect::PutHandOnTop(*count),
            Self::DrawThenDiscard(count) => SimEffect::DrawThenDiscard(*count),
            Self::DrawDiscard { draw, discard } => SimEffect::DrawDiscard {
                draw: *draw,
                discard: *discard,
            },
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
            Self::SacrificeSearch(_) | Self::Unsupported(_) => return None,
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
        let chapter_effects =
            lower_effect_sequence_with_fixed_tokens(&chapter.effects).unwrap_or_default();
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
