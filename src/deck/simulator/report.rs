//! Report renders for the simulator: the JSON payload and the human
//! stdout view. JSON adds station/bodies/engines metrics; the human view
//! gains a station line when the commander is a spacecraft.

use super::aggregate::SimStats;
use super::model::{Role, SimDeck};
use super::report_schema::*;
use crate::output::Output;

mod assumptions;
mod inputs;
mod sampling;

use sampling::fixed_percent;

/// Lock-pieces in the deck (Role::Lock census for the deck shape).
fn lock_count(deck: &SimDeck) -> i64 {
    deck.library_cards()
        .filter(|c| c.role == Role::Lock)
        .count() as i64
}

/// Booster-pieces in the deck (equipment, auras, pump spells).
fn booster_count(deck: &SimDeck) -> i64 {
    deck.library_cards()
        .filter(|c| c.role == Role::Booster)
        .count() as i64
}

/// Total cards in the deck (library + commanders).
fn total_cards(deck: &SimDeck) -> usize {
    deck.library_len() + deck.commanders.len()
}

/// Average nonland CMC across the library.
fn avg_cmc(deck: &SimDeck) -> f64 {
    let nonlands: Vec<&super::model::SimCard> = deck
        .library_cards()
        .filter(|c| c.role != Role::Land)
        .collect();
    if nonlands.is_empty() {
        return 0.0;
    }
    nonlands.iter().map(|c| c.cost.total() as f64).sum::<f64>() / nonlands.len() as f64
}

/// Curve bucket label for a CMC ("0".."6", "7+").
fn curve_bucket(cmc: u32) -> &'static str {
    match cmc {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        _ => "7+",
    }
}

/// Reader-facing limits that qualify the report's measurements.
pub(super) fn assumptions(deck: &SimDeck) -> Vec<String> {
    assumptions::for_deck(deck)
}

/// Round to two decimals, the report's standard precision.
fn round2(v: f64) -> f64 {
    super::report_schema::round2(v)
}

/// Convert phase counts to one-based turn keys.
fn phase_turn_values(
    observations: &[super::aggregate::PhaseObservationCounts],
    count: impl Fn(&super::aggregate::PhaseObservationCounts) -> u32,
) -> TurnValues {
    observations
        .iter()
        .enumerate()
        .map(|(index, observation)| ((index + 1) as u32, f64::from(count(observation))))
        .collect()
}

/// Include a turn mean only when at least one finished game completed its phase.
fn observed_turn_values(
    values: &[f64],
    observations: &[super::aggregate::PhaseObservationCounts],
    count: impl Fn(&super::aggregate::PhaseObservationCounts) -> u32,
) -> TurnValues {
    values
        .iter()
        .zip(observations)
        .enumerate()
        .filter(|(_, (_, observation))| count(observation) > 0)
        .map(|(index, (value, _))| ((index + 1) as u32, round2(*value)))
        .collect()
}

/// Include a per-turn percentage only when at least one phase observation exists.
fn observed_percent_turn_values(
    values: &[f64],
    observations: &[super::aggregate::PhaseObservationCounts],
    count: impl Fn(&super::aggregate::PhaseObservationCounts) -> u32,
) -> std::collections::BTreeMap<u32, Percent> {
    values
        .iter()
        .zip(observations)
        .enumerate()
        .filter(|(_, (_, observation))| count(observation) > 0)
        .map(|(index, (share, _))| ((index + 1) as u32, Percent::from_share(*share)))
        .collect()
}

/// Turn-indexed shares converted to public percentages at the report boundary.
fn percent_turn_values(values: &[f64], games: u32) -> std::collections::BTreeMap<u32, Percent> {
    values
        .iter()
        .enumerate()
        .filter(|_| games > 0)
        .map(|(index, share)| ((index + 1) as u32, Percent::from_share(*share)))
        .collect()
}

/// Convert the internal five-color array to the report's named color fields.
fn color_values(values: [f64; 5]) -> ColorValues {
    ColorValues {
        white: values[0],
        blue: values[1],
        black: values[2],
        red: values[3],
        green: values[4],
    }
}

