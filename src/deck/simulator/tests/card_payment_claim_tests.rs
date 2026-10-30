//! Cast-payment evidence separates pinned cards from synthetic general-core rules.
//!
//! Twobrid, convoke, improvise, delve, restrictions, and paid untaps have no
//! selected or original-351 source for these clauses. Their fixtures below are
//! synthetic. Phyrexian payment follows the published life-only policy. These
//! helpers do not claim support for Metamorph's copy or Deathrite's abilities.

use super::card_cost_claim_tests::{
    ability_record, assert_limit, assert_mana_report, assert_unchanged, place, printed_activation,
    source_record,
};
use super::card_report_tests::{execute_turn, report_executed_turn};
use super::card_runtime_tests::{attempt_index, renamed};
use super::card_test_support::{assert_library_rng, main_phase_state as state, named_row};
use super::game::{GameState, ManaPool};
use super::game_effects::activation::{pick_best_activation, resolve_activation};
use super::game_run::mana::{activate_mana_mode, build_pool};
use super::model::{CardIdx, SimDeck};
use super::oracle_parser::cost::parse_cost;
use super::scenario_test_support::{deck, row};
use crate::db::CardRow;

/// Deathrite's hybrid cast accepts either printed color and no unrelated color.
pub(super) fn assert_hybrid_cast(source: &CardRow) {
    for source in renamed(source) {
        for (color, accepted) in [(2, true), (4, true), (1, false)] {
            let cards = deck(&[
                source.clone(),
                row("Unmoved graveyard", "{99}", "Sorcery", ""),
                row("Unmoved library", "{99}", "Sorcery", ""),
            ]);
            assert_eq!(cards[CardIdx(0)].cost, parse_cost("{B/G}"));
            assert!(cards[CardIdx(0)].unlocked_abilities(0).next().is_none());
            let mut game = state(vec![0], vec![2]);
            game.graveyard.push(CardIdx(1));
            let mut pool = ManaPool::default();
            pool.fixed[color] = 1;
            let before = game.clone();
            let before_pool = pool.clone();
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
                accepted.then_some(1)
            );
            if !accepted {
                assert_unchanged(&game, &before, &pool, &before_pool);
                continue;
            }
            assert!(game.hand.is_empty() && game.exile.is_empty());
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
            assert_eq!(game.library, [CardIdx(2)]);
            assert_eq!(game.graveyard, [CardIdx(1)]);
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_eq!(game.life, 20);
            assert_eq!(game.life_paid, 0);
            assert_eq!(game.opponent_life_lost, 0);
            assert_eq!(pool.total(), 0);
            assert_library_rng(&game.library_rng, &before.library_rng);
        }
    }
}

