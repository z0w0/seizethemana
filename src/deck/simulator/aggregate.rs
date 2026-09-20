// Aggregation of game logs into the report values, plus the problem
// findings. Same thresholds as the pre-module simulator; new station and
// body metrics join additively.

use std::collections::HashMap;

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
    stats.interaction_ready_by_turn = vec![0.0; turns];
    stats.interaction_instant_count = deck
        .cards
        .iter()
        .filter(|c| c.is_interaction && c.is_instant_speed)
        .count();

    // Opening-hand land distribution.
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

    // Land-drop rates and screw/flood buckets.
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
            // hand + on the battlefield at end of turn 4. A deck whose
            // lands exceed ~35 reports flood near its hypergeometric
            // expectation (at the game's actual seen count).
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

    // Commander timing.
    if let Some(cmd) = deck.commanders.first() {
        let mut cast_turns: Vec<u32> = Vec::new();
        for log in logs {
            if let Some(t) = log.commander_castable {
                cast_turns.push(t);
                for t2 in t..=turns as u32 {
                    if (t2 as usize) < 13 {
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

    // Mana and velocity per turn.
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
            if t < log.interaction_ready.len() && log.interaction_ready[t] {
                stats.interaction_ready_by_turn[t] += 1.0 / n;
            }
        }
    }
    // Interaction tempo tax: average spare mana on ready turns.
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
    // Attack power p90 at turn 8 (or the last turn simulated).
    if turns >= 8 {
        let mut p90s: Vec<u32> = logs.iter().map(|l| l.attack_power[7]).collect();
        p90s.sort_unstable();
        stats.attack_power_p90 = p90s
            .get((p90s.len() as f64 * 0.9) as usize)
            .copied()
            .unwrap_or(0);
    }
    stats.extra_turns_pct = logs
        .iter()
        .filter(|l| l.extra_turns.iter().any(|e| *e > 0))
        .count() as f64
        / n;
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

    // Role access from first-sighting turns.
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

    // Color screw: casts blocked per color.
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

    // Graveyard census: average size at the end of each turn.
    if !logs.is_empty() {
        for t in 0..turns.min(logs[0].graveyard_size.len()) {
            stats
                .graveyard_by_turn
                .push(logs.iter().map(|log| log.graveyard_size[t]).sum::<u32>() as f64 / n);
        }
    }

    // Per-card castability from the draw-agnostic mana-readiness curve.
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
    stats
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

/// One combo pair's assembly timing: share of games where both pieces
/// were seen in hand by the target turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ComboAccess {
    /// Both piece names, "A + B".
    pub pair: String,
    /// Target turn: the later piece's cast-on-curve turn.
    pub target_turn: u32,
    /// Share of games with both pieces seen in hand by the target turn,
    /// serialized 0-100 (the JSON percent scale).
    #[serde(serialize_with = "crate::deck::simulator::report::serialize_pct")]
    pub pct_games: f64,
}

/// Compute combo pair access from game logs. Pieces the deck does not
/// hold report 0% so the gap stays visible.
pub fn piece_pair_access(
    logs: &[GameLog],
    deck: &SimDeck,
    pairs: &[(String, String)],
    turns: u32,
) -> Vec<ComboAccess> {
    let n = logs.len() as f64;
    if n == 0.0 {
        return Vec::new();
    }
    let find_idx = |name: &str| deck.cards.iter().position(|c| c.name == name);
    let mut out = Vec::new();
    for (a, b) in pairs {
        let (Some(ia), Some(ib)) = (find_idx(a), find_idx(b)) else {
            out.push(ComboAccess {
                pair: format!("{a} + {b}"),
                target_turn: 0,
                pct_games: 0.0,
            });
            continue;
        };
        let target = deck.cards[ia]
            .cost
            .total()
            .max(deck.cards[ib].cost.total())
            .max(1)
            .min(turns);
        let both = logs
            .iter()
            .filter(|log| {
                let seen_a = log.card_first_seen.get(&ia).is_some_and(|t| *t <= target);
                let seen_b = log.card_first_seen.get(&ib).is_some_and(|t| *t <= target);
                seen_a && seen_b
            })
            .count() as f64
            / n;
        out.push(ComboAccess {
            pair: format!("{a} + {b}"),
            target_turn: target,
            pct_games: both,
        });
    }
    out
}

/// Per-color cast-block share for one card: the deck had enough total
/// mana but missed this color's pips when the card was in hand.
#[derive(Debug, Clone)]
pub struct PipBlock {
    /// Card name.
    pub name: String,
    /// WUBRG color letter that was missing.
    pub color: char,
    /// Share of games with at least one pip-blocked cast of this card
    /// for this color.
    pub pct_games: f64,
}

/// Severity-scaled magnitude word for a share (the suggestion sizes to the
/// problem, not a fixed "2-3").
fn magnitude(pct: f64) -> &'static str {
    if pct >= 30.0 {
        "3-4"
    } else if pct >= 20.0 {
        "2-3"
    } else {
        "1-2"
    }
}

/// Nonland mana sources that join per turn: rocks, dorks, and ramp
/// spells. A rock-heavy deck's sources do not read as land-screwed.
fn ramp_source_count(deck: &SimDeck) -> usize {
    use super::model::Role;
    deck.cards
        .iter()
        .filter(|c| matches!(c.role, Role::Rock | Role::Dork | Role::RampSpell))
        .count()
}

/// The deck's mana base against the bracket target bands (research-derived:
/// EDHREC average decks n=46 across 11 commanders, 2026-09; Sam Black's
/// cEDH land-count guidance).
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ManaBase {
    /// Land count (basics + nonbasic lands).
    pub lands: usize,
    /// Rocks.
    pub rocks: usize,
    /// Creature mana sources.
    pub dorks: usize,
    /// Ramp spells (search lands, ritual-class accelerants).
    pub ramp_spells: usize,
    /// Total mana sources (lands + ramp).
    pub total_sources: usize,
    /// `[min, max]` land band for the deck's bracket.
    pub bracket_target_lands: [usize; 2],
    /// `[min, max]` ramp band for the deck's bracket.
    pub bracket_target_ramp: [usize; 2],
    /// The verdict sentence; "on target" when both counts sit in band.
    pub verdict: String,
    /// The bracket the bands came from (inferred from the Game Changer
    /// census when the command got no explicit `--bracket`).
    pub bracket: u8,
    /// True when `bracket` was inferred from the Game Changer census
    /// rather than passed explicitly.
    pub bracket_inferred: bool,
}

/// Land and ramp target bands by bracket (1-5). Lands-matter decks widen
/// the land band by 4.
fn bracket_bands(bracket: u8, lands_matter: bool) -> ([usize; 2], [usize; 2]) {
    let widen = if lands_matter { 4 } else { 0 };
    let (lands, ramp) = match bracket {
        5 => ([25usize, 31], [10usize, 16]),
        4 => ([32, 36], [9, 12]),
        3 => ([33, 38], [8, 11]),
        // Brackets 1-2 share the casual band.
        _ => ([34, 40], [7, 12]),
    };
    ([lands[0], lands[1] + widen], ramp)
}

/// Build the mana-base verdict for a deck.
///
/// `bracket` drives the target band; a bracket outside 1-5 falls back to
/// the casual band.
pub fn mana_base(deck: &SimDeck, bracket: u8, bracket_inferred: bool) -> ManaBase {
    use super::model::Role;
    let lands = deck.cards.iter().filter(|c| c.role == Role::Land).count();
    let rocks = deck.cards.iter().filter(|c| c.role == Role::Rock).count();
    let dorks = deck.cards.iter().filter(|c| c.role == Role::Dork).count();
    let ramp_spells = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::RampSpell)
        .count();
    let total_sources = lands + rocks + dorks + ramp_spells;
    // Lands-matter: the commander or any card name carries a landfall /
    // lands-matter engine signal. The parse marks extra-land-drop boards
    // (`extra_land_drops`); their presence widens the band.
    let lands_matter = deck.cards.iter().any(|c| c.extra_land_drops)
        || deck.commanders.iter().any(|c| c.extra_land_drops);
    let bracket = bracket.clamp(1, 5);
    let (land_band, ramp_band) = bracket_bands(bracket, lands_matter);
    // The lands-matter label rides on any verdict, so the widened band is
    // still a real comparison (a 20-land landfall deck reads "add lands").
    let suffix = if lands_matter {
        " (lands-matter band)"
    } else {
        ""
    };
    let verdict = if lands < land_band[0] {
        format!(
            "add {} lands ({} < band {}-{}){}",
            (land_band[0] - lands).max(2),
            lands,
            land_band[0],
            land_band[1],
            suffix
        )
    } else if lands > land_band[1] {
        format!(
            "trim {} lands ({} > band {}-{}); add rocks if sources are low{}",
            (lands - land_band[1]).max(1),
            lands,
            land_band[0],
            land_band[1],
            suffix
        )
    } else if ramp_spells + rocks + dorks < ramp_band[0] {
        format!(
            "add {} ramp ({} of band {}-{}){}",
            (ramp_band[0] - (rocks + dorks + ramp_spells)).max(1),
            rocks + dorks + ramp_spells,
            ramp_band[0],
            ramp_band[1],
            suffix
        )
    } else if lands_matter {
        format!("on target{suffix}")
    } else {
        "on target".to_string()
    };
    ManaBase {
        lands,
        rocks,
        dorks,
        ramp_spells,
        total_sources,
        bracket_target_lands: land_band,
        bracket_inferred,
        bracket_target_ramp: ramp_band,
        verdict,
        bracket,
    }
}

