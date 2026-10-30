//! Synthetic power and crew boundaries supplement existing counter and station packets.
//! Equipment payment is an explicit production action before the reported turn.
//! Reports do not mislabel that equip cost as mana spent on a spell.
//! Unknown printed power uses the declared generic two-power model, not a dynamic rule.

use crate::db::CardRow;
use crate::deck::simulator::card_test_support::main_phase_state;
use crate::deck::simulator::game::{CardRef, GameState, ManaPool};
use crate::deck::simulator::game_effects::activation::apply_minus_counter;
use crate::deck::simulator::game_effects::{resolve_sacrifice_uid, tap_budget};
use crate::deck::simulator::library_core_report_tests::{executed_report, put_permanents};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// Give a controlled source printed power without introducing inferred power rules.
fn body(name: &str, kind: &str, power: &str, text: &str) -> CardRow {
    let mut source = row(name, "{0}", kind, text);
    source.power = Some(power.into());
    source.toughness = Some("20".into());
    source
}

/// Supply only the modeled physical crew or station consumer.
fn consumer(kind: &str, threshold: u32) -> CardRow {
    match kind {
        "crew" => body(
            "Power consumer",
            "Artifact — Vehicle",
            "7",
            &format!("Crew {threshold}"),
        ),
        "station" => body(
            "Power consumer",
            "Artifact — Spacecraft",
            "7",
            &format!(
                "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at {threshold}+.)\n{threshold}+ | Flying"
            ),
        ),
        "combat" => row("Unused consumer", "{99}", "Artifact", ""),
        _ => panic!("the controlled consumer kind is explicit"),
    }
}

/// Supported all-creature statics affect each power consumer until the source leaves.
pub(super) fn assert_static_power_consumers_report() {
    for kind in ["combat", "crew", "station"] {
        for live in [true, false] {
            for removals in [0, 4] {
                let cards = deck(&[
                    body("Static host", "Creature", "3", ""),
                    row(
                        "Static bonus",
                        "{99}",
                        "Enchantment",
                        "Creatures you control get +2/+2.",
                    ),
                    consumer(kind, 5),
                ]);
                assert_eq!(cards.cards[1].flags.buff, Some((2, 2)));
                let mut game = state(vec![], vec![]);
                put_permanents(&cards, &mut game, &[0, 1]);
                for _ in 0..removals {
                    apply_minus_counter(&cards, &mut game, 1, 1);
                }
                if !live {
                    resolve_sacrifice_uid(&cards, &mut game, 1, 2);
                }
                if kind != "combat" {
                    put_permanents(&cards, &mut game, &[2]);
                }
                let (game, log, report) = executed_report(&cards, game, 1, vec![]);
                assert_eq!(game.battlefield[0].uid, 1);
                assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
                assert_eq!(game.graveyard, if live { vec![] } else { vec![CardIdx(1)] });
                assert_eq!(game.life, 20);
                let host_power = (3_i32 - removals + if live { 2 } else { 0 }).max(0) as u32;
                let online = host_power >= 5;
                let power = if kind == "combat" || kind == "crew" && !online {
                    host_power
                } else if online {
                    9
                } else {
                    0
                };
                assert_eq!(game.battlefield[0].counters.minus1, removals as u32);
                assert_eq!(log.attack_power, [power]);
                assert_eq!(
                    report["combat"]["attack_power_avg_by_turn"]["1"],
                    f64::from(power)
                );
                if kind != "combat" {
                    let target = game.battlefield.last().expect("consumer is on the board");
                    assert_eq!(target.uid, 3);
                    if kind == "crew" {
                        assert_eq!(target.crewed, online);
                    } else {
                        assert_eq!(target.counters.charge, host_power);
                        assert_eq!(target.animated, online);
                    }
                    assert_eq!(
                        report["creatures_by_turn"]["1"],
                        if online { 2.0 } else { 1.0 }
                    );
                }
                assert_eq!(
                    report["graveyard"]["avg_size_by_turn"]["1"],
                    if live { 0.0 } else { 1.0 }
                );
            }
        }
    }
}

