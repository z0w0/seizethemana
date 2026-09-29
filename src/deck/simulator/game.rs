//! One goldfish game: shuffles, mulligans, plays best-case turns. Pure
//! apart from the passed RNG: same deck + same seed = same game.
//!
//! State-representation contract:
//!   - Zones (`library`, `hand`, `graveyard`, `exile`, and the seen maps)
//!     hold indexes into the immutable `SimDeck.cards` table.
//!   - Per-instance state (tapped, counters, entry turn, spell data applied)
//!     lives only on the battlefield `Permanent` and resets on zone
//!     change (rule 122.2). The library/hand/graveyard entries are the
//!     card itself.
//!   - A card re-entering a zone is the same index; there is no
//!     incarnation counter. A milled, returned, and discarded card is
//!     one index throughout (the index-aliasing approximation).
//!
//! Turn pipeline (best-case agent):
//!   1 UNTAP     everything untaps; creature sickness clears
//!   2 UPKEEP    upkeep engines fire (per-turn draws)
//!   3 DRAW      draw 1
//!   4 LAND      play a land (verge gates, fetch searches, ETB triggers)
//!   5 CAST      saga chapters advance; cheapest castable spells
//!               (pip-aware); ETB triggers fire
//!   6 ACTIVATE  spend leftover mana on draw engines; each costs a tap
//!   7 TAP BUDGET remaining untapped creatures: mana only while casting
//!               still needs it, else station, else crew
//!   8 THRESHOLD station tiers unlock (permanent); crew reverts at end
//!   9 COMBAT    bodies attack; attack triggers fire
//!  10 END       Monarch draw; hand-limit discard

use super::game_effects::apply_effect_at;
use super::model::{
    CardIdx, Cost, Counters, KeywordSet, Role, SimAbility, SimAbilityCondition, SimDeck, SimEffect,
    SimTrigger,
};
use std::collections::{HashMap, HashSet};

/// One permanent on the battlefield.
#[derive(Debug, Clone)]
pub struct Permanent {
    /// Stable identity for the permanent, unique within one game.
    /// Engine and ETB bookkeeping key on this so removals shifting
    /// battlefield positions never alias another card.
    pub uid: u32,
    /// Which card this permanent is: a deck card, the cast commander,
    /// or a token body.
    pub card: CardRef,
    /// Tapped this turn (one tap per turn per permanent).
    pub tapped: bool,
    /// Summoning-sick creature (cannot tap, crew, or attack this turn).
    pub summoning_sick: bool,
    /// Counters on this permanent, split by kind (charge, +1/+1,
    /// -1/-1, loyalty, keyword).
    pub counters: Counters,
    /// True once the permanent is a creature by its station tier.
    pub animated: bool,
    /// True while crewed this turn (Vehicles; reverts at end of turn).
    pub crewed: bool,
    /// Turn the permanent entered the battlefield.
    pub entered_turn: usize,
    /// Saga chapters consumed.
    pub saga_step: u32,
    /// Once-per-turn abilities already fired this turn.
    pub fired: bool,
    /// Blink-shaped ETBs re-fire the host's Enters triggers once, next
    /// turn (Conjurer's Closet-style flickers).
    pub returned_trigger_pending: bool,
    /// True once this Equipment has paid an equip cost this game (the
    /// buff joins combat only while equipped).
    pub equipped: bool,
    /// Uid of the creature this Equipment suits up (equipped gear
    /// only). The buff joins that host's attack alone; a uid survives
    /// battlefield shifts that would stale an index.
    pub equip_host: Option<u32>,
    /// True while the permanent is face down (morph, disguise, or
    /// manifest): a 2/2 body with no name, text, or abilities until it
    /// is turned face up (CR 708.2).
    pub face_down: bool,
    /// True while the permanent is saddled this turn (CR 702.171;
    /// reverts at the turn boundary like crew).
    pub saddled: bool,
    /// True for an Army permanent created or grown by amass (CR 701.47).
    /// Army attack power comes from its +1/+1 counters.
    pub army: bool,
    /// True for a Jace planeswalker token created by "empower Jace"
    /// (CR 701.71). The token's loyalty abilities live on the synthetic
    /// Jace token card.
    pub jace_token: bool,
    /// True while the permanent is marked for sacrifice at the next end
    /// step (mobilize's Warrior tokens, CR 702.181a).
    pub sacrifice_at_end: bool,
    /// True while the permanent entered attacking this turn (mobilize
    /// tokens, CR 702.181a: "tapped and attacking"). Combat counts it
    /// as an attacker even though it entered after attackers were
    /// declared.
    pub attacking_this_turn: bool,
}