/// One deck problem found by the simulation.
#[derive(Debug, Clone)]
pub struct Problem {
    /// Problem kind ("mana_flood", "color_screw", "draw_starvation", ...).
    pub kind: &'static str,
    /// Severity bucket from the affected-game share.
    pub severity: &'static str,
    /// Share of games affected, when game-count based.
    pub pct_games: Option<f64>,
    /// Color letter for `color_screw` problems.
    pub color: Option<char>,
    /// Human explanation with the numbers.
    pub detail: String,
    /// Category + magnitude suggestion (never specific cards).
    pub suggestion: String,
}

/// Severity bucket for an affected-game share.
fn severity(pct: f64) -> &'static str {
    if pct >= 20.0 {
        "high"
    } else if pct >= 10.0 {
        "medium"
    } else {
        "low"
    }
}

/// Dedicated and choice source counts for one WUBRG color.
///
/// Dedicated = lands whose tap yield serves exactly this color (fixed pips
/// or single-color choice); choice = multi-color pickers that can serve it.
/// Shape names how the majority of dedicated sources enter.
fn color_source_shape(deck: &SimDeck, color_index: usize) -> (usize, usize, &'static str) {
    use super::model::Role;
    let mut dedicated = 0usize;
    let mut choice = 0usize;
    let mut enters_tapped = 0usize;
    for card in deck.cards.iter().filter(|c| c.role == Role::Land) {
        let Some(y) = &card.tap else { continue };
        let serves = y.any_pips > 0
            || y.opponent_any
            || y.choice.iter().any(|c| *c)
            || y.fixed[color_index] > 0;
        if !serves {
            continue;
        }
        let multi = y.any_pips > 0
            || y.opponent_any
            || y.choice.iter().filter(|c| **c).count() > 1
            || y.fixed.iter().filter(|p| **p > 0).count() > 1;
        if multi {
            choice += 1;
        } else {
            dedicated += 1;
        }
        if card.enters_tapped {
            enters_tapped += 1;
        }
    }
    let shape = if dedicated > 0 && enters_tapped * 2 >= dedicated {
        "tapped"
    } else {
        "untapped"
    };
    (dedicated, choice, shape)
}

