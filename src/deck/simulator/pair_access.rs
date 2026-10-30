//! First-hand-access counts for explicit card pairs, without retaining game logs.

use super::{findings::ComboAccess, game::GameLog, model::SimDeck};

/// Per-pair counts. Access does not establish simultaneous or payable combo use.
pub(super) struct PairAccumulator {
    rows: Vec<ComboAccess>,
    indexes: Vec<(Vec<usize>, Vec<usize>)>,
    repeated_piece: Vec<bool>,
    hits: Vec<u32>,
    runs: u32,
    turns: u32,
}

impl PairAccumulator {
    /// Resolve card names and deadlines once before executing games.
    pub(super) fn new(deck: &SimDeck, pairs: &[(String, String)], turns: u32) -> Self {
        let mut rows = Vec::new();
        let mut indexes = Vec::new();
        let mut repeated_piece = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (first, second) in pairs {
            let mut first = first.trim();
            let mut second = second.trim();
            if first.to_ascii_lowercase() > second.to_ascii_lowercase() {
                std::mem::swap(&mut first, &mut second);
            }
            if !seen.insert((first.to_ascii_lowercase(), second.to_ascii_lowercase())) {
                continue;
            }
            let matching = |name: &str| {
                deck.cards
                    .iter()
                    .enumerate()
                    .filter(|(_, card)| card.name.eq_ignore_ascii_case(name))
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>()
            };
            let first_indexes = matching(first);
            let second_indexes = matching(second);
            let target_turn =
                first_indexes
                    .first()
                    .zip(second_indexes.first())
                    .map_or(0, |(first, second)| {
                        deck.cards[*first]
                            .cost
                            .total()
                            .max(deck.cards[*second].cost.total())
                            .max(1)
                    });
            let first_name = first_indexes
                .first()
                .map_or(first, |index| deck.cards[*index].name.as_str());
            let second_name = second_indexes
                .first()
                .map_or(second, |index| deck.cards[*index].name.as_str());
            rows.push(ComboAccess {
                pair: format!("{first_name} + {second_name}"),
                target_turn,
                game_share: 0.0,
                event_games: 0,
                observed_games: 0,
            });
            indexes.push((first_indexes, second_indexes));
            repeated_piece.push(first.eq_ignore_ascii_case(second));
        }
        Self {
            hits: vec![0; rows.len()],
            rows,
            indexes,
            repeated_piece,
            runs: 0,
            turns,
        }
    }

    /// Observe a game's first hand entries, not a claim of executable assembly.
    pub(super) fn push(&mut self, log: &GameLog) {
        if matches!(
            log.completion,
            super::game::GameCompletion::Incomplete { .. }
        ) {
            return;
        }
        self.runs += 1;
        for (((indexes, repeated), row), hits) in self
            .indexes
            .iter()
            .zip(&self.repeated_piece)
            .zip(&self.rows)
            .zip(&mut self.hits)
        {
            if row.target_turn == 0 || row.target_turn > self.turns {
                continue;
            }
            let seen = |index: &&usize| {
                log.card_first_seen
                    .get(*index)
                    .is_some_and(|seen| *seen <= row.target_turn)
            };
            let first_count = indexes.0.iter().filter(seen).count();
            let second_count = indexes.1.iter().filter(seen).count();
            let required = if *repeated { 2 } else { 1 };
            if first_count >= required && second_count > 0 {
                *hits += 1;
            }
        }
    }

    /// Convert integer event counts to shares after consuming all games.
    pub(super) fn finish(mut self) -> Vec<ComboAccess> {
        for (row, hits) in self.rows.iter_mut().zip(self.hits) {
            row.event_games = hits;
            row.observed_games = self.runs;
            row.game_share = if self.runs == 0 {
                0.0
            } else {
                f64::from(hits) / f64::from(self.runs)
            };
        }
        self.rows
    }
}

#[cfg(test)]
#[path = "tests/pair_access_tests.rs"]
mod pair_access_tests;