/// Identifies deck cards, commanders, and synthetic tokens in a game zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardRef {
    /// A card in the main deck.
    Deck(CardIdx),
    /// A cast commander, `slot` indexing `SimDeck.commanders`.
    Commander {
        /// Index into `SimDeck.commanders`.
        slot: usize,
    },
    /// A created token. The kind names which synthetic card defines its
    /// abilities (plain 2/2 creature vs. the empower-Jace planeswalker).
    Token(TokenKind),
}

/// The kind of a created token, selecting its synthetic card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenKind {
    /// A plain 2/2 creature token (Soldiers, Warriors, Spirits).
    #[default]
    Creature,
    /// The empower-Jace planeswalker token (CR 701.71).
    Jace,
}

impl CardRef {
    /// The deck index, when the permanent is a deck card.
    pub fn deck_idx(self) -> Option<CardIdx> {
        match self {
            Self::Deck(idx) => Some(idx),
            Self::Commander { .. } | Self::Token(_) => None,
        }
    }
}

/// One played game's record.
#[derive(Debug, Clone)]
pub struct GameLog {
    /// Land drops made each turn (0-3: extra-turn replays and
    /// additional-land boards can push a single turn index past 1).
    pub land_drops: Vec<u8>,
    /// Mana available at the first main phase each turn.
    pub mana_available: Vec<f64>,
    /// Mana spent on casts each turn.
    pub mana_spent: Vec<f64>,
    /// Cumulative cards seen (drawn) by end of each turn.
    pub cards_seen: Vec<u32>,
    /// First turn the commander was castable; None when never.
    pub commander_castable: Option<u32>,
    /// First turn the board could pay each card's cost (draw-agnostic).
    pub mana_ready: Vec<Option<u32>>,
    /// First turn each role was seen in hand.
    pub first_seen: HashMap<Role, u32>,
    /// First turn a creature was seen in hand.
    pub first_creature: Option<u32>,
    /// Colors a cast was blocked for (had total mana, missing pips).
    pub blocked_colors: [bool; 5],
    /// Opening-hand land count.
    pub opener_lands: u8,
    /// True when the opening hand was redrawn.
    pub mulliganed: bool,
    /// Sum of land drops made in turns 1-4 (the screw metric's input).
    pub lands_by_4: u8,
    /// Lands seen (hand + battlefield + graveyard) by end of turn 4:
    /// the flood metric's measured value. The graveyard counts because
    /// the hypergeometric expectation scans everything seen, and a
    /// discarded land still flooded its draw. 11 is the nominal card
    /// window that expectation compares against (opener + 4 draws);
    /// draw engines widen the window, which `cards_seen_by_4` records.
    pub lands_seen_by_turn_4: u32,
    /// Cards seen by end of turn 4: the flood window's actual size.
    /// Draw engines widen it beyond the nominal 11, and the flood
    /// expectation must use this count to stay comparable.
    pub cards_seen_by_4: u32,
    /// First turn the commander spacecraft was animated (station online).
    pub station_online: Option<u32>,
    /// First turn the companion was fetched to hand (CR 702.139a).
    pub companion_online: Option<u32>,
    /// Creature permanents (including animated vehicles) per turn.
    pub creatures: Vec<u32>,
    /// Repeatable card sources available each turn.
    pub repeatable_sources_online: Vec<u32>,
    /// (card index, color index) pairs pip-blocked this game. Repeats
    /// allowed: one row per blocked cast attempt.
    pub pip_blocks: Vec<(usize, usize)>,
    /// Graveyard size at the end of each turn (milled + discarded cards).
    pub graveyard_size: Vec<u32>,
    /// First turn each card index was seen in hand (combo assembly).
    pub card_first_seen: HashMap<usize, u32>,
    /// First turn each card index reached the battlefield (cast, cheat-in,
    /// blink return, graveyard return) — combo assembly for battlefield
    /// pieces.
    pub card_first_battlefield: HashMap<usize, u32>,
    /// First turn each card index reached the graveyard (mill, discard,
    /// sacrifice) — combo assembly for graveyard pieces.
    pub card_first_graveyard: HashMap<usize, u32>,
    /// Total attacking power on the board at the combat phase of each
    /// turn (buffs, equipment, double strike included).
    pub attack_power: Vec<u32>,
    /// Number of graveyard casts that paid a supported flashback or escape cost.
    pub replay_casts: u32,
    /// Life spent on costs and activations during this game.
    pub life_paid: u32,
    /// Life gained during this game (lifelink, gain-life effects).
    pub life_gained: u32,
    /// Cards drawn by paying life or losing life to a draw effect.
    pub life_funded_draws: u32,
    /// Attacking bodies each turn (denominator for the evasion census).
    pub attackers: Vec<u32>,
    /// Attacking bodies with evasion (trample/flying/menace) each turn.
    pub evasive: Vec<u32>,
    /// Library size at the end of each turn (deck-out proximity).
    pub library_size: Vec<u32>,
    /// Cards self-milled (own-library mill + surveil) by end of turn.
    pub self_milled: Vec<u32>,
    /// Cards milled toward opponents ("target player/opponent mills")
    /// by end of turn — deck-out pressure on the table.
    pub opp_milled: Vec<u32>,
    /// Cards evaluated (drawn + milled + scried/surveiled) per turn,
    /// cumulative fraction of the library.
    pub awareness: Vec<f64>,
    /// Opponent life lost to life-loss effects by end of each turn.
    pub opponent_life_loss: Vec<u32>,
    /// Player damage (combat + combat-damage triggers) per turn, cumulative.
    pub player_damage: Vec<u32>,
    /// Extra turns taken by end of each turn.
    pub extra_turns: Vec<u32>,
    /// Counts and events for the key simulator milestones on each turn.
    pub milestones_by_turn: Vec<TurnMilestone>,
    /// First turn a win-threshold engine could fire (enough counters);
    /// None when the deck has no such engine or never reached it.
    pub win_threshold_turn: Option<u32>,
    /// First turn a planeswalker ultimate became affordable (loyalty
    /// >= the ultimate's cost); None when none exists or never reached.
    pub ultimate_online: Option<u32>,
    /// Ready-to-fire interaction (in hand + affordable) per turn.
    pub interaction_ready: Vec<bool>,
    /// Spare mana while interaction was ready, per turn.
    pub interaction_mana_held: Vec<f64>,
    /// True when a zero-cost mana activation looped past the cap
    /// (Basalt Monolith-class infinite engine).
    pub infinite_mana_suspected: bool,
}

