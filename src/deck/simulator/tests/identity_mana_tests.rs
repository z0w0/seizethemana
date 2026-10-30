//! Commander-identity mana uses stored identities in payment, bonuses, and reports.

use crate::db::CardRow;
use crate::deck::simulator::aggregate::aggregate;
use crate::deck::simulator::card_test_support::{assert_library_rng, main_phase_state, named_row};
use crate::deck::simulator::cast_pass::try_cast_from_hand;
use crate::deck::simulator::game::{GameState, ManaPool, new_perm_with};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_effects::activation::{pick_best_activation, spend_leftover};
use crate::deck::simulator::game_run::mana::{activate_mana_mode, tap_for_nonspell_payment};
use crate::deck::simulator::game_run::{TurnCensus, census::finish_log, run_turn};
use crate::deck::simulator::model::{CardIdx, Format, SimDeck};
use crate::deck::simulator::oracle_lower::parse_sim_card;
use crate::deck::simulator::oracle_parser::land::parse_tap_yield;
use crate::deck::simulator::scenario_test_support::{deck, row};

/// Keep the commander's printed white cost distinct from its stored identity.
fn identity_deck(source: &CardRow, identities: &[&str], target_cost: &str) -> SimDeck {
    let mut cards = deck(&[
        source.clone(),
        row("Payment target", target_cost, "Sorcery", ""),
        row("Future draw", "{100}", "Sorcery", ""),
    ]);
    for (slot, identity) in identities.iter().enumerate() {
        let mut commander = row(
            &format!("Commander {slot}"),
            "{100}{W}",
            "Legendary Creature",
            "",
        );
        commander.colors = "[\"W\"]".into();
        commander.color_identity = serde_json::to_string(
            &identity
                .chars()
                .map(|color| color.to_string())
                .collect::<Vec<_>>(),
        )
        .expect("test identity serializes");
        cards.commanders.push(parse_sim_card(&commander));
    }
    cards.format = Format::Commander;
    cards.rules = crate::deck::simulator::format::rules_for("commander");
    cards
}

/// Place the mana source on the battlefield without paying for its initial cast.
fn source_game(cards: &SimDeck) -> GameState {
    let mut game = main_phase_state(vec![1], vec![2]);
    let mut source = new_perm_with(7, cards, CardIdx(0), 0, false);
    source.summoning_sick = false;
    game.battlefield.push(source);
    game.next_uid = 7;
    game
}

/// Try an actual target spell through production payment and spell settlement.
fn cast_target(cards: &SimDeck, game: &mut GameState, pool: &mut ManaPool) -> Option<u32> {
    try_cast_from_hand(
        cards,
        game,
        pool,
        1,
        CardIdx(1),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    )
}

/// Renaming a card cannot change its parsed mana production.
fn source_names(source: &CardRow) -> [CardRow; 2] {
    let mut renamed = source.clone();
    renamed.name = "Renamed identity source".into();
    [source.clone(), renamed]
}

/// Exact identity wording remains distinct from unrestricted and opponent mana.
#[test]
fn oracle_identity_mana_retains_its_production_constraint() {
    for apostrophe in ["'", "’"] {
        let text =
            format!("Add one mana of any color in your commander{apostrophe}s color identity.");
        let mana = parse_tap_yield(&text).expect("complete identity production");
        assert_eq!(mana.commander_pips, 1);
        assert_eq!(mana.any_pips, 0);
        assert!(!mana.opponent_any);
        assert_eq!(mana.total(), 1);
        let mut context_free = ManaPool::default();
        crate::deck::simulator::game_mana::add_yield(&mana, &mut context_free);
        assert_eq!(context_free.total(), 0);
        assert!(parse_tap_yield(&text.replace('.', " unless an opponent pays {1}.")).is_none());
    }
    for name in ["Arcane Signet", "Command Tower"] {
        for source in source_names(&named_row(name)) {
            let card = parse_sim_card(&source);
            let mana = card.tap.expect("supported source-tap ability");
            assert_eq!(mana.commander_pips, 1);
            assert_eq!(mana.any_pips, 0);
        }
    }
}

