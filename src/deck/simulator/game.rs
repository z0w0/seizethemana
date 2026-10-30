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
//!   - A physical card keeps its index across zones. Graveyard-entry
//!     generations prevent old targets and triggers from following a card
//!     that leaves and re-enters the graveyard.
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

use super::model::{
    CardIdx, Cost, Counters, KeywordSet, SimAbility, SimAbilityCondition, SimDeck, SimEffect,
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
    /// Creature that cannot attack or pay its own tap/untap-symbol costs without
    /// haste. Written tap costs, such as crew, can still use it (CR 302.6).
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
    /// The permanent uses its land face, independent of face-down state.
    pub land_face: bool,
    /// Power and toughness gained from supported prowess triggers this turn.
    pub prowess_bonus: u32,
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
    /// A plain 2/2 creature token (such as a Soldier or Warrior).
    #[default]
    Creature,
    /// A 1/1 flying Spirit created by Afterlife.
    Spirit,
    /// A 0/0 Army whose power comes from +1/+1 counters.
    Army,
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

/// Basic land types of exposed physical battlefield lands.
pub(in crate::deck::simulator) mod land_types;
/// Game observations kept separate from mutable battlefield state.
mod log;
pub(in crate::deck::simulator) use land_types::controlled_land_types;
pub use log::{
    GameCompletion, GameLog, GamePhase, IncompleteReason, PlayerLossReason, TurnMilestone,
    TurnObservation,
};

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
    /// X chosen before preparing this activation's mana payment.
    pub(super) paid_x: u32,
    /// Cards drawn when it resolves.
    pub(super) draws: u32,
    /// Creature sacrificed as part of the activation cost.
    pub(super) sacrifice_uid: Option<u32>,
    /// Creature chosen for a counter effect, when one is available.
    pub(super) target_uid: Option<u32>,
}

/// One turn's spendable mana pool.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ManaPool {
    /// Fixed simultaneous pips from sources like Jegantha, and single-color
    /// choice sources pinned to their color.
    pub(super) fixed: [u32; 5],
    /// Portion of fixed pips that cannot pay generic mana costs.
    pub(super) fixed_no_generic: [u32; 5],
    /// Mana that can pay one pip of any color, but not explicit colorless.
    pub(super) flexible: u32,
    /// Colorless-only sources.
    pub(super) colorless: u32,
    /// Unknown-color production that can pay only generic costs.
    pub(super) generic_credit: u32,
    /// Banked Treasure tokens available for payment, not floated mana.
    pub(super) treasures: u32,
    /// Produced mana with exact type choices and spend restrictions.
    pub(super) lots: Vec<super::game_mana::ManaLot>,
}

