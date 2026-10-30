//! Clause map for pinned permanent events. Whole-card claims remain zero.
//!
//! `edhrec_top250`: `assert_ring_burden` covers tap payment, source burden,
//! draw scaling, and upkeep life timing. Protection and indestructible are excluded.
//! `draw_engines, edhrec_top250`: `assert_remora_upkeep` covers real mana,
//! increasing upkeep, unpaid sacrifice, and main-phase mana. The tax model is excluded.
//! `edhrec_top250`: `assert_solemn_death` covers self-death draw only;
//! `assert_plunderer_death` covers another creature's death and the Treasure bank;
//! `assert_reclamation_death` covers creature-death life and draw.
//! These are controlled actions and executed reports, not whole-card certificates.

use crate::db::CardRow;
use crate::deck::simulator::card_runtime_tests::renamed;
use crate::deck::simulator::card_test_support::{main_phase_state as state, named_row};
use crate::deck::simulator::game::{CardRef, ManaPool, new_perm_with};
use crate::deck::simulator::game_effects::activation::{pick_best_activation, resolve_activation};
use crate::deck::simulator::game_effects::resolve_sacrifice_uid;
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::oracle_lower::parse_sim_card;
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::scenario_test_support::{deck, row};

/// Bind a clause to its exact input, source span, and declared compiler status.
pub(super) fn assert_clause_source(source: &CardRow, wording: &str, status: &str) {
    let card = parse_sim_card(source);
    assert_eq!(
        card.source_hash,
        crate::deck::simulator::support::card_source_hash(source)
    );
    let evidence = card
        .support
        .iter()
        .find(|entry| entry.source.contains(wording))
        .expect("the clause has source evidence");
    let span = evidence.span.expect("printed clause has a source span");
    assert_eq!(evidence.source, source.oracle_text[span.start..span.end]);
    assert_eq!(
        serde_json::to_value(&evidence.status).expect("status serializes")["status"],
        status
    );
}

/// Tap payment adds burden to the source and life loss waits for its next upkeep.
pub(super) fn assert_ring_burden(source: &CardRow) {
    crate::deck::simulator::card_test_support::pinned_revision(source);
    for source in renamed(source) {
        assert_clause_source(&source, "Put a burden counter", "supported");
        assert_clause_source(&source, "At the beginning of your upkeep", "supported");
        assert_ring_tap_gate(&source);
        let mut rows = vec![source.clone()];
        rows.extend((0..4).map(|_| row("Wastes", "", "Basic Land", "{T}: Add {C}.")));
        rows.extend((0..12).map(|index| row(&format!("Library {index}"), "{99}", "Sorcery", "")));
        let cards = deck(&rows);
        let mut game = state(vec![0], (5..17).collect());
        game.next_uid = 100;
        for index in 1..5 {
            game.battlefield
                .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
        }
        let (game, log) = execute_turns(&cards, game, 2, vec![]);
        let ring = game
            .battlefield
            .iter()
            .find(|perm| perm.card == CardRef::Deck(CardIdx(0)))
            .expect("cast Ring remains on the battlefield");
        assert_eq!(ring.uid, 101);
        assert!(ring.tapped);
        assert_eq!(ring.counters.burden, 2);
        assert_eq!(ring.counters.charge, 0);
        assert_eq!(ring.counters.plus1, 0);
        assert_eq!(game.life, 19);
        assert_eq!(game.life_paid, 1);
        assert_eq!(game.seen, 4);
        assert_eq!(
            game.hand,
            [CardIdx(16), CardIdx(15), CardIdx(14), CardIdx(13)]
        );
        assert_eq!(game.library, (5..13).map(CardIdx).collect::<Vec<_>>());
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.battlefield.len(), 5);
        assert!(
            game.battlefield
                .iter()
                .filter(|perm| perm.uid != ring.uid)
                .all(|perm| perm.counters.burden == 0)
        );
        assert_eq!(log.cards_seen, [1, 4]);
        assert_eq!(log.mana_spent, [4.0, 0.0]);
        assert_eq!(log.mana_remaining, [0.0, 4.0]);
        assert_eq!(log.repeatable_sources_online, [1, 1]);
        let report = report_log(&cards, &log, 2);
        assert_eq!(report.outcomes.finished_games, 1);
        assert_eq!(report.draw.avg_life_paid, Some(1.0));
        assert_eq!(report.draw.avg_life_funded_draws, Some(0.0));
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], 4.0);
        assert_eq!(report.mana.avg_unused_by_turn[&2], 4.0);
        let support = report
            .card_support
            .iter()
            .find(|card| card.name == source.name)
            .expect("report includes the source");
        assert_eq!(
            support.source_hash.as_deref(),
            Some(cards.cards[0].source_hash.as_str())
        );
        assert!(
            support
                .abilities
                .iter()
                .any(|entry| entry.source.contains("Put a burden counter")
                    && entry.status == "supported")
        );
    }
}

