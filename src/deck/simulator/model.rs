//! Card data model for the goldfish simulator: costs, mana yields,
//! station striations, crew, spell data, flags, and abilities. Pure data
//! + small helpers; parsing lives in `oracle_parser` and `oracle_lower`, execution in `game`.
//!
//! Everything the model cannot express is dropped at parse time; the
//! documented limits ship in the output `assumptions` list (see `report`).

/// Named colors and basic land types.
mod colors;

/// Runtime abilities and their costs, conditions, and event subjects.
mod abilities;
/// Alternate-cost, reveal, and search data structures.
#[path = "model/cast_data.rs"]
mod cast_data;
/// Keyword abilities (CR 702) as fields; keyword actions (CR 701) lower
/// through the effect list.
#[path = "model/keyword_fields.rs"]
mod keyword_fields;
/// Runtime keywords, keyword sets, and the counter model.
mod keywords;

pub use abilities::{
    SimAbility, SimAbilityCondition, SimAbilityKind, SimActivation, SimActivationCost,
    SimActivationRestriction, SimEventSubject,
};
pub use cast_data::{
    AlternativeCastCost, AlternativeCostPayoff, CardFilter, RevealDestination, RevealLifeLoss,
    RevealRule, SearchCardType, SearchDestination, SearchSpec,
};
pub use colors::{BasicLandType, COLORS, ManaColor};
pub use keyword_fields::KeywordAbilities;
pub use keywords::{Counters, EnterCounters, KEYWORD_TABLE, Keyword, KeywordSet, PlayerCounters};

/// Default number of simulated games: ±0.5pp on percentages at 10k runs.
pub const DEFAULT_RUNS: u32 = 10_000;

/// Shared bound on repeatable game-loop passes that could otherwise run
/// forever (cast sweeps, graveyard casts, sacrifice exchanges, activation
/// loops). One number keeps every runaway-protection loop consistent and
/// the census flags (`infinite_mana_suspected`) comparable.
pub const MAX_LOOP_PASSES: u32 = 24;

/// Functional roles the simulator classifies each card into. One role per
/// card (first match wins) so role counts never double-count copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Role {
    /// Land (basic or nonbasic); one mana per turn when in play.
    Land,
    /// Nonland artifact that produces mana (Sol Ring, Arcane Signet).
    Rock,
    /// Creature that produces mana (Llanowar Elves, Birds of Paradise).
    Dork,
    /// Spell that makes mana or searches lands (rituals, ramp, fetches).
    RampSpell,
    /// One-shot or repeatable card-draw source.
    Draw,
    /// Targeted removal, counterspell, or board wipe.
    Removal,
    /// Static tax/restriction piece (Rishadan Port, Static Orb, Thalia):
    /// stax timing is the role-access question.
    Lock,
    /// Equipment, aura, or pump spell that suits up a threat (Voltron).
    Booster,
    /// Card the deck intends to win with: big threats and finishers.
    Wincon,
    /// Everything else.
    #[default]
    Other,
}

/// Mana a card asks for when casting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cost {
    /// Generic-mana value (the `{N}` parts; `{X}` counts as 1).
    pub generic: u32,
    /// Monocolor pips per WUBRG letter.
    pub pips: [u8; 5],
    /// Hybrid mana pips (`{W/U}`, CR 107.4e): payable from any of their
    /// component colors, so the pool treats them as flexible.
    pub hybrid_pips: u32,
    /// Phyrexian pips per WUBRG letter (`{W/P}`): each payable by its
    /// color or 2 life (CR 107.4f). The best-case agent pays life.
    pub phyrexian: [u8; 5],
}

impl Cost {
    /// Total mana value (generic plus every pip).
    pub fn total(&self) -> u32 {
        self.generic
            + self.pips.iter().map(|p| u32::from(*p)).sum::<u32>()
            + self.hybrid_pips
            + self.phyrexian.iter().map(|p| u32::from(*p)).sum::<u32>()
    }
}

