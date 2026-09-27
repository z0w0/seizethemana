//!! One goldfish game: shuffles, mulligans, plays best-case turns. Pure
//! apart from the passed RNG: same deck + same seed = same game.
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
use super::model::{Ability, AbilityTiming, CardIdx, Effect, Restriction, Role, SimDeck, TapYield};
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
    pub sick: bool,
    /// Charge counters (station thresholds, counter engines).
    pub counters: u32,
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
    /// Blink-shaped ETBs re-fire the host's OnEnter triggers once, next
    /// turn (Conjurer's Closet-style flickers).
    pub blink_pending: bool,
    /// Current planeswalker loyalty (cast commanders and walkers).
    pub loyalty: u32,
    /// True once this Equipment has paid an equip cost this game (the
    /// buff joins combat only while equipped).
    pub equipped: bool,
    /// Uid of the creature this Equipment suits up (equipped gear
    /// only). The buff joins that host's attack alone; a uid survives
    /// battlefield shifts that would stale an index.
    pub equip_host: Option<u32>,
}

/// Which card a battlefield permanent is. Deck cards carry an index
/// into `SimDeck.cards`; the commander names its command-zone slot;
/// tokens have no card at all. The variant replaces the old sentinel
/// values (`usize::MAX` commander, `usize::MAX - 1` token).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardRef {
    /// A card in the main deck.
    Deck(CardIdx),
    /// A cast commander, `slot` indexing `SimDeck.commanders`.
    Commander {
        /// Index into `SimDeck.commanders`.
        slot: usize,
    },
    /// A created token (2/2 body, no abilities).
    Token,
}

