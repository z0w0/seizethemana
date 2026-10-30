//! Uniform-draw baselines for seeing a spell and enough lands on curve.
//! These exact sampling probabilities are not limits on simulated casts or
//! mana readiness. Mulligans, ramp, selection, tutors, and extra draws can
//! change those outcomes.

/// n choose k.
pub(super) fn choose(n: u64, k: u64) -> f64 {
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
pub(super) fn hyper_at_least(deck: u64, copies: u64, seen: u64, need: u64) -> f64 {
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
    for x in seen.saturating_sub(deck - copies)..need {
        // P(exactly x of the copies appear in the seen cards).
        let noncopies = deck - copies;
        if x > copies || seen - x > noncopies {
            continue;
        }
        missing += choose(copies, x) * choose(noncopies, seen - x) / total;
    }
    (1.0 - missing).clamp(0.0, 1.0)
}

/// Sample size for a seven-card opener followed by one draw per turn.
fn cards_seen_by(turn: u32) -> u64 {
    7 + u64::from(turn)
}

/// Joint access to disjoint spell copies and lands in one uniform sample.
pub(super) fn spell_and_lands(
    deck: u64,
    lands: u64,
    copies: u64,
    seen: u64,
    lands_needed: u64,
) -> f64 {
    let seen = seen.min(deck);
    if copies == 0 || seen == 0 || lands_needed > lands.min(seen) {
        return 0.0;
    }
    let others = deck - lands - copies;
    let total = choose(deck, seen);
    let mut probability = 0.0;
    for land_count in lands_needed..=lands.min(seen) {
        let remaining = seen - land_count;
        for spell_count in 1.max(remaining.saturating_sub(others))..=copies.min(remaining) {
            probability += choose(lands, land_count)
                * choose(copies, spell_count)
                * choose(others, remaining - spell_count)
                / total;
        }
    }
    probability.clamp(0.0, 1.0)
}

/// P(6 or more lands among the first `seen` cards) for a deck of `deck`
/// cards holding `lands` lands under uniform sampling without replacement.
/// Draw counts that depend on earlier draws or selection do not follow this model.
pub fn flood_expectation(lands: usize, deck: usize, seen: usize) -> f64 {
    hyper_at_least(deck as u64, lands.min(deck) as u64, seen as u64, 6)
}

/// Uniform-draw access baseline for one distinct card name.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModeledAccess {
    /// Card name.
    pub name: String,
    /// Number of copies in the library.
    pub copies: u64,
    /// Target turn from the modeled spell cost.
    pub target_turn: u32,
    /// Joint spell-and-land access under the declared uniform-sample model.
    pub percent_modeled_access: super::report_schema::Percent,
}

/// Uniform-draw access baselines and their sampling assumptions.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HypgeoReport {
    /// Exact-probability interpretation.
    pub note: String,
    /// Per-card spell-and-land access baselines.
    pub cards: Vec<ModeledAccess>,
}

/// Spell-and-land access by the on-curve turn, within the requested horizon.
pub fn modeled_access(deck: &super::model::SimDeck, turns: u32) -> HypgeoReport {
    use std::collections::BTreeMap;
    let library = deck.library_len() as u64;
    let lands_in_deck = deck
        .library_cards()
        .filter(|c| c.role == super::model::Role::Land)
        .count() as u64;
    // Per (name, cost, copies) census of nonland cards.
    let mut by_name: BTreeMap<&str, (u32, u64)> = BTreeMap::new();
    for card in deck.library_cards() {
        if card.role == super::model::Role::Land {
            continue;
        }
        let entry = by_name
            .entry(card.name.as_str())
            .or_insert((card.cost.total(), 0));
        entry.1 += 1;
    }
    let rows: Vec<ModeledAccess> = by_name
        .into_iter()
        .filter_map(|(name, (cmc, copies))| {
            let target = cmc.max(1);
            if target > turns {
                return None;
            }
            let seen = cards_seen_by(target).min(library);
            let access = spell_and_lands(library, lands_in_deck, copies, seen, u64::from(cmc));
            Some(ModeledAccess {
                name: name.to_string(),
                copies,
                target_turn: target,
                percent_modeled_access: super::report_schema::Percent::from_share(access),
            })
        })
        .collect();
    HypgeoReport {
        note: "Exact joint probability of seeing the spell and at least its modeled cost in lands in a uniform sample of seven cards plus one draw per turn. The modeled cost uses unresolved X = 0 and can use a cheaper split-card face; it is not always printed mana value. This is an access baseline, not a cast-rate or mana-readiness ceiling. It excludes mulligans, extra draws, selection, tutors, ramp, colors, land timing, and MDFC land faces. Cards whose target turn is outside the simulated horizon are omitted.".to_string(),
        cards: rows,
    }
}
