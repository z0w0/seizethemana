use super::*;
use crate::deck::simulator::turn_loop_tests::{deck, row, state};
use crate::deck::simulator::{aggregate, game, game_commander, game_run, report, report_schema};
use rand::RngCore;

/// Run finite, non-mana cast sequences through full turns and public reporting.
#[test]
fn hand_cast_cap_requires_a_payable_next_cast_and_stops_later_phases() {
    let limit = crate::deck::simulator::model::MAX_LOOP_PASSES;
    for extra in [0, 1] {
        let count = limit + extra;
        let mut rows: Vec<_> = (0..count)
            .map(|index| {
                row(
                    &format!("Free artifact {index}"),
                    "{0}",
                    "Artifact",
                    "When this artifact enters, gain 1 life.",
                )
            })
            .collect();
        rows.push(row("Unaffordable", "{100}", "Sorcery", "Gain 1 life."));
        rows.push(row("Future draw", "{100}", "Sorcery", ""));
        let cards = deck(&rows);
        let mut st = state((0..=count as usize).collect(), vec![count as usize + 1]);
        let mut census = game_run::census::TurnCensus::new(2, cards.cards.len());
        let mut sources = vec![];
        let profile = game_commander::CommanderProfile::new(&cards);
        game_run::run_turn(
            &cards,
            &mut st,
            &mut census,
            &profile,
            &mut sources,
            1,
            false,
        );
        assert_eq!(st.spells_cast_this_turn, limit);
        assert_eq!(st.life_gained, limit);
        assert!(!st.infinite_mana_suspected);
        assert_eq!(st.completion.is_some(), extra > 0);
        let hand = st.hand.clone();
        game_run::run_turn(
            &cards,
            &mut st,
            &mut census,
            &profile,
            &mut sources,
            2,
            false,
        );
        if extra > 0 {
            assert_eq!(st.hand, hand);
            assert_eq!(st.current_turn, 1);
            assert_eq!(st.library, [CardIdx(count + 1)]);
            assert!(
                matches!(st.completion, Some(game::GameCompletion::Incomplete { reason: game::IncompleteReason::CastPass { configured_limit, remaining_work: 1 }, turn: 1, phase: game::GamePhase::Main }) if configured_limit == limit)
            );
            assert!(!census.turn_observations[0].main);
            assert!(!census.turn_observations[0].combat);
            assert!(!census.turn_observations[0].end_turn);
            assert_eq!(
                census.turn_observations[1],
                game::TurnObservation::default()
            );
        } else {
            assert!(census.turn_observations.iter().all(|turn| turn.end_turn));
        }
        let log = game_run::census::finish_log(&cards, st, census, 0, false, 2);
        assert_cap_report(&cards, log, extra > 0, "cast_pass_limit", "cast_pass");
    }
}

/// Own flashback, granted flashback, and escape share exact replay-budget rules.
#[test]
fn graveyard_cast_caps_cover_each_replay_route_and_public_counts() {
    let limit = crate::deck::simulator::model::MAX_LOOP_PASSES;
    for route in ["flashback", "granted", "escape"] {
        for extra in [0, 1] {
            let count = limit + extra;
            let (cards, mut st) = replay_scenario(route, count);
            let mut census = game_run::census::TurnCensus::new(2, cards.cards.len());
            let profile = game_commander::CommanderProfile::new(&cards);
            // Turn-start cleanup expires granted permissions; test that route at its main-phase boundary.
            if route == "granted" {
                st.current_turn = 1;
                st.current_phase = game::GamePhase::Main;
                let mut pool = game_run::build_pool(&cards, &mut st, 1);
                cast_graveyard_spells(
                    &cards,
                    &mut st,
                    &mut pool,
                    1,
                    &mut census.mana_spent,
                    &mut vec![],
                );
            } else {
                game_run::run_turn(
                    &cards,
                    &mut st,
                    &mut census,
                    &profile,
                    &mut vec![],
                    1,
                    false,
                );
            }
            assert_eq!(st.replay_casts, limit, "{route}, extra {extra}");
            assert_eq!(st.life_gained, limit, "{route}, extra {extra}");
            assert!(!st.infinite_mana_suspected);
            assert_eq!(st.completion.is_some(), extra > 0, "{route}");
            if extra > 0 {
                assert!(
                    matches!(st.completion, Some(game::GameCompletion::Incomplete { reason: game::IncompleteReason::GraveyardCastPass { configured_limit, remaining_work: 1 }, turn: 1, phase: game::GamePhase::Main }) if configured_limit == limit)
                );
                let remaining = st.graveyard.clone();
                game_run::run_turn(
                    &cards,
                    &mut st,
                    &mut census,
                    &profile,
                    &mut vec![],
                    2,
                    false,
                );
                assert_eq!(st.graveyard, remaining);
                assert_eq!(st.library, [CardIdx(count + 1)]);
                assert_eq!(
                    census.turn_observations[1],
                    game::TurnObservation::default()
                );
            } else {
                game_run::run_turn(
                    &cards,
                    &mut st,
                    &mut census,
                    &profile,
                    &mut vec![],
                    2,
                    false,
                );
            }
            let log = game_run::census::finish_log(&cards, st, census, 0, false, 2);
            assert_cap_report(
                &cards,
                log,
                extra > 0,
                "graveyard_cast_pass_limit",
                "graveyard_cast_pass",
            );
        }
    }
}

