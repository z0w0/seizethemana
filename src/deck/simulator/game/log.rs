//! Per-game observations consumed by streaming report accumulators.

use std::collections::HashMap;

use super::super::{model::Role, opponents::OpponentActivity};

/// A rules boundary in the player's turn or a scheduled opponent round.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum GamePhase {
    /// Upkeep actions.
    Upkeep,
    /// The player's draw step.
    Draw,
    /// The precombat or postcombat main phase.
    Main,
    /// The player's combat phase.
    Combat,
    /// End-step actions and cleanup.
    EndStep,
    /// Scheduled activity from synthetic opponents.
    OpponentRound,
}

/// A supported reason the player loses the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerLossReason {
    /// The player had zero or less life at a state-based action check.
    ZeroLife,
    /// The player attempted to draw from an empty library.
    FailedDraw,
}

/// A configured simulator work limit stopped unresolved work.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncompleteReason {
    /// A hand-cast pass stopped while another legal cast remained available.
    CastPass {
        /// Maximum committed casts in one hand-cast pass.
        configured_limit: u32,
        /// Lower bound on legal casts left after the final allowed cast.
        remaining_work: u32,
    },
    /// A graveyard pass stopped while another legal replay remained available.
    GraveyardCastPass {
        /// Maximum committed graveyard casts in one pass.
        configured_limit: u32,
        /// Lower bound on legal replays left after the final allowed cast.
        remaining_work: u32,
    },
    /// An effect requested more individual objects than the effect-work limit.
    EffectWork {
        /// Maximum objects created by one effect.
        configured_limit: u32,
        /// Requested objects not created.
        remaining_work: u32,
    },
    /// Trigger settlement stopped with pending triggers.
    TriggerSettlement {
        /// Maximum triggers resolved in one settlement.
        configured_limit: u32,
        /// Triggers still queued when the limit stopped resolution.
        remaining_work: u32,
    },
    /// The main-phase action loop stopped before reaching a stable state.
    TurnAction {
        /// Maximum action-loop passes in one main phase.
        configured_limit: u32,
        /// Pending cast or action candidates at the limit.
        remaining_work: u32,
    },
    /// The activation sweep stopped while an activation remained available.
    ActivationPass {
        /// Maximum activations in one sweep.
        configured_limit: u32,
        /// Available activations left after the final activation.
        remaining_work: u32,
    },
}

/// Whether a game reached its horizon or ended at a supported loss boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameCompletion {
    /// The game reached the requested observation horizon.
    HorizonReached,
    /// The player lost at the recorded rules boundary.
    PlayerLost {
        /// Reasons that caused this loss check to fail.
        reasons: Vec<PlayerLossReason>,
        /// One-based player turn, or the player turn associated with an opponent round.
        turn: u32,
        /// Phase where the loss was observed.
        phase: GamePhase,
    },
    /// The simulator stopped because a safety limit left work unresolved.
    Incomplete {
        /// The safety limit that stopped the game.
        reason: IncompleteReason,
        /// One-based player turn, or the player turn associated with an opponent round.
        turn: u32,
        /// Phase where the limit stopped work.
        phase: GamePhase,
    },
}

/// Phases that produced real observations in one scheduled turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TurnObservation {
    /// The player reached its draw step.
    pub draw: bool,
    /// The player reached its precombat main phase.
    pub main: bool,
    /// The player reached combat.
    pub combat: bool,
    /// The player reached the end step.
    pub end_step: bool,
    /// The player completed end-step actions and turn cleanup.
    pub end_turn: bool,
}

