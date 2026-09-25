// Aggregation of game logs into the report values: per-game stats, role
// access, castability, and color-screw census. Problem findings and the
// mana-base verdict live in `findings`.

use std::collections::HashMap;

use super::findings::PipBlock;
use super::game::GameLog;
use super::hypgeo::flood_expectation;
use super::model::{COLORS, Role, SimDeck};

/// Per-card castability row for nonland cards.
#[derive(Debug, Clone)]
pub struct CardCast {
    /// Card name.
    pub name: String,
    /// Mana value.
    pub cmc: f64,
    /// On-curve cast turn (ceil(cmc)).
    pub target_turn: u32,
    /// Fraction of games castable by the target turn.
    pub pct_by_target: f64,
    /// Average first-castable turn.
    pub avg_first_castable_turn: f64,
}

/// Aggregated simulation statistics.
#[derive(Debug, Default, Clone)]
pub struct SimStats {
    /// Games simulated.
    pub runs: u32,
    /// Turns simulated.
    pub turns: u32,
    /// Average lands in the opening hand.
    pub avg_opener_lands: f64,
    /// Fraction of openers redrawn.
    pub mulligan_rate: f64,
    /// Opening-hand land distribution [0, 1, 2, 3, 4, 5+].
    pub opener_pct: [f64; 6],
    /// P(all k land drops hit by turn k), k = 1..=5.
    pub hit_all_drops_by: [f64; 6],
    /// P(2 or fewer lands by turn 4).
    pub screw_pct: f64,
    /// P(6 or more lands in hand + on the battlefield at end of turn 4),
    /// the draw-aware flood metric. Calibrated so a land-heavy deck
    /// actually reports flood (a drops-made detector reads a 44-land
    /// deck as 0% flooded).
    pub flood_pct: f64,
    /// Hypergeometric expectation of the flood bucket at the deck's
    /// actual draw volume (mean per-game cards seen by t4), for
    /// calibration checks.
    pub flood_expectation: f64,
    /// Median land drops made by turn 4.
    pub p50_drops_by_4: u32,
    /// 95th-percentile land drops made by turn 4.
    pub p95_drops_by_4: u32,
    /// P(commander castable by turn t), t = 1..=12.
    pub commander_castable_by: [f64; 13],
    /// Average first-castable turn of the commander.
    pub avg_commander_cast_turn: f64,
    /// Median commander cast turn.
    pub p50_commander_cast_turn: u32,
    /// 95th-percentile commander cast turn.
    pub p95_commander_cast_turn: u32,
    /// Average unspent mana per turn.
    pub unused_mana: Vec<f64>,
    /// P(3+ mana floated at turn 6).
    pub floated_pct: f64,
    /// Average cards seen by each turn.
    pub cards_seen: Vec<f64>,
    /// Games with no draw source seen by turn 6.
    pub starved_pct: f64,
    /// P(at least one draw-role card seen in hand by turn t), t = 1..=N.
    /// Hand visibility, not engines online: `engines_online_by_turn` has
    /// the online count.
    pub draw_sources: Vec<f64>,
    /// P(removal seen by turn 5).
    pub removal_access_5: f64,
    /// P(draw source seen by turn 6).
    pub draw_access_6: f64,
    /// P(creature seen by turn 3).
    pub creature_access_3: f64,
    /// P(win condition seen by turn 8).
    pub wincon_access_8: f64,
    /// P(lock piece seen by turn 3) — stax timing.
    pub lock_access_3: f64,
    /// Per-color cast-blocking share (WUBRG).
    pub color_screw: [f64; 5],
    /// Worst color-screw share.
    pub color_screw_pct: f64,
    /// Top card x color pip-block offenders (worst 5).
    pub pip_blocks: Vec<PipBlock>,
    /// Average graveyard size at the end of each turn.
    pub graveyard_by_turn: Vec<f64>,
    /// Per-card castability rows.
    pub card_castability: Vec<CardCast>,
    /// Deck-shape counts for findings context.
    pub removal_count: usize,
    /// Board-wipe copies within `removal_count`.
    pub removal_wipes: usize,
    /// Targeted removal + counter copies within `removal_count`.
    pub removal_targeted: usize,
    pub wincon_count: usize,
    pub draw_count: usize,
    pub land_count: usize,
    /// P(commander station animated by turn 6) across games (0 when the
    /// commander is not a station card).
    pub station_online_pct: f64,
    /// Median first online turn for the commander station.
    pub station_p50_turn: u32,
    /// Average bodies in play by turn.
    pub bodies_by_turn: Vec<f64>,
    /// Average engines online by turn.
    pub engines_by_turn: Vec<f64>,
    /// Average attacking power on the board by turn.
    pub attack_power_by_turn: Vec<f64>,
    /// 90th-percentile attacking power by turn 8 (power curve, not a
    /// kill estimate).
    pub attack_power_p90: u32,
    /// Average attacking bodies with evasion by turn.
    pub evasive_by_turn: Vec<f64>,
    /// Average attacking bodies by turn (denominator of the evasion
    /// census).
    pub attackers_by_turn: Vec<f64>,
    /// Average library size by turn (deck-out proximity).
    pub library_by_turn: Vec<f64>,
    /// Average cards self-milled by turn (graveyard fuel velocity).
    pub self_milled_by_turn: Vec<f64>,
    /// Average cards milled toward opponents by turn (deck-out
    /// pressure).
    pub opp_milled_by_turn: Vec<f64>,
    /// Average fraction of the library evaluated by turn (drawn +
    /// milled + scried/surveiled).
    pub library_awareness_by_turn: Vec<f64>,
    /// Average life drained by turn (burn, drain engines).
    pub drain_total_by_turn: Vec<f64>,
    /// P(cumulative player damage at/above the life total by turn t),
    /// indexed 1..turns (lethal census; best-case goldfish, unblocked).
    pub lethal_damage_by_turn: Vec<f64>,
    /// Median first turn at/above the life total; None when never.
    pub p50_lethal_turn: Option<u32>,
    /// P(at least one extra turn taken by turn t).
    pub extra_turns_pct: f64,
    /// Median first turn a win-threshold engine could fire.
    pub win_threshold_p50_turn: u32,
    /// P(win-threshold engine online by turn 10).
    pub win_threshold_pct: f64,
    /// P(a planeswalker ultimate online by turn 10).
    pub ultimate_online_pct: f64,
    /// P(interaction ready — in hand + affordable) by turn.
    pub interaction_ready_by_turn: Vec<f64>,
    /// Average spare mana while interaction was ready.
    pub interaction_mana_held: f64,
    /// Instant-speed interaction copies in the deck (readiness
    /// denominator).
    pub interaction_instant_count: usize,
    /// Share of games where a zero-cost mana activation looped past the
    /// cap (Basalt Monolith-class infinite engine suspected).
    pub infinite_mana_pct: f64,
}

