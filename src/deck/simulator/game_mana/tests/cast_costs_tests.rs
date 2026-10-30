use super::*;
use crate::db::CardRow;
use crate::deck::simulator::game::ManaPool;
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_run::{TurnCensus, commander_phase};
use crate::deck::simulator::library_core_report_tests::put_permanents;
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use crate::deck::simulator::turn_loop_tests::cast;

/// Build a supported spell with a fixed Behold-or-pay additional cost.
fn behold_spell(cost: &str, generic: u32) -> CardRow {
    row(
        "Behold payment",
        cost,
        "Sorcery",
        &format!(
            "As an additional cost to cast this spell, behold an Elf or pay {{{generic}}}.\nGain 2 life."
        ),
    )
}

/// Each unsupported Behold choice adds the exact stated generic mana once.
#[test]
fn behold_extra_one_and_two_are_gated_and_paid_exactly_once() {
    for extra in [1, 2] {
        let cards = deck(&[behold_spell("{U}", extra)]);
        assert_eq!(
            cards.cards[0]
                .spell_data
                .behold_alternative
                .as_ref()
                .expect("typed Behold cost")
                .generic,
            extra
        );
        assert!(!cards.cards[0].spell_data.unsupported_additional_cost);
        for available in [extra - 1, extra] {
            let mut game = state(vec![0], vec![]);
            let mut pool = ManaPool {
                colorless: available,
                fixed: [0, 1, 0, 0, 0],
                ..Default::default()
            };
            let before = pool.clone();
            let paid = available == extra;
            assert_eq!(
                cast(&cards, &mut game, &mut pool),
                [if paid { f64::from(extra + 1) } else { 0.0 }]
            );
            assert_eq!(game.hand.is_empty(), paid);
            assert_eq!(game.life_gained, if paid { 2 } else { 0 });
            assert_eq!(game.spells_cast_this_turn, u32::from(paid));
            if paid {
                assert_eq!(pool.total(), 0);
                assert_eq!(game.graveyard, [CardIdx(0)]);
            } else {
                assert_eq!(pool, before);
                assert!(game.graveyard.is_empty());
            }
        }
    }
}

/// A matching reveal or permanent waives extra mana without spending that object.
#[test]
fn beholding_does_not_discard_exile_sacrifice_or_tap_the_choice() {
    let cards = deck(&[
        behold_spell("{U}", 2),
        row("Available Elf", "{99}", "Creature — Elf", ""),
    ]);
    for in_hand in [false, true] {
        let mut game = state(if in_hand { vec![0, 1] } else { vec![0] }, vec![]);
        if !in_hand {
            put_permanents(&cards, &mut game, &[1]);
        }
        let battlefield = format!("{:?}", game.battlefield);
        let mut pool = ManaPool {
            fixed: [0, 1, 0, 0, 0],
            ..Default::default()
        };
        assert_eq!(cast(&cards, &mut game, &mut pool), [1.0]);
        assert_eq!(game.hand, if in_hand { vec![CardIdx(1)] } else { vec![] });
        assert_eq!(format!("{:?}", game.battlefield), battlefield);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert!(game.exile.is_empty());
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.life_gained, 2);
        assert_eq!(pool.total(), 0);
    }
}

/// A spell cannot use its own hand characteristics to pay its Behold cost.
#[test]
fn a_pending_elf_spell_cannot_behold_itself() {
    let mut source = behold_spell("{U}", 1);
    source.type_line = "Creature — Elf".into();
    let cards = deck(&[source]);
    for extra in [0, 1] {
        let mut game = state(vec![0], vec![]);
        let mut pool = ManaPool {
            colorless: extra,
            fixed: [0, 1, 0, 0, 0],
            ..Default::default()
        };
        assert_eq!(
            cast(&cards, &mut game, &mut pool),
            [if extra == 1 { 2.0 } else { 0.0 }]
        );
        assert_eq!(game.hand.is_empty(), extra == 1);
        assert_eq!(game.battlefield.len(), extra as usize);
        assert_eq!(pool.total(), if extra == 1 { 0 } else { 1 });
    }
}

/// The generic reduction covers Behold's surcharge before the total reaches zero.
#[test]
fn generic_reductions_apply_after_behold_mana_is_added() {
    let cards = deck(&[
        row(
            "Live reducer",
            "{9}",
            "Artifact",
            "Spells you cast cost {3} less to cast.",
        ),
        behold_spell("{U}", 2),
    ]);
    let mut game = state(vec![1], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let mut pool = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        ..Default::default()
    };
    assert_eq!(cast(&cards, &mut game, &mut pool), [1.0]);
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(pool.total(), 0);
}

/// X pays the Behold surcharge once and still benefits from live flat reductions.
#[test]
fn x_selection_uses_behold_and_flat_reductions_together() {
    let cards = deck(&[
        row(
            "X reducer",
            "{9}",
            "Artifact",
            "Spells you cast cost {5} less to cast.",
        ),
        row(
            "Behold X",
            "{X}{B}",
            "Sorcery",
            "As an additional cost to cast this spell, behold an Elf or pay {2}.\nTarget player loses X life.",
        ),
    ]);
    let mut game = state(vec![1], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let mut pool = ManaPool {
        colorless: 1,
        fixed: [0, 0, 1, 0, 0],
        ..Default::default()
    };
    assert_eq!(cast(&cards, &mut game, &mut pool), [2.0]);
    assert_eq!(game.opponent_life_lost, 4);
    assert_eq!(game.graveyard, [CardIdx(1)]);
    assert_eq!(pool.total(), 0);
}

/// Commander tax and Behold mana are both added before the generic reduction.
#[test]
fn commander_behold_mana_and_recast_tax_share_one_generic_total() {
    let mut cards = deck(&[row(
        "Commander reducer",
        "{9}",
        "Artifact",
        "Creature spells you cast cost {3} less to cast.",
    )]);
    let mut commander = behold_spell("{U}", 2);
    commander.type_line = "Legendary Creature".into();
    cards.commanders = deck(&[commander]).cards;
    assert_eq!(cards.cards[0].cost_reductions.len(), 1);
    assert!(cards.commanders[0].has_mana_cost);
    assert!(!cards.commanders[0].spell_data.unsupported_additional_cost);
    assert_eq!(
        cards.commanders[0]
            .spell_data
            .behold_alternative
            .as_ref()
            .expect("typed commander Behold cost")
            .generic,
        2
    );
    let profile = CommanderProfile::new(&cards);
    let mut census = TurnCensus::new(3, 1);
    let mut game = state(vec![], vec![]);
    put_permanents(&cards, &mut game, &[0]);
    let mut repeatable = Vec::new();
    for (turn, generic, accepted) in [(1, 0, true), (2, 0, false), (3, 1, true)] {
        let casts = game.commander_casts.first().copied().unwrap_or(0);
        let mut pool = ManaPool {
            colorless: generic,
            fixed: [0, 1, 0, 0, 0],
            ..Default::default()
        };
        commander_phase(
            &cards,
            &mut game,
            &mut pool,
            &mut census,
            &profile,
            turn,
            &mut repeatable,
            false,
        );
        assert_eq!(game.commander_casts, [casts + u32::from(accepted)]);
        assert_eq!(pool.total(), if accepted { 0 } else { 1 });
        game.battlefield.retain(|permanent| {
            !matches!(
                permanent.card,
                crate::deck::simulator::game::CardRef::Commander { .. }
            )
        });
    }
    assert_eq!(census.mana_spent, [1.0, 0.0, 2.0]);
}
