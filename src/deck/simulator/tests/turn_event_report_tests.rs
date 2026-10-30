//! Synthetic event report controls. Generic bodies do not certify Dragon copies.

use crate::deck::simulator::card_test_support::assert_library_rng;
use crate::deck::simulator::game::{CardRef, TokenKind};
use crate::deck::simulator::library_core_report_tests::{executed_report, put_permanents};
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// Distinguish physical creature cards, generic token batches, and noncreature entries.
pub(super) fn assert_entry_filter_capture_report() {
    for (kind, text, gain, tokens) in [
        ("Creature", "", 4, 0),
        ("Sorcery", "Create two 1/1 creature tokens.", 2, 2),
        ("Artifact", "", 0, 0),
    ] {
        let cards = deck(&[
            row(
                "All creature observer",
                "{99}",
                "Artifact",
                "Whenever another creature you control enters, gain 1 life.",
            ),
            row(
                "Nontoken observer",
                "{99}",
                "Artifact",
                "Whenever another nontoken creature you control enters, gain 3 life.",
            ),
            row("Physical action", "{1}", kind, text),
            row("Funding land", "", "Basic Land", "{T}: Add {C}."),
        ]);
        let mut game = state(vec![2], vec![]);
        put_permanents(&cards, &mut game, &[0, 1, 3]);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(game.life, 20 + gain);
        assert_eq!(game.life_gained, gain as u32);
        assert!(game.hand.is_empty() && game.library.is_empty() && game.exile.is_empty());
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(game.battlefield[1].uid, 2);
        assert_eq!(game.battlefield[2].uid, 3);
        assert_eq!(game.battlefield.len(), if tokens > 0 { 5 } else { 4 });
        if tokens > 0 {
            assert_eq!(game.graveyard, [CardIdx(2)]);
            assert_eq!(game.next_uid, 5);
            for (index, token) in game.battlefield[3..].iter().enumerate() {
                assert_eq!(token.uid, 4 + index as u32);
                assert_eq!(token.card, CardRef::Token(TokenKind::Creature));
            }
        } else {
            assert!(game.graveyard.is_empty());
            assert_eq!(game.battlefield[3].card, CardRef::Deck(CardIdx(2)));
            assert_eq!(game.battlefield[3].uid, 4);
        }
        assert_library_rng(&game.library_rng, &rng);
        assert_eq!(log.life_gained, gain as u32);
        assert_eq!(log.mana_spent, [1.0]);
        assert_eq!(json["draw"]["avg_life_gained"], f64::from(gain));
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 1.0);
        assert_eq!(json["outcomes"]["player_loss_games"], 0);
    }
    assert_late_listener_capture();
}

/// A cascade child resolves before the new parent can observe its creature entry.
fn assert_late_listener_capture() {
    let parent = row(
        "New listener",
        "{2}",
        "Artifact",
        "Cascade\nWhenever another creature you control enters, gain 5 life.",
    );
    let mut old = parent.clone();
    old.name = "Existing listener".into();
    let cards = deck(&[
        parent,
        row(
            "Cascade creature",
            "{1}",
            "Creature",
            "When this creature enters, gain 2 life.",
        ),
        row("Library marker", "{99}", "Sorcery", ""),
        row("Funding one", "", "Basic Land", "{T}: Add {C}."),
        row("Funding two", "", "Basic Land", "{T}: Add {C}."),
        old,
    ]);
    for existing in [true, false] {
        let mut game = state(vec![0], vec![2, 1]);
        put_permanents(&cards, &mut game, &[3, 4]);
        if existing {
            put_permanents(&cards, &mut game, &[5]);
        }
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        let gain = if existing { 7 } else { 2 };
        let child_uid = if existing { 4 } else { 3 };
        assert_eq!(game.life, 20 + gain);
        assert_eq!(game.life_gained, gain as u32);
        assert_eq!(game.library, [CardIdx(2)]);
        assert!(game.hand.is_empty() && game.graveyard.is_empty());
        assert_eq!(
            game.battlefield[child_uid as usize - 1].card,
            CardRef::Deck(CardIdx(1))
        );
        assert_eq!(
            game.battlefield[child_uid as usize].card,
            CardRef::Deck(CardIdx(0))
        );
        assert_eq!(game.battlefield[child_uid as usize - 1].uid, child_uid);
        assert_eq!(game.battlefield[child_uid as usize].uid, child_uid + 1);
        assert_eq!(game.spells_cast_this_turn, 2);
        assert_library_rng(&game.library_rng, &rng);
        assert_eq!(log.mana_spent, [2.0]);
        assert_eq!(json["draw"]["avg_life_gained"], f64::from(gain));
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 2.0);
    }
}

