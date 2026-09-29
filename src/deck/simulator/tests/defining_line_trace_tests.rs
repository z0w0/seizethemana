use super::deck_test_support::*;
use super::game::{GameLog, run_game};
use super::model::SimDeck;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

#[test]
fn target_decks_have_successful_and_failed_fixed_seed_traces() {
    for name in [
        "dredge modern",
        "living-end",
        "neobrand",
        "ruby-storm-2026",
        "yawgmoth combo modern",
        "kinnan combo",
        "rogsilas turbo naus",
    ] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let sim_deck = build_sim_deck(&deck, &cards, None);
        let turns = if sim_deck.format == super::model::Format::Commander {
            10
        } else {
            8
        };
        let mut successes = 0;
        let mut failures = 0;
        for seed in [42, 2026, 20_260_926] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            for _ in 0..500 {
                let log = run_game(&sim_deck, &mut rng, turns);
                if defining_line_reached(name, &sim_deck, &log) {
                    successes += 1;
                } else {
                    failures += 1;
                }
            }
        }
        assert!(successes > 0, "{name} has no successful line trace");
        assert!(failures > 0, "{name} has no failed line trace");
    }
}

/// Check whether one game log records the fixture's main supported action.
fn defining_line_reached(name: &str, deck: &SimDeck, log: &GameLog) -> bool {
    match name {
        "dredge modern" => log
            .milestones_by_turn
            .iter()
            .any(|turn| turn.dredge_uses > 0),
        "living-end" => deck.cards.iter().enumerate().any(|(index, card)| {
            card.is_creature
                && log
                    .card_first_graveyard
                    .get(&index)
                    .zip(log.card_first_battlefield.get(&index))
                    .is_some_and(|(graveyard, battlefield)| graveyard < battlefield)
        }),
        "neobrand" => {
            let rider = deck
                .cards
                .iter()
                .position(|card| card.name == "Allosaurus Rider");
            let griselbrand = deck
                .cards
                .iter()
                .position(|card| card.name == "Griselbrand");
            rider.zip(griselbrand).is_some_and(|(rider, griselbrand)| {
                log.card_first_battlefield
                    .get(&griselbrand)
                    .is_some_and(|turn| *turn <= 8)
                    && log
                        .card_first_graveyard
                        .get(&rider)
                        .zip(log.card_first_battlefield.get(&griselbrand))
                        .is_some_and(|(sacrifice, entry)| sacrifice <= entry)
            })
        }
        "ruby-storm-2026" | "rogsilas turbo naus" => log.replay_casts > 0,
        "yawgmoth combo modern" => log.life_funded_draws > 0,
        "kinnan combo" => log.infinite_mana_suspected,
        _ => false,
    }
}
