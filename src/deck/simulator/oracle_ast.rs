//! Typed syntax tree for the supported subset of Magic Oracle text.

use super::model::{
    AlternativeCastCost, BasicLandType, Cost, DrawMatch, EnterCounters, Equipment, RevealRule,
    XClass,
};

/// Parsed resolution effects, including unsupported Oracle text.
mod effects;

pub use effects::OracleEffect;
/// Parsed land-entry conditions, separate from runtime predicates.
mod land;
pub use land::OracleLandEntryCondition;
/// Qualities and payments used by Behold instructions.
pub mod behold;
/// Selectors for flat generic spell-cost reductions.
mod costs;
pub use costs::{OracleSpellCostReduction, OracleSpellFilter, OracleSpellType};

/// Typed battlefield selections and creature rules with limited runtime support.
pub mod tribal;

/// Multiple-result search selectors and destination capacities.
pub mod search;

/// Parsed keyword abilities and Oracle statements for one card.
#[derive(Debug, Clone)]
pub struct OracleCard {
    /// Keywords supplied by Scryfall or written as standalone text lines.
    pub keywords: Vec<OracleKeyword>,
    /// Triggered, activated, static, spell, chapter, and unsupported nodes.
    pub abilities: Vec<OracleAbility>,
    /// Original Oracle wording and byte spans for nonsynthetic abilities.
    pub ability_sources: Vec<OracleAbilitySource>,
    /// Cast costs and spell facts read from their matching Oracle clauses.
    pub spell_data: OracleSpellData,
    /// Card facts parsed once from their owning Oracle statements.
    pub static_data: OracleStaticData,
    /// Land-entry and fetch rules parsed from this card's Oracle text.
    pub land: OracleLand,
    /// Station reminder tiers parsed from Oracle text.
    pub stations: OracleStations,
    /// Effect that triggers when this card moves from library to graveyard.
    pub library_graveyard_trigger: Option<OracleLibraryGraveyardEffect>,
}

/// Original wording for an ability before source-name normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleAbilitySource {
    /// Exact source text when a contiguous source span is available.
    pub text: String,
    /// Byte range in the card's Oracle text, excluding surrounding space.
    pub span: Option<OracleSourceSpan>,
}

/// Half-open byte range in the original Oracle text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct OracleSourceSpan {
    /// First byte of the ability.
    pub start: usize,
    /// Byte immediately after the ability.
    pub end: usize,
}

/// Supported action on a library-to-graveyard trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleLibraryGraveyardEffect {
    /// Return this card to the battlefield.
    ReturnToBattlefield,
    /// Drain opponents and gain the same amount of life.
    DrainAndGain(u32),
}

/// Station-specific syntax and type-line metadata.
#[derive(Debug, Clone, Default)]
pub struct OracleStations {
    /// Whether the type line identifies a Spacecraft or Planet.
    pub is_station_card: bool,
    /// Whether Oracle text contains parsed station reminder tiers.
    pub has_station_ability: bool,
    /// Activated abilities and animation thresholds listed by striation.
    pub striations: Vec<OracleStriation>,
}

/// One numbered station striation.
#[derive(Debug, Clone)]
pub struct OracleStriation {
    /// Charge-counter threshold for the listed abilities.
    pub at: u32,
    /// Whether this threshold also turns a spacecraft into a creature.
    pub animate: bool,
    /// Parsed activated abilities printed at this threshold.
    pub abilities: Vec<OracleActivatedAbility>,
}

/// Land-specific rules parsed from Oracle text.
#[derive(Debug, Clone, Default)]
pub struct OracleLand {
    /// An optional Behold choice that avoids tapped entry.
    pub behold_untapped_entry: Option<behold::OracleBeholdSpec>,
    /// Default tapped entry before a supported state condition is checked.
    pub enters_tapped: bool,
    /// Supported state that lets an otherwise tapped land enter untapped.
    pub untapped_entry_condition: Option<OracleLandEntryCondition>,
    /// Life paid to have the land enter untapped.
    pub life_to_untap: u32,
    /// Fetch target restriction, when this card is a fetch land.
    pub fetch: Option<OracleFetch>,
    /// Basic land types that gate a mana mode.
    pub gates: Vec<BasicLandType>,
    /// Whether a searched land enters tapped.
    pub fetch_enters_tapped: bool,
    /// Implicit any-color mana from a chosen or all-basic-types land.
    pub produces_any_color: bool,
}