/// Aggregate many game logs into the report values.
pub fn aggregate(logs: &[GameLog], deck: &SimDeck, turns: u32) -> SimStats {
    let turns = turns.max(1) as usize;
    let n = logs.len().max(1) as f64;
    let mut stats = SimStats {
        runs: logs.len() as u32,
        turns: turns as u32,
        ..SimStats::default()
    };
    stats.unused_mana = vec![0.0; turns];
    stats.cards_seen = vec![0.0; turns];
    stats.bodies_by_turn = vec![0.0; turns];
    stats.engines_by_turn = vec![0.0; turns];
    stats.attack_power_by_turn = vec![0.0; turns];
    stats.evasive_by_turn = vec![0.0; turns];
    stats.attackers_by_turn = vec![0.0; turns];
    stats.library_by_turn = vec![0.0; turns];
    stats.self_milled_by_turn = vec![0.0; turns];
    stats.opp_milled_by_turn = vec![0.0; turns];
    stats.library_awareness_by_turn = vec![0.0; turns];
    stats.drain_total_by_turn = vec![0.0; turns];
    stats.lethal_damage_by_turn = vec![0.0; turns];
    stats.interaction_ready_by_turn = vec![0.0; turns];
    stats.interaction_instant_count = deck
        .cards
        .iter()
        .filter(|c| c.is_interaction && c.is_instant_speed)
        .count();

    opener_stats(logs, &mut stats, n);
    screw_flood_stats(logs, deck, &mut stats, turns, n);
    commander_timing_stats(logs, deck, &mut stats, turns as u32, n);
    velocity_stats(logs, deck, &mut stats, turns, n);
    lethal_stats(logs, deck, &mut stats);
    interaction_stats(logs, &mut stats, n);
    misc_stats(logs, &mut stats, turns, n);
    role_access_stats(logs, &mut stats, turns, n);
    color_screw_stats(logs, deck, &mut stats, n);
    graveyard_stats(logs, &mut stats, turns, n);
    castability_stats(logs, deck, &mut stats, turns, n);
    stats
}

