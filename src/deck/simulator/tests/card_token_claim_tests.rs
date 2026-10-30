//! Token clause map. Whole-card claims remain zero.
//!
//! `edhrec_top250`: `assert_miirym_generic_body` checks another Dragon's entry,
//! rejects self and non-Dragon entry, and measures one generic 2/2 body.
//! This does not certify a copy, Dragon token characteristics, or legendary rules.
//! No supported producer makes a Dragon token, so the isolated nontoken event
//! boundary remains a gap. Its typed filter is checked without claiming that gap.
//! `edhrec_top250`: `assert_treasure_bank_events` places live entry and death
//! observers beside Plunderer bank creation and payment. Physical creature tokens
//! fire both observers; a bank balance creates and sacrifices no physical object.
//! `lathril_selected`: `assert_elite_condition_inert` checks only safety for the
//! unsupported another-Elf condition. It supplies no positive pinned execution claim.

use crate::db::CardRow;
use crate::deck::simulator::card_event_claim_tests::assert_clause_source;
use crate::deck::simulator::card_runtime_tests::{attempt_index, renamed};
use crate::deck::simulator::card_test_support::{main_phase_state as state, named_row};
use crate::deck::simulator::game::{CardRef, ManaPool, TokenKind, new_perm_with};
use crate::deck::simulator::game_effects::resolve_sacrifice_uid;
use crate::deck::simulator::model::{CardIdx, SimEventSubject, SimTrigger};
use crate::deck::simulator::permanent_core_report_tests::{execute_turns, report_log};
use crate::deck::simulator::scenario_test_support::{deck, row};

/// The supported Dragon entry creates one generic body, never a copy certificate.
pub(super) fn assert_miirym_generic_body(source: &CardRow) {
    crate::deck::simulator::card_test_support::pinned_revision(source);
    for source in renamed(source) {
        assert_clause_source(&source, "Whenever another nontoken Dragon", "compiler_only");
        for (kind, matching) in [("Creature — Dragon", true), ("Creature — Elf", false)] {
            let mut entering = row("Entering body", "{0}", kind, "");
            entering.power = Some("7".into());
            entering.toughness = Some("7".into());
            let cards = deck(&[source.clone(), entering]);
            let ability = cards.cards[0]
                .unlocked_abilities(0)
                .find(|ability| ability.trigger == SimTrigger::Enters)
                .expect("entry filter lowers");
            assert_eq!(ability.event_subject, SimEventSubject::Another);
            assert!(ability.event_filter.nontoken);
            assert_eq!(ability.event_filter.subtypes, ["dragon"]);
            let mut game = state(vec![1], vec![]);
            game.next_uid = 100;
            game.battlefield
                .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
            assert_eq!(
                attempt_index(&cards, &mut game, &mut ManaPool::default(), CardIdx(1)),
                Some(0)
            );
            assert_eq!(game.battlefield.len(), 2 + usize::from(matching));
            assert_eq!(game.battlefield[0].uid, 10);
            assert_eq!(game.battlefield[1].uid, 101);
            assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
            assert!(game.hand.is_empty() && game.library.is_empty() && game.graveyard.is_empty());
            assert_eq!(game.life, 20);
            assert_eq!(game.treasure_bank, 0);
            if matching {
                let token = &game.battlefield[2];
                assert_eq!(token.uid, 102);
                assert_eq!(token.card, CardRef::Token(TokenKind::Creature));
                assert!(token.summoning_sick && !token.tapped);
                assert_eq!(token.entered_turn, 1);
                let token_card = crate::deck::simulator::game::card_of(&cards, token);
                assert_eq!(token_card.printed_power, Some(2));
                assert_eq!(token_card.printed_toughness, Some(2));
                assert!(!token_card.is_legendary);
                assert!(!token_card.subtypes.iter().any(|kind| kind == "dragon"));
            }
            let (_, log) = execute_turns(&cards, game, 1, vec![]);
            let report = report_log(&cards, &log, 1);
            assert_eq!(
                report.creatures_by_turn[&1],
                if matching { 3.0 } else { 2.0 }
            );
            assert_eq!(report.combat.attack_power_avg_by_turn[&1], 6.0);
            assert_eq!(report.outcomes.finished_games, 1);
        }
        assert_miirym_self_entry(&source);
    }
    let mut changed = source.clone();
    changed.oracle_text = changed.oracle_text.replace("Dragon", "Elf");
    let cards = deck(&[
        changed,
        row("Dragon control", "{0}", "Creature — Dragon", ""),
    ]);
    let mut game = state(vec![1], vec![]);
    game.next_uid = 100;
    game.battlefield
        .push(new_perm_with(10, &cards, CardIdx(0), 0, false));
    assert_eq!(
        attempt_index(&cards, &mut game, &mut ManaPool::default(), CardIdx(1)),
        Some(0)
    );
    assert_eq!(game.battlefield.len(), 2);
    assert_eq!(game.next_uid, 101);
}