/// Convert internal color shortage shares to public percentages.
fn color_percents(values: [f64; 5], games: u32) -> ColorPercents {
    ColorPercents {
        white: (games > 0).then(|| Percent::from_share(values[0])),
        blue: (games > 0).then(|| Percent::from_share(values[1])),
        black: (games > 0).then(|| Percent::from_share(values[2])),
        red: (games > 0).then(|| Percent::from_share(values[3])),
        green: (games > 0).then(|| Percent::from_share(values[4])),
    }
}

/// One-line summary of the run.
fn summary_line(stats: &SimStats, deck: &SimDeck, findings: &[super::findings::Finding]) -> String {
    if stats.finished_games == 0 {
        let horizon = if deck
            .commanders
            .first()
            .is_some_and(|commander| commander.cost.total().max(1) > stats.turns)
        {
            " · commander curve turn outside horizon"
        } else {
            ""
        };
        return format!(
            "{} requested games · no finished games · performance unavailable{horizon}",
            stats.runs
        );
    }
    let cmd_part = deck.commanders.first().map(|cmd| {
        let turn = cmd.cost.total().max(1) as usize;
        if turn > stats.turns as usize {
            " · commander curve turn outside horizon".to_string()
        } else {
            let by = stats.commander_castable_by[turn];
            format!(" · commander payable on curve {:.0}%", by * 100.0)
        }
    });
    let base = format!(
        "{} games · {} finding{}",
        stats.runs,
        findings.len(),
        if findings.len() == 1 { "" } else { "s" }
    );
    match cmd_part {
        Some(part) => format!("{base}{part}"),
        None => base,
    }
}

/// Bench-section counts for the JSON `deck_shape` block.
#[derive(Debug, Clone, Copy, Default)]
pub struct BenchCounts {
    /// SIDEBOARD copies excluded from the simulated library.
    pub sideboard_cards: i64,
    /// MAYBEBOARD copies excluded from the simulated library.
    pub maybeboard_cards: i64,
}

/// Attach the exact external settings used by the run coordinator.
pub fn set_opponent_model(
    report: &mut SimReport,
    format: super::model::Format,
    parameters: super::opponents::OpponentConfig,
    stats: &SimStats,
) {
    report.opponent_model = Some(inputs::opponent_scenario(format, parameters, stats));
}

