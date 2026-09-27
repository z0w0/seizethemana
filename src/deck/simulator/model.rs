// Card data model for the goldfish simulator: costs, tap yields, station
// tiers, crew, and abilities. Pure data + small helpers; parsing lives in
// `parse`, execution in `game`.
//
// Everything the model cannot express is dropped at parse time; the
// documented limits ship in the output `assumptions` list (see `report`).

/// Colors tracked for mana modeling, WUBRG order.
pub const COLORS: [char; 5] = ['W', 'U', 'B', 'R', 'G'];

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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cost {
    /// Generic-mana value (the `{N}` parts; `{X}` counts as 1).
    pub generic: u32,
    /// Monocolor pips per WUBRG letter.
    pub pips: [u8; 5],
    /// Hybrid/phyrexian pips, payable from any of their colors.
    pub flex_pips: u32,
}

impl Cost {
    /// Total mana value (generic plus every pip).
    pub fn total(&self) -> u32 {
        self.generic + self.pips.iter().map(|p| u32::from(*p)).sum::<u32>() + self.flex_pips
    }
}

/// What one tap of a permanent yields.
///
/// Real cards fall into three shapes: an "or" choice (`Add {G} or {U}` or
/// "one mana of any color"), a fixed simultaneous set (`Add {W}{U}{B}{R}{G}`
/// on Jegantha), and colorless-only (`{C}`). Each is one tap = the listed
/// mana, never several independent sources.
#[derive(Debug, Clone, Default)]
pub struct TapYield {
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
    pub restriction: Option<Restriction>,
}

/// Spend-restriction classes the pool can honor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restriction {
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

impl TapYield {
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

/// What starts an ability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AbilityTiming {
    /// Activated: the player pays `cost` to fire it (incl. `{T}` costs).
    Activated,
    /// Fired when the card enters the battlefield.
    #[default]
    OnEnter,
    /// Fired once per turn while the card is on the battlefield.
    OnUpkeep,
    /// Fired at the beginning of the player's first main phase.
    PrecombatMainPhase,
    /// Fired at the beginning of the player's end step.
    OnEndStep,
    /// Fired when the card attacks.
    OnAttack,
    /// Fired when the card deals combat damage to a player (best case:
    /// every attacker connects). Thrummingbird proliferate, draw, drain.
    OnCombatDamage,
    /// Fired when another spell is cast (Y'shtola, Vivi, Jhoira).
    OnCastSpell,
    /// Fired when the permanent dies or is sacrificed.
    OnDeath,
    /// Fired when a land enters under the player's control.
    OnLandfall,
}

/// What an ability does when it resolves.
#[derive(Debug, Clone, Default)]
pub enum Effect {
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
    Mana(TapYield),
    /// Untap this permanent and repeat its tap-for-mana ability.
    UntapSelf,
    /// Produce `counters × N` mana at the first main phase, then clear the host's
    /// charge counters (banked-mana engines: Coalition Relic).
    ManaPerCounter(TapYield),
    /// Create token creatures (ETB tokens, crewed payoffs). The count is
    /// advisory: the sim tracks bodies, not individual tokens.
    Tokens(u32),
    /// Put N charge counters on this permanent (counter injection).
    Counters(u32),
    /// Extra land drop this turn (explore-style effects, land-search
    /// ETBs, saga ramp chapters). Puts one card in the hand; never
    /// re-fires and never sets the blink flag.
    ExtraLand,
    /// A blink-shaped ETB ("exile … return it to the battlefield").
    /// The permanent re-fires its own OnEnter triggers once, next turn
    /// (the re-arm lives in the ETB firing path, not in this effect).
    Blink,
    /// "You become the Monarch": from the turn after acquisition the
    /// Monarch draws one extra card at upkeep (engine, not one apply).
    Monarch,
    /// Put the top N cards of the library into the graveyard. Milled
    /// cards count as cards seen (velocity) and fill the graveyard log.
    Mill(u32),
    /// Return cards from the graveyard: to the hand (`to_hand`, draw
    /// credit) or the battlefield (a body, once per card).
    ReturnFromGraveyard {
        /// True: cards go to the hand (counts as cards seen).
        to_hand: bool,
        /// How many cards to return.
        count: u32,
    },
    /// Discard the hand into the graveyard, then draw that many (a
    /// wheel). Cards seen jump by the hand size; the graveyard log fills.
    Wheel,
    /// A wheel resolving mid-cast: skip one hand card (the cast spell
    /// itself, whose removal is deferred) so it is not double-zoned.
    /// Cast-phase detail: the shared effect set never produces it.
    WheelSkip(CardIdx),
    /// Draw N, then discard N (loot). Net velocity +N; the graveyard
    /// log fills with the discards.
    Loot(u32),
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
    /// Lose N life (drain, burn at a player). Opponent-scoped phrasing
    /// ("target player", "each opponent") maps here; creature-target
    /// burn does not. Commander family resolves at ×3 (three
    /// opponents); constructed resolves at ×1.
    Drain(u32),
}