/// Partner identities come from metadata rather than printed colors or mana costs.
#[test]
fn stored_commander_identity_includes_off_cost_colors_and_partners() {
    let source = named_row("Arcane Signet");
    let cards = identity_deck(&source, &["U", "R"], "{U}{R}");
    assert!(cards.commanders.iter().all(|card| card.colors[0]));
    assert_eq!(
        cards.commanders[0].color_identity,
        [false, true, false, false, false]
    );
    assert_eq!(
        cards.commanders[1].color_identity,
        [false, false, false, true, false]
    );
    assert_eq!(
        cards.commander_color_identity(),
        [false, true, false, true, false]
    );
    let mut game = source_game(&cards);
    game.battlefield
        .push(new_perm_with(8, &cards, CardIdx(0), 0, false));
    let mut pool = crate::deck::simulator::game_run::mana::build_pool(&cards, &mut game, 1);
    assert_eq!(pool.total(), 2);
    assert_eq!(cast_target(&cards, &mut game, &mut pool), Some(2));
    assert_eq!(pool.total(), 0);
    assert_eq!(game.graveyard, [CardIdx(1)]);
}

/// All production routes can cast only allowed colors or generic costs.
#[test]
fn native_leftover_and_nonspell_identity_mana_cast_only_eligible_costs() {
    for name in ["Arcane Signet", "Command Tower"] {
        for source in source_names(&named_row(name)) {
            for (cost, legal) in [
                ("{U}", true),
                ("{R}", true),
                ("{1}", true),
                ("{W}", false),
                ("{B}", false),
                ("{G}", false),
                ("{C}", false),
            ] {
                for route in ["native", "leftover", "nonspell"] {
                    let cards = identity_deck(&source, &["UR"], cost);
                    let mut game = source_game(&cards);
                    let permanent = game.battlefield[0].clone();
                    let mut pool = ManaPool::default();
                    match route {
                        "native" => assert!(activate_mana_mode(
                            &cards, &mut game, &permanent, &mut pool, 1
                        )),
                        "nonspell" => assert!(tap_for_nonspell_payment(
                            &cards, &mut game, &permanent, &mut pool, 1
                        )),
                        "leftover" => spend_leftover(&cards, &mut game, &mut pool, 1),
                        _ => unreachable!("fixed production route"),
                    }
                    assert_eq!(pool.total(), 1, "{} {cost} {route}", source.name);
                    assert_eq!(pool.colorless, 0);
                    assert!(game.battlefield[0].tapped);
                    assert_eq!(
                        cast_target(&cards, &mut game, &mut pool),
                        legal.then_some(1),
                        "{} {cost} {route}",
                        source.name
                    );
                    assert_eq!(pool.total(), u32::from(!legal));
                    assert_eq!(game.graveyard.contains(&CardIdx(1)), legal);
                    assert_eq!(game.hand.contains(&CardIdx(1)), !legal);
                }
            }
        }
    }
}

/// No commander and a colorless commander both leave these sources unable to pay any cost.
#[test]
fn empty_and_missing_commander_identity_do_not_produce_colorless_or_generic_mana() {
    for name in ["Arcane Signet", "Command Tower"] {
        for identities in [vec![], vec![""]] {
            for cost in ["{1}", "{U}", "{C}"] {
                let cards = identity_deck(&named_row(name), &identities, cost);
                assert_eq!(cards.commander_color_identity(), [false; 5]);
                let mut game = source_game(&cards);
                let permanent = game.battlefield[0].clone();
                let mut pool = ManaPool::default();
                assert!(!activate_mana_mode(
                    &cards, &mut game, &permanent, &mut pool, 1
                ));
                assert!(!tap_for_nonspell_payment(
                    &cards, &mut game, &permanent, &mut pool, 1
                ));
                assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
                spend_leftover(&cards, &mut game, &mut pool, 1);
                assert_eq!(pool.total(), 0);
                assert_eq!(cast_target(&cards, &mut game, &mut pool), None);
                assert!(!game.battlefield[0].tapped);
                assert!(game.graveyard.is_empty());
                assert!(game.completion.is_none());
                assert!(!game.infinite_mana_suspected);
            }
        }
    }
}

