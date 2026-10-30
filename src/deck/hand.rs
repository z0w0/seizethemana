//! `stm deck hand`: sample opening hands for a deck, dealt with the same
//! shuffle and mulligan rules the goldfish simulator uses. Seed N deals
//! the same opener as the sim's game #1, so "does this deck keep hands?"
//! has a reproducible answer.

use super::simulator::hand_payment::known_hand_payments;
use crate::db::CardRow;
use std::collections::HashMap;

/// One card in a dealt hand.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HandCard {
    /// Card name.
    pub name: String,
    /// Mana cost as printed.
    pub mana_cost: String,
    /// Mana value.
    pub mana_value: f64,
    /// Type line (Creature, Instant, ...).
    pub type_line: String,
}

/// One dealt hand, ready to render.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HandRow {
    /// Cards in the hand.
    pub cards: Vec<HandCard>,
    /// Land count (MDFC spell faces count land-able).
    pub lands: u8,
    /// True when the opener was redrawn under the format's policy.
    pub mulliganed: bool,
    /// Number of redraws under the pilot's mulligan policy.
    pub redraws: u8,
    /// Names of cards put on the bottom after keeping.
    pub bottomed_cards: Vec<String>,
    /// One-line plain-English read of the hand.
    pub advice: String,
    /// Known-hand payment check, separate from the mulligan policy.
    pub assessment: HandAssessment,
}

/// Bounded payment evidence for an opening hand, not a full play sequence.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HandAssessment {
    /// Last turn checked without assuming future draws.
    pub evaluation_turn: u8,
    /// Spells each payable on their own, not all payable together.
    pub individually_payable_spells: usize,
    /// Number of cards after the mulligan policy and bottoming.
    pub kept_size: usize,
    /// Stable reasons behind the assessment.
    pub reasons: Vec<HandReason>,
}

/// Structured evidence for the hand's land count and payment check.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandReason {
    /// Land count is below the pilot's keep band.
    LowLandCount,
    /// Land count is above the pilot's keep band.
    HighLandCount,
    /// No early spell passed the bounded payment check.
    NoEarlyPaymentFound,
    /// At least one spell passed the bounded payment check.
    EarlyPaymentFound,
    /// The check uses only known lands, not ramp or future cards.
    KnownLandsOnly,
    /// Targets and non-mana spell costs still need a separate check.
    ManaCostsOnly,
    /// Missing data or unsupported source requirements were not counted as payment.
    UnsupportedPaymentRequirements,
}

/// JSON response for a seeded hand sample.
#[derive(Debug, serde::Serialize)]
struct HandSampleReport<'a> {
    /// Seed used to deal the sample.
    seed: u64,
    /// Dealt hands.
    hands: &'a [HandRow],
}

/// Assess the kept hand independently of the rule that caused a redraw.
fn advice_for(lands: u8, mulliganed: bool, early_plays: usize, keep_band: (u8, u8)) -> String {
    let redraw_note = if mulliganed { "Redrawn hand. " } else { "" };
    let (lo, hi) = keep_band;
    if lands < lo {
        return format!(
            "{redraw_note}{lands} lands - mulligan candidate. Below the pilot's land band."
        );
    }
    if lands > hi {
        return format!(
            "{redraw_note}{lands} lands - mulligan candidate. Above the pilot's land band."
        );
    }
    if early_plays == 0 {
        return format!(
            "{redraw_note}{lands} lands. Borderline: no early payment found from known lands by turn 2. Check colors, entry timing, and other requirements."
        );
    }
    format!(
        "{redraw_note}{lands} lands, {early_plays} spells individually payable from known lands by turn 2. Keep candidate; check other spell requirements."
    )
}

/// The keep band the hand advice mirrors: the sim's redraw threshold per
/// mulligan policy (free-redraw formats keep inside their configured
/// band; London mulligans redraw outside 2-5).
fn keep_band_for(deck: &crate::deck::simulator::model::SimDeck) -> (u8, u8) {
    match deck.rules.mulligan {
        crate::deck::simulator::format::MulliganPolicy::FreeRedraw { land_band } => land_band,
        crate::deck::simulator::format::MulliganPolicy::London => (2, 5),
    }
}

