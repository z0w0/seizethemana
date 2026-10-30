use super::*;
use game::{CardRef, GameCompletion, GameLog, GamePhase, GameState, PlayerLossReason};
use game_commander::CommanderProfile;
use game_run::TurnCensus;
use model::{CardIdx, SimDeck};
use report_schema::{ObservationStatus, SimReport};
use scenario_test_support::{deck, row, state};

/// Seed hand-access records as the production opener does.
fn seen_state(hand: Vec<usize>, library: Vec<usize>) -> GameState {
    let mut st = state(hand, library);
    st.seen = st.hand.len() as u32;
    st.awareness_cards = st.seen;
    for index in st.hand.clone() {
        game::record_card_seen(&mut st, index, 1);
    }
    st
}

/// Run real turn phases until a terminal boundary or the requested horizon.
fn run_turns(cards: &SimDeck, st: &mut GameState, turns: usize) -> TurnCensus {
    let mut census =
        TurnCensus::with_commander_count(turns, cards.cards.len(), cards.commanders.len());
    let profile = CommanderProfile::new(cards);
    let mut sources = Vec::new();
    for turn in 1..=turns {
        game_run::run_turn(cards, st, &mut census, &profile, &mut sources, turn, false);
        if st.completion.is_some() {
            break;
        }
    }
    census
}

/// Keep the same aggregation and JSON paths used by production reports.
fn json_report(cards: &SimDeck, logs: &[GameLog], turns: u32) -> SimReport {
    let stats = aggregate::aggregate(logs, cards, turns);
    report::json_report(
        &stats,
        cards,
        "pre-loss events",
        1,
        &[],
        Default::default(),
        &Default::default(),
    )
}

/// A last-life payment records payability without entering or casting a partner.
#[test]
fn last_life_commander_payment_keeps_payability_and_report_timing() {
    let mut cards = deck(&[]);
    cards.commanders = deck(&[
        row(
            "Life commander",
            "{B/P}",
            "Legendary Creature",
            "When this creature enters, gain 3 life.",
        ),
        row("Partner", "{0}", "Legendary Creature", ""),
    ])
    .cards;
    let mut st = seen_state(vec![], vec![]);
    st.life = 2;

    let census = run_turns(&cards, &mut st, 4);

    assert_eq!(st.life, 0);
    assert_eq!(st.life_paid, 2);
    assert_eq!(st.life_gained, 0);
    assert_eq!(st.commander_casts, [1, 0]);
    assert!(st.battlefield.is_empty());
    assert_eq!(
        st.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::ZeroLife],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert_eq!(census.commander_castable, Some(1));
    assert!(!census.turn_observations[0].main);
    let log = game_run::census::finish_log(&cards, st, census, 0, false, 4);
    assert_eq!(log.commander_castable, Some(1));
    let payload = json_report(&cards, &[log], 4);
    let commander = payload.commander.expect("a commander report is available");
    assert_eq!(commander.payable_games, 1);
    assert_eq!(commander.avg_first_cast_turn, Some(1.0));
    assert_eq!(commander.p50_cast_turn, Some(1));
    assert_eq!(commander.p95_cast_turn, Some(1));
    assert_eq!(commander.percent_castable_by_turn[&4].value(), 100.0);
    assert_eq!(
        payload.sampling["commander.percent_castable_by_turn.4"].event_games,
        Some(1)
    );
    assert_eq!(
        payload.sampling["commander.percent_castable_by_curve"].event_games,
        Some(1)
    );
    assert!(payload.mana.avg_available_by_turn.is_empty());
}