/// Casting the Dragon source itself cannot satisfy its another-permanent restriction.
fn assert_miirym_self_entry(source: &CardRow) {
    let cards = deck(std::slice::from_ref(source));
    let mut game = state(vec![0], vec![]);
    game.next_uid = 100;
    let mut pool = ManaPool {
        fixed: [0, 1, 0, 1, 1],
        colorless: 3,
        ..Default::default()
    };
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        Some(6)
    );
    assert_eq!(pool, ManaPool::default());
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].uid, 101);
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(game.next_uid, 101);
    assert!(game.hand.is_empty() && game.graveyard.is_empty());
}

/// Live supported observers distinguish bank balances from physical token events.
pub(super) fn assert_treasure_bank_events(source: &CardRow) {
    crate::deck::simulator::card_test_support::pinned_revision(source);
    assert_clause_source(source, "Whenever another creature", "compiler_only");
    let entry = row(
        "Entry observer",
        "{0}",
        "Enchantment",
        "Whenever a permanent you control enters, gain 1 life.",
    );
    let death = row(
        "Death observer",
        "{0}",
        "Enchantment",
        "Whenever a permanent you control is put into a graveyard from the battlefield, gain 2 life.",
    );
    assert_clause_source(&entry, "Whenever a permanent", "supported");
    assert_clause_source(&death, "Whenever a permanent", "supported");
    let cards = deck(&[
        source.clone(),
        entry,
        death,
        row("Physical victim", "{0}", "Creature", ""),
        row("Bank-funded spell", "{1}", "Sorcery", ""),
        row(
            "Physical token spell",
            "{0}",
            "Sorcery",
            "Create a 1/1 Elf creature token.",
        ),
    ]);
    let mut game = state(vec![4, 5], vec![]);
    game.next_uid = 100;
    for index in 0..4 {
        game.battlefield
            .push(new_perm_with(10 + index, &cards, CardIdx(index), 0, false));
    }
    resolve_sacrifice_uid(&cards, &mut game, 1, 13);
    assert_eq!(game.life, 22);
    assert_eq!(game.life_gained, 2);
    assert_eq!(game.treasure_bank, 1);
    assert_eq!(game.next_uid, 100);
    assert_eq!(
        game.battlefield
            .iter()
            .map(|perm| perm.uid)
            .collect::<Vec<_>>(),
        [10, 11, 12]
    );
    assert_eq!(game.graveyard, [CardIdx(3)]);
    let mut pool = ManaPool {
        treasures: std::mem::take(&mut game.treasure_bank),
        ..Default::default()
    };
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(4)),
        Some(1)
    );
    assert_eq!(pool, ManaPool::default());
    assert_eq!(game.treasure_bank, 0);
    assert_eq!(game.life, 22);
    assert_eq!(game.life_gained, 2);
    assert_eq!(game.next_uid, 100);
    assert_eq!(game.battlefield.len(), 3);
    assert_eq!(game.graveyard, [CardIdx(3), CardIdx(4)]);
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(5)),
        Some(0)
    );
    assert_eq!(game.life, 23);
    assert_eq!(game.life_gained, 3);
    assert_eq!(game.treasure_bank, 0);
    assert_eq!(game.battlefield.len(), 4);
    assert_eq!(game.battlefield[3].uid, 101);
    assert_eq!(
        game.battlefield[3].card,
        CardRef::Token(TokenKind::Creature)
    );
    resolve_sacrifice_uid(&cards, &mut game, 1, 101);
    assert_eq!(game.life, 25);
    assert_eq!(game.life_gained, 5);
    assert_eq!(game.treasure_bank, 1);
    assert_eq!(game.battlefield.len(), 3);
    assert_eq!(game.next_uid, 101);
    assert_eq!(game.graveyard, [CardIdx(3), CardIdx(4), CardIdx(5)]);
    assert!(game.hand.is_empty() && game.library.is_empty() && game.exile.is_empty());
    assert_eq!(game.seen, 0);
    let (game, log) = execute_turns(&cards, game, 1, vec![]);
    assert_eq!(game.treasure_bank, 1);
    let report = report_log(&cards, &log, 1);
    assert_eq!(report.draw.avg_life_gained, Some(5.0));
    assert_eq!(report.graveyard.avg_size_by_turn[&1], 3.0);
    assert_eq!(report.creatures_by_turn[&1], 1.0);
    assert_eq!(report.outcomes.finished_games, 1);
}