/// Dying listeners retain the original batch and distinguish creature cards from tokens.
pub(super) fn assert_simultaneous_death_report() {
    for kind in ["Creature", "Artifact", "token"] {
        let token = kind == "token";
        let cards = deck(&[
            row(
                "Dying all-creature listener",
                "{99}",
                "Creature",
                "Whenever another creature you control dies, gain 1 life.",
            ),
            row(
                "Dying nontoken listener",
                "{99}",
                "Creature",
                "Whenever another nontoken creature you control dies, gain 3 life.",
            ),
            row(
                "Victim or producer",
                if token { "{0}" } else { "{99}" },
                if token { "Sorcery" } else { kind },
                if token {
                    "Create a 1/1 creature token."
                } else {
                    ""
                },
            ),
            row(
                "Batch exchange",
                "{1}",
                "Sorcery",
                "Each player exiles all creature cards from their graveyard, then sacrifices all creatures they control, then puts all cards they exiled this way onto the battlefield.",
            ),
            row("Funding land", "", "Basic Land", "{T}: Add {C}."),
        ]);
        let mut game = state(if token { vec![2, 3] } else { vec![3] }, vec![]);
        put_permanents(&cards, &mut game, &[0, 1]);
        if !token {
            put_permanents(&cards, &mut game, &[2]);
        }
        put_permanents(&cards, &mut game, &[4]);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        let gain = if token {
            5
        } else if kind == "Creature" {
            8
        } else {
            4
        };
        assert_eq!(game.life, 20 + gain);
        assert_eq!(game.life_gained, gain as u32);
        assert_eq!(
            game.graveyard,
            if token {
                vec![CardIdx(2), CardIdx(0), CardIdx(1), CardIdx(3)]
            } else if kind == "Creature" {
                vec![CardIdx(0), CardIdx(1), CardIdx(2), CardIdx(3)]
            } else {
                vec![CardIdx(0), CardIdx(1), CardIdx(3)]
            }
        );
        assert_eq!(
            game.battlefield.len(),
            if kind == "Artifact" { 2 } else { 1 }
        );
        assert!(
            game.battlefield
                .iter()
                .all(|permanent| permanent.card != CardRef::Deck(CardIdx(0))
                    && permanent.card != CardRef::Deck(CardIdx(1)))
        );
        assert_eq!(game.next_uid, 4);
        assert!(game.hand.is_empty() && game.library.is_empty() && game.exile.is_empty());
        assert_library_rng(&game.library_rng, &rng);
        assert_eq!(log.mana_spent, [1.0]);
        assert_eq!(log.life_gained, gain as u32);
        assert_eq!(json["draw"]["avg_life_gained"], f64::from(gain));
        assert_eq!(
            json["graveyard"]["avg_size_by_turn"]["1"],
            game.graveyard.len() as f64
        );
        assert_eq!(json["outcomes"]["finished_games"], 1);
        assert_eq!(json["outcomes"]["player_loss_games"], 0);
    }
}

/// Raid uses an actual attack, while each source keeps its own trigger and activation limits.
pub(super) fn assert_condition_usage_report() {
    for live in [true, false] {
        let cards = deck(&[
            row(
                "Conditional end reward",
                "{99}",
                "Artifact",
                "At the beginning of your end step, if you attacked this turn, gain 3 life.",
            ),
            row("Actual attacker", "{99}", "Creature", ""),
        ]);
        let mut game = state(vec![], vec![]);
        put_permanents(&cards, &mut game, &[0, 1]);
        game.opponents.seats[0].active = live;
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(game.attacked_this_turn, live);
        assert_eq!(game.life, if live { 23 } else { 20 });
        assert_eq!(game.opponents.seats[0].life, 20);
        assert_eq!(log.attack_power, [2]);
        assert_eq!(
            json["draw"]["avg_life_gained"],
            if live { 3.0 } else { 0.0 }
        );
        assert_eq!(json["combat"]["attack_power_avg_by_turn"]["1"], 2.0);
    }
    assert_independent_usage_ledgers();
}

/// Two real entries cannot reuse a bounded trigger or block a separate paid activation.
fn assert_independent_usage_ledgers() {
    let hybrid = "Whenever another creature you control enters, gain 1 life. This ability triggers only once each turn.\n{1}, {T}: Gain 2 life. Activate only once each turn.";
    for sources in [0, 1, 2] {
        let mut rows = vec![
            row("First hybrid", "{99}", "Artifact", hybrid),
            row("Second hybrid", "{99}", "Artifact", hybrid),
            row("First entry", "{0}", "Creature", ""),
            row("Second entry", "{0}", "Creature", ""),
            row("Next draw", "{99}", "Sorcery", ""),
        ];
        rows.extend((0..sources).map(|index| {
            row(
                &format!("Funding {index}"),
                "",
                "Basic Land",
                "{T}: Add {C}.",
            )
        }));
        let cards = deck(&rows);
        let mut game = state(vec![2, 3], vec![4]);
        put_permanents(&cards, &mut game, &[0, 1]);
        for index in 5..5 + sources {
            put_permanents(&cards, &mut game, &[index]);
        }
        let rng = game.library_rng.clone();
        let (first, log) = execute_turns(&cards, game.clone(), 1, vec![]);
        assert_eq!(first.triggered_this_turn.len(), 2);
        assert!(first.triggered_this_turn.contains(&(1, 0)));
        assert!(first.triggered_this_turn.contains(&(2, 0)));
        assert_eq!(first.activated_this_turn.len(), sources as usize);
        for uid in 1..=sources {
            assert!(first.activated_this_turn.contains(&(uid, 1)));
        }
        assert_eq!(first.life_gained, 2 + 2 * sources);
        assert_eq!(first.hand, Vec::<CardIdx>::new());
        assert_eq!(first.library, [CardIdx(4)]);
        assert_library_rng(&first.library_rng, &rng);
        let report = report_log(&cards, &log, 1);
        assert_eq!(
            report.draw.avg_life_gained,
            Some(f64::from(2 + 2 * sources))
        );
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], 0.0);
        let (second, log) = execute_turns(&cards, game, 2, vec![]);
        assert!(second.triggered_this_turn.is_empty());
        assert_eq!(second.activated_this_turn.len(), sources as usize);
        assert_eq!(second.life_gained, 2 + 4 * sources);
        assert_eq!(second.hand, [CardIdx(4)]);
        assert_library_rng(&second.library_rng, &rng);
        let report = report_log(&cards, &log, 2);
        assert_eq!(
            report.draw.avg_life_gained,
            Some(f64::from(2 + 4 * sources))
        );
        assert_eq!(report.outcomes.finished_games, 1);
    }
}
