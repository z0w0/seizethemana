//! Runtime selectors for flat generic spell-cost reductions.

use super::{ManaColor, SimCard};

/// A front-face card type that a spell-cost reduction can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellCostCardType {
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

impl SpellCostCardType {
    /// The normalized card type retained by the front-face model.
    pub fn name(self) -> &'static str {
        match self {
            Self::Artifact => "artifact",
            Self::Battle => "battle",
            Self::Creature => "creature",
            Self::Enchantment => "enchantment",
            Self::Instant => "instant",
            Self::Planeswalker => "planeswalker",
            Self::Sorcery => "sorcery",
        }
    }
}

/// The front-face properties a spell needs for one cost reduction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellCostFilter {
    /// Any spell.
    Any,
    /// At least one listed card type.
    Types(Vec<SpellCostCardType>),
    /// At least one listed color, checked only when front-face colors are known.
    Colors(Vec<ManaColor>),
}

impl SpellCostFilter {
    /// Match actual front-face data without using merged face flags or colors.
    pub fn matches(&self, card: &SimCard) -> bool {
        match self {
            Self::Any => true,
            Self::Types(types) => types
                .iter()
                .any(|kind| card.card_types.iter().any(|actual| actual == kind.name())),
            Self::Colors(colors) => card
                .search_colors
                .is_some_and(|known| colors.iter().any(|color| known[color.index()])),
        }
    }
}

/// A live source's flat reduction to matching spells' generic costs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellCostReduction {
    /// Generic mana removed from the selected total casting cost.
    pub amount: u32,
    /// Front-face spell properties that qualify for the reduction.
    pub filter: SpellCostFilter,
}

/// Check front-face selector rules independently of Oracle parsing.
#[cfg(test)]
#[path = "tests/costs_tests.rs"]
mod costs_tests;