/// Executed hybrid casts leave all of Deathrite's graveyard abilities unsupported.
pub(super) fn assert_hybrid_cast_report(source: &CardRow) {
    for (mana, accepted) in [("{B}", true), ("{G}", true), ("{U}", false)] {
        let cards = deck(&[
            source.clone(),
            row(
                "Physical source",
                "{99}",
                "Artifact",
                &format!("{{T}}: Add {mana}."),
            ),
            row("Graveyard instant", "{99}", "Instant", ""),
            row("Undrawn card", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![0], vec![3]);
        game.graveyard.push(CardIdx(2));
        place(&cards, &mut game, 1, false);
        let rng = game.library_rng.clone();
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        assert_eq!(game.graveyard, [CardIdx(2)]);
        assert_eq!(game.library, [CardIdx(3)]);
        assert!(game.exile.is_empty());
        assert_eq!(game.life, 20);
        assert_eq!(game.opponent_life_lost, 0);
        assert_library_rng(&game.library_rng, &rng);
        let (log, json) = report_executed_turn(&cards, game, census);
        let spent = f64::from(u32::from(accepted));
        assert_mana_report(&log, &json, 1.0, spent, 1.0 - spent);
        let record = source_record(&json, source);
        let abilities = record["abilities"].as_array().expect("Deathrite clauses");
        assert_eq!(abilities.len(), 3);
        assert!(
            abilities
                .iter()
                .all(|ability| ability["status"] == "unsupported")
        );
        assert_limit(&json, "Hybrid symbols use their allowed colors");
    }
}

/// Metamorph pays three actual mana and two life, even with blue mana available.
pub(super) fn assert_phyrexian_cast(source: &CardRow) {
    for source in renamed(source) {
        for (generic, blue, life, accepted) in [
            (3, 0, 20, true),
            (3, 1, 20, true),
            (2, 0, 20, false),
            (3, 1, 1, false),
        ] {
            let mut target = row("Copy candidate", "{99}", "Artifact Creature", "");
            target.power = Some("7".into());
            target.toughness = Some("7".into());
            let cards = deck(&[
                source.clone(),
                target,
                row("Undrawn card", "{99}", "Sorcery", ""),
            ]);
            assert_eq!(cards[CardIdx(0)].cost, parse_cost("{3}{U/P}"));
            assert!(cards[CardIdx(0)].unlocked_abilities(0).next().is_none());
            let mut game = state(vec![0], vec![2]);
            game.life = life;
            place(&cards, &mut game, 1, false);
            let mut pool = ManaPool {
                colorless: generic,
                fixed: [0, blue, 0, 0, 0],
                ..Default::default()
            };
            let before = game.clone();
            let before_pool = pool.clone();
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
                accepted.then_some(3)
            );
            if !accepted {
                assert_unchanged(&game, &before, &pool, &before_pool);
                continue;
            }
            assert_eq!(pool.colorless, 0);
            assert_eq!(pool.fixed[1], blue);
            assert_eq!(game.life, 18);
            assert_eq!(game.life_paid, 2);
            assert_eq!(game.spells_cast_this_turn, 1);
            assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
            assert_eq!(game.library, [CardIdx(2)]);
            assert_eq!(game.battlefield.len(), 2);
            assert_eq!(game.battlefield[1].card.deck_idx(), Some(CardIdx(0)));
            assert_eq!(cards[CardIdx(0)].printed_power, Some(0));
            assert_eq!(cards[CardIdx(0)].printed_toughness, Some(0));
            assert_eq!(game.battlefield[1].counters.plus1, 0);
            assert_eq!(game.seen, 0);
            assert_library_rng(&game.library_rng, &before.library_rng);
        }
    }
}

/// The report exposes life-only payment and never upgrades the unsupported copy effect.
pub(super) fn assert_phyrexian_cast_report(source: &CardRow) {
    for life in [1, 20] {
        let cards = deck(&[
            source.clone(),
            row(
                "Physical source",
                "{99}",
                "Artifact",
                "{T}: Add {C}{C}{C}{U}.",
            ),
            row("Undrawn card", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![0], vec![2]);
        game.life = life;
        place(&cards, &mut game, 1, false);
        let census = execute_turn(&cards, &mut game);
        let accepted = life == 20;
        assert_eq!(game.life, if accepted { 18 } else { 1 });
        assert_eq!(game.life_paid, if accepted { 2 } else { 0 });
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        assert_eq!(game.battlefield.len(), if accepted { 2 } else { 1 });
        assert_eq!(game.library, [CardIdx(2)]);
        let (log, json) = report_executed_turn(&cards, game, census);
        let spent = if accepted { 3.0 } else { 0.0 };
        assert_mana_report(&log, &json, 4.0, spent, 4.0 - spent);
        assert_eq!(
            json["draw"]["avg_life_paid"],
            if accepted { 2.0 } else { 0.0 }
        );
        let record = source_record(&json, source);
        let copy = ability_record(
            record,
            "You may have this creature enter as a copy of any artifact or creature on the battlefield, except it's an artifact in addition to its other types.",
        );
        assert_eq!(copy["status"], "unsupported");
        assert_eq!(
            copy["reason"],
            "The Oracle statement has no executable grammar rule."
        );
        assert_limit(
            &json,
            "pays two life for each Phyrexian pip and does not switch to colored payment",
        );
    }
}

/// Synthetic twobrid accepts one matching color or two generic units, never one unrelated unit.
pub(super) fn assert_synthetic_twobrid() {
    let cards = deck(&[row("Synthetic twobrid", "{2/W}", "Sorcery", "Gain 1 life.")]);
    for (white, blue, generic, accepted, spent) in [
        (1, 0, 0, true, 1),
        (0, 1, 0, false, 0),
        (0, 0, 1, false, 0),
        (0, 0, 2, true, 2),
    ] {
        let mut game = state(vec![0], vec![]);
        let mut pool = ManaPool {
            fixed: [white, blue, 0, 0, 0],
            colorless: generic,
            ..Default::default()
        };
        let before = game.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
            accepted.then_some(spent)
        );
        if !accepted {
            assert_unchanged(&game, &before, &pool, &before_pool);
        }
        assert_eq!(pool.total(), before_pool.total() - spent);
        assert_eq!(game.life, if accepted { 21 } else { 20 });
        assert_eq!(game.life_gained, u32::from(accepted));
        assert_eq!(
            game.graveyard,
            if accepted { vec![CardIdx(0)] } else { vec![] }
        );
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        assert_library_rng(&game.library_rng, &before.library_rng);
    }
    for (mana, accepted, spent) in [
        ("{W}", true, 1.0),
        ("{U}", false, 0.0),
        ("{C}", false, 0.0),
        ("{C}{C}", true, 2.0),
    ] {
        let cards = deck(&[
            row("Synthetic twobrid", "{2/W}", "Sorcery", "Gain 1 life."),
            row(
                "Synthetic source",
                "{99}",
                "Artifact",
                &format!("{{T}}: Add {mana}."),
            ),
        ]);
        let mut game = state(vec![0], vec![]);
        place(&cards, &mut game, 1, false);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.life_gained, u32::from(accepted));
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        let (log, json) = report_executed_turn(&cards, game, census);
        let available = if mana == "{C}{C}" { 2.0 } else { 1.0 };
        assert_mana_report(&log, &json, available, spent, available - spent);
        assert_limit(
            &json,
            "Twobrid symbols use one matching colored mana or two generic mana",
        );
    }
}

/// Build one real blue artifact creature, not separate resources for each keyword.
fn tap_payment_scenario(cost: &str, keywords: &str, artifact: bool) -> (SimDeck, GameState) {
    let mut permanent = row(
        "Synthetic payment body",
        "{0}",
        if artifact {
            "Artifact Creature"
        } else {
            "Creature"
        },
        "",
    );
    permanent.colors = "[\"U\"]".into();
    permanent.power = Some("0".into());
    permanent.toughness = Some("1".into());
    let cards = deck(&[
        row(
            "Synthetic tap payment",
            cost,
            "Sorcery",
            &format!("{keywords}\nGain 1 life."),
        ),
        permanent,
        row(
            "Synthetic source",
            "",
            "Basic Land - Wastes",
            "{T}: Add {C}.",
        ),
        row(
            "Second synthetic tap payment",
            cost,
            "Sorcery",
            &format!("{keywords}\nGain 1 life."),
        ),
    ]);
    let mut game = state(vec![0], vec![]);
    place(&cards, &mut game, 1, false);
    (cards, game)
}

/// Commit one complete cast and ensure a tapped payment body cannot pay again.
fn assert_tap_payment_action(
    cost: &str,
    keywords: &str,
    artifact: bool,
    mana: u32,
    accepted: bool,
) {
    let (cards, mut game) = tap_payment_scenario(cost, keywords, artifact);
    game.hand.push(CardIdx(3));
    let mut pool = ManaPool {
        colorless: mana,
        ..Default::default()
    };
    let before = game.clone();
    let before_pool = pool.clone();
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        accepted.then_some(mana)
    );
    if !accepted {
        assert_unchanged(&game, &before, &pool, &before_pool);
        return;
    }
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].uid, before.battlefield[0].uid);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.life_gained, 1);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.hand, [CardIdx(3)]);
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_eq!(pool.total(), 0);
    let before = game.clone();
    let before_pool = pool.clone();
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(3)),
        None
    );
    assert_unchanged(&game, &before, &pool, &before_pool);
}

