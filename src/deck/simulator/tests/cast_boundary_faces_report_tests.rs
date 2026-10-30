//! Synthetic face-route controls. These do not certify a whole printed card.

use super::*;
use crate::deck::simulator::game_effects::creature_power;

/// The declared cheaper split-face policy pays one cost and resolves one effect sequence.
fn assert_split_report() {
    let cards = deck(&[
        row(
            "Synthetic split spell",
            "{3} // {1}",
            "Sorcery // Sorcery",
            "Gain 2 life.",
        ),
        row("Split payment", "{99}", "Artifact", "{T}: Add {C}."),
    ]);
    assert_eq!(cards.cards[0].min_cost.total(), 1);
    for funded in [false, true] {
        let mut game = state(vec![0], vec![]);
        if funded {
            put_permanents(&cards, &mut game, &[1]);
        }
        let random = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.hand, if funded { vec![] } else { vec![CardIdx(0)] });
        assert_eq!(
            game.graveyard,
            if funded { vec![CardIdx(0)] } else { vec![] }
        );
        assert_eq!(game.spells_cast_this_turn, u32::from(funded));
        assert_eq!(game.life, if funded { 22 } else { 20 });
        assert_eq!(game.life_gained, if funded { 2 } else { 0 });
        assert_eq!(game.life_paid, 0);
        assert!(game.library.is_empty() && game.exile.is_empty());
        assert_library_rng(&game.library_rng, &random);
        assert_eq!(log.mana_spent, [f64::from(funded)]);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(
            report.draw.avg_life_gained,
            Some(if funded { 2.0 } else { 0.0 })
        );
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], f64::from(funded));
        assert_assumption(&report, "Split cards use the modeled cheaper face");
    }
}

/// Keep plot timing separate from the real cast's additional life cost and entry effect.
fn plot_deck(mana: usize) -> SimDeck {
    deck(&[
        body(
            "Synthetic plot body",
            "{5}",
            "Plot {1}\nAs an additional cost to cast this spell, pay 1 life.\nWhen this creature enters, gain 3 life.",
            4,
        ),
        row(
            "Plot payment",
            "{99}",
            "Artifact",
            &format!("{{T}}: Add {}.", "{C}".repeat(mana)),
        ),
        row("Turn-two draw", "{99}", "Sorcery", ""),
        row("Library remainder", "{99}", "Sorcery", ""),
    ])
}

