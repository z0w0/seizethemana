//! Keyword abilities (CR 702) carried as card runtime fields. Triggered
//! keyword abilities (mobilize, afterlife) are synthesized into the
//! ability list instead; activated ones (power-up, exhaust) carry flags
//! on their `SimAbility`; keyword actions (CR 701) lower as effects.

use super::Cost;

/// Keyword abilities (CR 702) the card model carries as runtime fields.
///
/// Triggered keyword abilities (mobilize, afterlife) are synthesized into
/// the ability list instead; activated ones (power-up, exhaust) carry
/// flags on their [`SimAbility`]. Keyword actions (CR 701) lower through
/// the effect list (amass, explore, connive, the Ring, empower Jace),
/// never as card fields.
#[derive(Debug, Clone, Default)]
pub struct KeywordAbilities {
    /// Saddle N (CR 702.171): the activated ability taps creatures with
    /// total power N to mark this permanent saddled until end of turn.
    pub saddle: Option<u32>,
    /// Convoke (CR 702.51): tap creatures to pay the spell's mana.
    pub convoke: bool,
    /// Delve (CR 702.66): exile graveyard cards to pay generic mana.
    pub delve: bool,
    /// Storm (CR 702.40): copy the spell once per other spell cast
    /// before it this turn.
    pub storm: bool,
    /// Offspring [cost] (CR 702.175): pay the extra cost on cast to
    /// create a 1/1 token copy on entry.
    pub offspring: Option<Cost>,
    /// Plot [cost] (CR 702.170): exile from hand now, cast free later.
    pub plot: Option<Cost>,
    /// Living metal (CR 702.161): the Vehicle is an artifact creature
    /// during your turn.
    pub living_metal: bool,
    /// Teamwork N (CR 702.194): optional additional cost tapping
    /// creatures with total power N.
    pub teamwork: Option<u32>,
}
