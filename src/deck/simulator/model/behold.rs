//! Executable Behold qualities and generic-mana alternatives.

/// Complete type requirements for a supported Behold choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeholdSpec {
    /// All required card types or supertypes, already normalized to lowercase.
    pub card_types: Vec<String>,
    /// All required subtypes, already normalized to lowercase.
    pub subtypes: Vec<String>,
    /// Required distinct choices; only one is currently executable.
    pub count: u32,
}

impl BeholdSpec {
    /// Check exposed characteristics without reading Oracle wording.
    pub(in crate::deck::simulator) fn matches_characteristics(
        &self,
        card_types: &[String],
        subtypes: &[String],
    ) -> bool {
        self.card_types.iter().all(|kind| card_types.contains(kind))
            && self.subtypes.iter().all(|kind| subtypes.contains(kind))
    }
}

/// Additional generic mana owed unless a matching Behold choice is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeholdAlternative {
    /// The nonconsuming choice that replaces the extra mana payment.
    pub spec: BeholdSpec,
    /// Generic mana owed if the player cannot or does not behold.
    pub generic: u32,
}

/// Runtime type checks use every field of the compiled quality.
#[cfg(test)]
#[path = "tests/behold_tests.rs"]
mod behold_tests;
