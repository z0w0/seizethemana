//! Effects resolved by runtime abilities and spells.

use super::{LifeLossScope, ManaYield, SearchSpec};

/// What an ability does when it resolves.
#[derive(Debug, Clone, Default)]
pub enum SimEffect {
    /// Resolve a complete conditional reward after a matching Behold choice.
    Behold {
        /// Qualities required by the choice.
        spec: super::behold::BeholdSpec,
        /// Dependent effects in printed order.
        effects: Vec<SimEffect>,
    },
    /// Keep the optional untap tied to one search's result identity.
    SearchThenBeholdUntap {
        /// Complete search and placement data.
        search: super::SearchSpec,
        /// Matching choice needed for untapping the found land.
        spec: super::behold::BeholdSpec,
    },
    /// No modeled effect (ability exists but does nothing in the sim).
    #[default]
    None,
    /// Draw N cards.
    Draw(u32),
    /// Reorder only exposed top cards, then optionally shuffle the library.
    LookAtTop {
        /// Maximum cards the controller may inspect.
        count: u32,
        /// The instruction permits a full-library shuffle.
        may_shuffle: bool,
    },
    /// Return the stated number of current hand cards to the library top.
    PutHandOnTop(u32),
    /// Choose distinct library cards and assign each to one destination.
    MultiSearch(super::MultiSearchSpec),
    /// Draw a card and put a -1/-1 counter on a chosen creature.
    DrawAndMinusCounter,
    /// Gain N life.
    GainLife(u32),
    /// Search the library for a card that matches the parsed constraints.
    Search(SearchSpec),
    /// Produce mana (activated mana abilities, planets at threshold).
    Mana(ManaYield),
    /// Untap this permanent and repeat its tap-for-mana ability.
    UntapSelf,
    /// Produce `counters × N` mana at the first main phase, then clear the host's
    /// charge counters (banked-mana engines: Coalition Relic).
    ManaPerCounter(ManaYield),
    /// Create token creatures (ETB tokens, crewed payoffs). The count is
    /// advisory: the sim tracks creatures, not individual tokens.
    Tokens(u32),
    /// Bank Treasure tokens as mana instead of adding creature tokens.
    Treasures(u32),
    /// Create `per_opponent` tokens for each opponent ("for each
    /// opponent, create a 1/1 … token"). The commander family multiplies
    /// by three; constructed tables multiply by one.
    TokensEachOpponent {
        /// Tokens created per opponent.
        per_opponent: u32,
    },
    /// Put N charge counters on this permanent (counter injection).
    Counters(u32),
    /// Extra land drop this turn (explore-style effects, land-search
    /// ETBs, saga ramp chapters). Puts one card in the hand; never
    /// re-fires and never sets a return flag.
    ExtraLand,
    /// Exile and return the source permanent.
    /// The permanent re-fires its own Enters triggers once, next turn
    /// (the re-arm lives in the ETB firing path, not in this effect).
    ExileThenReturnSource,
    /// "You become the Monarch": from the turn after acquisition the
    /// Monarch draws one extra card at upkeep (engine, not one apply).
    Monarch,
    /// Put the top N cards of the library into the graveyard. Milled
    /// cards count as cards seen (velocity) and fill the graveyard log.
    Mill(u32),
    /// Return cards from the graveyard: to the hand (`to_hand`, draw
    /// credit) or the battlefield (a creature, once per card).
    ReturnFromGraveyard {
        /// True: cards go to the hand (counts as cards seen).
        to_hand: bool,
        /// How many cards to return.
        count: u32,
    },
    /// Discard the hand into the graveyard, then draw seven cards.
    /// Cards seen jump by the hand size; the graveyard log fills.
    DiscardHandThenDrawSeven,
    /// Draw N, then discard N. Net velocity +N; the graveyard
    /// log fills with the discards.
    DrawThenDiscard(u32),
    /// Draw all requested cards, then discard the stated number.
    DrawDiscard {
        /// Cards drawn before discarding.
        draw: u32,
        /// Cards discarded after drawing.
        discard: u32,
    },
    /// Look at the top N cards (scry, surveil). Zero draw credit: the
    /// cards feed `library_awareness_by_turn` instead.
    Scry(u32),
    /// Surveil N cards. Each is seen and moved from the library to the
    /// graveyard, so library-mill triggers resolve.
    Surveil(u32),
    /// Take an extra turn after this one. The next configured turn slot
    /// runs the full turn pipeline and is marked as an extra turn.
    ExtraTurn,
    /// Upkeep threshold engine: when the host holds `counters` charge
    /// counters it wins (Darksteel Reactor, Helix Pinnacle class).
    /// An unparseable counter amount parses to `u32::MAX`, so the
    /// engine can never reach it: the engine fires only when the
    /// amount was readable in the oracle text.
    WinThreshold {
        /// Counters needed to win.
        counters: u32,
    },
    /// Lose N life (life_loss, burn at a player). Opponent-scoped phrasing
    /// ("target player", "each opponent") maps here; creature-target
    /// burn does not. The scope decides the table multiplier (CR 119.3):
    /// "each opponent" hits every opponent; "target player" hits exactly
    /// one; "each player" hits every opponent plus the player.
    LoseLife {
        /// Life the effect asks each affected player to lose.
        amount: u32,
        /// Who the effect reaches.
        scope: LifeLossScope,
    },
    /// Deal damage to players. Damage remains distinct from life loss
    /// for the damage census and later replacement-effect support.
    Damage {
        /// Damage dealt to each affected player.
        amount: u32,
        /// Players who receive the damage.
        scope: LifeLossScope,
    },
    /// The player gets N energy counters (CR 122.1: player counters).
    Energy(u32),
    /// Proliferate (CR 701.34): add one more counter of each kind that
    /// the chosen permanents and the player already have.
    Proliferate,
    /// Amass N (CR 701.47): create a 0/0 Army body if none exists, then
    /// put N +1/+1 counters on the chosen Army. Army power comes from
    /// its counters.
    Amass(u32),
    /// The Ring tempts the player (CR 701.54): the Ring emblem levels
    /// unlock at 2, 3, and 4 tempts, and "Whenever the Ring tempts you"
    /// triggers fire.
    RingTempts,
    /// Empower Jace N (CR 701.71): create a Jace planeswalker token if
    /// none exists, then add N loyalty to it.
    EmpowerJace(u32),
    /// Mobilize N (CR 702.181): create N 1/1 Warrior tokens, tapped and
    /// attacking, sacrificed at the beginning of the next end step.
    Mobilize(u32),
    /// Explore (CR 701.44): reveal the top card; a land goes to hand,
    /// otherwise a +1/+1 counter joins the exploring permanent.
    Explore,
    /// Connive N (CR 701.50): draw N, discard N, then a +1/+1 counter
    /// if a nonland card was discarded.
    Connive(u32),
    /// Afterlife N (CR 702.135): create N 1/1 white-black Spirit tokens
    /// with flying when this permanent is put into a graveyard.
    Afterlife(u32),
    /// The One Ring class: "{T}, Put a burden counter on this
    /// permanent: Draw a card for each burden counter on it." The
    /// activation adds one burden counter, then draws for the new total.
    AddBurdenCounter,
    /// "At the beginning of your upkeep, you lose 1 life for each burden
    /// counter on this permanent" (The One Ring class).
    BurdenLifeLoss,
}
