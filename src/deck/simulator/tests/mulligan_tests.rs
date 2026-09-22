use super::aggregate::aggregate;
use super::deal::{bottom_position, count_lands_in, london_mulligan, take_n};
use super::format::rules_for;
use super::game::run_game;
use super::model::{Format, Role, SimCard, SimDeck};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// A London-rules deck of lands + vanilla spells.
fn london_deck(lands: usize, spells: usize) -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..lands {
        cards.push(SimCard {
            name: "Plains".into(),
            role: Role::Land,
            tap: Some(super::parse_land::parse_tap_yield("Add {W}.").unwrap()),
            ..SimCard::default()
        });
    }
    for _ in 0..spells {
        cards.push(SimCard {
            name: "Bear".into(),
            role: Role::Other,
            ..SimCard::default()
        });
    }
    SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: rules_for("constructed"),
    }
}

/// The bottom choice: a flooded redrawn hand (4+ lands) sheds a land; a
/// starved hand sheds a spell. A redrawn hand nets exactly 6 cards.
#[test]
fn london_bottoms_toward_three_lands() {
    let deck = london_deck(30, 30);
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let mut exercised = 0;
    for _ in 0..600 {
        let mut library: Vec<usize> = (0..deck.cards.len()).collect();
        for i in (1..library.len()).rev() {
            let j = rng.random_range(0..=i);
            library.swap(i, j);
        }
        let hand = take_n(&mut library, 7);
        let pre = count_lands_in(&deck, &hand);
        if (2..=5).contains(&pre) {
            continue;
        }
        let hand = london_mulligan(&deck, &mut library, &mut rng);
        assert_eq!(hand.len(), 6, "redrawn hand nets 6 cards");
        let post = count_lands_in(&deck, &hand);
        if pre >= 4 {
            assert!(
                post < pre,
                "flooded redraw must bottom a land: {pre}->{post}"
            );
        }
        exercised += 1;
    }
    assert!(exercised > 50, "not enough redraws exercised ({exercised})");
}

/// The bottom position rule, tested directly on synthetic hands.
#[test]
fn bottom_position_flooded_sheds_land_starved_sheds_spell() {
    let deck = london_deck(40, 20); // indexes 0..40 lands, 40..60 spells
    let land_hand: Vec<usize> = (0..6).chain(vec![45]).collect(); // 6 lands + 1 spell
    let pos = bottom_position(&deck, &land_hand, 6);
    assert!(pos < 6, "flooded hand must bottom a land, bottomed {pos}");
    let spell_hand: Vec<usize> = (40..46).chain(vec![3]).collect(); // 6 spells + 1 land
    let pos = bottom_position(&deck, &spell_hand, 1);
    assert_eq!(
        pos, 0,
        "starved hand must bottom a spell (first spell at 40)"
    );
}

/// Kept hands stay at 7 cards: the London bottom applies only to
/// redrawn hands. Kept games play a full hand (their land drops and
/// mana come up on schedule; the mulligan rate stays bounded).
#[test]
fn london_keeps_untouched() {
    let deck = london_deck(20, 40);
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.mulligan_rate < 0.45,
        "20-land deck ships too often: {:.3}",
        stats.mulligan_rate
    );
    for log in &logs {
        if !log.mulliganed {
            assert!(
                (2..=5).contains(&log.opener_lands),
                "kept opener holds {} lands",
                log.opener_lands
            );
            // A kept 7-card hand with 2+ lands drops a land by t3 most
            // games; an accidental bottom would starve the early turns.
            assert!(log.lands_by_4 >= 1, "kept hand dropped no land by t4");
        }
    }
}