/// Per-turn events that show whether the deck's engine actions resolved.
#[derive(Debug, Clone, Default)]
pub struct TurnMilestone {
    /// Permanent cards that entered from a cascade free cast.
    pub free_cast_permanents_entered: u32,
    /// Draws replaced by dredge.
    pub dredge_uses: u32,
    /// Successful flashback and escape casts.
    pub graveyard_casts: u32,
    /// Cards drawn through life-funded actions.
    pub life_funded_draws: u32,
    /// Whether a bounded positive-mana activation loop was reached.
    pub positive_mana_loop: bool,
}

/// One chosen activation in the spend-leftover-mana pass.
pub(super) struct Activation {
    /// Battlefield position of the source.
    pub(super) pos: usize,
    /// Stable uid of the source (survives battlefield shifts).
    pub(super) uid: u32,
    /// The ability that fired (cloned so re-lookup is exact).
    pub(super) ability: SimAbility,
    /// The mana cost actually owed after power-up discounts (CR
    /// 702.193b). Equal to `ability.cost` for ordinary activations.
    pub(super) ability_cost: Cost,
    /// Cards drawn when it resolves.
    pub(super) draws: u32,
    /// Creature sacrificed as part of the activation cost.
    pub(super) sacrifice_uid: Option<u32>,
    /// Creature chosen for a counter effect, when one is available.
    pub(super) target_uid: Option<u32>,
}

/// One turn's spendable mana pool.
#[derive(Debug, Clone, Default)]
pub(super) struct ManaPool {
    /// Fixed simultaneous pips from sources like Jegantha, and single-color
    /// choice sources pinned to their color.
    pub(super) fixed: [u32; 5],
    /// Portion of fixed pips that cannot pay generic mana costs.
    pub(super) fixed_no_generic: [u32; 5],
    /// Choice sources with more than one reachable color (each pays one
    /// mana of any tracked color).
    pub(super) flexible: u32,
    /// Colorless-only sources.
    pub(super) colorless: u32,
    /// Creature-only yield bucket (Secluded Courtyard-style lands).
    pub(super) creature_only: u32,
    /// Legendary-only yield bucket (Plaza of Heroes-style lands).
    pub(super) legendary_only: u32,
    /// Artifact-only yield bucket (Steelswarm Operator-style lands).
    pub(super) artifact_only: u32,
    /// Instant/sorcery-only yield bucket.
    pub(super) instant_sorcery_only: u32,
}