/// A terminal cast trigger keeps the commander's known payable turn.
#[test]
fn commander_cast_trigger_loss_keeps_payability_before_entry() {
    let mut cards = deck(&[row(
        "Cast listener",
        "{9}",
        "Artifact",
        "Whenever you cast a spell, draw a card.",
    )]);
    cards.commanders = deck(&[
        row("Commander", "{0}", "Legendary Creature", ""),
        row("Partner", "{0}", "Legendary Creature", ""),
    ])
    .cards;
    let mut st = seen_state(vec![], vec![]);
    st.battlefield
        .push(game::new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.next_uid = 1;

    let census = run_turns(&cards, &mut st, 4);

    assert!(st.failed_draw);
    assert_eq!(st.commander_casts, [1, 0]);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(census.commander_castable, Some(1));
    let log = game_run::census::finish_log(&cards, st, census, 0, false, 4);
    let payload = json_report(&cards, &[log], 4);
    let commander = payload.commander.expect("a commander report is available");
    assert_eq!(commander.payable_games, 1);
    assert_eq!(commander.p50_cast_turn, Some(1));
    assert_eq!(commander.percent_castable_by_turn[&1].value(), 100.0);
    assert_eq!(
        payload.sampling["commander.percent_castable_by_turn.1"].event_games,
        Some(1)
    );
}

/// Failed life or mana checks do not count, and a payable partner stays separate.
#[test]
fn unpayable_commanders_do_not_record_a_payable_turn() {
    for (cost, partner) in [("{B/P}", false), ("{U}", false), ("{B/P}", true)] {
        let mut cards = deck(&[]);
        cards.commanders = deck(&[row("Commander", cost, "Legendary Creature", "")]).cards;
        if partner {
            cards
                .commanders
                .extend(deck(&[row("Partner", "{0}", "Legendary Creature", "")]).cards);
        }
        let mut st = seen_state(vec![], vec![]);
        st.life = 1;

        let census = run_turns(&cards, &mut st, 1);

        assert!(st.completion.is_none());
        assert_eq!(st.life, 1);
        assert_eq!(st.commander_casts[0], 0);
        assert_eq!(census.commander_castable, partner.then_some(1));
        if partner {
            assert_eq!(st.commander_casts, [0, 1]);
            assert_eq!(st.battlefield[0].card, CardRef::Commander { slot: 1 });
        } else {
            assert!(st.battlefield.is_empty());
        }
        let log = game_run::census::finish_log(&cards, st, census, 0, false, 1);
        let payload = json_report(&cards, &[log], 1);
        let commander = payload.commander.expect("a commander report is available");
        assert_eq!(commander.payable_games, u32::from(partner));
        assert_eq!(commander.avg_first_cast_turn, partner.then_some(1.0));
        let rate = &payload.sampling["commander.percent_castable_by_turn.1"];
        assert_eq!(
            (rate.event_games, rate.observed_games),
            (Some(u32::from(partner)), 1)
        );
    }
}

/// Lose at turn-two upkeep after observing an exact opening land count.
fn land_upkeep_loss(lands: usize, turns: usize) -> (SimDeck, GameLog) {
    let mut cards = deck(&vec![
        row(
            "Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}."
        );
        6
    ]);
    cards.commanders = deck(&[row(
        "Upkeep commander",
        "{0}",
        "Legendary Creature",
        "At the beginning of your upkeep, you lose 1 life.",
    )])
    .cards;
    let mut st = seen_state((0..lands).collect(), vec![]);
    st.life = 1;
    let census = run_turns(&cards, &mut st, turns);
    assert_eq!(
        st.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::ZeroLife],
            turn: 2,
            phase: GamePhase::Upkeep,
        })
    );
    assert_eq!(census.lands_seen[0], lands as u32);
    assert!(census.lands_seen[1..].iter().all(|count| *count == 0));
    assert!(census.cards_seen[1..].iter().all(|count| *count == 0));
    let log = game_run::census::finish_log(&cards, st, census, lands as u8, false, turns);
    (cards, log)
}

/// A known fixed-window success survives loss without filling missing turn means.
#[test]
fn early_upkeep_loss_retains_only_reached_land_thresholds() {
    for lands in [5, 6] {
        let (cards, log) = land_upkeep_loss(lands, 4);
        assert_eq!(log.lands_seen_by_turn_4, lands as u32);
        assert_eq!(log.cards_seen_by_4, lands as u32);
        assert_eq!(log.turns_observed, 1);
        assert!(log.turn_observations[1..].iter().all(|row| !row.end_turn));
        assert_eq!(log.cards_seen, [lands as u32, 0, 0, 0]);
        let payload = json_report(&cards, &[log], 4);
        let expected = f64::from(lands == 6) * 100.0;
        assert_eq!(
            payload
                .land_drops
                .percent_games_with_six_or_more_lands_by_turn_4
                .expect("finished loss supports the fixed-window event rate")
                .value(),
            expected
        );
        let rate = &payload.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"];
        assert_eq!(
            (rate.event_games, rate.observed_games),
            (Some(u32::from(lands == 6)), 1)
        );
        assert_eq!(rate.percent.expect("observed event rate").value(), expected);
        assert_eq!(payload.velocity.avg_cards_seen.len(), 1);
        assert_eq!(payload.velocity.avg_cards_seen[&1], lands as f64);
        assert_eq!(payload.phase_observations.end_turn_by_turn[&4], 0.0);
    }
}