/// Opening-hand land distribution and mulligan rate.
fn opener_stats(logs: &[GameLog], stats: &mut SimStats, n: f64) {
    let mut opener_counts = [0i64; 7];
    for log in logs {
        opener_counts[(log.opener_lands as usize).min(6)] += 1;
    }
    let opener_total = opener_counts.iter().sum::<i64>().max(1) as f64;
    stats.avg_opener_lands = logs.iter().map(|l| f64::from(l.opener_lands)).sum::<f64>() / n;
    stats.mulligan_rate = logs.iter().filter(|l| l.mulliganed).count() as f64 / n;
    stats.opener_pct = [
        opener_counts[0] as f64 / opener_total,
        opener_counts[1] as f64 / opener_total,
        opener_counts[2] as f64 / opener_total,
        opener_counts[3] as f64 / opener_total,
        opener_counts[4] as f64 / opener_total,
        (opener_counts[5] + opener_counts[6]) as f64 / opener_total,
    ];
}

/// Land-drop rates and the screw/flood buckets against the
/// hypergeometric expectation at each game's own draw volume.
fn screw_flood_stats(logs: &[GameLog], deck: &SimDeck, stats: &mut SimStats, turns: usize, n: f64) {
    let mut drops_by_4: Vec<u32> = Vec::new();
    let mut seen_by_4: Vec<u32> = Vec::new();
    for log in logs {
        for k in 1..=5.min(turns) {
            let made: u32 = log.land_drops[..k].iter().map(|d| u32::from(*d)).sum();
            // Extra-land decks can overshoot (Aesi plays two a turn);
            // hitting every drop through turn k means at least k drops.
            if made >= k as u32 {
                stats.hit_all_drops_by[k] += 1.0 / n;
            }
        }
        if turns >= 4 {
            drops_by_4.push(u32::from(log.lands_by_4));
            if log.lands_by_4 <= 2 {
                stats.screw_pct += 1.0 / n;
            }
            // Flood = a flood-grade window, not drops made: 6+ lands in
            // hand, on the battlefield, and in the graveyard at end of
            // turn 4. A deck whose lands exceed ~35 reports flood near
            // its hypergeometric expectation (at the game's actual seen
            // count).
            if log.lands_seen_by_11 >= 6 {
                stats.flood_pct += 1.0 / n;
            }
            seen_by_4.push(log.cards_seen_by_4);
        }
    }
    if !drops_by_4.is_empty() {
        stats.p50_drops_by_4 = percentile(&mut drops_by_4, 0.5);
        stats.p95_drops_by_4 = percentile(&mut drops_by_4, 0.95);
    }
    // Expectation at the deck's actual draw volume: each game's own
    // cards-seen count feeds its own hypergeometric window, so the
    // printed baseline matches the measured bucket even for cantrip
    // decks (a fixed 11-card window reads draw-heavy decks as floodier
    // than they are).
    let lands_count = deck.cards.iter().filter(|c| c.role == Role::Land).count();
    let deck_size = deck.cards.len();
    stats.flood_expectation = if seen_by_4.is_empty() {
        0.0
    } else {
        seen_by_4
            .iter()
            .map(|&seen| flood_expectation(lands_count, deck_size, seen as usize))
            .sum::<f64>()
            / seen_by_4.len() as f64
    };
}

