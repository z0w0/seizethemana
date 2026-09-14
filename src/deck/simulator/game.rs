// One goldfish game: shuffles, mulligans, plays best-case turns. Pure
// apart from the passed RNG: same deck + same seed = same game.
//
// Turn pipeline (best-case agent):
//   1 UNTAP     everything untaps; creature sickness clears
//   2 UPKEEP    upkeep engines and saga chapters fire (per-turn draws)
//   3 DRAW      draw 1
//   4 LAND      play a land (verge gates, fetch searches, ETB triggers)
//   5 CAST      cheapest castable spells (pip-aware); ETB triggers fire
//   6 ACTIVATE  spend leftover mana on draw engines; each costs a tap
//   7 TAP BUDGET remaining untapped creatures: mana only while casting
//               still needs it, else station, else crew
//   8 THRESHOLD station tiers unlock (permanent); crew reverts at end
//   9 COMBAT    bodies attack; attack triggers fire
//  10 END       hand-limit discard

use super::game_effects::apply_effect;
use super::model::{Effect, Role, SimDeck, TapYield, Trigger};
use std::collections::HashMap;

/// One permanent on the battlefield.
#[derive(Debug, Clone)]
pub struct InPlay {
    /// Index into `SimDeck.cards` (`usize::MAX` for the cast commander).
    pub card: usize,
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
    /// True when this is the cast commander on the battlefield.
    pub is_commander: bool,
    /// Commander slot (index into `SimDeck.commanders`); 0 otherwise.
    pub commander_slot: usize,
}

/// One played game's record.
#[derive(Debug, Clone)]
pub struct GameLog {
    /// Land drops made each turn (0/1).
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
    /// Lands seen by turn 4 (for the screw/flood buckets).
    pub lands_by_4: u8,
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
}

/// One chosen activation in the spend-leftover-mana pass.
pub(super) struct Activation {
    /// Battlefield position of the source.
    pub(super) pos: usize,
    /// Total activation cost.
    pub(super) cost: u32,
    /// Cards drawn when it resolves.
    pub(super) draws: u32,
    /// Mana produced (mana activations feed the pool).
    pub(super) mana_yield: Option<TapYield>,
    /// Charge counters added to the host (counter engines).
    pub(super) counters: u32,
    /// Bodies the activation sacrifices (aristocrat outlets).
    pub(super) sacrifice_bodies: u32,
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
}

/// Mutable per-game state the effect helpers share: one battlefield, the
/// zones, and the zone-census maps (first turn each card reached a zone).
pub(super) struct GameState {
    pub(super) battlefield: Vec<InPlay>,
    pub(super) library: Vec<usize>,
    pub(super) hand: Vec<usize>,
    pub(super) seen: u32,
    pub(super) graveyard: Vec<usize>,
    /// First turn each card index reached the battlefield. Filled by every
    /// zone transition (cast, cheat-in, blink, reanimation); copied into
    /// the log at the end of the game.
    pub(super) battlefield_seen: HashMap<usize, u32>,
    /// First turn each card index reached the graveyard (mill, discard,
    /// sacrifice); copied into the log at the end of the game.
    pub(super) graveyard_seen: HashMap<usize, u32>,
}

impl Pool {
    /// Total mana available (a choice source still produces one mana);
    /// creature-only yield pays creature casts only.
    pub(super) fn total(&self) -> u32 {
        self.fixed.iter().sum::<u32>() + self.flexible + self.colorless + self.creature_only
    }
}

/// Fold a tap yield into the pool. Multi-color and any-color choice
/// sources become flexible; single-color choice sources pin to their
/// color as a fixed pip; fixed sets stay fixed; `{C}` stays colorless.
/// Alternative-mode sources (several tap abilities on one permanent)
/// yield exactly one mana: one choice/any color or one colorless.
/// Land families the sim recognizes as fetches ("search … for a … land").
pub(super) fn fetches_land_text(card: &super::model::SimCard) -> bool {
    let name = card.name.to_ascii_lowercase();
    [
        "flooded strand",
        "polluted delta",
        "windswept heath",
        "wooded foothills",
        "fabled passage",
        "scalding tarn",
        "arid mesa",
        "marsh flats",
        "misty rainforest",
        "bloodstained mire",
        "verdant catacombs",
        "prismatic vista",
        "terramorphic expanse",
        "escape tunnel",
    ]
    .iter()
    .any(|f| name.starts_with(f))
}