/// Mutable per-game state the effect helpers share: one battlefield, the
/// zones, and the zone-census maps (first turn each card reached a zone).
pub(super) struct GameState {
    pub(super) battlefield: Vec<Permanent>,
    pub(super) library: Vec<CardIdx>,
    pub(super) hand: Vec<CardIdx>,
    pub(super) seen: u32,
    pub(super) graveyard: Vec<CardIdx>,
    /// Cards removed from the game by costs or resolving effects.
    pub(super) exile: Vec<CardIdx>,
    /// True once the player has become the Monarch. From the next turn
    /// the Monarch draws one extra card at the beginning of their end
    /// step (CR 725.2).
    pub(super) is_monarch: bool,
    /// First turn each card reached the battlefield. Filled by every
    /// zone transition (cast, cheat-in, blink, reanimation); copied into
    /// the log at the end of the game.
    pub(super) battlefield_seen: HashMap<CardIdx, u32>,
    /// First turn each card reached the graveyard (mill, discard,
    /// sacrifice); copied into the log at the end of the game.
    pub(super) graveyard_seen: HashMap<CardIdx, u32>,
    /// Cards cast with a modeled alternate cost.
    /// Banked Treasure tokens: each is one any-color pip, sacrificed to
    /// use (the token itself is not tracked on the battlefield).
    pub(super) treasure_bank: u32,
    /// Running mill census, split by direction (self = graveyard fuel,
    /// opponent = deck-out pressure). Filled by Mill effects.
    pub(super) milled_self: u32,
    pub(super) milled_opp: u32,
    /// Running opponent life lost to life-loss effects.
    /// Damage dealt to opponents only; life the goldfish pays itself
    /// (additional costs) is counted in `life_paid` and never feeds the
    /// lethal census.
    pub(super) opponent_life_lost: u32,
    /// Direct damage dealt to players during the current turn.
    pub(super) damage_dealt_this_turn: u32,
    /// Life gained from resolved Oracle effects.
    pub(super) life_gained: u32,
    /// Graveyard instances granted flashback by the resolving spell.
    pub(super) flashback_permissions: HashSet<CardIdx>,
    /// Successful flashback and escape casts this game.
    pub(super) replay_casts: u32,
    /// Per-turn evidence for engine actions reported to the user.
    pub(super) milestones_by_turn: HashMap<u32, TurnMilestone>,
    /// Draws directly funded by life payments or life loss.
    pub(super) life_funded_draws: u32,
    /// Life the goldfish pays itself (additional cast costs, "pay N
    /// life" activations). Kept separate from opponent life lost so the lethal
    /// census measures damage dealt, not resources spent.
    pub(super) life_paid: u32,
    /// Current player life available to pay costs and resolve life effects.
    pub(super) life: i32,
    /// Cards evaluated (drawn + milled + scried/surveiled), cumulative.
    pub(super) awareness_cards: u32,
    /// Extra turns queued by effects this game.
    pub(super) extra_turns_queued: u32,
    /// Noncreature spells cast this turn (prowess power bumps and
    /// cast-count engines). Reset at the start of each turn.
    pub(super) prowess_casts: u32,
    /// True when a repeated zero-cost activation produced more mana than
    /// it cost this game (Basalt Monolith-class engine loop). A census
    /// flag, not a resolution: the sim caps the loop.
    pub(super) infinite_mana_suspected: bool,
    /// Monotone uid source for battlefield permanents.
    pub(super) next_uid: u32,
    /// Player-held counters (energy). Once per game, not per permanent.
    pub(super) player_counters: super::model::PlayerCounters,
    /// Activated abilities used this turn, keyed by permanent uid and
    /// stable ability id.
    pub(super) activated_this_turn: HashSet<(u32, usize)>,
    /// Once-per-game activations already used.
    pub(super) activated_once: HashSet<(u32, usize)>,
    /// Once-per-turn triggered abilities already fired.
    pub(super) triggered_this_turn: HashSet<(u32, usize)>,
    /// True once the companion has been fetched to hand (CR 702.139a):
    /// the {3} special action is once per game.
    pub(super) companion_fetched: bool,
    /// Times the Ring has tempted the player (CR 701.54c). The emblem's
    /// levels unlock at 2, 3, and 4 tempts.
    pub(super) ring_tempts: u32,
    /// One-shot extra land drops granted this turn ("You may play an
    /// additional land this turn"). Reset at the start of each turn.
    pub(super) extra_land_drops_this_turn: u32,
    /// Spells cast this turn, for Storm's copy count (CR 702.40a).
    pub(super) spells_cast_this_turn: u32,
    /// True once any creature attacked this turn (Raid conditions).
    pub(super) attacked_this_turn: bool,
    /// Plotted cards (CR 702.170): (card, turn it became plotted). A
    /// plotted card may be cast for free from any later turn.
    pub(super) plotted: Vec<(CardIdx, u32)>,
    /// The Ring-bearer designation (CR 701.54a): the chosen creature's
    /// uid. Re-chosen on every temptation; best case the strongest
    /// attacker.
    pub(super) ring_bearer: Option<u32>,
}

/// Return the milestone record for one turn, creating it when needed.
pub(super) fn milestone_for_turn(st: &mut GameState, turn: u32) -> &mut TurnMilestone {
    st.milestones_by_turn.entry(turn).or_default()
}

/// Reserve the next permanent uid.
pub(super) fn take_uid(st: &mut GameState) -> u32 {
    st.next_uid += 1;
    st.next_uid
}

