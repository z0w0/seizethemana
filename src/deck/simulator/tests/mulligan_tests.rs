//! Tests for opening hands and the mulligan policy.
use super::aggregate::aggregate;
use super::deal::{bottom_position, count_lands_in, deal_opener, reshuffle_hand, take_n};
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
            tap: Some(super::oracle_parser::land::parse_tap_yield("Add {W}.").unwrap()),
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
        companion: None,
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
        let mut library: Vec<crate::deck::simulator::model::CardIdx> = (0..deck.cards.len() as u32)
            .map(crate::deck::simulator::model::CardIdx)
            .collect();
        for i in (1..library.len()).rev() {
            let j = rng.random_range(0..=i);
            library.swap(i, j);
        }
        let _shipped_hand = take_n(&mut library, 7);
        let redrawn_hand = take_n(&mut library.clone(), 7);
        let redrawn_lands = count_lands_in(&deck, &redrawn_hand);
        if (2..=5).contains(&redrawn_lands) {
            continue;
        }
        let mut hand = redrawn_hand.clone();
        let bottomed_pos = bottom_position(&deck, &hand, redrawn_lands);
        let expected = hand.remove(bottomed_pos);
        library.insert(0, expected);
        assert_eq!(hand.len(), 6, "redrawn hand nets 6 cards");
        let post = count_lands_in(&deck, &hand);
        assert!(
            library.contains(&expected),
            "bottomed card returns to library"
        );
        if redrawn_lands >= 4 {
            assert!(
                post < redrawn_lands,
                "flooded redraw must bottom a land: {redrawn_lands}->{post}"
            );
        } else {
            assert_eq!(post, redrawn_lands, "starved redraw bottoms a spell");
        }
        exercised += 1;
    }
    assert!(exercised > 50, "not enough redraws exercised ({exercised})");
}

/// The bottom position rule, tested directly on synthetic hands.
#[test]
fn bottom_position_flooded_sheds_land_starved_sheds_spell() {
    let deck = london_deck(40, 20); // indexes 0..40 lands, 40..60 spells
    let to_idx = |v: Vec<usize>| {
        v.into_iter()
            .map(|i| crate::deck::simulator::model::CardIdx(i as u32))
            .collect::<Vec<_>>()
    };
    let land_hand = to_idx((0..6).chain(vec![45]).collect()); // 6 lands + 1 spell
    let pos = bottom_position(&deck, &land_hand, 6);
    assert!(pos < 6, "flooded hand must bottom a land, bottomed {pos}");
    let spell_hand = to_idx((40..46).chain(vec![3]).collect()); // 6 spells + 1 land
    let pos = bottom_position(&deck, &spell_hand, 1);
    assert_eq!(
        pos, 0,
        "starved hand must bottom a spell (first spell at 40)"
    );
}

#[test]
fn rejected_hand_returns_to_the_reshuffled_library() {
    let mut overlap_runs = 0;
    for seed in 0..100 {
        let mut library: Vec<_> = (0..53)
            .map(crate::deck::simulator::model::CardIdx)
            .collect();
        let mut rejected: Vec<_> = (53..60)
            .map(crate::deck::simulator::model::CardIdx)
            .collect();
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        reshuffle_hand(&mut library, &mut rejected, &mut rng);
        assert!(rejected.is_empty(), "the rejected hand is returned");
        let next_hand = take_n(&mut library, 7);
        if next_hand
            .iter()
            .any(|card| (53..60).contains(&card.index()))
        {
            overlap_runs += 1;
        }
    }
    assert!(
        overlap_runs > 40,
        "a reshuffled deck should redraw cards from the rejected hand; got {overlap_runs}/100"
    );
}

#[test]
fn london_redraws_seven_cards_until_keep_then_bottoms() {
    let deck = london_deck(0, 60);
    let mut rng = ChaCha8Rng::seed_from_u64(17);
    let opener = deal_opener(&deck, &mut rng);

    assert_eq!(opener.hand.len(), 1, "six mulligans leave one card");
    assert_eq!(opener.library.len(), 59);
    assert_eq!(opener.lands, 0);
    assert!(opener.mulliganed);
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
