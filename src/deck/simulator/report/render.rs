//! Human summaries for simulator observations.

use super::*;

/// Show consistency measurements with their observation counts and model limits.
pub fn print_report(out: &Output, report: &SimReport) {
    print_overview(out, report);
    if report.outcomes.finished_games == 0 {
        out.print_wrapped("  Rates, averages, and timings are unavailable: no finished games.");
        return;
    }
    print_findings(out, report);
    let turn = report.turns.clamp(1, 6);
    println!();
    out.print_wrapped(&out.styles().header("Opening and mana"));
    print_opening(out, report);
    print_land_drops(out, report);
    print_commander(out, report);
    print_companion(out, report);
    print_turn_means(out, report, turn);
    print_access(out, report);
    print_combat(out, report, turn);
    print_thresholds(out, report, turn);
    print_mana_readiness(out, report);
}

/// State the run size, rate denominator, and limits before any measurements.
fn print_overview(out: &Output, report: &SimReport) {
    let s = out.styles();
    out.print_wrapped(&format!(
        "{}  {}",
        s.header(&report.name),
        s.dim(&format!(
            "{} games · up to {} turns per game",
            s.thousands(i64::from(report.runs)),
            report.turns
        )),
    ));
    out.print_wrapped(&format!(
        "  {} cards · {} lands · {} ramp sources · average nonland mana value {:.1}",
        report.deck_shape.total_cards,
        report.deck_shape.lands,
        report.deck_shape.artifact_mana_sources
            + report.deck_shape.creature_mana_sources
            + report.deck_shape.ramp_spells,
        report.deck_shape.average_nonland_mana_value,
    ));
    out.print_wrapped(&format!(
        "  Outcomes: {} finished, including {} player losses · {} incomplete of {} requested",
        report.outcomes.finished_games,
        report.outcomes.player_loss_games,
        report.outcomes.incomplete_games,
        report.outcomes.requested_games,
    ));
    out.print_wrapped("  Finished games reached the turn limit or a supported player loss.");
    if report.outcomes.incomplete_games > 0 {
        out.print_wrapped("  Incomplete games stopped at a safety limit. Primary estimates exclude them. This can bias results, especially for engine-heavy decks.");
    }
    out.print_wrapped(&format!(
        "  Unless noted, rates use all {} finished games. Averages use games that completed the stated phase.",
        report.outcomes.finished_games,
    ));
    out.print_wrapped("  Goldfish consistency, not win rates: opponents do not block, counter spells, or cast removal.");
    out.print_wrapped("  Some abilities are unsupported and may have no effect. See --json for card support, assumptions, the opponent scenario, and all metrics.");
}

/// Describe the kept opening hand without treating a missing value as zero.
fn print_opening(out: &Output, report: &SimReport) {
    let s = out.styles();
    let Some(lands) = report.opening_hand.avg_lands else {
        out.print_wrapped("  Opening hand unavailable: no observations.");
        return;
    };
    out.print_wrapped(&format!(
        "  {}  {}  {:.1} lands on average · mulligan rate {}",
        s.bar(lands / 7.0, 10),
        s.dim("kept opening hand"),
        lands,
        percent_text(report.opening_hand.percent_mulliganed),
    ));
}

/// Separate land plays from land access and label target counts as guidance.
fn print_land_drops(out: &Output, report: &SimReport) {
    if report.turns < 4 {
        return;
    }
    let s = out.styles();
    let all_drops = report
        .land_drops
        .percent_games_hitting_all_land_drops_by_turn
        .get(&4)
        .copied();
    out.print_wrapped(&format!(
        "  {}  {}  at least 4 drops {} · at most 2 drops {} · 6+ lands seen {}",
        s.bar(all_drops.map_or(0.0, Percent::value) / 100.0, 10),
        s.dim("land drops by turn 4"),
        percent_text(all_drops),
        percent_text(
            report
                .land_drops
                .percent_games_with_two_or_fewer_lands_by_turn_4
        ),
        percent_text(
            report
                .land_drops
                .percent_games_with_six_or_more_lands_by_turn_4
        ),
    ));
    let ramp = report.mana_base.artifact_mana_sources
        + report.mana_base.creature_mana_sources
        + report.mana_base.ramp_spells;
    let target = match report.mana_base.bracket {
        Some(bracket) => format!(
            "{}-{} lands + {}-{} ramp sources for bracket {}",
            report.mana_base.bracket_target_lands[0],
            report.mana_base.bracket_target_lands[1],
            report.mana_base.bracket_target_ramp[0],
            report.mana_base.bracket_target_ramp[1],
            bracket,
        ),
        None => format!(
            "{}-{} lands",
            report.mana_base.bracket_target_lands[0], report.mana_base.bracket_target_lands[1],
        ),
    };
    out.print_wrapped(&format!(
        "  Mana base: {} lands · {} ramp sources · reference range {} · {}",
        report.mana_base.lands, ramp, target, report.mana_base.verdict,
    ));
}