/// Fetch target restriction parsed from a land's search ability.
#[derive(Debug, Clone)]
pub struct OracleFetch {
    /// A Behold choice that untaps this search's result after the shuffle.
    pub behold_untap: Option<behold::OracleBeholdSpec>,
    /// WUBRG types the search can find.
    pub target_types: Vec<BasicLandType>,
    /// Whether the target must have the Basic supertype.
    pub basic_only: bool,
    /// Life paid to activate the fetch ability.
    pub life_cost: u32,
}

impl OracleCard {
    /// Return true when the card has a keyword with this parsed name.
    pub fn has_keyword(&self, expected: &OracleKeywordName) -> bool {
        self.keywords
            .iter()
            .any(|keyword| &keyword.name == expected)
    }

    /// Return a numeric parameter attached to a keyword, when present.
    pub fn keyword_number(&self, expected: &OracleKeywordName) -> Option<u32> {
        self.keywords
            .iter()
            .filter(|keyword| &keyword.name == expected)
            .find_map(|keyword| {
                keyword
                    .arguments
                    .iter()
                    .find_map(|argument| match argument {
                        KeywordArgument::Number(value) => Some(*value),
                        _ => None,
                    })
            })
    }

    /// Return a mana-cost parameter attached to a keyword, when present
    /// ("Offspring {1}{G}").
    pub fn keyword_cost(&self, expected: &OracleKeywordName) -> Option<Cost> {
        self.keywords
            .iter()
            .filter(|keyword| &keyword.name == expected)
            .find_map(|keyword| {
                keyword
                    .arguments
                    .iter()
                    .find_map(|argument| match argument {
                        KeywordArgument::Mana(cost) => Some(*cost),
                        _ => None,
                    })
            })
    }

    /// Return a life-payment parameter attached to a keyword, when present
    /// ("Cycling—Pay 2 life").
    pub fn keyword_life(&self, expected: &OracleKeywordName) -> Option<u32> {
        self.keywords
            .iter()
            .filter(|keyword| &keyword.name == expected)
            .find_map(|keyword| {
                keyword
                    .arguments
                    .iter()
                    .find_map(|argument| match argument {
                        KeywordArgument::Life(amount) => Some(*amount),
                        _ => None,
                    })
            })
    }

    /// Return the basic land type named by a landcycling keyword, when
    /// present ("Forestcycling" → Forest).
    pub fn basic_land_type(&self) -> Option<BasicLandType> {
        self.keywords.iter().find_map(|keyword| {
            keyword
                .arguments
                .iter()
                .find_map(|argument| match argument {
                    KeywordArgument::BasicLandType(land) => Some(*land),
                    _ => None,
                })
        })
    }

    /// Return parsed Saga chapters in Oracle order.
    pub fn saga_chapters(&self) -> impl Iterator<Item = &OracleSagaChapter> {
        self.abilities.iter().filter_map(|ability| match ability {
            OracleAbility::SagaChapter(chapter) => Some(chapter),
            _ => None,
        })
    }
}

/// One ability statement in Oracle text.
#[derive(Debug, Clone)]
pub enum OracleAbility {
    /// An ability activated by paying a cost.
    Activated(OracleActivatedAbility),
    /// An ability that triggers from a game event.
    Triggered(OracleTriggeredAbility),
    /// A continuous rule that applies while a condition holds.
    Static(OracleStaticAbility),
    /// A one-shot effect from casting or resolving a spell.
    Spell(OracleSpellAbility),
    /// One or more Saga chapter abilities.
    SagaChapter(OracleSagaChapter),
    /// Text that was kept but does not match a supported syntax shape.
    #[allow(dead_code)] // Keep unknown Oracle wording available to parser clients.
    Unsupported(OracleUnsupported),
}

