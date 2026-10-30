//! Store-backed combo assembly: join the deck's simulated cards against the
//! Spellbook `combos`/`combo_pieces` tables and measure how often each
//! variant's pieces reach their required zones by the target turn.
//!
//! This is a consistency diagnostic, not a combo-quality judgment: the sim
//! says how often the pieces come together, never whether the combo wins.
//! Quality labels (`produces`, bracket tag) come from Spellbook data.

use super::game::GameLog;
use super::model::SimDeck;
use super::report_schema::{ObservationStatus, RateEstimate};

/// One zone a combo piece must reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceZone {
    Battlefield,
    Hand,
    Graveyard,
    Command,
    Library,
    Exile,
}

impl PieceZone {
    /// Parse a Spellbook zone code.
    pub fn parse(code: &str) -> Option<PieceZone> {
        match code {
            "B" => Some(PieceZone::Battlefield),
            "H" => Some(PieceZone::Hand),
            "G" => Some(PieceZone::Graveyard),
            "C" => Some(PieceZone::Command),
            "L" => Some(PieceZone::Library),
            "E" => Some(PieceZone::Exile),
            _ => None,
        }
    }
}

/// One stored combo variant with its pieces, ready for assembly checks.
#[derive(Debug, Clone)]
pub struct ComboCandidate {
    pub id: String,
    pub produces: Vec<String>,
    pub mana_value_needed: u32,
    pub bracket_tag: Option<String>,
    /// Deck index of each piece (the piece's primary zone requirement).
    pub pieces: Vec<PieceRef>,
    /// True when the deck holds every piece.
    pub complete: bool,
    /// Names of the pieces the deck lacks (near-miss payload).
    pub missing: Vec<String>,
}

/// One piece resolved against the deck.
#[derive(Debug, Clone)]
pub struct PieceRef {
    pub name: String,
    /// Every matching physical copy in `SimDeck.cards`.
    pub indices: Vec<usize>,
    /// First-arrival evidence from the game's initial command zone.
    pub starts_in_command_zone: bool,
    /// First-arrival evidence from the game's initial shuffled library.
    pub starts_in_library: bool,
    pub zones: Vec<PieceZone>,
}

/// Assembly outcome for one variant in one game set.
#[derive(Debug, Clone)]
pub struct ComboAccess {
    /// Piece names joined with " + ".
    pub combo: String,
    /// Spellbook variant id.
    pub id: String,
    /// What the combo produces (first feature names, capped).
    pub produces: Vec<String>,
    /// Bracket tag letter (Spellbook's own classification).
    pub bracket_tag: Option<String>,
    /// Target turn: max(manaValueNeeded, highest piece mana value).
    pub target_turn: u32,
    /// Share of games where every piece reached a required zone in time.
    pub game_share: f64,
    /// Game-level first-arrival estimate; not simultaneous availability.
    pub access: RateEstimate,
    /// Names of the pieces the deck lacks (empty for complete combos).
    pub missing: Vec<String>,
}

/// Zone telemetry the assembly predicate reads from a game log.
/// First turn a card reached each zone in one game log. `None` means the
/// card never got there; the combo measurer treats that as "never".
struct ZoneTimes {
    hand: Option<u32>,
    battlefield: Option<u32>,
    graveyard: Option<u32>,
}

/// Read the per-card zone timeline for one deck index out of the log.
fn zone_times(log: &GameLog, index: usize) -> ZoneTimes {
    ZoneTimes {
        hand: log.card_first_seen.get(&index).copied(),
        battlefield: log.card_first_battlefield.get(&index).copied(),
        graveyard: log.card_first_graveyard.get(&index).copied(),
    }
}

/// Read historical access to any listed zone, including initial-zone evidence.
fn piece_assembled(log: &GameLog, piece: &PieceRef, target: u32) -> bool {
    piece.zones.iter().any(|z| match z {
        PieceZone::Hand => piece.indices.iter().any(|index| {
            zone_times(log, *index)
                .hand
                .is_some_and(|turn| turn <= target)
        }),
        PieceZone::Battlefield => piece.indices.iter().any(|index| {
            zone_times(log, *index)
                .battlefield
                .is_some_and(|turn| turn <= target)
        }),
        PieceZone::Graveyard => piece.indices.iter().any(|index| {
            zone_times(log, *index)
                .graveyard
                .is_some_and(|turn| turn <= target)
        }),
        PieceZone::Library => piece.starts_in_library,
        PieceZone::Command => piece.starts_in_command_zone,
        PieceZone::Exile => false,
    })
}

