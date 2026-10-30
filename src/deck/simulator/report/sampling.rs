//! Whole-game event denominators and sampling uncertainty.

use super::*;

/// Keep a fixed-turn percentage unavailable until its whole window is observed.
pub(super) fn fixed_percent(stats: &SimStats, turn: u32, share: f64) -> Option<Percent> {
    (stats.finished_games > 0 && stats.turns >= turn).then(|| Percent::from_share(share))
}

/// Build intervals from exact event counts and the declared game subset.
fn estimate(stats: &SimStats, turn: u32, path: &str) -> RateEstimate {
    let diagnostic = path == "win_conditions.percent_games_with_suspected_infinite_mana"
        || path.starts_with("milestones.percent_games_with_positive_mana_loop_by_turn.");
    let games = if diagnostic {
        stats.runs
    } else {
        stats.finished_games
    };
    if stats.turns < turn {
        RateEstimate::unavailable(ObservationStatus::OutsideHorizon)
    } else if games == 0 {
        RateEstimate::unavailable(ObservationStatus::NoObservations)
    } else {
        RateEstimate::observed(stats.event_games.get(path).copied().unwrap_or(0), games)
    }
}

/// Describe event rates only. Mean counts and mean library fractions are not binomial.
pub(super) fn rates(
    stats: &SimStats,
    deck: &SimDeck,
) -> std::collections::BTreeMap<String, RateEstimate> {
    let mut rates = std::collections::BTreeMap::new();
    for (path, turn, _share) in [
        ("opening_hand.percent_0_lands", 1, stats.opener_pct[0]),
        ("opening_hand.percent_1_land", 1, stats.opener_pct[1]),
        ("opening_hand.percent_2_lands", 1, stats.opener_pct[2]),
        ("opening_hand.percent_3_lands", 1, stats.opener_pct[3]),
        ("opening_hand.percent_4_lands", 1, stats.opener_pct[4]),
        (
            "opening_hand.percent_five_or_more_lands",
            1,
            stats.opener_pct[5],
        ),
        ("opening_hand.percent_mulliganed", 1, stats.mulligan_rate),
        (
            "land_drops.percent_games_with_two_or_fewer_lands_by_turn_4",
            4,
            stats.screw_pct,
        ),
        (
            "land_drops.percent_games_with_six_or_more_lands_by_turn_4",
            4,
            stats.flood_pct,
        ),
        (
            "mana.percent_games_with_three_or_more_unused_mana_by_turn_6",
            6,
            stats.floated_pct,
        ),
        (
            "draw.percent_games_with_no_draw_source_by_turn_6",
            6,
            stats.starved_pct,
        ),
        (
            "role_access.removal_spell_percent_seen_by_turn_5",
            5,
            stats.removal_access_5,
        ),
        (
            "role_access.draw_source_percent_seen_by_turn_6",
            6,
            stats.draw_access_6,
        ),
        (
            "role_access.creature_role_percent_seen_by_turn_3",
            3,
            stats.creature_access_3,
        ),
        (
            "role_access.win_condition_role_percent_seen_by_turn_8",
            8,
            stats.wincon_access_8,
        ),
        (
            "role_access.lock_role_percent_seen_by_turn_3",
            3,
            stats.lock_access_3,
        ),
        (
            "win_conditions.percent_games_with_extra_turn",
            1,
            stats.extra_turns_pct,
        ),
        (
            "win_conditions.percent_games_reaching_counter_win_threshold",
            1,
            stats.win_threshold_pct,
        ),
        (
            "win_conditions.percent_games_with_affordable_ultimate",
            1,
            stats.ultimate_online_pct,
        ),
        (
            "win_conditions.percent_games_with_suspected_infinite_mana",
            1,
            stats.infinite_mana_pct,
        ),
    ] {
        rates.insert(path.to_string(), estimate(stats, turn, path));
    }
    for (path, values) in [
        (
            "draw.draw_source_percent_seen_by_turn",
            stats.draw_sources.as_slice(),
        ),
        (
            "interaction.percent_games_with_ready_interaction_by_turn",
            stats.interaction_ready_by_turn.as_slice(),
        ),
        (
            "win_conditions.percent_games_at_or_above_table_life_by_turn",
            stats.lethal_damage_by_turn.as_slice(),
        ),
        (
            "milestones.percent_games_with_positive_mana_loop_by_turn",
            stats.positive_mana_loop_by_turn.as_slice(),
        ),
        (
            "land_drops.percent_games_hitting_all_land_drops_by_turn",
            &stats.hit_all_drops_by[1..=stats.turns.min(5) as usize],
        ),
        (
            "commander.percent_castable_by_turn",
            &stats.commander_castable_by[1..=stats.turns as usize],
        ),
    ] {
        for (index, _) in values.iter().enumerate() {
            let turn = index as u32 + 1;
            let row = if path.starts_with("commander.") && deck.commanders.is_empty() {
                RateEstimate::unavailable(ObservationStatus::NotApplicable)
            } else {
                estimate(stats, turn, &format!("{path}.{turn}"))
            };
            rates.insert(format!("{path}.{turn}"), row);
        }
    }
    for card in &stats.card_castability {
        rates.insert(
            format!("card_castability[{}].percent_castable_by_target", card.name),
            estimate(
                stats,
                card.target_turn,
                &format!("card_castability[{}].percent_castable_by_target", card.name),
            ),
        );
    }
    for (slot, games) in stats.commander_combat_threshold_games.iter().enumerate() {
        rates.insert(
            format!("combat.commander_potential[{slot}].percent_games_reaching_21"),
            if deck.commanders.get(slot).is_some() {
                RateEstimate::observed(*games, stats.finished_games)
            } else {
                RateEstimate::unavailable(ObservationStatus::NotApplicable)
            },
        );
    }
    for (path, applicable, turn, _share) in [
        (
            "station.online_by_t6",
            deck.commanders
                .first()
                .is_some_and(|card| card.animate_at().is_some()),
            6,
            stats.station_online_pct,
        ),
        (
            "companion.in_hand_by_t6",
            deck.companion.is_some(),
            6,
            stats.companion_online_pct,
        ),
    ] {
        rates.insert(
            path.to_string(),
            if applicable {
                estimate(stats, turn, path)
            } else {
                RateEstimate::unavailable(ObservationStatus::NotApplicable)
            },
        );
    }
    if let Some(commander) = deck.commanders.first() {
        let turn = commander.cost.total().max(1);
        rates.insert(
            "commander.percent_castable_by_curve".to_string(),
            estimate(stats, turn, "commander.percent_castable_by_curve"),
        );
    }
    for (color, _) in ["white", "blue", "black", "red", "green"]
        .into_iter()
        .zip(stats.color_screw)
    {
        rates.insert(
            format!("color_mana_shortage.{color}"),
            estimate(stats, 1, &format!("color_mana_shortage.{color}")),
        );
    }
    for block in &stats.pip_blocks {
        rates.insert(
            format!(
                "color_pip_blocks[{}:{}].percent_of_games",
                block.name,
                block.color.symbol()
            ),
            estimate(
                stats,
                1,
                &format!(
                    "color_pip_blocks[{}:{}].percent_of_games",
                    block.name,
                    block.color.symbol()
                ),
            ),
        );
    }
    rates
}