/// An activated ability with parsed costs, restrictions, and effects.
#[derive(Debug, Clone)]
pub struct OracleActivatedAbility {
    /// Individual payment instructions from the activation cost.
    pub costs: Vec<ActivationCost>,
    /// Parsed effects, including an explicit node for unsupported text.
    pub effects: Vec<OracleEffect>,
    /// Restrictions that limit when or how often the ability can be used.
    pub restrictions: Vec<AbilityRestriction>,
    /// True when this activation meets CR 605.1a-b's mana-ability rules.
    pub is_mana_ability: bool,
    /// This mana output scales with the number of spells cast this turn.
    pub mana_per_spell_cast: bool,
}

/// One payment component of an activated ability.
#[derive(Debug, Clone)]
#[allow(dead_code)] // The AST keeps costs that the runtime model cannot pay yet.
pub enum ActivationCost {
    /// Complete selected-permanent payment without runtime support yet.
    Tribal(tribal::OracleTribalCost),
    /// Mana paid to activate the ability.
    Mana(Cost),
    /// Tap the source or another object.
    Tap(ActivationTarget),
    /// Untap the source or another object.
    Untap(ActivationTarget),
    /// Sacrifice one or more objects.
    Sacrifice {
        /// Number of objects sacrificed.
        count: u32,
        /// Required object class.
        object: CostObject,
        /// Prevent the source from paying a cost that requires another object.
        exclude_source: bool,
    },
    /// Pay life as part of the activation cost.
    PayLife(u32),
    /// Discard cards as part of the activation cost.
    Discard(u32),
    /// Remove counters from an object.
    RemoveCounter {
        /// Counter kind when stated in the cost.
        kind: Option<String>,
        /// Number of counters removed.
        count: u32,
    },
    /// Add or remove loyalty counters.
    Loyalty(i32),
    /// Pay energy counters.
    Energy(u32),
    /// Preserve a cost phrase the parser does not yet classify.
    Other(String),
}

/// Object selected by a tap or untap cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationTarget {
    /// The permanent with the ability.
    Source,
    /// Another creature or permanent.
    AnotherObject,
}

/// Object class selected by a sacrifice cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostObject {
    /// A creature.
    Creature,
    /// The permanent with this ability.
    Source,
    /// Any permanent or card.
    Any,
}

/// A restriction on activating an ability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbilityRestriction {
    /// Control a land with any listed basic land type, in WUBRG order.
    ControlsLandTypes([bool; 5]),
    /// The activation is limited to once per turn.
    OncePerTurn,
    /// The activation is limited to once per game (Exhaust, CR 702.177;
    /// Power-up, CR 702.193).
    OncePerGame,
    /// The activation is a power-up ability (CR 702.193): its cost is
    /// reduced by the permanent's mana cost the turn it entered.
    PowerUp,
    /// The activation is limited to a sorcery-speed window.
    SorcerySpeed,
    /// A condition that remains visible but is not evaluated by the sim.
    Condition(String),
}

/// A typed event that can cause a triggered ability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OracleTriggerEvent {
    /// The creature equipped by this permanent dies, not any creature.
    EquippedCreatureDies,
    /// One source ability listens for any of these separate events.
    AnyOf(Vec<OracleTriggerEvent>),
    /// A permanent enters the battlefield.
    Enters(ObjectSubject),
    /// The player's upkeep step begins (CR 603.2b).
    BeginningOfUpkeep(PlayerScope),
    /// The player's end step begins (CR 603.2b).
    BeginningOfEndStep(PlayerScope),
    /// The player's precombat main phase begins (CR 603.2b).
    BeginningOfPrecombatMain(PlayerScope),
    /// Another phase or step begins; the phase or step name rides in the
    /// text and the simulator does not model its timing.
    BeginningOfOther {
        /// The phase or step named by the text ("combat", "draw step").
        phase: String,
        /// Player whose phase or step begins.
        player: PlayerScope,
    },
    /// A permanent attacks.
    Attacks(ObjectSubject),
    /// The player attacks; this event occurs once per combat.
    PlayerAttacks,
    /// A permanent deals combat damage to a player.
    CombatDamageToPlayer(ObjectSubject),
    /// A player casts a spell.
    CastsSpell {
        /// Whether this card's own spell was cast.
        this_spell: bool,
    },
    /// An opponent casts a spell, subject to the ability's event filter.
    OpponentSpellCast {
        /// Whether creature spells are excluded.
        noncreature: bool,
        /// Whether only the opponent's first such spell each turn qualifies.
        first: bool,
    },
    /// An opponent draws one card.
    OpponentDraw,
    /// A permanent dies or is put into a graveyard.
    Dies(ObjectSubject),
    /// A land enters under a player's control.
    LandEnters(PlayerScope),
    /// A nonland permanent you control is tapped to activate a mana
    /// ability. Every supported wording taps one of your nonland
    /// permanents, so the variant carries no fields.
    TappedForMana,
    /// A state trigger on a counter threshold: "When [this] has N or
    /// more [kind] counters on it" (Darksteel Reactor class). The
    /// threshold number rides in the event text, not the resolution.
    WinsAtCounters {
        /// Counters needed to win.
        counters: u32,
    },
    /// The Ring tempts the player (CR 701.54d): "Whenever the Ring
    /// tempts you, …".
    RingTempts,
    /// The event is kept as text because the event grammar is unsupported.
    Other(String),
}

