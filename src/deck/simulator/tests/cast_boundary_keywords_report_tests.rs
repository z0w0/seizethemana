//! Synthetic keyword controls preserve generic-body and target-policy limits.

use super::*;
use crate::deck::simulator::game_effects::creature_power;
use crate::deck::simulator::model::{SimEffect, SimTrigger};

/// Mobilize creates only the supported generic bodies and sacrifices them at the end step.
fn assert_mobilize_report() {
    for (amount, live) in [(2, true), (2, false), (8, true), (9, true)] {
        let cards = deck(&[
            body(
                "Synthetic mobilize leader",
                "{99}",
                &format!("Mobilize {amount}"),
                1,
            ),
            row(
                "Mobilize death observer",
                "{99}",
                "Enchantment",
                "Whenever a creature you control dies, gain 1 life.",
            ),
        ]);
        assert!(
            cards.cards[0]
                .unlocked_abilities(0)
                .any(|ability| ability.trigger == SimTrigger::Attacks
                    && matches!(ability.effect, SimEffect::Mobilize(count) if count == amount))
        );
        let mut game = state(vec![], vec![]);
        put_permanents(&cards, &mut game, &[0, 1]);
        game.opponents.seats[0].active = live;
        let random = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        let incomplete = amount > 8;
        let rewards = if live && !incomplete { amount } else { 0 };
        assert_eq!(game.life_gained, rewards);
        assert_eq!(game.life, 20 + rewards as i32);
        assert_eq!(game.next_uid, 2 + if live { amount.min(8) } else { 0 });
        assert_eq!(game.battlefield.len(), if incomplete { 10 } else { 2 });
        assert_eq!(game.battlefield[0].tapped, live);
        assert!(
            game.hand.is_empty()
                && game.library.is_empty()
                && game.graveyard.is_empty()
                && game.exile.is_empty()
        );
        assert_library_rng(&game.library_rng, &random);
        let report = report_log(&cards, &log, 1);
        assert_assumption(
            &report,
            "Generic token and target choices also limit mobilize",
        );
        if incomplete {
            assert_eq!(
                log.completion,
                GameCompletion::Incomplete {
                    reason: IncompleteReason::EffectWork {
                        configured_limit: 8,
                        remaining_work: 1
                    },
                    turn: 1,
                    phase: GamePhase::Combat,
                }
            );
            for (offset, token) in game.battlefield[2..].iter().enumerate() {
                assert_eq!(token.card, CardRef::Token(TokenKind::Creature));
                assert_eq!(token.uid, offset as u32 + 3);
                assert!(token.tapped && token.attacking_this_turn && token.sacrifice_at_end);
                assert_eq!(card_of(&cards, token).printed_power, Some(2));
                assert!(card_of(&cards, token).subtypes.is_empty());
            }
            assert_eq!(report.outcomes.finished_games, 0);
            assert_eq!(report.outcomes.incomplete_games, 1);
            assert!(log.turn_observations[0].main);
            assert_eq!(report.phase_observations.main_by_turn[&1], 0.0);
            assert_eq!(report.phase_observations.combat_by_turn[&1], 0.0);
            assert_eq!(report.draw.avg_life_gained, None);
            assert!(report.creatures_by_turn.is_empty());
            assert!(report.combat.attack_power_avg_by_turn.is_empty());
        } else {
            assert_finished_turn(&report, &log);
            assert_eq!(game.battlefield[0].uid, 1);
            assert_eq!(game.battlefield[1].uid, 2);
            assert_eq!(log.attack_power, [1 + if live { amount * 2 } else { 0 }]);
            assert_eq!(log.attackers, [1 + if live { amount } else { 0 }]);
            assert_eq!(report.creatures_by_turn[&1], 1.0);
            assert_eq!(report.draw.avg_life_gained, Some(f64::from(rewards)));
            assert_eq!(
                report.combat.attack_power_avg_by_turn[&1],
                f64::from(log.attack_power[0])
            );
        }
    }
}