/// Find deck problems from the aggregated stats.
pub fn find_problems(stats: &SimStats, deck: &SimDeck) -> Vec<Problem> {
    use super::model::COLORS;
    let mut problems = Vec::new();
    let commander = !deck.commanders.is_empty();
    let turns = stats.turns as usize;

    if turns >= 4 && stats.screw_pct >= 0.20 {
        let sources = ramp_source_count(deck);
        let suggestion = if sources < 6 {
            format!(
                "add {} two-mana rocks or land slots (only {sources} nonland ramp sources)",
                magnitude(stats.screw_pct * 100.0)
            )
        } else if sources >= 10 {
            // A rock-heavy deck screwing on color, not volume: more lands
            // will not help. The color_screw findings name the missing pips.
            "check the color_screw findings: the deck has enough nonland sources, so fix the missing colors (any-color sources, fixing lands)".to_string()
        } else {
            format!(
                "add {} land slots ({} nonland ramp sources already)",
                magnitude(stats.screw_pct * 100.0),
                sources
            )
        };
        problems.push(Problem {
            kind: "mana_screw",
            severity: severity(stats.screw_pct * 100.0),
            pct_games: Some(stats.screw_pct * 100.0),
            color: None,
            detail: format!(
                "{:.1}% of games had 2 or fewer lands by turn 4",
                stats.screw_pct * 100.0
            ),
            suggestion,
        });
    }
    // Flood fires when the rate sits well above the velocity-adjusted
    // expectation (the 11-card baseline is meaningless for cantrip
    // decks, and a rate at or under the expectation is no finding at
    // all). Lands-matter decks flood by design: their finding reads as
    // an observation, never a trim instruction.
    if turns >= 4 && stats.flood_pct >= 0.20 {
        let lands_matter = deck.cards.iter().any(|c| c.extra_land_drops)
            || deck.commanders.iter().any(|c| c.extra_land_drops);
        let detail = format!(
            "{:.1}% of games saw 6+ lands by turn 4 (expectation at the deck's actual draw volume: {:.1}%)",
            stats.flood_pct * 100.0,
            stats.flood_expectation * 100.0
        );
        // At or under the expectation is not a finding: the deck draws
        // its share of lands, no trim implied.
        if stats.flood_pct > stats.flood_expectation + 0.10 {
            let lands_matter_note = if lands_matter {
                " (lands-matter deck: check the plan before trimming)"
            } else {
                ""
            };
            problems.push(Problem {
                kind: "mana_flood",
                severity: severity(stats.flood_pct * 100.0),
                pct_games: Some(stats.flood_pct * 100.0),
                color: None,
                detail,
                suggestion: format!(
                    "trim {} land slots toward the curve{lands_matter_note}",
                    magnitude(stats.flood_pct * 100.0)
                ),
            });
        }
    }
    if commander && turns >= 4 {
        let cmc_turn =
            (deck.commanders.first().map(|c| c.cost.total()).unwrap_or(0) as usize).clamp(1, turns);
        let by_curve = stats.commander_castable_by[cmc_turn.min(12)];
        if by_curve < 0.60 {
            problems.push(Problem {
                kind: "commander_late",
                severity: severity((1.0 - by_curve) * 100.0),
                pct_games: Some((1.0 - by_curve) * 100.0),
                color: None,
                detail: format!(
                    "commander castable by turn {cmc_turn} in only {:.1}% of games",
                    by_curve * 100.0
                ),
                suggestion: "add 2-3 ramp sources or lower the early curve".to_string(),
            });
        }
    }
    /// True for the unlimited basic land names (the same exemption
    /// `deck update`'s singleton guard uses).
    fn is_basic_name(name: &str) -> bool {
        matches!(
            name,
            "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" | "Wastes"
        ) || name.starts_with("Snow-Covered")
    }

    // Color screw: any color pip missed in 10%+ of games.
    let basic_count = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Land && is_basic_name(&c.name))
        .count();
    for (i, pct) in stats.color_screw.iter().enumerate() {
        if *pct >= 0.10 {
            // Rank the fix by what the deck lacks. At few basics the
            // standard "swap basics" advice is wrong: the deck's fix is
            // any-color sources (rainbow lands, Fellwar-class rocks,
            // Prism-class banks).
            let (few_sources, choice_sources, shape) = color_source_shape(deck, i);
            let suggestion = if basic_count <= 8 {
                "add any-color sources (rainbow lands, Fellwar-class rocks, or banked-pip artifacts)".to_string()
            } else if choice_sources > 0 {
                format!(
                    "swap basics for lands that also tap for {} (choice sources exist but basics still dominate)",
                    COLORS[i],
                )
            } else {
                format!(
                    "add ~2-3 {} sources (dual lands that tap for {} beat more basics)",
                    COLORS[i], COLORS[i],
                )
            };
            problems.push(Problem {
                kind: "color_screw",
                severity: severity(pct * 100.0),
                pct_games: Some(pct * 100.0),
                color: Some(COLORS[i]),
                detail: format!(
                    "{} mana pips missed in {:.1}% of games (enough total mana, wrong colors; {} dedicated {} source{}, {} choice land{})",
                    COLORS[i],
                    pct * 100.0,
                    few_sources,
                    shape,
                    if few_sources == 1 { "" } else { "s" },
                    choice_sources,
                    if choice_sources == 1 { "" } else { "s" }
                ),
                suggestion,
            });
        }
    }
    let draw_turn = if commander { 6 } else { 5 };
    if turns >= draw_turn && stats.starved_pct >= 0.25 {
        problems.push(Problem {
            kind: "draw_starvation",
            severity: severity(stats.starved_pct * 100.0),
            pct_games: Some(stats.starved_pct * 100.0),
            color: None,
            detail: format!(
                "{:.1}% of games saw no draw source by turn {draw_turn}",
                stats.starved_pct * 100.0
            ),
            suggestion: "add 2-3 draw engines".to_string(),
        });
    }
    if turns >= 6 && stats.unused_mana[5] >= 2.5 {
        problems.push(Problem {
            kind: "mana_unused",
            severity: severity((stats.unused_mana[5] / 2.5) * 100.0),
            pct_games: None,
            color: None,
            detail: format!(
                "{:.1} mana left unspent on average by turn 6",
                stats.unused_mana[5]
            ),
            suggestion: "add cheaper spells or more card draw to spend the mana".to_string(),
        });
    }
    let threshold = if commander { 0.60 } else { 0.55 };
    // Board-discount cards (improvise, affinity) cast far earlier in real
    // games than the parse-time floor implies; exempt them from the
    // dead-cards finding. Their castability rows still show the curve.
    let discount_names: std::collections::HashSet<&str> = deck
        .cards
        .iter()
        .filter(|c| c.board_discount)
        .map(|c| c.name.as_str())
        .collect();
    // Reactive spells (removal, fogs, protection) never fire in a
    // goldfish: their castability row measures the mana base, not the
    // card. Exempt them from the dead-cards finding; role_access judges
    // them instead.
    let reactive_names: std::collections::HashSet<&str> = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Removal)
        .map(|c| c.name.as_str())
        .collect();
    let dead: Vec<&CardCast> = stats
        .card_castability
        .iter()
        .filter(|c| {
            c.pct_by_target < threshold
                && !discount_names.contains(c.name.as_str())
                && !reactive_names.contains(c.name.as_str())
        })
        .collect();
    // Distinct names only: a 4-of reports once, not four times.
    let dead_names: Vec<String> = dead
        .iter()
        .map(|c| c.name.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if dead_names.len() >= 3 {
        // Worst by average castability across each card's copies.
        let mut worst: Vec<(&String, f64)> = dead_names
            .iter()
            .map(|name| {
                let rows: Vec<f64> = dead
                    .iter()
                    .filter(|c| &c.name == name)
                    .map(|c| c.pct_by_target)
                    .collect();
                (name, rows.iter().sum::<f64>() / rows.len().max(1) as f64)
            })
            .collect();
        worst.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        worst.truncate(3);
        let names: Vec<String> = worst.iter().map(|(n, _)| (*n).clone()).collect();
        problems.push(Problem {
            kind: "dead_cards",
            severity: severity(dead_names.len() as f64 * 5.0),
            pct_games: None,
            color: None,
            detail: format!(
                "{} cards cast on time under {:.0}%; worst: {}",
                dead_names.len(),
                threshold * 100.0,
                names.join(", ")
            ),
            suggestion: "cut or discount late cards, or add ramp".to_string(),
        });
    }
    if turns >= 5 && stats.removal_count > 0 && stats.removal_access_5 < 0.40 {
        problems.push(Problem {
            kind: "category_starved",
            severity: severity((1.0 - stats.removal_access_5) * 100.0),
            pct_games: Some((1.0 - stats.removal_access_5) * 100.0),
            color: None,
            detail: format!(
                "removal seen by turn 5 in only {:.1}% of games ({} copies)",
                stats.removal_access_5 * 100.0,
                stats.removal_count
            ),
            suggestion: "add 2-3 interaction pieces".to_string(),
        });
    }
    if commander && turns >= 8 && stats.wincon_count > 0 && stats.wincon_access_8 < 0.40 {
        problems.push(Problem {
            kind: "category_starved",
            severity: severity((1.0 - stats.wincon_access_8) * 100.0),
            pct_games: Some((1.0 - stats.wincon_access_8) * 100.0),
            color: None,
            detail: format!(
                "win conditions seen by turn 8 in only {:.1}% of games ({} copies)",
                stats.wincon_access_8 * 100.0,
                stats.wincon_count
            ),
            suggestion: "add 1-2 win conditions or more draw".to_string(),
        });
    }
    // Interaction readiness: access is fine but the answer is rarely
    // affordable with spare mana. Capacity, not events.
    if turns >= 5
        && stats.interaction_instant_count > 0
        && stats.interaction_ready_by_turn[4] < 0.40
        && stats.removal_access_5 >= 0.40
    {
        problems.push(Problem {
            kind: "interaction_unready",
            severity: severity((1.0 - stats.interaction_ready_by_turn[4]) * 100.0),
            pct_games: Some((1.0 - stats.interaction_ready_by_turn[4]) * 100.0),
            color: None,
            detail: format!(
                "instant-speed interaction ready (in hand + affordable) by turn 5 in only {:.1}% of games ({} instant-speed copies)",
                stats.interaction_ready_by_turn[4] * 100.0,
                stats.interaction_instant_count
            ),
            suggestion: "add cheaper instant-speed answers".to_string(),
        });
    }
    problems
}
