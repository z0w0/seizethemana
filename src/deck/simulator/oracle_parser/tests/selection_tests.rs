use super::*;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::oracle_ast::{OracleAbility, OracleCard};
use crate::deck::simulator::oracle_parser::parse_oracle_card;

/// Collect typed resolution rules without lowering them into runtime effects.
fn resolution_effects(card: &OracleCard) -> Vec<&OracleEffect> {
    card.abilities
        .iter()
        .flat_map(|ability| match ability {
            OracleAbility::Spell(ability) => ability.effects.as_slice(),
            OracleAbility::Triggered(ability) => ability.effects.as_slice(),
            _ => &[],
        })
        .collect()
}

/// Current captured draw-and-life sources retain their full controller subject.
#[test]
fn captured_draw_life_sources_have_two_typed_effects() {
    for (name, amount, loss) in [
        ("Night's Whisper", 2, 2),
        ("Phyrexian Arena", 1, 1),
        ("Sign in Blood", 2, 2),
    ] {
        let card = parse_oracle_card(&named_row(name));
        let effects = resolution_effects(&card);
        assert!(
            matches!(effects.as_slice(), [OracleEffect::Draw(draw), OracleEffect::LoseLife {
            amount: life,
            scope: LifeLossScope::SelfOnly,
        }] if *draw == amount && *life == loss),
            "{name}: {effects:?}"
        );
    }
}

/// Captured top-selection wording becomes typed rules without checking the card name.
#[test]
fn captured_top_selection_sources_parse_before_lowering() {
    let brainstorm = parse_oracle_card(&named_row("Brainstorm"));
    assert!(matches!(
        resolution_effects(&brainstorm).as_slice(),
        [OracleEffect::Draw(3), OracleEffect::PutHandOnTop(2)]
    ));
    let ponder = parse_oracle_card(&named_row("Ponder"));
    assert!(matches!(
        resolution_effects(&ponder).as_slice(),
        [
            OracleEffect::LookAtTop {
                count: 3,
                may_shuffle: true
            },
            OracleEffect::Draw(1)
        ]
    ));
}

/// Changing source identity cannot add or remove a draw or selection rule.
#[test]
fn renamed_captured_sources_keep_the_same_typed_rules() {
    for name in ["Ponder", "Brainstorm", "Night's Whisper", "Sign in Blood"] {
        let mut source = named_row(name);
        let original = parse_oracle_card(&source);
        source.name = "Renamed source".into();
        let renamed = parse_oracle_card(&source);
        assert_eq!(
            format!("{:?}", resolution_effects(&original)),
            format!("{:?}", resolution_effects(&renamed)),
            "{name}"
        );
    }
}

/// Complete controller wording keeps counts, instruction order, and repeated subjects.
#[test]
fn controller_draw_life_grammar_is_complete_and_ordered() {
    for text in [
        "You draw two cards and lose 2 life.",
        "Draw two cards and you lose two life.",
        "Draw two cards. You lose 2 life.",
        "  YOU draw two cards\n and lose 2 life.  ",
    ] {
        let effects = parse_draw_life(text).expect("complete controller draw and life loss");
        assert!(
            matches!(
                effects.as_slice(),
                [
                    OracleEffect::Draw(2),
                    OracleEffect::LoseLife {
                        amount: 2,
                        scope: LifeLossScope::SelfOnly,
                    }
                ]
            ),
            "{text}"
        );
    }
    let effects = parse_draw_life("You lose 3 life, then draw four cards.")
        .expect("life loss before the draw remains first");
    assert!(matches!(
        effects.as_slice(),
        [
            OracleEffect::LoseLife {
                amount: 3,
                scope: LifeLossScope::SelfOnly,
            },
            OracleEffect::Draw(4)
        ]
    ));
}

/// A target-player draw uses the explicit solitaire choice to target the controller.
#[test]
fn target_player_draws_and_linked_life_loss_choose_self() {
    for text in [
        "Target player draws three cards and loses 4 life.",
        "Target player draws three cards. That player loses 4 life.",
        "Target player draws three cards and they lose four life.",
    ] {
        let effects = parse_draw_life(text).expect("complete target-player draw and life loss");
        assert!(
            matches!(
                effects.as_slice(),
                [
                    OracleEffect::Draw(3),
                    OracleEffect::LoseLife {
                        amount: 4,
                        scope: LifeLossScope::SelfOnly,
                    }
                ]
            ),
            "{text}"
        );
    }
    assert!(matches!(
        parse_draw_life("Target player draws a card.")
            .expect("a player can target themself")
            .as_slice(),
        [OracleEffect::Draw(1)]
    ));
}

/// Opponent subjects, variable amounts, and incomplete tails are not self-draw rules.
#[test]
fn draw_life_grammar_rejects_other_subjects_and_unknown_tails() {
    for text in [
        "Target opponent draws two cards and loses 2 life.",
        "Each player draws two cards and loses 2 life.",
        "Each opponent draws two cards and loses 2 life.",
        "Draw two cards and target opponent loses 2 life.",
        "Target player draws two cards and you lose 2 life.",
        "You may draw two cards and lose 2 life.",
        "Draw X cards and lose 2 life.",
        "Draw two cards and lose X life.",
        "Draw two cards and lose 2 life for each creature you control.",
        "Draw two cards and lose 2 life. Create a Treasure token.",
        "If you control a creature, draw two cards and lose 2 life.",
        "Draw two cards instead and lose 2 life.",
    ] {
        assert!(parse_draw_life(text).is_none(), "{text}");
    }
}

/// The selection grammar uses written counts instead of fixed spell-specific amounts.
#[test]
fn selection_counts_and_optional_shuffle_come_from_the_text() {
    let effects = parse_top_selection(
        "Draw four cards, then put three cards from your hand on top of your library in any order.",
    )
    .expect("generic draw and return counts");
    assert!(matches!(
        effects.as_slice(),
        [OracleEffect::Draw(4), OracleEffect::PutHandOnTop(3)]
    ));
    let effects = parse_top_selection(
        "Look at the top five cards of your library, then put them back in any order. Draw two cards.",
    )
    .expect("a look does not require a shuffle");
    assert!(matches!(
        effects.as_slice(),
        [
            OracleEffect::LookAtTop {
                count: 5,
                may_shuffle: false
            },
            OracleEffect::Draw(2)
        ]
    ));
    assert!(matches!(
        parse_top_selection(
            "Look at the top two cards of your library, then put them back in any order. You may shuffle your library."
        )
        .expect("the next Oracle paragraph can contain a separate draw")
        .as_slice(),
        [OracleEffect::LookAtTop {
            count: 2,
            may_shuffle: true
        }]
    ));
}

/// Selection does not consume partial instructions or change another player's library.
#[test]
fn selection_grammar_rejects_wrong_zones_and_unknown_consequences() {
    for text in [
        "Draw three cards, then discard two cards.",
        "Target opponent draws three cards, then put two cards from your hand on top of your library in any order.",
        "Draw three cards, then put two cards from your graveyard on top of your library in any order.",
        "Draw three cards, then put two cards from your hand on the bottom of your library in any order.",
        "Draw three cards, then put two cards from your hand on top of your library in any order. Draw a card.",
        "Look at the top three cards of target player's library, then put them back in any order.",
        "Look at the top three cards of your library.",
        "Look at the top three cards of your library, then put them back in any order. Shuffle your library. Draw a card.",
        "Look at the top three cards of your library, then put them back in any order. You may shuffle. Draw a card. Gain 1 life.",
        "Look at the top X cards of your library, then put them back in any order.",
    ] {
        assert!(parse_top_selection(text).is_none(), "{text}");
    }
}