/// A tapped source cannot add counters, draw, or pay life a second time.
fn assert_ring_tap_gate(source: &CardRow) {
    let cards = deck(&[
        source.clone(),
        row("Unrelated artifact", "{0}", "Artifact", ""),
        row("First draw", "{99}", "Sorcery", ""),
        row("Second draw", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![], vec![2, 3]);
    game.battlefield
        .push(new_perm_with(8, &cards, CardIdx(1), 0, false));
    game.battlefield[0].counters.burden = 7;
    game.battlefield
        .push(new_perm_with(9, &cards, CardIdx(0), 0, true));
    let mut pool = ManaPool::default();
    assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
    assert_eq!(game.library, [CardIdx(2), CardIdx(3)]);
    game.battlefield[1].tapped = false;
    let activation =
        pick_best_activation(&cards, &game, &pool, 1).expect("untapped Ring can pay its tap cost");
    assert_eq!(activation.uid, 9);
    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
    assert_eq!(game.battlefield[0].counters.burden, 7);
    assert_eq!(game.battlefield[1].counters.burden, 1);
    assert!(game.battlefield[1].tapped);
    assert_eq!(game.hand, [CardIdx(3)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_paid, 0);
    assert_eq!(pool, ManaPool::default());
    assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
    assert_eq!(game.hand, [CardIdx(3)]);
    assert_eq!(game.battlefield[1].counters.burden, 1);
}

/// Real sources pay ages one through three; age four sacrifices the unpaid source.
pub(super) fn assert_remora_upkeep(source: &CardRow) {
    crate::deck::simulator::card_test_support::pinned_revision(source);
    for source in renamed(source) {
        assert_clause_source(&source, "Whenever an opponent casts", "approximated");
        let mut rows = vec![
            source.clone(),
            row("Main-phase test", "{3}", "Sorcery", "Gain 7 life."),
        ];
        rows.extend((0..3).map(|_| row("Island", "", "Basic Land — Island", "{T}: Add {U}.")));
        rows.extend((0..6).map(|index| row(&format!("Library {index}"), "{99}", "Sorcery", "")));
        let cards = deck(&rows);
        assert_eq!(
            cards.cards[0]
                .cumulative_upkeep
                .expect("mana upkeep lowers")
                .generic,
            1
        );
        let mut game = state(vec![1], (5..11).collect());
        game.next_uid = 100;
        game.battlefield
            .push(new_perm_with(20, &cards, CardIdx(0), 0, false));
        for index in 2..5 {
            game.battlefield
                .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
        }
        let mut payment = game.clone();
        for age in 1..=3 {
            for perm in &mut payment.battlefield {
                perm.tapped = false;
            }
            crate::deck::simulator::opponents::pay_cumulative_upkeep(&cards, &mut payment, age);
            assert_eq!(
                payment
                    .battlefield
                    .iter()
                    .filter(|perm| perm.tapped)
                    .count(),
                age as usize
            );
            assert_eq!(payment.opponents.upkeep_ages[&20], age);
            assert!(payment.graveyard.is_empty());
        }
        let (game, log) = execute_turns(&cards, game, 4, vec![]);
        assert_eq!(game.opponents.upkeep_ages[&20], 4);
        assert_eq!(game.battlefield.len(), 3);
        assert!(
            game.battlefield
                .iter()
                .all(|perm| perm.tapped && perm.uid != 20)
        );
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert!(game.hand.contains(&CardIdx(1)));
        assert_eq!(game.hand, [CardIdx(1), CardIdx(10), CardIdx(9), CardIdx(8)]);
        assert_eq!(game.seen, 3);
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.treasure_bank, 0);
        assert_eq!(log.mana_available, [2.0, 1.0, 0.0, 0.0]);
        assert_eq!(log.mana_remaining, [2.0, 1.0, 0.0, 0.0]);
        assert_eq!(log.mana_spent, [0.0; 4]);
        let report = report_log(&cards, &log, 4);
        assert_eq!(report.outcomes.finished_games, 1);
        assert_eq!(report.graveyard.avg_size_by_turn[&3], 0.0);
        assert_eq!(report.graveyard.avg_size_by_turn[&4], 1.0);
        for (turn, available) in [(1, 2.0), (2, 1.0), (3, 0.0), (4, 0.0)] {
            assert_eq!(report.mana.avg_available_by_turn[&turn], available);
            assert_eq!(report.mana.avg_unused_by_turn[&turn], available);
        }
    }
}

/// Only the source's own death draws; another permanent's death does not.
pub(super) fn assert_solemn_death(source: &CardRow) {
    crate::deck::simulator::card_test_support::pinned_revision(source);
    for source in renamed(source) {
        assert_clause_source(&source, "When this creature dies", "supported");
        for own_death in [true, false] {
            let cards = deck(&[
                source.clone(),
                row("Other", "{0}", "Creature", ""),
                row("Reward", "{99}", "Sorcery", ""),
            ]);
            let mut game = state(vec![], vec![2]);
            for index in 0..2 {
                game.battlefield
                    .push(new_perm_with(index + 10, &cards, CardIdx(index), 0, false));
            }
            resolve_sacrifice_uid(&cards, &mut game, 1, if own_death { 10 } else { 11 });
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].uid, if own_death { 11 } else { 10 });
            assert_eq!(game.graveyard, [CardIdx(if own_death { 0 } else { 1 })]);
            assert_eq!(game.hand, if own_death { vec![CardIdx(2)] } else { vec![] });
            assert_eq!(
                game.library,
                if own_death { vec![] } else { vec![CardIdx(2)] }
            );
            assert_eq!(game.seen, u32::from(own_death));
            assert_eq!(game.life, 20);
            assert_eq!(game.treasure_bank, 0);
            assert!(game.exile.is_empty());
            let (game, log) = execute_turns(&cards, game, 1, vec![]);
            assert_eq!(game.seen, u32::from(own_death));
            assert_eq!(log.cards_seen, [u32::from(own_death)]);
            assert_eq!(
                report_log(&cards, &log, 1).graveyard.avg_size_by_turn[&1],
                1.0
            );
        }
    }
}