/// Prepare distinct replay cards, legal Escape fodder, and real mana sources.
fn replay_scenario(route: &str, count: u32) -> (SimDeck, GameState) {
    let mut rows: Vec<_> = (0..count).map(|index| {
        let (kind, cost, text) = match route {
            "flashback" => ("Sorcery", "{10}", "Gain 1 life.\nFlashback {1}"),
            "granted" => ("Sorcery", "{1}", "Gain 1 life."),
            "escape" => ("Creature", "{10}", "When this creature enters, gain 1 life.\nEscape—{1}, Exile three other cards from your graveyard."),
            _ => unreachable!("fixed test routes"),
        };
        row(&format!("Replay {index}"), cost, kind, text)
    }).collect();
    rows.push(row(
        "Unaffordable replay",
        "{100}",
        "Sorcery",
        "Gain 1 life.\nFlashback {100}",
    ));
    rows.push(row("Future draw", "{100}", "Sorcery", ""));
    let fodder_start = rows.len() as u32;
    if route == "escape" {
        rows.extend(
            (0..count * 3)
                .map(|index| row(&format!("Escape fodder {index}"), "{100}", "Artifact", "")),
        );
    }
    let fodder_end = rows.len() as u32;
    let land_start = rows.len() as u32;
    rows.extend((0..count).map(|_| row("Forest", "", "Basic Land — Forest", "{T}: Add {G}.")));
    let cards = deck(&rows);
    let mut st = state(vec![], vec![count as usize + 1]);
    st.graveyard = (fodder_start..fodder_end)
        .chain(0..=count)
        .map(CardIdx)
        .collect();
    for index in 0..count {
        st.battlefield.push(game::new_perm_with(
            10_000 + index,
            &cards,
            CardIdx(land_start + index),
            0,
            false,
        ));
    }
    if route == "granted" {
        st.flashback_permissions
            .extend((0..count).map(|index| (CardIdx(index), 0)));
    }
    (cards, st)
}

