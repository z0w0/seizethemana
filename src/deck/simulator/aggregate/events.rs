//! Exact successes for whole-game event rates, separate from turn-state means.

use super::*;
use crate::deck::simulator::{game::GameCompletion, model::Role};

/// Count an event once per game under its declared completion policy.
pub(super) fn record(stats: &mut SimStats, log: &GameLog, deck: &SimDeck) {
    let mut add = |path: String, event: bool| {
        *stats.event_games.entry(path).or_default() += u32::from(event);
    };
    add(
        "win_conditions.percent_games_with_suspected_infinite_mana".into(),
        log.infinite_mana_suspected,
    );
    for (turn, row) in log
        .milestones_by_turn
        .iter()
        .enumerate()
        .take(stats.turns as usize)
    {
        add(
            format!(
                "milestones.percent_games_with_positive_mana_loop_by_turn.{}",
                turn + 1
            ),
            row.positive_mana_loop,
        );
    }
    if matches!(log.completion, GameCompletion::Incomplete { .. }) {
        return;
    }
    opening_events(log, &mut add);
    fixed_events(log, stats.turns, &mut add);
    turn_events(log, deck, stats.turns, &mut add);
    card_events(log, deck, stats.turns, &stats.card_castability, &mut add);
}

/// Count mutually exclusive opening-hand land buckets.
fn opening_events(log: &GameLog, add: &mut impl FnMut(String, bool)) {
    for (bucket, path) in [
        "percent_0_lands",
        "percent_1_land",
        "percent_2_lands",
        "percent_3_lands",
        "percent_4_lands",
        "percent_five_or_more_lands",
    ]
    .into_iter()
    .enumerate()
    {
        add(
            format!("opening_hand.{path}"),
            usize::from(log.opener_lands).min(5) == bucket,
        );
    }
}

/// Count fixed-window access and game-level events before deriving percentages.
fn fixed_events(log: &GameLog, turns: u32, add: &mut impl FnMut(String, bool)) {
    let seen = |role, by| log.first_seen.get(&role).is_some_and(|turn| *turn <= by);
    for (path, event) in [
        ("opening_hand.percent_mulliganed", log.mulliganed),
        (
            "land_drops.percent_games_with_two_or_fewer_lands_by_turn_4",
            log.lands_by_4 <= 2,
        ),
        (
            "land_drops.percent_games_with_six_or_more_lands_by_turn_4",
            log.lands_seen_by_turn_4 >= 6,
        ),
        (
            "mana.percent_games_with_three_or_more_unused_mana_by_turn_6",
            log.mana_remaining.get(5).is_some_and(|mana| *mana >= 3.0)
                && log.turn_observations.get(5).is_some_and(|row| row.main),
        ),
        (
            "draw.percent_games_with_no_draw_source_by_turn_6",
            !seen(Role::Draw, 6),
        ),
        (
            "role_access.removal_spell_percent_seen_by_turn_5",
            seen(Role::Removal, 5),
        ),
        (
            "role_access.draw_source_percent_seen_by_turn_6",
            seen(Role::Draw, 6),
        ),
        (
            "role_access.creature_role_percent_seen_by_turn_3",
            log.first_creature.is_some_and(|turn| turn <= 3),
        ),
        (
            "role_access.win_condition_role_percent_seen_by_turn_8",
            seen(Role::Wincon, 8),
        ),
        (
            "role_access.lock_role_percent_seen_by_turn_3",
            seen(Role::Lock, 3),
        ),
        (
            "win_conditions.percent_games_with_extra_turn",
            log.extra_turns.iter().any(|count| *count > 0),
        ),
        (
            "win_conditions.percent_games_reaching_counter_win_threshold",
            log.win_threshold_turn.is_some_and(|turn| turn <= turns),
        ),
        (
            "win_conditions.percent_games_with_affordable_ultimate",
            log.ultimate_online.is_some_and(|turn| turn <= turns),
        ),
        (
            "station.online_by_t6",
            log.station_online.is_some_and(|turn| turn <= 6),
        ),
        (
            "companion.in_hand_by_t6",
            log.companion_online.is_some_and(|turn| turn <= 6),
        ),
    ] {
        add(path.into(), event);
    }
}

/// Count by-turn events with finished early losses kept in the denominator.
fn turn_events(log: &GameLog, deck: &SimDeck, turns: u32, add: &mut impl FnMut(String, bool)) {
    let seen = |role, by| log.first_seen.get(&role).is_some_and(|turn| *turn <= by);
    let mut drops = 0u32;
    let mut reached_damage = false;
    for turn in 0..turns as usize {
        drops += u32::from(log.land_drops[turn]);
        reached_damage |= f64::from(log.player_damage[turn])
            + f64::from(log.opponent_life_loss[turn])
            >= deck.rules.life_target();
        for (path, event) in [
            (
                "draw.draw_source_percent_seen_by_turn",
                seen(Role::Draw, turn as u32 + 1),
            ),
            (
                "interaction.percent_games_with_ready_interaction_by_turn",
                log.turn_observations[turn].main && log.interaction_ready[turn],
            ),
            (
                "win_conditions.percent_games_at_or_above_table_life_by_turn",
                reached_damage,
            ),
            (
                "land_drops.percent_games_hitting_all_land_drops_by_turn",
                drops > turn as u32,
            ),
            (
                "commander.percent_castable_by_turn",
                log.commander_castable
                    .is_some_and(|first| first <= turn as u32 + 1),
            ),
        ] {
            add(format!("{path}.{}", turn + 1), event);
        }
    }
}

/// Count each physical-card group and color block at most once per game.
fn card_events(
    log: &GameLog,
    deck: &SimDeck,
    turns: u32,
    cards: &[CardCast],
    add: &mut impl FnMut(String, bool),
) {
    if let Some(commander) = deck.commanders.first() {
        add(
            "commander.percent_castable_by_curve".into(),
            log.commander_castable
                .is_some_and(|turn| turn <= commander.cost.total().max(1)),
        );
    }
    for (color, blocked) in ["white", "blue", "black", "red", "green"]
        .into_iter()
        .zip(log.blocked_colors)
    {
        add(format!("color_mana_shortage.{color}"), blocked);
    }
    let mut blocks = std::collections::BTreeSet::new();
    for (index, color) in &log.pip_blocks {
        blocks.insert(format!(
            "color_pip_blocks[{}:{}].percent_of_games",
            deck.cards[*index].name,
            crate::deck::simulator::model::COLORS[*color].symbol()
        ));
    }
    for path in blocks {
        add(path, true);
    }
    for card in cards {
        let event = deck
            .cards
            .iter()
            .enumerate()
            .filter(|(_, copy)| copy.name == card.name)
            .any(|(index, _)| {
                log.mana_ready
                    .get(index)
                    .copied()
                    .flatten()
                    .is_some_and(|turn| turn <= card.target_turn && turn <= turns)
            });
        add(
            format!("card_castability[{}].percent_castable_by_target", card.name),
            event,
        );
    }
}
