//! Typed syntax tree for the supported subset of Magic Oracle text.

use super::model::{Cost, SearchSpec, TapYield};

/// Parsed keyword abilities and Oracle statements for one card.
#[derive(Debug, Clone)]
pub struct OracleCard {
    /// Keywords supplied by Scryfall or written as standalone text lines.
    pub keywords: Vec<KeywordAbility>,
    /// Triggered, activated, static, spell, chapter, and unsupported nodes.
    pub abilities: Vec<OracleAbility>,
}

impl OracleCard {
    /// Return true when the card has a keyword with this parsed name.
    pub fn has_keyword(&self, expected: &KeywordName) -> bool {
        self.keywords
            .iter()
            .any(|keyword| &keyword.name == expected)
    }

    /// Return a numeric parameter attached to a keyword, when present.
    pub fn keyword_number(&self, expected: &KeywordName) -> Option<u32> {
        self.keywords
            .iter()
            .find(|keyword| &keyword.name == expected)?
            .arguments
            .first()
            .map(|argument| match argument {
                KeywordArgument::Number(value) => *value,
            })
    }

    /// Return parsed Saga chapters in Oracle order.
    pub fn saga_chapters(&self) -> impl Iterator<Item = &SagaChapter> {
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
    Activated(ActivatedAbility),
    /// An ability that triggers from a game event.
    Triggered(TriggeredAbility),
    /// A continuous rule that applies while a condition holds.
    Static(StaticAbility),
    /// A one-shot effect from casting or resolving a spell.
    Spell(SpellAbility),
    /// One or more Saga chapter abilities.
    SagaChapter(SagaChapter),
    /// Text that was kept but does not match a supported syntax shape.
    #[allow(dead_code)] // Keep unknown Oracle wording available to parser clients.
    Unsupported(UnsupportedAbility),
}

/// An activated ability with parsed costs, restrictions, and effects.
#[derive(Debug, Clone)]
pub struct ActivatedAbility {
    /// Individual payment instructions from the activation cost.
    pub costs: Vec<ActivationCost>,
    /// Parsed effects, including an explicit node for unsupported text.
    pub effects: Vec<OracleEffect>,
    /// Restrictions that limit when or how often the ability can be used.
    pub restrictions: Vec<AbilityRestriction>,
}

/// One payment component of an activated ability.
#[derive(Debug, Clone)]
#[allow(dead_code)] // The AST keeps costs that the runtime model cannot pay yet.
pub enum ActivationCost {
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
    /// Any permanent or card.
    Any,
}

/// A restriction on activating an ability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbilityRestriction {
    /// The activation is limited to once per turn.
    OncePerTurn,
    /// The activation is limited to a sorcery-speed window.
    SorcerySpeed,
    /// A condition that remains visible but is not evaluated by the sim.
    Condition(String),
}

/// A typed event that can cause a triggered ability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerEvent {
    /// A permanent enters the battlefield.
    Enters(ObjectSubject),
    /// A turn step begins.
    BeginningOfStep {
        /// Step named by the text.
        step: TurnStep,
        /// Player whose step begins.
        player: PlayerScope,
    },
    /// A permanent attacks.
    Attacks(ObjectSubject),
    /// A permanent deals combat damage to a player.
    CombatDamageToPlayer(ObjectSubject),
    /// A player casts a spell.
    CastsSpell {
        /// Whether this card's own spell was cast.
        this_spell: bool,
    },
    /// A permanent dies or is put into a graveyard.
    Dies(ObjectSubject),
    /// A land enters under a player's control.
    LandEnters(PlayerScope),
    /// A nonland permanent you control is tapped to activate a mana
    /// ability. Every supported wording taps one of your nonland
    /// permanents, so the variant carries no fields.
    TappedForMana,
    /// The event is kept as text because the event grammar is unsupported.
    Other(String),
}