/// Another creature creates one banked Treasure; self and noncreature deaths do not.
pub(super) fn assert_plunderer_death(source: &CardRow) {
    crate::deck::simulator::card_test_support::pinned_revision(source);
    for source in renamed(source) {
        assert_clause_source(&source, "Whenever another creature", "compiler_only");
        for (kind, own_death, count) in [
            ("Creature", false, 1),
            ("Artifact", false, 0),
            ("Creature", true, 0),
        ] {
            let cards = deck(&[source.clone(), row("Victim", "{0}", kind, "")]);
            let mut game = state(vec![], vec![]);
            game.next_uid = 100;
            game.battlefield
                .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
            game.battlefield
                .push(new_perm_with(11, &cards, CardIdx(1), 0, false));
            resolve_sacrifice_uid(&cards, &mut game, 1, if own_death { 10 } else { 11 });
            assert_eq!(game.treasure_bank, count);
            assert_eq!(game.next_uid, 100);
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].uid, if own_death { 11 } else { 10 });
            assert_eq!(game.graveyard, [CardIdx(if own_death { 0 } else { 1 })]);
            assert_eq!(game.life, 20);
            assert_eq!(game.seen, 0);
            assert!(game.hand.is_empty() && game.library.is_empty() && game.exile.is_empty());
            let (game, log) = execute_turns(&cards, game, 1, vec![]);
            assert_eq!(game.treasure_bank, count);
            assert_eq!(log.cards_seen, [0]);
            assert_eq!(
                report_log(&cards, &log, 1).graveyard.avg_size_by_turn[&1],
                1.0
            );
        }
    }
}

/// Creature deaths gain and draw once; noncreature deaths grant neither reward.
pub(super) fn assert_reclamation_death(source: &CardRow) {
    crate::deck::simulator::card_test_support::pinned_revision(source);
    for source in renamed(source) {
        assert_clause_source(
            &source,
            "Whenever a creature you control dies",
            "compiler_only",
        );
        for (kind, rewards) in [("Creature", true), ("Artifact", false)] {
            let cards = deck(&[
                source.clone(),
                row("Victim", "{0}", kind, ""),
                row("Reward", "{99}", "Sorcery", ""),
            ]);
            let mut game = state(vec![], vec![2]);
            game.battlefield
                .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
            game.battlefield
                .push(new_perm_with(11, &cards, CardIdx(1), 0, false));
            resolve_sacrifice_uid(&cards, &mut game, 1, 11);
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].uid, 10);
            assert_eq!(game.graveyard, [CardIdx(1)]);
            assert_eq!(game.hand, if rewards { vec![CardIdx(2)] } else { vec![] });
            assert_eq!(
                game.library,
                if rewards { vec![] } else { vec![CardIdx(2)] }
            );
            assert_eq!(game.life, 20 + i32::from(rewards));
            assert_eq!(game.life_gained, u32::from(rewards));
            assert_eq!(game.seen, u32::from(rewards));
            let (_, log) = execute_turns(&cards, game, 1, vec![]);
            assert_eq!(
                report_log(&cards, &log, 1).draw.avg_life_gained,
                Some(f64::from(u32::from(rewards)))
            );
        }
    }
}

/// Check the pinned Ring clauses without a registry dependency.
#[test]
fn pinned_ring_burden_action_and_report() {
    assert_ring_burden(&named_row("The One Ring"));
}

/// Check the pinned Remora upkeep without relying on opponent tax events.
#[test]
fn pinned_remora_upkeep_action_and_report() {
    assert_remora_upkeep(&named_row("Mystic Remora"));
}

/// Check the pinned self-death reward and its nonmatching death control.
#[test]
fn pinned_solemn_death_action_and_report() {
    assert_solemn_death(&named_row("Solemn Simulacrum"));
}

/// Check the pinned another-creature death filter without retesting payment choices.
#[test]
fn pinned_plunderer_death_action_and_report() {
    assert_plunderer_death(&named_row("Pitiless Plunderer"));
}

/// Check the pinned linked life and draw reward for a creature death.
#[test]
fn pinned_reclamation_death_action_and_report() {
    assert_reclamation_death(&named_row("Moldervine Reclamation"));
}
