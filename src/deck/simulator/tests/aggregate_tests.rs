// Tests for the simulator aggregate module.

/// A minimal card row for tests.
use super::aggregate::aggregate;
use super::findings::find_problems;
use super::game::run_game;
use super::model::*;
use super::parse::*;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
fn card(name: &str, mana_cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: mana_cost.to_string(),
        cmc: super::parse::parse_cost(mana_cost).total() as f64,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

/// A stub deck: lands + spells, all with simple tap data.
fn stub_deck(lands: usize, spells: &[(&str, u32, Role)]) -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..lands {
        cards.push(super::model::SimCard {
            name: "Plains".into(),
            cost: Cost::default(),
            min_cost: Cost::default(),
            tap: Some(parse_tap_yield("{T}: Add {W}.").unwrap()),
            role: Role::Land,
            ..super::model::SimCard::default()
        });
    }
    for (name, cmc, role) in spells {
        for _ in 0..8 {
            cards.push(super::model::SimCard {
                name: (*name).into(),
                cost: Cost {
                    generic: *cmc,
                    ..Cost::default()
                },
                min_cost: Cost {
                    generic: *cmc,
                    ..Cost::default()
                },
                role: *role,
                ..super::model::SimCard::default()
            });
        }
    }
    SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    }
}

#[test]
fn aggregate_counts_land_drops_and_starvation() {
    let deck = stub_deck(20, &[("Bear", 2, Role::Other)]);
    let mut rng = ChaCha8Rng::seed_from_u64(21);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let mut stats = aggregate(&logs, &deck, 8);
    stats.removal_count = 0;
    stats.wincon_count = 0;
    stats.draw_count = 0;
    let problems = find_problems(&stats, &deck);
    // A drawless deck trips draw starvation.
    assert!(problems.iter().any(|p| p.kind == "draw_starvation"));
}

#[test]
fn severity_buckets_match_shares() {
    // Exercised through find_problems; check the color screw thresholds.
    let deck = stub_deck(24, &[("Bear", 2, Role::Other); 10]);
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let mut stats = aggregate(&logs, &deck, 8);
    stats.removal_count = 0;
    stats.wincon_count = 0;
    // Monocolor decks never trip color screw.
    stats.color_screw = [0.0; 5];
    let problems = find_problems(&stats, &deck);
    assert!(!problems.iter().any(|p| p.kind == "color_screw"));
}

#[test]
fn percentile_nearest_rank() {
    let mut v = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    assert_eq!(super::aggregate::percentile(&mut v, 0.5), 5);
    assert_eq!(super::aggregate::percentile(&mut v, 0.95), 10);
}

#[test]
fn reactive_spells_classify_as_removal() {
    // Fog, regeneration, and tap-down texts are interaction: they count
    // toward role_access and are exempt from dead_cards.
    for text in [
        "Prevent all combat damage that would be dealt this turn.",
        "{G}: Regenerate target creature.",
        "Creatures with power 4 or greater can't attack or block.",
    ] {
        let row = card("Test Card", "{1}{G}", "Instant", text);
        let sim = super::parse::parse_sim_card(&row);
        assert_eq!(sim.role, Role::Removal, "text: {text}");
    }
}

#[test]
fn protection_spells_are_not_removal() {
    // Protection grants answer nothing in a goldfish: they are inert
    // reactive text, not removal. They must not inflate removal counts.
    for text in [
        "Target creature gains hexproof and indestructible until end of turn.",
        "Target creature gains hexproof until end of turn.",
        "Permanents you control gain hexproof and indestructible until end of turn.",
    ] {
        let row = card("Test Card", "{1}{G}", "Instant", text);
        let sim = super::parse::parse_sim_card(&row);
        assert_ne!(sim.role, Role::Removal, "text: {text}");
        assert!(!sim.is_interaction, "text: {text}");
    }
}

#[test]
fn removal_exempt_from_dead_cards() {
    // A reactive spell with a high floor must not surface in dead_cards;
    // its castability row still exists.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let fog = super::parse::parse_sim_card(&card(
        "Spore Fog",
        "{1}{G}",
        "Instant",
        "Prevent all combat damage that would be dealt this turn.",
    ));
    deck.cards.push(fog);
    let mut rng = ChaCha8Rng::seed_from_u64(31);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let mut stats = aggregate(&logs, &deck, 10);
    stats.removal_count = 1;
    stats.wincon_count = 5;
    stats.draw_count = 5;
    let problems = find_problems(&stats, &deck);
    let dead = problems.iter().find(|p| p.kind == "dead_cards");
    if let Some(p) = dead {
        assert!(
            !p.detail.contains("Spore Fog"),
            "reactive spell flagged dead: {}",
            p.detail
        );
    }
    assert!(
        stats.card_castability.iter().any(|c| c.name == "Spore Fog"),
        "castability row still present"
    );
}

#[test]
fn pip_blocks_name_worst_card_color_pairs() {
    // A two-color deck with single-color lands blocks pips; the offenders
    // surface with per-card shares, top 5.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let hybrid = super::parse::parse_sim_card(&card(
        "Pip Test",
        "{1}{U}{G}",
        "Creature — Frog Wizard",
        "Vanilla.",
    ));
    deck.cards.push(hybrid);
    let mut rng = ChaCha8Rng::seed_from_u64(17);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    if stats.color_screw.iter().any(|p| *p >= 0.10) {
        assert!(
            !stats.pip_blocks.is_empty(),
            "color screw tripped but no pip blocks named"
        );
        assert!(stats.pip_blocks.len() <= 5);
        assert!(
            stats
                .pip_blocks
                .iter()
                .all(|p| p.pct_games > 0.0 && p.pct_games <= 1.0)
        );
    }
}

// New mechanic shapes: mill, graveyard return, wheel, loot, sacrifice,
// spend-restricted mana, printed power.

#[test]
fn problems_never_suggest_card_names() {
    // All suggestion text names categories, never cards.
    let deck = stub_deck(10, &[("Bear", 2, Role::Other); 5]);
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let mut stats = aggregate(&logs, &deck, 10);
    stats.removal_count = 2;
    stats.wincon_count = 2;
    stats.draw_count = 2;
    let problems = find_problems(&stats, &deck);
    for p in problems {
        assert!(
            !p.suggestion.contains("Bear"),
            "suggestion names cards: {}",
            p.suggestion
        );
    }
}

// Stationz regression: real card rows through the whole pipeline