/// A body cast this turn can pay the cost but cannot gain a later combat tap.
fn assert_tap_payment_report(
    cost: &str,
    keywords: &str,
    artifact: bool,
    funded: bool,
    accepted: bool,
) {
    let (cards, _) = tap_payment_scenario(cost, keywords, artifact);
    let mut game = state(vec![1, 0], vec![]);
    if funded {
        place(&cards, &mut game, 2, false);
    }
    let rng = game.library_rng.clone();
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.spells_cast_this_turn, 1 + u32::from(accepted));
    assert_eq!(game.life_gained, u32::from(accepted));
    let body = game
        .battlefield
        .iter()
        .find(|permanent| permanent.card.deck_idx() == Some(CardIdx(1)))
        .expect("the payment body entered during the report turn");
    assert_eq!(body.entered_turn, 1);
    assert!(body.summoning_sick);
    assert_eq!(body.tapped, accepted);
    assert!(!game.attacked_this_turn);
    assert_eq!(game.battlefield.len(), 1 + usize::from(funded));
    assert_eq!(game.hand, if accepted { vec![] } else { vec![CardIdx(0)] });
    assert_eq!(
        game.graveyard,
        if accepted { vec![CardIdx(0)] } else { vec![] }
    );
    assert_library_rng(&game.library_rng, &rng);
    let (log, json) = report_executed_turn(&cards, game, census);
    assert_eq!(log.attackers, [0]);
    assert_eq!(log.creatures, [1]);
    let mana = f64::from(u32::from(funded));
    assert_mana_report(
        &log,
        &json,
        mana,
        if accepted { mana } else { 0.0 },
        if accepted { 0.0 } else { mana },
    );
    assert_limit(&json, "Convoke matches each available creature once");
    assert_limit(
        &json,
        "A shared artifact creature cannot pay twice through convoke and improvise",
    );
}

