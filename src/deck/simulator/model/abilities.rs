//! Runtime ability types and the costs, conditions, and subjects they carry.

use super::{Cost, SimEffect, SimTrigger};

/// Runtime kind of a parsed ability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SimAbilityKind {
    /// An activated ability with a payment and restrictions.
    Activated,
    /// An activated mana ability under CR 605.1a.
    ManaActivated,
    /// A supported continuous or state-based ability form.
    Static,
    /// An event-triggered ability.
    #[default]
    Triggered,
    /// A triggered mana ability under CR 605.1b.
    ManaTriggered,
}

impl SimAbilityKind {
    /// Whether this kind is activated by a player.
    pub fn is_activated(self) -> bool {
        matches!(self, Self::Activated | Self::ManaActivated)
    }

    /// Whether this kind listens for a game event.
    pub fn is_triggered(self) -> bool {
        matches!(self, Self::Triggered | Self::ManaTriggered)
    }
}

/// One executable ability: what fires it, what it costs, and what it does.
#[derive(Debug, Clone, Default)]
pub struct SimAbility {
    /// Stable position in the card's lowered ability list.
    pub id: usize,
    /// Whether this runtime node is activated, triggered, or static.
    pub kind: SimAbilityKind,
    /// Event for triggered abilities. Activated abilities use `Never`.
    pub trigger: SimTrigger,
    /// Payment and usage limits for an activated ability.
    pub activation: Option<SimActivation>,
    /// First supported effect, retained for single-effect readers.
    pub effect: SimEffect,
    /// Supported effects in Oracle resolution order.
    pub effects: Vec<SimEffect>,
    /// Once-per-turn limit on this triggered ability.
    pub once_per_turn: bool,
    /// Intervening-if condition. Unsupported conditions prevent lowering.
    pub condition: Option<SimAbilityCondition>,
    /// Permanent relationship required by an object-based trigger.
    pub event_subject: SimEventSubject,
}

impl SimAbility {
    /// Return effects in resolution order, including legacy single effects.
    pub fn effect_sequence(&self) -> &[SimEffect] {
        if self.effects.is_empty() {
            std::slice::from_ref(&self.effect)
        } else {
            &self.effects
        }
    }
}

/// Supported costs and limits for one activated ability.
#[derive(Debug, Clone, Default)]
pub struct SimActivation {
    /// Costs that must all be payable before any are paid.
    pub costs: Vec<SimActivationCost>,
    /// Limits on when or how often the activation may be used.
    pub restrictions: Vec<SimActivationRestriction>,
}

impl SimActivation {
    /// Combine mana components into the activation's payable mana cost.
    pub fn mana_cost(&self) -> Cost {
        self.costs
            .iter()
            .find_map(|cost| match cost {
                SimActivationCost::Mana(cost) => Some(*cost),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Whether the activation taps its source.
    pub fn taps_source(&self) -> bool {
        self.costs.contains(&SimActivationCost::TapSource)
    }

    /// Whether the activation untaps its source.
    pub fn untaps_source(&self) -> bool {
        self.costs.contains(&SimActivationCost::UntapSource)
    }

    /// Number of creatures sacrificed as the cost.
    pub fn creature_sacrifices(&self) -> u32 {
        self.costs
            .iter()
            .find_map(|cost| match cost {
                SimActivationCost::SacrificeCreatures(count) => Some(*count),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Whether the source permanent is sacrificed.
    pub fn sacrifices_source(&self) -> bool {
        self.costs.contains(&SimActivationCost::SacrificeSource)
    }

    /// Life paid as a cost.
    pub fn life_payment(&self) -> u32 {
        self.costs
            .iter()
            .find_map(|cost| match cost {
                SimActivationCost::PayLife(amount) => Some(*amount),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Energy counters paid as a cost.
    pub fn energy_payment(&self) -> u32 {
        self.costs
            .iter()
            .find_map(|cost| match cost {
                SimActivationCost::Energy(amount) => Some(*amount),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Signed loyalty change paid as a cost.
    pub fn loyalty_change(&self) -> i32 {
        self.costs
            .iter()
            .find_map(|cost| match cost {
                SimActivationCost::Loyalty(amount) => Some(*amount),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Number of charge counters removed as a cost.
    pub fn charge_counter_payment(&self) -> u32 {
        self.costs
            .iter()
            .find_map(|cost| match cost {
                SimActivationCost::RemoveChargeCounters(amount) => Some(*amount),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Whether this restriction applies.
    pub fn has_restriction(&self, restriction: SimActivationRestriction) -> bool {
        self.restrictions.contains(&restriction)
    }
}

/// One individually modeled activation cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimActivationCost {
    /// Mana paid from the pool.
    Mana(Cost),
    /// Tap the source permanent.
    TapSource,
    /// Untap the source permanent.
    UntapSource,
    /// Sacrifice one or more creatures.
    SacrificeCreatures(u32),
    /// Sacrifice the source permanent.
    SacrificeSource,
    /// Pay life.
    PayLife(u32),
    /// Pay energy counters.
    Energy(u32),
    /// Remove charge counters from the source.
    RemoveChargeCounters(u32),
    /// Add or remove loyalty counters.
    Loyalty(i32),
}

/// Restriction on an activated ability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimActivationRestriction {
    /// Use only once each turn.
    OncePerTurn,
    /// Use only once each game.
    OncePerGame,
    /// Power-up: reduce the cost by the permanent's mana cost on entry turn.
    PowerUp,
}

/// A modeled intervening-if condition (CR 603.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimAbilityCondition {
    /// Descend N: N or more permanent cards in the graveyard.
    Descend(u32),
    /// Threshold: seven or more cards in the graveyard.
    Threshold,
    /// Raid: a creature attacked this turn.
    Raid,
    /// Ferocious: the player controls a creature with power 4 or more.
    Ferocious,
    /// Metalcraft: the player controls three or more artifacts.
    Metalcraft,
}

/// Relationship between a trigger event and its source permanent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SimEventSubject {
    /// The event concerns the permanent with this ability.
    This,
    /// The event concerns a different permanent.
    Another,
    /// The event may concern this or another permanent.
    #[default]
    Any,
}