impl CardRef {
    /// The deck index, when the permanent is a deck card.
    pub fn deck_idx(self) -> Option<CardIdx> {
        match self {
            Self::Deck(idx) => Some(idx),
            Self::Commander { .. } | Self::Token => None,
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
    pub lands_seen_by_11: u32,
    /// Cards seen by end of turn 4: the flood window's actual size.
    /// Draw engines widen it beyond the nominal 11, and the flood
    /// expectation must use this count to stay comparable.
    pub cards_seen_by_4: u32,
    /// First turn the commander spacecraft was animated (station online).
    pub station_online: Option<u32>,
    /// Bodies (creatures, crewed vehicles, animated spacecraft) per turn.
    pub bodies: Vec<u32>,
    /// Repeatable engines online per turn.
    pub engines_online: Vec<u32>,
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
    /// Card indexes cast with a modeled alternate cost.
    #[cfg(test)]
    pub alternate_casts: Vec<usize>,
    /// Total attacking power on the board at the combat phase of each
    /// turn (buffs, equipment, double strike included).
    pub attack_power: Vec<u32>,
    /// Number of graveyard casts that paid a supported flashback or escape cost.
    pub replay_casts: u32,
    /// Life spent on costs and activations during this game.
    pub life_paid: u32,
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
    /// Life drained (burn, drain engines) by end of each turn.
    pub drain_total: Vec<u32>,
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
    pub(super) ability: Ability,
    /// Total activation cost.
    pub(super) cost: u32,
    /// Cards drawn when it resolves.
    pub(super) draws: u32,
    /// Filtered library search resolved by the activation.
    pub(super) search: Option<super::model::SearchSpec>,
    /// Mana produced (mana activations feed the pool).
    pub(super) mana_yield: Option<TapYield>,
    /// Charge counters added to the host (counter engines).
    pub(super) counters: u32,
    /// Life drained when it resolves (drain activations).
    pub(super) drain: u32,
    /// Creature sacrificed as part of the activation cost.
    pub(super) sacrifice_uid: Option<u32>,
    /// Creature chosen for a counter effect, when one is available.
    pub(super) target_uid: Option<u32>,
}

/// One turn's spendable mana pool.
#[derive(Debug, Clone, Default)]
pub(super) struct Pool {
    /// Fixed simultaneous pips from sources like Jegantha, and single-color
    /// choice sources pinned to their color.
    pub(super) fixed: [u32; 5],
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
    #[cfg(test)]
    pub(super) alternate_casts: Vec<CardIdx>,
    /// Banked Treasure tokens: each is one any-color pip, sacrificed to
    /// use (the token itself is not tracked on the battlefield).
    pub(super) treasure_bank: u32,
    /// Running mill census, split by direction (self = graveyard fuel,
    /// opponent = deck-out pressure). Filled by Mill effects.
    pub(super) milled_self: u32,
    pub(super) milled_opp: u32,
    /// Running life drained (burn, drain engines, combat-damage drains).
    /// Damage dealt to opponents only; life the goldfish pays itself
    /// (additional costs) is counted in `life_paid` and never feeds the
    /// lethal census.
    pub(super) drained: u32,
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
    /// life" activations). Kept separate from `drained` so the lethal
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

impl Pool {
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

    /// Mana reachable for a cast of the given restricted class: the
    /// general pool plus the class's own bucket. A restricted bucket
    /// never pays another cast class.
    pub(super) fn usable_for(&self, restriction: Restriction) -> u32 {
        let general = self.fixed.iter().sum::<u32>() + self.flexible + self.colorless;
        general
            + match restriction {
                Restriction::Creature => self.creature_only,
                Restriction::Legendary => self.legendary_only,
                Restriction::Artifact => self.artifact_only,
                Restriction::InstantSorcery => self.instant_sorcery_only,
            }
    }
}

/// Fallback target types for fetch lands whose Oracle text omits its target
/// pair. Oracle text remains the source of truth when it names the types.
pub(super) fn fetch_target_pair(name: &str) -> &'static [&'static str] {
    match name {
        "Flooded Strand" => &["Plains", "Island"],
        "Polluted Delta" => &["Island", "Swamp"],
        "Windswept Heath" => &["Plains", "Forest"],
        "Wooded Foothills" => &["Mountain", "Forest"],
        "Scalding Tarn" => &["Island", "Mountain"],
        "Arid Mesa" => &["Plains", "Mountain"],
        "Marsh Flats" => &["Plains", "Swamp"],
        "Misty Rainforest" => &["Island", "Forest"],
        "Bloodstained Mire" => &["Swamp", "Mountain"],
        "Verdant Catacombs" => &["Swamp", "Forest"],
        "Fabled Passage"
        | "Prismatic Vista"
        | "Terramorphic Expanse"
        | "Evolving Wilds"
        | "Escape Tunnel" => &["Plains", "Island", "Swamp", "Mountain", "Forest"],
        _ => &[],
    }
}

/// Naive body power for stationing and crewing (the sim does not track
/// individual power; every body contributes this).
pub(super) const BODY_POWER: u32 = 2;

/// Static card data for a battlefield permanent. Commanders resolve
/// through `deck.commanders[slot]`; tokens are 2/2 bodies with no
/// abilities. A commander permanent with no matching commander entry
/// (empty commanders list) resolves as a token body instead of panicking.
pub(super) fn card_of<'a>(deck: &'a SimDeck, perm: &Permanent) -> &'a super::model::SimCard {
    match perm.card {
        CardRef::Commander { slot } => deck
            .commanders
            .get(slot)
            .unwrap_or_else(|| token_body_card()),
        CardRef::Deck(idx) => deck
            .cards
            .get(idx.index())
            .unwrap_or_else(|| token_body_card()),
        CardRef::Token => token_body_card(),
    }
}

/// Register a planeswalker's +1 token ability as a repeatable engine.
///
/// A loyalty-gain activation that creates tokens is once-per-turn token
/// fuel (Liliana, Dreadhorde General class): it feeds sacrifice engines
/// and body counts every turn after the first activation. The engine is
/// zero-draw (no card draw) and keyed to the permanent's uid so it
/// drops out when the permanent leaves.
pub(super) fn register_loyalty_token_engines(
    deck: &SimDeck,
    st: &GameState,
    pos: usize,
    engines: &mut Vec<(u32, u32)>,
) {
    let Some(perm) = st.battlefield.get(pos) else {
        return;
    };
    if engines.iter().any(|(u, _)| *u == perm.uid) {
        return;
    }
    let card = card_of(deck, perm);
    let is_token_engine = card
        .abilities()
        .any(|a| a.loyalty_gain > 0 && matches!(a.effect, super::model::Effect::Tokens(_)));
    if is_token_engine {
        engines.push((perm.uid, 0));
    }
}

