//! Typed quantities, battlefield selections, and creature rules awaiting runtime support.

#![allow(dead_code)] // Parser clients need these fields; runtime lowering rejects them until its data model can use them.

use super::{OracleActivatedAbility, OracleEventFilter, PlayerScope};
use crate::deck::simulator::model::{ManaColor, ManaYield};

/// A fixed amount or a value read from the source event or battlefield.
#[derive(Debug, Clone)]
pub enum OracleQuantity {
    /// The written number, including zero when the wording permits it.
    Fixed(u32),
    /// Combat damage from the event that caused this ability to trigger.
    TriggerCombatDamage,
    /// Number of battlefield permanents that match every selector requirement.
    MatchingPermanents(OraclePermanentSelector),
    /// Power of the permanent with this ability.
    SourcePower,
}

/// Requirements for battlefield permanents, not cards in other zones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OraclePermanentSelector {
    /// All type and subtype requirements must match.
    pub filter: OracleEventFilter,
    /// `Any` includes every player's battlefield, not only the controller's.
    pub controller: PlayerScope,
    /// Exclude the permanent with this ability.
    pub exclude_source: bool,
    /// The selected permanent must be untapped before paying the cost.
    pub untapped: bool,
}

/// One chosen target or every matching battlefield permanent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OraclePermanentSelection {
    /// One target that meets every selector requirement.
    Target(OraclePermanentSelector),
    /// Every matching permanent; this does not target them.
    Each(OraclePermanentSelector),
}

/// Printed characteristics and entry instructions for a creature token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleCreatureToken {
    /// Printed power before counters or continuous effects.
    pub power: u32,
    /// Printed toughness before counters or continuous effects.
    pub toughness: u32,
    /// Printed colors; an empty list means colorless.
    pub colors: Vec<ManaColor>,
    /// Every printed creature subtype, in lowercase Oracle order.
    pub subtypes: Vec<String>,
    /// Whether the instruction makes the token enter tapped.
    pub tapped: bool,
    /// Whether the instruction makes the token enter attacking.
    pub attacking: bool,
}

/// Complete creature and mana effects that do not yet have runtime lowering.
#[derive(Debug, Clone)]
pub enum OracleTribalEffect {
    /// Create tokens with all printed creature characteristics.
    CreateCreatureTokens {
        /// Number of tokens to create.
        quantity: OracleQuantity,
        /// Characteristics shared by the created tokens.
        token: OracleCreatureToken,
    },
    /// Produce the complete mana bundle once for each unit of the quantity.
    Mana {
        /// Fixed, simultaneous, or choice mana produced per unit.
        yield_: ManaYield,
        /// Multiplier from the battlefield or source, not a fixed fallback.
        quantity: OracleQuantity,
    },
    /// Untap only the stated source, target, or matching permanents.
    Untap(OraclePermanentSelection),
}

/// Written payments that select battlefield permanents rather than use `{T}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OracleTribalCost {
    /// Tap distinct matching untapped permanents; summoning sickness does not apply.
    TapPermanents {
        /// Number of distinct permanents required.
        count: u32,
        /// Complete battlefield and source-exclusion requirements.
        selector: OraclePermanentSelector,
    },
    /// Return matching battlefield permanents to their owners' hands.
    ReturnToHand {
        /// Number of permanents returned as payment.
        count: u32,
        /// Type, controller, and source-exclusion requirements.
        selector: OraclePermanentSelector,
    },
}

/// Scoped continuous rules whose target set or mana ability is not executable yet.
#[derive(Debug, Clone)]
pub enum OracleTribalStaticEffect {
    /// Change power and toughness only for the matching battlefield permanents.
    CreatureBuff {
        /// Matching permanents, including any source exclusion.
        selector: OraclePermanentSelector,
        /// Signed power adjustment.
        power: i32,
        /// Signed toughness adjustment.
        toughness: i32,
    },
    /// Grant the complete parsed mana ability to each matching permanent.
    ManaGrant {
        /// Matching permanents, including any source exclusion.
        selector: OraclePermanentSelector,
        /// Costs, restrictions, and all ordered effects of the granted ability.
        ability: OracleActivatedAbility,
    },
}