/// Player or object scope named by Oracle text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerScope {
    /// The simulated player.
    You,
    /// An opponent.
    Opponent,
    /// A player not further specified.
    Any,
}

/// Object that caused or satisfies a trigger event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectSubject {
    /// The permanent with the ability.
    ThisPermanent,
    /// Another creature or permanent.
    AnotherPermanent,
    /// A land controlled by the stated player.
    ControlledLand,
    /// A subject not further specified.
    Any,
}

/// A triggered ability with its event, condition, and resolution effects.
#[derive(Debug, Clone)]
pub struct OracleTriggeredAbility {
    /// Event that causes the ability to trigger.
    pub event: OracleTriggerEvent,
    /// Effects of the triggered ability.
    pub effects: Vec<OracleEffect>,
    /// Whether this is a triggered mana ability under CR 605.1b.
    pub is_mana_ability: bool,
    /// Whether the text limits this ability to once each turn.
    pub once_per_turn: bool,
    /// Intervening "if" clause text (CR 603.4), when the trigger
    /// sentence carries one between the event and the resolution.
    /// Unsupported conditions prevent runtime lowering.
    pub condition: Option<String>,
    /// Complete recognized type filters and any unmodeled event restriction.
    pub event_filter: OracleEventFilter,
    /// Optional controller payment or opponent tax offered on resolution.
    pub resolution_payment: Option<OracleResolutionPayment>,
}

/// Object restrictions retained separately from trigger timing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OracleEventFilter {
    /// Required card types. Every listed type must match.
    pub card_types: Vec<String>,
    /// Required subtypes, using lowercase Oracle words.
    pub subtypes: Vec<String>,
    /// Whether creature spells are excluded.
    pub noncreature: bool,
    /// Whether token permanents are excluded.
    pub nontoken: bool,
    /// Whether only that player's first matching event each turn qualifies.
    pub first_each_turn: bool,
    /// Restriction the runtime cannot evaluate; it gates the whole ability.
    pub unsupported: Option<String>,
}

/// A payment tied to a triggered ability's effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OracleResolutionPayment {
    /// Optional mana paid by the controller before receiving the effects.
    PayMana(Cost),
    /// Optional mana paid by the triggering opponent to prevent the effects.
    OpponentTax(OracleTaxAmount),
    /// Required payment wording that is not executable.
    Unsupported(String),
}

/// Fixed or source-dependent opponent tax amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleTaxAmount {
    /// A fixed mana cost.
    Mana(Cost),
    /// Generic mana equal to the source's power at resolution.
    SourcePower,
}

/// A static ability and its continuous effects.
#[derive(Debug, Clone)]
pub struct OracleStaticAbility {
    /// Parsed continuous rules or explicit unsupported nodes.
    pub effects: Vec<OracleStaticEffect>,
}