impl ManaPool {
    /// Total mana available (a choice source still produces one mana);
    /// restricted yield pays only its cast class.
    pub(super) fn total(&self) -> u32 {
        self.fixed.iter().sum::<u32>()
            + self.flexible
            + self.colorless
            + self.creature_only
            + self.legendary_only
            + self.artifact_only
            + self.instant_sorcery_only
    }
}

/// Naive body power for stationing and crewing (the sim does not track
/// individual power; every body contributes this).
pub(super) const TOKEN_CREATURE_POWER: u32 = 2;

/// Naive body toughness for -1/-1 counter death checks on bodies with no
/// printed toughness (tokens are 2/2 under the flat body model).
pub(super) const TOKEN_CREATURE_TOUGHNESS: u32 = 2;

/// Static card data for a battlefield permanent. Commanders resolve
/// through `deck.commanders[slot]`; tokens are 2/2 bodies with no
/// abilities. A commander permanent with no matching commander entry
/// (empty commanders list) resolves as a token body instead of panicking.
pub(super) fn card_of<'a>(deck: &'a SimDeck, perm: &Permanent) -> &'a super::model::SimCard {
    match perm.card {
        CardRef::Commander { slot } => deck
            .commanders
            .get(slot)
            .unwrap_or_else(|| token_creature_card()),
        CardRef::Deck(idx) => deck
            .cards
            .get(idx.index())
            .unwrap_or_else(|| token_creature_card()),
        CardRef::Token(TokenKind::Creature) => token_creature_card(),
        CardRef::Token(TokenKind::Jace) => jace_token_card(),
    }
}

/// Every keyword a permanent has right now: printed keywords, static
/// keyword grants from the board, and keyword counters. Combat, mana,
/// and sickness paths read this instead of the card's baked flags, so
/// granted and counter keywords behave like printed ones.
pub(super) fn effective_keywords(deck: &SimDeck, st: &GameState, perm: &Permanent) -> KeywordSet {
    if perm.face_down {
        return KeywordSet::new();
    }
    let mut set = card_of(deck, perm).printed_keywords;
    for granter in &st.battlefield {
        for grant in &card_of(deck, granter).flags.keyword_grants {
            if keyword_grant_applies(deck, grant.target, perm, granter) {
                set.insert(grant.keyword);
            }
        }
    }
    for keyword in super::model::KEYWORD_TABLE
        .iter()
        .map(|(_, keyword)| *keyword)
    {
        if perm.counters.keyword.contains(keyword) {
            set.insert(keyword);
        }
    }
    set
}

/// Evaluate a modeled intervening-if condition (CR 603.4) against the
/// live game state. Unsupported shapes are excluded during lowering.
pub(super) fn condition_met(
    deck: &SimDeck,
    st: &GameState,
    condition: &SimAbilityCondition,
) -> bool {
    match condition {
        SimAbilityCondition::Descend(n) => {
            st.graveyard
                .iter()
                .filter(|i| deck[**i].is_permanent)
                .count() as u32
                >= *n
        }
        SimAbilityCondition::Threshold => st.graveyard.len() >= 7,
        SimAbilityCondition::Raid => st.attacked_this_turn,
        SimAbilityCondition::Ferocious => st.battlefield.iter().any(|p| {
            let power = card_of(deck, p)
                .printed_power
                .unwrap_or(TOKEN_CREATURE_POWER)
                + p.counters.plus1;
            power >= 4
        }),
        SimAbilityCondition::Metalcraft => {
            st.battlefield
                .iter()
                .filter(|p| card_of(deck, p).is_artifact)
                .count()
                >= 3
        }
    }
}

/// Abilities a battlefield permanent can use right now. A face-down
/// permanent has none (CR 708.2). Station striations are gated by the
/// permanent's charge counters (CR 721.2a): a `{N+}` ability does not
/// exist below N counters, so trigger paths must not fire it early.
pub(super) fn permanent_abilities<'a>(
    deck: &'a SimDeck,
    perm: &'a Permanent,
) -> Box<dyn Iterator<Item = &'a SimAbility> + 'a> {
    if perm.face_down {
        Box::new(std::iter::empty())
    } else {
        Box::new(card_of(deck, perm).unlocked_abilities(perm.counters.charge))
    }
}

/// Check whether an object's event matches a trigger's source relationship.
pub(super) fn trigger_subject_matches(
    subject: super::model::SimEventSubject,
    source_uid: u32,
    event_uid: u32,
) -> bool {
    match subject {
        super::model::SimEventSubject::This => source_uid == event_uid,
        super::model::SimEventSubject::Another => source_uid != event_uid,
        super::model::SimEventSubject::Any => true,
    }
}

