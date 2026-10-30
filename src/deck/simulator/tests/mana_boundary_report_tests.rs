//! Mana boundary actions and executed reports. Synthetic cases prove engine rules only.

use super::card_test_support::{assert_library_rng, main_phase_state, named_row, pinned_revision};
use super::game::{GameLog, GameState, ManaPool};
use super::game_run::mana::{activate_mana_mode, build_pool};
use super::library_core_report_tests::{
    assert_report_clause, cast_action, executed_report, put_permanents,
};
use super::model::{CardIdx, SimDeck, SpendRestriction};
use super::oracle_parser::cost::parse_cost;
use super::scenario_test_support::{deck, row, state};
use crate::db::CardRow;
use serde_json::Value;

/// Keep synthetic search evidence separate from pinned source certificates.
#[path = "mana_search_boundary_report_tests.rs"]
mod search_reports;

/// Keep untap-symbol and unknown-cost actions in one small test file.
#[path = "mana_activation_boundary_report_tests.rs"]
mod activation_reports;

/// Keep work-limit and loop diagnostics separate from source-card evidence.
#[path = "mana_work_boundary_report_tests.rs"]
mod work_reports;

/// Check public mana observations against the actual completed turn census.
fn assert_mana_report(log: &GameLog, json: &Value, available: &[f64], spent: &[f64]) {
    assert_eq!(log.mana_available, available);
    assert_eq!(log.mana_spent, spent);
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["player_loss_games"], 0);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    for (turn, (&available, &spent)) in available.iter().zip(spent).enumerate() {
        let key = (turn + 1).to_string();
        assert_eq!(json["mana"]["avg_available_by_turn"][&key], available);
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"][&key], spent);
        assert_eq!(
            json["mana"]["avg_unused_by_turn"][&key],
            log.mana_remaining[turn]
        );
        assert_eq!(json["phase_observations"]["main_by_turn"][&key], 1.0);
        assert_eq!(json["phase_observations"]["end_turn_by_turn"][&key], 1.0);
        assert!(log.turn_observations[turn].main);
        assert!(log.turn_observations[turn].end_turn);
    }
}

/// Rejected actions must preserve physical resources and the library random stream.
fn assert_resources_unchanged(game: &GameState, before: &GameState) {
    assert_eq!(game.hand, before.hand);
    assert_eq!(game.library, before.library);
    assert_eq!(game.graveyard, before.graveyard);
    assert_eq!(game.exile, before.exile);
    assert_eq!(
        format!("{:?}", game.battlefield),
        format!("{:?}", before.battlefield)
    );
    assert_eq!(game.life, before.life);
    assert_eq!(game.life_paid, before.life_paid);
    assert_eq!(game.life_gained, before.life_gained);
    assert_eq!(game.player_counters, before.player_counters);
    assert_eq!(game.treasure_bank, before.treasure_bank);
    assert_eq!(game.seen, before.seen);
    assert_eq!(game.next_uid, before.next_uid);
    assert_eq!(game.activated_this_turn, before.activated_this_turn);
    assert_eq!(game.activated_once, before.activated_once);
    assert_eq!(game.triggered_this_turn, before.triggered_this_turn);
    assert_eq!(game.completion, before.completion);
    assert_library_rng(&game.library_rng, &before.library_rng);
}

/// Build a pinned Lantern with two physical lands and an unrelated colorless rock.
fn static_grant_scenario(cost: &str, granted: bool) -> (SimDeck, GameState) {
    let lantern = named_row("Chromatic Lantern");
    pinned_revision(&lantern);
    let cards = deck(&[
        lantern,
        row("Grant payment", cost, "Sorcery", ""),
        row("Plains", "", "Basic Land - Plains", "{T}: Add {W}."),
        row(
            "Colorless rock control",
            "{99}",
            "Artifact",
            "{T}: Add {C}.",
        ),
    ]);
    let mut game = main_phase_state(vec![1], vec![]);
    put_permanents(&cards, &mut game, &[2, 2, 3]);
    if granted {
        put_permanents(&cards, &mut game, &[0]);
        game.battlefield[3].tapped = true;
    }
    (cards, game)
}