/// Hand and graveyard access during a terminal spell still count in its land window.
#[test]
fn terminal_draw_and_mill_keep_land_access_without_an_end_turn() {
    let mut rows = vec![row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."); 6];
    rows.push(row(
        "Terminal spell",
        "{0}",
        "Sorcery",
        "Draw two cards. Mill three cards. You lose 1 life.",
    ));
    let cards = deck(&rows);
    let mut st = seen_state(vec![0, 6], vec![1, 2, 3, 4, 5]);
    st.life = 1;

    let census = run_turns(&cards, &mut st, 4);

    assert!(matches!(
        st.completion,
        Some(GameCompletion::PlayerLost { .. })
    ));
    assert_eq!(st.card_first_seen.len(), 4);
    assert_eq!(st.graveyard_seen.len(), 4);
    assert_eq!(st.battlefield_seen.get(&CardIdx(0)), Some(&1));
    assert_eq!(st.milled_self, 3);
    assert!(census.lands_seen.iter().all(|count| *count == 0));
    assert!(census.cards_seen.iter().all(|count| *count == 0));
    let log = game_run::census::finish_log(&cards, st, census, 1, false, 4);
    assert_eq!(log.lands_seen_by_turn_4, 6);
    assert_eq!(log.cards_seen_by_4, 7);
    assert_eq!(log.turns_observed, 0);
    let payload = json_report(&cards, &[log], 4);
    let rate = &payload.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"];
    assert_eq!((rate.event_games, rate.observed_games), (Some(1), 1));
    assert!(payload.velocity.avg_cards_seen.is_empty());
}

/// Reaching six lands on turn five cannot change the turn-four event.
#[test]
fn land_threshold_reached_after_turn_four_does_not_count() {
    let mut rows = vec![row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."); 6];
    rows.extend((0..3).map(|index| row(&format!("Filler {index}"), "{20}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut st = seen_state((0..5).collect(), vec![5, 8, 7, 6]);

    let census = run_turns(&cards, &mut st, 5);

    assert!(st.completion.is_none());
    assert_eq!(st.card_first_seen.get(&5), Some(&5));
    assert_eq!(census.lands_seen, [5, 5, 5, 5, 6]);
    let log = game_run::census::finish_log(&cards, st, census, 5, false, 5);
    assert_eq!(log.lands_seen_by_turn_4, 5);
    let payload = json_report(&cards, &[log], 5);
    let rate = &payload.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"];
    assert_eq!((rate.event_games, rate.observed_games), (Some(0), 1));
    assert_eq!(rate.percent.expect("observed event rate").value(), 0.0);
}

/// A known early success does not make a short requested window available.
#[test]
fn short_horizon_keeps_the_turn_four_land_rate_unavailable() {
    let (cards, log) = land_upkeep_loss(6, 3);
    assert_eq!(log.lands_seen_by_turn_4, 6);
    let payload = json_report(&cards, &[log], 3);
    assert!(
        payload
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4
            .is_none()
    );
    let rate = &payload.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"];
    assert_eq!(rate.status, ObservationStatus::OutsideHorizon);
    assert_eq!((rate.event_games, rate.observed_games), (None, 0));
}

/// Incomplete early successes stay outside the primary event count and denominator.
#[test]
fn incomplete_land_success_is_excluded_from_primary_rates() {
    let mut rows = vec![row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."); 6];
    rows.push(row(
        "Capped spell",
        "{0}",
        "Sorcery",
        "Create nine 1/1 white Soldier creature tokens.",
    ));
    let cards = deck(&rows);
    assert!(matches!(
        cards.cards[6].spell_data.effects.as_slice(),
        [model::SimEffect::Tokens(9)]
    ));
    let mut st = seen_state((0..7).collect(), vec![]);
    let census = run_turns(&cards, &mut st, 4);
    assert_eq!(
        st.completion,
        Some(GameCompletion::Incomplete {
            reason: game::IncompleteReason::EffectWork {
                configured_limit: 8,
                remaining_work: 1,
            },
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert_eq!(
        st.battlefield
            .iter()
            .filter(|permanent| matches!(permanent.card, CardRef::Token(_)))
            .count(),
        8
    );
    let capped = game_run::census::finish_log(&cards, st, census, 6, false, 4);
    assert_eq!(capped.lands_seen_by_turn_4, 6);
    assert_eq!(capped.cards_seen_by_4, 7);
    assert!(capped.cards_seen.iter().all(|count| *count == 0));
    assert!(capped.turn_observations.iter().all(|turn| !turn.end_turn));
    let payload = json_report(&cards, std::slice::from_ref(&capped), 4);
    assert!(
        payload
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4
            .is_none()
    );
    let rate = &payload.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"];
    assert_eq!(rate.status, ObservationStatus::NoObservations);
    assert_eq!((rate.event_games, rate.observed_games), (None, 0));
    assert!(payload.velocity.avg_cards_seen.is_empty());
    assert!(payload.mana.avg_available_by_turn.is_empty());

    let mut st = seen_state((0..5).collect(), vec![]);
    let census = run_turns(&cards, &mut st, 4);
    assert!(matches!(
        st.completion,
        Some(GameCompletion::PlayerLost { .. })
    ));
    let lost = game_run::census::finish_log(&cards, st, census, 5, false, 4);
    assert_eq!(lost.lands_seen_by_turn_4, 5);
    let payload = json_report(&cards, &[lost, capped], 4);
    assert_eq!(
        (
            payload.outcomes.finished_games,
            payload.outcomes.incomplete_games
        ),
        (1, 1)
    );
    assert_eq!(
        payload
            .land_drops
            .percent_games_with_six_or_more_lands_by_turn_4
            .expect("one finished game supports the rate")
            .value(),
        0.0
    );
    let rate = &payload.sampling["land_drops.percent_games_with_six_or_more_lands_by_turn_4"];
    assert_eq!((rate.event_games, rate.observed_games), (Some(0), 1));
    assert_eq!(rate.percent.expect("observed event rate").value(), 0.0);
}
