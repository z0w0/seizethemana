//! Synthetic cleanup policy checks. MDFC hidden-face eligibility belongs to its owner.
//! Cleanup is a hand-to-graveyard move, not a draw or a library-to-graveyard event.

use crate::deck::simulator::card_test_support::assert_library_rng;
use crate::deck::simulator::game_effects::resolve_sacrifice_uid;
use crate::deck::simulator::library_core_report_tests::{executed_report, put_permanents};
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// Highest mana values leave first unless a live, face-up supported source lifts the limit.
pub(super) fn assert_cleanup_policy_report() {
    for source_state in ["absent", "live", "removed", "face_down"] {
        let mut rows = (0..9)
            .map(|index| {
                row(
                    &format!("Cleanup card {index}"),
                    &format!("{{{}}}", 10 + index),
                    "Sorcery",
                    "",
                )
            })
            .collect::<Vec<_>>();
        rows[8].oracle_text = "When this card is put into your graveyard from your library, each opponent loses 3 life and you gain 3 life.".into();
        rows.push(row("Cleanup library control", "{99}", "Sorcery", ""));
        rows.push(row(
            "Synthetic hand-limit source",
            "{99}",
            "Enchantment",
            "You have no maximum hand size.",
        ));
        let cards = deck(&rows);
        assert!(cards.cards[8].library_graveyard_trigger.is_some());
        assert!(cards.cards[10].flags.no_max_hand_size);
        let mut game = state((0..9).collect(), vec![9]);
        if source_state != "absent" {
            put_permanents(&cards, &mut game, &[10]);
        }
        if source_state == "removed" {
            resolve_sacrifice_uid(&cards, &mut game, 1, 1);
        }
        if source_state == "face_down" {
            game.battlefield[0].face_down = true;
        }
        let rng = game.library_rng.clone();
        let (game, log, report) = executed_report(&cards, game, 1, vec![]);
        let unlimited = source_state == "live";
        assert_eq!(
            game.hand,
            if unlimited {
                (0..9).map(CardIdx).collect::<Vec<_>>()
            } else {
                (0..7).map(CardIdx).collect::<Vec<_>>()
            }
        );
        let mut graveyard = if source_state == "removed" {
            vec![CardIdx(10)]
        } else {
            vec![]
        };
        if !unlimited {
            graveyard.extend([CardIdx(8), CardIdx(7)]);
        }
        assert_eq!(game.graveyard, graveyard);
        assert_eq!(game.library, [CardIdx(9)]);
        assert!(game.exile.is_empty());
        assert_eq!(game.seen, 0);
        assert_eq!(game.life, 20);
        assert_eq!(game.life_gained, 0);
        assert_eq!(game.opponent_life_lost, 0);
        assert_eq!(game.treasure_bank, 0);
        assert_library_rng(&game.library_rng, &rng);
        assert_eq!(log.cards_seen, [0]);
        assert_eq!(log.graveyard_size, [graveyard.len() as u32]);
        if !unlimited {
            assert_eq!(log.card_first_graveyard[&8], 1);
            assert_eq!(log.card_first_graveyard[&7], 1);
            assert_eq!(game.graveyard_generations[&CardIdx(8)], 1);
            assert_eq!(game.graveyard_generations[&CardIdx(7)], 1);
        } else {
            assert!(!log.card_first_graveyard.contains_key(&8));
            assert!(!log.card_first_graveyard.contains_key(&7));
        }
        assert_eq!(
            report["graveyard"]["avg_size_by_turn"]["1"],
            graveyard.len() as f64
        );
        assert_eq!(report["velocity"]["avg_cards_seen"]["1"], 0.0);
        assert_eq!(report["velocity"]["self_milled_by_turn"]["1"], 0.0);
        assert_eq!(report["draw"]["avg_life_gained"], 0.0);
        assert_eq!(
            report["win_conditions"]["opponent_life_loss_by_turn"]["1"],
            0.0
        );
        if matches!(source_state, "live" | "face_down") {
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].uid, 1);
            assert_eq!(game.battlefield[0].face_down, source_state == "face_down");
        } else {
            assert!(game.battlefield.is_empty());
        }
    }
}