/// Synthetic convoke uses creature colors for colored symbols and taps each body once.
pub(super) fn assert_synthetic_convoke() {
    for (cost, accepted) in [
        ("{U}", true),
        ("{1}", true),
        ("{W/U}", true),
        ("{B}", false),
        ("{C}", false),
        ("{U}{U}", false),
    ] {
        assert_tap_payment_action(cost, "Convoke", false, 0, accepted);
    }
    let (cards, mut game) = tap_payment_scenario("{U}", "Convoke", false);
    game.battlefield[0].tapped = true;
    let before = game.clone();
    let mut pool = ManaPool::default();
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        None
    );
    assert_unchanged(&game, &before, &pool, &ManaPool::default());
    assert_tap_payment_report("{U}", "Convoke", false, false, true);
    assert_tap_payment_report("{B}", "Convoke", false, false, false);
}

/// Synthetic improvise pays generic cost only and requires a real untapped artifact.
pub(super) fn assert_synthetic_improvise() {
    for (cost, artifact, accepted) in [
        ("{1}", true, true),
        ("{1}", false, false),
        ("{U}", true, false),
        ("{C}", true, false),
    ] {
        assert_tap_payment_action(cost, "Improvise", artifact, 0, accepted);
    }
    let (cards, mut game) = tap_payment_scenario("{1}", "Improvise", true);
    game.battlefield[0].tapped = true;
    let before = game.clone();
    let mut pool = ManaPool::default();
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        None
    );
    assert_unchanged(&game, &before, &pool, &ManaPool::default());
    assert_tap_payment_report("{1}", "Improvise", true, false, true);
    assert_tap_payment_report("{U}", "Improvise", true, false, false);
}

/// Synthetic combined payment cannot count the same artifact creature twice.
pub(super) fn assert_synthetic_shared_tap() {
    for funded in [false, true] {
        assert_tap_payment_action(
            "{1}{U}",
            "Convoke\nImprovise",
            true,
            u32::from(funded),
            funded,
        );
        assert_tap_payment_report("{1}{U}", "Convoke\nImprovise", true, funded, funded);
    }
}