/// What one tap of a mana ability yields.
///
/// Real cards fall into three shapes: an "or" choice (`Add {G} or {U}` or
/// "one mana of any color"), a fixed simultaneous set (`Add {W}{U}{B}{R}{G}`
/// on Jegantha), and colorless-only (`{C}`). Each is one tap = the listed
/// mana, never several independent sources.
#[derive(Debug, Clone, Default)]
pub struct ManaYield {
    /// Fixed simultaneous pips produced on every tap (Jegantha).
    pub fixed: [u8; 5],
    /// Colors this source can choose from when tapped (choice sources).
    pub choice: [bool; 5],
    /// Any-color pips produced per tap (Command Tower 1, Gilded Lotus 3).
    pub any_pips: u32,
    /// True when the "any color" clause depends on an opponent's board
    /// ("any color a land an opponent controls could produce" — Fellwar
    /// Stone). Goldfish approximation: generic-only mana from turn two.
    pub opponent_any: bool,
    /// True when these pips may pay colored costs but not generic mana.
    pub cannot_pay_generic: bool,
    /// Conditional scaling: the tap's output grows with board state.
    pub scaling: Option<Scale>,
    /// Colorless-only production (`{C}`).
    pub colorless: u32,
    /// True when merged from several separate tap abilities: one tap
    /// yields one mana of any reachable color, never the sum
    /// (Plaza of Heroes, Verge lands).
    pub alternatives: bool,
    /// Spend restriction: the mana may only pay for certain casts
    /// ("spend this mana only to cast creature spells" — Secluded
    /// Courtyard; "…a legendary spell" — Plaza of Heroes).
    pub restriction: Option<SpendRestriction>,
}

/// Spend-restriction classes the pool can honor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendRestriction {
    /// Creature spells only (Secluded Courtyard).
    Creature,
    /// Artifact spells only (Steelswarm Operator).
    Artifact,
    /// Legendary spells only (Plaza of Heroes).
    Legendary,
    /// Instant and sorcery spells only (Hydro-Channeler).
    InstantSorcery,
}

/// Board-state growth of a tap yield.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// One any-color pip per color among permanents you control
    /// (Faeburrow Elder, Bloom Tender, Plaza of Heroes' legendary mode).
    ColorsPresent,
    /// One any-color pip per charge counter on the source
    /// (Astral Cornucopia, Crystalline Crawler).
    PerChargeCounter,
}

impl ManaYield {
    /// Total mana produced by one tap. Alternative-mode sources (several
    /// tap abilities on one permanent) yield one mana, never the sum.
    pub fn total(&self) -> u32 {
        if self.alternatives {
            let reachable = self.any_pips > 0 || self.choice.iter().any(|c| *c);
            return u32::from(reachable || self.colorless > 0);
        }
        self.fixed.iter().map(|p| u32::from(*p)).sum::<u32>()
            + self.any_pips
            + u32::from(self.choice.iter().any(|c| *c))
            + self.colorless
    }
}

/// Event whose timing the goldfish models for a triggered ability
/// (CR 603.2b).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SimTrigger {
    /// No event trigger. Used by activated abilities.
    #[default]
    Never,
    /// Fired when the card enters the battlefield.
    Enters,
    /// Fired at the beginning of the player's upkeep step.
    Upkeep,
    /// Fired at the beginning of the player's precombat main phase.
    PrecombatMain,
    /// Fired at the beginning of the player's end step.
    EndStep,
    /// Fired when the card attacks.
    Attacks,
    /// Fired once when the player declares one or more attackers.
    PlayerAttacks,
    /// Fired when the card deals combat damage to a player (best case:
    /// every attacker connects). Thrummingbird proliferate, draw, drain.
    CombatDamage,
    /// Fired when another spell is cast (Y'shtola, Vivi, Jhoira).
    SpellCast,
    /// Fired when the permanent dies or is sacrificed.
    Dies,
    /// Fired when a land enters under the player's control.
    LandEnters,
    /// Fired when a nonland permanent is tapped to produce mana.
    TappedForMana,
    /// Fired when the Ring tempts the player (CR 701.54d).
    RingTempts,
}