/// Pay the real equip cost and remove an earlier object to shift battlefield positions.
fn equipment_action(kind: &str, funded: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row("Earlier object", "{0}", "Artifact", ""),
        body("Equipment host", "Creature", "3", ""),
        body("Other body", "Creature", "1", ""),
        row(
            "Paid Equipment",
            "{0}",
            "Artifact — Equipment",
            "Equipped creature gets +2/+0.\nEquip {2}",
        ),
        consumer(kind, 5),
    ]);
    assert_eq!(
        cards.cards[3]
            .flags
            .equipment
            .expect("equip cost lowers")
            .cost,
        2
    );
    let mut game = main_phase_state(vec![], vec![]);
    put_permanents(&cards, &mut game, &[0, 1, 2, 3]);
    for permanent in &mut game.battlefield {
        permanent.summoning_sick = false;
    }
    let mut pool = ManaPool {
        colorless: if funded { 2 } else { 1 },
        ..Default::default()
    };
    tap_budget(&cards, &mut game.battlefield, &mut pool, &[]);
    assert_eq!(pool.total(), if funded { 0 } else { 1 });
    assert_eq!(game.battlefield[3].equipped, funded);
    assert_eq!(game.battlefield[3].equip_host, funded.then_some(2));
    assert!(!game.battlefield[1].tapped && !game.battlefield[2].tapped);
    resolve_sacrifice_uid(&cards, &mut game, 1, 1);
    assert_eq!(game.battlefield[0].uid, 2);
    assert_eq!(game.battlefield[2].uid, 4);
    assert_eq!(game.battlefield[2].equip_host, funded.then_some(2));
    if kind != "combat" {
        put_permanents(&cards, &mut game, &[4]);
    }
    (cards, game)
}

/// Paid Equipment follows its host through index shifts, crew, station, and host loss.
pub(super) fn assert_equipment_host_report() {
    for kind in ["combat", "crew", "station"] {
        for funded in [true, false] {
            let (cards, game) = equipment_action(kind, funded);
            let (game, log, report) = executed_report(&cards, game, 1, vec![]);
            assert_eq!(game.battlefield[0].uid, 2);
            assert_eq!(game.battlefield[1].uid, 3);
            assert_eq!(game.battlefield[2].uid, 4);
            assert_eq!(game.battlefield[2].equip_host, funded.then_some(2));
            assert_eq!(game.graveyard, [CardIdx(0)]);
            let power = match kind {
                "combat" => {
                    if funded {
                        6
                    } else {
                        4
                    }
                }
                "crew" => {
                    if funded {
                        8
                    } else {
                        4
                    }
                }
                "station" => {
                    if funded {
                        8
                    } else {
                        0
                    }
                }
                _ => unreachable!(),
            };
            assert_eq!(log.attack_power, [power]);
            assert_eq!(log.mana_spent, [0.0]);
            assert_eq!(
                report["combat"]["attack_power_avg_by_turn"]["1"],
                f64::from(power)
            );
            assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 0.0);
            assert_eq!(game.life, 20);
            if kind != "combat" {
                let target = &game.battlefield[3];
                assert_eq!(target.uid, 5);
                if kind == "crew" {
                    assert_eq!(target.crewed, funded);
                } else {
                    assert_eq!(target.counters.charge, if funded { 5 } else { 4 });
                    assert_eq!(target.animated, funded);
                }
            }
        }
    }
    assert_equipment_counter_order_report();
    let (cards, mut game) = equipment_action("combat", true);
    resolve_sacrifice_uid(&cards, &mut game, 1, 2);
    let (game, log, report) = executed_report(&cards, game, 1, vec![]);
    assert_eq!(
        game.battlefield
            .iter()
            .map(|permanent| permanent.uid)
            .collect::<Vec<_>>(),
        [3, 4]
    );
    assert_eq!(game.battlefield[1].equip_host, Some(2));
    assert_eq!(game.graveyard, [CardIdx(0), CardIdx(1)]);
    assert_eq!(log.attack_power, [1]);
    assert_eq!(report["combat"]["attack_power_avg_by_turn"]["1"], 1.0);
    assert_eq!(report["creatures_by_turn"]["1"], 1.0);
}