/// True when a permanent is a creature: its type says
/// creature, a station tier animated it, or it has living metal during
/// the player's turn (CR 702.161). Attached Reconfigure gear
/// (CR 702.151b) stops being a creature.
pub(super) fn is_creature_permanent(deck: &SimDeck, perm: &Permanent) -> bool {
    if perm.animated || perm.face_down {
        return true;
    }
    let card = card_of(deck, perm);
    if !card.is_creature {
        // Living metal (CR 702.161): during your turn the Vehicle is an
        // artifact creature. Every simulated turn is the player's, so
        // the flag always applies.
        return card.keyword_abilities.living_metal;
    }
    !(perm.equipped && card.flags.equipment.is_some_and(|gear| gear.reconfigure))
}

/// True when a static keyword grant from `granter` reaches `perm`.
fn keyword_grant_applies(
    deck: &SimDeck,
    target: super::model::GrantTarget,
    perm: &Permanent,
    granter: &Permanent,
) -> bool {
    match target {
        // "This creature has flying" applies only while the source is
        // the granted permanent itself.
        super::model::GrantTarget::Source => granter.uid == perm.uid,
        super::model::GrantTarget::CreaturesYouControl => card_of(deck, perm).is_creature,
        super::model::GrantTarget::LandsYouControl => card_of(deck, perm).is_land,
        super::model::GrantTarget::PermanentsYouControl
        | super::model::GrantTarget::SpellsYouCast => true,
    }
}

/// Register a planeswalker's +1 token ability as a repeatable source.
///
/// A loyalty-gain activation that creates tokens is once-per-turn token
/// fuel (Liliana, Dreadhorde General class): it feeds sacrifice engines
/// and body counts every turn after the first activation. The engine is
/// zero-draw (no card draw) and keyed to the permanent's uid so it
/// drops out when the permanent leaves.
pub(super) fn register_loyalty_token_source(
    deck: &SimDeck,
    st: &GameState,
    pos: usize,
    repeatable_sources: &mut Vec<(u32, u32)>,
) {
    let Some(perm) = st.battlefield.get(pos) else {
        return;
    };
    let card = card_of(deck, perm);
    let creates_repeatable_tokens = card
        .unlocked_abilities(perm.counters.charge)
        .any(|ability| {
            ability
                .activation
                .as_ref()
                .is_some_and(|activation| activation.loyalty_change() > 0)
                && ability
                    .effect_sequence()
                    .first()
                    .is_some_and(|effect| matches!(effect, super::model::SimEffect::Tokens(_)))
        });
    if creates_repeatable_tokens {
        register_repeatable_source(repeatable_sources, perm.uid, 0);
    }
}

/// Register one battlefield permanent for repeatable-source accounting.
pub(super) fn register_repeatable_source(
    sources: &mut Vec<(u32, u32)>,
    uid: u32,
    draw_amount: u32,
) {
    if !sources.iter().any(|(source_uid, _)| *source_uid == uid) {
        sources.push((uid, draw_amount));
    }
}

/// Lazily-built static data for a 2/2 creature token (no abilities, no tap
/// yield). Built once, read-only.
pub(super) fn token_creature_card() -> &'static super::model::SimCard {
    use std::sync::OnceLock;
    static CREATURE: OnceLock<super::model::SimCard> = OnceLock::new();
    CREATURE.get_or_init(|| super::model::SimCard {
        name: "Token".to_string(),
        is_creature: true,
        ..super::model::SimCard::default()
    })
}

/// Lazily-built static data for the Jace planeswalker token created by
/// "empower Jace" (CR 701.71): a blue Jace planeswalker with 0 loyalty,
/// "[-1]: Surveil 1", and "[-3]: Draw a card". Built once, read-only.
pub(super) fn jace_token_card() -> &'static super::model::SimCard {
    use std::sync::OnceLock;
    static JACE: OnceLock<super::model::SimCard> = OnceLock::new();
    JACE.get_or_init(|| super::model::SimCard {
        name: "Jace".to_string(),
        striations: vec![super::model::SimStriation {
            at: 0,
            animate: false,
            abilities: vec![
                SimAbility {
                    kind: super::model::SimAbilityKind::Activated,
                    trigger: SimTrigger::Never,
                    effect: SimEffect::Scry(1),
                    activation: Some(super::model::SimActivation {
                        costs: vec![super::model::SimActivationCost::Loyalty(-1)],
                        restrictions: Vec::new(),
                    }),
                    ..SimAbility::default()
                },
                SimAbility {
                    kind: super::model::SimAbilityKind::Activated,
                    trigger: SimTrigger::Never,
                    effect: SimEffect::Draw(1),
                    activation: Some(super::model::SimActivation {
                        costs: vec![super::model::SimActivationCost::Loyalty(-3)],
                        restrictions: Vec::new(),
                    }),
                    ..SimAbility::default()
                },
            ],
        }],
        ..super::model::SimCard::default()
    })
}

