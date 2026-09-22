use super::hypgeo::{cast_ceilings, choose, flood_expectation, hyper_at_least};

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
    let deck = crate::deck::simulator::model::SimDeck {
        cards: vec![],
        commanders: vec![],
        format: crate::deck::simulator::model::Format::Commander,
        rules: crate::deck::simulator::format::rules_for("commander"),
    };
    let payload = cast_ceilings(&deck, 10);
    assert!(payload.get("cards").is_some());
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