/// Amass adds counters to one supported Army and does not preserve the printed Orc subtype.
fn assert_amass_report() {
    for cast_second in [false, true] {
        let cards = deck(&[
            row("Synthetic first amass", "{0}", "Sorcery", "Amass Orcs 2."),
            row("Synthetic second amass", "{0}", "Sorcery", "Amass Orcs 3."),
        ]);
        let game = state(if cast_second { vec![0, 1] } else { vec![0] }, vec![]);
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.battlefield.len(), 1);
        let army = &game.battlefield[0];
        assert_eq!(army.card, CardRef::Token(TokenKind::Army));
        assert_eq!(army.uid, 1);
        assert!(army.army && army.summoning_sick && !army.tapped);
        assert_eq!(army.counters.plus1, if cast_second { 5 } else { 2 });
        assert_eq!(army.counters.charge, 0);
        assert_eq!(
            creature_power(army, &cards),
            if cast_second { 5 } else { 2 }
        );
        assert_eq!(card_of(&cards, army).subtypes, ["army"]);
        assert_eq!(card_of(&cards, army).printed_power, Some(0));
        assert_eq!(game.next_uid, 1);
        assert_eq!(
            game.graveyard,
            if cast_second {
                vec![CardIdx(0), CardIdx(1)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(game.spells_cast_this_turn, if cast_second { 2 } else { 1 });
        assert_eq!(game.life, 20);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(report.creatures_by_turn[&1], 1.0);
        assert_eq!(report.combat.attack_power_avg_by_turn[&1], 0.0);
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], 0.0);
        assert_assumption(
            &report,
            "A recognized keyword is not proof of complete rules coverage",
        );
    }
}

/// Offspring pays its extra cost once and creates a generic body without copied entry text.
fn assert_offspring_report() {
    for mana in 0..=3 {
        let cards = deck(&[
            body(
                "Synthetic offspring body",
                "{1}",
                "Offspring {2}\nWhen this creature enters, gain 1 life.",
                5,
            ),
            row(
                "Offspring payment",
                "{99}",
                "Artifact",
                &format!("{{T}}: Add {}.", "{C}".repeat(mana)),
            ),
        ]);
        assert_eq!(
            cards.cards[0]
                .keyword_abilities
                .offspring
                .expect("offspring cost parsed")
                .total(),
            2
        );
        let mut game = state(vec![0], vec![]);
        if mana > 0 {
            put_permanents(&cards, &mut game, &[1]);
        }
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        let cast = mana > 0;
        let offspring = mana == 3;
        assert_eq!(
            game.battlefield.len(),
            usize::from(cast) * 2 + usize::from(offspring)
        );
        assert_eq!(game.hand, if cast { vec![] } else { vec![CardIdx(0)] });
        assert_eq!(game.spells_cast_this_turn, u32::from(cast));
        assert_eq!(game.life_gained, u32::from(cast));
        assert_eq!(game.life, 20 + i32::from(cast));
        assert_eq!(game.life_paid, 0);
        assert!(game.graveyard.is_empty() && game.library.is_empty() && game.exile.is_empty());
        if offspring {
            let token = game.battlefield.last().expect("offspring body entered");
            assert_eq!(token.uid, 3);
            assert_eq!(token.card, CardRef::Token(TokenKind::Creature));
            assert!(token.summoning_sick && !token.tapped);
            let data = card_of(&cards, token);
            assert_eq!(data.printed_power, Some(2));
            assert!(data.subtypes.is_empty());
            assert_eq!(data.unlocked_abilities(0).count(), 0);
        }
        let spent = if offspring { 3.0 } else { f64::from(cast) };
        assert_eq!(log.mana_spent, [spent]);
        assert_eq!(log.mana_remaining, [mana as f64 - spent]);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(
            report.creatures_by_turn[&1],
            f64::from(cast) + f64::from(offspring)
        );
        assert_eq!(report.draw.avg_life_gained, Some(f64::from(cast)));
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], spent);
        assert_assumption(
            &report,
            "Printed token colors, subtypes, sizes, and abilities are not generally preserved",
        );
    }
}

/// Explore uses the actual top card and gives its nonland counter to the chosen best body.
fn assert_explore_report() {
    for (land, target) in [(true, true), (false, true), (false, false)] {
        let cards = deck(&[
            row(
                "Synthetic explore observer",
                "{99}",
                "Enchantment",
                "At the beginning of your first main phase, explore.",
            ),
            body("Chosen explore body", "{99}", "", 5),
            row(
                "Physical explore top",
                if land { "" } else { "{99}" },
                if land { "Land" } else { "Sorcery" },
                "",
            ),
            row("Library remainder", "{99}", "Sorcery", ""),
            body("Unchosen explore body", "{99}", "", 1),
        ]);
        assert!(
            cards.cards[0]
                .unlocked_abilities(0)
                .any(|ability| matches!(ability.effect, SimEffect::Explore))
        );
        let mut game = state(vec![], vec![3, 2]);
        put_permanents(&cards, &mut game, if target { &[0, 1, 4] } else { &[0] });
        let random = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.seen, u32::from(land));
        assert_eq!(
            game.library,
            if land {
                vec![CardIdx(3)]
            } else {
                vec![CardIdx(3), CardIdx(2)]
            }
        );
        assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
        if target {
            assert_eq!(game.battlefield[1].counters.plus1, u32::from(!land));
            assert_eq!(game.battlefield[1].uid, 2);
            assert_eq!(game.battlefield[2].counters.plus1, 0);
            assert_eq!(game.battlefield[2].card, CardRef::Deck(CardIdx(4)));
        }
        assert_eq!(game.life, 20);
        assert_library_rng(&game.library_rng, &random);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(report.velocity.avg_cards_seen[&1], f64::from(land));
        assert_eq!(
            report.velocity.library_remaining_by_turn[&1],
            if land { 1.0 } else { 2.0 }
        );
        assert_eq!(
            report.combat.attack_power_avg_by_turn[&1],
            if target { 6.0 + f64::from(!land) } else { 0.0 }
        );
        assert_eq!(report.creatures_by_turn[&1], f64::from(target) * 2.0);
    }
}