/// A parsed continuous effect.
#[derive(Debug, Clone)]
pub enum OracleStaticEffect {
    /// A complete spell selector and its flat generic cost reduction.
    SpellCostReduction(OracleSpellCostReduction),
    /// A complete scoped rule without runtime support yet.
    Tribal(tribal::OracleTribalStaticEffect),
    /// Grant a keyword to a class of permanents.
    KeywordGrant {
        /// Objects that receive the keyword.
        target: StaticTarget,
        /// Granted keyword.
        keyword: OracleKeyword,
    },
    /// Change power and toughness for a class of creatures.
    CreatureBuff {
        /// Power adjustment.
        power: i32,
        /// Toughness adjustment.
        toughness: i32,
    },
    /// Reduce the cost of a spell class.
    CostReduction {
        /// Number of generic mana reduced.
        amount: u32,
        /// Spell class, retained in Oracle words.
        spell_class: String,
    },
    /// Allow an additional land play.
    AdditionalLandDrop(u32),
    /// A permanent that skips the untap step ("This artifact doesn't
    /// untap during your untap step").
    DoesntUntap,
    /// Win when this permanent reaches a stated counter threshold.
    WinsAtCounters(u32),
    /// Preserve a static rule not yet modeled. Kept in the syntax tree for
    /// parser clients; the simulator never reads the text.
    #[allow(dead_code)] // Unsupported clauses remain in the syntax tree and stay inert.
    Unsupported(String),
}

/// Objects selected by a supported static rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticTarget {
    /// This card itself.
    Source,
    /// Creatures the player controls.
    CreaturesYouControl,
    /// Lands the player controls.
    LandsYouControl,
    /// Spells the player casts.
    SpellsYouCast,
    /// Objects the player controls, with no narrower class.
    PermanentsYouControl,
}

/// A one-shot spell effect.
#[derive(Debug, Clone)]
pub struct OracleSpellAbility {
    /// Parsed resolution effects.
    pub effects: Vec<OracleEffect>,
}

/// Cast costs and one-shot spell facts parsed from Oracle statements.
#[derive(Debug, Clone, Default)]
pub struct OracleSpellData {
    /// Reveal or choose a matching object, or pay extra generic mana.
    pub behold_alternative: Option<behold::OracleBeholdAlternative>,
    /// An additional cast requirement is present but cannot be paid exactly.
    pub unsupported_additional_cost: bool,
    /// Additional cost a cast must pay (CR 118.8).
    pub additional_cost: Option<OracleAdditionalCost>,
    /// Alternative cost a cast may pay instead of its mana cost
    /// (CR 118.9).
    pub alternative_cost: Option<AlternativeCastCost>,
    /// A reveal-and-life-loss rule (Ad Nauseam class).
    pub reveal_rule: Option<RevealRule>,
    /// The spell exiles itself after resolving.
    pub exiles_on_resolve: bool,
    /// The spell searches using the sacrificed creature's mana value.
    pub search_after_sacrifice: Option<super::model::SacrificeSearch>,
    /// The spell exchanges graveyard creatures for battlefield creatures.
    pub graveyard_creature_exchange: bool,
    /// The spell grants flashback to instant and sorcery cards in the
    /// graveyard as it resolves.
    pub grants_flashback: bool,
    /// The spell grants escape to nonland cards in the graveyard.
    pub grants_escape: bool,
    /// The X-cost effect class for an `{X}` spell.
    pub x_class: Option<XClass>,
}

/// An additional cost a spell must pay as it is cast (CR 118.8).
#[derive(Debug, Clone, Copy, Default)]
pub struct OracleAdditionalCost {
    /// Creatures the cast sacrifices.
    pub sacrifice_creatures: u32,
    /// Artifact-or-creature sacrifices; lowering currently selects creatures.
    pub sacrifice_artifacts_or_creatures: u32,
    /// Cards the cast discards from the hand.
    pub discard_cards: u32,
    /// Life the cast pays.
    pub pay_life: u32,
}

/// Card facts parsed from Oracle statements and card metadata.
#[derive(Debug, Clone, Default)]
pub struct OracleStaticData {
    /// Generic mana cost paid once per age counter during cumulative upkeep.
    pub cumulative_upkeep: Option<Cost>,
    /// An upkeep drawback exists but its payment is not executable.
    pub unsupported_cumulative_upkeep: bool,
    /// The card may begin the game on the battlefield (CR 103.6a).
    pub starts_on_battlefield: bool,
    /// The controller has no maximum hand size (CR 402.2).
    pub no_max_hand_size: bool,
    /// A draw effect that scales with a controlled permanent class.
    pub draw_per_controlled_permanent: Option<DrawMatch>,
    /// Counters the card enters with, scoped to its entry statement.
    pub enter_counters: EnterCounters,
    /// Whether this permanent skips its untap step.
    pub doesnt_untap: bool,
    /// Whether the card has flash as a keyword.
    pub has_flash: bool,
    /// Static bonus while this permanent is saddled.
    pub saddled_buff: Option<(i32, i32)>,
    /// Parsed equipment or reconfigure data.
    pub equipment: Option<Equipment>,
    /// Whether a mill effect targets an opponent.
    pub mills_opponent: bool,
    /// Removal and counterspell shape used by the readiness diagnostic.
    pub is_interaction: bool,
    /// Board-wide removal shape used by the readiness diagnostic.
    pub sweeps: bool,
}