/// Oracle-triggered effect when this card moves from library to graveyard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryGraveyardTrigger {
    /// Put this card onto the battlefield.
    ReturnToBattlefield,
    /// Drain opponents and gain the same amount of life.
    DrainAndGain(u32),
}

/// Alternate casting cost parsed from the Oracle cost sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlternativeCastCost {
    /// Card category required from the hand.
    pub filter: CardFilter,
    /// Number of cards exiled to pay the cost.
    pub count: u32,
    /// Effect based on the exiled cards after the cast resolves.
    pub payoff: AlternativeCostPayoff,
}

/// Oracle-derived filter for cards used to pay an alternate cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardFilter {
    /// Required color, when named by the cost.
    pub color: Option<char>,
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
    pub color: Option<char>,
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

/// One executable ability: what fires it, what it costs, what it does.
#[derive(Debug, Clone, Default)]
pub struct Ability {
    /// What fires the ability: an activation, a trigger event, or a tap.
    pub trigger: AbilityTiming,
    /// Mana to pay when activating (0 when free / triggered).
    pub cost: Cost,
    /// Effect when the ability resolves.
    pub effect: Effect,
    /// Consumes the source's tap (mana abilities and tap-activated draws).
    pub taps: bool,
    /// Consumes one charge counter per fire (Pentad Prism's "remove a
    /// charge counter: add one mana of any color"). The source stays
    /// untapped and fires once per turn while counters last.
    pub uses_counters: bool,
    /// Bodies sacrificed as part of the cost (aristocrats outlets). The
    /// activation consumes an untapped body and fires its OnDeath
    /// triggers.
    pub sacrifice_bodies: u32,
    /// True when the card's text limits the activation ("Activate only
    /// once each turn"): free untapped activations fire once per turn
    /// instead of looping (no infinite-mana census flag).
    pub once_per_turn: bool,
    /// Loyalty spent to activate (planeswalker minus abilities). 0 = not
    /// a loyalty activation.
    pub loyalty_cost: u32,
    /// Loyalty gained by activation (planeswalker plus abilities). 0 =
    /// not a loyalty-gaining activation.
    pub loyalty_gain: u32,
    /// Life paid as an activation cost.
    pub life_cost: u32,
}

/// A station tier: abilities unlocked at a charge-counter threshold.
///
/// Rule source: CR 702.184/721 — a station card's text box has one or two
/// striations, each led by an `{N+}` symbol meaning "as long as this
/// permanent has N or more charge counters, it has [abilities]" and, when a
/// P/T box is printed in the same striation, "…and is a creature with base
/// P/T". Planets never animate (no P/T box); a spacecraft animates only at
/// the tier whose striation carries the P/T box.
#[derive(Debug, Clone, Default)]
pub struct Tier {
    /// Charge-counter threshold (`12+` → 12).
    pub at: u32,
    /// True when this tier's striation also animates the card as a creature.
    pub animate: bool,
    /// Abilities unlocked at this tier (draw engines, mana taps).
    pub abilities: Vec<Ability>,
}