/// Commander cast timing and station-online rates.
fn commander_timing_stats(
    logs: &[GameLog],
    deck: &SimDeck,
    stats: &mut SimStats,
    turns: u32,
    n: f64,
) {
    let Some(cmd) = deck.commanders.first() else {
        return;
    };
    let mut cast_turns: Vec<u32> = Vec::new();
    for log in logs {
        if let Some(t) = log.commander_castable {
            cast_turns.push(t);
            for t2 in t..=turns {
                // Cap at the array length: later games contribute
                // nothing to the curve's tail.
                if (t2 as usize) < stats.commander_castable_by.len() {
                    stats.commander_castable_by[t2 as usize] += 1.0 / n;
                }
            }
        }
    }
    if !cast_turns.is_empty() {
        let mut sorted = cast_turns.clone();
        stats.p50_commander_cast_turn = percentile(&mut sorted, 0.5);
        stats.p95_commander_cast_turn = percentile(&mut sorted, 0.95);
        stats.avg_commander_cast_turn =
            cast_turns.iter().map(|t| f64::from(*t)).sum::<f64>() / cast_turns.len() as f64;
    }

    // Station online: commander spacecraft animated by turn 6.
    if cmd.animate_at().is_some() {
        let online: Vec<Option<u32>> = logs.iter().map(|l| l.station_online).collect();
        stats.station_online_pct =
            online.iter().filter(|t| t.is_some_and(|t| t <= 6)).count() as f64 / n;
        let mut turns_vec: Vec<u32> = online.iter().flatten().copied().collect();
        if !turns_vec.is_empty() {
            stats.station_p50_turn = percentile(&mut turns_vec, 0.5);
        }
    }
}

/// Per-turn mana, velocity, board, and damage census averages.
fn velocity_stats(logs: &[GameLog], deck: &SimDeck, stats: &mut SimStats, turns: usize, n: f64) {
    let life_target = deck.format.life_target();
    for log in logs {
        for t in 0..turns {
            stats.unused_mana[t] += (log.mana_available[t] - log.mana_spent[t]).max(0.0) / n;
            stats.cards_seen[t] += f64::from(log.cards_seen[t]) / n;
            if t < log.bodies.len() {
                stats.bodies_by_turn[t] += f64::from(log.bodies[t]) / n;
            }
            if t < log.engines_online.len() {
                stats.engines_by_turn[t] += f64::from(log.engines_online[t]) / n;
            }
            if t < log.attack_power.len() {
                stats.attack_power_by_turn[t] += f64::from(log.attack_power[t]) / n;
            }
            if t < log.evasive.len() {
                stats.evasive_by_turn[t] += f64::from(log.evasive[t]) / n;
            }
            if t < log.attackers.len() {
                stats.attackers_by_turn[t] += f64::from(log.attackers[t]) / n;
            }
            if t < log.library_size.len() {
                stats.library_by_turn[t] += f64::from(log.library_size[t]) / n;
            }
            if t < log.self_milled.len() {
                stats.self_milled_by_turn[t] += f64::from(log.self_milled[t]) / n;
            }
            if t < log.opp_milled.len() {
                stats.opp_milled_by_turn[t] += f64::from(log.opp_milled[t]) / n;
            }
            if t < log.awareness.len() {
                stats.library_awareness_by_turn[t] += log.awareness[t] / n;
            }
            if t < log.drain_total.len() {
                stats.drain_total_by_turn[t] += f64::from(log.drain_total[t]) / n;
            }
            if t < log.player_damage.len() {
                // Player damage: combat damage to players + the drain
                // census (burn/drain effects already carry the format
                // multiplier for three opponents).
                let dealt = f64::from(log.player_damage[t]) + f64::from(log.drain_total[t]);
                if dealt >= life_target {
                    stats.lethal_damage_by_turn[t] += 1.0 / n;
                }
            }
            if t < log.interaction_ready.len() && log.interaction_ready[t] {
                stats.interaction_ready_by_turn[t] += 1.0 / n;
            }
        }
    }
}

