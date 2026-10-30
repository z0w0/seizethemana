//! Final simulator statistics, separate from game-log storage.

use super::super::findings::PipBlock;
use std::collections::BTreeMap;

/// Draw-agnostic mana readiness for at least one copy of a nonland card.
#[derive(Debug, Clone)]
pub struct CardCast {
    /// Card name.
    pub name: String,
    /// Printed mana value.
    pub cmc: f64,
    /// Target turn from the minimum supported cast cost.
    pub target_turn: u32,
    /// Share of games payable by the target turn.
    pub pct_by_target: f64,
    /// Conditional mean first payable turn; valid only with payable games.
    pub avg_first_castable_turn: f64,
    /// Games where at least one copy became payable within the horizon.
    pub payable_games: u32,
}

/// Aggregated game measurements. Conditional timings include observation counts.
#[derive(Debug, Default, Clone)]
pub struct SimStats {
    /// Games observed.
    pub runs: u32,
    /// Games that reached the requested horizon or a supported player-loss boundary.
    pub finished_games: u32,
    /// Finished games lost at a supported player-loss boundary.
    pub player_loss_games: u32,
    /// Games stopped by a configured simulator work limit.
    pub incomplete_games: u32,
    /// Incomplete-game counts grouped by safety-limit reason.
    pub incomplete_reasons: BTreeMap<String, u32>,
    /// Exact cap locations and remaining-work evidence, grouped across games.
    pub incomplete_observations: BTreeMap<
        (
            super::super::game::GamePhase,
            u32,
            super::super::game::IncompleteReason,
        ),
        u32,
    >,
    /// Exact whole-game successes keyed by the public metric path.
    pub event_games: BTreeMap<String, u32>,
    /// Raw opponent activity across all requested games, including incomplete games.
    pub raw_opponent_activity: super::super::report_schema::OpponentActivityTotals,
    /// Player turns in the observation horizon.
    pub turns: u32,
    /// Turns where the simulator completed each named phase.
    pub phase_observations: Vec<PhaseObservationCounts>,
    /// Average opening lands.
    pub avg_opener_lands: f64,
    /// Share of games taking a mulligan.
    pub mulligan_rate: f64,
    /// Opening land shares [0, 1, 2, 3, 4, 5+].
    pub opener_pct: [f64; 6],
    /// Share making at least k land drops through turn k, indexed from one.
    pub hit_all_drops_by: [f64; 6],
    /// Share with two or fewer lands by turn four.
    pub screw_pct: f64,
    /// Share seeing at least six lands by turn four.
    pub flood_pct: f64,
    /// Mean exact flood expectation at each game's actual draw volume.
    pub flood_expectation: f64,
    /// Median land drops by turn four.
    pub p50_drops_by_4: u32,
    /// 95th percentile land drops by turn four.
    pub p95_drops_by_4: u32,
    /// Commander payable share, indexed by turn from one through the horizon.
    pub commander_castable_by: Vec<f64>,
    /// Conditional mean first payable commander turn.
    pub avg_commander_cast_turn: f64,
    /// Conditional median first payable commander turn.
    pub p50_commander_cast_turn: u32,
    /// Conditional 95th percentile first payable commander turn.
    pub p95_commander_cast_turn: u32,
    /// Games used for commander timing.
    pub commander_payable_games: u32,
    /// Mean unused spendable mana by turn.
    pub unused_mana: Vec<f64>,
    /// Mean spendable mana before main-phase casts by turn.
    pub mana_available: Vec<f64>,
    /// Mean pool mana consumed by casts and cast costs by turn.
    pub mana_spent_on_casts: Vec<f64>,
    /// Share with at least three unused mana after turn six actions.
    pub floated_pct: f64,
    /// Mean cards seen by turn.
    pub cards_seen: Vec<f64>,
    /// Share without draw-source access by turn six.
    pub starved_pct: f64,
    /// Share with draw-source access by turn, distinct from online engines.
    pub draw_sources: Vec<f64>,
    /// Share with removal access by turn five.
    pub removal_access_5: f64,
    /// Share with draw-source access by turn six.
    pub draw_access_6: f64,
    /// Share with creature access by turn three.
    pub creature_access_3: f64,
    /// Share with win-condition access by turn eight.
    pub wincon_access_8: f64,
    /// Share with lock-piece access by turn three.
    pub lock_access_3: f64,
    /// Share with a color-pip block, ordered WUBRG.
    pub color_screw: [f64; 5],
    /// Largest color-pip block share.
    pub color_screw_pct: f64,
    /// Five largest card-name and color block shares.
    pub pip_blocks: Vec<PipBlock>,
    /// Mean graveyard size by turn.
    pub graveyard_by_turn: Vec<f64>,
    /// Mean successful graveyard replay casts per game.
    pub replay_casts_avg: f64,
    /// Mean life paid per game.
    pub life_paid_avg: f64,
    /// Mean life gained per game.
    pub life_gained_avg: f64,
    /// Mean life-funded draws per game.
    pub life_funded_draws_avg: f64,
    /// Mana readiness by card name.
    pub card_castability: Vec<CardCast>,
    /// Removal-role copies for finding context.
    pub removal_count: usize,
    /// Board-wipe copies within the removal count.
    pub removal_wipes: usize,
    /// Targeted removal and counterspell copies.
    pub removal_targeted: usize,
    /// Win-condition role copies.
    pub wincon_count: usize,
    /// Draw-role copies.
    pub draw_count: usize,
    /// Land-role copies.
    pub land_count: usize,
    /// Share with an active commander station by turn six.
    pub station_online_pct: f64,
    /// Conditional median station activation turn.
    pub station_p50_turn: u32,
    /// Games used for station timing.
    pub station_online_games: u32,
    /// Share with a companion in hand by turn six.
    pub companion_online_pct: f64,
    /// Conditional median companion hand-entry turn.
    pub companion_p50_turn: u32,
    /// Games used for companion timing.
    pub companion_online_games: u32,
    /// Mean creature count by turn.
    pub creatures_by_turn: Vec<f64>,
    /// Mean online repeatable sources by turn.
    pub repeatable_sources_by_turn: Vec<f64>,
    /// Mean attack power by turn.
    pub attack_power_by_turn: Vec<f64>,
    /// Mean potential combat damage by commander slot and turn.
    pub commander_combat_by_turn: Vec<Vec<f64>>,
    /// Mean cumulative potential combat damage by commander slot and turn.
    pub commander_combat_cumulative_by_turn: Vec<Vec<f64>>,
    /// Games reaching 21 cumulative potential combat damage by commander slot.
    pub commander_combat_threshold_games: Vec<u32>,
    /// Conditional median turn reaching 21 by commander slot.
    pub commander_combat_threshold_p50: Vec<Option<u32>>,
    /// 90th percentile attack power on turn eight.
    pub attack_power_p90: u32,
    /// Mean evasive attackers by turn.
    pub evasive_by_turn: Vec<f64>,
    /// Mean attackers by turn.
    pub attackers_by_turn: Vec<f64>,
    /// Mean remaining library size by turn.
    pub library_by_turn: Vec<f64>,
    /// Mean self-milled cards by turn.
    pub self_milled_by_turn: Vec<f64>,
    /// Mean cards milled from opponents by turn.
    pub opp_milled_by_turn: Vec<f64>,
    /// Mean known fraction of the library by turn, not a binomial event.
    pub library_awareness_by_turn: Vec<f64>,
    /// Mean cumulative direct opponent life loss by turn.
    pub opponent_life_loss_by_turn: Vec<f64>,
    /// Mean cumulative player damage by turn.
    pub player_damage_by_turn: Vec<f64>,
    /// Share at or above starting table life by turn.
    pub lethal_damage_by_turn: Vec<f64>,
    /// Conditional median first turn reaching the table damage threshold.
    pub p50_lethal_turn: Option<u32>,
    /// Games used for damage-threshold timing.
    pub lethal_games: u32,
    /// Share taking at least one extra turn within the horizon.
    pub extra_turns_pct: f64,
    /// Conditional median first counter-based win-threshold turn.
    pub win_threshold_p50_turn: u32,
    /// Games used for counter-threshold timing.
    pub win_threshold_games: u32,
    /// Share reaching a counter-based win threshold within the horizon.
    pub win_threshold_pct: f64,
    /// Share with an affordable ultimate within the horizon.
    pub ultimate_online_pct: f64,
    /// Share with a payable instant-speed answer in hand by turn.
    pub interaction_ready_by_turn: Vec<f64>,
    /// Mean spare mana after a hypothetical interaction payment on ready turns.
    pub interaction_mana_held: f64,
    /// Player-turn observations used for the conditional interaction mana mean.
    pub interaction_ready_turns: usize,
    /// Mean free-cast permanent entries by turn.
    pub free_cast_permanents_by_turn: Vec<f64>,
    /// Mean dredge uses by turn.
    pub dredge_uses_by_turn: Vec<f64>,
    /// Mean graveyard casts by turn.
    pub graveyard_casts_by_turn: Vec<f64>,
    /// Mean life-funded draws by turn.
    pub life_funded_draws_by_turn: Vec<f64>,
    /// Share reaching a positive mana loop during each turn.
    pub positive_mana_loop_by_turn: Vec<f64>,
    /// Instant-speed interaction copies.
    pub interaction_instant_count: usize,
    /// Share hitting a positive mana-loop execution cap.
    pub infinite_mana_pct: f64,
    /// Individual successful opponent draws across finished games.
    pub opponent_draws: u64,
    /// Paid opponent casts across finished games.
    pub opponent_casts: u64,
    /// Successful optional tax payments across finished games, not mana spent.
    pub opponent_payments: u64,
    /// Scheduled opponent casts skipped for missing resources in finished games.
    pub opponent_skipped_casts: u64,
}

/// Per-turn counts used to define the samples behind runtime means.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhaseObservationCounts {
    /// Completed draw steps.
    pub draw: u32,
    /// Completed precombat main phases.
    pub main: u32,
    /// Completed combat phases.
    pub combat: u32,
    /// Completed end steps.
    pub end_step: u32,
    /// Completed player turns.
    pub end_turn: u32,
}