/// One Saga chapter trigger, including combined chapter symbols.
#[derive(Debug, Clone)]
pub struct OracleSagaChapter {
    /// Chapter numbers that share this effect.
    pub chapters: Vec<u32>,
    /// Effects that resolve for each listed chapter.
    pub effects: Vec<OracleEffect>,
}

/// Return true when an effect does more than produce mana.
///
/// A CR 605 mana ability must not have a non-mana effect. Unsupported text
/// counts when it mentions the library. The activation and trigger mana
/// checks share this test.
pub fn is_non_mana_effect(effect: &OracleEffect) -> bool {
    match effect {
        OracleEffect::Search(_)
        | OracleEffect::MultiSearch(_)
        | OracleEffect::SacrificeSearch(_)
        | OracleEffect::Draw(_)
        | OracleEffect::Behold { .. }
        | OracleEffect::SearchThenBeholdUntap { .. }
        | OracleEffect::LookAtTop { .. }
        | OracleEffect::PutHandOnTop(_)
        | OracleEffect::DrawThenDiscard(_)
        | OracleEffect::DrawDiscard { .. }
        | OracleEffect::DrawAndMinusCounter
        | OracleEffect::Scry(_)
        | OracleEffect::Surveil(_)
        | OracleEffect::Mill(_) => true,
        OracleEffect::Unsupported(text) => text.to_ascii_lowercase().contains("library"),
        _ => false,
    }
}

/// Target class named by a damage effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageTarget {
    /// A player or opponent.
    Player,
    /// A creature or planeswalker.
    Permanent,
    /// A battle.
    Battle,
    /// "Any target" does not distinguish players from permanents.
    AnyTarget,
    /// A target class the parser does not model.
    Other,
}

/// A keyword ability read from Scryfall or Oracle text.
#[derive(Debug, Clone)]
pub struct OracleKeyword {
    /// Parsed keyword identifier, including names not yet modeled here.
    pub name: OracleKeywordName,
    /// Numeric keyword parameters.
    pub arguments: Vec<KeywordArgument>,
}