/// Keep payment evidence separate from land-band and mulligan policy results.
fn assess_hand(
    lands: u8,
    early_plays: usize,
    kept_size: usize,
    keep_band: (u8, u8),
    skipped_requirements: bool,
) -> HandAssessment {
    let mut reasons = vec![HandReason::KnownLandsOnly, HandReason::ManaCostsOnly];
    if skipped_requirements {
        reasons.push(HandReason::UnsupportedPaymentRequirements);
    }
    if lands < keep_band.0 {
        reasons.push(HandReason::LowLandCount);
    }
    if lands > keep_band.1 {
        reasons.push(HandReason::HighLandCount);
    }
    reasons.push(if early_plays == 0 {
        HandReason::NoEarlyPaymentFound
    } else {
        HandReason::EarlyPaymentFound
    });
    HandAssessment {
        evaluation_turn: 2,
        individually_payable_spells: early_plays,
        kept_size,
        reasons,
    }
}

/// Card-row display fields for one hand, falling back to the sim card
/// when the oracle has no row (unknown name).
fn hand_cards(
    deck: &super::simulator::model::SimDeck,
    hand: &[super::simulator::model::CardIdx],
    cards: &HashMap<String, CardRow>,
) -> Vec<HandCard> {
    hand.iter()
        .map(|i| {
            let sim = &deck[*i];
            match cards.get(&sim.name) {
                Some(row) => HandCard {
                    name: row.name.clone(),
                    mana_cost: row.mana_cost.clone(),
                    mana_value: row.cmc,
                    type_line: row.type_line.clone(),
                },
                None => HandCard {
                    name: sim.name.clone(),
                    mana_cost: String::new(),
                    mana_value: f64::from(sim.cost.total()),
                    type_line: String::new(),
                },
            }
        })
        .collect()
}

/// Entry point for `stm deck hand <name>`.
pub fn hand(
    paths: &crate::paths::Paths,
    conn: &rusqlite::Connection,
    out: &mut crate::output::Output,
    name: &str,
    seed: Option<u64>,
    count: u32,
    json: bool,
) -> anyhow::Result<i32> {
    let (_path, deck) = super::store::load_deck(paths, name)?;
    let cards = super::stats::lookup_names(conn, &deck)?;
    let sim_deck = super::simulator::deck::build_sim_deck(&deck, &cards, None);
    let total = sim_deck.cards.len() + sim_deck.commanders.len();
    if total == 0 {
        out.error(&format!("deck {name:?} has no cards to sample"));
        out.hint(&format!(
            "add cards with: stm deck update {name:?} --add '1 <card>'"
        ));
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let count = count.clamp(1, 10);
    let seed = seed.unwrap_or_else(rand::random::<u64>);

    let mut rows: Vec<HandRow> = Vec::with_capacity(count as usize);
    for game_id in 0..count {
        let mut rng = super::simulator::rng::game_rng(
            seed,
            u64::from(game_id),
            super::simulator::rng::RngDomain::Opening,
        );
        let opener = super::simulator::deal::deal_opener(&sim_deck, &mut rng);
        let payments = known_hand_payments(&sim_deck, &opener.hand, &cards);
        let early = payments.payable_spells;
        let advice = advice_for(
            opener.lands,
            opener.mulliganed,
            early,
            keep_band_for(&sim_deck),
        );
        rows.push(HandRow {
            cards: hand_cards(&sim_deck, &opener.hand, &cards),
            lands: opener.lands,
            mulliganed: opener.mulliganed,
            redraws: opener.redraws,
            bottomed_cards: opener
                .bottomed
                .iter()
                .map(|index| sim_deck[*index].name.clone())
                .collect(),
            advice,
            assessment: assess_hand(
                opener.lands,
                early,
                opener.hand.len(),
                keep_band_for(&sim_deck),
                payments.skipped_requirements,
            ),
        });
    }

    if json {
        let wrapped = HandSampleReport { seed, hands: &rows };
        println!("{}", serde_json::to_string_pretty(&wrapped)?);
        return Ok(crate::cli::codes::OK);
    }

    let styles = out.styles();
    let seed_note = styles.dim(&format!("(seed {seed})"));
    for (n, row) in rows.iter().enumerate() {
        out.print_wrapped(&format!(
            "{} {}  {}",
            styles.header(&format!("Hand {}", n + 1)),
            seed_note,
            row.advice
        ));
        for card in &row.cards {
            let cost = if card.mana_cost.is_empty() {
                String::new()
            } else {
                format!("  {}", styles.mana_pips(&card.mana_cost))
            };
            let type_note = if card.type_line.is_empty() {
                String::new()
            } else {
                format!("  {}", styles.dim(&card.type_line))
            };
            out.print_wrapped(&format!(
                "  {}{cost}{type_note}",
                styles.card_name(&card.name)
            ));
        }
        println!();
    }
    Ok(crate::cli::codes::OK)
}

#[cfg(test)]
#[path = "tests/hand_tests.rs"]
mod hand_tests;