/// The pilot chooses each land's granted ability without adding another tap.
pub(super) fn assert_static_grants_report() {
    for granted in [false, true] {
        for cost in ["{U}{U}{1}", "{U}{U}{U}", "{C}{C}", "{4}"] {
            let (cards, mut game) = static_grant_scenario(cost, granted);
            let rng = game.library_rng.clone();
            let identities: Vec<_> = game.battlefield.iter().map(|card| card.uid).collect();
            let mut pool = build_pool(&cards, &mut game, 1);
            assert_eq!(pool.total(), 3);
            assert_eq!(pool.flexible, if granted { 2 } else { 0 });
            assert_eq!(pool.fixed[0], if granted { 0 } else { 2 });
            assert_eq!(pool.colorless, 1);
            assert!(game.battlefield.iter().all(|source| source.tapped));
            let before = game.clone();
            assert_eq!(build_pool(&cards, &mut game, 1).total(), 0);
            assert_resources_unchanged(&game, &before);
            let legal = granted && cost == "{U}{U}{1}";
            assert_eq!(
                cast_action(&cards, &mut game, &mut pool),
                if legal { 3.0 } else { 0.0 }
            );
            assert_eq!(pool.total(), if legal { 0 } else { 3 });
            assert_eq!(
                game.graveyard,
                if legal { vec![CardIdx(1)] } else { vec![] }
            );
            assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(1)] });
            assert_eq!(
                game.battlefield
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                identities
            );
            assert_library_rng(&game.library_rng, &rng);
        }
    }
    for (granted, cost, legal) in [
        (true, "{U}{U}{U}{1}", true),
        (false, "{U}{U}{U}{1}", false),
        (true, "{U}{U}{U}{U}{1}", false),
        (true, "{C}{C}", false),
    ] {
        let (cards, game) = static_grant_scenario(cost, granted);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        let available = if granted { 4.0 } else { 3.0 };
        assert_mana_report(&log, &json, &[available], &[if legal { 4.0 } else { 0.0 }]);
        assert_eq!(log.mana_remaining, [if legal { 0.0 } else { available }]);
        assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(1)] });
        assert_eq!(
            game.graveyard,
            if legal { vec![CardIdx(1)] } else { vec![] }
        );
        assert!(game.battlefield.iter().all(|source| source.tapped));
        assert_library_rng(&game.library_rng, &rng);
        assert_report_clause(
            &json,
            &named_row("Chromatic Lantern"),
            "Lands you control",
            "unsupported",
            None,
        );
    }
}

/// Keep native wording fixed while varying the grant and two physical payment copies.
fn restricted_grant_scenario(kind: &str, cost: &str, granted: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic restricted land",
            "",
            "Land",
            "{T}: Add {G}. Spend this mana only to cast creature spells.",
        ),
        row(
            "Synthetic land grant",
            "{99}",
            "Enchantment",
            "Lands you control have \"{T}: Add one mana of any color.\"",
        ),
        row("Restricted grant payment", cost, kind, ""),
        row("Synthetic eligible creature demand", "{99}", "Creature", ""),
    ]);
    let mut game = main_phase_state(if granted { vec![2, 2] } else { vec![2, 2, 3] }, vec![]);
    put_permanents(&cards, &mut game, &[0]);
    if granted {
        put_permanents(&cards, &mut game, &[1]);
    }
    (cards, game)
}

