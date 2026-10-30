//! Tests for uniform-sample spell-and-land access.
use super::hypgeo::{choose, flood_expectation, hyper_at_least, modeled_access, spell_and_lands};

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
fn modeled_access_handles_an_empty_library() {
    let deck = crate::deck::simulator::model::SimDeck {
        companion: None,
        cards: vec![],
        commanders: vec![],
        format: crate::deck::simulator::model::Format::Commander,
        rules: crate::deck::simulator::format::rules_for("commander"),
    };
    let payload = modeled_access(&deck, 10);
    assert!(payload.cards.is_empty());
}

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

/// Exhaustive small samples provide an independent joint-probability check.
#[test]
fn joint_access_matches_every_small_uniform_sample() {
    for size in 1..=8u64 {
        for lands in 0..size {
            for copies in 1..=size - lands {
                for seen in 0..=size {
                    for need in 0..=lands + 1 {
                        let mut samples = 0u32;
                        let mut matches = 0u32;
                        for sample in 0..1u32 << size {
                            if u64::from(sample.count_ones()) != seen {
                                continue;
                            }
                            samples += 1;
                            let land_count = (sample & ((1 << lands) - 1)).count_ones();
                            let spell_mask = ((1 << copies) - 1) << lands;
                            if u64::from(land_count) >= need && sample & spell_mask != 0 {
                                matches += 1;
                            }
                        }
                        let expected = f64::from(matches) / f64::from(samples);
                        let actual = spell_and_lands(size, lands, copies, seen, need);
                        assert!((expected - actual).abs() < 1e-12);
                    }
                }
            }
        }
    }
}

/// Spell and land access are dependent because they share the same sample.
#[test]
fn joint_access_is_not_the_product_of_marginals() {
    assert_eq!(spell_and_lands(2, 1, 1, 1, 1), 0.0);
    assert_eq!(hyper_at_least(2, 1, 1, 1).powi(2), 0.25);
    assert_eq!(spell_and_lands(2, 1, 1, 2, 1), 1.0);
    assert_eq!(spell_and_lands(2, 1, 1, 20, 1), 1.0);
}

/// Out-of-horizon spells are absent; too few lands cannot satisfy a cost.
#[test]
fn baseline_keeps_the_real_cost_and_horizon() {
    use crate::deck::simulator::model::{Cost, Format, Role, SimCard, SimDeck};
    let deck = SimDeck {
        companion: None,
        cards: vec![
            SimCard {
                role: Role::Land,
                ..SimCard::default()
            },
            SimCard {
                name: "Two mana".into(),
                cost: Cost {
                    generic: 2,
                    ..Cost::default()
                },
                ..SimCard::default()
            },
            SimCard {
                name: "Free spell".into(),
                ..SimCard::default()
            },
        ],
        commanders: vec![],
        format: Format::Constructed,
        rules: crate::deck::simulator::format::rules_for("modern"),
    };
    let short = modeled_access(&deck, 1);
    assert_eq!(short.cards.len(), 1);
    assert_eq!(short.cards[0].name, "Free spell");
    assert_eq!(short.cards[0].percent_modeled_access.value(), 100.0);
    let longer = modeled_access(&deck, 2);
    let costly = longer
        .cards
        .iter()
        .find(|row| row.name == "Two mana")
        .expect("costly row");
    assert_eq!(costly.percent_modeled_access.value(), 0.0);
    assert!(longer.note.contains("not a cast-rate"));
    let json = serde_json::to_value(&longer).expect("serialize modeled access");
    let costly_json = json["cards"]
        .as_array()
        .expect("access rows")
        .iter()
        .find(|row| row["name"] == "Two mana")
        .expect("costly access row");
    assert_eq!(costly_json["percent_modeled_access"], 0.0);
    assert_eq!(costly_json.as_object().expect("typed access row").len(), 4);
    let parsed: super::hypgeo::HypgeoReport =
        serde_json::from_value(json).expect("read modeled access");
    assert_eq!(parsed, longer);
}

/// Small samples must not underflow while summing impossible lower tails.
#[test]
fn near_full_samples_and_empty_flood_windows_are_valid() {
    assert_eq!(hyper_at_least(5, 5, 5, 5), 1.0);
    assert_eq!(hyper_at_least(5, 4, 5, 4), 1.0);
    assert_eq!(flood_expectation(6, 6, 0), 0.0);
    assert_eq!(flood_expectation(0, 0, 0), 0.0);
}