/// Keep commander payment readiness separate from actual casts and conditional timing.
fn print_commander(out: &Output, report: &SimReport) {
    let Some(cmd) = &report.commander else {
        return;
    };
    let s = out.styles();
    let mana_value = (cmd.mana_value as u32).max(1);
    let curve = cmd.percent_castable_by_curve.map_or_else(
        || {
            if mana_value > report.turns {
                format!("turn {mana_value} is beyond the turn limit")
            } else {
                "unavailable (no observations)".to_string()
            }
        },
        |percent| format!("{:.2}% payable by turn {mana_value}", percent.value()),
    );
    out.print_wrapped(&format!(
        "  {}  {}  {} · {}",
        s.bar(
            cmd.percent_castable_by_curve.map_or(0.0, Percent::value) / 100.0,
            10
        ),
        s.dim("commander mana readiness"),
        s.card_name(&cmd.name),
        curve,
    ));
    out.print_wrapped(&format!(
        "    First payable turn: median {} · 95th percentile {} among {} payable games over the run. Payable does not mean cast.",
        timing(cmd.p50_cast_turn),
        timing(cmd.p95_cast_turn),
        cmd.payable_games,
    ));
    if report.turns >= 6
        && let Some(station) = &report.station
    {
        out.print_wrapped(&format!(
            "  {}  {}  {} by turn 6 · median first active turn {} among {} games over the run",
            s.bar(station.online_by_t6.map_or(0.0, Percent::value) / 100.0, 10),
            s.dim("station active"),
            percent_text(station.online_by_t6),
            timing(station.p50_online_turn),
            station.online_games,
        ));
    }
}

/// Distinguish companion arrival in hand from a companion cast.
fn print_companion(out: &Output, report: &SimReport) {
    let Some(companion) = &report.companion else {
        return;
    };
    if report.turns < 6 {
        return;
    }
    let s = out.styles();
    out.print_wrapped(&format!(
        "  {}  {}  {} by turn 6 · median first arrival turn {} among {} games over the run",
        s.bar(
            companion.in_hand_by_t6.map_or(0.0, Percent::value) / 100.0,
            10
        ),
        s.dim("companion in hand"),
        percent_text(companion.in_hand_by_t6),
        timing(companion.p50_online_turn),
        companion.in_hand_games,
    ));
}

/// Give each turn average its phase count and explain the cards-seen measure.
fn print_turn_means(out: &Output, report: &SimReport, turn: u32) {
    println!();
    out.print_wrapped(&out.styles().header(&format!("Turn {turn} averages")));
    out.print_wrapped(&format!(
        "  Unspent pool mana after main actions: {} · {}",
        mean_text(report.mana.avg_unused_by_turn.get(&turn).copied()),
        observation_count(&report.phase_observations.main_by_turn, turn),
    ));
    out.print_wrapped(&format!(
        "  Cards seen: {} · creatures: {} · {} completing the turn",
        mean_text(report.velocity.avg_cards_seen.get(&turn).copied()),
        mean_text(report.creatures_by_turn.get(&turn).copied()),
        observation_count(&report.phase_observations.end_turn_by_turn, turn),
    ));
    out.print_wrapped("  Unspent pool mana excludes untapped sources and unspent Treasures.");
    out.print_wrapped("  Cards seen includes the kept hand, draws, cards milled from your library, searches, and modeled reveals. Repeat visits can count again. This is not hand size.");
}

/// Explain card access and payable interaction without implying resolved effects.
fn print_access(out: &Output, report: &SimReport) {
    if report.turns < 5 {
        return;
    }
    let s = out.styles();
    println!();
    out.print_wrapped(&s.header("Card access and interaction"));
    let removal = report.role_access.removal_spell_percent_seen_by_turn_5;
    out.print_wrapped(&format!(
        "  {}  {}  {}",
        s.bar(removal.map_or(0.0, Percent::value) / 100.0, 10),
        s.dim("removal card seen by turn 5"),
        percent_text(removal),
    ));
    if report.turns >= 6 {
        let draw = report.role_access.draw_source_percent_seen_by_turn_6;
        out.print_wrapped(&format!(
            "  {}  {}  {} · no draw card seen {}",
            s.bar(draw.map_or(0.0, Percent::value) / 100.0, 10),
            s.dim("draw card seen by turn 6"),
            percent_text(draw),
            percent_text(report.draw.percent_games_with_no_draw_source_by_turn_6),
        ));
    }
    out.print_wrapped(
        "  Seeing a card does not show that a draw engine was active or a removal spell resolved.",
    );
    if report.interaction.instant_speed_count == 0 {
        out.print_wrapped(
            "  Interaction readiness does not apply: no cards classified as instant-speed answers.",
        );
        return;
    }
    let ready = report
        .interaction
        .percent_games_with_ready_interaction_by_turn
        .get(&5)
        .copied();
    out.print_wrapped(&format!(
        "  {}  {}  {} · {} instant-speed answers in the deck",
        s.bar(ready.map_or(0.0, Percent::value) / 100.0, 10),
        s.dim("answer in hand and payable at turn 5"),
        percent_text(ready),
        report.interaction.instant_speed_count,
    ));
    out.print_wrapped(&format!(
        "    Average mana left after the cheapest payable answer: {} across {} ready-turn observations over the run. This is a hypothetical payment, not a decision to hold mana.",
        mean_text(report.interaction.mana_held_avg),
        report.interaction.ready_turn_observations,
    ));
}