/// Native and granted abilities use different spend rules but share one physical tap.
fn assert_restricted_grant_action(kind: &str, cost: &str, granted: bool, legal: bool) {
    let (cards, mut game) = restricted_grant_scenario(kind, cost, granted);
    let native = cards.cards[0]
        .tap
        .as_ref()
        .expect("restricted native source");
    assert_eq!(native.restriction, Some(SpendRestriction::Creature));
    let source_uid = game.battlefield[0].uid;
    let mut pool = build_pool(&cards, &mut game, 1);
    assert_eq!(pool.total(), 1);
    assert_eq!(pool.flexible, u32::from(granted));
    if granted {
        assert!(pool.lots.is_empty());
    } else {
        assert_eq!(pool.lots.len(), 1);
        assert_eq!(pool.lots[0].amount, 1);
        assert_eq!(pool.lots[0].types, 16);
        assert_eq!(pool.lots[0].restriction, Some(SpendRestriction::Creature));
    }
    let before = game.clone();
    let before_pool = pool.clone();
    assert_eq!(
        cast_action(&cards, &mut game, &mut pool),
        if legal { 1.0 } else { 0.0 }
    );
    if !legal {
        assert_resources_unchanged(&game, &before);
        assert_eq!(pool, before_pool);
    }
    assert_restricted_grant_state(&game, kind, granted, legal);
    assert_eq!(pool.total(), u32::from(!legal));
    assert_eq!(game.battlefield[0].uid, source_uid);
    let before = game.clone();
    let before_pool = pool.clone();
    let source = game.battlefield[0].clone();
    assert!(!activate_mana_mode(
        &cards, &mut game, &source, &mut pool, 1
    ));
    assert_eq!(build_pool(&cards, &mut game, 1).total(), 0);
    assert_eq!(cast_action(&cards, &mut game, &mut pool), 0.0);
    assert_resources_unchanged(&game, &before);
    assert_eq!(pool, before_pool);
}

/// Require exact physical copies and source state after the one available payment.
fn assert_restricted_grant_state(game: &GameState, kind: &str, granted: bool, legal: bool) {
    assert_eq!(
        game.hand,
        if legal {
            if granted {
                vec![CardIdx(2)]
            } else {
                vec![CardIdx(2), CardIdx(3)]
            }
        } else {
            if granted {
                vec![CardIdx(2), CardIdx(2)]
            } else {
                vec![CardIdx(2), CardIdx(2), CardIdx(3)]
            }
        }
    );
    let creature_cast = legal && kind == "Creature";
    let expected_cards = [
        Some(CardIdx(0)),
        granted.then_some(CardIdx(1)),
        creature_cast.then_some(CardIdx(2)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    assert_eq!(
        game.battlefield
            .iter()
            .map(|source| source.card.deck_idx().expect("physical card"))
            .collect::<Vec<_>>(),
        expected_cards
    );
    assert_eq!(
        game.graveyard,
        if legal && kind == "Sorcery" {
            vec![CardIdx(2)]
        } else {
            vec![]
        }
    );
    assert!(game.library.is_empty() && game.exile.is_empty());
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.battlefield[0].uid, 1);
    assert_eq!(
        game.next_uid,
        1 + u32::from(granted) + u32::from(creature_cast)
    );
    assert_eq!(game.life, 20);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.spells_cast_this_turn, u32::from(legal));
}

/// Reports measure the chosen granted ability, not inherited native restrictions.
fn assert_restricted_grant_report(kind: &str, cost: &str, granted: bool, legal: bool) {
    let (cards, game) = restricted_grant_scenario(kind, cost, granted);
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_mana_report(&log, &json, &[1.0], &[if legal { 1.0 } else { 0.0 }]);
    assert_eq!(log.mana_remaining, [f64::from(u32::from(!legal))]);
    assert_restricted_grant_state(&game, kind, granted, legal);
    assert_library_rng(&game.library_rng, &rng);
}

/// The pilot chooses an unrestricted granted ability; native restrictions remain separate.
pub(super) fn assert_static_grant_restrictions_report() {
    for granted in [false, true] {
        for kind in ["Creature", "Sorcery"] {
            for cost in ["{G}", "{U}", "{1}", "{C}"] {
                let legal = if granted {
                    cost != "{C}"
                } else {
                    kind == "Creature" && matches!(cost, "{G}" | "{1}")
                };
                assert_restricted_grant_action(kind, cost, granted, legal);
                assert_restricted_grant_report(kind, cost, granted, legal);
            }
        }
    }
    activation_reports::assert_granted_admission_routes();
    assert_mdfc_grant_face_report();
}