/// Build distinct graveyard instances for a two-generic, one-blue delve spell.
fn delve_scenario(mana: &str) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic delve",
            "{2}{U}",
            "Sorcery",
            "Delve\nGain 1 life.",
        ),
        row(
            "Synthetic source",
            "{99}",
            "Artifact",
            &format!("{{T}}: Add {mana}."),
        ),
        row("Retained graveyard", "{99}", "Sorcery", ""),
        row("Second exile", "{99}", "Sorcery", ""),
        row("First exile", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![]);
    game.graveyard.extend([CardIdx(2), CardIdx(3), CardIdx(4)]);
    place(&cards, &mut game, 1, false);
    (cards, game)
}

/// Synthetic delve consumes only two physical instances and cannot replace the blue pip.
pub(super) fn assert_synthetic_delve() {
    for (mana, accepted) in [("{U}", true), ("{C}", false)] {
        let (cards, mut game) = delve_scenario(mana);
        let mut pool = build_pool(&cards, &mut game, 1);
        let before = game.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
            accepted.then_some(1)
        );
        if accepted {
            assert_eq!(game.exile, [CardIdx(4), CardIdx(3)]);
            assert_eq!(game.graveyard, [CardIdx(2), CardIdx(0)]);
            assert!(game.hand.is_empty());
            assert_eq!(pool.total(), 0);
            assert_eq!(game.life_gained, 1);
        } else {
            assert_unchanged(&game, &before, &pool, &before_pool);
        }
        assert_library_rng(&game.library_rng, &before.library_rng);
        let (cards, mut game) = delve_scenario(mana);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.exile,
            if accepted {
                vec![CardIdx(4), CardIdx(3)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.graveyard,
            if accepted {
                vec![CardIdx(2), CardIdx(0)]
            } else {
                vec![CardIdx(2), CardIdx(3), CardIdx(4)]
            }
        );
        assert_eq!(game.spells_cast_this_turn, u32::from(accepted));
        let (log, json) = report_executed_turn(&cards, game, census);
        let spent = f64::from(u32::from(accepted));
        assert_mana_report(&log, &json, 1.0, spent, 1.0 - spent);
        assert_eq!(log.graveyard_size, [if accepted { 2 } else { 3 }]);
        assert_limit(&json, "Delve exiles graveyard cards for generic costs");
    }
    for generic in [0, 1] {
        let (cards, mut game) = delve_scenario("{U}");
        game.graveyard = vec![CardIdx(4)];
        let mut pool = ManaPool {
            fixed: [0, 1, 0, 0, 0],
            colorless: generic,
            ..Default::default()
        };
        let before = game.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
            (generic == 1).then_some(2)
        );
        if generic == 0 {
            assert_unchanged(&game, &before, &pool, &before_pool);
        } else {
            assert_eq!(game.exile, [CardIdx(4)]);
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(pool.total(), 0);
        }
    }
}

/// Synthetic snow and unknown symbols reject complete casts without a free reward.
pub(super) fn assert_synthetic_unpaid_symbols() {
    for cost in ["{S}", "{Y}"] {
        let cards = deck(&[
            row("Synthetic unpaid cost", cost, "Sorcery", "Gain 3 life."),
            row(
                "Synthetic source",
                "{99}",
                "Artifact",
                "{T}: Add {C}{C}{U}.",
            ),
        ]);
        let mut game = state(vec![0], vec![]);
        place(&cards, &mut game, 1, false);
        let mut pool = ManaPool {
            flexible: 5,
            colorless: 5,
            ..Default::default()
        };
        let before = game.clone();
        let before_pool = pool.clone();
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
            None
        );
        assert_unchanged(&game, &before, &pool, &before_pool);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.hand, [CardIdx(0)]);
        assert_eq!(game.life_gained, 0);
        assert!(game.graveyard.is_empty());
        assert_eq!(game.spells_cast_this_turn, 0);
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_mana_report(&log, &json, 3.0, 0.0, 3.0);
        assert_limit(
            &json,
            "Snow costs and unsupported mana symbols cannot be paid",
        );
    }
}

