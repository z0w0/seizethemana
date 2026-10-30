use super::*;
use crate::deck::simulator::library_core_report_tests::put_permanents;
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// Plotting does not pay a casting surcharge; the later cast pays it exactly once.
#[test]
fn plotted_casts_keep_behold_mana_and_leave_unpayable_cards_plotted() {
    let cards = deck(&[row(
        "Plotted Behold",
        "{9}",
        "Sorcery",
        "Plot {1}\nAs an additional cost to cast this spell, behold an Elf or pay {2}.\nGain 2 life.",
    )]);
    let mut game = state(vec![0], vec![]);
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    plot_unaffordable_cards(&cards, &mut game, &mut pool, 1);
    assert_eq!(game.plotted, [(CardIdx(0), 1)]);
    assert_eq!(pool.total(), 0);
    assert_eq!(game.spells_cast_this_turn, 0);
    let mut spent = [0.0, 0.0];
    pool.colorless = 1;
    cast_plotted_cards(&cards, &mut game, &mut pool, 2, &mut spent, &mut Vec::new());
    assert_eq!(game.plotted, [(CardIdx(0), 1)]);
    assert_eq!(pool.colorless, 1);
    assert_eq!(spent, [0.0, 0.0]);
    assert_eq!(game.life_gained, 0);
    pool.colorless = 2;
    cast_plotted_cards(&cards, &mut game, &mut pool, 2, &mut spent, &mut Vec::new());
    assert!(game.plotted.is_empty());
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(pool.total(), 0);
    assert_eq!(spent, [0.0, 2.0]);
    assert_eq!(game.life_gained, 2);
}

/// A reveal or live generic reduction can cover the later plotted surcharge.
#[test]
fn plotted_behold_cost_uses_nonconsuming_choices_and_live_reductions() {
    let cards = deck(&[
        row(
            "Plotted Behold",
            "{9}",
            "Sorcery",
            "Plot {1}\nAs an additional cost to cast this spell, behold an Elf or pay {2}.\nGain 2 life.",
        ),
        row("Revealed Elf", "{99}", "Creature — Elf", ""),
        row(
            "Live reducer",
            "{99}",
            "Artifact",
            "Spells you cast cost {2} less to cast.",
        ),
    ]);
    for reveal in [false, true] {
        let mut game = state(if reveal { vec![0, 1] } else { vec![0] }, vec![]);
        if !reveal {
            put_permanents(&cards, &mut game, &[2]);
        }
        let mut pool = ManaPool {
            colorless: 1,
            ..Default::default()
        };
        plot_unaffordable_cards(&cards, &mut game, &mut pool, 1);
        assert_eq!(game.plotted, [(CardIdx(0), 1)]);
        assert_eq!(pool.total(), 0);
        let mut spent = [0.0, 0.0];
        let battlefield = format!("{:?}", game.battlefield);
        cast_plotted_cards(&cards, &mut game, &mut pool, 2, &mut spent, &mut Vec::new());
        assert!(game.plotted.is_empty());
        assert_eq!(game.hand, if reveal { vec![CardIdx(1)] } else { vec![] });
        assert_eq!(format!("{:?}", game.battlefield), battlefield);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(pool.total(), 0);
        assert_eq!(spent, [0.0, 0.0]);
    }
}