/// Turn step named by a triggered ability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStep {
    /// Upkeep step.
    Upkeep,
    /// End step.
    EndStep,
    /// First main phase.
    FirstMainPhase,
    /// Another step not modeled by the turn loop.
    Other,
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
pub struct TriggeredAbility {
    /// Event that causes the ability to trigger.
    pub event: TriggerEvent,
    /// Effects of the triggered ability.
    pub effects: Vec<OracleEffect>,
    /// Whether the text limits this ability to once each turn.
    pub once_per_turn: bool,
    /// Intervening "if" clause text (CR 603.4), when the trigger
    /// sentence carries one between the event and the resolution. The
    /// sim lowers the text but cannot evaluate any shape yet; the
    /// runtime treats every condition as true.
    pub condition: Option<String>,
}

/// A static ability and its continuous effects.
#[derive(Debug, Clone)]
pub struct StaticAbility {
    /// Parsed continuous rules or explicit unsupported nodes.
    pub effects: Vec<StaticEffect>,
}

/// A parsed continuous effect.
#[derive(Debug, Clone)]
pub enum StaticEffect {
    /// Grant a keyword to a class of permanents.
    KeywordGrant {
        /// Objects that receive the keyword.
        target: StaticTarget,
        /// Granted keyword.
        keyword: KeywordAbility,
    },
    /// Grant a mana ability to a class of permanents.
    ManaGrant {
        /// Objects that receive the mana ability.
        target: StaticTarget,
        /// Parsed mana yield. Runtime data the pool code does not read yet;
        /// the target class is the consumed part.
        #[allow(dead_code)] // The runtime grant model uses the target class only.
        yield_: TapYield,
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
    AdditionalLandDrop,
    /// Win when this permanent reaches a stated counter threshold.
    WinThreshold(u32),
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
pub struct SpellAbility {
    /// Parsed resolution effects.
    pub effects: Vec<OracleEffect>,
}

/// One Saga chapter trigger, including combined chapter symbols.
#[derive(Debug, Clone)]
pub struct SagaChapter {
    /// Chapter numbers that share this effect.
    pub chapters: Vec<u32>,
    /// Effects that resolve for each listed chapter.
    pub effects: Vec<OracleEffect>,
}

/// A parsed effect, or an explicit node for text the effect parser cannot read.
#[derive(Debug, Clone)]
pub enum OracleEffect {
    /// Draw one or more cards.
    Draw(u32),
    /// Draw cards, then discard cards.
    Loot(u32),
    /// Draw cards and put a -1/-1 counter on a creature.
    DrawAndMinusCounter,
    /// Gain life.
    GainLife(u32),
    /// Search using parsed card constraints.
    Search(SearchSpec),
    /// Add mana.
    Mana(TapYield),
    /// Untap the source permanent.
    UntapSelf,
    /// Release mana stored on charge counters.
    ManaPerCounter(TapYield),
    /// Create creature or artifact tokens.
    Tokens(u32),
    /// Put counters on the source.
    Counters(u32),
    /// Make an additional land play available.
    ExtraLand,
    /// Exile and return a permanent.
    Blink,
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
    Wheel,
    /// Take an extra turn.
    ExtraTurn,
    /// Scry or surveil cards.
    Look {
        /// Number of cards examined.
        count: u32,
        /// Put the examined cards into the graveyard when true.
        surveil: bool,
    },
    /// Drain life from a player or players.
    Drain(u32),
    /// Win when a counter threshold is reached.
    WinThreshold(u32),
    /// Preserve an effect not supported by the simulator.
    #[allow(dead_code)] // Unsupported clauses remain in the syntax tree and stay inert.
    Unsupported(String),
}

/// A keyword ability read from Scryfall or Oracle text.
#[derive(Debug, Clone)]
pub struct KeywordAbility {
    /// Parsed keyword identifier, including names not yet modeled here.
    pub name: KeywordName,
    /// Numeric keyword parameters.
    pub arguments: Vec<KeywordArgument>,
}

/// A keyword name known to the parser or preserved as a future name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeywordName {
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
    /// A keyword not listed above.
    Other(String),
}

/// A numeric argument attached to a keyword name.
#[derive(Debug, Clone)]
pub enum KeywordArgument {
    /// A numeric value.
    Number(u32),
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
pub struct UnsupportedAbility {
    /// Best-effort syntax class.
    pub kind: UnsupportedAbilityKind,
    /// Original Oracle text.
    pub source: String,
}