/// Build one restricted source with legal and illegal classes still in hand.
fn restriction_scenario(artifact_only: bool) -> (SimDeck, GameState) {
    let class = if artifact_only {
        "artifact"
    } else {
        "creature"
    };
    let cards = deck(&[
        row(
            "Synthetic restricted source",
            "{99}",
            "Artifact",
            &format!("{{T}}: Add {{G}}{{G}}. Spend this mana only to cast {class} spells."),
        ),
        row("Illegal spell", "{G}", "Sorcery", "Gain 3 life."),
        row(
            "Legal spell",
            "{G}",
            if artifact_only {
                "Artifact"
            } else {
                "Creature"
            },
            "",
        ),
        row("Legal artifact creature", "{G}", "Artifact Creature", ""),
        row(
            "Other class",
            "{G}",
            if artifact_only {
                "Creature"
            } else {
                "Artifact"
            },
            "",
        ),
    ]);
    let mut game = state(vec![1, 2, 3, 4], vec![]);
    place(&cards, &mut game, 0, false);
    (cards, game)
}

/// Synthetic class-restricted mana cannot fund another class or a nonspell payment.
pub(super) fn assert_synthetic_restrictions() {
    for artifact_only in [false, true] {
        let (cards, mut game) = restriction_scenario(artifact_only);
        let mut pool = build_pool(&cards, &mut game, 1);
        assert_eq!(pool.total(), 2);
        assert!(game.battlefield[0].tapped);
        for index in [1, 4] {
            let before = game.clone();
            let before_pool = pool.clone();
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(index)),
                None
            );
            assert_unchanged(&game, &before, &pool, &before_pool);
        }
        for index in [2, 3] {
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(index)),
                Some(1)
            );
        }
        assert_eq!(pool.total(), 0);
        assert_eq!(game.hand, [CardIdx(1), CardIdx(4)]);
        assert_eq!(game.battlefield.len(), 3);
        assert_eq!(game.spells_cast_this_turn, 2);
        assert_eq!(game.life, 20);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        let (cards, mut game) = restriction_scenario(artifact_only);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.hand, [CardIdx(1), CardIdx(4)]);
        assert_eq!(game.spells_cast_this_turn, 2);
        assert_eq!(game.life, 20);
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_mana_report(&log, &json, 2.0, 2.0, 0.0);
        assert_limit(
            &json,
            "Creature and artifact restrictions can both pay an artifact creature",
        );
    }
    let (mut cards, mut game) = restriction_scenario(false);
    let mut pool = build_pool(&cards, &mut game, 1);
    let sink = super::oracle_lower::parse_sim_card(&row(
        "Synthetic sink",
        "{99}",
        "Artifact",
        "{1}, {T}: Draw a card.",
    ));
    cards.cards.push(sink);
    place(&cards, &mut game, 5, false);
    assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
    let sink = printed_activation(&cards, &game, 1, 0);
    let before = game.clone();
    let before_pool = pool.clone();
    resolve_activation(&cards, &mut game, &mut pool, 1, &sink);
    assert_unchanged(&game, &before, &pool, &before_pool);

    let (cards, mut game) = restriction_scenario(false);
    game.hand = vec![CardIdx(1), CardIdx(4)];
    let mut pool = ManaPool::default();
    let before = game.clone();
    let before_pool = pool.clone();
    let source = game.battlefield[0].clone();
    assert!(!activate_mana_mode(
        &cards, &mut game, &source, &mut pool, 1
    ));
    assert_unchanged(&game, &before, &pool, &before_pool);
}

/// Build a finite synthetic untap source whose second tap cannot fund another untap.
fn untap_scenario(funded: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic untap source",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}.\n{3}: Untap this artifact.",
        ),
        row("External payment", "{99}", "Artifact", "{T}: Add {C}."),
    ]);
    let mut game = state(vec![], vec![]);
    place(&cards, &mut game, 0, false);
    if funded {
        place(&cards, &mut game, 1, false);
    }
    (cards, game)
}

