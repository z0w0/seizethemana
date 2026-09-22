// `stm deck hand`: sample opening hands for a deck, dealt with the same
// shuffle and mulligan rules the goldfish simulator uses. Seed N deals
// the same opener as the sim's game #1, so "does this deck keep hands?"
// has a reproducible answer.

use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

/// One card in a dealt hand.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HandCard {
    /// Card name.
    pub name: String,
    /// Mana cost as printed.
    pub mana_cost: String,
    /// Mana value.
    pub cmc: f64,
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
    /// One-line plain-English read of the hand.
    pub advice: String,
}

/// Keep/mulligan advice for one opener: land count against the format's
/// keep band, plus whether the hand has early plays. Mirrors the sim's
/// mulligan thresholds so the advice never contradicts the sim. Kept
/// hands hold 7 cards under the free-redraw policy, 6 under London.
fn advice_for(lands: u8, mulliganed: bool, early_plays: usize, keep_band: (u8, u8)) -> String {
    if mulliganed {
        return "Redrawn under the mulligan rule. Check the new hand before keeping.".to_string();
    }
    let (lo, hi) = keep_band;
    if lands < lo {
        return format!("{lands} lands - mulligan. This hand does nothing early.");
    }
    if lands > hi {
        return format!("{lands} lands - mulligan. Too much land, too few spells.");
    }
    if early_plays == 0 {
        return format!(
            "{lands} lands, but nothing castable early. Borderline - mulligan if the rest of the hand is slow."
        );
    }
    format!("{lands} lands, {early_plays} early plays. Keep this.")
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

/// Count spells in the hand castable by turn 2 (cost 2 or less, nonland).
fn early_play_count(deck: &crate::deck::simulator::model::SimDeck, hand: &[usize]) -> usize {
    hand.iter()
        .filter(|i| {
            let c = &deck.cards[**i];
            !c.opens_in_play && c.cost.total() <= 2 && c.role != super::simulator::model::Role::Land
        })
        .count()
}

/// Card-row display fields for one hand, falling back to the sim card
/// when the oracle has no row (unknown name).
fn hand_cards(
    deck: &super::simulator::model::SimDeck,
    hand: &[usize],
    cards: &HashMap<String, CardRow>,
) -> Vec<HandCard> {
    hand.iter()
        .map(|i| {
            let sim = &deck.cards[*i];
            match cards.get(&sim.name) {
                Some(row) => HandCard {
                    name: row.name.clone(),
                    mana_cost: row.mana_cost.clone(),
                    cmc: row.cmc,
                    type_line: row.type_line.clone(),
                },
                None => HandCard {
                    name: sim.name.clone(),
                    mana_cost: String::new(),
                    cmc: f64::from(sim.cost.total()),
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
        out.error("deck has no cards");
        out.hint("add cards with: stm deck update <name> --add <spec>");
        return Ok(crate::cli::codes::NO_RESULTS);
    }
    let count = count.clamp(1, 10);
    let seed = seed.unwrap_or_else(rand::random::<u64>);

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut rows: Vec<HandRow> = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let opener = super::simulator::deal::deal_opener(&sim_deck, &mut rng);
        let early = early_play_count(&sim_deck, &opener.hand);
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
            advice,
        });
    }

    if json {
        let wrapped = serde_json::json!({ "seed": seed, "hands": rows });
        println!("{}", serde_json::to_string_pretty(&wrapped)?);
        return Ok(crate::cli::codes::OK);
    }

    let styles = out.styles();
    let seed_note = styles.dim(&format!("(seed {seed})"));
    for (n, row) in rows.iter().enumerate() {
        println!(
            "{} {}  {}",
            styles.header(&format!("Hand {}", n + 1)),
            seed_note,
            row.advice
        );
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
            println!("  {}{cost}{type_note}", styles.card_name(&card.name),);
        }
        println!();
    }
    Ok(crate::cli::codes::OK)
}

#[cfg(test)]
#[path = "tests/hand_tests.rs"]
mod hand_tests;