/// A new battlefield permanent with a pre-reserved uid (call sites that
/// hold another borrow of the state reserve it via [`take_uid`] first).
pub(super) fn new_perm_with(
    uid: u32,
    deck: &SimDeck,
    card: CardIdx,
    turn: u32,
    tapped: bool,
) -> Permanent {
    let sim = &deck[card];
    Permanent {
        uid,
        card: CardRef::Deck(card),
        tapped,
        // Living-metal Vehicles (CR 702.161) are creatures during the
        // player's turn, so summoning sickness applies from entry.
        summoning_sick: (sim.is_creature || sim.keyword_abilities.living_metal)
            && !sim.flags.has_haste,
        counters: Counters {
            loyalty: sim.starting_loyalty.unwrap_or(0),
            ..sim.enter_counters.fixed()
        },
        animated: false,
        crewed: false,
        entered_turn: turn as usize,
        saga_step: 0,
        fired: false,
        returned_trigger_pending: false,
        equipped: false,
        equip_host: None,
        face_down: false,
        saddled: false,
        army: false,
        jace_token: false,
        sacrifice_at_end: false,
        attacking_this_turn: false,
    }
}

/// A fresh token permanent (2/2 body, sick the turn it enters).
pub(super) fn new_token_perm(uid: u32, turn: u32) -> Permanent {
    Permanent {
        uid,
        card: CardRef::Token(TokenKind::Creature),
        tapped: false,
        summoning_sick: true,
        counters: Counters::default(),
        animated: false,
        crewed: false,
        entered_turn: turn as usize,
        saga_step: 0,
        fired: false,
        returned_trigger_pending: false,
        equipped: false,
        equip_host: None,
        face_down: false,
        saddled: false,
        army: false,
        jace_token: false,
        sacrifice_at_end: false,
        attacking_this_turn: false,
    }
}

/// A commander permanent joining the battlefield from the command zone.
/// Creature commanders are summoning-sick like any other creature (CR
/// 302.6: the command zone grants no haste).
pub(super) fn new_commander_perm(
    uid: u32,
    slot: usize,
    loyalty: u32,
    turn: usize,
    is_creature: bool,
    has_haste: bool,
) -> Permanent {
    Permanent {
        uid,
        card: CardRef::Commander { slot },
        tapped: false,
        summoning_sick: is_creature && !has_haste,
        counters: Counters {
            loyalty,
            ..Counters::default()
        },
        animated: false,
        crewed: false,
        entered_turn: turn,
        saga_step: 0,
        fired: false,
        returned_trigger_pending: false,
        equipped: false,
        equip_host: None,
        face_down: false,
        saddled: false,
        army: false,
        jace_token: false,
        sacrifice_at_end: false,
        attacking_this_turn: false,
    }
}

/// Resolve battlefield triggers for one player-controlled event.
pub(super) fn fire_triggers(deck: &SimDeck, st: &mut GameState, trigger: SimTrigger, turn: u32) {
    let triggers = st
        .battlefield
        .iter()
        .flat_map(|source| {
            let source_uid = source.uid;
            let source_card = source.card.deck_idx();
            let mills_opponent = card_of(deck, source).mills_opponent;
            permanent_abilities(deck, source)
                .filter(|ability| ability.kind.is_triggered() && ability.trigger == trigger)
                .filter(|ability| {
                    ability
                        .condition
                        .is_none_or(|condition| condition_met(deck, st, &condition))
                })
                .map(move |ability| (source_uid, source_card, mills_opponent, ability.clone()))
        })
        .collect::<Vec<_>>();
    for (source_uid, source_card, mills_opponent, ability) in triggers {
        let ability_key = (source_uid, ability.id);
        if ability.once_per_turn && st.triggered_this_turn.contains(&ability_key) {
            continue;
        }
        if ability
            .condition
            .is_some_and(|condition| !condition_met(deck, st, &condition))
        {
            continue;
        }
        for effect in ability.effect_sequence() {
            apply_effect_at(deck, effect, st, turn, mills_opponent, source_card);
        }
        if ability.once_per_turn {
            st.triggered_this_turn.insert(ability_key);
        }
    }
}