/// Median lethal turn: the first turn each game crossed the life total,
/// then the median over games that ever crossed.
fn lethal_stats(logs: &[GameLog], deck: &SimDeck, stats: &mut SimStats) {
    let mut lethal_turns: Vec<u32> = Vec::new();
    for log in logs {
        if let Some(t) = log
            .player_damage
            .iter()
            .zip(log.drain_total.iter())
            .enumerate()
            .find(|(_, (d, dr))| f64::from(*d + *dr) >= deck.format.life_target())
            .map(|(t, _)| t as u32 + 1)
        {
            lethal_turns.push(t);
        }
    }
    stats.p50_lethal_turn = if lethal_turns.is_empty() {
        None
    } else {
        // Nearest-rank median; `percentile` sorts its input itself.
        Some(percentile(&mut lethal_turns, 0.5))
    };
}

/// Interaction tempo: ready-turn share and average spare mana held.
fn interaction_stats(logs: &[GameLog], stats: &mut SimStats, n: f64) {
    let ready_turns: usize = logs
        .iter()
        .map(|l| l.interaction_ready.iter().filter(|r| **r).count())
        .sum();
    let held_total: f64 = logs
        .iter()
        .flat_map(|l| {
            l.interaction_ready
                .iter()
                .zip(l.interaction_mana_held.iter())
                .filter(|(r, _)| **r)
                .map(|(_, h)| *h)
                .collect::<Vec<f64>>()
        })
        .sum();
    stats.interaction_mana_held = held_total / f64::from(ready_turns as u32).max(1.0);
    stats.extra_turns_pct = logs
        .iter()
        .filter(|l| l.extra_turns.iter().any(|e| *e > 0))
        .count() as f64
        / n;
}

/// Misc one-off aggregates: percentiles, win/ultimate/infinite rates,
/// role access, and per-card castability.
fn misc_stats(logs: &[GameLog], stats: &mut SimStats, turns: usize, n: f64) {
    // Attack power p90 at turn 8 (or the last turn simulated).
    if turns >= 8 {
        let mut p90s: Vec<u32> = logs.iter().map(|l| l.attack_power[7]).collect();
        stats.attack_power_p90 = percentile(&mut p90s, 0.9);
    }
    let mut win_turns: Vec<u32> = logs.iter().filter_map(|l| l.win_threshold_turn).collect();
    win_turns.sort_unstable();
    stats.win_threshold_p50_turn = win_turns
        .get(win_turns.len() / 2)
        .copied()
        .unwrap_or_default();
    stats.win_threshold_pct = logs
        .iter()
        .filter(|l| l.win_threshold_turn.is_some())
        .count() as f64
        / n;
    stats.ultimate_online_pct =
        logs.iter().filter(|l| l.ultimate_online.is_some()).count() as f64 / n;
    stats.infinite_mana_pct = logs.iter().filter(|l| l.infinite_mana_suspected).count() as f64 / n;
    if turns >= 6 {
        stats.floated_pct = logs
            .iter()
            .filter(|l| (l.mana_available[5] - l.mana_spent[5]).max(0.0) >= 3.0)
            .count() as f64
            / n;
    }
}

/// Role access from first-sighting turns (starvation and access rates).
fn role_access_stats(logs: &[GameLog], stats: &mut SimStats, turns: usize, n: f64) {
    let starved = logs
        .iter()
        .filter(|l| {
            !l.first_seen
                .get(&Role::Draw)
                .is_some_and(|t| *t <= 6u32.min(turns as u32))
        })
        .count() as f64;
    stats.starved_pct = starved / n;
    stats.draw_access_6 = 1.0 - stats.starved_pct;
    stats.removal_access_5 = logs
        .iter()
        .filter(|l| {
            l.first_seen
                .get(&Role::Removal)
                .is_some_and(|t| *t <= 5u32.min(turns as u32))
        })
        .count() as f64
        / n;
    stats.creature_access_3 = logs
        .iter()
        .filter(|l| l.first_creature.is_some_and(|t| t <= 3))
        .count() as f64
        / n;
    stats.wincon_access_8 = logs
        .iter()
        .filter(|l| {
            l.first_seen
                .get(&Role::Wincon)
                .is_some_and(|t| *t <= 8u32.min(turns as u32))
        })
        .count() as f64
        / n;
    stats.lock_access_3 = logs
        .iter()
        .filter(|l| {
            l.first_seen
                .get(&Role::Lock)
                .is_some_and(|t| *t <= 3u32.min(turns as u32))
        })
        .count() as f64
        / n;

    // P(a draw-role card is in hand by turn t) across games: hand
    // visibility of the draw role, not engines online.
    stats.draw_sources = (0..turns)
        .map(|t| {
            logs.iter()
                .filter(|log| {
                    log.first_seen
                        .get(&Role::Draw)
                        .is_some_and(|seen| *seen <= (t + 1) as u32)
                })
                .count() as f64
                / n
        })
        .collect();
}