/// Equipment bonuses apply before clamping a negative host's current power to zero.
fn assert_equipment_counter_order_report() {
    for kind in ["combat", "crew", "station"] {
        let (cards, mut game) = equipment_action(kind, true);
        for _ in 0..4 {
            apply_minus_counter(&cards, &mut game, 1, 2);
        }
        let (game, log, report) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(game.battlefield[0].uid, 2);
        assert_eq!(game.battlefield[0].counters.minus1, 4);
        assert_eq!(game.battlefield[2].equip_host, Some(2));
        let power = if kind == "station" { 0 } else { 2 };
        assert_eq!(log.attack_power, [power]);
        assert_eq!(
            report["combat"]["attack_power_avg_by_turn"]["1"],
            f64::from(power)
        );
        if kind == "crew" {
            assert!(!game.battlefield[3].crewed);
        }
        if kind == "station" {
            assert_eq!(game.battlefield[3].counters.charge, 2);
            assert!(!game.battlefield[3].animated);
        }
    }
}

/// Hidden characteristics and unknown printed power never grant seven-power crew or station.
pub(super) fn assert_hidden_power_consumers_report() {
    for kind in ["combat", "crew", "station"] {
        for (printed, face_down, power) in [("7", false, 7), ("7", true, 2), ("*", false, 2)] {
            let cards = deck(&[
                body("Power source", "Creature", printed, "Lifelink"),
                consumer(kind, 3),
            ]);
            assert_eq!(
                cards.cards[0].printed_power,
                if printed == "*" { None } else { Some(7) }
            );
            let mut game = state(vec![], vec![]);
            put_permanents(&cards, &mut game, &[0]);
            game.battlefield[0].face_down = face_down;
            if kind != "combat" {
                put_permanents(&cards, &mut game, &[1]);
            }
            let (game, log, report) = executed_report(&cards, game, 1, vec![]);
            assert_eq!(game.battlefield[0].uid, 1);
            assert_eq!(game.battlefield[0].face_down, face_down);
            let output = if kind == "combat" || kind == "crew" && power < 3 {
                power
            } else if power >= 3 {
                7
            } else {
                0
            };
            assert_eq!(log.attack_power, [output]);
            assert_eq!(
                report["combat"]["attack_power_avg_by_turn"]["1"],
                f64::from(output)
            );
            let gain = if (kind == "combat" || kind == "crew" && power < 3) && !face_down {
                power
            } else {
                0
            };
            assert_eq!(game.life, 20 + gain as i32);
            assert_eq!(report["draw"]["avg_life_gained"], f64::from(gain));
            if kind != "combat" {
                assert_eq!(game.battlefield[1].uid, 2);
                if kind == "crew" {
                    assert_eq!(game.battlefield[1].crewed, power >= 3);
                } else {
                    assert_eq!(game.battlefield[1].counters.charge, power);
                    assert_eq!(game.battlefield[1].animated, power >= 3);
                }
            }
        }
    }
}

/// Sick bodies can pay crew, but crewing cannot bypass the Vehicle's entry turn.
pub(super) fn assert_sick_crew_report() {
    for (old_vehicle, crew_power, turns) in [(true, "3", 1), (false, "3", 2), (true, "2", 1)] {
        let cards = deck(&[
            body("Crew Vehicle", "Artifact — Vehicle", "7", "Crew 3"),
            body("Fresh crew", "Creature", crew_power, ""),
            row("Turn draw", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(if old_vehicle { vec![1] } else { vec![0, 1] }, vec![2]);
        if old_vehicle {
            put_permanents(&cards, &mut game, &[0]);
        }
        let (game, log, report) = executed_report(&cards, game, turns, vec![]);
        let enough = crew_power == "3";
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(game.battlefield[1].uid, 2);
        assert_eq!(game.battlefield[0].crewed, enough);
        assert_eq!(game.battlefield[1].tapped, enough);
        assert_eq!(game.battlefield[1].summoning_sick, turns == 1);
        assert_eq!(game.battlefield[1].entered_turn, 1);
        let first_power = if old_vehicle && enough { 7 } else { 0 };
        assert_eq!(log.attack_power[0], first_power);
        assert_eq!(
            report["combat"]["attack_power_avg_by_turn"]["1"],
            f64::from(first_power)
        );
        assert_eq!(
            report["creatures_by_turn"]["1"],
            if enough { 2.0 } else { 1.0 }
        );
        if turns == 2 {
            assert_eq!(log.attack_power, [0, 7]);
            assert_eq!(report["combat"]["attack_power_avg_by_turn"]["2"], 7.0);
            assert_eq!(game.hand, [CardIdx(2)]);
        }
        assert_eq!(game.life, 20);
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
    }
}