/// Build candidates from stored variants: resolve every piece to a deck
/// index, drop variants with unmodeled (exile-only) requirements, and tag
/// completeness.
pub fn candidates(
    variants: Vec<(
        crate::spellbook::ComboVariant,
        Vec<crate::spellbook::ComboPieceRow>,
    )>,
    deck: &SimDeck,
    format_key: &str,
) -> (Vec<ComboCandidate>, usize) {
    let mut out = Vec::new();
    let mut excluded = 0usize;
    let mut by_name = std::collections::BTreeMap::<&str, Vec<usize>>::new();
    for (index, card) in deck.cards.iter().enumerate() {
        by_name.entry(&card.name).or_default().push(index);
    }
    for (variant, rows) in variants {
        if !variant.legalities.get(format_key).copied().unwrap_or(false) {
            excluded += 1;
            continue;
        }
        let mut pieces = Vec::new();
        let mut missing = Vec::new();
        let mut has_unmodeled_zone = false;
        // Split faces share an ordinal. Prefer a held face, then its name.
        let mut by_ordinal: std::collections::BTreeMap<i64, &crate::spellbook::ComboPieceRow> =
            std::collections::BTreeMap::new();
        for row in &rows {
            let held = by_name.contains_key(row.name.as_str())
                || deck.commanders.iter().any(|card| card.name == row.name);
            by_ordinal
                .entry(row.ordinal)
                .and_modify(|slot| {
                    let slot_held = by_name.contains_key(slot.name.as_str())
                        || deck.commanders.iter().any(|card| card.name == slot.name);
                    if (held && !slot_held) || (held == slot_held && row.name < slot.name) {
                        *slot = row;
                    }
                })
                .or_insert(row);
        }
        for row in by_ordinal.values() {
            let zones: Vec<PieceZone> = row
                .zones
                .iter()
                .filter_map(|z| PieceZone::parse(z))
                .collect();
            // Exile-zone pieces have no modeled path to the zone; the
            // variant is excluded rather than misreported as 0%.
            if zones.is_empty() || zones.contains(&PieceZone::Exile) {
                has_unmodeled_zone = true;
                break;
            }
            let indices = by_name.get(row.name.as_str()).cloned().unwrap_or_default();
            let starts_in_command_zone = deck.commanders.iter().any(|card| card.name == row.name);
            let starts_in_library = indices
                .iter()
                .any(|index| deck.companion != Some(super::model::CardIdx(*index as u32)));
            if starts_in_command_zone && !zones.contains(&PieceZone::Command) {
                // Commander zone arrivals beyond the initial zone have no per-card telemetry.
                has_unmodeled_zone = true;
                break;
            }
            if (indices.is_empty() && !starts_in_command_zone)
                || (row.must_be_commander && !starts_in_command_zone)
            {
                missing.push(row.name.clone());
            }
            pieces.push(PieceRef {
                name: row.name.clone(),
                indices,
                starts_in_command_zone,
                starts_in_library,
                zones,
            });
        }
        if has_unmodeled_zone {
            excluded += 1;
            continue;
        }
        let Ok(mana_value_needed) = u32::try_from(variant.mana_value_needed.max(1)) else {
            excluded += 1;
            continue;
        };
        out.push(ComboCandidate {
            id: variant.id.clone(),
            produces: variant.produces.clone(),
            mana_value_needed,
            bracket_tag: variant.bracket_tag.clone(),
            pieces,
            complete: missing.is_empty(),
            missing,
        });
    }
    (out, excluded)
}

/// Measure assembly for every candidate: complete combos and near-misses.
///
/// A near-miss reports 0% (its pieces cannot assemble); the payload is the
/// missing name. `variants_considered` counts the candidates measured.
pub struct Assembly {
    pub complete: Vec<ComboAccess>,
    pub near_misses: Vec<ComboAccess>,
    pub variants_considered: usize,
}