/// Fire Enters triggers for the permanent at `pos`. A deferred firing is a
/// blink re-fire and never re-arms, so each entry re-fires at most once.
pub(super) fn fire_on_enter(
    deck: &SimDeck,
    st: &mut GameState,
    pos: usize,
    turn: u32,
    deferred: bool,
) {
    let Some(perm) = st.battlefield.get(pos) else {
        return;
    };
    let perm_uid = perm.uid;
    let perm_card = perm.card;
    let card = card_of(deck, perm);
    let is_saga = card.is_saga;
    let mill_opp = card.mills_opponent;
    // Read ahead (CR 702.155b): the controller chooses a starting
    // chapter and the Saga enters with that many lore counters; the
    // chosen chapter fires on entry. Best case: the first chapter whose
    // parsed effects do something, so a later chapter is not wasted.
    let chosen_chapter = if card.read_ahead {
        card.saga
            .chapters
            .iter()
            .position(|chapter| !chapter.is_empty())
            .unwrap_or(0)
    } else {
        0
    };
    let first_chapter = card
        .saga
        .chapters
        .get(chosen_chapter)
        .cloned()
        .unwrap_or_default();
    let etb_effects: Vec<PendingEnterTrigger> = st
        .battlefield
        .iter()
        .filter(|source| !deferred || source.uid == perm_uid)
        .flat_map(|source| {
            let source_card = card_of(deck, source);
            let triggered_this_turn = &st.triggered_this_turn;
            permanent_abilities(deck, source)
                .filter(|ability| {
                    ability.kind.is_triggered() && ability.trigger == SimTrigger::Enters
                })
                .filter(|ability| {
                    trigger_subject_matches(ability.event_subject, source.uid, perm_uid)
                })
                .map(move |ability| PendingEnterTrigger {
                    source_uid: source.uid,
                    source_card: source.card.deck_idx(),
                    mills_opponent: source_card.mills_opponent,
                    ability_id: ability.id,
                    once_per_turn: ability.once_per_turn,
                    fired: triggered_this_turn.contains(&(source.uid, ability.id)),
                    condition: ability.condition,
                    effects: ability.effect_sequence().to_vec(),
                })
        })
        .collect();
    let has_real_etb = etb_effects
        .iter()
        .any(|trigger| trigger.source_uid == perm_uid);
    let mut once_fired = std::collections::HashSet::new();
    for PendingEnterTrigger {
        source_uid,
        source_card,
        mills_opponent,
        ability_id,
        once_per_turn,
        fired,
        condition,
        effects,
    } in etb_effects
    {
        let ability_key = (source_uid, ability_id);
        if once_per_turn && (fired || once_fired.contains(&ability_key)) {
            continue;
        }
        // Intervening-if conditions are checked for the event and again
        // immediately before resolution. Earlier triggers may change state.
        if condition.is_some_and(|condition| !condition_met(deck, st, &condition)) {
            continue;
        }
        // Treasure banking reads the firing card: an ETB "create a
        // Treasure token" banks pips from the entering card.
        for effect in &effects {
            apply_effect_at(deck, effect, st, turn, mills_opponent, source_card);
        }
        // Blink-shaped ETBs ("exile … return it to the battlefield")
        // re-fire the host's Enters triggers once, next turn. The
        // deferred firing does not re-arm: one re-fire per entry. The
        // uid (not the card index) keys the re-arm: with two copies of
        // the blink card in play, the host copy re-arms itself, never a
        // sibling. Land-search and Monarch ETBs parse to other effects
        // and never arm the flag.
        if !deferred
            && effects
                .iter()
                .any(|effect| matches!(effect, SimEffect::ExileThenReturnSource))
            && has_real_etb
            && source_card.is_some()
            && let Some(p) = st.battlefield.iter_mut().find(|p| p.uid == source_uid)
        {
            p.returned_trigger_pending = true;
        }
        if once_per_turn {
            once_fired.insert(ability_key);
            st.triggered_this_turn.insert(ability_key);
        }
    }
    if !deferred && is_saga {
        if let Some(saga) = st
            .battlefield
            .iter_mut()
            .find(|permanent| permanent.uid == perm_uid)
        {
            // The Saga enters with the chosen chapter's lore counter;
            // the next precombat main phase advances from there
            // (CR 702.155b, 714.3c).
            saga.saga_step = (chosen_chapter + 1) as u32;
        }
        for effect in &first_chapter {
            if !matches!(effect, SimEffect::None) {
                apply_effect_at(deck, effect, st, turn, mill_opp, perm_card.deck_idx());
            }
        }
    }
}

/// Snapshot one enters-trigger before event effects change the battlefield.
struct PendingEnterTrigger {
    source_uid: u32,
    source_card: Option<CardIdx>,
    mills_opponent: bool,
    ability_id: usize,
    once_per_turn: bool,
    fired: bool,
    condition: Option<SimAbilityCondition>,
    effects: Vec<SimEffect>,
}

/// Opening-hand size in every supported format.
pub(super) const OPENING_HAND: usize = 7;

/// Maximum hand size at end of turn.
pub(super) const HAND_LIMIT: usize = 7;

/// The turn loop lives in `game_run`; re-exported for callers.
pub use super::game_run::run_game;