/// Who a life-loss effect reaches (CR 119.3: losing life adjusts the
/// player's life total). The goldfish table has one
/// opponent in constructed and three in the commander family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LifeLossScope {
    /// "Each opponent loses N life" / "deals N damage to each opponent":
    /// every opponent loses N.
    #[default]
    EachOpponent,
    /// "Target player loses N" / "deals N damage to target player":
    /// exactly one player loses N.
    TargetPlayer,
    /// "Each player loses N": every opponent and the player lose N.
    EachPlayer,
}

impl LifeLossScope {
    /// Total life lost off the table when this scope resolves: the
    /// per-player amount times the affected opponents. "Each player"
    /// also costs the goldfish N, so the caller charges life separately.
    pub fn table_multiplier(self, format: Format) -> u32 {
        match self {
            Self::EachOpponent | Self::EachPlayer => format.life_loss_mult(),
            Self::TargetPlayer => 1,
        }
    }
}

/// What an ability does when it resolves.
#[derive(Debug, Clone, Default)]
pub enum SimEffect {
    /// No modeled effect (ability exists but does nothing in the sim).
    #[default]
    None,
    /// Draw N cards.
    Draw(u32),
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

/// Oracle-triggered effect when this card moves from library to graveyard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryGraveyardTrigger {
    /// Put this card onto the battlefield.
    ReturnToBattlefield,
    /// Drain opponents and gain the same amount of life.
    DrainAndGain(u32),
}

/// A station striation: abilities unlocked at a charge-counter threshold.
///
/// Station card rules come from CR 702.184 and 721: a station card's text
/// box has one or two striations,
/// each led by an `{N+}` symbol meaning "as long as this
/// permanent has N or more charge counters, it has [abilities]" and, when a
/// P/T box is printed in the same striation, "…and is a creature with base
/// P/T". Planets never animate (no P/T box); a spacecraft animates only at
/// the striation that carries the P/T box.
#[derive(Debug, Clone, Default)]
pub struct SimStriation {
    /// Charge-counter threshold (`12+` → 12).
    pub at: u32,
    /// True when this striation also animates the card as a creature.
    pub animate: bool,
    /// Abilities unlocked at this striation (draw engines, mana taps).
    pub abilities: Vec<SimAbility>,
}

/// Saga chapter data: the ordered chapter effects of a Saga card.
///
/// Rule source: CR 714.2 — each chapter is a triggered ability, indexed by
/// the Roman numeral in the Oracle text. `chapters[i]` is chapter i+1's
/// effect list; an empty list marks a chapter whose wording is
/// unsupported. A chapter may resolve several effects ("create a token.
/// The Ring tempts you."), so each slot holds a list.
#[derive(Debug, Clone, Default)]
pub struct SimSaga {
    /// Chapter effects in play order (chapter I first). The length is
    /// the chapter count.
    pub chapters: Vec<Vec<SimEffect>>,
}