/// Check typed reasons and reconciled public counts from the production log path.
fn assert_cap_report(
    cards: &SimDeck,
    log: game::GameLog,
    incomplete: bool,
    label: &str,
    kind: &str,
) {
    let stats = aggregate::aggregate(&[log], cards, 2);
    assert_eq!(stats.runs, 1);
    assert_eq!(stats.finished_games, u32::from(!incomplete));
    assert_eq!(stats.incomplete_games, u32::from(incomplete));
    let payload = report::json_report(
        &stats,
        cards,
        "cast budget",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert_eq!(
        payload.outcomes.finished_games + payload.outcomes.incomplete_games,
        1
    );
    if incomplete {
        assert_eq!(payload.outcomes.incomplete_reasons[label], 1);
        let json = serde_json::to_value(&payload).expect("serialize cap report");
        let evidence = &json["outcomes"]["incomplete_observations"][0];
        assert_eq!(evidence["reason"]["kind"], kind);
        assert_eq!(
            evidence["reason"]["configured_limit"],
            crate::deck::simulator::model::MAX_LOOP_PASSES
        );
        assert_eq!(evidence["reason"]["remaining_work"], 1);
        assert_eq!(evidence["phase"], "main");
        assert_eq!(evidence["games"], 1);
        assert_eq!(
            serde_json::from_value::<report_schema::SimReport>(json)
                .expect("read current cap report"),
            payload
        );
    } else {
        assert!(payload.outcomes.incomplete_reasons.is_empty());
        assert!(payload.outcomes.incomplete_observations.is_empty());
    }
}

/// Cast probes use actual payment and target gates, without touching real state or RNG.
#[test]
fn cast_work_probes_reject_unpayable_candidates_without_side_effects() {
    for (cost, text) in [
        ("{2}", "Gain 1 life."),
        ("{R}", "Gain 1 life."),
        (
            "{0}",
            "As an additional cost to cast this spell, sacrifice a creature. Gain 1 life.",
        ),
        (
            "{0}",
            "Return target creature card from your graveyard to the battlefield.",
        ),
    ] {
        let cards = deck(&[row("Candidate", cost, "Sorcery", text)]);
        let st = state(vec![0], vec![]);
        let pool = ManaPool {
            colorless: 1,
            ..Default::default()
        };
        let mut expected_rng = st.library_rng.clone();
        assert!(
            !work::has_hand_cast(&cards, &st, &pool, 1, &[CardIdx(0)]),
            "{text}"
        );
        assert_eq!(st.hand, [CardIdx(0)]);
        assert!(st.graveyard.is_empty());
        assert_eq!(pool.total(), 1);
        assert_eq!(st.spells_cast_this_turn, 0);
        assert_eq!(st.library_rng.clone().next_u64(), expected_rng.next_u64());
    }
}

/// The final hand-cast probe also recognizes legal face-down and alternative-cost casts.
#[test]
fn cast_work_probes_cover_face_down_and_alternative_routes() {
    let cards = deck(&[row(
        "Face-down candidate",
        "{100}",
        "Creature",
        "Morph {100}",
    )]);
    let st = state(vec![0], vec![]);
    assert!(work::has_hand_cast(
        &cards,
        &st,
        &ManaPool {
            colorless: 3,
            ..Default::default()
        },
        1,
        &[CardIdx(0)]
    ));
    assert!(!work::has_hand_cast(
        &cards,
        &st,
        &ManaPool {
            colorless: 2,
            ..Default::default()
        },
        1,
        &[CardIdx(0)]
    ));
    assert!(st.battlefield.is_empty());
    assert_eq!(st.hand, [CardIdx(0)]);
    let mut cards = deck(&[
        row(
            "Pitch candidate",
            "{100}",
            "Sorcery",
            "You may exile one blue card from your hand rather than pay this spell's mana cost.\nGain 1 life.",
        ),
        row("Blue fodder", "{100}{U}", "Sorcery", ""),
    ]);
    cards.cards[1].colors[1] = true;
    assert!(cards.cards[0].spell_data.alternative_cast_cost.is_some());
    let st = state(vec![0, 1], vec![]);
    assert!(work::has_hand_cast(
        &cards,
        &st,
        &ManaPool::default(),
        1,
        &[CardIdx(0)]
    ));
    assert_eq!(st.hand, [CardIdx(0), CardIdx(1)]);
    assert!(st.exile.is_empty());
}

/// Budget completion uses the next route's full payment, not merely a remaining hand entry.
#[test]
fn hand_budget_checks_face_down_and_alternative_payment_after_the_last_cast() {
    let limit = crate::deck::simulator::model::MAX_LOOP_PASSES;
    for route in ["face_down", "alternative"] {
        for payable in [false, true] {
            let mut rows: Vec<_> = (0..limit)
                .map(|index| row(&format!("Artifact {index}"), "{0}", "Artifact", ""))
                .collect();
            rows.push(if route == "face_down" {
                row("Pending", "{100}", "Creature", "Morph {100}")
            } else {
                row("Pending", "{100}", "Sorcery", "You may exile one blue card from your hand rather than pay this spell's mana cost.\nGain 1 life.")
            });
            rows.push(row("Blue payment", "{100}{U}", "Sorcery", ""));
            let mut cards = deck(&rows);
            cards.cards[limit as usize + 1].colors[1] = true;
            let mut st = state((0..=limit as usize).collect(), vec![]);
            st.current_turn = 1;
            st.current_phase = game::GamePhase::Main;
            if route == "alternative" && payable {
                st.hand.push(CardIdx(limit + 1));
            }
            let original_hand_len = st.hand.len();
            let mut pool = ManaPool {
                colorless: if route == "face_down" {
                    if payable { 3 } else { 2 }
                } else {
                    0
                },
                ..Default::default()
            };
            let original_pool = pool.total();
            cast_pass(
                &cards,
                &mut st,
                &mut pool,
                1,
                &mut [0.0],
                &mut vec![],
                &mut vec![],
                &mut [false; 5],
            );
            assert_eq!(st.spells_cast_this_turn, limit);
            assert_eq!(st.battlefield.len(), limit as usize);
            assert_eq!(st.hand.len(), original_hand_len - limit as usize);
            assert!(st.hand.contains(&CardIdx(limit)));
            assert!(st.exile.is_empty());
            assert_eq!(pool.total(), original_pool);
            assert_eq!(st.completion.is_some(), payable, "{route}");
            if payable {
                assert!(matches!(
                    st.completion,
                    Some(game::GameCompletion::Incomplete {
                        reason: game::IncompleteReason::CastPass {
                            remaining_work: 1,
                            ..
                        },
                        ..
                    })
                ));
            }
        }
    }
}

/// Replay work requires complete mana, life, target, and Escape payments.
#[test]
fn graveyard_work_probes_require_all_costs_and_preserve_random_state() {
    let cases = [
        ("Gain 1 life.\nFlashback {R}", 20, false),
        (
            "As an additional cost to cast this spell, pay 3 life.\nGain 1 life.\nFlashback {B/P}",
            4,
            false,
        ),
        (
            "Return target creature card from your graveyard to the battlefield.\nFlashback {1}",
            20,
            false,
        ),
        ("Gain 1 life.\nFlashback {1}", 20, true),
        (
            "Gain 1 life.\nEscape—{1}, Exile three other cards from your graveyard.",
            20,
            false,
        ),
    ];
    for (text, life, payable) in cases {
        let cards = deck(&[row("Replay", "{100}", "Sorcery", text)]);
        let mut st = state(vec![], vec![]);
        st.graveyard.push(CardIdx(0));
        st.life = life;
        let pool = ManaPool {
            colorless: 1,
            ..Default::default()
        };
        let mut expected_rng = st.library_rng.clone();
        assert_eq!(
            work::has_graveyard_cast(&cards, &st, &pool),
            payable,
            "{text}"
        );
        assert_eq!(st.graveyard, [CardIdx(0)]);
        assert_eq!(st.life, life);
        assert_eq!(pool.total(), 1);
        assert!(st.exile.is_empty());
        assert_eq!(st.replay_casts, 0);
        assert_eq!(st.library_rng.clone().next_u64(), expected_rng.next_u64());
    }
    let cards = deck(&[row(
        "Restricted replay",
        "{1}",
        "Sorcery",
        "Gain 1 life.\nFlashback {1}",
    )]);
    let mut st = state(vec![], vec![]);
    st.graveyard.push(CardIdx(0));
    let pool = crate::deck::simulator::turn_loop_tests::restricted_pool(
        1,
        crate::deck::simulator::model::SpendRestriction::Creature,
    );
    assert!(!work::has_graveyard_cast(&cards, &st, &pool));
}

/// Optional teamwork uses the same current-power total to validate and commit its taps.
#[test]
fn teamwork_gate_and_payment_include_counters_and_equipment() {
    for minus in [1, 2] {
        let mut body = row("Body", "{1}", "Creature", "");
        body.power = Some("1".into());
        let cards = deck(&[
            row("Team spell", "{0}", "Sorcery", "Teamwork 4\nGain 1 life."),
            body,
            row(
                "Gear",
                "{1}",
                "Artifact — Equipment",
                "Equipped creature gets +2/+0.\nEquip {1}",
            ),
        ]);
        assert_eq!(cards.cards[0].keyword_abilities.teamwork, Some(4));
        let mut st = state(vec![0], vec![]);
        let mut creature = game::new_perm_with(1, &cards, CardIdx(1), 0, false);
        creature.counters.plus1 = 2;
        creature.counters.minus1 = minus;
        let mut gear = game::new_perm_with(2, &cards, CardIdx(2), 0, false);
        gear.equipped = true;
        gear.equip_host = Some(1);
        st.battlefield = vec![creature, gear];
        assert_eq!(
            try_cast_from_hand(
                &cards,
                &mut st,
                &mut ManaPool::default(),
                1,
                CardIdx(0),
                &mut vec![],
                &mut vec![],
                &mut [false; 5]
            ),
            Some(0)
        );
        assert_eq!(st.battlefield[0].tapped, minus == 1);
        assert!(!st.battlefield[1].tapped);
        assert_eq!(st.life_gained, 1);
        assert_eq!(st.graveyard, [CardIdx(0)]);
    }
}

/// Life-paid spell telemetry counts actual draws, including partial resolving draws after loss.
#[test]
fn life_funded_spell_draws_do_not_count_unresolved_or_failed_draws() {
    for (life, library, expected_draws) in [(1, vec![1, 2], 0), (2, vec![1], 1), (2, vec![1, 2], 2)]
    {
        let cards = deck(&[
            row(
                "Life-funded draw",
                "{0}",
                "Sorcery",
                "As an additional cost to cast this spell, pay 1 life.\nDraw two cards.",
            ),
            row("First card", "{100}", "Sorcery", ""),
            row("Second card", "{100}", "Sorcery", ""),
        ]);
        let mut st = state(vec![0], library);
        st.life = life;
        st.current_turn = 1;
        st.current_phase = game::GamePhase::Main;
        assert_eq!(
            try_cast_from_hand(
                &cards,
                &mut st,
                &mut ManaPool::default(),
                1,
                CardIdx(0),
                &mut vec![],
                &mut vec![],
                &mut [false; 5]
            ),
            Some(0)
        );
        assert_eq!(st.life_paid, 1);
        assert_eq!(st.hand.len(), expected_draws as usize);
        assert_eq!(st.life_funded_draws, expected_draws);
        assert_eq!(
            st.milestones_by_turn
                .get(&1)
                .map_or(0, |row| row.life_funded_draws),
            expected_draws
        );
        assert_eq!(st.completion.is_some(), expected_draws < 2);
    }
}