/// A keyword name known to the parser or preserved as a future name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OracleKeywordName {
    /// Flying.
    Flying,
    /// Haste.
    Haste,
    /// Double strike.
    DoubleStrike,
    /// Prowess.
    Prowess,
    /// Trample.
    Trample,
    /// Menace.
    Menace,
    /// Flash.
    Flash,
    /// Undying.
    Undying,
    /// Cascade.
    Cascade,
    /// Crew.
    Crew,
    /// Cycling.
    Cycling,
    /// Basic landcycling.
    BasicLandcycling,
    /// Landcycling.
    Landcycling,
    /// Dredge.
    Dredge,
    /// Kicker.
    Kicker,
    /// Flashback.
    Flashback,
    /// Escape.
    Escape,
    /// Transform.
    Transform,
    /// Ward.
    Ward,
    /// First strike.
    FirstStrike,
    /// Deathtouch.
    Deathtouch,
    /// Lifelink.
    Lifelink,
    /// Vigilance.
    Vigilance,
    /// Reach.
    Reach,
    /// Defender.
    Defender,
    /// Indestructible.
    Indestructible,
    /// Hexproof.
    Hexproof,
    /// Protection.
    Protection,
    /// Affinity.
    Affinity,
    /// Improvise.
    Improvise,
    /// Companion (CR 702.139): the card may start outside the game.
    Companion,
    /// Reconfigure (CR 702.151): attach or unattach an Equipment that is
    /// also a creature.
    Reconfigure,
    /// Warp (CR 702.185): cast from hand for an alternative warp cost.
    Warp,
    /// Read ahead (CR 702.155): a Saga enters with a chosen starting
    /// number of lore counters.
    ReadAhead,
    /// Morph (CR 702.37): cast the card face down as a 2/2, then turn it
    /// face up by paying the morph cost.
    Morph,
    /// Megamorph (CR 702.37): morph whose face-up turn adds a +1/+1
    /// counter.
    Megamorph,
    /// Disguise (CR 702.168): morph-like, with ward {2}.
    Disguise,
    /// Manifest (CR 701.40): put the top card face down as a 2/2; it may
    /// be turned face up by paying its mana cost.
    Manifest,
    /// Proliferate (CR 701.34): add one counter of each kind already
    /// present.
    Proliferate,
    /// Mobilize (CR 702.181): attack trigger creating N tapped-and-
    /// attacking Warrior tokens sacrificed at the next end step.
    Mobilize,
    /// Saddle (CR 702.171): tap creatures with total power N to mark
    /// the permanent saddled until end of turn.
    Saddle,
    /// Storm (CR 702.40): copy the spell once per other spell cast
    /// before it this turn.
    Storm,
    /// Convoke (CR 702.51): tap creatures to pay the spell's mana.
    Convoke,
    /// Delve (CR 702.66): exile graveyard cards to pay generic mana.
    Delve,
    /// Offspring (CR 702.175): pay an extra cost on cast to create a
    /// 1/1 token copy on entry.
    Offspring,
    /// Plot (CR 702.170): exile from hand now, cast free on a later
    /// turn.
    Plot,
    /// Living metal (CR 702.161): during your turn the Vehicle is an
    /// artifact creature.
    LivingMetal,
    /// Afterlife (CR 702.135): on death create N 1/1 flying Spirits.
    Afterlife,
    /// Power-up (CR 702.193): once-per-game activated ability with an
    /// entry-turn discount.
    PowerUp,
    /// Teamwork (CR 702.194): optional additional cost tapping
    /// creatures with total power N.
    Teamwork,
    /// Exhaust (CR 702.177): once-per-game activated ability.
    Exhaust,
    /// Amass (CR 701.47): grow or create an Army with N +1/+1
    /// counters.
    Amass,
    /// Explore (CR 701.44): reveal the top card; land to hand, else a
    /// +1/+1 counter.
    Explore,
    /// Connive (CR 701.50): draw N, discard N, then a +1/+1 counter if
    /// a nonland was discarded.
    Connive,
    /// Empower Jace (CR 701.71): create or grow a Jace planeswalker
    /// token.
    EmpowerJace,
    /// A keyword not listed above.
    Other(String),
}

/// A typed argument attached to a keyword name.
#[derive(Debug, Clone)]
pub enum KeywordArgument {
    /// Object class named by affinity, including unsupported scopes.
    AffinityFor(String),
    /// A numeric value ("Mobilize 2" → 2).
    Number(u32),
    /// A mana cost ("Offspring {1}{G}", "Plot {2}{R}").
    Mana(Cost),
    /// A life payment ("Cycling—Pay 2 life" → 2).
    Life(u32),
    /// A basic land type named by a landcycling keyword
    /// ("Forestcycling" → Forest, "Plainscycling" → Plains).
    BasicLandType(BasicLandType),
    /// Number of other graveyard cards required by an Escape cost.
    EscapeCards(u32),
    /// Unsupported payment text retained so lowering cannot omit a requirement.
    #[allow(dead_code)]
    // The AST keeps this text for compiler evidence, not runtime execution.
    Other(String),
}

/// The ability class recorded for an unsupported statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedAbilityKind {
    /// An activated ability whose effect is not modeled.
    Activated,
    /// A triggered ability whose effect is not modeled.
    Triggered,
    /// Text not recognized as an ability form.
    Unknown,
}

/// An Oracle statement kept without assigning it an unsupported game effect.
#[derive(Debug, Clone)]
#[allow(dead_code)] // The AST retains unsupported text without executing it.
pub struct OracleUnsupported {
    /// Best-effort syntax class.
    pub kind: UnsupportedAbilityKind,
    /// Original Oracle text.
    pub source: String,
}
