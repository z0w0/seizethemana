//! Typed JSON contract for simulator reports and optional analysis blocks.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Deserialize an optional value whose JSON key must be present.
pub fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

/// A public percentage measured from zero to one hundred.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64")]
pub struct Percent(f64);

impl Percent {
    /// Convert an internal zero-to-one share at the report boundary.
    pub fn from_share(share: f64) -> Self {
        Self(round2(share * 100.0))
    }

    /// Round an already scaled public percentage.
    pub fn from_percent(percent: f64) -> Self {
        Self(round2(percent))
    }

    /// Return the public numeric percentage.
    pub fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Percent {
    type Error = &'static str;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if value.is_finite() && (0.0..=100.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err("percent must be finite and between 0 and 100")
        }
    }
}

impl From<Percent> for f64 {
    fn from(value: Percent) -> Self {
        value.0
    }
}

/// Report percentages keyed by the five Magic colors.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorPercents {
    /// White percentage.
    pub white: Option<Percent>,
    /// Blue percentage.
    pub blue: Option<Percent>,
    /// Black percentage.
    pub black: Option<Percent>,
    /// Red percentage.
    pub red: Option<Percent>,
    /// Green percentage.
    pub green: Option<Percent>,
}

impl Default for Percent {
    fn default() -> Self {
        Self(0.0)
    }
}

/// Color counts or mana amounts keyed by the five Magic colors.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorValues {
    /// White value.
    pub white: f64,
    /// Blue value.
    pub blue: f64,
    /// Black value.
    pub black: f64,
    /// Red value.
    pub red: f64,
    /// Green value.
    pub green: f64,
}

/// The complete normal simulator report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimReport {
    /// Deck name.
    pub name: String,
    /// Rules format key.
    pub format: String,
    /// Number of games simulated.
    pub runs: u32,
    /// Requested, finished, lost, and incomplete game counts.
    pub outcomes: RunOutcomes,
    /// Counts of completed turn phases behind the reported per-turn means.
    pub phase_observations: PhaseObservationMeasures,
    /// Turns simulated per game.
    pub turns: u32,
    /// Random-number seed.
    pub seed: u64,
    /// Versioned inputs needed to reproduce and compare this run.
    pub provenance: ReportProvenance,
    /// Synthetic opponent settings and observed activity from simulated games.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opponent_model: Option<OpponentScenario>,
    /// Game-level event counts and sampling intervals keyed by metric path.
    pub sampling: BTreeMap<String, RateEstimate>,
    /// Compiler evidence by card. This is not a claim of complete runtime coverage.
    pub card_support: Vec<CardSupportReport>,
    /// Static deck census.
    pub deck_shape: DeckShape,
    /// Assumptions and limits that qualify the report.
    pub assumptions: Vec<String>,
    /// Opening-hand measures.
    pub opening_hand: OpeningHand,
    /// Land-play measures.
    pub land_drops: LandDrops,
    /// Static land and ramp target assessment.
    pub mana_base: super::findings::ManaBase,
    /// Commander timing, absent when the deck has no commander.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub commander: Option<CommanderReport>,
    /// Station timing, absent for a non-spacecraft commander.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub station: Option<StationReport>,
    /// Companion timing, absent when the deck has no companion.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub companion: Option<CompanionReport>,
    /// Creatures controlled by turn, keyed by one-based turn number.
    pub creatures_by_turn: TurnValues,
    /// Repeatable draw or upkeep sources online by turn.
    pub repeatable_sources_by_turn: TurnValues,
    /// Mana-use measures.
    pub mana: ManaMeasures,
    /// Draw-access measures.
    pub draw: DrawMeasures,
    /// Static role access measures.
    pub role_access: RoleAccess,
    /// Card velocity and library measures.
    pub velocity: VelocityMeasures,
    /// Combat measures.
    pub combat: CombatMeasures,
    /// Win-condition and life-loss measures.
    pub win_conditions: WinMeasures,
    /// Interaction readiness measures.
    pub interaction: InteractionMeasures,
    /// Color mana shortage percentages.
    pub color_mana_shortage: ColorPercents,
    /// Individual card and color pip blocks.
    pub color_pip_blocks: Vec<ColorPipBlock>,
    /// Graveyard measures.
    pub graveyard: GraveyardMeasures,
    /// Event milestones.
    pub milestones: Milestones,
    /// Static per-color source census.
    pub color_sources: ColorSourceCensus,
    /// Card castability rows.
    pub card_castability: Vec<CardCastability>,
    /// Findings produced by this run.
    pub findings: Vec<Finding>,
    /// Short human-readable run summary.
    pub summary: String,
    /// Explicit combo-pair access, when requested and available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combo_access: Option<Vec<PairAccessReport>>,
    /// Colored-source audit, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colored_sources: Option<ColoredSourcesReport>,
    /// Store-backed combo assembly, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combos: Option<CombosReport>,
    /// Complete win-producing combo paths, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_paths: Option<WinPathsReport>,
    /// Exact uniform-sample access baselines, when requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hypgeo: Option<super::hypgeo::HypgeoReport>,
}