/// One-shot effects and extra costs that ride a spell's cast.
///
/// The cast phase reads these when the spell resolves. Split cards zero
/// the spell data (the cast pays the cheaper face), so every field stays
/// zero/None for them.
#[derive(Debug, Clone, Default)]
pub struct SimSpellData {
    /// One-shot mana on cast (rituals), never joining the tap pool.
    pub mana_on_cast: Option<ManaYield>,
    /// Repeatable mana on cast: "add {N} for each spell you've cast this
    /// turn" (Vivi class). Joins the pool per spell cast, every turn the
    /// host is on the battlefield.
    pub mana_per_cast: Option<ManaYield>,
    /// One-shot draw on cast (cantrips, Divination).
    pub draws_on_cast: u32,
    /// Life gained when this spell resolves.
    pub life_gain_on_cast: u32,
    /// Oracle-derived alternate cost, when the spell has one.
    pub alternative_cast_cost: Option<AlternativeCastCost>,
    /// Oracle-derived reveal effect, when the spell reveals cards for life.
    pub reveal_rule: Option<RevealRule>,
    /// One-shot token creation on cast ("Create N 1/1 Soldier tokens").
    pub tokens_on_cast: u32,
    /// One-shot Treasure token creation on cast. Treasures enter the mana bank.
    pub treasures_on_cast: u32,
    /// One-shot scry count on cast. Awareness credit, not draw.
    pub scry_on_cast: u32,
    /// One-shot surveil count on cast. The cards move to the graveyard.
    pub surveils_on_cast: u32,
    /// One-shot extra turn on cast ("take an extra turn").
    pub extra_turns_on_cast: bool,
    /// Additional land plays granted until end of the current turn.
    pub extra_land_drops_on_cast: u32,
    /// One-shot life loss at a player on cast (burn, drain).
    pub life_loss_on_resolve: u32,
    /// Who [`Self::life_loss_on_resolve`] reaches (CR 119.3). Target-player
    /// burn counts once; each-opponent burn counts per opponent.
    pub life_loss_scope: LifeLossScope,
    /// Direct player damage on cast, separate from life loss.
    pub damage_on_resolve: u32,
    /// Who direct player damage reaches.
    pub damage_scope: LifeLossScope,
    /// One-shot mill on cast or on entering ("mill N").
    pub mills_on_enter: u32,
    /// One-shot energy on cast ("you get {E}{E}").
    pub energy_on_cast: u32,
    /// One-shot amass on cast (CR 701.47): grow or create an Army.
    pub amass_on_cast: u32,
    /// One-shot empower Jace on cast (CR 701.71).
    pub empower_jace_on_cast: u32,
    /// True when the resolving spell causes the Ring to tempt the
    /// player (CR 701.54).
    pub tempts_ring_on_cast: bool,
    /// One-shot explore count on cast (CR 701.44).
    pub explores_on_cast: u32,
    /// One-shot connive count on cast (CR 701.50).
    pub connives_on_cast: u32,
    /// One-shot discard-hand-then-draw-seven effect on cast.
    pub discard_hand_then_draw_seven_on_cast: bool,
    /// Creatures the cast consumes ("as an additional cost to cast this
    /// spell, sacrifice a creature").
    pub additional_cost_creatures: u32,
    /// Cards discarded from hand as an additional cast cost.
    pub additional_cost_discards: u32,
    /// Life the cast costs on top of mana ("as an additional cost …
    /// pay N life"). Best case: the agent pays it.
    pub additional_cost_life: u32,
    /// One-shot charge-counter injection on cast (Drill Too Deep).
    pub counters_on_cast: u32,
    /// X-cost effect class: the spell pays the leftover pool as X and
    /// scales its effect (drain X, draw X, mill X, tokens X). None when
    /// not an X spell. `SimEffect::LoseLife { amount: 0, .. }` carries
    /// the class; the game loop substitutes the paid X.
    pub x_class: Option<XClass>,
    /// Optional kicker cost (pips and generic); paid from spare mana when
    /// affordable. The kicker rider bumps drain/damage amounts.
    pub kicker: Option<Cost>,
    /// Cast searches that use the sacrificed creature's mana value.
    pub search_after_sacrifice: bool,
    /// Cast effects that exchange graveyard creatures and battlefield creatures.
    pub graveyard_creature_exchange: bool,
    /// This spell grants flashback to instant and sorcery instances in
    /// the graveyard when it resolves.
    pub grants_flashback: bool,
    /// Nonland cards in the graveyard may be cast with escape while this
    /// permanent remains on the battlefield.
    pub grants_escape: bool,
    /// The card's own printed flashback cost (CR 702.34): the instance
    /// in the graveyard may cast for this cost, then exiles.
    pub own_flashback: Option<Cost>,
    /// The card's own printed escape cost (CR 702.138): the instance in
    /// the graveyard may cast by paying this plus exiling three other
    /// cards.
    pub own_escape: Option<Cost>,
    /// Cycling activation cost.
    pub cycling_cost: Option<Cost>,
    /// Life paid for cycling (Street Wraith-style cycling).
    pub cycling_life: u32,
    /// Land type searched by landcycling.
    pub landcycling_type: Option<BasicLandType>,
}