/// The original name does not retain identity restrictions after its Oracle text changes.
#[test]
fn changing_oracle_production_changes_the_actual_eligible_colors() {
    for name in ["Arcane Signet", "Command Tower"] {
        for (text, cost, legal) in [
            ("{T}: Add one mana of any color.", "{G}", true),
            ("{T}: Add {G}.", "{G}", true),
            ("{T}: Add {G}.", "{U}", false),
        ] {
            let mut source = named_row(name);
            source.oracle_text = text.into();
            let cards = identity_deck(&source, &[], cost);
            assert_eq!(
                cards.cards[0]
                    .tap
                    .as_ref()
                    .expect("changed mana mode")
                    .commander_pips,
                0
            );
            let mut game = source_game(&cards);
            let permanent = game.battlefield[0].clone();
            let mut pool = ManaPool::default();
            assert!(activate_mana_mode(
                &cards, &mut game, &permanent, &mut pool, 1
            ));
            assert_eq!(
                cast_target(&cards, &mut game, &mut pool),
                legal.then_some(1)
            );
        }
    }
}

/// A nonland tap bonus copies only a type the identity source actually produced.
#[test]
fn identity_mana_bonus_respects_colors_and_land_source_type() {
    for (name, cost, expected, legal, identity) in [
        ("Arcane Signet", "{U}{U}", 2, true, Some("UR")),
        ("Arcane Signet", "{U}{R}", 2, false, Some("UR")),
        ("Arcane Signet", "{G}{G}", 2, false, Some("UR")),
        ("Command Tower", "{U}", 1, true, Some("UR")),
        ("Command Tower", "{U}{U}", 1, false, Some("UR")),
        ("Arcane Signet", "{1}", 0, false, Some("")),
        ("Arcane Signet", "{1}", 0, false, None),
    ] {
        let identities: Vec<_> = identity.into_iter().collect();
        let mut cards = identity_deck(&named_row(name), &identities, cost);
        cards.cards.push(parse_sim_card(&row(
            "Nonland mana bonus", "{100}", "Enchantment",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        )));
        assert!(cards.cards[3].flags.bonus_mana_on_nonland_tap);
        for leftover in [false, true] {
            let mut game = source_game(&cards);
            game.battlefield
                .push(new_perm_with(8, &cards, CardIdx(3), 0, false));
            let mut pool = if leftover {
                let mut pool = ManaPool::default();
                spend_leftover(&cards, &mut game, &mut pool, 1);
                pool
            } else {
                crate::deck::simulator::game_run::mana::build_pool(&cards, &mut game, 1)
            };
            assert_eq!(pool.total(), expected, "{name} {cost} leftover={leftover}");
            assert_eq!(game.battlefield[0].tapped, expected > 0);
            assert!(game.completion.is_none());
            assert_eq!(
                cast_target(&cards, &mut game, &mut pool),
                legal.then_some(expected)
            );
        }
    }
}

/// Fellwar-style opponent colors do not inherit the commander's identity restriction.
#[test]
fn opponent_and_unrestricted_mana_keep_their_existing_color_choices() {
    let source = named_row("Fellwar Stone");
    let cards = identity_deck(&source, &["U"], "{G}");
    let mana = cards.cards[0].tap.as_ref().expect("opponent mana mode");
    assert!(mana.opponent_any);
    assert_eq!(mana.commander_pips, 0);
    let mut game = source_game(&cards);
    game.opponents.config.land_colors = Some(vec!["G".into()]);
    game.opponents.seats[0].active = true;
    game.opponents.seats[0].lands = 1;
    let source = game.battlefield[0].clone();
    let mut pool = ManaPool::default();
    assert!(activate_mana_mode(&cards, &mut game, &source, &mut pool, 1));
    assert_eq!(cast_target(&cards, &mut game, &mut pool), Some(1));
}

