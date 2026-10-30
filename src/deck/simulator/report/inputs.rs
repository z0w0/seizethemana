//! Versioned input descriptions for reproducible simulator reports.

use sha2::{Digest, Sha256};

use super::*;

/// Hash length-prefixed strings without ambiguous joins.
fn fingerprint(values: impl IntoIterator<Item = String>) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

/// Record deck contents separately from the order consumed by the simulator.
pub(super) fn provenance(deck: &SimDeck) -> ReportProvenance {
    let ordered: Vec<String> = deck
        .library_cards()
        .map(|card| format!("library:{}", card.name))
        .chain(
            deck.commanders
                .iter()
                .map(|card| format!("commander:{}", card.name)),
        )
        .chain(
            deck.companion
                .iter()
                .map(|index| format!("companion:{}", deck[*index].name)),
        )
        .collect();
    let execution_order_fingerprint = fingerprint(ordered.iter().cloned());
    let mut canonical = ordered;
    canonical.sort();
    canonical.push(format!("format:{}", deck.rules.key));
    let card_data_fingerprint = if deck
        .cards
        .iter()
        .chain(&deck.commanders)
        .any(|card| card.source_hash.is_empty())
    {
        None
    } else {
        let sources: std::collections::BTreeSet<String> = deck
            .cards
            .iter()
            .chain(&deck.commanders)
            .map(|card| format!("{}:{}", card.name, card.source_hash))
            .collect();
        Some(fingerprint(sources))
    };
    ReportProvenance {
        schema_version: 3,
        model_version: 2,
        policy_version: 2,
        rng_version: format!("chacha8-sha256-domains-v{}", super::super::rng::RNG_VERSION),
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        deck_fingerprint: fingerprint(canonical),
        execution_order_fingerprint,
        card_data_fingerprint,
        fingerprint_note: "SHA-256 over length-prefixed values. Deck contents cover the active library, commanders, and companion, including quantities and format. Other bench cards and source deck-file formatting are excluded. Card data covers sorted distinct card-name and exact source-hash pairs. Execution order is hashed separately.".to_string(),
    }
}

/// Describe settings supplied by the run coordinator, not inferred defaults.
pub(super) fn opponent_scenario(
    format: super::super::model::Format,
    parameters: super::super::opponents::OpponentConfig,
    stats: &SimStats,
) -> OpponentScenario {
    OpponentScenario {
        model_version: super::super::opponents::MODEL_VERSION,
        identity: if parameters.active { "synthetic_table" } else { "inactive" }.to_string(),
        seats: format.life_loss_mult(),
        parameters,
        observed_activity: opponent_activity(stats),
        note: "Illustrative schedule, not measured real-world table activity. Combat is unblocked and opponents do not cast removal. Observed activity comes from the actual shared table ledger.".to_string(),
    }
}

/// Keep exact table totals separate from model settings and sampling intervals.
fn opponent_activity(stats: &SimStats) -> OpponentActivityReport {
    OpponentActivityReport {
        attempted_games: stats.runs,
        raw_totals: stats.raw_opponent_activity.clone(),
        observed_games: stats.finished_games,
        totals: OpponentActivityTotals {
            draws: stats.opponent_draws, casts: stats.opponent_casts,
            payments: stats.opponent_payments, skipped_casts: stats.opponent_skipped_casts,
        },
        per_game: (stats.finished_games > 0).then(|| {
            let n = f64::from(stats.finished_games);
            OpponentActivityAverages {
                draws: round2(stats.opponent_draws as f64 / n), casts: round2(stats.opponent_casts as f64 / n),
                payments: round2(stats.opponent_payments as f64 / n), skipped_casts: round2(stats.opponent_skipped_casts as f64 / n),
            }
        }),
        note: "Raw totals include all requested games. Totals and per-game means use finished games, including player losses. Payments count successful optional tax decisions, not mana spent. Excluding incomplete games can bias these means.".to_string(),
    }
}

/// Expose compiler records without upgrading them to verified runtime coverage.
pub(super) fn card_support(deck: &SimDeck) -> Vec<CardSupportReport> {
    let mut by_name =
        std::collections::BTreeMap::<&str, (&super::super::model::SimCard, u32)>::new();
    for card in deck.cards.iter().chain(&deck.commanders) {
        by_name
            .entry(&card.name)
            .and_modify(|(_, copies)| *copies += 1)
            .or_insert((card, 1));
    }
    by_name.into_iter().map(|(name, (card, copies))| CardSupportReport {
        name: name.to_string(), copies,
        source_hash: (!card.source_hash.is_empty()).then(|| card.source_hash.clone()),
        abilities: card.support.iter().map(|record| {
            use super::super::support::AbilitySupportStatus;
            let (status, rule_id, reason) = match &record.status {
                AbilitySupportStatus::Supported => ("supported", None, None),
                AbilitySupportStatus::CompilerOnly { reason } => ("compiler_only", None, Some(reason.clone())),
                AbilitySupportStatus::Approximated { rule_id } => ("approximated", Some(rule_id.to_string()), None),
                AbilitySupportStatus::Unsupported { reason } => ("unsupported", None, Some(reason.clone())),
            };
            AbilitySupportReport {
                identity: format!("ability:{}", record.ability_index), source: record.source.clone(),
                span_start: record.span.map(|span| span.start), span_end: record.span.map(|span| span.end),
                status: status.to_string(), rule_id, reason,
            }
        }).collect(),
        note: if card.source_hash.is_empty() {
            "Card-source data is missing. This is not evidence of a playable zero-cost card or exact rules support. The report has no complete card-data fingerprint."
        } else {
            "Compiler evidence only. Separate card-level static fields and runtime interactions are not fully verified by these records. No records means no ability-level evidence, not full support."
        }.to_string(),
    }).collect()
}