/// Put a front-face grant on its spell face or play its actual land face.
fn mdfc_grant_scenario(land_face: bool, external_grant: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic mana grant front // land back",
            "{99} // ",
            "Creature // Land",
            "Creatures you control have \"{T}: Add one mana of any color.\"\n{T}, Pay 2 life: Add {G}{G}{G}.\n//\n{T}: Add {C}.",
        ),
        row(
            "Synthetic external MDFC land grant",
            "{99}",
            "Enchantment",
            "Lands you control have \"{T}: Add one mana of any color.\"",
        ),
        row("Synthetic MDFC face payment", "{U}", "Sorcery", ""),
        row("Synthetic front-grant recipient", "{99}", "Creature", ""),
    ]);
    assert!(cards.cards[0].is_mdfc_spell);
    assert!(cards.cards[0].flags.grant.is_some());
    let mut game = main_phase_state(vec![2, 2], vec![]);
    game.life = 1;
    for seat in &mut game.opponents.seats {
        seat.active = false;
    }
    put_permanents(&cards, &mut game, &[3]);
    game.battlefield[0].summoning_sick = false;
    if external_grant {
        put_permanents(&cards, &mut game, &[1]);
    }
    if land_face {
        game.hand.insert(0, CardIdx(0));
        assert!(super::cast_pass::play_land(&cards, &mut game, 1));
    } else {
        put_permanents(&cards, &mut game, &[0]);
        game.battlefield
            .last_mut()
            .expect("placed front face")
            .summoning_sick = false;
    }
    let source = game.battlefield.last().expect("physical MDFC remains");
    assert_eq!(source.land_face, land_face);
    assert_eq!(super::game::is_land_permanent(source), land_face);
    assert_eq!(
        super::game::is_creature_permanent(&cards, source),
        !land_face
    );
    (cards, game)
}

