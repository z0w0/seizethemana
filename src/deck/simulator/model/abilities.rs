//! Runtime ability types and the costs, conditions, and subjects they carry.

use super::{Cost, SimCard, SimEffect, SimTrigger};

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
    /// Required properties of the spell or permanent that caused the event.
    pub event_filter: SimEventFilter,
    /// Optional resolution payment required before the effects occur.
    pub resolution_payment: Option<SimResolutionPayment>,
    /// Payment the triggering opponent may make to prevent these effects.
    pub opponent_tax: Option<OpponentTax>,
}

impl SimAbility {
    /// Return the ordered effects, or the standalone effect when no sequence is set.
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
                SimActivationCost::SacrificeCreatures { count, .. } => Some(*count),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Whether a creature sacrifice requires a permanent other than the source.
    pub fn creature_sacrifice_excludes_source(&self) -> bool {
        self.costs.iter().any(|cost| {
            matches!(
                cost,
                SimActivationCost::SacrificeCreatures {
                    exclude_source: true,
                    ..
                }
            )
        })
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

    /// Whether the activation pays a loyalty symbol, including a zero symbol.
    pub fn has_loyalty_cost(&self) -> bool {
        self.costs
            .iter()
            .any(|cost| matches!(cost, SimActivationCost::Loyalty(_)))
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
    SacrificeCreatures {
        /// Number of distinct creatures required.
        count: u32,
        /// The printed cost requires another creature.
        exclude_source: bool,
    },
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
    /// Control a physical land with any listed basic land type.
    ControlsLandTypes([bool; 5]),
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

/// Properties required of an event object. All listed types must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimEventFilter {
    /// Required card types, stored as lowercase Oracle words.
    pub card_types: Vec<String>,
    /// Required creature or permanent subtypes, stored in lowercase.
    pub subtypes: Vec<String>,
    /// The event object must not be a creature.
    pub noncreature: bool,
    /// The event object must not be a token.
    pub nontoken: bool,
    /// Only the first matching event from that player each turn can trigger.
    pub first_each_turn: bool,
}

impl SimEventFilter {
    /// Match object properties. The event dispatcher must check turn history.
    pub fn matches_card(&self, card: &SimCard, is_token: bool) -> bool {
        (!self.noncreature || !card.is_creature)
            && (!self.nontoken || !is_token)
            && self
                .card_types
                .iter()
                .all(|kind| card.card_types.contains(kind))
            && self
                .subtypes
                .iter()
                .all(|kind| card.subtypes.contains(kind))
    }
}

/// Payment offered when a triggered ability resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimResolutionPayment {
    /// The controller may pay this cost to receive the effects.
    PayMana(Cost),
}

/// Mana amount requested by an opponent tax trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpponentTax {
    /// A fixed generic mana payment.
    Fixed(u32),
    /// Generic mana equal to the source's power when the ability resolves.
    SourcePower,
}