/// Reconciled counts for the requested simulator games.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunOutcomes {
    /// Games requested by the caller.
    pub requested_games: u32,
    /// Games that reached the horizon or a supported loss boundary.
    pub finished_games: u32,
    /// Finished games lost at a supported loss boundary.
    pub player_loss_games: u32,
    /// Games stopped by a simulator work limit.
    pub incomplete_games: u32,
    /// Incomplete games grouped by the safety limit that stopped them.
    pub incomplete_reasons: BTreeMap<String, u32>,
    /// Typed cap evidence grouped by reason, turn, and phase.
    pub incomplete_observations: Vec<IncompleteObservation>,
}

/// A safety limit that left real work unfinished in one or more games.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncompleteObservation {
    /// Work limit and exact or lower-bound remaining-work count.
    pub reason: super::game::IncompleteReason,
    /// One-based player-turn slot.
    pub turn: u32,
    /// Phase where execution stopped.
    pub phase: super::game::GamePhase,
    /// Requested games with this same cap evidence.
    pub games: u32,
}

/// Number of games that completed each phase of each player turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseObservationMeasures {
    /// Completed draw steps by turn.
    pub draw_by_turn: TurnValues,
    /// Completed precombat main phases by turn.
    pub main_by_turn: TurnValues,
    /// Completed combat phases by turn.
    pub combat_by_turn: TurnValues,
    /// Completed end steps by turn.
    pub end_step_by_turn: TurnValues,
    /// Completed player turns by turn.
    pub end_turn_by_turn: TurnValues,
}

/// Turn-indexed numeric values, serialized as an object with string keys.
pub type TurnValues = BTreeMap<u32, f64>;

/// Versioned simulator inputs. Fingerprints use SHA-256 over canonical inputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportProvenance {
    /// Public JSON contract version.
    pub schema_version: u32,
    /// Simulator rules model version.
    pub model_version: u32,
    /// Action and mulligan policy version.
    pub policy_version: u32,
    /// Random-number algorithm and stream contract.
    pub rng_version: String,
    /// Application package version.
    pub engine_version: String,
    /// Active simulation names, copy counts, zones, and format; excludes other bench sections.
    pub deck_fingerprint: String,
    /// Card order used by the pilot and seeded shuffle.
    pub execution_order_fingerprint: String,
    /// Exact card-source data, null when source hashes are unavailable.
    pub card_data_fingerprint: Option<String>,
    /// Scope of the hashed inputs, separate from source deck-file text.
    pub fingerprint_note: String,
}

/// Synthetic opponent settings and measurements from the simulated table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpponentScenario {
    /// Version of the illustrative schedule.
    pub model_version: u32,
    /// Stable preset identity.
    pub identity: String,
    /// Number of seats before any elimination.
    pub seats: u32,
    /// Shared scenario parameters.
    pub parameters: super::opponents::OpponentConfig,
    /// Actual activity across observed games, separate from declared settings.
    pub observed_activity: OpponentActivityReport,
    /// Explains how to interpret these uncalibrated settings.
    pub note: String,
}

/// Counts from the actual external table ledger, not an estimate of real tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpponentActivityReport {
    /// All requested games, including those stopped by a work limit.
    pub attempted_games: u32,
    /// Exact activity across all requested games.
    pub raw_totals: OpponentActivityTotals,
    /// Games in the observation denominator.
    pub observed_games: u32,
    /// Exact summed event and payment counts.
    pub totals: OpponentActivityTotals,
    /// Means per game, absent when there are no observed games.
    pub per_game: Option<OpponentActivityAverages>,
    /// Count definitions and remaining measurement limits.
    pub note: String,
}

