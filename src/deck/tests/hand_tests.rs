use crate::deck::hand::{HandCard, HandRow, advice_for, early_play_count};
use crate::deck::simulator::deal::deal_opener;
use crate::deck::simulator::format::rules_for;
use crate::deck::simulator::model::{Format, Role, SimCard, SimDeck};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A commander deck: 40 basics + spells, one commander.
fn commander_deck(cheap: usize, expensive: usize) -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..40 {
        cards.push(SimCard {
            name: "Island".into(),
            role: Role::Land,
            ..SimCard::default()
        });
    }
    for n in 0..cheap {
        cards.push(SimCard {
            name: format!("Cheap {n}"),
            role: Role::Other,
            ..SimCard::default()
        });
    }
    for n in 0..expensive {
        cards.push(SimCard {
            name: format!("Expensive {n}"),
            role: Role::Wincon,
            cost: crate::deck::simulator::model::Cost {
                generic: 6,
                ..Default::default()
            },
            ..SimCard::default()
        });
    }
    SimDeck {
        cards,
        commanders: vec![SimCard {
            name: "Commander".into(),
            role: Role::Wincon,
            ..SimCard::default()
        }],
        format: Format::Commander,
        rules: rules_for("commander"),
    }
}

/// Seed reproducibility: the same seed deals identical hands in order.
#[test]
fn same_seed_deals_same_hands() {
    let deck = commander_deck(20, 20);
    let mut rng_a = ChaCha8Rng::seed_from_u64(42);
    let mut rng_b = ChaCha8Rng::seed_from_u64(42);
    for _ in 0..5 {
        let a = deal_opener(&deck, &mut rng_a);
        let b = deal_opener(&deck, &mut rng_b);
        assert_eq!(a.hand, b.hand);
        assert_eq!(a.lands, b.lands);
        assert_eq!(a.mulliganed, b.mulliganed);
    }
}

/// Hands hold at most 7 cards, and the land counts match the mulligan
/// policy's keep band for kept hands.
#[test]
fn hands_respect_the_keep_band() {
    let deck = commander_deck(20, 20);
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    for _ in 0..50 {
        let opener = deal_opener(&deck, &mut rng);
        assert!(opener.hand.len() <= 7, "hand over 7 cards");
        if !opener.mulliganed {
            assert!(
                (2..=6).contains(&opener.lands),
                "kept opener holds {} lands",
                opener.lands
            );
        }
    }
}

/// Advice strings follow the policy: red-flag hands say mulligan, good
/// hands say keep, redrawn hands tell the reader to look again. The
/// commander keep band is 2-6, so a 6-land hand is a keep.
#[test]
fn advice_matches_policy() {
    let band = (2u8, 6u8);
    assert!(advice_for(0, false, 2, band).contains("mulligan"));
    assert!(advice_for(7, false, 2, band).contains("mulligan"));
    // 6 lands is inside the commander keep band — never "mulligan".
    assert!(advice_for(6, false, 2, band).contains("Keep"));
    assert!(advice_for(4, false, 3, band).contains("Keep"));
    assert!(advice_for(4, false, 0, band).contains("Borderline"));
    assert!(advice_for(3, true, 2, band).contains("Redrawn"));
    // London formats keep only 2-5 lands: 6 lands advises a mulligan.
    let london = (2u8, 5u8);
    assert!(advice_for(6, false, 2, london).contains("mulligan"));
    assert!(advice_for(5, false, 2, london).contains("Keep"));
}

/// Early-play counting only counts cheap nonland spells.
#[test]
fn early_plays_count_cheap_spells_only() {
    let deck = commander_deck(20, 20);
    let cheap: Vec<usize> = (40..45).collect(); // Cheap 0..5
    assert_eq!(early_play_count(&deck, &cheap), 5);
    let expensive: Vec<usize> = (60..63).collect(); // Expensive 0..3
    assert_eq!(early_play_count(&deck, &expensive), 0);
}

/// HandRow serializes with the documented keys.
#[test]
fn hand_row_shape() {
    let row = HandRow {
        cards: vec![HandCard {
            name: "Sol Ring".into(),
            mana_cost: "{1}".into(),
            cmc: 1.0,
            type_line: "Artifact".into(),
        }],
        lands: 2,
        mulliganed: false,
        advice: "Keep this.".into(),
    };
    let json = serde_json::to_value(&row).expect("serialize");
    assert!(json.get("cards").is_some());
    assert!(json.get("lands").is_some());
    assert!(json.get("mulliganed").is_some());
    assert!(json.get("advice").is_some());
}
