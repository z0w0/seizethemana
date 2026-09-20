// Exact hypergeometric cast-on-curve ceilings, `stm deck simulate
// --hypgeo`. Static probability math, no RNG: for each nonland spell, the
// chance that enough of the deck's cards are visible by the on-curve turn
// to cover the cost. Printed beside the simulated castability as a drift
// check: the Monte Carlo must not beat its own ceiling by much, and a big
// gap below the ceiling points at the mana base rather than the draw.

/// n choose k.
fn choose(n: u64, k: u64) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut result = 1.0f64;
    for i in 0..k {
        result *= (n - i) as f64 / (k - i) as f64;
    }
    result
}

/// P(X >= need) for X ~ Hypergeometric(N = deck, K = copies, n = seen),
/// with the window clamped to the deck size (a seen count past the deck
/// size cannot sample distinct cards).
/// Complement-summed so the tails stay accurate at small probabilities.
fn hyper_at_least(deck: u64, copies: u64, seen: u64, need: u64) -> f64 {
    if need == 0 {
        return 1.0;
    }
    let seen = seen.min(deck);
    let max_success = copies.min(seen);
    if need > max_success {
        return 0.0;
    }
    let total = choose(deck, seen);
    let mut missing = 0.0f64;
    for x in 0..need {
        // P(exactly x of the copies appear in the seen cards).
        let noncopies = deck - copies;
        if x > copies || seen - x > noncopies {
            continue;
        }
        missing += choose(copies, x) * choose(noncopies, seen - x) / total;
    }
    1.0 - missing
}

/// The number of cards seen by the end of turn `t` on the play (opener +
/// one draw per turn).
pub fn cards_seen_by(turn: u32) -> u64 {
    // Opener plus one draw per turn. A command-zone card sits outside the
    // library, but the ratios barely move and the ceiling stays
    // conservative either way.
    7u64 + turn as u64
}

/// P(6 or more lands among the first `seen` cards) for a deck of `deck`
/// cards holding `lands` lands. The flood bucket's exact expectation at
/// the game's actual draw volume (`seen`), so the simulated flood rate
/// can be checked against the math.
pub fn flood_expectation(lands: usize, deck: usize, seen: usize) -> f64 {
    let deck = deck.max(1) as u64;
    let lands = (lands.min(deck as usize)) as u64;
    let seen = (seen.max(1) as u64).min(deck);
    1.0 - {
        let mut below = 0.0f64;
        let total = choose(deck, seen);
        for x in 0..=5u64 {
            let nonlands = deck - lands;
            if x > lands || x > seen || seen - x > nonlands {
                continue;
            }
            below += choose(lands, x) * choose(nonlands, seen - x) / total;
        }
        below
    }
}

/// Per-card cast-on-curve ceilings over the whole deck (nonland, distinct
/// names). Returns the JSON payload for the report.
pub fn cast_ceilings(deck: &super::model::SimDeck, turns: u32) -> serde_json::Value {
    use std::collections::BTreeMap;
    let library = deck.cards.len() as u64;
    let lands_in_deck = deck
        .cards
        .iter()
        .filter(|c| c.role == super::model::Role::Land)
        .count() as u64;
    // Per (name, cost, copies) census of nonland cards.
    let mut by_name: BTreeMap<&str, (u32, u64)> = BTreeMap::new();
    for card in &deck.cards {
        if card.role == super::model::Role::Land {
            continue;
        }
        let entry = by_name
            .entry(card.name.as_str())
            .or_insert((card.cost.total(), 0));
        entry.1 += 1;
    }
    let rows: Vec<serde_json::Value> = by_name
        .into_iter()
        .map(|(name, (cmc, copies))| {
            let target = cmc.max(1).min(turns);
            let seen = cards_seen_by(target).min(library);
            // Ceiling = P(enough lands by target turn) x P(the card is
            // visible by target turn). Both exact hypergeometric. This is
            // an upper bound on the real cast-by-turn rate; the sim's
            // castability rows are draw-agnostic mana readiness and
            // exceed it by design.
            let lands_needed = target.min(lands_in_deck as u32) as u64;
            let land_chance = hyper_at_least(library, lands_in_deck, seen, lands_needed);
            let spell_chance = hyper_at_least(library, copies, seen, 1);
            let ceiling = land_chance * spell_chance;
            serde_json::json!({
                "name": name,
                "copies": copies as i64,
                "target_turn": target,
                "pct_castable_ceiling": (ceiling * 10_000.0).round() / 100.0,
            })
        })
        .collect();
    serde_json::json!({
        "note": "exact P(card visible AND lands on time) by cast-on-curve turn; an upper bound on the real cast rate — the sim's card_castability is draw-agnostic mana readiness and exceeds it",
        "cards": rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_basics() {
        assert_eq!(choose(5, 0), 1.0);
        assert_eq!(choose(5, 5), 1.0);
        assert_eq!(choose(5, 2), 10.0);
        assert_eq!(choose(52, 5), 2_598_960.0);
    }

    #[test]
    fn hyper_at_least_basics() {
        // Drawing at least 1 land from 24 lands in 60 cards, seeing 7:
        // the complement of C(36,7)/C(60,7) ≈ 97.8%.
        let p = hyper_at_least(60, 24, 7, 1);
        assert!((0.95..0.99).contains(&p), "{p}");
        // At least 4 lands by turn 4 (11 seen) from 24 sources ≈ 72.6%
        // (complement-summed: P(0..3 lands) subtracted from 1).
        let p4 = hyper_at_least(60, 24, 11, 4);
        assert!((0.70..0.76).contains(&p4), "{p4}");
        // Needing more copies than exist is impossible.
        assert_eq!(hyper_at_least(60, 1, 7, 2), 0.0);
        // Needing zero is certain.
        assert_eq!(hyper_at_least(60, 1, 7, 0), 1.0);
    }

    #[test]
    fn ceilings_carry_rows() {
        let deck = super::super::model::SimDeck {
            cards: vec![],
            commanders: vec![],
            format: super::super::model::Format::Commander,
            rules: crate::deck::simulator::format::rules_for("commander"),
        };
        let payload = cast_ceilings(&deck, 10);
        assert!(payload.get("cards").is_some());
    }
}

#[cfg(test)]
mod deck_size_tests {
    use super::*;

    #[test]
    fn flood_expectation_uses_the_real_deck_size() {
        // 24 lands in a 60-card deck: expectation is computed against 60,
        // not a hardcoded 99 (24/99 reads far too low).
        let sixty = flood_expectation(24, 60, 11);
        let ninety_nine = flood_expectation(24, 99, 11);
        assert!(
            (0.22..0.24).contains(&sixty),
            "24 lands in 60 sees 6+ of 11 ≈ 22.5%: {sixty}"
        );
        assert!(
            ninety_nine < 0.03,
            "24 lands in 99 (the wrong-denominator value) ≈ 2%: {ninety_nine}"
        );
        assert!(
            sixty > ninety_nine * 8.0,
            "24/60 floods far harder than 24/99: {sixty} vs {ninety_nine}"
        );
    }
}