/// Exact sums use wide counters so multiple games cannot overflow a game counter.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpponentActivityTotals {
    /// Successful individual draws.
    pub draws: u64,
    /// Spells that paid their card and mana costs.
    pub casts: u64,
    /// Successful optional tax payments, not the amount of mana spent.
    pub payments: u64,
    /// Scheduled casts skipped for missing card or mana resources.
    pub skipped_casts: u64,
}

/// Mean external activity per observed game, not binomial event probabilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpponentActivityAverages {
    /// Mean successful individual draws.
    pub draws: f64,
    /// Mean paid casts.
    pub casts: f64,
    /// Mean successful optional tax payments.
    pub payments: f64,
    /// Mean scheduled casts skipped for missing resources.
    pub skipped_casts: f64,
}

/// Per-card compiler evidence, with duplicate physical copies counted once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardSupportReport {
    /// Stable card-name identity.
    pub name: String,
    /// Physical copies across the library, command zone, and companion zone.
    pub copies: u32,
    /// Exact source hash, absent when the compiler did not receive card data.
    pub source_hash: Option<String>,
    /// Lowering evidence, which can omit separate card-level static fields.
    pub abilities: Vec<AbilitySupportReport>,
    /// Prevents empty or supported records from implying complete rules coverage.
    pub note: String,
}

/// One ability's lowering result and original source range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbilitySupportReport {
    /// Stable row key within this compiled card definition.
    pub identity: String,
    /// Original Oracle wording, or the compiler's synthesized-source label.
    pub source: String,
    /// First source byte, absent for synthesized abilities.
    pub span_start: Option<usize>,
    /// First byte after the source range, absent for synthesized abilities.
    pub span_end: Option<usize>,
    /// Compiler result: supported, compiler_only, approximated, or unsupported.
    pub status: String,
    /// Declared approximation rule, when applicable.
    pub rule_id: Option<String>,
    /// Lowering gap, when applicable.
    pub reason: Option<String>,
}

/// Why an event rate is available or cannot be measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    /// At least one whole game observed the requested window.
    Observed,
    /// The requested turn is after the simulation endpoint.
    OutsideHorizon,
    /// This deck does not have the measured object or mechanic.
    NotApplicable,
    /// No games observed the requested window.
    NoObservations,
}

/// A binomial interval for independent whole-game event observations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceInterval {
    /// Interval method, separate from scenario and model uncertainty.
    pub method: String,
    /// Nominal coverage, as a share from zero to one.
    pub confidence_level: f64,
    /// Lower percentage bound.
    pub lower: Percent,
    /// Upper percentage bound.
    pub upper: Percent,
}

/// Event counts for one whole-game probability estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateEstimate {
    /// Whether this metric has observations.
    pub status: ObservationStatus,
    /// Independent games in the denominator.
    pub observed_games: u32,
    /// Games where the event occurred; null when the metric is unavailable.
    pub event_games: Option<u32>,
    /// Event percentage; an observed zero differs from no observations.
    pub percent: Option<Percent>,
    /// Sampling interval, which does not cover model error.
    pub interval: Option<ConfidenceInterval>,
}

impl RateEstimate {
    /// Build a game-level estimate with a 95% Wilson score interval.
    pub fn observed(events: u32, games: u32) -> Self {
        if games == 0 {
            return Self::unavailable(ObservationStatus::NoObservations);
        }
        assert!(events <= games, "event count cannot exceed observed games");
        let n = f64::from(games);
        let p = f64::from(events) / n;
        let z = 1.959963984540054;
        let denominator = 1.0 + z * z / n;
        let center = (p + z * z / (2.0 * n)) / denominator;
        let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denominator;
        Self {
            status: ObservationStatus::Observed,
            observed_games: games,
            event_games: Some(events),
            percent: Some(Percent::from_share(p)),
            interval: Some(ConfidenceInterval {
                method: "wilson_score".to_string(),
                confidence_level: 0.95,
                lower: Percent::from_share((center - half).max(0.0)),
                upper: Percent::from_share((center + half).min(1.0)),
            }),
        }
    }

    /// Keep unavailable windows separate from observed event-free games.
    pub fn unavailable(status: ObservationStatus) -> Self {
        Self {
            status,
            observed_games: 0,
            event_games: None,
            percent: None,
            interval: None,
        }
    }
}

