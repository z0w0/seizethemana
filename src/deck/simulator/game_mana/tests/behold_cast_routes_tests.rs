use crate::deck::simulator::game::ManaPool;
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use crate::deck::simulator::turn_loop_tests::cast;

/// A flashback or escape base cost cannot waive the Behold surcharge.
#[test]
fn replay_routes_pay_behold_once_before_the_spell_leaves_the_graveyard() {
    for escape in [false, true] {
        let keyword = if escape {
            "Escape—{1}{U}, Exile three other cards from your graveyard."
        } else {
            "Flashback {1}{U}"
        };
        let cards = deck(&[
            row(
                "Replay Behold",
                "{9}{U}",
                "Sorcery",
                &format!(
                    "As an additional cost to cast this spell, behold an Elf or pay {{2}}.\nGain 2 life.\n{keyword}"
                ),
            ),
            row("First escape fodder", "{99}", "Sorcery", ""),
            row("Second escape fodder", "{99}", "Sorcery", ""),
            row("Third escape fodder", "{99}", "Sorcery", ""),
        ]);
        assert!(if escape {
            cards.cards[0].spell_data.own_escape.is_some()
        } else {
            cards.cards[0].spell_data.own_flashback.is_some()
        });
        for generic in [2, 3] {
            let mut game = state(vec![], vec![]);
            game.graveyard = if escape {
                vec![CardIdx(0), CardIdx(1), CardIdx(2), CardIdx(3)]
            } else {
                vec![CardIdx(0)]
            };
            let before_graveyard = game.graveyard.clone();
            let mut pool = ManaPool {
                colorless: generic,
                fixed: [0, 1, 0, 0, 0],
                ..Default::default()
            };
            let before = pool.clone();
            let accepted = generic == 3;
            assert_eq!(
                cast(&cards, &mut game, &mut pool),
                [if accepted { 4.0 } else { 0.0 }]
            );
            assert_eq!(game.replay_casts, u32::from(accepted));
            assert_eq!(game.life_gained, if accepted { 2 } else { 0 });
            if accepted {
                assert_eq!(pool.total(), 0);
                assert_eq!(game.exile.len(), if escape { 3 } else { 1 });
            } else {
                assert_eq!(pool, before);
                assert_eq!(game.graveyard, before_graveyard);
                assert!(game.exile.is_empty());
            }
        }
    }
}

/// Exiling a card for an alternative cost does not replace additional Behold mana.
#[test]
fn a_pitch_alternative_still_pays_or_beholds_before_exiling_its_card() {
    for (quality, generic, accepted, spent) in [
        ("Human", 1, false, 0.0),
        ("Human", 2, true, 2.0),
        ("Elf", 0, true, 0.0),
    ] {
        let mut pitch = row(
            "Blue pitch card",
            "{99}{U}",
            &format!("Creature — {quality}"),
            "",
        );
        pitch.colors = "[\"U\"]".into();
        let cards = deck(&[
            row(
                "Pitch Behold",
                "{9}{U}",
                "Sorcery",
                "You may exile one blue card from your hand rather than pay this spell's mana cost.\nAs an additional cost to cast this spell, behold an Elf or pay {2}.\nGain 2 life.",
            ),
            pitch,
        ]);
        assert!(cards.cards[0].spell_data.alternative_cast_cost.is_some());
        let mut game = state(vec![0, 1], vec![]);
        let mut pool = ManaPool {
            colorless: generic,
            ..Default::default()
        };
        let before = pool.clone();
        assert_eq!(cast(&cards, &mut game, &mut pool), [spent]);
        assert_eq!(game.hand.is_empty(), accepted);
        if accepted {
            assert_eq!(game.exile, [CardIdx(1)]);
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(game.life_gained, 2);
            assert_eq!(pool.total(), 0);
        } else {
            assert_eq!(pool, before);
            assert_eq!(game.hand, [CardIdx(0), CardIdx(1)]);
            assert!(game.exile.is_empty());
            assert_eq!(game.life_gained, 0);
        }
    }
}

/// Cascade waives only the hit's mana cost, not its mandatory extra generic mana.
#[test]
fn cascade_does_not_escape_behold_additional_costs() {
    let cards = deck(&[
        row("Cascade parent", "{5}", "Sorcery", "Cascade"),
        row(
            "Cascade Behold hit",
            "{1}",
            "Sorcery",
            "As an additional cost to cast this spell, behold an Elf or pay {2}.\nGain 2 life.",
        ),
    ]);
    for extra in [1, 2] {
        let mut game = state(vec![0], vec![1]);
        let mut pool = ManaPool {
            colorless: 5 + extra,
            ..Default::default()
        };
        let accepted = extra == 2;
        assert_eq!(
            cast(&cards, &mut game, &mut pool),
            [if accepted { 7.0 } else { 5.0 }]
        );
        assert_eq!(game.life_gained, if accepted { 2 } else { 0 });
        assert_eq!(game.spells_cast_this_turn, if accepted { 2 } else { 1 });
        assert_eq!(
            game.library,
            if accepted { vec![] } else { vec![CardIdx(1)] }
        );
        assert_eq!(pool.total(), if accepted { 0 } else { 1 });
    }
}