/// An unsupported another-Elf condition cannot become an unconditional token reward.
pub(super) fn assert_elite_condition_inert(source: &CardRow) {
    crate::deck::simulator::card_test_support::pinned_revision(source);
    for source in renamed(source) {
        assert_clause_source(&source, "if you control another Elf", "unsupported");
        for another_elf in [true, false] {
            let cards = deck(&[
                source.clone(),
                row("Other Elf", "{0}", "Creature — Elf", ""),
            ]);
            assert!(
                !cards.cards[0]
                    .unlocked_abilities(0)
                    .any(|ability| ability.trigger == SimTrigger::Enters)
            );
            let mut game = state(vec![0], vec![]);
            game.next_uid = 100;
            if another_elf {
                game.battlefield
                    .push(new_perm_with(10, &cards, CardIdx(1), 0, false));
            }
            let mut pool = ManaPool {
                fixed: [0, 0, 0, 0, 1],
                colorless: 1,
                ..Default::default()
            };
            assert_eq!(
                attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
                Some(2)
            );
            assert_eq!(pool, ManaPool::default());
            assert_eq!(game.battlefield.len(), 1 + usize::from(another_elf));
            assert_eq!(game.next_uid, 101);
            assert_eq!(
                game.battlefield.last().expect("cast Elite entered").uid,
                101
            );
            assert_eq!(
                game.battlefield.last().expect("cast Elite entered").card,
                CardRef::Deck(CardIdx(0))
            );
            assert!(
                game.battlefield
                    .iter()
                    .all(|perm| !matches!(perm.card, CardRef::Token(_)))
            );
            assert!(game.hand.is_empty() && game.graveyard.is_empty());
            assert_eq!(game.life, 20);
            assert_eq!(game.seen, 0);
            let (_, log) = execute_turns(&cards, game, 1, vec![]);
            assert_eq!(
                report_log(&cards, &log, 1).creatures_by_turn[&1],
                1.0 + f64::from(u32::from(another_elf))
            );
        }
    }
}

/// Check the pinned Dragon entry while retaining its generic-body limit.
#[test]
fn pinned_miirym_generic_body_action_and_report() {
    assert_miirym_generic_body(&named_row("Miirym, Sentinel Wyrm"));
}

/// Check bank events beside supported physical-token observers.
#[test]
fn pinned_plunderer_bank_event_action_and_report() {
    assert_treasure_bank_events(&named_row("Pitiless Plunderer"));
}

/// Check the wider Lathril source's unsupported-condition safety only.
#[test]
fn pinned_elite_condition_safety_action_and_report() {
    assert_elite_condition_inert(&named_row("Dwynen's Elite"));
}