/// Static keyword and interaction flags that travel together in combat,
/// mana, and readiness paths. One parse pass in `static_flags` fills the
/// group; every field documents its rule source.
#[derive(Debug, Clone, Default)]
pub struct SimStaticFlags {
    /// +1 power per land drop made after this permanent entered
    /// (landfall +1/+1 counter patterns).
    pub landfall: bool,
    /// True when the creature ignores summoning sickness (haste):
    /// attacks, taps, and crews the turn it enters.
    pub has_haste: bool,
    /// Extra land drop each turn ("you may play an additional land").
    /// Feeds `land_drops[]` as one extra drop per turn while in play.
    pub extra_land_drops: bool,
    /// Skips the untap step ("This artifact doesn't untap during your
    /// untap step", Basalt Monolith class). The untap step leaves it
    /// tapped; only an untap activation clears the tap.
    pub doesnt_untap: bool,
    /// Castable at instant speed (Instant type or flash). Powers the
    /// interaction-readiness metric.
    pub is_instant_speed: bool,
    /// Interaction role (removal or counterspells): feeds readiness.
    pub is_interaction: bool,
    /// True when the card destroys or sweeps every permanent of a class
    /// ("destroy all creatures"): a board sweep. Sweeps count as
    /// interaction capacity but never fire in a goldfish.
    pub sweeps: bool,
    /// Static mana grant while on the battlefield ("creatures you control
    /// have {T}: add one mana of any color" — Enduring Vitality;
    /// "lands you control have…" — Chromatic Lantern). Each matching
    /// permanent adds one flexible pip per turn, capped at 2.
    pub grant: Option<Grant>,
    /// Oracle text grants one extra mana when a nonland permanent is
    /// tapped for mana (Kinnan-style replacement-independent trigger).
    pub bonus_mana_on_nonland_tap: bool,
    /// Mana ability requires three artifacts under the metalcraft rule.
    pub requires_metalcraft: bool,
    /// Static creature buff while on the battlefield ("creatures you
    /// control get +2/+2"): (power, toughness). Power joins the attack
    /// sum.
    pub buff: Option<(i32, i32)>,
    /// Static "while saddled" buff (CR 702.171b), from "As long as this
    /// permanent is saddled, it gets +X/+Y." Power joins the attack sum
    /// only while the saddle cost is paid.
    pub saddled_buff: Option<(i32, i32)>,
    /// Equipment stats: (equip cost, equipped-creature buff). None when
    /// not Equipment.
    pub equipment: Option<Equipment>,
    /// Static keyword grants this card hands to other permanents
    /// ("creatures you control have flying"). Applied at runtime by
    /// `game::effective_keywords`.
    pub keyword_grants: Vec<KeywordGrant>,
    /// "You have no maximum hand size" (CR 402.2): while this permanent
    /// is on the battlefield the end-step discard is skipped.
    pub no_max_hand_size: bool,
}

/// One static keyword grant: which permanents receive which keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeywordGrant {
    /// Permanent class that receives the keyword.
    pub target: GrantTarget,
    /// Keyword granted to the target class.
    pub keyword: Keyword,
}

/// Permanent classes a static keyword grant can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantTarget {
    /// The granter itself ("this creature has flying").
    Source,
    /// Every creature the player controls.
    CreaturesYouControl,
    /// Every land the player controls.
    LandsYouControl,
    /// Every permanent the player controls.
    PermanentsYouControl,
    /// Every spell the player casts.
    SpellsYouCast,
}

