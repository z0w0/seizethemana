// Store-backed combo assembly: join the deck's simulated cards against the
// Spellbook `combos`/`combo_pieces` tables and measure how often each
// variant's pieces reach their required zones by the target turn.
//
// This is a consistency diagnostic, not a combo-quality judgment: the sim
// says how often the pieces come together, never whether the combo wins.
// Quality labels (`produces`, bracket tag) come from Spellbook data.

use super::game::GameLog;
use super::model::SimDeck;

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
    /// Card index in `SimDeck.cards`; `None` when the deck lacks the card
    /// (the piece must be the commander or another deck lacks it).
    pub index: Option<usize>,
    pub zones: Vec<PieceZone>,
    pub must_be_commander: bool,
}

/// Assembly outcome for one variant in one game set.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ComboAccess {
    /// Piece names joined with " + ".
    pub combo: String,
    /// Spellbook variant id.
    pub id: String,
    /// What the combo produces (first feature names, capped).
    pub produces: Vec<String>,
    /// Bracket tag letter (Spellbook's own classification).
    pub bracket_tag: Option<String>,
    /// Target turn: max(manaValueNeeded, highest piece cmc), clamped.
    pub target_turn: u32,
    /// Share of games where every piece reached a required zone in time,
    /// serialized 0-100 (the JSON percent scale).
    #[serde(serialize_with = "crate::deck::simulator::report::serialize_pct")]
    pub pct_games: f64,
    /// Names of the pieces the deck lacks (empty for complete combos).
    pub missing: Vec<String>,
}

/// Zone telemetry the assembly predicate reads from a game log.
struct ZoneTimes {
    hand: Option<u32>,
    battlefield: Option<u32>,
    graveyard: Option<u32>,
}

fn zone_times(log: &GameLog, index: usize) -> ZoneTimes {
    ZoneTimes {
        hand: log.card_first_seen.get(&index).copied(),
        battlefield: log.card_first_battlefield.get(&index).copied(),
        graveyard: log.card_first_graveyard.get(&index).copied(),
    }
}

/// True when the piece reached one of its required zones by `target`. A
/// piece flagged `must_be_commander` also counts the command-zone cast
/// (deck-implied), which the sim records via the battlefield log.
fn piece_assembled(times: &ZoneTimes, piece: &PieceRef, target: u32) -> bool {
    if piece.must_be_commander {
        return times.battlefield.is_some_and(|t| t <= target);
    }
    piece.zones.iter().any(|z| match z {
        PieceZone::Hand => times.hand.is_some_and(|t| t <= target),
        PieceZone::Battlefield => times.battlefield.is_some_and(|t| t <= target),
        PieceZone::Graveyard => times.graveyard.is_some_and(|t| t <= target),
        PieceZone::Library => times.hand.is_some_and(|t| t <= target),
        // The command zone is not indexed per card; a command-zone piece
        // assembles when the commander was cast (deck-implied).
        PieceZone::Command => times.battlefield.is_some_and(|t| t <= target),
        // Exile pieces never pass candidate building; unreachable.
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
    for (variant, rows) in variants {
        if !variant.legalities.get(format_key).copied().unwrap_or(false) {
            excluded += 1;
            continue;
        }
        let mut pieces = Vec::new();
        let mut missing = Vec::new();
        let mut has_unmodeled_zone = false;
        // One slot per ordinal: a face split produces several rows with
        // the same ordinal. Prefer a face the deck holds; otherwise keep
        // the first-seen row.
        let mut by_ordinal: std::collections::BTreeMap<i64, &crate::spellbook::ComboPieceRow> =
            std::collections::BTreeMap::new();
        for row in &rows {
            let held = deck.cards.iter().any(|c| c.name == row.name);
            by_ordinal
                .entry(row.ordinal)
                .and_modify(|slot| {
                    if deck.cards.iter().any(|c| c.name == slot.name) {
                        // Already a held face.
                    } else if held {
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
            let index = deck.cards.iter().position(|c| c.name == row.name);
            // A piece that must be the commander resolves through the
            // command zone; it is never a "missing" deck card.
            if index.is_none() && !row.must_be_commander {
                missing.push(row.name.clone());
            }
            pieces.push(PieceRef {
                name: row.name.clone(),
                index,
                zones,
                must_be_commander: row.must_be_commander,
            });
        }
        if has_unmodeled_zone {
            excluded += 1;
            continue;
        }
        out.push(ComboCandidate {
            id: variant.id.clone(),
            produces: variant.produces.clone(),
            mana_value_needed: u32::try_from(variant.mana_value_needed.max(1)).unwrap_or(1),
            bracket_tag: variant.bracket_tag.clone(),
            pieces: pieces.clone(),
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
/// priciest held piece, clamped to 1..=turns.
fn target_turn(candidate: &ComboCandidate, deck: &SimDeck, turns: u32) -> u32 {
    let piece_cmc = candidate
        .pieces
        .iter()
        .filter_map(|p| p.index)
        .map(|i| deck.cards[i].cost.total())
        .max()
        .unwrap_or(0);
    candidate.mana_value_needed.max(piece_cmc).max(1).min(turns)
}

/// Run the assembly measurement over the game logs.
pub fn measure(
    candidates: &[ComboCandidate],
    logs: &[GameLog],
    deck: &SimDeck,
    turns: u32,
) -> Assembly {
    let n = logs.len() as f64;
    if n == 0.0 {
        return Assembly {
            complete: Vec::new(),
            near_misses: Vec::new(),
            variants_considered: candidates.len(),
        };
    }
    let mut complete = Vec::new();
    let mut near_misses = Vec::new();
    for candidate in candidates {
        let target = target_turn(candidate, deck, turns);
        let combo = candidate
            .pieces
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(" + ");
        let pct = if candidate.complete {
            let hits = logs
                .iter()
                .filter(|log| {
                    candidate.pieces.iter().all(|piece| {
                        piece
                            .index
                            .is_some_and(|i| piece_assembled(&zone_times(log, i), piece, target))
                    })
                })
                .count() as f64;
            hits / n
        } else {
            0.0
        };
        let row = ComboAccess {
            combo,
            id: candidate.id.clone(),
            produces: candidate.produces.iter().take(2).cloned().collect(),
            bracket_tag: candidate.bracket_tag.clone(),
            target_turn: target,
            pct_games: pct,
            missing: candidate.missing.clone(),
        };
        if candidate.complete {
            complete.push(row);
        } else {
            near_misses.push(row);
        }
    }
    Assembly {
        complete,
        near_misses,
        variants_considered: candidates.len(),
    }
}

/// Rank complete combos: assembly rate first, feature count second.
pub fn rank_complete(rows: &mut [ComboAccess]) {
    rows.sort_by(|a, b| {
        b.pct_games
            .partial_cmp(&a.pct_games)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.produces.len().cmp(&a.produces.len()))
    });
}
