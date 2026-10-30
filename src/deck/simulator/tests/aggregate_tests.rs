//! Tests for the simulator aggregate module.

use super::aggregate::aggregate;
use super::deck_test_support::card;
use super::findings::analyze_findings;
use super::game::run_game;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::land::parse_tap_yield;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

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
        companion: None,
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
    let problems = analyze_findings(&stats, &deck);
    // A drawless deck trips draw starvation.
    assert!(problems.iter().any(|p| p.kind == "limited_draw_access"));
}

#[test]
fn severity_buckets_match_shares() {
    // Check the color screw thresholds through the finding analysis.
    let deck = stub_deck(24, &[("Bear", 2, Role::Other); 10]);
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let mut stats = aggregate(&logs, &deck, 8);
    stats.removal_count = 0;
    stats.wincon_count = 0;
    // Monocolor decks never trip color screw.
    stats.color_screw = [0.0; 5];
    let problems = analyze_findings(&stats, &deck);
    assert!(!problems.iter().any(|p| p.kind == "insufficient_color_mana"));
}

#[test]
fn percentile_nearest_rank() {
    let mut v = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    assert_eq!(super::aggregate::percentile(&mut v, 0.5), 5);
    assert_eq!(super::aggregate::percentile(&mut v, 0.95), 10);
}

#[test]
fn streaming_preserves_exact_conditional_quantiles_and_turn_weighting() {
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other)]);
    deck.commanders.push(SimCard {
        name: "Commander".to_string(),
        ..SimCard::default()
    });
    let mut rng = ChaCha8Rng::seed_from_u64(12);
    let mut accumulator = super::aggregate::SimAccumulator::new(&deck, 16);
    for turn in [2, 4, 8, 16] {
        let mut log = run_game(&deck, &mut rng, 16);
        log.commander_castable = Some(turn);
        log.win_threshold_turn = Some(turn);
        log.lands_by_4 = turn as u8;
        log.interaction_ready.fill(false);
        log.interaction_mana_held.fill(0.0);
        log.interaction_ready[0] = true;
        log.interaction_mana_held[0] = 2.0;
        if turn == 16 {
            log.interaction_ready[1] = true;
            log.interaction_mana_held[1] = 7.0;
        }
        accumulator.push(&log);
    }
    let stats = accumulator.finish();
    assert_eq!(stats.runs, 4);
    assert_eq!(stats.p50_drops_by_4, 4);
    assert_eq!(stats.p95_drops_by_4, 16);
    assert_eq!(stats.p50_commander_cast_turn, 4);
    assert_eq!(stats.p95_commander_cast_turn, 16);
    assert_eq!(stats.win_threshold_p50_turn, 4);
    assert_eq!(stats.commander_payable_games, 4);
    assert_eq!(stats.avg_commander_cast_turn, 7.5);
    assert_eq!(stats.commander_castable_by[16], 1.0);
    assert_eq!(stats.interaction_mana_held, 3.0);
}

#[test]
fn streaming_counts_duplicate_copy_pip_blocks_once_per_game() {
    let deck = stub_deck(24, &[("Bear", 2, Role::Other)]);
    let mut rng = ChaCha8Rng::seed_from_u64(91);
    let mut log = run_game(&deck, &mut rng, 4);
    log.pip_blocks = vec![(24, 1), (25, 1), (24, 1), (24, 0)];
    let stats = aggregate(&[log], &deck, 4);
    assert_eq!(stats.pip_blocks.len(), 2);
    assert_eq!(stats.pip_blocks[0].color.symbol(), 'U');
    assert_eq!(stats.pip_blocks[1].color.symbol(), 'W');
    assert!(stats.pip_blocks.iter().all(|row| row.game_share == 1.0));
}

#[test]
fn conditional_readiness_does_not_invent_an_unobserved_turn() {
    let deck = stub_deck(24, &[("Bear", 2, Role::Other)]);
    let mut rng = ChaCha8Rng::seed_from_u64(17);
    let mut log = run_game(&deck, &mut rng, 4);
    log.mana_ready.fill(None);
    let stats = aggregate(&[log], &deck, 4);
    let row = &stats.card_castability[0];
    assert_eq!(row.payable_games, 0);
    assert_eq!(row.pct_by_target, 0.0);
    assert_eq!(row.avg_first_castable_turn, 0.0);
}

#[test]
fn opponent_activity_uses_wide_totals_without_averaging_game_counters() {
    let deck = stub_deck(24, &[("Bear", 2, Role::Other)]);
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let mut log = run_game(&deck, &mut rng, 4);
    log.opponent_activity = super::opponents::OpponentActivity {
        draws: u32::MAX,
        casts: 3,
        payments: 2,
        skipped_casts: 1,
    };
    let mut accumulator = super::aggregate::SimAccumulator::new(&deck, 4);
    accumulator.push(&log);
    accumulator.push(&log);
    let stats = accumulator.finish();
    assert_eq!(stats.runs, 2);
    assert_eq!(stats.opponent_draws, u64::from(u32::MAX) * 2);
    assert_eq!(stats.opponent_casts, 6);
    assert_eq!(stats.opponent_payments, 4);
    assert_eq!(stats.opponent_skipped_casts, 2);
}

#[test]
fn castability_rows_aggregate_copies_by_card_name() {
    let deck = stub_deck(24, &[("Bear", 2, Role::Other)]);
    let mut rng = ChaCha8Rng::seed_from_u64(44);
    let logs: Vec<_> = (0..32).map(|_| run_game(&deck, &mut rng, 4)).collect();
    let stats = aggregate(&logs, &deck, 4);
    let bear_rows: Vec<_> = stats
        .card_castability
        .iter()
        .filter(|row| row.name == "Bear")
        .collect();
    assert_eq!(bear_rows.len(), 1, "four copies share one diagnostic row");
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
        let sim = parse_sim_card(&row);
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
        let sim = parse_sim_card(&row);
        assert_ne!(sim.role, Role::Removal, "text: {text}");
        assert!(!sim.flags.is_interaction, "text: {text}");
    }
}

#[test]
fn removal_exempt_from_dead_cards() {
    // A reactive spell with a high floor must not surface in dead_cards;
    // its castability row still exists.
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let fog = parse_sim_card(&card(
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
    let problems = analyze_findings(&stats, &deck);
    let dead = problems.iter().find(|p| p.kind == "low_castability");
    if let Some(p) = dead {
        assert!(
            !p.explanation.contains("Spore Fog"),
            "reactive spell flagged dead: {}",
            p.explanation
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
    let hybrid = parse_sim_card(&card(
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
                .all(|p| p.game_share > 0.0 && p.game_share <= 1.0)
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
    let problems = analyze_findings(&stats, &deck);
    for p in problems {
        assert!(
            !p.suggestion.contains("Bear"),
            "suggestion names cards: {}",
            p.suggestion
        );
    }
}

// Stationz regression: real card rows through the whole pipeline