/// The card-level capabilities the simulator executes.
#[derive(Debug, Clone, Default)]
pub struct SimCard {
    /// Card name (matches the deck entry).
    pub name: String,
    /// Mana cost to cast (lands cost 0 and enter by the land rule).
    pub cost: Cost,
    /// Printed mana value, retained when an alternate cost is used.
    pub mana_value: u32,
    /// True when the type line contains Basic.
    pub is_basic_land: bool,
    /// True when the card's type line contains a land face.
    pub is_land: bool,
    /// True when the card's type line contains a planeswalker face.
    pub is_planeswalker: bool,
    /// Cheapest castable mana value after reductions (warp, affinity-lite).
    pub min_cost: Cost,
    /// True when the card has a printed mana cost. A missing cost is not
    /// equivalent to a printed `{0}` cost.
    pub has_mana_cost: bool,
    /// Mana this card produces on tap; None when it cannot tap for mana.
    pub tap: Option<ManaYield>,
    /// Station striations, in threshold order (empty when not a station
    /// card).
    pub striations: Vec<SimStriation>,
    /// Crew cost for Vehicles (None when not a Vehicle).
    pub crew: Option<u32>,
    /// Creature-ness for body counting and dork timing.
    pub is_creature: bool,
    /// True when the creature type line includes Human.
    pub is_human: bool,
    /// True for instant and sorcery spells, which go to the graveyard after
    /// resolving instead of remaining on the battlefield.
    pub is_instant_or_sorcery: bool,
    /// True when the card's type line makes it a permanent (creature,
    /// artifact, enchantment, land, planeswalker, or battle). Feeds
    /// descend counts.
    pub is_permanent: bool,
    /// True when this spell exiles itself after resolving.
    pub exile_on_resolve: bool,
    /// True when the type line carries Legendary. Gates the
    /// legendary-only spend restriction (Plaza of Heroes).
    pub is_legendary: bool,
    /// Artifact-ness for improvise/affinity discount math.
    pub is_artifact: bool,
    /// True when the card is a Spacecraft or Planet (stationable type).
    pub is_station_card: bool,
    /// Counters gained on entering (Reckoner Bankbuster).
    pub enter_counters: EnterCounters,
    /// Functional role.
    pub role: Role,
    /// Enters the battlefield tapped (Restless Reef, planets, most duals).
    pub enters_tapped: bool,
    /// Life paid to have the land enter untapped (shock-land style).
    pub life_to_untap: u32,
    /// Life paid to activate a fetch land, when its Oracle text requires it.
    pub fetch_life_cost: u32,
    /// True when Oracle text sacrifices this land to search for a land card.
    pub is_fetch_land: bool,
    /// Basic-land types named by the fetch restriction, WUBRG order.
    pub fetch_target_types: Vec<BasicLandType>,
    /// True when Oracle text restricts the search to basic lands.
    pub fetch_basic_only: bool,
    /// True when the search effect itself puts its target onto the battlefield tapped.
    pub fetch_enters_tapped: bool,
    /// Basic types this land checks for (verge gates: "control a Swamp").
    pub gate_types: Vec<BasicLandType>,
    /// True when the card may begin the game on the battlefield (Leyline).
    pub opens_in_play: bool,
    /// True when the card is a saga (staged per-turn effects).
    pub is_saga: bool,
    /// Saga chapter effects in play order (empty when not a saga).
    pub saga: SimSaga,
    /// One-shot cast effects and extra cast costs, parsed from Oracle
    /// data. The cast phase reads these when the spell resolves.
    pub spell_data: SimSpellData,
    /// Mill effects target opponents ("target player mills N") instead
    /// of the deck's own library (deck-out pressure direction).
    pub mills_opponent: bool,
    /// True when Oracle text grants undying (CR 702.93).
    pub has_undying: bool,
    /// Number of cards milled when this card replaces a draw from the graveyard.
    pub dredge: Option<u32>,
    /// SimEffect that triggers when this card moves from library to graveyard.
    pub library_graveyard_trigger: Option<LibraryGraveyardTrigger>,
    /// Basic land subtypes printed on the card, WUBRG order.
    pub land_types: [bool; 5],
    /// True when the card has cascade: reveal from the library top until
    /// the first eligible lower printed mana value card and cast it free.
    pub has_cascade: bool,
    /// Printed power (creatures); crew and station use it instead of the
    /// flat body power. Tokens and unknowns stay flat.
    pub printed_power: Option<u32>,
    /// Printed toughness used for supported -1/-1 counter effects.
    pub printed_toughness: Option<u32>,
    /// Starting loyalty for planeswalkers; activations spend it.
    pub starting_loyalty: Option<u32>,
    /// Cost cuts scale with the artifact count on the board (improvise,
    /// affinity): the discount grows as artifacts enter, instead of the
    /// parse-time flat −2.
    pub battlefield_discount: bool,
    /// Printed colors of the card (subset of WUBRG, by index). Powers
    /// "one mana per color among permanents you control" scaling.
    pub colors: [bool; 5],
    /// Static keyword, interaction, and combat flags parsed from the
    /// keyword array and Oracle text (combat, mana, and readiness paths).
    pub flags: SimStaticFlags,
    /// Printed keywords as a runtime set (flying, deathtouch, …); static
    /// grants and keyword counters merge into this at combat time.
    pub printed_keywords: KeywordSet,
    /// True when the card carries the Companion keyword (CR 702.139). A
    /// bench card with this flag may start outside the game as the
    /// deck's companion.
    pub has_companion: bool,
    /// Morph, megamorph, or disguise cost (CR 702.37/702.168). The card
    /// may be cast face down as a 2/2 for {3}, then turned face up for
    /// this cost.
    pub morph_cost: Option<Cost>,
    /// Read ahead (CR 702.155): as this Saga enters, its controller may
    /// choose a starting chapter and enter with that many lore counters.
    /// The goldfish takes the best chapter for its board.
    pub read_ahead: bool,
    /// True when the card is an enchantment (scaling draw engines count
    /// enchantments on the battlefield).
    pub is_enchantment: bool,
    /// True when the card enters and buffs the whole board by X ("+X/+X,
    /// where X is the number of creatures you control"): a one-shot
    /// combat buff for the turn it enters.
    pub buffs_battlefield_on_entry: bool,
    /// True when the card's +1/+1 counters join its body power
    /// ("enters with X +1/+1 counters").
    pub counters_are_power: bool,
    /// Board-count-gated draw engine ("draw a card for each
    /// enchantment/artifact/land/creature you control"): the engine
    /// draws the matching permanent count each turn, capped at 8.
    pub draws_per_matching: Option<DrawMatch>,
    /// True when the card is a land/spell modal double-faced card: one
    /// face is a Land, the other is castable. It plays as a land when
    /// no other land is in hand, else it waits as a castable spell.
    pub is_mdfc_spell: bool,
    /// True when the card's rarity is mythic (the rarity column). Feeds
    /// the shared MDFC land weight.
    pub mythic: bool,
    /// Keyword abilities (CR 702) that are continuous or payment rules;
    /// triggered ones (mobilize, afterlife) live in the ability list.
    pub keyword_abilities: KeywordAbilities,
}

