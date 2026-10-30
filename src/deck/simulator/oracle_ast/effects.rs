//! Parsed resolution effects, including unsupported Oracle text.

use super::{DamageTarget, search, tribal};
use crate::deck::simulator::model::{LifeLossScope, ManaYield, SacrificeSearch, SearchSpec};

/// A parsed effect, or an explicit node for text the effect parser cannot read.
#[derive(Debug, Clone)]
pub enum OracleEffect {
    /// Resolve the dependent effects only after a matching Behold choice.
    Behold {
        /// Required qualities of the revealed card or chosen permanent.
        spec: super::behold::OracleBeholdSpec,
        /// Complete dependent resolution instructions.
        effects: Vec<OracleEffect>,
    },
    /// Search, shuffle, and optionally untap only the found land.
    SearchThenBeholdUntap {
        /// The complete preceding search instruction.
        search: super::super::model::SearchSpec,
        /// The matching Behold choice needed to untap the result.
        spec: super::behold::OracleBeholdSpec,
    },
    /// A complete creature or mana rule without runtime support yet.
    Tribal(tribal::OracleTribalEffect),
    /// Search using the creature sacrificed as an additional cast cost.
    SacrificeSearch(SacrificeSearch),
    /// Draw one or more cards.
    Draw(u32),
    /// Reorder the exposed top cards, with an optional full-library shuffle.
    LookAtTop {
        /// Maximum cards exposed by the instruction.
        count: u32,
        /// Whether the controller may shuffle after returning the cards.
        may_shuffle: bool,
    },
    /// Put the stated number of hand cards on top of the library.
    PutHandOnTop(u32),
    /// Draw cards, then discard cards.
    DrawThenDiscard(u32),
    /// Draw and discard different fixed counts, in that order.
    DrawDiscard {
        /// Cards drawn before any discard.
        draw: u32,
        /// Cards discarded after drawing.
        discard: u32,
    },
    /// Draw cards and put a -1/-1 counter on a creature.
    DrawAndMinusCounter,
    /// Gain life.
    GainLife(u32),
    /// Search using parsed card constraints.
    Search(SearchSpec),
    /// Select multiple cards and retain every destination before runtime support exists.
    MultiSearch(search::OracleMultiSearch),
    /// Add mana.
    Mana(ManaYield),
    /// Untap the source permanent.
    UntapSource,
    /// Release mana stored on charge counters.
    ManaPerCounter(ManaYield),
    /// Create creature or artifact tokens.
    CreateTokens(u32),
    /// Create Treasure tokens from this statement.
    CreateTreasureTokens(u32),
    /// Put charge counters on the source.
    PutChargeCounters(u32),
    /// Make an additional land play available (CR 305.3).
    AdditionalLandPlay,
    /// Exile and return a permanent.
    ExileThenReturn,
    /// Become the Monarch.
    Monarch,
    /// Move cards from the library into the graveyard.
    Mill(u32),
    /// Return cards from a graveyard.
    ReturnFromGraveyard {
        /// Put cards into hand when true; put them on the battlefield otherwise.
        to_hand: bool,
        /// Number of cards affected.
        count: u32,
    },
    /// Discard a hand and draw a replacement hand.
    DiscardHandThenDraw,
    /// Take an extra turn.
    ExtraTurn,
    /// Scry N: look at the top N cards of the library (CR 701.22).
    Scry(u32),
    /// Surveil N: look at the top N cards and put any number into the
    /// graveyard (CR 701.25).
    Surveil(u32),
    /// Lose life, scoped by target phrasing.
    LoseLife {
        /// Life lost per affected player.
        amount: u32,
        /// Who the effect reaches (CR 119.3).
        scope: LifeLossScope,
    },
    /// Damage remains distinct from life loss for target and replacement rules.
    Damage {
        /// Damage dealt to each selected target.
        amount: u32,
        /// Target class stated by the effect.
        target: DamageTarget,
        /// Player scope when the target is a player.
        scope: LifeLossScope,
    },
    /// Create `per_opponent` tokens for each opponent ("for each
    /// opponent, create a 1/1 … token"). The runtime multiplies by the
    /// format's opponent count (three in the commander family).
    CreateTokensPerOpponent {
        /// Tokens created per opponent.
        per_opponent: u32,
    },
    /// The player gets N energy counters.
    Energy(u32),
    /// Amass N: create or grow an Army body with N +1/+1 counters
    /// (CR 701.47).
    Amass(u32),
    /// The Ring tempts the player (CR 701.54).
    RingTempts,
    /// Empower Jace N: create or grow a Jace planeswalker token
    /// (CR 701.71).
    EmpowerJace(u32),
    /// Explore (CR 701.44): land to hand, else a +1/+1 counter.
    Explore,
    /// Connive N (CR 701.50): draw N, discard N, then a +1/+1 counter
    /// if a nonland was discarded.
    Connive(u32),
    /// Mobilize N (CR 702.181): create N tapped-and-attacking 1/1
    /// Warrior tokens sacrificed at the beginning of the next end step.
    /// Synthesized as an attack trigger from the keyword entry.
    Mobilize(u32),
    /// Afterlife N (CR 702.135): create N 1/1 flying Spirit tokens when
    /// the permanent is put into a graveyard. Synthesized as a death
    /// trigger from the keyword entry.
    Afterlife(u32),
    /// "{T}, put a burden counter on this permanent: draw a card for
    /// each burden counter on it" (The One Ring class).
    AddBurdenCounter,
    /// "At the beginning of your upkeep, you lose 1 life for each burden
    /// counter on this permanent" (The One Ring class).
    BurdenLifeLoss,
    /// Add one counter of each kind already on the chosen permanents and
    /// the player (CR 701.34).
    Proliferate,
    /// Win when a counter threshold is reached.
    WinsAtCounters(u32),
    /// Preserve an effect not supported by the simulator.
    #[allow(dead_code)] // Unsupported clauses remain in the syntax tree and stay inert.
    Unsupported(String),
}
