//! Typed Behold requirements kept separate from executable card data.

/// The printed quality and number of distinct objects a Behold instruction needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleBeholdSpec {
    /// Every required card type or supertype, in lowercase.
    pub card_types: Vec<String>,
    /// Every required subtype, in lowercase.
    pub subtypes: Vec<String>,
    /// Number of distinct cards or permanents required by the instruction.
    pub count: u32,
}

/// An additional cost paid by Behold or by the stated generic mana amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleBeholdAlternative {
    /// Quality and count that permit payment without extra mana.
    pub spec: OracleBeholdSpec,
    /// Generic mana owed when the player does not behold.
    pub generic: u32,
}