/// Karsten's MDFC land weight: a land/spell MDFC is one card, so its
/// land face counts as a partial source — 0.75 for a mythic (the
/// high-impact slot the deck wants on the battlefield), 0.4 otherwise.
pub fn mdfc_land_weight(mythic: bool) -> f64 {
    if mythic { 0.75 } else { 0.4 }
}

/// The permanent class a scaling draw engine counts each turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawMatch {
    /// Enchantments you control.
    Enchantments,
    /// Artifacts you control.
    Artifacts,
    /// Lands you control.
    Lands,
    /// Creatures you control.
    Creatures,
}

/// One Equipment: the suit-up cost and the equipped-creature buff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Equipment {
    /// Equip cost (mana).
    pub cost: u32,
    /// Equipped-creature buff (power, toughness).
    pub buff: (i32, i32),
    /// True for Reconfigure (CR 702.151): the gear is a creature body
    /// until it attaches, then it stops being a creature.
    pub reconfigure: bool,
}

/// One static mana grant: the permanent class it empowers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// Lands you control each tap for one any-color pip.
    Lands,
    /// Creatures you control each tap for one any-color pip.
    Creatures,
}

/// Which static mana grants are live on the battlefield.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Grants {
    /// A creatures grant is live.
    pub creatures: bool,
    /// A lands grant is live.
    pub lands: bool,
}

/// The effect class of an X-cost spell: what scales with the paid X.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XClass {
    /// "Target player loses X life" / "deals X damage".
    Drain,
    /// "Draw X cards".
    Draw,
    /// "Mill X cards".
    Mill,
    /// "Create X 1/1 tokens".
    Tokens,
    /// "Create X Treasure tokens".
    Treasures,
    /// "Reveal the top X cards, put any number of permanent cards onto
    /// the battlefield": best case X bodies join.
    RevealPermanents,
    /// "Enters with X +1/+1 counters": the counters join the body power.
    Counters,
}

