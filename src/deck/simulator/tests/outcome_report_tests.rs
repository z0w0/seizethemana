use super::*;
use crate::deck::simulator::turn_loop_tests::{deck, row};
use crate::deck::simulator::{findings, game, report, report_schema, report_view};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Produce a real full log as a starting point for isolated reducer tests.
fn logs() -> (SimDeck, GameLog) {
    let cards = deck(
        &(0..20)
            .map(|index| row(&format!("Spell {index}"), "{20}", "Sorcery", ""))
            .collect::<Vec<_>>(),
    );
    let log =
        crate::deck::simulator::game_run::run_game(&cards, &mut ChaCha8Rng::seed_from_u64(7), 3);
    (cards, log)
}

/// Make truncation explicit in isolated aggregation input, not card evidence.
fn incomplete(log: &mut GameLog) {
    log.completion = game::GameCompletion::Incomplete {
        reason: game::IncompleteReason::ActivationPass {
            configured_limit: 32,
            remaining_work: 1,
        },
        turn: 2,
        phase: game::GamePhase::Main,
    };
}

/// Early finished losses count as failures while missing means remain unobserved.
#[test]
fn finished_losses_and_phase_means_use_distinct_samples() {
    let (cards, mut complete) = logs();
    complete
        .first_seen
        .insert(crate::deck::simulator::model::Role::Draw, 2);
    complete.mana_available = vec![4.0; 3];
    complete.interaction_ready = vec![true; 3];
    complete.opponent_activity.draws = 6;
    let mut lost = complete.clone();
    lost.completion = game::GameCompletion::PlayerLost {
        reasons: vec![game::PlayerLossReason::ZeroLife],
        turn: 2,
        phase: game::GamePhase::Draw,
    };
    lost.turn_observations[1..].fill(game::TurnObservation::default());
    lost.first_seen.clear();
    lost.mana_available = vec![2.0, 999.0, 999.0];
    lost.opponent_activity.draws = 2;
    let mut capped = complete.clone();
    incomplete(&mut capped);
    capped.opponent_activity.draws = 100;
    capped.infinite_mana_suspected = true;
    capped.milestones_by_turn[1].positive_mana_loop = true;
    let mut stats = aggregate(&[complete, lost, capped], &cards, 3);
    assert_eq!(
        (
            stats.runs,
            stats.finished_games,
            stats.player_loss_games,
            stats.incomplete_games
        ),
        (3, 2, 1, 1)
    );
    assert_eq!(stats.mana_available, [3.0, 4.0, 4.0]);
    assert_eq!(stats.interaction_ready_by_turn, [1.0, 0.5, 0.5]);
    // A rounded or modified share cannot change exact interval event counts.
    stats.draw_sources[1] = 0.49;
    let mut payload = report::json_report(
        &stats,
        &cards,
        "subset",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );
    let rate = &payload.sampling["draw.draw_source_percent_seen_by_turn.2"];
    assert_eq!((rate.event_games, rate.observed_games), (Some(1), 2));
    let diagnostic = &payload.sampling["win_conditions.percent_games_with_suspected_infinite_mana"];
    assert_eq!(
        (diagnostic.event_games, diagnostic.observed_games),
        (Some(1), 3)
    );
    assert_eq!(payload.phase_observations.main_by_turn[&2], 1.0);
    assert_eq!(payload.mana.avg_available_by_turn[&2], 4.0);
    report::set_opponent_model(&mut payload, cards.format, Default::default(), &stats);
    let activity = &payload
        .opponent_model
        .as_ref()
        .expect("opponent model")
        .observed_activity;
    assert_eq!((activity.attempted_games, activity.observed_games), (3, 2));
    assert_eq!(activity.raw_totals.draws, 108);
    assert_eq!(activity.totals.draws, 8);
    assert_eq!(
        activity.per_game.as_ref().expect("finished mean").draws,
        4.0
    );
    assert_eq!(payload.outcomes.incomplete_observations[0].games, 1);
    assert_eq!(payload.outcomes.incomplete_observations[0].turn, 2);
    let encoded = serde_json::to_value(&payload).expect("serialize report");
    assert_eq!(
        serde_json::from_value::<report_schema::SimReport>(encoded).expect("read current report"),
        payload
    );
}

