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
        let mut first_failure = None;
        for seed in [42, 2026, 20_260_926] {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            for _ in 0..500 {
                let log = run_game(&sim_deck, &mut rng, turns);
                if defining_line_reached(name, &sim_deck, &log) {
                    successes += 1;
                } else {
                    failures += 1;
                    first_failure.get_or_insert(log);
                }
            }
        }
        let failure_detail = first_failure
            .as_ref()
            .map(|log| explain_failed_line(name, &sim_deck, log));
        println!(
            "trace {name} seeds=42,2026,20260926 runs=1500 success={successes} failure={failures}; failed_example={failure_detail:?}"
        );
        assert!(successes > 0, "{name} has no successful line trace");
        assert!(failures > 0, "{name} has no failed line trace");
    }
}

/// Describe the missing resource recorded by one failed defining-line trace.
fn explain_failed_line(name: &str, deck: &SimDeck, log: &GameLog) -> String {
    match name {
        "dredge modern" => format!(
            "no dredge replacement; self-milled={}, final-library={}",
            log.self_milled.last().copied().unwrap_or_default(),
            log.library_size.last().copied().unwrap_or_default()
        ),
        "living-end" => {
            let graveyard_creatures = deck
                .cards
                .iter()
                .enumerate()
                .filter(|(_, card)| card.is_creature)
                .filter(|(index, _)| log.card_first_graveyard.contains_key(index))
                .count();
            let payoff = deck
                .cards
                .iter()
                .position(|card| card.name == "Living End")
                .and_then(|index| log.card_first_battlefield.get(&index));
            format!(
                "graveyard creature copies={graveyard_creatures}; Living End battlefield turn={payoff:?}"
            )
        }
        "neobrand" => {
            let rider = deck
                .cards
                .iter()
                .position(|card| card.name == "Allosaurus Rider");
            let payoff = deck
                .cards
                .iter()
                .position(|card| card.name == "Griselbrand");
            format!(
                "Rider alternative cast={}, Griselbrand battlefield turn={:?}",
                rider.is_some_and(|index| log.alternate_casts.contains(&index)),
                payoff.and_then(|index| log.card_first_battlefield.get(&index))
            )
        }
        "ruby-storm-2026" | "rogsilas turbo naus" => {
            format!("legal graveyard replay casts={}", log.replay_casts)
        }
        "yawgmoth combo modern" => format!(
            "life-funded draws={}, life paid={}",
            log.life_funded_draws, log.life_paid
        ),
        "kinnan combo" => {
            let basalt = deck.cards.iter().any(|card| card.name == "Basalt Monolith");
            format!(
                "Basalt present={basalt}; positive mana loop={}",
                log.infinite_mana_suspected
            )
        }
        _ => "the fixture has no failure explanation".to_string(),
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
                log.alternate_casts.contains(&rider)
                    && log
                        .card_first_battlefield
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