/// Synthetic paid untap permits one source reuse, but failure leaves its tap intact.
pub(super) fn assert_synthetic_paid_untap() {
    for funded in [false, true] {
        let (cards, mut game) = untap_scenario(funded);
        let mut pool = build_pool(&cards, &mut game, 1);
        assert_eq!(pool.total(), if funded { 3 } else { 2 });
        assert!(game.battlefield[0].tapped);
        let untap = printed_activation(&cards, &game, 0, 1);
        let before = game.clone();
        let before_pool = pool.clone();
        resolve_activation(&cards, &mut game, &mut pool, 1, &untap);
        if !funded {
            assert_unchanged(&game, &before, &pool, &before_pool);
        } else {
            assert_eq!(pool.total(), 0);
            assert!(!game.battlefield[0].tapped);
            let uid = game.battlefield[0].uid;
            let source = game.battlefield[0].clone();
            assert!(activate_mana_mode(&cards, &mut game, &source, &mut pool, 1));
            assert_eq!(game.battlefield[0].uid, uid);
            assert_eq!(pool.colorless, 2);
            assert!(game.battlefield[0].tapped);
            let before = game.clone();
            let before_pool = pool.clone();
            resolve_activation(&cards, &mut game, &mut pool, 1, &untap);
            assert_unchanged(&game, &before, &pool, &before_pool);
        }
        let (cards, mut game) = untap_scenario(funded);
        let rng = game.library_rng.clone();
        let census = execute_turn(&cards, &mut game);
        assert!(game.battlefield[0].tapped);
        assert_eq!(game.battlefield.len(), if funded { 2 } else { 1 });
        assert!(!game.infinite_mana_suspected);
        assert_eq!(game.spells_cast_this_turn, 0);
        assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
        assert_library_rng(&game.library_rng, &rng);
        let (log, json) = report_executed_turn(&cards, game, census);
        assert_mana_report(&log, &json, if funded { 3.0 } else { 2.0 }, 0.0, 2.0);
        assert_limit(&json, "Supported untap effects can make it available again");
    }
}

/// Pin hybrid cast payment without certifying Deathrite's graveyard abilities.
#[test]
fn pinned_deathrite_cast_uses_black_or_green_but_not_an_unrelated_color() {
    let source = named_row("Deathrite Shaman");
    assert_hybrid_cast(&source);
    assert_hybrid_cast_report(&source);
}

/// Pin the life-only approximation without certifying Metamorph's copy effect.
#[test]
fn pinned_metamorph_cast_pays_three_mana_and_two_life_without_copying() {
    let source = named_row("Phyrexian Metamorph");
    assert_phyrexian_cast(&source);
    assert_phyrexian_cast_report(&source);
}

/// General-core twobrid evidence is synthetic, not pinned source integration.
#[test]
fn synthetic_twobrid_action_and_turn_reports_keep_both_legal_payment_sizes() {
    assert_synthetic_twobrid();
}

/// General-core convoke evidence uses one real matching payment body.
#[test]
fn synthetic_convoke_action_and_turn_reports_keep_color_and_tap_limits() {
    assert_synthetic_convoke();
}

/// General-core improvise evidence does not turn artifacts into colored mana.
#[test]
fn synthetic_improvise_action_and_turn_reports_pay_only_generic_costs() {
    assert_synthetic_improvise();
}

/// General-core combined payment cannot count one permanent twice.
#[test]
fn synthetic_convoke_and_improvise_share_the_same_physical_tap() {
    assert_synthetic_shared_tap();
}

/// General-core delve evidence checks physical exile and a separate blue payment.
#[test]
fn synthetic_delve_action_and_turn_reports_exile_only_enough_instances() {
    assert_synthetic_delve();
}

/// Unsupported cost symbols cannot grant a free spell in an executed turn.
#[test]
fn synthetic_snow_and_unknown_costs_stay_unpaid_in_actions_and_reports() {
    assert_synthetic_unpaid_symbols();
}

/// General-core mana restrictions survive the full turn's cast choices.
#[test]
fn synthetic_restricted_mana_cannot_escape_its_cast_class() {
    assert_synthetic_restrictions();
}

/// General-core untap evidence pays the full activation before reusing a source.
#[test]
fn synthetic_paid_untap_reuses_one_source_but_unpaid_untap_cannot() {
    assert_synthetic_paid_untap();
}