/// Counts and mana-curve distribution for a deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeckShape {
    /// Total cards, including commanders.
    pub total_cards: usize,
    /// Average mana value of nonland library cards.
    pub average_nonland_mana_value: f64,
    /// Sideboard cards not simulated.
    pub sideboard_cards: i64,
    /// Maybeboard cards not simulated.
    pub maybeboard_cards: i64,
    /// Lands in the library.
    pub lands: i64,
    /// Artifact mana sources by role census.
    pub artifact_mana_sources: i64,
    /// Creature mana sources by role census.
    pub creature_mana_sources: i64,
    /// Ramp spells by role census.
    pub ramp_spells: i64,
    /// Draw sources by role census.
    pub draw_sources: i64,
    /// Removal spells by role census.
    pub removal_spells: i64,
    /// Targeted removal spells by role census.
    pub targeted_removal_spells: i64,
    /// Mass-removal spells by role census.
    pub mass_removal_spells: i64,
    /// Win conditions by role census.
    pub win_conditions: i64,
    /// Lock pieces by role census.
    pub locks: i64,
    /// Booster pieces by role census.
    pub boosters: i64,
    /// Nonland cards by mana-value bucket.
    pub curve: BTreeMap<String, i64>,
}

/// Opening-hand and mulligan measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpeningHand {
    /// Average lands in the opening hand.
    pub avg_lands: Option<f64>,
    /// Opening hands with no lands.
    pub percent_0_lands: Option<Percent>,
    /// Opening hands with one land.
    pub percent_1_land: Option<Percent>,
    /// Opening hands with two lands.
    pub percent_2_lands: Option<Percent>,
    /// Opening hands with three lands.
    pub percent_3_lands: Option<Percent>,
    /// Opening hands with four lands.
    pub percent_4_lands: Option<Percent>,
    /// Opening hands with five or more lands.
    pub percent_five_or_more_lands: Option<Percent>,
    /// Games that took a mulligan.
    pub percent_mulliganed: Option<Percent>,
}

/// Land-play consistency measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandDrops {
    /// Games hitting all available land drops by each turn.
    pub percent_games_hitting_all_land_drops_by_turn: BTreeMap<u32, Percent>,
    /// Games with two or fewer lands by turn four.
    pub percent_games_with_two_or_fewer_lands_by_turn_4: Option<Percent>,
    /// Games seeing six or more lands by turn four.
    pub percent_games_with_six_or_more_lands_by_turn_4: Option<Percent>,
    /// Exact six-land expectation at the same draw volume.
    pub expected_percent_with_six_or_more_lands_by_turn_4: Option<Percent>,
    /// Median land drops by turn four.
    pub p50_drops_by_4: Option<u32>,
    /// 95th percentile land drops by turn four.
    pub p95_drops_by_4: Option<u32>,
}

/// Commander cast timing measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommanderReport {
    /// Commander name.
    pub name: String,
    /// Printed mana value.
    pub mana_value: f64,
    /// Games castable by each turn.
    pub percent_castable_by_turn: BTreeMap<u32, Percent>,
    /// Average first payable turn among games where the commander became payable.
    pub avg_first_cast_turn: Option<f64>,
    /// Median first payable turn among games where the commander became payable.
    pub p50_cast_turn: Option<u32>,
    /// 95th percentile first payable turn among games where the commander became payable.
    pub p95_cast_turn: Option<u32>,
    /// Games used for conditional timing, out of all simulated games.
    pub payable_games: u32,
    /// Games castable by the mana-value turn, null outside the run horizon.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub percent_castable_by_curve: Option<Percent>,
}

/// Station online timing measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StationReport {
    /// Games online by turn six.
    pub online_by_t6: Option<Percent>,
    /// Median online turn among games where the station became active.
    pub p50_online_turn: Option<u32>,
    /// Games used for conditional timing, out of all simulated games.
    pub online_games: u32,
}

/// Companion access timing measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanionReport {
    /// Games with the companion in hand by turn six.
    pub in_hand_by_t6: Option<Percent>,
    /// Median turn to reach hand among games where the companion was fetched.
    pub p50_online_turn: Option<u32>,
    /// Games used for conditional timing, out of all simulated games.
    pub in_hand_games: u32,
}