/// Mutable per-game state the effect helpers share: one battlefield, the
/// zones, and the zone-census maps (first turn each card reached a zone).
#[derive(Clone)]
pub(super) struct GameState {
    /// First terminal loss recorded at a supported rules boundary.
    pub(super) completion: Option<GameCompletion>,
    /// True after any attempted draw fails, even if a later effect adds cards.
    pub(super) failed_draw: bool,
    /// Current scheduled player turn.
    pub(super) current_turn: u32,
    /// Phase used to locate a terminal loss.
    pub(super) current_phase: GamePhase,
    /// Triggers captured while a spell, ability, or cost is in progress.
    pub(super) resolution: super::resolution::ResolutionState,
    /// Random stream reserved for committed library operations.
    pub(super) library_rng: rand_chacha::ChaCha8Rng,
    /// Shared synthetic opponent facts and resource budgets.
    pub(super) opponents: super::opponents::OpponentTable,
    /// Command-zone casts per commander, used for the additional two-mana tax.
    pub(super) commander_casts: Vec<u32>,
    pub(super) battlefield: Vec<Permanent>,
    pub(super) library: Vec<CardIdx>,
    pub(super) hand: Vec<CardIdx>,
    pub(super) seen: u32,
    pub(super) graveyard: Vec<CardIdx>,
    /// Entry generation for each physical card's current or last graveyard stay.
    pub(super) graveyard_generations: HashMap<CardIdx, u32>,
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
    /// First turn each card reached the hand, recorded when it arrives.
    pub(super) card_first_seen: HashMap<usize, u32>,
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
    /// Physical card and graveyard generation granted flashback until the next turn.
    pub(super) flashback_permissions: HashSet<(CardIdx, u32)>,
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
    /// Noncreature spells observed this turn. Reset before the next turn's actions.
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

/// Record the first turn a card enters the hand.
pub(super) fn record_card_seen(st: &mut GameState, idx: CardIdx, turn: u32) {
    st.card_first_seen.entry(idx.index()).or_insert(turn);
}

/// Identify a graveyard stay; seeded starting cards use generation zero.
pub(super) fn graveyard_generation(st: &GameState, index: CardIdx) -> u32 {
    st.graveyard_generations.get(&index).copied().unwrap_or(0)
}

/// Check that a physical card still has the captured graveyard identity.
pub(super) fn graveyard_entry_matches(st: &GameState, index: CardIdx, generation: u32) -> bool {
    graveyard_generation(st, index) == generation && st.graveyard.contains(&index)
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
    /// Spendable capacity, including banked Treasures and restricted mana.
    pub(super) fn total(&self) -> u32 {
        self.mana_total() + self.treasures
    }

    /// Produced mana and generic credit, excluding banked Treasure tokens.
    pub(super) fn mana_total(&self) -> u32 {
        self.fixed.iter().sum::<u32>()
            + self.flexible
            + self.colorless
            + self.generic_credit
            + self.lots.iter().map(|lot| lot.amount).sum::<u32>()
    }
}

/// Naive body power for stationing and crewing (the sim does not track
/// individual power; every body contributes this).
pub(super) const TOKEN_CREATURE_POWER: u32 = 2;

/// Naive body toughness for -1/-1 counter death checks on bodies with no
/// printed toughness (tokens are 2/2 under the flat body model).
pub(super) const TOKEN_CREATURE_TOUGHNESS: u32 = 2;

/// Resolve an object's immutable definition. Physical card and commander
/// references must belong to this game's compiled deck.
pub(super) fn card_of<'a>(deck: &'a SimDeck, perm: &Permanent) -> &'a super::model::SimCard {
    match perm.card {
        CardRef::Commander { slot } => deck
            .commanders
            .get(slot)
            .expect("commander object refers to its compiled command-zone card"),
        CardRef::Deck(idx) => deck
            .cards
            .get(idx.index())
            .expect("physical card index belongs to the compiled deck"),
        CardRef::Token(TokenKind::Creature) => token_creature_card(),
        CardRef::Token(TokenKind::Spirit) => token_spirit_card(),
        CardRef::Token(TokenKind::Army) => token_army_card(),
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
    let card = card_of(deck, perm);
    let mut set = if perm.land_face && card.is_mdfc_spell {
        KeywordSet::new()
    } else {
        card.printed_keywords
    };
    for granter in &st.battlefield {
        if granter.face_down || granter.land_face && card_of(deck, granter).is_mdfc_spell {
            continue;
        }
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

/// Credit a noncreature cast only to permanents that currently have prowess.
pub(super) fn record_noncreature_cast(deck: &SimDeck, st: &mut GameState) {
    st.prowess_casts += 1;
    for position in 0..st.battlefield.len() {
        if effective_keywords(deck, st, &st.battlefield[position])
            .contains(super::model::Keyword::Prowess)
        {
            st.battlefield[position].prowess_bonus += 1;
        }
    }
}

/// Evaluate a modeled intervening-if condition (CR 603.4) against the
/// live game state. Unsupported shapes are excluded during lowering.
pub(super) fn condition_met(
    deck: &SimDeck,
    st: &GameState,
    condition: &SimAbilityCondition,
) -> bool {
    match condition {
        SimAbilityCondition::ControlsLandTypes(required) => {
            land_types::controls_land_types(deck, &st.battlefield, *required)
        }
        SimAbilityCondition::Descend(n) => {
            st.graveyard
                .iter()
                .filter(|i| deck[**i].is_permanent)
                .count() as u32
                >= *n
        }
        SimAbilityCondition::Threshold => st.graveyard.len() >= 7,
        SimAbilityCondition::Raid => st.attacked_this_turn,
        SimAbilityCondition::Ferocious => st.battlefield.iter().any(|permanent| {
            is_creature_permanent(deck, permanent)
                && super::game_effects::current_creature_power_signed(
                    permanent,
                    deck,
                    &st.battlefield,
                ) >= 4
        }),
        SimAbilityCondition::Metalcraft => {
            st.battlefield
                .iter()
                .filter(|p| !p.face_down && card_of(deck, p).is_artifact)
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
    perm: &Permanent,
) -> impl Iterator<Item = &'a SimAbility> {
    let card = card_of(deck, perm);
    let face_down = perm.face_down;
    let land_face = perm.land_face;
    card.unlocked_abilities(perm.counters.charge)
        .filter(move |ability| {
            !face_down
                && (!card.is_mdfc_spell
                    || card.land_face_ability_ids.contains(&ability.id) == land_face)
        })
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

/// Test the causing object's types, subtypes, and token status before dispatch.
pub(super) fn event_filter_matches(
    filter: &super::model::SimEventFilter,
    card: &super::model::SimCard,
    token: bool,
) -> bool {
    filter.matches_card(card, token)
}

/// True when a permanent is a creature: its type says
/// creature, a station tier animated it, or it has living metal during
/// the player's turn (CR 702.161). Attached Reconfigure gear
/// (CR 702.151b) stops being a creature.
pub(super) fn is_creature_permanent(deck: &SimDeck, perm: &Permanent) -> bool {
    if perm.animated || perm.face_down || perm.crewed {
        return true;
    }
    let card = card_of(deck, perm);
    if perm.land_face && card.is_mdfc_spell {
        return false;
    }
    if !card.is_creature {
        // Living metal (CR 702.161): during your turn the Vehicle is an
        // artifact creature. Every simulated turn is the player's, so
        // the flag always applies.
        return card.keyword_abilities.living_metal;
    }
    !(perm.equipped && card.flags.equipment.is_some_and(|gear| gear.reconfigure))
}

/// Classify the physical face without treating a spell's land back face as a land.
pub(super) fn is_land_permanent(perm: &Permanent) -> bool {
    perm.land_face && !perm.face_down
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
        super::model::GrantTarget::CreaturesYouControl => is_creature_permanent(deck, perm),
        super::model::GrantTarget::LandsYouControl => is_land_permanent(perm),
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
        card_types: vec!["creature".into()],
        is_creature: true,
        is_permanent: true,
        printed_power: Some(2),
        printed_toughness: Some(2),
        ..super::model::SimCard::default()
    })
}

/// Lazily-built static data for the 1/1 flying Spirit created by Afterlife.
pub(super) fn token_spirit_card() -> &'static super::model::SimCard {
    use std::sync::OnceLock;
    static SPIRIT: OnceLock<super::model::SimCard> = OnceLock::new();
    SPIRIT.get_or_init(|| {
        let mut printed_keywords = super::model::KeywordSet::new();
        printed_keywords.insert(super::model::Keyword::Flying);
        super::model::SimCard {
            name: "Spirit Token".to_string(),
            card_types: vec!["creature".into()],
            subtypes: vec!["spirit".into()],
            is_creature: true,
            is_permanent: true,
            printed_power: Some(1),
            printed_toughness: Some(1),
            colors: [true, false, true, false, false],
            printed_keywords,
            ..super::model::SimCard::default()
        }
    })
}

/// Lazily-built static data for the 0/0 Army created by amass.
pub(super) fn token_army_card() -> &'static super::model::SimCard {
    use std::sync::OnceLock;
    static ARMY: OnceLock<super::model::SimCard> = OnceLock::new();
    ARMY.get_or_init(|| super::model::SimCard {
        name: "Army Token".to_string(),
        card_types: vec!["creature".into()],
        subtypes: vec!["army".into()],
        is_creature: true,
        is_permanent: true,
        printed_power: Some(0),
        printed_toughness: Some(0),
        colors: [false, false, true, false, false],
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
        card_types: vec!["legendary".into(), "planeswalker".into()],
        subtypes: vec!["jace".into()],
        is_planeswalker: true,
        is_permanent: true,
        is_legendary: true,
        colors: [false, true, false, false, false],
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
        land_face: sim.card_types.iter().any(|kind| kind == "land"),
        prowess_bonus: 0,
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
        land_face: false,
        prowess_bonus: 0,
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
        land_face: false,
        prowess_bonus: 0,
        saddled: false,
        army: false,
        jace_token: false,
        sacrifice_at_end: false,
        attacking_this_turn: false,
    }
}

/// Capture battlefield listeners before any effect can change the board.
pub(super) fn fire_triggers(deck: &SimDeck, st: &mut GameState, trigger: SimTrigger, turn: u32) {
    super::resolution::begin_resolution(st);
    let triggers = st
        .battlefield
        .iter()
        .flat_map(|source| {
            permanent_abilities(deck, source)
                .filter(|ability| ability.kind.is_triggered() && ability.trigger == trigger)
                .filter_map(|ability| {
                    super::resolution::prepare_trigger(deck, st, source, ability, turn)
                })
        })
        .collect::<Vec<_>>();
    for trigger in triggers {
        super::resolution::enqueue_trigger(st, trigger);
    }
    super::resolution::end_resolution(deck, st, None);
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
    let card = if perm.face_down {
        token_creature_card()
    } else {
        card_of(deck, perm)
    };
    let is_saga = card.is_saga;
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
    let has_chapter = card.saga.chapters.get(chosen_chapter).is_some();
    let triggers: Vec<_> = st
        .battlefield
        .iter()
        .filter(|source| !deferred || source.uid == perm_uid)
        .flat_map(|source| {
            permanent_abilities(deck, source)
                .filter(|ability| {
                    ability.kind.is_triggered() && ability.trigger == SimTrigger::Enters
                })
                .filter(|ability| {
                    trigger_subject_matches(ability.event_subject, source.uid, perm_uid)
                })
                .filter(|ability| {
                    if perm.land_face && card.is_mdfc_spell {
                        ability
                            .event_filter
                            .card_types
                            .iter()
                            .all(|kind| kind == "land")
                            && ability.event_filter.subtypes.is_empty()
                    } else {
                        event_filter_matches(
                            &ability.event_filter,
                            card,
                            matches!(perm.card, CardRef::Token(_)),
                        )
                    }
                })
                .filter_map(|ability| {
                    let mut trigger =
                        super::resolution::prepare_trigger(deck, st, source, ability, turn)?;
                    if !deferred
                        && source.uid == perm_uid
                        && source.card.deck_idx().is_some()
                        && ability
                            .effect_sequence()
                            .iter()
                            .any(|effect| matches!(effect, SimEffect::ExileThenReturnSource))
                    {
                        super::resolution::rearm_enter_blink(&mut trigger);
                    }
                    Some(trigger)
                })
        })
        .collect();
    super::resolution::begin_resolution(st);
    for trigger in triggers {
        super::resolution::enqueue_trigger(st, trigger);
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
        if has_chapter {
            let source = st.battlefield[pos].clone();
            super::resolution::queue_saga_chapter(st, &source, chosen_chapter, turn);
        }
    }
    super::resolution::end_resolution(deck, st, None);
}

/// Opening-hand size in every supported format.
pub(super) const OPENING_HAND: usize = 7;

/// Maximum hand size at end of turn.
pub(super) const HAND_LIMIT: usize = 7;

/// The turn loop lives in `game_run`; re-exported for callers.
#[cfg(test)]
pub use super::game_run::run_game;