/// Full turns report only real capacity and never mark excluded colors as mana-ready.
#[test]
fn identity_mana_full_turn_reports_do_not_invent_capacity_or_readiness() {
    for name in ["Arcane Signet", "Command Tower"] {
        for (identities, cost, available, legal) in [
            (vec!["UR"], "{U}", 1.0, true),
            (vec!["U", "R"], "{R}", 1.0, true),
            (vec!["UR"], "{G}", 1.0, false),
            (vec![""], "{1}", 0.0, false),
            (vec![], "{C}", 0.0, false),
        ] {
            let cards = identity_deck(&named_row(name), &identities, cost);
            let mut game = source_game(&cards);
            let mut census =
                TurnCensus::with_commander_count(1, cards.cards.len(), cards.commanders.len());
            run_turn(
                &cards,
                &mut game,
                &mut census,
                &CommanderProfile::new(&cards),
                &mut Vec::new(),
                1,
                false,
            );
            assert_eq!(census.mana_ready[1], legal.then_some(1));
            assert_eq!(game.graveyard.contains(&CardIdx(1)), legal);
            assert_eq!(game.battlefield[0].tapped, available > 0.0);
            let log = finish_log(&cards, game, census, 0, false, 1);
            assert_eq!(log.mana_available, [available]);
            assert_eq!(log.mana_spent, [if legal { 1.0 } else { 0.0 }]);
            assert_eq!(log.mana_remaining, [if legal { 0.0 } else { available }]);
            let stats = aggregate(std::slice::from_ref(&log), &cards, 1);
            if cost == "{G}" {
                let findings = crate::deck::simulator::findings::analyze_findings(&stats, &cards);
                let color_problem = findings
                    .iter()
                    .find(|finding| {
                        finding.kind == "insufficient_color_mana"
                            && finding.color
                                == Some(crate::deck::simulator::model::ManaColor::Green)
                    })
                    .expect("the actual failed cast records a green pip block");
                assert!(
                    color_problem
                        .explanation
                        .contains("0 dedicated untapped sources, 0 choice lands")
                );
            }
            let report = crate::deck::simulator::report::json_report(
                &stats,
                &cards,
                "Identity mana",
                0,
                &[],
                Default::default(),
                &Default::default(),
            );
            let json = serde_json::to_value(&report).expect("report serializes");
            assert_eq!(json["provenance"]["schema_version"], 3);
            assert_eq!(json["mana"]["avg_available_by_turn"]["1"], available);
            assert_eq!(
                json["mana"]["avg_cast_spent_by_turn"]["1"],
                if legal { 1.0 } else { 0.0 }
            );
            assert_eq!(
                report.color_sources.flexible_nonland_sources,
                u32::from(name == "Arcane Signet" && available > 0.0)
            );
            if name == "Command Tower" {
                assert_eq!(
                    report.color_sources.choice_source_lands.blue,
                    if available > 0.0 { 1.0 } else { 0.0 }
                );
                assert_eq!(
                    report.color_sources.choice_source_lands.red,
                    if available > 0.0 { 1.0 } else { 0.0 }
                );
                assert_eq!(report.color_sources.choice_source_lands.white, 0.0);
                assert_eq!(report.color_sources.choice_source_lands.black, 0.0);
                assert_eq!(report.color_sources.choice_source_lands.green, 0.0);
            }
        }
    }
}

/// Merged summaries retain identity choices without restricting supported sibling modes.
#[test]
fn identity_mana_summaries_keep_fixed_and_unrestricted_sibling_modes() {
    for (sibling, any_pips) in [("{T}: Add {G}.", 0), ("{T}: Add one mana of any color.", 1)] {
        for identities in [vec!["UR"], vec![], vec![""]] {
            let mut source = named_row("Command Tower");
            source.oracle_text = format!("{}\n{sibling}", source.oracle_text);
            let cards = identity_deck(&source, &identities, "{G}");
            let mana = cards.cards[0].tap.as_ref().expect("supported source modes");
            assert!(mana.alternatives);
            assert_eq!(mana.commander_pips, 1);
            assert_eq!(mana.any_pips, any_pips);
            assert_eq!(mana.total(), 1);
            let mut game = source_game(&cards);
            let mut pool = crate::deck::simulator::game_run::mana::build_pool(&cards, &mut game, 1);
            assert_eq!(cast_target(&cards, &mut game, &mut pool), Some(1));
            let stats = aggregate(&[], &cards, 1);
            let report = crate::deck::simulator::report::json_report(
                &stats,
                &cards,
                "Source summary",
                0,
                &[],
                Default::default(),
                &Default::default(),
            );
            assert_eq!(
                report.color_sources.choice_source_lands.green
                    + report.color_sources.fixed_source_lands.green,
                1.0
            );
            assert_eq!(
                report.color_sources.choice_source_lands.white,
                f64::from(any_pips)
            );
            assert_eq!(
                report.color_sources.choice_source_lands.black,
                f64::from(any_pips)
            );
        }
    }
}