/// Saga chapter data: the ordered chapter effects of a Saga card.
///
/// Rule source: CR 714.2 — each chapter is a triggered ability, indexed by
/// the Roman numeral in the Oracle text. `chapters[i]` is chapter i+1's
/// effect; `Effect::None` marks a chapter whose wording is unsupported.
#[derive(Debug, Clone, Default)]
pub struct SagaData {
    /// Chapter effects in play order (chapter I first). The length is
    /// the chapter count.
    pub chapters: Vec<Effect>,
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
    /// Cheapest castable mana value after reductions (warp, affinity-lite).
    pub min_cost: Cost,
    /// True when the card has a printed mana cost. A missing cost is not
    /// equivalent to a printed `{0}` cost.
    pub has_mana_cost: bool,
    /// Mana this card produces on tap; None when it cannot tap for mana.
    pub tap: Option<TapYield>,
    /// Station tiers, in threshold order (empty when not a station card).
    pub station_tiers: Vec<Tier>,
    /// Crew cost for Vehicles (None when not a Vehicle).
    pub crew: Option<u32>,
    /// Creature-ness for body counting and dork timing.
    pub is_creature: bool,
    /// True when the creature type line includes Human.
    pub is_human: bool,
    /// True for instant and sorcery spells, which go to the graveyard after
    /// resolving instead of remaining on the battlefield.
    pub is_instant_or_sorcery: bool,
    /// True when this spell exiles itself after resolving.
    pub exile_on_resolve: bool,
    /// True when the type line carries Legendary. Gates the
    /// legendary-only spend restriction (Plaza of Heroes).
    pub is_legendary: bool,
    /// Artifact-ness for improvise/affinity discount math.
    pub is_artifact: bool,
    /// True when the card is a Spacecraft or Planet (stationable type).
    pub is_station_card: bool,
    /// Charge counters gained on entering (Reckoner Bankbuster).
    pub enter_counters: u32,
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
    pub fetch_target_types: [bool; 5],
    /// True when Oracle text restricts the search to basic lands.
    pub fetch_basic_only: bool,
    /// True when the search effect itself puts its target onto the battlefield tapped.
    pub fetch_enters_tapped: bool,
    /// Basic types this land checks for (verge gates: "control a Swamp").
    pub gate_types: Vec<&'static str>,
    /// True when the card may begin the game on the battlefield (Leyline).
    pub opens_in_play: bool,
    /// True when the card is a saga (staged per-turn effects).
    pub is_saga: bool,
    /// Saga chapter effects in play order (empty when not a saga).
    pub saga: SagaData,
    /// One-shot charge-counter injection on cast (Drill Too Deep).
    pub counters_on_cast: u32,
    /// One-shot mana on cast (rituals), never joining the tap pool.
    pub mana_on_cast: Option<TapYield>,
    /// Repeatable mana on cast: "add {N} for each spell you've cast this
    /// turn" (Vivi class). Joins the pool per spell cast, every turn the
    /// host is on the battlefield.
    pub mana_per_cast: Option<TapYield>,
    /// One-shot draw on cast (cantrips, Divination).
    pub draws_on_cast: u32,
    /// Life gained when this spell resolves.
    pub life_gain_on_cast: u32,
    /// Oracle-derived alternate cost, when the spell has one.
    pub alternative_cast_cost: Option<AlternativeCastCost>,
    /// Oracle-derived reveal effect, when the spell reveals cards for life.
    pub reveal_rule: Option<RevealRule>,
    /// One-shot mill on cast or on entering ("mill N").
    pub mills_on_enter: u32,
    /// One-shot token creation on cast ("Create N 1/1 Soldier tokens").
    /// The count feeds the token-body path (Treasure cards bank).
    pub tokens_on_cast: u32,
    /// One-shot scry count on cast. Awareness credit, not draw.
    pub scry_on_cast: u32,
    /// One-shot surveil count on cast. The cards move to the graveyard.
    pub surveils_on_cast: u32,
    /// Mill effects target opponents ("target player mills N") instead
    /// of the deck's own library (deck-out pressure direction).
    pub mills_opponent: bool,
    /// One-shot extra turn on cast ("take an extra turn").
    pub extra_turns_on_cast: bool,
    /// One-shot life loss at a player on cast (burn, drain).
    pub drain_on_cast: u32,
    /// Bodies the cast consumes ("as an additional cost to cast this
    /// spell, sacrifice a creature"). The cast consumes a body.
    pub additional_cost_bodies: u32,
    /// Cards discarded from hand as an additional cast cost.
    pub additional_cost_discards: u32,
    /// Life the cast costs on top of mana ("as an additional cost …
    /// pay N life"). Best case: the agent pays it.
    pub additional_cost_life: u32,
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
    /// The source sacrifices itself when its parsed mana ability resolves.
    pub sacrifices_for_mana: bool,
    /// True when Oracle text grants undying.
    pub has_undying: bool,
    /// Cycling activation cost.
    pub cycling_cost: Option<Cost>,
    /// Life paid for cycling (Street Wraith-style cycling).
    pub cycling_life: u32,
    /// Land type searched by landcycling.
    pub landcycling_type: Option<char>,
    /// Number of cards milled when this card replaces a draw from the graveyard.
    pub dredge: Option<u32>,
    /// Effect that triggers when this card moves from library to graveyard.
    pub library_graveyard_trigger: Option<LibraryGraveyardTrigger>,
    /// Basic land subtypes printed on the card, WUBRG order.
    pub land_types: [bool; 5],
    /// One-shot wheel on cast ("each player discards their hand, then
    /// draws seven"): the cast resolves a full wheel.
    pub wheel_on_cast: bool,
    /// X-cost effect class: the spell pays the leftover pool as X and
    /// scales its effect (drain X, draw X, mill X, tokens X). None when
    /// not an X spell. `Effect::Drain(0)` carries the class; the game
    /// loop substitutes the paid X.
    pub x_class: Option<XClass>,
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
    pub board_discount: bool,
    /// Printed colors of the card (subset of WUBRG, by index). Powers
    /// "one mana per color among permanents you control" scaling.
    pub colors: [bool; 5],
    /// Treasure tokens created per token effect (Stark Industries
    /// Executive). Each treasure is one banked any-color pip, sacrificed
    /// to use; the bank lives on the game state, not the card.
    pub treasures_on_token: bool,
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
    /// Equipment stats: (equip cost, equipped-creature buff, draws when
    /// the equipped creature dies). None when not Equipment.
    pub equipment: Option<Equipment>,
    /// Attack power ×2 (double strike). Goldfish: no blockers, so the
    /// first-strike layer is pure damage multiplication.
    pub double_strike: bool,
    /// +1 power per noncreature spell cast this turn (prowess),
    /// credited in the combat phase of the same turn.
    pub prowess: bool,
    /// +1 power per land drop made after this permanent entered
    /// (landfall +1/+1 counter patterns).
    pub landfall: bool,
    /// Evasion census flag (trample, flying, menace): counted in the
    /// attack block, no math.
    pub evasion: bool,
    /// Castable at instant speed (Instant type or flash). Powers the
    /// interaction-readiness metric.
    pub is_instant_speed: bool,
    /// Interaction role (removal or counterspells): feeds readiness.
    pub is_interaction: bool,
    /// True when the creature ignores summoning sickness (haste):
    /// attacks, taps, and crews the turn it enters.
    pub has_haste: bool,
    /// Extra land drop each turn ("you may play an additional land").
    /// Feeds `land_drops[]` as one extra drop per turn while in play.
    pub extra_land_drops: bool,
    /// Optional kicker cost (pips and generic); paid from spare mana when
    /// affordable. The kicker rider bumps drain/damage amounts.
    pub kicker: Option<Cost>,
    /// True when the card destroys or sweeps every permanent of a class
    /// ("destroy all creatures"): a board wipe. Wipes count as
    /// interaction capacity but never fire in a goldfish.
    pub wipe: bool,
    /// True when the card is an enchantment (scaling draw engines count
    /// enchantments on the battlefield).
    pub is_enchantment: bool,
    /// True when the card enters and buffs the whole board by X ("+X/+X,
    /// where X is the number of creatures you control"): a one-shot
    /// combat buff for the turn it enters.
    pub buffs_board_on_enter: bool,
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
}