/// A played land can receive a grant without exposing its front's costs or static grant.
fn assert_mdfc_grant_face_report() {
    for land_face in [false, true] {
        for external_grant in [false, true] {
            assert_mdfc_grant_face_action(land_face, external_grant);
            let (cards, game) = mdfc_grant_scenario(land_face, external_grant);
            let source_uid = game.battlefield.last().expect("physical MDFC").uid;
            let rng = game.library_rng.clone();
            let grants = super::game_run::mana::grants_active(&cards, &game);
            assert_eq!(grants.creatures, !land_face);
            assert_eq!(grants.lands, external_grant);
            let mut initial = game.clone();
            let initial_pool = build_pool(&cards, &mut initial, 1);
            assert_eq!(
                initial_pool,
                ManaPool {
                    flexible: u32::from(!land_face || external_grant),
                    colorless: u32::from(land_face && !external_grant),
                    ..ManaPool::default()
                }
            );
            assert_eq!(initial.battlefield[0].tapped, !land_face);
            assert_eq!(
                initial.battlefield.last().expect("physical MDFC").tapped,
                land_face
            );
            assert_eq!(initial.life, 1);
            assert_eq!(initial.life_paid, 0);
            let (game, log, json) = executed_report(&cards, game, 1, vec![]);
            let casts = if land_face {
                u32::from(external_grant)
            } else {
                2
            };
            assert_mana_report(&log, &json, &[1.0], &[f64::from(casts)]);
            assert_eq!(
                log.mana_remaining,
                [f64::from(u32::from(land_face && !external_grant))]
            );
            assert_eq!(game.hand, vec![CardIdx(2); 2 - casts as usize]);
            assert_eq!(game.graveyard, vec![CardIdx(2); casts as usize]);
            let source = game.battlefield.last().expect("physical MDFC remains");
            assert_eq!(source.uid, source_uid);
            assert_eq!(source.land_face, land_face);
            assert!(source.tapped);
            assert_eq!(game.battlefield[0].tapped, !land_face);
            assert_eq!(game.life, 1);
            assert_eq!(game.life_paid, 0);
            assert_eq!(json["draw"]["avg_life_paid"], 0.0);
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// The current MDFC face chooses one output and cannot pay another tap from a stale copy.
fn assert_mdfc_grant_face_action(land_face: bool, external_grant: bool) {
    let (cards, mut game) = mdfc_grant_scenario(land_face, external_grant);
    let source = game.battlefield.last().expect("physical MDFC").clone();
    let rng = game.library_rng.clone();
    let mut pool = ManaPool::default();
    assert!(activate_mana_mode(&cards, &mut game, &source, &mut pool, 1));
    let granted = !land_face || external_grant;
    assert_eq!(
        pool,
        ManaPool {
            flexible: u32::from(granted),
            colorless: u32::from(!granted),
            ..ManaPool::default()
        }
    );
    assert_eq!(
        super::card_runtime_tests::attempt_index(&cards, &mut game, &mut pool, CardIdx(2)),
        granted.then_some(1)
    );
    assert_eq!(
        game.hand,
        if granted {
            vec![CardIdx(2)]
        } else {
            vec![CardIdx(2), CardIdx(2)]
        }
    );
    assert_eq!(
        game.graveyard,
        if granted { vec![CardIdx(2)] } else { vec![] }
    );
    let before = game.clone();
    let before_pool = pool.clone();
    assert!(!activate_mana_mode(
        &cards, &mut game, &source, &mut pool, 1
    ));
    assert_resources_unchanged(&game, &before);
    assert_eq!(pool, before_pool);
    assert_eq!(game.life, 1);
    assert_eq!(game.life_paid, 0);
    assert_library_rng(&game.library_rng, &rng);
}

/// Build one synthetic fixed alternative with an optional same-type mana bonus.
fn alternative_scenario(cost: &str, bonus: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic fixed alternative",
            "{99}",
            "Artifact",
            "{T}: Add {U} or {G}.",
        ),
        row("Alternative payment", cost, "Sorcery", ""),
        row(
            "Synthetic same-type bonus",
            "{99}",
            "Enchantment",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        ),
    ]);
    let mut game = main_phase_state(vec![1], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    if bonus {
        put_permanents(&cards, &mut game, &[2]);
    }
    (cards, game)
}

/// Fixed alternatives and their bonus cannot pay two different types from one tap.
pub(super) fn assert_alternative_types_report() {
    for bonus in [false, true] {
        for (cost, legal, chosen) in [
            ("{U}", true, 2),
            ("{G}", true, 16),
            ("{U}{U}", bonus, 2),
            ("{G}{G}", bonus, 16),
            ("{U}{G}", false, 2),
            ("{R}", false, 2),
            ("{C}", false, 2),
        ] {
            let (cards, mut game) = alternative_scenario(cost, bonus);
            let rng = game.library_rng.clone();
            let identities: Vec<_> = game.battlefield.iter().map(|source| source.uid).collect();
            let mut pool = build_pool(&cards, &mut game, 1);
            let output = if bonus { 2 } else { 1 };
            assert_eq!(pool.total(), output);
            assert!(game.battlefield[0].tapped);
            if bonus {
                assert_eq!(pool.lots.len(), 2);
                assert!(
                    pool.lots
                        .iter()
                        .all(|lot| lot.types == chosen && lot.amount == 1)
                );
            }
            let source = game.battlefield[0].clone();
            let before = pool.clone();
            assert!(!activate_mana_mode(
                &cards, &mut game, &source, &mut pool, 1
            ));
            assert_eq!(pool, before);
            let spent = if legal {
                f64::from(parse_cost(cost).total())
            } else {
                0.0
            };
            assert_eq!(cast_action(&cards, &mut game, &mut pool), spent);
            assert_eq!(pool.total() as f64, f64::from(output) - spent);
            assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(1)] });
            assert_eq!(
                game.graveyard,
                if legal { vec![CardIdx(1)] } else { vec![] }
            );
            assert!(game.library.is_empty() && game.exile.is_empty());
            assert_eq!(
                game.battlefield
                    .iter()
                    .map(|source| source.uid)
                    .collect::<Vec<_>>(),
                identities
            );
            assert_library_rng(&game.library_rng, &rng);

            let (cards, game) = alternative_scenario(cost, bonus);
            let (game, log, json) = executed_report(&cards, game, 1, vec![]);
            assert_mana_report(&log, &json, &[f64::from(output)], &[spent]);
            assert_eq!(log.mana_remaining, [f64::from(output) - spent]);
            assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(1)] });
            assert_eq!(
                game.graveyard,
                if legal { vec![CardIdx(1)] } else { vec![] }
            );
            assert!(game.library.is_empty() && game.exile.is_empty());
            assert_eq!(
                game.battlefield
                    .iter()
                    .map(|source| source.uid)
                    .collect::<Vec<_>>(),
                identities
            );
            assert!(game.battlefield[0].tapped);
            assert!(!game.infinite_mana_suspected);
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// Build pinned opponent-dependent mana without declaring any opponent land colors.
fn unknown_opponent_scenario(cost: &str, active: bool) -> (SimDeck, GameState) {
    let source = named_row("Fellwar Stone");
    pinned_revision(&source);
    let cards = deck(&[
        source,
        row("Unknown-color payment", cost, "Sorcery", ""),
        row(
            "Synthetic opponent mana bonus",
            "{99}",
            "Enchantment",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        ),
        row("Turn-two draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![1], vec![3]);
    game.opponents.config.land_colors = None;
    game.opponents.seats[0].active = active;
    game.opponents.seats[0].lands = u32::from(active);
    put_permanents(&cards, &mut game, &[0, 2]);
    (cards, game)
}

/// Unknown opponent mana and its bonus fund generic costs only, from turn two.
pub(super) fn assert_unknown_opponents_report() {
    assert_empty_opponent_colors_report();
    for active in [false, true] {
        for turn in [1, 2] {
            for cost in ["{2}", "{G}", "{C}", "{3}"] {
                let (cards, mut game) = unknown_opponent_scenario(cost, active);
                game.current_turn = turn;
                let before = game.clone();
                let mut pool = ManaPool::default();
                let source = game.battlefield[0].clone();
                let produces = active && turn == 2;
                assert_eq!(
                    activate_mana_mode(&cards, &mut game, &source, &mut pool, turn),
                    produces
                );
                if !produces {
                    assert_resources_unchanged(&game, &before);
                }
                let output = if produces { 2 } else { 0 };
                assert_eq!(pool.generic_credit, output);
                assert_eq!(pool.total(), output);
                assert_eq!(pool.colorless, 0);
                assert_eq!(pool.fixed, [0; 5]);
                assert_eq!(pool.flexible, 0);
                assert!(pool.lots.is_empty());
                let legal = produces && cost == "{2}";
                assert_eq!(
                    cast_action(&cards, &mut game, &mut pool),
                    if legal { 2.0 } else { 0.0 }
                );
                assert_eq!(game.graveyard.contains(&CardIdx(1)), legal);
                assert_eq!(pool.total(), if legal { 0 } else { output });
                assert_library_rng(&game.library_rng, &before.library_rng);
            }
        }
        for cost in ["{2}", "{G}", "{C}", "{3}"] {
            let (cards, game) = unknown_opponent_scenario(cost, active);
            let rng = game.library_rng.clone();
            let (game, log, json) = executed_report(&cards, game, 2, vec![]);
            let legal = active && cost == "{2}";
            let output = if active { 2.0 } else { 0.0 };
            assert_mana_report(
                &log,
                &json,
                &[0.0, output],
                &[0.0, if legal { 2.0 } else { 0.0 }],
            );
            assert_eq!(log.mana_remaining, [0.0, if legal { 0.0 } else { output }]);
            assert_eq!(game.graveyard.contains(&CardIdx(1)), legal);
            assert_eq!(
                game.hand,
                if legal {
                    vec![CardIdx(3)]
                } else {
                    vec![CardIdx(1), CardIdx(3)]
                }
            );
            assert!(game.library.is_empty());
            assert!(game.exile.is_empty());
            assert_eq!(game.battlefield.len(), 2);
            assert_eq!(game.battlefield[0].tapped, active);
            assert_library_rng(&game.library_rng, &rng);
            assert_report_clause(
                &json,
                &named_row("Fellwar Stone"),
                "opponent controls",
                "supported",
                None,
            );
        }
    }
}

/// An explicit empty declaration grants neither the native mana nor its bonus.
fn assert_empty_opponent_colors_report() {
    let (cards, mut game) = unknown_opponent_scenario("{2}", true);
    game.current_turn = 2;
    game.opponents.config.land_colors = Some(vec![String::new()]);
    let before = game.clone();
    let mut pool = ManaPool::default();
    let source = game.battlefield[0].clone();
    assert!(!activate_mana_mode(
        &cards, &mut game, &source, &mut pool, 2
    ));
    assert_resources_unchanged(&game, &before);
    assert_eq!(pool, ManaPool::default());
    let (game, log, json) = executed_report(&cards, game, 2, vec![]);
    assert_mana_report(&log, &json, &[0.0, 0.0], &[0.0, 0.0]);
    assert_eq!(log.mana_remaining, [0.0, 0.0]);
    assert_eq!(game.hand, [CardIdx(1), CardIdx(3)]);
    assert!(game.graveyard.is_empty() && game.library.is_empty() && game.exile.is_empty());
    assert!(!game.battlefield[0].tapped);
    assert_eq!(
        json["opponent_model"]["parameters"]["land_colors"],
        serde_json::json!([""])
    );
    assert_library_rng(&game.library_rng, &before.library_rng);
}

/// Expose synthetic sickness controls without certifying a named untap source.
pub(super) fn assert_synthetic_untap_sickness_report() {
    activation_reports::assert_synthetic_untap_sickness_report();
}

/// Expose complete unknown-cost rejection actions and executed report controls.
pub(super) fn assert_unknown_activation_cost_report() {
    activation_reports::assert_unknown_activation_cost_report();
}

/// Expose raw-color selection without certifying a synthetic tutor as a real card.
pub(super) fn assert_search_colors_report() {
    search_reports::assert_search_colors_report();
}

/// Expose supported front-face selectors and strict mana-value relations.
pub(super) fn assert_search_types_relations_report() {
    search_reports::assert_search_types_relations_report();
}

/// An actual main-work cap reports pending granted mana without inventing finished observations.
pub(super) fn assert_granted_main_work_report() {
    work_reports::assert_granted_main_work_report();
}

/// Actual granted and native loop actions keep diagnostic evidence separate from completion.
pub(super) fn assert_granted_positive_loop_report() {
    work_reports::assert_granted_positive_loop_report();
}

/// Synthetic granted work remains visible at the main-action boundary.
#[test]
fn synthetic_granted_main_work_boundaries_reach_reports() {
    assert_granted_main_work_report();
}

/// Synthetic loop diagnostics measure actual growth rather than completion or a proven win.
#[test]
fn synthetic_granted_positive_loop_diagnostics_reach_reports() {
    assert_granted_positive_loop_report();
}

/// A pinned land grant shares each physical source's single tap.
#[test]
fn pinned_static_grants_execute_and_report() {
    assert_static_grants_report();
}

/// The chosen granted ability has its own output and shares the native source's tap.
#[test]
fn synthetic_static_grant_restrictions_execute_and_report() {
    assert_static_grant_restrictions_report();
}

/// Fixed alternatives keep one type across their source and bonus output.
#[test]
fn synthetic_alternative_types_execute_and_report() {
    assert_alternative_types_report();
}

/// Pinned unknown-opponent production cannot supply a colored or colorless pip.
#[test]
fn pinned_unknown_opponents_execute_and_report() {
    assert_unknown_opponents_report();
}

/// New creatures cannot pay an untap-symbol activation without haste.
#[test]
fn synthetic_untap_sickness_executes_and_reports() {
    assert_synthetic_untap_sickness_report();
}

/// An unknown mandatory payment blocks all output and leaves real resources intact.
#[test]
fn synthetic_unknown_activation_cost_executes_and_reports() {
    assert_unknown_activation_cost_report();
}

/// Search reports count only raw stored-color hits.
#[test]
fn synthetic_search_colors_execute_and_report() {
    assert_search_colors_report();
}

/// Paid searches retain exact front-face types and supported mana-value bounds.
#[test]
fn synthetic_search_types_relations_execute_and_report() {
    assert_search_types_relations_report();
}