impl SimCard {
    /// Abilities a permanent with `charge` charge counters has right now:
    /// striation 0 plus every striation whose threshold is reached
    /// (CR 721.2a: "{N+}[abilities]" means "as long as this permanent has
    /// N or more charge counters on it, it has [abilities]"). Trigger and
    /// upkeep paths must use this instead of an unrestricted striation scan,
    /// or locked striations fire early.
    pub fn unlocked_abilities(&self, charge: u32) -> impl Iterator<Item = &SimAbility> {
        self.striations
            .iter()
            .filter(move |tier| tier.at == 0 || charge >= tier.at)
            .flat_map(|tier| tier.abilities.iter())
    }

    /// Chapter count of a saga: the parsed chapter list length. Sagas
    /// without parsed chapters read as three chapters (the common shape).
    pub fn chapter_count(&self) -> usize {
        if !self.is_saga {
            return 0;
        }
        self.saga.chapters.len().max(3)
    }

    /// The station striation this card animates at, when any (spacecraft
    /// only).
    pub fn animate_at(&self) -> Option<u32> {
        self.striations.iter().find(|t| t.animate).map(|t| t.at)
    }
}

/// Formats the simulator knows.
///
/// Pure library shape: how big the deck is and whether a command zone
/// exists. Per-format behavior (mulligan policy, turn defaults) comes
/// from the format rules table ([`super::format`]), not from branches on
/// this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Commander, brawl, oathbreaker: 100-card singleton with a commander.
    Commander,
    /// Any 60-card constructed format.
    Constructed,
}

impl Format {
    /// Opponents a player-targeted life loss resolves against: three in
    /// the commander family ("each opponent"), one in constructed.
    pub fn life_loss_mult(self) -> u32 {
        match self {
            Self::Commander => 3,
            Self::Constructed => 1,
        }
    }

    /// Starting life total the goldfish races to zero.
    pub fn life_target(self) -> f64 {
        match self {
            Self::Commander => 120.0,
            Self::Constructed => 20.0,
        }
    }
}

/// A deck ready to simulate: the library plus the command-zone commander.
#[derive(Debug)]
pub struct SimDeck {
    /// Card storage. The library plus the companion, which needs an
    /// index so casts can resolve it (CR 702.139). The companion never
    /// shuffles into the library; see [`Self::library_len`].
    pub cards: Vec<SimCard>,
    /// Commander card(s) starting in the command zone.
    pub commanders: Vec<SimCard>,
    /// Companion (CR 702.139) starting outside the game, as an index
    /// into `cards`. A deck may reveal at most one companion (CR
    /// 103.2b), so the first bench card with the Companion keyword wins.
    pub companion: Option<CardIdx>,
    /// Library shape (command-zone singleton vs constructed).
    pub format: Format,
    /// Per-format behavior: mulligan policy and turn defaults.
    pub rules: super::format::FormatRules,
}

impl SimDeck {
    /// Cards that shuffle into the library. The companion starts outside
    /// the game (CR 702.139a), so it is not one of them.
    pub fn library_len(&self) -> usize {
        self.cards.len() - usize::from(self.companion.is_some())
    }

    /// The library cards, excluding the companion. Library-shape metrics
    /// (land counts, cast ceilings) read this, not `cards`.
    pub fn library_cards(&self) -> impl Iterator<Item = &SimCard> {
        self.cards
            .iter()
            .enumerate()
            .filter(|(pos, _)| self.companion != Some(CardIdx(*pos as u32)))
            .map(|(_, card)| card)
    }
}

/// An index into `SimDeck.cards`. A newtype so zone vectors and zone
/// maps cannot mix with lengths or turn numbers; commanders and tokens
/// are not deck cards and never carry one ([`super::game::CardRef`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CardIdx(pub u32);

impl CardIdx {
    /// The index as a `usize` position (zone vector and report use).
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl std::ops::Index<CardIdx> for SimDeck {
    type Output = SimCard;

    fn index(&self, idx: CardIdx) -> &SimCard {
        &self.cards[idx.0 as usize]
    }
}
