//! Alternate-cost, reveal, and search data structures for spells and
//! card selection.

use super::ManaColor;

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
}