/// Mana-use measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManaMeasures {
    /// Average spendable pool before casts and activations by turn.
    pub avg_available_by_turn: TurnValues,
    /// Average pool mana consumed by casts and cast costs by turn.
    pub avg_cast_spent_by_turn: TurnValues,
    /// Average spendable mana left after main-phase actions by turn.
    pub avg_unused_by_turn: TurnValues,
    /// Games with three or more spendable mana left after turn-six main-phase actions.
    pub percent_games_with_three_or_more_unused_mana_by_turn_6: Option<Percent>,
}

/// Draw-access and life measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawMeasures {
    /// Draw-source access by turn.
    pub draw_source_percent_seen_by_turn: BTreeMap<u32, Percent>,
    /// Games without a draw source by turn six.
    pub percent_games_with_no_draw_source_by_turn_6: Option<Percent>,
    /// Average life paid.
    pub avg_life_paid: Option<f64>,
    /// Average life gained.
    pub avg_life_gained: Option<f64>,
    /// Average life-funded draws.
    pub avg_life_funded_draws: Option<f64>,
}

/// Static role-access measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleAccess {
    /// Games with a removal spell seen by turn five.
    pub removal_spell_percent_seen_by_turn_5: Option<Percent>,
    /// Games with a draw source seen by turn six.
    pub draw_source_percent_seen_by_turn_6: Option<Percent>,
    /// Games with a creature role seen by turn three.
    pub creature_role_percent_seen_by_turn_3: Option<Percent>,
    /// Games with a win-condition role seen by turn eight.
    pub win_condition_role_percent_seen_by_turn_8: Option<Percent>,
    /// Games with a lock role seen by turn three.
    pub lock_role_percent_seen_by_turn_3: Option<Percent>,
}

/// Card velocity and library measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VelocityMeasures {
    /// Average cards seen by turn.
    pub avg_cards_seen: TurnValues,
    /// Mean share of library contents observed by turn, not a whole-game event rate.
    pub library_awareness_by_turn: BTreeMap<u32, Percent>,
    /// Average cards self-milled by turn.
    pub self_milled_by_turn: TurnValues,
    /// Average cards milled from opponents by turn.
    pub opponent_milled_by_turn: TurnValues,
    /// Average cards left in the library by turn.
    pub library_remaining_by_turn: TurnValues,
}

/// Combat measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombatMeasures {
    /// Average attack power by turn.
    pub attack_power_avg_by_turn: TurnValues,
    /// Average player damage by turn.
    pub player_damage_avg_by_turn: TurnValues,
    /// 90th percentile attack power by turn eight.
    pub attack_power_p90_by_t8: Option<f64>,
    /// Average attackers by turn.
    pub attackers_by_turn: TurnValues,
    /// Average evasive attackers by turn.
    pub evasive_by_turn: TurnValues,
    /// Potential combat damage by commander slot. This does not apply damage to opponents.
    pub commander_potential: Vec<CommanderCombatPotential>,
}

/// Potential combat contribution from one commander across the requested games.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommanderCombatPotential {
    /// Stable position in the deck's commander list.
    pub slot: u32,
    /// Commander card name.
    pub name: String,
    /// Mean combat potential assigned to this commander on each turn.
    pub damage_by_turn: TurnValues,
    /// Mean cumulative combat potential assigned to this commander by each turn.
    pub cumulative_damage_by_turn: TurnValues,
    /// Share of finished games reaching 21 cumulative potential damage.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub percent_games_reaching_21: Option<Percent>,
    /// Finished games in the threshold denominator.
    pub observed_games: u32,
    /// Games that reached the 21-point potential threshold.
    pub games_reaching_21: u32,
    /// Median turn among games that reached the threshold.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub median_turn_to_21: Option<u32>,
    /// Limits of this potential-only measure.
    pub note: String,
}