/// Color screw: casts blocked per color and the top pip blocks.
fn color_screw_stats(logs: &[GameLog], deck: &SimDeck, stats: &mut SimStats, n: f64) {
    for log in logs {
        for (i, blocked) in log.blocked_colors.iter().enumerate() {
            if *blocked {
                stats.color_screw[i] += 1.0 / n;
            }
        }
    }
    stats.color_screw_pct = stats.color_screw.iter().cloned().fold(0.0f64, f64::max);

    // Pip blocks per card x color: share of games where that card's cast
    // was blocked for that color's missing pips.
    let mut block_count: HashMap<(usize, usize), u64> = HashMap::new();
    for log in logs {
        let mut seen_this_game: std::collections::HashSet<(usize, usize)> =
            std::collections::HashSet::new();
        for pair in &log.pip_blocks {
            seen_this_game.insert(*pair);
        }
        for pair in seen_this_game {
            *block_count.entry(pair).or_insert(0) += 1;
        }
    }
    let mut pip_blocks: Vec<PipBlock> = block_count
        .into_iter()
        .map(|((idx, ci), count)| PipBlock {
            name: deck.cards[idx].name.clone(),
            color: COLORS[ci],
            pct_games: count as f64 / n,
        })
        .collect();
    pip_blocks.sort_by(|a, b| {
        b.pct_games
            .partial_cmp(&a.pct_games)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.name.cmp(&b.name))
    });
    pip_blocks.truncate(5);
    stats.pip_blocks = pip_blocks;
}

/// Graveyard census: average size at the end of each turn.
fn graveyard_stats(logs: &[GameLog], stats: &mut SimStats, turns: usize, n: f64) {
    if !logs.is_empty() {
        for t in 0..turns.min(logs[0].graveyard_size.len()) {
            stats
                .graveyard_by_turn
                .push(logs.iter().map(|log| log.graveyard_size[t]).sum::<u32>() as f64 / n);
        }
    }
}

/// Per-card castability from the draw-agnostic mana-readiness curve.
fn castability_stats(logs: &[GameLog], deck: &SimDeck, stats: &mut SimStats, turns: usize, n: f64) {
    let mut by_target = vec![0u64; deck.cards.len()];
    let mut first_sum = vec![0.0f64; deck.cards.len()];
    let mut first_n = vec![0u64; deck.cards.len()];
    for log in logs {
        for (idx, ready) in log.mana_ready.iter().enumerate() {
            if let Some(turn) = ready {
                let target = target_turn(deck.cards[idx].min_cost.total() as f64);
                if *turn <= target {
                    by_target[idx] += 1;
                }
                first_sum[idx] += f64::from(*turn);
                first_n[idx] += 1;
            }
        }
    }
    stats.card_castability = deck
        .cards
        .iter()
        .enumerate()
        .filter(|(_, card)| card.role != Role::Land)
        .map(|(idx, card)| CardCast {
            name: card.name.clone(),
            cmc: card.cost.total() as f64,
            target_turn: target_turn(card.min_cost.total() as f64),
            pct_by_target: by_target[idx] as f64 / n,
            avg_first_castable_turn: if first_n[idx] > 0 {
                first_sum[idx] / first_n[idx] as f64
            } else {
                f64::from(turns as u32 + 1)
            },
        })
        .collect();
}

/// Target turn to cast a card on curve (turn N casts N-mana spells).
fn target_turn(cmc: f64) -> u32 {
    (cmc.ceil() as u32).max(1)
}

/// Nearest-rank percentile of a slice (sorts it).
pub(super) fn percentile(values: &mut [u32], q: f64) -> u32 {
    values.sort_unstable();
    if values.is_empty() {
        return 0;
    }
    let idx = ((q * values.len() as f64).ceil() as usize).clamp(1, values.len());
    values[idx - 1]
}