/// A one-color identity is one dedicated land source, not an unrestricted choice land.
#[test]
fn single_color_identity_source_census_counts_only_its_stored_color() {
    let cards = identity_deck(&named_row("Command Tower"), &["U"], "{U}");
    let stats = aggregate(&[], &cards, 1);
    let report = crate::deck::simulator::report::json_report(
        &stats,
        &cards,
        "Single color",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert_eq!(report.color_sources.fixed_source_lands.blue, 1.0);
    assert_eq!(report.color_sources.choice_source_lands.blue, 0.0);
    assert_eq!(report.color_sources.fixed_source_lands.white, 0.0);
    assert_eq!(report.color_sources.choice_source_lands.white, 0.0);
}

/// A synthetic external land grant funds one green cast without waiving native identity rules.
pub(super) fn assert_independent_any_color_grants_with_empty_identity() {
    let source = named_row("Command Tower");
    crate::deck::simulator::card_test_support::pinned_revision(&source);
    let mut cards = identity_deck(&source, &[], "{G}");
    cards.cards.push(parse_sim_card(&row(
        "Land mana grant",
        "{100}",
        "Artifact",
        "Lands you control have \"{T}: Add one mana of any color.\"",
    )));
    assert!(matches!(
        cards.cards[3].flags.grant,
        Some(crate::deck::simulator::model::Grant::Lands)
    ));
    assert!(cards.commanders.is_empty());
    assert_eq!(cards.commander_color_identity(), [false; 5]);
    assert_eq!(
        cards.cards[0]
            .tap
            .as_ref()
            .expect("native identity source")
            .commander_pips,
        1
    );
    for leftover in [false, true] {
        let mut native = source_game(&cards);
        let native_rng = native.library_rng.clone();
        let mut native_pool = ManaPool::default();
        if leftover {
            spend_leftover(&cards, &mut native, &mut native_pool, 1);
        } else {
            native_pool =
                crate::deck::simulator::game_run::mana::build_pool(&cards, &mut native, 1);
        }
        assert_eq!(native_pool, ManaPool::default());
        assert!(!native.battlefield[0].tapped);
        assert_eq!(cast_target(&cards, &mut native, &mut native_pool), None);
        assert_eq!(native.hand, [CardIdx(1)]);
        assert_eq!(native.library, [CardIdx(2)]);
        assert!(native.graveyard.is_empty() && native.exile.is_empty());
        assert_library_rng(&native.library_rng, &native_rng);
        let mut game = source_game(&cards);
        game.battlefield
            .push(new_perm_with(8, &cards, CardIdx(3), 0, false));
        let rng = game.library_rng.clone();
        let mut pool = if leftover {
            let mut pool = ManaPool::default();
            spend_leftover(&cards, &mut game, &mut pool, 1);
            pool
        } else {
            crate::deck::simulator::game_run::mana::build_pool(&cards, &mut game, 1)
        };
        assert_eq!(pool.total(), 1);
        assert_eq!(
            pool,
            ManaPool {
                flexible: 1,
                ..ManaPool::default()
            }
        );
        assert!(game.battlefield[0].tapped);
        assert_eq!(game.battlefield[0].uid, 7);
        assert!(!game.battlefield[1].tapped);
        assert_eq!(cast_target(&cards, &mut game, &mut pool), Some(1));
        assert_eq!(pool, ManaPool::default());
        assert!(game.hand.is_empty());
        assert_eq!(game.graveyard, [CardIdx(1)]);
        assert_eq!(game.library, [CardIdx(2)]);
        spend_leftover(&cards, &mut game, &mut pool, 1);
        assert_eq!(pool, ManaPool::default());
        assert_eq!(game.battlefield.len(), 2);
        assert!(game.battlefield[0].tapped);
        assert!(game.completion.is_none());
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// Independent granted mana remains available when the native commander identity is empty.
#[test]
fn independent_any_color_grants_do_not_inherit_the_empty_identity() {
    assert_independent_any_color_grants_with_empty_identity();
}