/// Win-condition, life-loss, and threshold measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WinMeasures {
    /// Opponent life lost from effects by turn.
    pub opponent_life_loss_by_turn: TurnValues,
    /// Games with an extra turn.
    pub percent_games_with_extra_turn: Option<Percent>,
    /// Median turn reaching the counter-based win threshold.
    pub win_threshold_p50_turn: Option<u32>,
    /// Games used for the conditional counter-threshold timing.
    pub counter_win_threshold_games: u32,
    /// Games reaching the counter-based win threshold.
    pub percent_games_reaching_counter_win_threshold: Option<Percent>,
    /// Games with an affordable planeswalker ultimate.
    pub percent_games_with_affordable_ultimate: Option<Percent>,
    /// Games with suspected infinite mana.
    pub percent_games_with_suspected_infinite_mana: Option<Percent>,
    /// Games at or above table life by turn.
    pub percent_games_at_or_above_table_life_by_turn: BTreeMap<u32, Percent>,
    /// Median turn among games that reached the damage threshold.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub p50_lethal_turn: Option<u32>,
    /// Games used for the conditional damage-threshold timing.
    pub damage_threshold_games: u32,
    /// Limits and conditional denominator for the lethal estimate.
    pub lethal_note: String,
}

/// Interaction readiness measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionMeasures {
    /// Per-turn share of games that ended the main phase with a payable
    /// instant-speed interaction card in hand.
    pub percent_games_with_ready_interaction_by_turn: BTreeMap<u32, Percent>,
    /// Average general plus instant/sorcery-restricted mana left after
    /// hypothetical payment for the cheapest payable answer on ready turns.
    pub mana_held_avg: Option<f64>,
    /// Player-turn observations in the conditional spare-mana mean.
    pub ready_turn_observations: usize,
    /// Instant-speed interaction count.
    pub instant_speed_count: usize,
    /// Readiness measure clarification.
    pub note: String,
}

/// A card whose color pips could not be paid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorPipBlock {
    /// Card name.
    pub name: String,
    /// Blocked color letter.
    pub color: String,
    /// Games with at least one blocked cast.
    pub percent_of_games: Percent,
}

/// Graveyard measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraveyardMeasures {
    /// Average graveyard size by turn.
    pub avg_size_by_turn: TurnValues,
    /// Average replay casts.
    pub replay_casts_avg: Option<f64>,
}

/// Turn-indexed action and loop milestones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Milestones {
    /// Average free-cast permanents entering by turn.
    pub free_cast_permanents_entered_avg_by_turn: TurnValues,
    /// Average dredge uses by turn.
    pub dredge_uses_avg_by_turn: TurnValues,
    /// Average graveyard casts by turn.
    pub graveyard_casts_avg_by_turn: TurnValues,
    /// Average life-funded draws by turn.
    pub life_funded_draws_avg_by_turn: TurnValues,
    /// Games with a positive mana loop during each turn.
    pub percent_games_with_positive_mana_loop_by_turn: BTreeMap<u32, Percent>,
    /// Clarification of turn-based counts and loop shares.
    pub note: String,
}

/// Static land and nonland mana-source census.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorSourceCensus {
    /// Single-color land sources.
    pub fixed_source_lands: ColorValues,
    /// Choice land sources, counted for every color they can make.
    pub choice_source_lands: ColorValues,
    /// Nonland mana sources with supported output.
    pub flexible_nonland_sources: u32,
    /// Sources whose mana yield scales.
    pub scaling_sources: u32,
    /// Census interpretation.
    pub note: String,
}

/// Draw-agnostic mana readiness for at least one copy of a card name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardCastability {
    /// Card name.
    pub name: String,
    /// Mana value.
    pub mana_value: f64,
    /// Target cast turn.
    pub target_turn: u32,
    /// Games with enough mana by the target turn, null outside the horizon.
    pub percent_castable_by_target: Option<Percent>,
    /// Average first payable turn among games that became payable.
    pub avg_first_castable_turn: Option<f64>,
    /// Games used for conditional timing, out of all simulated games.
    pub payable_games: u32,
}

/// Finding evidence row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingEvidence {
    /// Card name or count label behind the finding.
    pub subject: String,
    /// Evidence explanation.
    pub explanation: String,
    /// Game share when the evidence is game-based.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub percent_of_games: Option<Percent>,
}

/// One stable, typed deck diagnosis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable machine-readable diagnosis kind.
    pub kind: String,
    /// Stable identity, including color when the finding is color-specific.
    pub identity: String,
    /// Severity category.
    pub severity: String,
    /// Game share when the diagnosis is game-based.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub percent_of_games: Option<Percent>,
    /// Color letter for color-specific diagnoses.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub color: Option<String>,
    /// Explanation of the measured deficit.
    pub explanation: String,
    /// Suggested deck change.
    pub suggestion: String,
    /// Cards or counts supporting this diagnosis.
    pub evidence: Vec<FindingEvidence>,
}