/// Label combat as potential and keep each commander's contribution separate.
fn print_combat(out: &Output, report: &SimReport, turn: u32) {
    let s = out.styles();
    println!();
    out.print_wrapped(&s.header("Potential combat and thresholds"));
    let Some(attack_power) = report.combat.attack_power_avg_by_turn.get(&turn) else {
        out.print_wrapped(&format!(
            "  Turn {turn} combat unavailable: no finished games completed combat."
        ));
        return;
    };
    let high_power = report.combat.attack_power_p90_by_t8.map_or_else(
        || {
            if report.turns < 8 {
                "beyond the turn limit".to_string()
            } else {
                "unavailable (no combat observations)".to_string()
            }
        },
        |power| format!("{power:.0}"),
    );
    out.print_wrapped(&format!(
        "  {}  Turn {}: {:.0} average attack power · {} completing combat",
        s.bar((attack_power / 40.0).min(1.0), 10),
        turn,
        attack_power,
        observation_count(&report.phase_observations.combat_by_turn, turn),
    ));
    out.print_wrapped(&format!(
        "    Average evasive attackers: {} · 90th percentile attack power at turn 8: {}",
        mean_text(report.combat.evasive_by_turn.get(&turn).copied()),
        high_power,
    ));
    for row in &report.combat.commander_potential {
        let Some(cumulative) = row
            .cumulative_damage_by_turn
            .get(&turn)
            .filter(|v| **v > 0.0)
        else {
            continue;
        };
        out.print_wrapped(&format!(
            "  Commander combat: {} · {:.1} average cumulative potential damage by turn {} · {} reach 21 by turn {} ({} of {} finished games)",
            s.card_name(&row.name),
            cumulative,
            turn,
            percent_text(row.percent_games_reaching_21),
            report.turns,
            row.games_reaching_21,
            row.observed_games,
        ));
    }
    out.print_wrapped("  Combat assumes attacks are unblocked. Potential combat damage does not reduce opponent life. Each commander counts separately; reaching 21 does not prove an opponent loss.");
}

/// Separate modeled life loss and conditional thresholds from game wins.
fn print_thresholds(out: &Output, report: &SimReport, turn: u32) {
    let s = out.styles();
    if let Some(life_loss) = report
        .win_conditions
        .opponent_life_loss_by_turn
        .get(&turn)
        .filter(|loss| **loss > 0.0)
    {
        out.print_wrapped(&format!(
            "  {}  Opponent life lost to effects by turn {}: {:.1} on average · {} completing the turn",
            s.bar((life_loss / 30.0).min(1.0), 10),
            turn,
            life_loss,
            observation_count(&report.phase_observations.end_turn_by_turn, turn),
        ));
    }
    if let Some(p50) = report.win_conditions.p50_lethal_turn {
        out.print_wrapped(&format!(
            "  {}  Table damage threshold: median turn {} among {} of {} finished games reaching it by turn {}",
            s.bar(f64::from(report.win_conditions.damage_threshold_games) / f64::from(report.outcomes.finished_games).max(1.0), 10),
            p50,
            report.win_conditions.damage_threshold_games,
            report.outcomes.finished_games,
            report.turns,
        ));
    } else {
        out.print_wrapped(&format!(
            "  Table damage threshold: not reached in finished games within {} turns.",
            report.turns,
        ));
    }
    out.print_wrapped("  The table damage threshold compares modeled damage and direct life loss with starting table life. It does not prove a legal win.");
    if let Some(percent) = report
        .win_conditions
        .percent_games_with_extra_turn
        .filter(|p| p.value() > 0.0)
    {
        out.print_wrapped(&format!(
            "  {}  Games taking an extra turn: {:.2}%",
            s.bar(percent.value() / 100.0, 10),
            percent.value(),
        ));
    }
    if let Some(percent) = report
        .win_conditions
        .percent_games_reaching_counter_win_threshold
        .filter(|p| p.value() > 0.0)
    {
        out.print_wrapped(&format!(
            "  {}  Counter-based win threshold: {:.2}% by turn {} · median turn {} among {} games reaching it; not a declared win",
            s.bar(percent.value() / 100.0, 10),
            percent.value(),
            report.turns,
            timing(report.win_conditions.win_threshold_p50_turn),
            report.win_conditions.counter_win_threshold_games,
        ));
    }
    if let Some(percent) = report
        .win_conditions
        .percent_games_with_affordable_ultimate
        .filter(|p| p.value() > 0.0)
    {
        out.print_wrapped(&format!(
            "  {}  Planeswalker ultimate affordable: {:.2}% by turn {}; this does not show that it resolved",
            s.bar(percent.value() / 100.0, 10),
            percent.value(),
            report.turns,
        ));
    }
}