/// Target turn for a variant: the later of the mana value needed and the
/// priciest held piece. An unobserved target is not moved into the horizon.
fn target_turn(candidate: &ComboCandidate, deck: &SimDeck) -> u32 {
    let piece_cmc = candidate
        .pieces
        .iter()
        .flat_map(|p| p.indices.iter().copied())
        .map(|i| deck.cards[i].cost.total())
        .chain(
            deck.commanders
                .iter()
                .filter(|card| candidate.pieces.iter().any(|piece| piece.name == card.name))
                .map(|card| card.cost.total()),
        )
        .max()
        .unwrap_or(0);
    candidate.mana_value_needed.max(piece_cmc).max(1)
}

/// Run the assembly measurement over the game logs.
#[cfg(test)]
pub fn measure(
    candidates: &[ComboCandidate],
    logs: &[GameLog],
    deck: &SimDeck,
    turns: u32,
) -> Assembly {
    let mut accumulator = ComboAccumulator::new(candidates, deck, turns);
    for log in logs {
        accumulator.push(log);
    }
    accumulator.finish()
}

/// Constant-size per-candidate access counts; full game logs are never retained.
pub struct ComboAccumulator<'a> {
    candidates: &'a [ComboCandidate],
    targets: Vec<u32>,
    hits: Vec<u32>,
    runs: u32,
    turns: u32,
}

impl<'a> ComboAccumulator<'a> {
    /// Resolve target turns before executing games.
    pub fn new(candidates: &'a [ComboCandidate], deck: &SimDeck, turns: u32) -> Self {
        Self {
            candidates,
            targets: candidates
                .iter()
                .map(|candidate| target_turn(candidate, deck))
                .collect(),
            hits: vec![0; candidates.len()],
            runs: 0,
            turns,
        }
    }

    /// Observe first zone arrivals, not simultaneous legal combo availability.
    pub fn push(&mut self, log: &GameLog) {
        if matches!(
            log.completion,
            super::game::GameCompletion::Incomplete { .. }
        ) {
            return;
        }
        self.runs += 1;
        for ((candidate, target), hits) in self
            .candidates
            .iter()
            .zip(&self.targets)
            .zip(&mut self.hits)
        {
            if candidate.complete
                && *target <= self.turns
                && candidate
                    .pieces
                    .iter()
                    .all(|piece| piece_assembled(log, piece, *target))
            {
                *hits += 1;
            }
        }
    }

    /// Build stable report rows with explicit unavailable statuses.
    pub fn finish(self) -> Assembly {
        let mut complete = Vec::new();
        let mut near_misses = Vec::new();
        for ((candidate, target), hits) in self.candidates.iter().zip(self.targets).zip(self.hits) {
            let combo = candidate
                .pieces
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(" + ");
            let access = if !candidate.complete {
                RateEstimate::unavailable(ObservationStatus::NotApplicable)
            } else if target > self.turns {
                RateEstimate::unavailable(ObservationStatus::OutsideHorizon)
            } else {
                RateEstimate::observed(hits, self.runs)
            };
            let row = ComboAccess {
                combo,
                id: candidate.id.clone(),
                produces: candidate.produces.clone(),
                bracket_tag: candidate.bracket_tag.clone(),
                target_turn: target,
                game_share: f64::from(hits) / f64::from(self.runs).max(1.0),
                access,
                missing: candidate.missing.clone(),
            };
            if candidate.complete {
                complete.push(row);
            } else {
                near_misses.push(row);
            }
        }
        near_misses.sort_by(|a, b| {
            a.missing
                .len()
                .cmp(&b.missing.len())
                .then(a.id.cmp(&b.id))
                .then(a.combo.cmp(&b.combo))
        });
        Assembly {
            complete,
            near_misses,
            variants_considered: self.candidates.len(),
        }
    }
}

/// Rank complete combos: assembly rate first, feature count second.
pub fn rank_complete(rows: &mut [ComboAccess]) {
    rows.sort_by(|a, b| {
        b.access
            .percent
            .is_some()
            .cmp(&a.access.percent.is_some())
            .then(
                b.game_share
                    .partial_cmp(&a.game_share)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(b.produces.len().cmp(&a.produces.len()))
            .then(a.id.cmp(&b.id))
            .then(a.combo.cmp(&b.combo))
    });
}
