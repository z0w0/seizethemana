//! Typed selectors for flat generic spell-cost reductions.

use crate::deck::simulator::model::ManaColor;

/// A spell card type named by a cost-reduction ability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleSpellType {
    /// Artifact spells.
    Artifact,
    /// Battle spells.
    Battle,
    /// Creature spells.
    Creature,
    /// Enchantment spells, including Auras.
    Enchantment,
    /// Instant spells.
    Instant,
    /// Planeswalker spells.
    Planeswalker,
    /// Sorcery spells.
    Sorcery,
}

/// The complete spell selector of a supported cost-reduction ability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OracleSpellFilter {
    /// Every spell its controller casts.
    Any,
    /// A spell with at least one listed card type.
    Types(Vec<OracleSpellType>),
    /// A spell with at least one listed color.
    Colors(Vec<ManaColor>),
}

/// One unconditional ability that reduces matching spells' generic costs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleSpellCostReduction {
    /// Generic mana removed once from each matching spell's total cost.
    pub amount: u32,
    /// Spells the ability can reduce.
    pub filter: OracleSpellFilter,
}