/// Connive two draws and discards two real cards but adds at most one chosen-body counter.
fn assert_connive_report() {
    for (land, target) in [(false, true), (true, true), (false, false)] {
        let cards = deck(&[
            row(
                "Synthetic connive observer",
                "{99}",
                "Enchantment",
                "At the beginning of your first main phase, connive 2.",
            ),
            body("Chosen connive body", "{99}", "", 5),
            row(
                "Oldest discard",
                if land { "" } else { "{99}" },
                if land { "Land" } else { "Artifact" },
                "",
            ),
            row(
                "Second discard",
                if land { "" } else { "{99}" },
                if land { "Land" } else { "Artifact" },
                "",
            ),
            row("First physical draw", "{99}", "Sorcery", ""),
            row("Second physical draw", "{99}", "Sorcery", ""),
            row("Library remainder", "{99}", "Sorcery", ""),
            body("Unchosen connive body", "{99}", "", 1),
        ]);
        assert!(
            cards.cards[0]
                .unlocked_abilities(0)
                .any(|ability| matches!(ability.effect, SimEffect::Connive(2)))
        );
        let mut game = state(vec![2, 3], vec![6, 5, 4]);
        put_permanents(&cards, &mut game, if target { &[0, 1, 7] } else { &[0] });
        let random = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.hand, [CardIdx(4), CardIdx(5)]);
        assert_eq!(game.graveyard, [CardIdx(2), CardIdx(3)]);
        assert_eq!(game.library, [CardIdx(6)]);
        assert!(game.exile.is_empty());
        assert_eq!(game.seen, 2);
        assert_eq!(game.life, 20);
        if target {
            assert_eq!(game.battlefield[1].counters.plus1, u32::from(!land));
            assert_eq!(game.battlefield[2].counters.plus1, 0);
            assert_eq!(game.battlefield[2].card, CardRef::Deck(CardIdx(7)));
        }
        assert_library_rng(&game.library_rng, &random);
        assert_eq!(log.cards_seen, [2]);
        assert_eq!(log.graveyard_size, [2]);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(report.velocity.avg_cards_seen[&1], 2.0);
        assert_eq!(report.velocity.library_remaining_by_turn[&1], 1.0);
        assert_eq!(
            report.combat.attack_power_avg_by_turn[&1],
            if target { 6.0 + f64::from(!land) } else { 0.0 }
        );
        assert_assumption(
            &report,
            "Generic token and target choices also limit mobilize, amass, offspring, explore, connive",
        );
    }
}

/// The pinned mixed Bowmasters clause is not evidence for the synthetic amass path.
fn assert_bowmasters_keyword_safety_report() {
    let source = named_row("Orcish Bowmasters");
    pinned_revision(&source);
    let cards = deck(&[
        source.clone(),
        row("Bowmasters payment", "{99}", "Artifact", "{T}: Add {C}{B}."),
    ]);
    let mut game = state(vec![0], vec![]);
    put_permanents(&cards, &mut game, &[1]);
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert_eq!(game.battlefield.len(), 2);
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(0)));
    assert!(game.battlefield.iter().all(|permanent| !permanent.army));
    assert_eq!(game.opponent_life_lost, 0);
    assert_eq!(game.opponents.seats[0].life, 20);
    assert_eq!(log.mana_spent, [2.0]);
    let report = report_log(&cards, &log, 1);
    assert_finished_turn(&report, &log);
    assert_eq!(report.creatures_by_turn[&1], 1.0);
    let json = serde_json::to_value(report).expect("report serializes");
    crate::deck::simulator::library_core_report_tests::assert_report_clause(
        &json,
        &source,
        "amass Orcs",
        "unsupported",
        None,
    );
}

/// Execute synthetic supported keyword paths without promoting recognition to card support.
pub(super) fn assert_keyword_limits_report() {
    assert_mobilize_report();
    assert_amass_report();
    assert_offspring_report();
    assert_explore_report();
    assert_connive_report();
    assert_bowmasters_keyword_safety_report();
}