/// Basic land types a land name implies (verge gates). Match keys off
/// known mana-base names because the sim has no type data; fetches count
/// for their full target pair.
pub(super) fn land_types(name: &str) -> &'static [&'static str] {
    match name {
        "Plains" => &["Plains"],
        "Island" => &["Island"],
        "Swamp" => &["Swamp"],
        "Mountain" => &["Mountain"],
        "Forest" => &["Forest"],
        "Hallowed Fountain" => &["Plains", "Island"],
        "Temple Garden" => &["Plains", "Forest"],
        "Godless Shrine" => &["Plains", "Swamp"],
        "Sacred Foundry" => &["Plains", "Mountain"],
        "Breeding Pool" => &["Forest", "Island"],
        "Watery Grave" => &["Island", "Swamp"],
        "Steam Vents" => &["Island", "Mountain"],
        "Overgrown Tomb" => &["Swamp", "Forest"],
        "Stomping Ground" => &["Mountain", "Forest"],
        "Blood Crypt" => &["Swamp", "Mountain"],
        // Fetches and generic search lands satisfy their whole target set.
        "Flooded Strand"
        | "Polluted Delta"
        | "Windswept Heath"
        | "Wooded Foothills"
        | "Scalding Tarn"
        | "Arid Mesa"
        | "Marsh Flats"
        | "Misty Rainforest"
        | "Bloodstained Mire"
        | "Verdant Catacombs"
        | "Fabled Passage"
        | "Terramorphic Expanse"
        | "Escape Tunnel" => &["Plains", "Island", "Swamp", "Mountain", "Forest"],
        _ => &[],
    }
}

/// Naive body power for stationing and crewing (the sim does not track
/// individual power; every body contributes this).
pub(super) const BODY_POWER: u32 = 2;

/// Static card data for a battlefield permanent. The cast commander
/// (`usize::MAX`) resolves through `deck.commanders[0]`; token bodies
/// (`usize::MAX - 1`) are 2/2 bodies with no abilities.
pub(super) fn card_of<'a>(deck: &'a SimDeck, perm: &InPlay) -> &'a super::model::SimCard {
    if perm.is_commander {
        // `is_commander` is only set when a commander exists.
        &deck.commanders[perm.commander_slot.min(deck.commanders.len() - 1)]
    } else if perm.card >= usize::MAX - 1 {
        token_body_card()
    } else {
        &deck.cards[perm.card]
    }
}

/// Static data for a 2/2 token body (no abilities, no tap yield).
pub(super) fn token_body() -> super::model::SimCard {
    super::model::SimCard {
        name: "Token".to_string(),
        is_creature: true,
        ..super::model::SimCard::default()
    }
}

/// Lazily-built token body card (built once, read-only).
pub(super) fn token_body_card() -> &'static super::model::SimCard {
    use std::sync::OnceLock;
    static BODY: OnceLock<super::model::SimCard> = OnceLock::new();
    BODY.get_or_init(token_body)
}

/// A new battlefield permanent from a card index.
pub(super) fn new_perm(deck: &SimDeck, card: usize, turn: u32, tapped: bool) -> InPlay {
    InPlay {
        card,
        tapped,
        sick: deck.cards[card].is_creature,
        counters: deck.cards[card].enter_counters,
        animated: false,
        crewed: false,
        entered_turn: turn as usize,
        saga_step: 0,
        fired: false,
        blink_pending: false,
        loyalty: deck.cards[card].starting_loyalty.unwrap_or(0),
        is_commander: false,
        commander_slot: 0,
    }
}

/// Fire OnEnter triggers for the permanent at `pos`. Token payoffs become
/// small battlefield bodies. Upkeep engines register at the call site.
pub(super) fn fire_on_enter(deck: &SimDeck, st: &mut GameState, pos: usize, turn: u32) {
    let Some(perm) = st.battlefield.get(pos) else {
        return;
    };
    let perm_card = perm.card;
    let has_real_etb = card_of(deck, perm)
        .abilities()
        .any(|a| a.trigger == Trigger::OnEnter);
    for ability in card_of(deck, perm).abilities() {
        if ability.trigger != Trigger::OnEnter {
            continue;
        }
        apply_effect(deck, &ability.effect, st, turn);
        // Blink-shaped ETBs ("exile … return it to the battlefield")
        // re-fire the host's OnEnter triggers once, next turn.
        if matches!(ability.effect, Effect::ExtraLand)
            && has_real_etb
            && perm_card < usize::MAX - 1
            && let Some(p) = st
                .battlefield
                .iter_mut()
                .find(|p| p.card == perm_card && p.card < usize::MAX - 1)
        {
            p.blink_pending = true;
        }
    }
}

/// Opening-hand size in every supported format.
pub(super) const OPENING_HAND: usize = 7;

/// Maximum hand size at end of turn.
pub(super) const HAND_LIMIT: usize = 7;

/// The turn loop lives in `game_run`; re-exported for callers.
pub use super::game_run::run_game;