/// Every affected public primary value is unavailable without finished games.
#[test]
fn all_incomplete_reports_do_not_invent_performance() {
    let (cards, mut log) = logs();
    incomplete(&mut log);
    let stats = aggregate(&[log], &cards, 3);
    assert!(findings::analyze_findings(&stats, &cards).is_empty());
    let mut payload = report::json_report(
        &stats,
        &cards,
        "capped",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert!(payload.opening_hand.avg_lands.is_none());
    assert!(payload.opening_hand.percent_0_lands.is_none());
    assert!(
        payload
            .win_conditions
            .percent_games_with_extra_turn
            .is_none()
    );
    assert!(payload.draw.avg_life_paid.is_none());
    assert!(payload.graveyard.replay_casts_avg.is_none());
    assert!(payload.color_mana_shortage.white.is_none());
    assert!(payload.commander.is_none());
    assert!(payload.combat.commander_potential.is_empty());
    assert!(payload.combat.attack_power_avg_by_turn.is_empty());
    assert!(payload.draw.draw_source_percent_seen_by_turn.is_empty());
    assert!(
        payload
            .interaction
            .percent_games_with_ready_interaction_by_turn
            .is_empty()
    );
    assert!(payload.card_castability.iter().all(
        |row| row.percent_castable_by_target.is_none() && row.avg_first_castable_turn.is_none()
    ));
    assert_eq!(
        payload.sampling["opening_hand.percent_0_lands"].status,
        report_schema::ObservationStatus::NoObservations
    );
    report::set_opponent_model(&mut payload, cards.format, Default::default(), &stats);
    assert!(
        payload
            .opponent_model
            .as_ref()
            .expect("model")
            .observed_activity
            .per_game
            .is_none()
    );
    let diff = report_view::diff_reports(&payload, &payload).expect("current baseline comparison");
    assert!(diff.is_empty());
    assert!(payload.summary.contains("no finished games"));
}

/// Missing later states do not become zero-valued means in JSON or baseline diffs.
#[test]
fn all_losses_keep_missing_later_phases_unavailable() {
    let (cards, mut log) = logs();
    let baseline_stats = aggregate(&[log.clone()], &cards, 3);
    let baseline = report::json_report(
        &baseline_stats,
        &cards,
        "loss",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );
    log.completion = game::GameCompletion::PlayerLost {
        reasons: vec![game::PlayerLossReason::FailedDraw],
        turn: 2,
        phase: game::GamePhase::Draw,
    };
    log.turn_observations[1..].fill(game::TurnObservation::default());
    let stats = aggregate(&[log], &cards, 3);
    let payload = report::json_report(
        &stats,
        &cards,
        "loss",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert_eq!(stats.finished_games, 1);
    assert_eq!(stats.player_loss_games, 1);
    assert_eq!(payload.mana.avg_available_by_turn.len(), 1);
    assert_eq!(payload.phase_observations.main_by_turn[&3], 0.0);
    assert_eq!(
        payload.sampling["draw.draw_source_percent_seen_by_turn.3"].observed_games,
        1
    );
    let diff =
        report_view::diff_reports(&baseline, &payload).expect("compare partial observations");
    assert!(
        diff.metrics
            .iter()
            .any(|delta| delta.path == "mana.avg_available_by_turn.3"
                && delta.new == report_view::MetricValue::Null)
    );
}

/// Pair and stored-combo analyses use the same finished subset as primary rates.
#[test]
fn optional_access_analyses_exclude_incomplete_early_successes() {
    let (cards, mut complete) = logs();
    complete.card_first_seen.clear();
    let mut capped = complete.clone();
    capped.card_first_seen.insert(0, 1);
    capped.card_first_seen.insert(1, 1);
    incomplete(&mut capped);
    let pairs = vec![(cards.cards[0].name.clone(), cards.cards[1].name.clone())];
    let rows = findings::piece_pair_access(&[complete.clone(), capped.clone()], &cards, &pairs, 20);
    let pair = report_schema::PairAccessReport::from_access(&rows[0], 2, 20);
    assert_eq!(
        (pair.access.event_games, pair.access.observed_games),
        (Some(0), 1)
    );
    let only_capped = findings::piece_pair_access(&[capped.clone()], &cards, &pairs, 20);
    assert_eq!(
        report_schema::PairAccessReport::from_access(&only_capped[0], 1, 20)
            .access
            .status,
        report_schema::ObservationStatus::NoObservations
    );
    let candidate = crate::deck::simulator::combos::ComboCandidate {
        id: "test".into(),
        produces: vec![],
        mana_value_needed: 1,
        bracket_tag: None,
        complete: true,
        missing: vec![],
        pieces: vec![crate::deck::simulator::combos::PieceRef {
            name: cards.cards[0].name.clone(),
            indices: vec![0],
            starts_in_command_zone: false,
            starts_in_library: true,
            zones: vec![crate::deck::simulator::combos::PieceZone::Hand],
        }],
    };
    let candidates = [candidate];
    let assembly = crate::deck::simulator::combos::measure(
        &candidates,
        &[complete, capped.clone()],
        &cards,
        20,
    );
    assert_eq!(
        (
            assembly.complete[0].access.event_games,
            assembly.complete[0].access.observed_games
        ),
        (Some(0), 1)
    );
    let assembly = crate::deck::simulator::combos::measure(&candidates, &[capped], &cards, 20);
    assert_eq!(
        assembly.complete[0].access.status,
        report_schema::ObservationStatus::NoObservations
    );
}