/// Build the JSON payload for the report.
pub fn json_report(
    stats: &SimStats,
    deck: &SimDeck,
    name: &str,
    seed: u64,
    findings: &[super::findings::Finding],
    bench: BenchCounts,
    mana_base: &super::findings::ManaBase,
) -> SimReport {
    let turns = stats.turns as usize;
    let lands = deck
        .library_cards()
        .filter(|c| c.role == Role::Land)
        .count() as i64;
    let artifact_mana_sources = deck
        .library_cards()
        .filter(|c| c.role == Role::Rock)
        .count() as i64;
    let creature_mana_sources = deck
        .library_cards()
        .filter(|c| c.role == Role::Dork)
        .count() as i64;
    let ramp = deck
        .library_cards()
        .filter(|c| c.role == Role::RampSpell)
        .count() as i64;
    let curve = ["0", "1", "2", "3", "4", "5", "6", "7+"]
        .into_iter()
        .map(|bucket| {
            let count = deck
                .library_cards()
                .filter(|c| c.role != Role::Land && curve_bucket(c.cost.total()) == bucket)
                .count() as i64;
            (bucket.to_string(), count)
        })
        .collect();
    let commander = deck.commanders.first().map(|cmd| {
        let cmc = cmd.cost.total() as usize;
        // "On curve" only means something when the sim ran long enough
        // to reach the commander's curve turn; a clamped short run
        // would read as a misleading 0%. Null past the horizon.
        let curve_turn = cmc.max(1);
        let on_curve = (stats.finished_games > 0 && curve_turn <= turns)
            .then(|| Percent::from_share(stats.commander_castable_by[curve_turn]));
        CommanderReport {
            name: cmd.name.clone(),
            mana_value: cmd.cost.total() as f64,
            percent_castable_by_turn: percent_turn_values(
                &stats.commander_castable_by[1..=turns],
                stats.finished_games,
            ),
            avg_first_cast_turn: (stats.commander_payable_games > 0)
                .then(|| round2(stats.avg_commander_cast_turn)),
            p50_cast_turn: (stats.commander_payable_games > 0)
                .then_some(stats.p50_commander_cast_turn),
            p95_cast_turn: (stats.commander_payable_games > 0)
                .then_some(stats.p95_commander_cast_turn),
            payable_games: stats.commander_payable_games,
            percent_castable_by_curve: on_curve,
        }
    });
    let report_findings: Vec<Finding> = findings
        .iter()
        .map(|p| Finding {
            kind: p.kind.to_string(),
            identity: p.color.map_or_else(
                || p.kind.to_string(),
                |color| format!("{}:{}", p.kind, color.symbol()),
            ),
            severity: p.severity.to_string(),
            percent_of_games: p.game_share.map(Percent::from_share),
            color: p.color.map(|color| color.symbol().to_string()),
            explanation: p.explanation.clone(),
            suggestion: p.suggestion.clone(),
            evidence: p
                .evidence
                .iter()
                .map(|item| FindingEvidence {
                    subject: item.name.clone(),
                    explanation: item.explanation.clone(),
                    percent_of_games: item.game_share.map(Percent::from_share),
                })
                .collect(),
        })
        .collect();
    let station = if deck
        .commanders
        .first()
        .is_some_and(|cmd| cmd.animate_at().is_some())
    {
        Some(StationReport {
            online_by_t6: fixed_percent(stats, 6, stats.station_online_pct),
            p50_online_turn: (stats.station_online_games > 0).then_some(stats.station_p50_turn),
            online_games: stats.station_online_games,
        })
    } else {
        None
    };
    let companion = if deck.companion.is_some() {
        Some(CompanionReport {
            in_hand_by_t6: fixed_percent(stats, 6, stats.companion_online_pct),
            p50_online_turn: (stats.companion_online_games > 0).then_some(stats.companion_p50_turn),
            in_hand_games: stats.companion_online_games,
        })
    } else {
        None
    };
    SimReport {
        name: name.to_string(),
        format: deck.rules.key.to_string(),
        runs: stats.runs,
        outcomes: RunOutcomes {
            requested_games: stats.runs,
            finished_games: stats.finished_games,
            player_loss_games: stats.player_loss_games,
            incomplete_games: stats.incomplete_games,
            incomplete_reasons: stats.incomplete_reasons.clone(),
            incomplete_observations: stats.incomplete_observations.iter().map(|((phase, turn, reason), games)| IncompleteObservation { reason: *reason, turn: *turn, phase: *phase, games: *games }).collect(),
        },
        phase_observations: PhaseObservationMeasures {
            draw_by_turn: phase_turn_values(&stats.phase_observations, |counts| counts.draw),
            main_by_turn: phase_turn_values(&stats.phase_observations, |counts| counts.main),
            combat_by_turn: phase_turn_values(&stats.phase_observations, |counts| counts.combat),
            end_step_by_turn: phase_turn_values(&stats.phase_observations, |counts| counts.end_step),
            end_turn_by_turn: phase_turn_values(&stats.phase_observations, |counts| counts.end_turn),
        },
        turns: stats.turns,
        seed,
        provenance: inputs::provenance(deck),
        opponent_model: None,
        sampling: sampling::rates(stats, deck),
        card_support: inputs::card_support(deck),
        deck_shape: DeckShape {
            total_cards: total_cards(deck),
            average_nonland_mana_value: round2(avg_cmc(deck)),
            sideboard_cards: bench.sideboard_cards,
            maybeboard_cards: bench.maybeboard_cards,
            lands,
            artifact_mana_sources,
            creature_mana_sources,
            ramp_spells: ramp,
            draw_sources: stats.draw_count as i64,
            removal_spells: stats.removal_count as i64,
            targeted_removal_spells: stats.removal_targeted as i64,
            mass_removal_spells: stats.removal_wipes as i64,
            win_conditions: stats.wincon_count as i64,
            locks: lock_count(deck),
            boosters: booster_count(deck),
            curve,
        },
        assumptions: assumptions(deck),
        opening_hand: OpeningHand {
            avg_lands: (stats.finished_games > 0).then(|| round2(stats.avg_opener_lands)),
            percent_0_lands: fixed_percent(stats, 1, stats.opener_pct[0]),
            percent_1_land: fixed_percent(stats, 1, stats.opener_pct[1]),
            percent_2_lands: fixed_percent(stats, 1, stats.opener_pct[2]),
            percent_3_lands: fixed_percent(stats, 1, stats.opener_pct[3]),
            percent_4_lands: fixed_percent(stats, 1, stats.opener_pct[4]),
            percent_five_or_more_lands: fixed_percent(stats, 1, stats.opener_pct[5]),
            percent_mulliganed: fixed_percent(stats, 1, stats.mulligan_rate),
        },
        land_drops: LandDrops {
            percent_games_hitting_all_land_drops_by_turn:
                percent_turn_values(&stats.hit_all_drops_by[1..=turns.min(5)], stats.finished_games),
            percent_games_with_two_or_fewer_lands_by_turn_4:
                fixed_percent(stats, 4, stats.screw_pct),
            percent_games_with_six_or_more_lands_by_turn_4:
                fixed_percent(stats, 4, stats.flood_pct),
            expected_percent_with_six_or_more_lands_by_turn_4:
                fixed_percent(stats, 4, stats.flood_expectation),
            p50_drops_by_4: (stats.finished_games > 0 && turns >= 4).then_some(stats.p50_drops_by_4),
            p95_drops_by_4: (stats.finished_games > 0 && turns >= 4).then_some(stats.p95_drops_by_4),
        },
        mana_base: mana_base.clone(),
        commander,
        station,
        companion,
        creatures_by_turn: observed_turn_values(
            &stats.creatures_by_turn[..turns],
            &stats.phase_observations[..turns],
            |counts| counts.end_turn,
        ),
        repeatable_sources_by_turn: observed_turn_values(
            &stats.repeatable_sources_by_turn[..turns],
            &stats.phase_observations[..turns],
            |counts| counts.end_turn,
        ),
        mana: ManaMeasures {
            avg_available_by_turn: observed_turn_values(&stats.mana_available[..turns], &stats.phase_observations[..turns], |counts| counts.main),
            avg_cast_spent_by_turn: observed_turn_values(&stats.mana_spent_on_casts[..turns], &stats.phase_observations[..turns], |counts| counts.main),
            avg_unused_by_turn: observed_turn_values(&stats.unused_mana[..turns], &stats.phase_observations[..turns], |counts| counts.main),
            percent_games_with_three_or_more_unused_mana_by_turn_6:
                fixed_percent(stats, 6, stats.floated_pct),
        },
        draw: DrawMeasures {
            draw_source_percent_seen_by_turn: percent_turn_values(&stats.draw_sources[..turns], stats.finished_games),
            percent_games_with_no_draw_source_by_turn_6: fixed_percent(stats, 6, stats.starved_pct),
            avg_life_paid: (stats.finished_games > 0).then(|| round2(stats.life_paid_avg)),
            avg_life_gained: (stats.finished_games > 0).then(|| round2(stats.life_gained_avg)),
            avg_life_funded_draws: (stats.finished_games > 0).then(|| round2(stats.life_funded_draws_avg)),
        },
        role_access: RoleAccess {
            removal_spell_percent_seen_by_turn_5: fixed_percent(stats, 5, stats.removal_access_5),
            draw_source_percent_seen_by_turn_6: fixed_percent(stats, 6, stats.draw_access_6),
            creature_role_percent_seen_by_turn_3: fixed_percent(stats, 3, stats.creature_access_3),
            win_condition_role_percent_seen_by_turn_8: fixed_percent(stats, 8, stats.wincon_access_8),
            lock_role_percent_seen_by_turn_3: fixed_percent(stats, 3, stats.lock_access_3),
        },
        velocity: VelocityMeasures {
            avg_cards_seen: observed_turn_values(&stats.cards_seen[..turns], &stats.phase_observations[..turns], |counts| counts.end_turn),
            library_awareness_by_turn: observed_percent_turn_values(&stats.library_awareness_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.end_turn),
            self_milled_by_turn: observed_turn_values(&stats.self_milled_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.end_turn),
            opponent_milled_by_turn: observed_turn_values(&stats.opp_milled_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.end_turn),
            library_remaining_by_turn: observed_turn_values(&stats.library_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.end_turn),
        },
        combat: CombatMeasures {
            attack_power_avg_by_turn: observed_turn_values(&stats.attack_power_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.combat),
            player_damage_avg_by_turn: observed_turn_values(&stats.player_damage_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.end_turn),
            attack_power_p90_by_t8: (stats.finished_games > 0 && turns >= 8 && stats.phase_observations[7].combat > 0).then_some(f64::from(stats.attack_power_p90)),
            attackers_by_turn: observed_turn_values(&stats.attackers_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.combat),
            evasive_by_turn: observed_turn_values(&stats.evasive_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.combat),
            commander_potential: deck
                .commanders
                .iter()
                .enumerate()
                .map(|(slot, commander)| CommanderCombatPotential {
                    slot: slot as u32,
                    name: commander.name.clone(),
                    damage_by_turn: observed_turn_values(&stats.commander_combat_by_turn[slot], &stats.phase_observations, |counts| counts.combat),
                    cumulative_damage_by_turn: observed_turn_values(&stats.commander_combat_cumulative_by_turn[slot], &stats.phase_observations, |counts| counts.combat),
                    percent_games_reaching_21: (stats.finished_games > 0).then(|| {
                        Percent::from_share(
                            f64::from(stats.commander_combat_threshold_games[slot])
                                / f64::from(stats.finished_games),
                        )
                    }),
                    observed_games: stats.finished_games,
                    games_reaching_21: stats.commander_combat_threshold_games[slot],
                    median_turn_to_21: stats.commander_combat_threshold_p50[slot],
                    note: "Potential only. Repeated attacks may not reach the same opponent. This is not a kill or win probability.".to_string(),
                })
                .collect(),
        },
        win_conditions: WinMeasures {
            opponent_life_loss_by_turn: observed_turn_values(&stats.opponent_life_loss_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.end_turn),
            percent_games_with_extra_turn: fixed_percent(stats, 1, stats.extra_turns_pct),
            win_threshold_p50_turn: (stats.win_threshold_games > 0).then_some(stats.win_threshold_p50_turn),
            counter_win_threshold_games: stats.win_threshold_games,
            percent_games_reaching_counter_win_threshold:
                fixed_percent(stats, 1, stats.win_threshold_pct),
            percent_games_with_affordable_ultimate: fixed_percent(stats, 1, stats.ultimate_online_pct),
            percent_games_with_suspected_infinite_mana:
                (stats.runs > 0).then(|| Percent::from_share(stats.infinite_mana_pct)),
            percent_games_at_or_above_table_life_by_turn:
                percent_turn_values(&stats.lethal_damage_by_turn[..turns], stats.finished_games),
            p50_lethal_turn: stats.p50_lethal_turn,
            damage_threshold_games: stats.lethal_games,
            lethal_note: "Declared-scenario policy estimate. The median turn is conditional on games that reach the damage threshold; it is not a win-rate estimate or a guaranteed upper bound.".to_string(),
        },
        interaction: InteractionMeasures {
            percent_games_with_ready_interaction_by_turn:
                percent_turn_values(&stats.interaction_ready_by_turn[..turns], stats.finished_games),
            mana_held_avg: (stats.interaction_ready_turns > 0).then(|| round2(stats.interaction_mana_held)),
            ready_turn_observations: stats.interaction_ready_turns,
            instant_speed_count: stats.interaction_instant_count,
            note: "capacity, not events".to_string(),
        },
        color_mana_shortage: color_percents(stats.color_screw, stats.finished_games),
        color_pip_blocks: stats
            .pip_blocks
            .iter()
            .map(|block| ColorPipBlock {
                name: block.name.clone(),
                color: block.color.symbol().to_string(),
                percent_of_games: Percent::from_share(block.game_share),
            })
            .collect(),
        graveyard: GraveyardMeasures {
            avg_size_by_turn: observed_turn_values(&stats.graveyard_by_turn[..turns.min(stats.graveyard_by_turn.len())], &stats.phase_observations[..turns.min(stats.phase_observations.len())], |counts| counts.end_turn),
            replay_casts_avg: (stats.finished_games > 0).then(|| round2(stats.replay_casts_avg)),
        },
        milestones: Milestones {
            free_cast_permanents_entered_avg_by_turn:
                observed_turn_values(&stats.free_cast_permanents_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.main),
            dredge_uses_avg_by_turn: observed_turn_values(&stats.dredge_uses_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.main),
            graveyard_casts_avg_by_turn: observed_turn_values(&stats.graveyard_casts_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.main),
            life_funded_draws_avg_by_turn: observed_turn_values(&stats.life_funded_draws_by_turn[..turns], &stats.phase_observations[..turns], |counts| counts.main),
            percent_games_with_positive_mana_loop_by_turn:
                percent_turn_values(&stats.positive_mana_loop_by_turn[..turns], stats.runs),
            note: "Action counts are events in that turn. Positive mana loop is the share of games that reached one during that turn.".to_string(),
        },
        color_sources: color_source_census(deck),
        card_castability: stats
            .card_castability
            .iter()
            .map(|card| CardCastability {
                name: card.name.clone(),
                mana_value: round2(card.cmc),
                target_turn: card.target_turn,
                percent_castable_by_target: fixed_percent(stats, card.target_turn, card.pct_by_target),
                avg_first_castable_turn: (card.payable_games > 0).then(|| round2(card.avg_first_castable_turn)),
                payable_games: card.payable_games,
            })
            .collect(),
        findings: report_findings,
        summary: summary_line(stats, deck, findings),
        combo_access: None,
        colored_sources: None,
        combos: None,
        win_paths: None,
        hypgeo: None,
    }
}