/// Plot spends mana now but cannot cast until a later turn; printed-cost casts stay available.
fn assert_plot_report() {
    for (mana, turns, plotted, cast, paid, gained) in [
        (0, 1, false, false, 0, 0),
        (1, 1, true, false, 0, 0),
        (1, 2, false, true, 1, 3),
        (5, 1, false, true, 1, 3),
    ] {
        let cards = plot_deck(mana);
        assert_eq!(
            cards.cards[0]
                .keyword_abilities
                .plot
                .expect("plot cost parsed")
                .total(),
            1
        );
        let mut game = state(vec![0], vec![3, 2]);
        if mana > 0 {
            put_permanents(&cards, &mut game, &[1]);
        }
        let random = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, turns, vec![]);
        assert_eq!(
            game.plotted,
            if plotted {
                vec![(CardIdx(0), 1)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.hand,
            if turns == 2 {
                vec![CardIdx(2)]
            } else if !plotted && !cast {
                vec![CardIdx(0)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            game.library,
            if turns == 2 {
                vec![CardIdx(3)]
            } else {
                vec![CardIdx(3), CardIdx(2)]
            }
        );
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.spells_cast_this_turn, u32::from(cast));
        assert_eq!(game.life_paid, paid);
        assert_eq!(game.life_gained, gained);
        assert_eq!(game.life, 20 - paid as i32 + gained as i32);
        assert_eq!(
            game.battlefield_seen.get(&CardIdx(0)),
            cast.then_some(&(turns as u32))
        );
        if cast {
            let permanent = game.battlefield.last().expect("cast body entered");
            assert_eq!(permanent.card, CardRef::Deck(CardIdx(0)));
            assert_eq!(permanent.entered_turn, turns);
            assert!(permanent.summoning_sick);
            assert!(!permanent.face_down && !permanent.land_face && !permanent.tapped);
        }
        assert_library_rng(&game.library_rng, &random);
        assert_eq!(
            log.mana_spent,
            vec![if mana == 5 { 5.0 } else { 0.0 }; turns]
        );
        assert_eq!(log.mana_remaining[0], 0.0);
        if turns == 2 {
            assert_eq!(log.mana_remaining[1], 1.0);
        }
        let report = report_log(&cards, &log, turns as u32);
        assert_finished_turn(&report, &log);
        assert_eq!(report.draw.avg_life_paid, Some(f64::from(paid)));
        assert_eq!(report.draw.avg_life_gained, Some(f64::from(gained)));
        assert_eq!(report.creatures_by_turn[&(turns as u32)], f64::from(cast));
        assert_eq!(
            report.mana.avg_cast_spent_by_turn[&1],
            if mana == 5 { 5.0 } else { 0.0 }
        );
    }
}

/// Morph-family bodies cost {3}, lose printed entry text, and do not reenter when turned up.
fn assert_morph_report() {
    for keyword in ["Morph", "Megamorph", "Disguise"] {
        for blue in [false, true] {
            assert_face_up_turns(keyword, blue);
        }
        assert_short_face_down_payment(keyword);
        assert_printed_face_payment(keyword);
    }
}

/// A face-up payment uses a later actual colored source without another entry event.
fn assert_face_up_turns(keyword: &str, blue: bool) {
    let cards = deck(&[
        body(
            "Synthetic hidden body",
            "{4}{G}",
            &format!("{keyword} {{U}}\nWhen this creature enters, draw a card."),
            5,
        ),
        row(
            "Face-down payment",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}{C}.",
        ),
        row(
            "Turn-two land",
            "",
            "Land",
            if blue {
                "{T}: Add {U}."
            } else {
                "{T}: Add {G}."
            },
        ),
        row("Undrawn entry reward", "{99}", "Sorcery", ""),
    ]);
    assert_eq!(
        cards.cards[0]
            .morph_cost
            .expect("morph-family cost parsed")
            .total(),
        1
    );
    let mut game = state(vec![0], vec![3, 2]);
    put_permanents(&cards, &mut game, &[1]);
    let random = game.library_rng.clone();
    let (game, log) = execute_turns(&cards, game, 2, vec![]);
    assert_eq!(game.battlefield.len(), 3);
    let hidden = &game.battlefield[1];
    assert_eq!(hidden.card, CardRef::Deck(CardIdx(0)));
    assert_eq!(hidden.uid, 2);
    assert_eq!(hidden.entered_turn, 1);
    assert_eq!(hidden.face_down, !blue);
    assert_eq!(hidden.counters.plus1, 0);
    assert_eq!(creature_power(hidden, &cards), if blue { 5 } else { 2 });
    assert_eq!(game.battlefield[2].card, CardRef::Deck(CardIdx(2)));
    assert_eq!(game.battlefield[2].uid, 3);
    assert!(game.battlefield[2].land_face && game.battlefield[2].tapped);
    assert_eq!(game.battlefield_seen.get(&CardIdx(0)), Some(&1));
    assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
    assert_eq!(game.library, [CardIdx(3)]);
    assert_eq!(game.seen, 1);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.spells_cast_this_turn, 0);
    assert_library_rng(&game.library_rng, &random);
    assert_eq!(log.cards_seen, [0, 1]);
    assert_eq!(log.mana_spent, [3.0, 0.0]);
    assert_eq!(log.mana_remaining, [0.0, if blue { 3.0 } else { 4.0 }]);
    assert_eq!(log.attack_power, [0, if blue { 5 } else { 2 }]);
    let report = report_log(&cards, &log, 2);
    assert_finished_turn(&report, &log);
    assert_eq!(report.velocity.avg_cards_seen[&1], 0.0);
    assert_eq!(report.velocity.avg_cards_seen[&2], 1.0);
    assert_eq!(
        report.combat.attack_power_avg_by_turn[&2],
        if blue { 5.0 } else { 2.0 }
    );
    assert_eq!(
        report.mana.avg_unused_by_turn[&2],
        if blue { 3.0 } else { 4.0 }
    );
    assert_assumption(
        &report,
        "A later face-up payment does not re-fire entry triggers",
    );
}

