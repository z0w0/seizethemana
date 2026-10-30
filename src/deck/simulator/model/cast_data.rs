//! Land entry, alternate costs, and card selection data.

use super::{ManaColor, SimCard};

/// A supported board condition that lets a land enter untapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandEntryCondition {
    /// Count other lands before this land enters.
    OtherLandsAtMost(u32),
    /// Require at least this many other lands before entry.
    OtherLandsAtLeast(u32),
    /// Require at least this many active opponent seats.
    OpponentsAtLeast(u32),
    /// Require a controlled land with any named basic land type, in WUBRG order.
    ControlsLandTypes([bool; 5]),
}

impl LandEntryCondition {
    /// Check entry against the physical lands controlled before this land enters.
    pub fn is_met_with_land_types(
        self,
        other_lands: u32,
        active_opponents: u32,
        land_types: [bool; 5],
    ) -> bool {
        match self {
            Self::OtherLandsAtMost(count) => other_lands <= count,
            Self::OtherLandsAtLeast(count) => other_lands >= count,
            Self::OpponentsAtLeast(count) => active_opponents >= count,
            Self::ControlsLandTypes(required) => required
                .into_iter()
                .zip(land_types)
                .any(|(required, controlled)| required && controlled),
        }
    }
}

impl SimCard {
    /// Preview land entry using the controlled land types before placement.
    pub fn entry_is_tapped_with_land_types(
        &self,
        other_lands: u32,
        active_opponents: u32,
        land_types: [bool; 5],
    ) -> bool {
        self.untapped_entry_condition
            .map_or(self.enters_tapped, |condition| {
                !condition.is_met_with_land_types(other_lands, active_opponents, land_types)
            })
    }
}

/// A creature search linked to the printed mana value of a sacrificed creature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SacrificeSearch {
    /// Shuffle the remaining library when the search finishes.
    pub shuffle: bool,
    /// Add this amount to the sacrificed creature's mana value.
    pub mana_value_offset: u32,
    /// Accept any creature at or below that value instead of an exact match.
    pub allow_lower_mana_value: bool,
    /// The found creature enters with an additional +1/+1 counter.
    pub enters_with_plus1_counter: bool,
}

/// Alternate casting cost parsed from the Oracle cost sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlternativeCastCost {
    /// Card category required from the hand.
    pub filter: CardFilter,
    /// Number of cards exiled to pay the cost.
    pub count: u32,
    /// SimEffect based on the exiled cards after the cast resolves.
    pub payoff: AlternativeCostPayoff,
}

/// Oracle-derived filter for cards used to pay an alternate cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardFilter {
    /// Required color, when named by the cost.
    pub color: Option<ManaColor>,
}

/// Resolution effects paid by an alternate cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlternativeCostPayoff {
    /// No additional effect.
    None,
    /// Gain life equal to the total mana value of exiled cards.
    GainLifeEqualToExiledManaValue,
}

/// A reveal instruction and its destination parsed from Oracle text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevealRule {
    /// Where each revealed card moves.
    pub destination: RevealDestination,
    /// Life loss for each card, when stated by the effect.
    pub life_loss: RevealLifeLoss,
}

/// Destination supported by the reveal-and-life-loss rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealDestination {
    /// Put each revealed card into its owner's hand.
    Hand,
}

/// Oracle-derived life-loss amount for a revealed card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealLifeLoss {
    /// Lose life equal to the revealed card's mana value.
    ManaValue,
}

/// Constraints for the small set of library searches the simulator supports.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchSpec {
    /// Required card type, when the Oracle text names one.
    pub card_type: Option<SearchCardType>,
    /// Required color, when the Oracle text names one.
    pub color: Option<ManaColor>,
    /// True when the search requires a colorless card.
    pub colorless: bool,
    /// Exact mana value, when the Oracle text names one.
    pub mana_value: Option<u32>,
    /// Highest allowed mana value, when the Oracle text says "or less".
    pub max_mana_value: Option<u32>,
    /// Lowest allowed mana value, when the Oracle text says "or more".
    pub min_mana_value: Option<u32>,
    /// Zone where a found card goes.
    pub destination: SearchDestination,
    /// Whether the search may choose not to find a match.
    pub optional: bool,
    /// Limit a search to the first N library cards.
    pub top_count: Option<usize>,
    /// Shuffle the remaining library after a full-library search.
    pub shuffle: bool,
    /// Reject Human cards when the Oracle search specifies non-Human.
    pub non_human: bool,
    /// Basic land types allowed by a land search, WUBRG order.
    pub land_types: [bool; 5],
    /// True when a land search requires the Basic supertype.
    pub basic_land_only: bool,
}

/// Card types used by supported searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchCardType {
    /// Creature card.
    Creature,
    /// Land card.
    Land,
    /// Basic land card.
    BasicLand,
    /// Artifact card.
    Artifact,
    /// Enchantment card.
    Enchantment,
    /// Artifact or enchantment card.
    ArtifactOrEnchantment,
    /// Instant card.
    Instant,
    /// Sorcery card.
    Sorcery,
    /// Instant or sorcery card.
    InstantSorcery,
    /// Planeswalker card.
    Planeswalker,
    /// Any permanent card.
    Permanent,
}

/// Destination for a supported library search.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchDestination {
    /// Put the card into its owner's hand.
    #[default]
    Hand,
    /// Put the card onto the battlefield.
    Battlefield,
    /// Put the card onto the battlefield tapped.
    BattlefieldTapped,
    /// Put the card on top of the library.
    LibraryTop,
    /// Exile the card.
    Exile,
    /// Put the card into its owner's graveyard.
    Graveyard,
}