/// Per-color mana-source census over the deck's lands and rocks.
///
/// Static analysis (no simulation): for each WUBRG color, how many
/// sources can produce it and by which shape (fixed pips, choice,
/// any-color). The `fixing` array counts sources producing 2+ colors.
fn color_source_census(deck: &SimDeck) -> ColorSourceCensus {
    use super::model::Role;
    let mut fixed = [0u32; 5];
    let mut choice = [0u32; 5];
    let mut flexible_nonland = 0u32;
    let mut scaling = 0u32;
    let commander_colors = deck.commander_color_identity();
    for card in deck.library_cards() {
        if card.role == Role::Land {
            let Some(y) = &card.tap else { continue };
            for (i, p) in y.fixed.iter().enumerate() {
                if *p > 0 {
                    fixed[i] += 1;
                }
            }
            if y.any_pips > 0 && !y.opponent_any {
                for c in choice.iter_mut() {
                    *c += 1;
                }
            } else if !y.opponent_any {
                let eligible: [bool; 5] = std::array::from_fn(|index| {
                    y.choice[index] || y.commander_pips > 0 && commander_colors[index]
                });
                let colors = eligible.iter().filter(|c| **c).count();
                if colors >= 2 {
                    for (i, c) in eligible.iter().enumerate() {
                        if *c {
                            choice[i] += 1;
                        }
                    }
                } else if colors == 1
                    && let Some(i) = eligible.iter().position(|c| *c)
                {
                    fixed[i] += 1;
                }
            }
        } else if card.role == Role::Rock || card.role == Role::Dork {
            // Non-land mana sources: flexible output per turn (rocks tap
            // for one; dorks join on the first body turn).
            if let Some(y) = &card.tap
                && (y.any_pips > 0
                    || y.commander_pips > 0 && commander_colors.iter().any(|color| *color)
                    || y.choice.iter().any(|c| *c)
                    || y.fixed.iter().any(|p| *p > 0))
            {
                flexible_nonland += 1;
            }
        }
        if card.tap.as_ref().is_some_and(|y| y.scaling.is_some()) {
            scaling += 1;
        }
    }
    ColorSourceCensus {
        fixed_source_lands: color_values(fixed.map(f64::from)),
        choice_source_lands: color_values(choice.map(f64::from)),
        flexible_nonland_sources: flexible_nonland,
        scaling_sources: scaling,
        note: "Static land tap-yield census, not a mana-readiness guarantee. Choice lands count for each known eligible color. Unknown opponent colors do not count as colored sources. Flexible nonland sources count rocks and dorks.".to_string(),
    }
}

/// Human report rendering.
mod render;
pub use render::print_report;
