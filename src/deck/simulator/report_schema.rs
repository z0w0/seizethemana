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
    pub white: Percent,
    /// Blue percentage.
    pub blue: Percent,
    /// Black percentage.
    pub black: Percent,
    /// Red percentage.
    pub red: Percent,
    /// Green percentage.
    pub green: Percent,
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
    /// Turns simulated per game.
    pub turns: u32,
    /// Random-number seed.
    pub seed: u64,
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
    /// Exact hypergeometric ceilings, when requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hypgeo: Option<super::hypgeo::HypgeoReport>,
}

/// Turn-indexed numeric values, serialized as an object with string keys.
pub type TurnValues = BTreeMap<u32, f64>;

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
    pub avg_lands: f64,
    /// Opening hands with no lands.
    pub percent_0_lands: Percent,
    /// Opening hands with one land.
    pub percent_1_land: Percent,
    /// Opening hands with two lands.
    pub percent_2_lands: Percent,
    /// Opening hands with three lands.
    pub percent_3_lands: Percent,
    /// Opening hands with four lands.
    pub percent_4_lands: Percent,
    /// Opening hands with five or more lands.
    pub percent_five_or_more_lands: Percent,
    /// Games that took a mulligan.
    pub percent_mulliganed: Percent,
}

/// Land-play consistency measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandDrops {
    /// Games hitting all available land drops by each turn.
    pub percent_games_hitting_all_land_drops_by_turn: BTreeMap<u32, Percent>,
    /// Games with two or fewer lands by turn four.
    pub percent_games_with_two_or_fewer_lands_by_turn_4: Percent,
    /// Games seeing six or more lands by turn four.
    pub percent_games_with_six_or_more_lands_by_turn_4: Percent,
    /// Exact six-land expectation at the same draw volume.
    pub expected_percent_with_six_or_more_lands_by_turn_4: Percent,
    /// Median land drops by turn four.
    pub p50_drops_by_4: u32,
    /// 95th percentile land drops by turn four.
    pub p95_drops_by_4: u32,
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
    /// Average first castable turn.
    pub avg_first_cast_turn: f64,
    /// Median first castable turn.
    pub p50_cast_turn: u32,
    /// 95th percentile first castable turn.
    pub p95_cast_turn: u32,
    /// Games castable by the mana-value turn, null outside the run horizon.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub percent_castable_by_curve: Option<Percent>,
}

/// Station online timing measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StationReport {
    /// Games online by turn six.
    pub online_by_t6: Percent,
    /// Median online turn.
    pub p50_online_turn: u32,
}

/// Companion access timing measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanionReport {
    /// Games with the companion in hand by turn six.
    pub in_hand_by_t6: Percent,
    /// Median turn the companion reaches hand.
    pub p50_online_turn: u32,
}

/// Mana-use measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManaMeasures {
    /// Average unused mana by turn.
    pub avg_unused_by_turn: TurnValues,
    /// Games with three or more unused mana by turn six.
    pub percent_games_with_three_or_more_unused_mana_by_turn_6: Percent,
}

/// Draw-access and life measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawMeasures {
    /// Draw-source access by turn.
    pub draw_source_percent_seen_by_turn: BTreeMap<u32, Percent>,
    /// Games without a draw source by turn six.
    pub percent_games_with_no_draw_source_by_turn_6: Percent,
    /// Average life paid.
    pub avg_life_paid: f64,
    /// Average life gained.
    pub avg_life_gained: f64,
    /// Average life-funded draws.
    pub avg_life_funded_draws: f64,
}

/// Static role-access measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleAccess {
    /// Games with a removal spell seen by turn five.
    pub removal_spell_percent_seen_by_turn_5: Percent,
    /// Games with a draw source seen by turn six.
    pub draw_source_percent_seen_by_turn_6: Percent,
    /// Games with a creature role seen by turn three.
    pub creature_role_percent_seen_by_turn_3: Percent,
    /// Games with a win-condition role seen by turn eight.
    pub win_condition_role_percent_seen_by_turn_8: Percent,
    /// Games with a lock role seen by turn three.
    pub lock_role_percent_seen_by_turn_3: Percent,
}

/// Card velocity and library measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VelocityMeasures {
    /// Average cards seen by turn.
    pub avg_cards_seen: TurnValues,
    /// Share of games where library contents are known by turn.
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
    pub attack_power_p90_by_t8: f64,
    /// Average attackers by turn.
    pub attackers_by_turn: TurnValues,
    /// Average evasive attackers by turn.
    pub evasive_by_turn: TurnValues,
}

/// Win-condition, life-loss, and threshold measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WinMeasures {
    /// Opponent life lost from effects by turn.
    pub opponent_life_loss_by_turn: TurnValues,
    /// Games with an extra turn.
    pub percent_games_with_extra_turn: Percent,
    /// Median turn reaching the counter-based win threshold.
    pub win_threshold_p50_turn: u32,
    /// Games reaching the counter-based win threshold.
    pub percent_games_reaching_counter_win_threshold: Percent,
    /// Games with an affordable planeswalker ultimate.
    pub percent_games_with_affordable_ultimate: Percent,
    /// Games with suspected infinite mana.
    pub percent_games_with_suspected_infinite_mana: Percent,
    /// Games at or above table life by turn.
    pub percent_games_at_or_above_table_life_by_turn: BTreeMap<u32, Percent>,
    /// Median best-case lethal turn.
    #[serde(deserialize_with = "deserialize_present_option")]
    pub p50_lethal_turn: Option<u32>,
    /// Limits on the lethal estimate.
    pub lethal_note: String,
}

/// Interaction readiness measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionMeasures {
    /// Games with ready interaction by turn.
    pub percent_games_with_ready_interaction_by_turn: BTreeMap<u32, Percent>,
    /// Average mana held for interaction.
    pub mana_held_avg: f64,
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
    pub replay_casts_avg: f64,
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

/// Castability summary for one distinct card name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardCastability {
    /// Card name.
    pub name: String,
    /// Mana value.
    pub mana_value: f64,
    /// Target cast turn.
    pub target_turn: u32,
    /// Games with enough mana by the target turn.
    pub percent_castable_by_target: Percent,
    /// Average first turn with enough mana.
    pub avg_first_castable_turn: f64,
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
    /// Percentage of games where all required pieces reached their zones.
    pub percent_of_games: Percent,
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
            percent_of_games: Percent::from_share(row.game_share),
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
    pub percent_of_games: Percent,
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