/// Lazily-built static data for a 2/2 token body (no abilities, no tap
/// yield). Built once, read-only.
pub(super) fn token_body_card() -> &'static super::model::SimCard {
    use std::sync::OnceLock;
    static BODY: OnceLock<super::model::SimCard> = OnceLock::new();
    BODY.get_or_init(|| super::model::SimCard {
        name: "Token".to_string(),
        is_creature: true,
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
        sick: sim.is_creature && !sim.flags.has_haste,
        counters: if sim.enter_counters == super::parse_land::X_ENTRY_COUNTERS {
            0
        } else {
            sim.enter_counters
        },
        animated: false,
        crewed: false,
        entered_turn: turn as usize,
        saga_step: 0,
        fired: false,
        blink_pending: false,
        loyalty: sim.starting_loyalty.unwrap_or(0),
        equipped: false,
        equip_host: None,
    }
}

/// A fresh token permanent (2/2 body, sick the turn it enters).
pub(super) fn new_token_perm(uid: u32, turn: u32) -> Permanent {
    Permanent {
        uid,
        card: CardRef::Token,
        tapped: false,
        sick: true,
        counters: 0,
        animated: false,
        crewed: false,
        entered_turn: turn as usize,
        saga_step: 0,
        fired: false,
        blink_pending: false,
        loyalty: 0,
        equipped: false,
        equip_host: None,
    }
}

/// A commander permanent joining the battlefield from the command zone.
pub(super) fn new_commander_perm(uid: u32, slot: usize, loyalty: u32, turn: usize) -> Permanent {
    Permanent {
        uid,
        card: CardRef::Commander { slot },
        tapped: false,
        sick: false,
        counters: 0,
        animated: false,
        crewed: false,
        entered_turn: turn,
        saga_step: 0,
        fired: false,
        blink_pending: false,
        loyalty,
        equipped: false,
        equip_host: None,
    }
}

/// Resolve battlefield triggers for one player-controlled event.
pub(super) fn fire_triggers(deck: &SimDeck, st: &mut GameState, trigger: AbilityTiming, turn: u32) {
    let effects = st
        .battlefield
        .iter()
        .flat_map(|permanent| {
            let card = card_of(deck, permanent);
            card.abilities()
                .filter(|ability| ability.trigger == trigger)
                .map(move |ability| (permanent.card, card.mills_opponent, ability.effect.clone()))
        })
        .collect::<Vec<_>>();
    for (source, mills_opponent, effect) in effects {
        apply_effect_at(deck, &effect, st, turn, mills_opponent, source.deck_idx());
    }
}

/// Fire OnEnter triggers for the permanent at `pos`. A deferred firing is a
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
    let is_saga = card_of(deck, perm).is_saga;
    let first_chapter = card_of(deck, perm).saga.chapters.first().cloned();
    let has_real_etb = card_of(deck, perm)
        .abilities()
        .any(|a| a.trigger == AbilityTiming::OnEnter);
    let mill_opp = card_of(deck, perm).mills_opponent;
    let etb_effects: Vec<Effect> = card_of(deck, perm)
        .abilities()
        .filter(|a| a.trigger == AbilityTiming::OnEnter)
        .map(|a| a.effect.clone())
        .collect();
    for effect in &etb_effects {
        apply_effect_at(deck, effect, st, turn, mill_opp, None);
        // Blink-shaped ETBs ("exile … return it to the battlefield")
        // re-fire the host's OnEnter triggers once, next turn. The
        // deferred firing does not re-arm: one re-fire per entry. The
        // uid (not the card index) keys the re-arm: with two copies of
        // the blink card in play, the host copy re-arms itself, never a
        // sibling. Land-search and Monarch ETBs parse to other effects
        // and never arm the flag.
        if !deferred
            && matches!(effect, Effect::Blink)
            && has_real_etb
            && perm_card.deck_idx().is_some()
            && let Some(p) = st.battlefield.iter_mut().find(|p| p.uid == perm_uid)
        {
            p.blink_pending = true;
        }
    }
    if !deferred && is_saga {
        if let Some(saga) = st
            .battlefield
            .iter_mut()
            .find(|permanent| permanent.uid == perm_uid)
        {
            saga.saga_step = 1;
        }
        if let Some(chapter) = first_chapter
            && !matches!(chapter, Effect::None)
        {
            apply_effect_at(deck, &chapter, st, turn, mill_opp, None);
        }
    }
}

/// Opening-hand size in every supported format.
pub(super) const OPENING_HAND: usize = 7;

/// Maximum hand size at end of turn.
pub(super) const HAND_LIMIT: usize = 7;

/// The turn loop lives in `game_run`; re-exported for callers.
pub use super::game_run::run_game;