/// Per-color colored-source audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColoredSourcesReport {
    /// Format used for audit bands.
    pub format: String,
    /// Land count, including weighted spell/land MDFCs.
    pub lands: f64,
    /// Colored sources by color.
    pub sources: ColorValues,
    /// Source credits by category and color.
    pub credits: SourceCredits,
    /// Colored pip requirements by card.
    pub requirements: Vec<ColorRequirement>,
    /// Largest colored mana deficits, formatted for display.
    pub worst_deficits: Vec<String>,
    /// Lands entering tapped.
    pub tapland_count: usize,
    /// Turn-one untapped sources by color.
    pub untapped_t1_sources: ColorValues,
}

/// Colored-source credits by source category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceCredits {
    /// Land source credits.
    pub lands: ColorValues,
    /// Creature mana-source credits.
    pub creature_mana_sources: ColorValues,
    /// Artifact mana-source credits.
    pub artifact_mana_sources: ColorValues,
    /// Cantrip source credits.
    pub cantrips: ColorValues,
}

/// Colored pip requirement and source coverage for one card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorRequirement {
    /// Card name.
    pub name: String,
    /// Printed mana cost.
    pub mana_cost: String,
    /// Mana value.
    pub mana_value: f64,
    /// Required pips by color.
    pub needs: ColorValues,
    /// Available source credits by color.
    pub have: ColorValues,
    /// Remaining deficits by color.
    pub deficit: ColorValues,
    /// Whether every colored requirement meets the target.
    pub ok: bool,
}

/// Store-backed combo summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombosReport {
    /// Store supplying the combo data.
    pub source: String,
    /// Number of variants considered.
    pub variants_considered: usize,
    /// Whether the local combo store is complete.
    /// Complete combo variants.
    pub complete: Vec<ComboAccessReport>,
    /// One-card-away variants.
    pub near_misses: Vec<ComboAccessReport>,
}

/// Typed public report row for combo assembly access.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComboAccessReport {
    /// Joined piece names.
    pub combo: String,
    /// Spellbook variant identifier.
    pub id: String,
    /// Effects the combo produces.
    pub produces: Vec<String>,
    /// Spellbook bracket tag.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub bracket_tag: Option<String>,
    /// Target assembly turn.
    pub target_turn: u32,
    /// Percentage where every piece first reached a listed zone. Not execution.
    pub percent_of_games: Option<Percent>,
    /// Whole-game access counts and a sampling interval, when observable.
    pub access: RateEstimate,
    /// Piece names missing from the deck.
    pub missing: Vec<String>,
}

impl From<&super::combos::ComboAccess> for ComboAccessReport {
    fn from(row: &super::combos::ComboAccess) -> Self {
        Self {
            combo: row.combo.clone(),
            id: row.id.clone(),
            produces: row.produces.clone(),
            bracket_tag: row.bracket_tag.clone(),
            target_turn: row.target_turn,
            percent_of_games: row.access.percent,
            access: row.access.clone(),
            missing: row.missing.clone(),
        }
    }
}

/// Typed public report row for a requested two-card pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairAccessReport {
    /// Joined piece names.
    pub pair: String,
    /// Target turn.
    pub target_turn: u32,
    /// Percentage of games where both pieces were seen.
    pub percent_of_games: Option<Percent>,
    /// Whole-game access counts, separate from executable combo availability.
    pub access: RateEstimate,
}

impl PairAccessReport {
    /// Preserve unavailable deadlines instead of labeling them zero-percent access.
    pub fn from_access(row: &super::findings::ComboAccess, _runs: u32, turns: u32) -> Self {
        let access = if row.target_turn == 0 {
            RateEstimate::unavailable(ObservationStatus::NotApplicable)
        } else if row.target_turn > turns {
            RateEstimate::unavailable(ObservationStatus::OutsideHorizon)
        } else {
            RateEstimate::observed(row.event_games, row.observed_games)
        };
        Self {
            pair: row.pair.clone(),
            target_turn: row.target_turn,
            percent_of_games: access.percent,
            access,
        }
    }
}

/// Complete combo paths that produce a win feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WinPathsReport {
    /// Number of paths returned.
    pub count: usize,
    /// Win-producing paths.
    pub paths: Vec<ComboAccessReport>,
}

/// Round a report number to two decimal places.
pub fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