/// A two-mana pool cannot create the supported three-mana face-down body.
fn assert_short_face_down_payment(keyword: &str) {
    let cards = deck(&[
        body(
            "Synthetic unaffordable hidden body",
            "{4}{G}",
            &format!("{keyword} {{U}}\nWhen this creature enters, draw a card."),
            5,
        ),
        row(
            "Short face-down payment",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}.",
        ),
        row("Entry reward", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![2]);
    put_permanents(&cards, &mut game, &[1]);
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert_eq!(game.hand, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(log.mana_remaining, [2.0]);
    let report = report_log(&cards, &log, 1);
    assert_finished_turn(&report, &log);
    assert_eq!(report.creatures_by_turn[&1], 0.0);
    assert_eq!(report.velocity.avg_cards_seen[&1], 0.0);
}

/// A payable printed face keeps its ordinary entry effect instead of using the hidden route.
fn assert_printed_face_payment(keyword: &str) {
    let cards = deck(&[
        body(
            "Synthetic printed body",
            "{4}{G}",
            &format!("{keyword} {{U}}\nWhen this creature enters, draw a card."),
            5,
        ),
        row(
            "Printed face payment",
            "{99}",
            "Artifact",
            "{T}: Add {C}{C}{C}{C}{G}.",
        ),
        row("Actual entry draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![2]);
    put_permanents(&cards, &mut game, &[1]);
    let random = game.library_rng.clone();
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert_eq!(game.hand, [CardIdx(2)]);
    assert!(game.library.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
    assert_eq!(game.battlefield.len(), 2);
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(0)));
    assert!(!game.battlefield[1].face_down);
    assert_eq!(game.battlefield[1].entered_turn, 1);
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_eq!(game.seen, 1);
    assert_library_rng(&game.library_rng, &random);
    assert_eq!(log.mana_spent, [5.0]);
    let report = report_log(&cards, &log, 1);
    assert_finished_turn(&report, &log);
    assert_eq!(report.velocity.avg_cards_seen[&1], 1.0);
    assert_eq!(report.mana.avg_cast_spent_by_turn[&1], 5.0);
}

/// Unsupported Warp and manifest instructions cannot create an alternate cast or hidden body.
fn assert_unsupported_face_routes_report() {
    for mana in [1, 5] {
        let cards = deck(&[
            body(
                "Synthetic Warp body",
                "{5}",
                "Warp {1}\nWhen this creature enters, gain 3 life.",
                5,
            ),
            row(
                "Warp payment",
                "{99}",
                "Artifact",
                &format!("{{T}}: Add {}.", "{C}".repeat(mana)),
            ),
            row("Library remainder", "{99}", "Sorcery", ""),
        ]);
        let mut game = state(vec![0], vec![2]);
        put_permanents(&cards, &mut game, &[1]);
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.hand, if mana == 1 { vec![CardIdx(0)] } else { vec![] });
        assert!(game.exile.is_empty() && game.plotted.is_empty() && game.graveyard.is_empty());
        assert_eq!(game.battlefield.len(), if mana == 1 { 1 } else { 2 });
        assert_eq!(game.spells_cast_this_turn, u32::from(mana == 5));
        assert_eq!(game.life_gained, if mana == 1 { 0 } else { 3 });
        assert_eq!(log.mana_spent, [if mana == 1 { 0.0 } else { 5.0 }]);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(
            report.draw.avg_life_gained,
            Some(if mana == 1 { 0.0 } else { 3.0 })
        );
        assert_assumption(&report, "Warp casting is disabled");
    }
    let cards = deck(&[
        row(
            "Synthetic manifest instruction",
            "{0}",
            "Sorcery",
            "Manifest the top card of your library.",
        ),
        body(
            "Unmanifested physical card",
            "{0}",
            "When this creature enters, gain 3 life.",
            5,
        ),
    ]);
    let game = state(vec![0], vec![1]);
    let random = game.library_rng.clone();
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(1)]);
    assert!(game.hand.is_empty() && game.battlefield.is_empty() && game.exile.is_empty());
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_eq!(game.seen, 0);
    assert_eq!(game.life_gained, 0);
    assert_library_rng(&game.library_rng, &random);
    let report = report_log(&cards, &log, 1);
    assert_finished_turn(&report, &log);
    assert_eq!(report.creatures_by_turn[&1], 0.0);
    assert_eq!(report.draw.avg_life_gained, Some(0.0));
    assert_eq!(report.velocity.avg_cards_seen[&1], 0.0);
    assert_assumption(
        &report,
        "Manifest and other unsupported face-down instructions stay inert",
    );
}

/// Execute only the missing face-choice stages; existing MDFC packets remain separate.
pub(super) fn assert_face_choices_report() {
    assert_split_report();
    assert_plot_report();
    assert_morph_report();
    assert_unsupported_face_routes_report();
}
