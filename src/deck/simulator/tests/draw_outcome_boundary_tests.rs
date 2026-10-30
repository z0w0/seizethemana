//! Actual draws and dredge replacements keep separate life-funded draw counts.

use super::card_test_support::assert_library_rng;
use super::game_effects::{CardZone, move_to_graveyard};
use super::library_core_report_tests::executed_report;
use super::model::CardIdx;
use super::scenario_test_support::{deck, row, state};

/// A paid cycling action can complete through dredge without recording a draw.
#[test]
fn life_cycling_counts_draws_but_not_dredge_replacements() {
    let cards = deck(&[
        row(
            "Synthetic life cycler",
            "{99}",
            "Creature",
            "Cycling—Pay 2 life. (Pay 2 life, Discard this card: Draw a card.)",
        ),
        row("Synthetic dredger", "{99}", "Sorcery", "Dredge 3"),
        row("First library card", "{99}", "Sorcery", ""),
        row("Second library card", "{99}", "Sorcery", ""),
        row("Third library card", "{99}", "Sorcery", ""),
    ]);
    for dredge in [false, true] {
        let mut game = state(vec![0], vec![2, 3, 4]);
        game.life = 3;
        if dredge {
            move_to_graveyard(&cards, &mut game, CardIdx(1), 1, CardZone::Hand);
        }
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_eq!(game.life, 1);
        assert_eq!(game.life_paid, 2);
        assert_eq!(game.life_funded_draws, u32::from(!dredge));
        assert_eq!(game.hand, [CardIdx(if dredge { 1 } else { 4 })]);
        assert_eq!(game.seen, if dredge { 3 } else { 1 });
        assert_eq!(game.milled_self, if dredge { 3 } else { 0 });
        assert_eq!(log.life_funded_draws, u32::from(!dredge));
        assert_eq!(log.milestones_by_turn[0].dredge_uses, u32::from(dredge));
        assert_eq!(json["draw"]["avg_life_funded_draws"], f64::from(!dredge));
        assert_eq!(
            json["milestones"]["dredge_uses_avg_by_turn"]["1"],
            f64::from(dredge)
        );
        assert_eq!(json["outcomes"]["finished_games"], 1);
        assert_library_rng(&game.library_rng, &rng);
    }
}