/// One game's observations. Timing samples use player turns starting at one.
#[derive(Debug, Clone, PartialEq)]
pub struct GameLog {
    /// Terminal outcome or successful completion of the requested horizon.
    pub completion: GameCompletion,
    /// Number of player turns observed before the game ended.
    pub turns_observed: u32,
    /// Phase observations for each requested player turn.
    pub turn_observations: Vec<TurnObservation>,
    /// Actual external draws, casts, payments, and skipped actions.
    pub opponent_activity: OpponentActivity,
    /// Land drops in each observed turn.
    pub land_drops: Vec<u8>,
    /// Spendable mana before the first main-phase action of each turn.
    pub mana_available: Vec<f64>,
    /// Mana spent on casts and cast costs, excluding other actions.
    pub mana_spent: Vec<f64>,
    /// Spendable mana left after the main-phase action policy.
    pub mana_remaining: Vec<f64>,
    /// Cumulative cards seen by each turn's end.
    pub cards_seen: Vec<u32>,
    /// First payable commander turn, including a payment that caused a loss.
    pub commander_castable: Option<u32>,
    /// First payable turn per physical card, without requiring hand access.
    pub mana_ready: Vec<Option<u32>>,
    /// First hand access by role.
    pub first_seen: HashMap<Role, u32>,
    /// First creature hand access, if observed.
    pub first_creature: Option<u32>,
    /// Colors missing from an attempted cast with sufficient total mana.
    pub blocked_colors: [bool; 5],
    /// Land-capable cards in the kept opener.
    pub opener_lands: u8,
    /// Whether the pilot redrew the opener.
    pub mulliganed: bool,
    /// Land drops through turn four.
    pub lands_by_4: u8,
    /// Lands seen through turn four, retaining known access before an early loss.
    /// Shorter horizons retain their last known count but do not support this rate.
    pub lands_seen_by_turn_4: u32,
    /// Actual cards seen through turn four or an earlier terminal boundary.
    /// This fixed-window sample does not fill missing per-turn observations.
    pub cards_seen_by_4: u32,
    /// First commander animation through station, if observed.
    pub station_online: Option<u32>,
    /// First companion access to hand, if observed (CR 702.139a).
    pub companion_online: Option<u32>,
    /// Creature permanents per turn, including animated Vehicles.
    pub creatures: Vec<u32>,
    /// Repeatable card sources available per turn.
    pub repeatable_sources_online: Vec<u32>,
    /// Attempted casts blocked by a color, as card/color index pairs.
    pub pip_blocks: Vec<(usize, usize)>,
    /// Graveyard size at each turn's end.
    pub graveyard_size: Vec<u32>,
    /// First hand access per physical card, not executable combo assembly.
    pub card_first_seen: HashMap<usize, u32>,
    /// First battlefield arrival per physical card.
    pub card_first_battlefield: HashMap<usize, u32>,
    /// First graveyard arrival per physical card.
    pub card_first_graveyard: HashMap<usize, u32>,
    /// Attacking power per turn, including buffs and double strike.
    pub attack_power: Vec<u32>,
    /// Potential combat damage by commander slot and turn.
    pub commander_combat_damage: Vec<Vec<u32>>,
    /// Supported flashback or escape casts that paid their costs.
    pub replay_casts: u32,
    /// Life spent on costs.
    pub life_paid: u32,
    /// Life gained from effects and lifelink.
    pub life_gained: u32,
    /// Draws funded by life costs or linked life loss.
    pub life_funded_draws: u32,
    /// Attacking creatures per turn.
    pub attackers: Vec<u32>,
    /// Attacking creatures with flying, menace, or trample per turn.
    pub evasive: Vec<u32>,
    /// Remaining library size per turn.
    pub library_size: Vec<u32>,
    /// Cumulative cards milled from the player's library.
    pub self_milled: Vec<u32>,
    /// Cumulative cards milled toward opponents.
    pub opp_milled: Vec<u32>,
    /// Cumulative fraction of the library examined by draws or selection.
    pub awareness: Vec<f64>,
    /// Cumulative opponent life loss from effects.
    pub opponent_life_loss: Vec<u32>,
    /// Cumulative player damage, including combat and damage triggers.
    pub player_damage: Vec<u32>,
    /// Cumulative extra turns taken.
    pub extra_turns: Vec<u32>,
    /// Resolved action counts per turn.
    pub milestones_by_turn: Vec<TurnMilestone>,
    /// First observed turn a supported win threshold could fire.
    pub win_threshold_turn: Option<u32>,
    /// First observed turn a planeswalker ultimate became affordable.
    pub ultimate_online: Option<u32>,
    /// Interaction in hand with an affordable cost per turn.
    pub interaction_ready: Vec<bool>,
    /// Spare mana on turns with payable interaction.
    pub interaction_mana_held: Vec<f64>,
    /// A bounded positive-mana activation path reached its work limit.
    pub infinite_mana_suspected: bool,
}

/// Resolved actions, separate from role classifications and card access.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnMilestone {
    /// Permanent cards entering through cascade free casts.
    pub free_cast_permanents_entered: u32,
    /// Draws replaced by dredge.
    pub dredge_uses: u32,
    /// Successful flashback and escape casts.
    pub graveyard_casts: u32,
    /// Cards drawn through life-funded actions.
    pub life_funded_draws: u32,
    /// A bounded positive-mana activation path reached its work limit.
    pub positive_mana_loop: bool,
}