/// Show the lowest payment readiness and observed color gaps without assuming hand access.
fn print_mana_readiness(out: &Output, report: &SimReport) {
    let s = out.styles();
    let mut worst: Vec<&CardCastability> = report
        .card_castability
        .iter()
        .filter(|card| card.percent_castable_by_target.is_some())
        .collect();
    worst.sort_by(|a, b| {
        a.percent_castable_by_target
            .partial_cmp(&b.percent_castable_by_target)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.name.cmp(&b.name))
    });
    worst.truncate(3);
    if !worst.is_empty() {
        println!();
        out.print_wrapped(&s.header("Mana readiness (lowest 3)"));
        out.print_wrapped("  Enough modeled mana does not mean the card was drawn, had a legal target, or was cast.");
        for card in worst {
            out.print_wrapped(&format!(
                "    {}  {}  payable by turn {} in {} of finished games",
                s.card_name(&card.name),
                s.dim(&format!("mana value {:.0}", card.mana_value)),
                card.target_turn,
                percent_text(card.percent_castable_by_target),
            ));
        }
    }
    if [
        report.color_mana_shortage.white,
        report.color_mana_shortage.blue,
        report.color_mana_shortage.black,
        report.color_mana_shortage.red,
        report.color_mana_shortage.green,
    ]
    .iter()
    .flatten()
    .any(|p| p.value() >= 10.0)
        && !report.color_pip_blocks.is_empty()
    {
        println!();
        out.print_wrapped(&s.header("Color requirements that blocked payment"));
        for p in report.color_pip_blocks.iter().take(3) {
            out.print_wrapped(&format!(
                "    {}  {} mana symbols could not be paid in {:.2}% of finished games",
                s.card_name(&p.name),
                p.color,
                p.percent_of_games.value(),
            ));
        }
    }
}

/// Present findings as diagnostic checks, not runtime errors or required edits.
fn print_findings(out: &Output, report: &SimReport) {
    let s = out.styles();
    println!();
    out.print_wrapped(&s.header("Findings"));
    if report.findings.is_empty() {
        out.print_wrapped(
            "  No findings under the current checks. This does not verify card support.",
        );
        return;
    }
    out.print_wrapped("  Use these findings to choose a next check, not an automatic deck edit.");
    for p in &report.findings {
        out.print_wrapped(&format!(
            "  {} {}",
            s.glyph("finding:", crate::output::GlyphKind::Warn),
            p.explanation,
        ));
        for offender in &p.evidence {
            let share = offender
                .percent_of_games
                .map(|percent| format!(" ({:.2}% of finished games)", percent.value()))
                .unwrap_or_default();
            out.print_wrapped(&format!(
                "    {} {}{}",
                s.card_name(&offender.subject),
                offender.explanation,
                share,
            ));
        }
        out.print_wrapped(&format!("    Next check: {}", p.suggestion));
    }
}

/// Format an available percentage without substituting zero for missing data.
fn percent_text(percent: Option<Percent>) -> String {
    percent.map_or_else(
        || "unavailable (no observations)".to_string(),
        |percent| format!("{:.2}%", percent.value()),
    )
}

/// Keep missing averages distinct from measured zeroes.
fn mean_text(mean: Option<f64>) -> String {
    mean.map_or_else(
        || "unavailable (no observations)".to_string(),
        |mean| format!("{mean:.1}"),
    )
}

/// Keep conditional timings empty when no game reached the event.
fn timing(turn: Option<u32>) -> String {
    turn.map_or_else(|| "not observed".to_string(), |turn| turn.to_string())
}

/// Give the number of games used for a per-phase average.
fn observation_count(counts: &TurnValues, turn: u32) -> String {
    counts.get(&turn).map_or_else(
        || "observation count unavailable".to_string(),
        |games| format!("{games:.0} games"),
    )
}
