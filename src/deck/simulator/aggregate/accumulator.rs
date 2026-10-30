//! Streaming reduction with exact scalar samples for conditional percentiles.

use super::*;
use std::collections::HashMap;

use super::super::findings::PipBlock;
use super::super::hypgeo::flood_expectation;
use super::super::model::{COLORS, Role};

/// Bounded game-log reduction. Only scalar percentile samples grow with runs.
pub struct SimAccumulator<'a> {
    deck: &'a SimDeck,
    stats: SimStats,
    flood_cache: HashMap<u32, f64>,
    drops: Vec<u32>,
    commander_turns: Vec<u32>,
    station_turns: Vec<u32>,
    companion_turns: Vec<u32>,
    win_turns: Vec<u32>,
    lethal_turns: Vec<u32>,
    attack_power: Vec<u32>,
    commander_threshold_turns: Vec<Vec<u32>>,
    pip_blocks: HashMap<(&'a str, usize), u32>,
    seen_blocks: std::collections::HashSet<(&'a str, usize)>,
    card_groups: Vec<Vec<usize>>,
    card_sums: Vec<u64>,
    ready_turns: usize,
    held_mana: f64,
    lands: usize,
}

impl<'a> SimAccumulator<'a> {
    /// Prepare deck-local counters before any game runs.
    pub fn new(deck: &'a SimDeck, turns: u32) -> Self {
        assert!(turns > 0, "aggregation requires at least one player turn");
        let mut stats = empty_stats(deck, turns);
        let mut by_name = std::collections::BTreeMap::<&str, Vec<usize>>::new();
        for (index, card) in deck.cards.iter().enumerate() {
            if card.role != Role::Land {
                by_name.entry(&card.name).or_default().push(index);
            }
        }
        let mut card_groups = Vec::new();
        for (name, copies) in by_name {
            let card = &deck.cards[copies[0]];
            stats.card_castability.push(CardCast {
                name: name.to_string(),
                cmc: f64::from(card.cost.total()),
                target_turn: card.min_cost.total().max(1),
                pct_by_target: 0.0,
                avg_first_castable_turn: 0.0,
                payable_games: 0,
            });
            card_groups.push(copies);
        }
        Self {
            deck,
            stats,
            flood_cache: HashMap::new(),
            card_sums: vec![0; card_groups.len()],
            card_groups,
            drops: Vec::new(),
            commander_turns: Vec::new(),
            station_turns: Vec::new(),
            companion_turns: Vec::new(),
            win_turns: Vec::new(),
            lethal_turns: Vec::new(),
            attack_power: Vec::new(),
            commander_threshold_turns: vec![Vec::new(); deck.commanders.len()],
            pip_blocks: HashMap::new(),
            seen_blocks: std::collections::HashSet::new(),
            ready_turns: 0,
            held_mana: 0.0,
            lands: deck
                .library_cards()
                .filter(|card| card.role == Role::Land)
                .count(),
        }
    }

    /// Consume one game's measurements without retaining or cloning its log.
    pub fn push(&mut self, log: &GameLog) {
        self.stats.runs += 1;
        let raw = &mut self.stats.raw_opponent_activity;
        raw.draws += u64::from(log.opponent_activity.draws);
        raw.casts += u64::from(log.opponent_activity.casts);
        raw.payments += u64::from(log.opponent_activity.payments);
        raw.skipped_casts += u64::from(log.opponent_activity.skipped_casts);
        super::events::record(&mut self.stats, log, self.deck);
        match &log.completion {
            super::super::game::GameCompletion::HorizonReached => {
                self.stats.finished_games += 1;
            }
            super::super::game::GameCompletion::PlayerLost { .. } => {
                self.stats.finished_games += 1;
                self.stats.player_loss_games += 1;
            }
            super::super::game::GameCompletion::Incomplete { .. } => {
                self.stats.incomplete_games += 1;
                self.stats.infinite_mana_pct += f64::from(log.infinite_mana_suspected);
                for (turn, row) in log
                    .milestones_by_turn
                    .iter()
                    .take(self.stats.turns as usize)
                    .enumerate()
                {
                    self.stats.positive_mana_loop_by_turn[turn] +=
                        f64::from(row.positive_mana_loop);
                }
                if let super::super::game::GameCompletion::Incomplete {
                    reason,
                    turn,
                    phase,
                } = &log.completion
                {
                    *self
                        .stats
                        .incomplete_observations
                        .entry((*phase, *turn, *reason))
                        .or_default() += 1;
                    let label = match reason {
                        super::super::game::IncompleteReason::CastPass { .. } => "cast_pass_limit",
                        super::super::game::IncompleteReason::GraveyardCastPass { .. } => {
                            "graveyard_cast_pass_limit"
                        }
                        super::super::game::IncompleteReason::EffectWork { .. } => {
                            "effect_work_limit"
                        }
                        super::super::game::IncompleteReason::TriggerSettlement { .. } => {
                            "trigger_settlement_limit"
                        }
                        super::super::game::IncompleteReason::TurnAction { .. } => {
                            "turn_action_limit"
                        }
                        super::super::game::IncompleteReason::ActivationPass { .. } => {
                            "activation_pass_limit"
                        }
                    };
                    *self
                        .stats
                        .incomplete_reasons
                        .entry(label.to_string())
                        .or_default() += 1;
                }
                return;
            }
        }
        for (counts, observation) in self
            .stats
            .phase_observations
            .iter_mut()
            .zip(&log.turn_observations)
        {
            counts.draw += u32::from(observation.draw);
            counts.main += u32::from(observation.main);
            counts.combat += u32::from(observation.combat);
            counts.end_step += u32::from(observation.end_step);
            counts.end_turn += u32::from(observation.end_turn);
        }
        self.stats.avg_opener_lands += f64::from(log.opener_lands);
        self.stats.mulligan_rate += f64::from(log.mulliganed);
        self.stats.opener_pct[(log.opener_lands as usize).min(5)] += 1.0;
        self.stats.replay_casts_avg += f64::from(log.replay_casts);
        self.stats.life_paid_avg += f64::from(log.life_paid);
        self.stats.life_gained_avg += f64::from(log.life_gained);
        self.stats.life_funded_draws_avg += f64::from(log.life_funded_draws);
        self.stats.opponent_draws += u64::from(log.opponent_activity.draws);
        self.stats.opponent_casts += u64::from(log.opponent_activity.casts);
        self.stats.opponent_payments += u64::from(log.opponent_activity.payments);
        self.stats.opponent_skipped_casts += u64::from(log.opponent_activity.skipped_casts);
        self.land_stats(log);
        self.timing_stats(log);
        self.turn_stats(log);
        self.commander_combat_stats(log);
        self.role_stats(log);
        self.card_stats(log);
        self.color_stats(log);
    }

    /// Use each game's actual draw volume while caching fixed-deck expectations.
    fn land_stats(&mut self, log: &GameLog) {
        let mut made = 0u32;
        for (turn, drops) in log
            .land_drops
            .iter()
            .take(self.stats.turns.min(5) as usize)
            .enumerate()
        {
            made += u32::from(*drops);
            if made > turn as u32 {
                self.stats.hit_all_drops_by[turn + 1] += 1.0;
            }
        }
        if self.stats.turns >= 4 {
            self.drops.push(u32::from(log.lands_by_4));
            self.stats.screw_pct += f64::from(log.lands_by_4 <= 2);
            self.stats.flood_pct += f64::from(log.lands_seen_by_turn_4 >= 6);
            self.stats.flood_expectation += *self
                .flood_cache
                .entry(log.cards_seen_by_4)
                .or_insert_with(|| {
                    flood_expectation(
                        self.lands,
                        self.deck.library_len(),
                        log.cards_seen_by_4 as usize,
                    )
                });
        }
        if self.stats.turns >= 6 {
            self.stats.floated_pct +=
                f64::from(log.mana_remaining.get(5).is_some_and(|mana| *mana >= 3.0));
        }
    }

    /// Record conditional first-event samples without inventing absent turns.
    fn timing_stats(&mut self, log: &GameLog) {
        let horizon = self.stats.turns;
        if let Some(turn) = log.commander_castable.filter(|turn| *turn <= horizon) {
            self.commander_turns.push(turn);
            for by in turn.max(1)..=horizon {
                self.stats.commander_castable_by[by as usize] += 1.0;
            }
        }
        if self
            .deck
            .commanders
            .first()
            .is_some_and(|card| card.animate_at().is_some())
        {
            self.station_turns
                .extend(log.station_online.filter(|turn| *turn <= horizon));
            self.stats.station_online_pct +=
                f64::from(horizon >= 6 && log.station_online.is_some_and(|turn| turn <= 6));
        }
        if self.deck.companion.is_some() {
            self.companion_turns
                .extend(log.companion_online.filter(|turn| *turn <= horizon));
            self.stats.companion_online_pct +=
                f64::from(horizon >= 6 && log.companion_online.is_some_and(|turn| turn <= 6));
        }
        self.win_turns
            .extend(log.win_threshold_turn.filter(|turn| *turn <= horizon));
        self.stats.win_threshold_pct +=
            f64::from(log.win_threshold_turn.is_some_and(|turn| turn <= horizon));
        self.stats.ultimate_online_pct +=
            f64::from(log.ultimate_online.is_some_and(|turn| turn <= horizon));
        self.stats.infinite_mana_pct += f64::from(log.infinite_mana_suspected);
        self.stats.extra_turns_pct += f64::from(
            log.extra_turns
                .iter()
                .take(horizon as usize)
                .any(|count| *count > 0),
        );
        if horizon >= 8 && log.turn_observations.get(7).is_some_and(|turn| turn.combat) {
            self.attack_power.extend(log.attack_power.get(7).copied());
        }
    }

    /// Sum turn values in game order, then divide only during finalization.
    fn turn_stats(&mut self, log: &GameLog) {
        for (turn, observation) in log
            .turn_observations
            .iter()
            .take(self.stats.turns as usize)
            .enumerate()
        {
            if observation.main {
                self.stats.unused_mana[turn] += log.mana_remaining[turn];
                self.stats.mana_available[turn] += log.mana_available[turn];
                self.stats.mana_spent_on_casts[turn] += log.mana_spent[turn];
                self.stats.interaction_ready_by_turn[turn] +=
                    f64::from(log.interaction_ready[turn]);
            }
            if observation.combat {
                self.stats.attack_power_by_turn[turn] += f64::from(log.attack_power[turn]);
                self.stats.evasive_by_turn[turn] += f64::from(log.evasive[turn]);
                self.stats.attackers_by_turn[turn] += f64::from(log.attackers[turn]);
            }
            if observation.end_turn {
                self.stats.cards_seen[turn] += f64::from(log.cards_seen[turn]);
                self.stats.creatures_by_turn[turn] += f64::from(log.creatures[turn]);
                self.stats.repeatable_sources_by_turn[turn] +=
                    f64::from(log.repeatable_sources_online[turn]);
                self.stats.library_by_turn[turn] += f64::from(log.library_size[turn]);
                self.stats.self_milled_by_turn[turn] += f64::from(log.self_milled[turn]);
                self.stats.opp_milled_by_turn[turn] += f64::from(log.opp_milled[turn]);
                self.stats.library_awareness_by_turn[turn] += log.awareness[turn];
                self.stats.opponent_life_loss_by_turn[turn] +=
                    f64::from(log.opponent_life_loss[turn]);
                self.stats.player_damage_by_turn[turn] += f64::from(log.player_damage[turn]);
                self.stats.graveyard_by_turn[turn] += f64::from(log.graveyard_size[turn]);
            }
        }
        let mut first_lethal = None;
        let mut reached_lethal = false;
        for (turn, (damage, loss)) in log
            .player_damage
            .iter()
            .zip(&log.opponent_life_loss)
            .take(self.stats.turns as usize)
            .enumerate()
        {
            reached_lethal |=
                f64::from(*damage) + f64::from(*loss) >= self.deck.rules.life_target();
            if reached_lethal {
                self.stats.lethal_damage_by_turn[turn] += 1.0;
                first_lethal.get_or_insert(turn as u32 + 1);
            }
        }
        self.lethal_turns.extend(first_lethal);
        for (turn, (ready, held)) in log
            .interaction_ready
            .iter()
            .zip(&log.interaction_mana_held)
            .take(self.stats.turns as usize)
            .enumerate()
        {
            if *ready && log.turn_observations[turn].main {
                self.ready_turns += 1;
                self.held_mana += held;
            }
        }
        self.milestone_stats(log);
    }

    /// Keep each commander's potential damage separate from other combat output.
    fn commander_combat_stats(&mut self, log: &GameLog) {
        for slot in 0..self.deck.commanders.len() {
            let mut cumulative = 0u32;
            let mut threshold_turn = None;
            for (turn, damage) in log.commander_combat_damage[slot]
                .iter()
                .take(self.stats.turns as usize)
                .enumerate()
            {
                cumulative = cumulative.saturating_add(*damage);
                if cumulative >= 21 && threshold_turn.is_none() {
                    threshold_turn = Some(turn as u32 + 1);
                }
                if !log.turn_observations[turn].combat {
                    continue;
                }
                self.stats.commander_combat_by_turn[slot][turn] += f64::from(*damage);
                self.stats.commander_combat_cumulative_by_turn[slot][turn] += f64::from(cumulative);
            }
            if let Some(turn) = threshold_turn {
                self.stats.commander_combat_threshold_games[slot] += 1;
                self.commander_threshold_turns[slot].push(turn);
            }
        }
    }

    /// Count actual actions separately from card access and role metadata.
    fn milestone_stats(&mut self, log: &GameLog) {
        for (turn, row) in log
            .milestones_by_turn
            .iter()
            .take(self.stats.turns as usize)
            .enumerate()
        {
            if !log.turn_observations[turn].main {
                continue;
            }
            self.stats.free_cast_permanents_by_turn[turn] +=
                f64::from(row.free_cast_permanents_entered);
            self.stats.dredge_uses_by_turn[turn] += f64::from(row.dredge_uses);
            self.stats.graveyard_casts_by_turn[turn] += f64::from(row.graveyard_casts);
            self.stats.life_funded_draws_by_turn[turn] += f64::from(row.life_funded_draws);
            self.stats.positive_mana_loop_by_turn[turn] += f64::from(row.positive_mana_loop);
        }
    }

    /// Fixed-window role access requires the whole declared turn window.
    fn role_stats(&mut self, log: &GameLog) {
        let seen = |role, turn| {
            self.stats.turns >= turn
                && log
                    .first_seen
                    .get(&role)
                    .is_some_and(|first| *first <= turn)
        };
        let draw = seen(Role::Draw, 6);
        self.stats.starved_pct += f64::from(self.stats.turns >= 6 && !draw);
        self.stats.draw_access_6 += f64::from(draw);
        self.stats.removal_access_5 += f64::from(seen(Role::Removal, 5));
        self.stats.wincon_access_8 += f64::from(seen(Role::Wincon, 8));
        self.stats.lock_access_3 += f64::from(seen(Role::Lock, 3));
        self.stats.creature_access_3 +=
            f64::from(self.stats.turns >= 3 && log.first_creature.is_some_and(|turn| turn <= 3));
        if let Some(turn) = log.first_seen.get(&Role::Draw) {
            for share in self
                .stats
                .draw_sources
                .iter_mut()
                .skip(turn.saturating_sub(1) as usize)
            {
                *share += 1.0;
            }
        }
    }

    /// Measure at least one payable copy per whole game, not independent copies.
    fn card_stats(&mut self, log: &GameLog) {
        for ((copies, row), sum) in self
            .card_groups
            .iter()
            .zip(&mut self.stats.card_castability)
            .zip(&mut self.card_sums)
        {
            let first = copies
                .iter()
                .filter_map(|index| log.mana_ready.get(*index).copied().flatten())
                .filter(|turn| *turn <= self.stats.turns)
                .min();
            if let Some(turn) = first {
                row.payable_games += 1;
                row.pct_by_target += f64::from(turn <= row.target_turn);
                *sum += u64::from(turn);
            }
        }
    }

    /// Count each card-name and color block once per game, including duplicates.
    fn color_stats(&mut self, log: &GameLog) {
        for (sum, blocked) in self.stats.color_screw.iter_mut().zip(log.blocked_colors) {
            *sum += f64::from(blocked);
        }
        self.seen_blocks.clear();
        for (index, color) in &log.pip_blocks {
            let key = (self.deck.cards[*index].name.as_str(), *color);
            if self.seen_blocks.insert(key) {
                *self.pip_blocks.entry(key).or_default() += 1;
            }
        }
    }

    /// Finalize averages and exact nearest-rank conditional percentiles.
    pub fn finish(mut self) -> SimStats {
        let n = f64::from(self.stats.finished_games).max(1.0);
        divide_sums(&mut self.stats, n);
        self.stats.infinite_mana_pct /= f64::from(self.stats.runs).max(1.0);
        for value in &mut self.stats.positive_mana_loop_by_turn {
            *value /= f64::from(self.stats.runs).max(1.0);
        }
        for (turn, counts) in self.stats.phase_observations.iter().enumerate() {
            for value in [
                &mut self.stats.unused_mana[turn],
                &mut self.stats.mana_available[turn],
                &mut self.stats.mana_spent_on_casts[turn],
                &mut self.stats.free_cast_permanents_by_turn[turn],
                &mut self.stats.dredge_uses_by_turn[turn],
                &mut self.stats.graveyard_casts_by_turn[turn],
                &mut self.stats.life_funded_draws_by_turn[turn],
            ] {
                *value /= f64::from(counts.main).max(1.0);
            }
            for value in [
                &mut self.stats.attack_power_by_turn[turn],
                &mut self.stats.evasive_by_turn[turn],
                &mut self.stats.attackers_by_turn[turn],
            ] {
                *value /= f64::from(counts.combat).max(1.0);
            }
            for slot in 0..self.deck.commanders.len() {
                self.stats.commander_combat_by_turn[slot][turn] /=
                    f64::from(counts.combat).max(1.0);
                self.stats.commander_combat_cumulative_by_turn[slot][turn] /=
                    f64::from(counts.combat).max(1.0);
            }
            for value in [
                &mut self.stats.cards_seen[turn],
                &mut self.stats.creatures_by_turn[turn],
                &mut self.stats.repeatable_sources_by_turn[turn],
                &mut self.stats.library_by_turn[turn],
                &mut self.stats.self_milled_by_turn[turn],
                &mut self.stats.opp_milled_by_turn[turn],
                &mut self.stats.library_awareness_by_turn[turn],
                &mut self.stats.opponent_life_loss_by_turn[turn],
                &mut self.stats.player_damage_by_turn[turn],
                &mut self.stats.graveyard_by_turn[turn],
            ] {
                *value /= f64::from(counts.end_turn).max(1.0);
            }
        }
        for value in &mut self.stats.interaction_ready_by_turn {
            *value /= n;
        }
        self.finish_timings();
        self.stats.interaction_mana_held = self.held_mana / self.ready_turns.max(1) as f64;
        self.stats.interaction_ready_turns = self.ready_turns;
        for (row, sum) in self.stats.card_castability.iter_mut().zip(self.card_sums) {
            row.pct_by_target /= n;
            row.avg_first_castable_turn = sum as f64 / f64::from(row.payable_games).max(1.0);
        }
        self.stats.pip_blocks = self
            .pip_blocks
            .into_iter()
            .map(|((name, color), count)| PipBlock {
                name: name.to_string(),
                color: COLORS[color],
                game_share: f64::from(count) / n,
            })
            .collect();
        self.stats.pip_blocks.sort_by(|a, b| {
            b.game_share
                .total_cmp(&a.game_share)
                .then(a.name.cmp(&b.name))
                .then(a.color.symbol().cmp(&b.color.symbol()))
        });
        self.stats.pip_blocks.truncate(5);
        self.stats.color_screw_pct = self.stats.color_screw.iter().copied().fold(0.0, f64::max);
        self.stats
    }

    /// Sort every scalar sample once, including samples used by two quantiles.
    fn finish_timings(&mut self) {
        self.drops.sort_unstable();
        self.stats.p50_drops_by_4 = sorted_percentile(&self.drops, 0.5);
        self.stats.p95_drops_by_4 = sorted_percentile(&self.drops, 0.95);
        self.commander_turns.sort_unstable();
        self.stats.commander_payable_games = self.commander_turns.len() as u32;
        self.stats.avg_commander_cast_turn = self
            .commander_turns
            .iter()
            .map(|turn| f64::from(*turn))
            .sum::<f64>()
            / self.commander_turns.len().max(1) as f64;
        self.stats.p50_commander_cast_turn = sorted_percentile(&self.commander_turns, 0.5);
        self.stats.p95_commander_cast_turn = sorted_percentile(&self.commander_turns, 0.95);
        self.stats.station_online_games = self.station_turns.len() as u32;
        self.stats.station_p50_turn = percentile(&mut self.station_turns, 0.5);
        self.stats.companion_online_games = self.companion_turns.len() as u32;
        self.stats.companion_p50_turn = percentile(&mut self.companion_turns, 0.5);
        self.stats.win_threshold_games = self.win_turns.len() as u32;
        self.stats.win_threshold_p50_turn = percentile(&mut self.win_turns, 0.5);
        self.stats.lethal_games = self.lethal_turns.len() as u32;
        self.stats.p50_lethal_turn =
            (!self.lethal_turns.is_empty()).then(|| percentile(&mut self.lethal_turns, 0.5));
        self.stats.attack_power_p90 = percentile(&mut self.attack_power, 0.9);
        self.stats.commander_combat_threshold_p50 = self
            .commander_threshold_turns
            .iter_mut()
            .map(|turns| (!turns.is_empty()).then(|| percentile(turns, 0.5)))
            .collect();
    }
}

/// Allocate fixed-horizon buffers once, outside the game-reduction loop.
fn empty_stats(deck: &SimDeck, turns: u32) -> SimStats {
    let mut stats = SimStats {
        turns,
        ..SimStats::default()
    };
    macro_rules! vectors { ($($field:ident),* $(,)?) => { $(stats.$field = vec![0.0; stats.turns as usize];)* }; }
    vectors!(
        unused_mana,
        mana_available,
        mana_spent_on_casts,
        cards_seen,
        draw_sources,
        graveyard_by_turn,
        creatures_by_turn,
        repeatable_sources_by_turn,
        attack_power_by_turn,
        evasive_by_turn,
        attackers_by_turn,
        library_by_turn,
        self_milled_by_turn,
        opp_milled_by_turn,
        library_awareness_by_turn,
        opponent_life_loss_by_turn,
        player_damage_by_turn,
        lethal_damage_by_turn,
        interaction_ready_by_turn,
        free_cast_permanents_by_turn,
        dredge_uses_by_turn,
        graveyard_casts_by_turn,
        life_funded_draws_by_turn,
        positive_mana_loop_by_turn
    );
    stats.commander_castable_by = vec![0.0; stats.turns as usize + 1];
    stats.phase_observations = vec![PhaseObservationCounts::default(); stats.turns as usize];
    stats.commander_combat_by_turn = vec![vec![0.0; stats.turns as usize]; deck.commanders.len()];
    stats.commander_combat_cumulative_by_turn =
        vec![vec![0.0; stats.turns as usize]; deck.commanders.len()];
    stats.commander_combat_threshold_games = vec![0; deck.commanders.len()];
    stats.commander_combat_threshold_p50 = vec![None; deck.commanders.len()];
    stats.interaction_instant_count = deck
        .cards
        .iter()
        .filter(|card| card.flags.is_interaction && card.flags.is_instant_speed)
        .count();
    stats
}

/// Scale summed measurements once, after ordered game reduction.
fn divide_sums(stats: &mut SimStats, n: f64) {
    macro_rules! scalars { ($($field:ident),* $(,)?) => { $(stats.$field /= n;)* }; }
    scalars!(
        avg_opener_lands,
        mulligan_rate,
        screw_pct,
        flood_pct,
        flood_expectation,
        floated_pct,
        starved_pct,
        removal_access_5,
        draw_access_6,
        creature_access_3,
        wincon_access_8,
        lock_access_3,
        replay_casts_avg,
        life_paid_avg,
        life_gained_avg,
        life_funded_draws_avg,
        station_online_pct,
        companion_online_pct,
        extra_turns_pct,
        win_threshold_pct,
        ultimate_online_pct,
    );
    macro_rules! vectors { ($($field:ident),* $(,)?) => { $(for value in &mut stats.$field { *value /= n; })* }; }
    vectors!(
        opener_pct,
        hit_all_drops_by,
        commander_castable_by,
        color_screw,
        draw_sources,
        lethal_damage_by_turn,
    );
}