/// One static mana grant: the permanent class it empowers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// Lands you control each tap for one any-color pip.
    Lands,
    /// Creatures you control each tap for one any-color pip.
    Creatures,
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
    /// "Reveal the top X cards, put any number of permanent cards onto
    /// the battlefield": best case X bodies join.
    RevealPermanents,
    /// "Enters with X +1/+1 counters": the counters join the body power.
    Counters,
}

impl SimCard {
    /// Abilities from the base card (tier 0) plus unlocked tiers.
    pub fn abilities(&self) -> impl Iterator<Item = &Ability> {
        self.station_tiers.iter().flat_map(|t| t.abilities.iter())
    }

    /// Chapter count of a saga: the parsed chapter list length. Sagas
    /// without parsed chapters read as three chapters (the common shape).
    pub fn chapter_count(&self) -> usize {
        if !self.is_saga {
            return 0;
        }
        self.saga.chapters.len().max(3)
    }

    /// The station tier this card animates at, when any (spacecraft only).
    pub fn animate_at(&self) -> Option<u32> {
        self.station_tiers.iter().find(|t| t.animate).map(|t| t.at)
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
    /// Opponents a player-targeted drain resolves against: three in the
    /// commander family ("each opponent"), one in constructed.
    pub fn drain_mult(self) -> u32 {
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
    /// Cards in the library (commander excluded for commander decks).
    pub cards: Vec<SimCard>,
    /// Commander card(s) starting in the command zone.
    pub commanders: Vec<SimCard>,
    /// Library shape (command-zone singleton vs constructed).
    pub format: Format,
    /// Per-format behavior: mulligan policy and turn defaults.
    pub rules: super::format::FormatRules,
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

/// Cards drawn when a draw spell resolves: numerals and number words win
/// ("draw two cards", "draw seven cards"), otherwise 1.
pub fn draw_amount(text: &str) -> u32 {
    if !text.contains("draw ") && !text.contains("draws ") && !text.contains("investigate") {
        return 0;
    }
    for (word, n) in [
        ("seven", 7u32),
        ("six", 6),
        ("five", 5),
        ("four", 4),
        ("three", 3),
        ("two", 2),
        ("5", 5),
        ("4", 4),
        ("3", 3),
        ("2", 2),
    ] {
        if text.contains(&format!("draw {word}")) || text.contains(&format!("draws {word}")) {
            return n;
        }
    }
    1
}

/// Number-word and numeral amounts for generic clauses ("scry 2",
/// "look at the top three cards"). Returns 1 when present but uncounted.
pub fn amount_after(text: &str, needle: &str) -> u32 {
    let mut from = 0;
    while let Some(rel) = text[from..].find(needle) {
        let tail = &text[from + rel + needle.len()..];
        let digits: String = tail
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<u32>() {
            return n;
        }
        for (word, n) in [
            ("seven", 7u32),
            ("six", 6),
            ("five", 5),
            ("four", 4),
            ("three", 3),
            ("two", 2),
        ] {
            if tail.trim_start().starts_with(word) {
                return n;
            }
        }
        from += rel + needle.len();
    }
    1
}

/// True when the text reads a mill clause against the opponent. Plain
/// "mill" is self-mill; the opponent shapes name a target.
pub fn mills_opponent(text: &str) -> bool {
    text.contains("target player mills")
        || (text.contains("target opponent") && text.contains("mill"))
        || text.contains("each opponent mills")
}
