//! Exact cast costs and linked rewards from pinned Oracle wording.

use super::*;
use crate::deck::simulator::game::{ManaPool, new_perm_with};
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::turn_loop_tests::{cast, deck, row, state};

/// Village Rites and Deadly Dispute pay a legal creature sacrifice before drawing.
#[test]
fn creature_sacrifice_draw_spells_pay_and_receive_their_rewards() {
    for (name, cost, text, black, treasures) in [
        (
            "Village Rites",
            "{B}",
            "As an additional cost to cast this spell, sacrifice a creature.\nDraw two cards.",
            1,
            0,
        ),
        (
            "Deadly Dispute",
            "{1}{B}",
            "As an additional cost to cast this spell, sacrifice an artifact or creature.\nDraw two cards and create a Treasure token.",
            2,
            1,
        ),
    ] {
        let cards = deck(&[
            row(name, cost, "Instant", text),
            row("Sacrifice", "{0}", "Creature — Elf", ""),
            row("Drawn one", "{9}", "Sorcery", ""),
            row("Drawn two", "{9}", "Sorcery", ""),
        ]);
        let mut st = state(vec![0], vec![2, 3]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(1), 0, false));
        st.next_uid = 1;
        let mut pool = ManaPool {
            fixed: [0, 0, black, 0, 0],
            ..Default::default()
        };
        cast(&cards, &mut st, &mut pool);
        assert_eq!(st.hand, [CardIdx(3), CardIdx(2)], "{name}");
        assert_eq!(st.graveyard, [CardIdx(1), CardIdx(0)], "{name}");
        assert!(st.battlefield.is_empty(), "{name}");
        assert_eq!(st.treasure_bank, treasures, "{name}");
        assert_eq!(st.spells_cast_this_turn, 1, "{name}");
    }
}

/// Thrill of Possibility discards another hand card and draws exactly two.
#[test]
fn thrill_of_possibility_pays_its_discard_before_drawing() {
    let cards = deck(&[
        row(
            "Thrill of Possibility",
            "{1}{R}",
            "Instant",
            "As an additional cost to cast this spell, discard a card.\nDraw two cards.",
        ),
        row("Discarded", "{9}", "Sorcery", ""),
        row("Drawn one", "{9}", "Sorcery", ""),
        row("Drawn two", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0, 1], vec![2, 3]);
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 2, 0],
        ..Default::default()
    };
    cast(&cards, &mut st, &mut pool);
    assert_eq!(st.graveyard, [CardIdx(1), CardIdx(0)]);
    assert_eq!(st.hand, [CardIdx(3), CardIdx(2)]);
    assert!(st.library.is_empty());
    assert_eq!(pool.total(), 0);
}

/// Grim Tutor loses life after its search instead of dropping the drawback.
#[test]
fn grim_tutor_search_and_life_loss_resolve_together() {
    let cards = deck(&[
        row(
            "Grim Tutor",
            "{1}{B}{B}",
            "Sorcery",
            "Search your library for a card, put that card into your hand, then shuffle. You lose 3 life.",
        ),
        row("Search one", "{9}", "Sorcery", ""),
        row("Search two", "{9}", "Sorcery", ""),
    ]);
    assert!(matches!(
        cards.cards[0].spell_data.effects.as_slice(),
        [
            SimEffect::Search(_),
            SimEffect::LoseLife {
                amount: 3,
                scope: super::super::model::LifeLossScope::SelfOnly
            }
        ]
    ));
    let mut st = state(vec![0], vec![1, 2]);
    let mut pool = ManaPool {
        fixed: [0, 0, 3, 0, 0],
        ..Default::default()
    };
    cast(&cards, &mut st, &mut pool);
    assert_eq!(st.life, 17);
    assert_eq!(st.hand.len(), 1);
    assert_eq!(st.library.len(), 1);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(pool.total(), 0);
}

/// Eldritch Evolution searches once using printed sacrificed mana value plus two.
#[test]
fn eldritch_evolution_has_one_linked_search_and_no_unrestricted_reward() {
    use crate::deck::simulator::game::CardRef;
    for eligible in [true, false] {
        let cards = deck(&[
            row(
                "Eldritch Evolution",
                "{1}{G}{G}",
                "Sorcery",
                "As an additional cost to cast this spell, sacrifice a creature.\nSearch your library for a creature card with mana value X or less, where X is 2 plus the sacrificed creature's mana value, put that card onto the battlefield, then shuffle. Exile Eldritch Evolution.",
            ),
            row("Sacrificed creature", "{1}{G}", "Creature — Elf", ""),
            row("Lower eligible creature", "{2}{G}", "Creature — Beast", ""),
            row("Ineligible creature", "{6}{G}", "Creature — Beast", ""),
            row("Upper eligible creature", "{3}{G}", "Creature — Beast", ""),
        ]);
        let rule = cards.cards[0]
            .spell_data
            .search_after_sacrifice
            .expect("typed linked search");
        assert_eq!(rule.mana_value_offset, 2);
        assert!(rule.allow_lower_mana_value);
        assert!(!rule.enters_with_plus1_counter);
        assert!(
            cards.cards[0].spell_data.effects.is_empty(),
            "there is no second generic search"
        );
        let mut st = state(vec![0], if eligible { vec![2, 3, 4] } else { vec![3] });
        let mut sacrifice = new_perm_with(1, &cards, CardIdx(1), 0, false);
        sacrifice.counters.plus1 = 20;
        st.battlefield.push(sacrifice);
        st.next_uid = 1;
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 0, 3],
            ..Default::default()
        };
        cast(&cards, &mut st, &mut pool);
        assert_eq!(pool.total(), 0);
        assert!(st.hand.is_empty());
        assert_eq!(st.graveyard, [CardIdx(1)]);
        assert_eq!(st.exile, [CardIdx(0)]);
        assert_eq!(st.spells_cast_this_turn, 1);
        if eligible {
            assert_eq!(st.battlefield.len(), 1);
            assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(4)));
            assert_eq!(st.battlefield[0].counters.plus1, 0);
            assert_eq!(st.library.len(), 2);
            assert!(st.library.contains(&CardIdx(2)));
            assert!(st.library.contains(&CardIdx(3)));
        } else {
            assert!(st.battlefield.is_empty());
            assert_eq!(st.library, [CardIdx(3)]);
        }
    }
}

/// A tapped-entry artifact produces no entry-turn mana; an untapped control does.
#[test]
fn timeless_lotus_waits_for_untap_while_other_artifacts_do_not() {
    for (name, entry, tapped) in [
        ("Timeless Lotus", "Timeless Lotus enters tapped.\n", true),
        ("Untapped Artifact", "", false),
    ] {
        let cards = deck(&[row(
            name,
            "{5}",
            "Artifact",
            &format!("{entry}{{T}}: Add {{W}}{{U}}{{B}}{{R}}{{G}}."),
        )]);
        let mut st = state(vec![0], vec![]);
        let mut pool = ManaPool {
            colorless: 5,
            ..Default::default()
        };
        assert_eq!(cast(&cards, &mut st, &mut pool), [5.0]);
        assert_eq!(pool.total(), 0);
        assert_eq!(st.battlefield.len(), 1);
        assert_eq!(st.battlefield[0].tapped, tapped, "{name}");
        let first_turn = crate::deck::simulator::game_run::build_pool(&cards, &mut st, 1);
        assert_eq!(first_turn.total(), if tapped { 0 } else { 5 }, "{name}");
        st.battlefield[0].tapped = false;
        let next_turn = crate::deck::simulator::game_run::build_pool(&cards, &mut st, 2);
        assert_eq!(next_turn.total(), 5, "{name}");
    }
}

/// Filtered returns stay inert rather than selecting a card of the wrong type.
#[test]
fn unsupported_graveyard_returns_never_move_the_wrong_card() {
    for (name, text, grave_type) in [
        (
            "Raise Dead",
            "Return target creature card from your graveyard to your hand.",
            "Basic Land — Forest",
        ),
        (
            "Land return",
            "Return target land card from your graveyard to the battlefield.",
            "Creature — Elf",
        ),
    ] {
        let cards = deck(&[
            row(name, "{B}", "Sorcery", text),
            row("Wrong type", "{9}", grave_type, ""),
        ]);
        assert!(cards.cards[0].spell_data.effects.is_empty(), "{name}");
        let mut st = state(vec![0], vec![]);
        st.graveyard.push(CardIdx(1));
        let mut pool = ManaPool {
            fixed: [0, 0, 1, 0, 0],
            ..Default::default()
        };
        cast(&cards, &mut st, &mut pool);
        assert!(st.hand.is_empty(), "{name}");
        assert!(st.battlefield.is_empty(), "{name}");
        assert_eq!(st.graveyard, [CardIdx(1), CardIdx(0)], "{name}");
    }
}

/// Eternal Witness can return any card without inventing a creature restriction.
#[test]
fn eternal_witness_optional_return_can_recover_a_land() {
    let cards = deck(&[
        row(
            "Eternal Witness",
            "{1}{G}{G}",
            "Creature — Human Shaman",
            "When Eternal Witness enters, you may return target card from your graveyard to your hand.",
        ),
        row("Forest", "", "Basic Land — Forest", ""),
    ]);
    assert!(cards.cards[0].unlocked_abilities(0).any(|ability| matches!(
        ability.effect,
        SimEffect::ReturnFromGraveyard {
            to_hand: true,
            count: 1
        }
    )));
    let mut st = state(vec![0], vec![]);
    st.graveyard.push(CardIdx(1));
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 0, 3],
        ..Default::default()
    };
    cast(&cards, &mut st, &mut pool);
    assert_eq!(st.hand, [CardIdx(1)]);
    assert!(st.graveyard.is_empty());
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(pool.total(), 0);
}

/// A supported battlefield return selects a creature and uses normal entry rules.
#[test]
fn zombify_returns_a_creature_without_changing_its_destination() {
    let cards = deck(&[
        row(
            "Zombify",
            "{3}{B}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield.",
        ),
        row("Returned creature", "{9}", "Creature — Elf", ""),
        row("Forest", "", "Basic Land — Forest", ""),
    ]);
    let mut st = state(vec![0], vec![]);
    st.graveyard = vec![CardIdx(1), CardIdx(2)];
    let mut pool = ManaPool {
        fixed: [0, 0, 4, 0, 0],
        ..Default::default()
    };
    cast(&cards, &mut st, &mut pool);
    assert!(st.hand.is_empty());
    assert_eq!(st.graveyard, [CardIdx(2), CardIdx(0)]);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(
        st.battlefield[0].card,
        crate::deck::simulator::game::CardRef::Deck(CardIdx(1))
    );
    assert!(!st.battlefield[0].tapped);
    assert_eq!(pool.total(), 0);
}

/// Warp cannot replace normal payment until its required exile rules can execute.
#[test]
fn warp_cannot_cast_cheaply_but_the_normal_printed_cost_remains_legal() {
    for available in [3, 4] {
        let cards = deck(&[row(
            "Mightform Harmonizer",
            "{2}{G}{G}",
            "Creature — Insect Druid",
            "Warp {2}{G} (You may cast this card from your hand for its warp cost. Exile it at the beginning of the next end step, then you may cast it from exile on a later turn.)",
        )]);
        let mut st = state(vec![0], vec![]);
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 0, available],
            ..Default::default()
        };
        cast(&cards, &mut st, &mut pool);
        let cast_count = u32::from(available == 4);
        assert_eq!(st.spells_cast_this_turn, cast_count);
        assert_eq!(st.battlefield.len(), cast_count as usize);
        assert_eq!(
            st.hand,
            if available == 4 {
                vec![]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(pool.total(), if available == 4 { 0 } else { available });
        assert!(st.exile.is_empty());
    }
}

/// Shivan Reef's colored mode includes controller damage, not opponent damage.
#[test]
fn shivan_reef_preserves_its_mana_modes_and_controller_damage() {
    use crate::deck::simulator::oracle_ast::DamageTarget;
    use crate::deck::simulator::oracle_parser::parse_oracle_card;

    let source = row(
        "Shivan Reef",
        "",
        "Land",
        "{T}: Add {C}.\n{T}: Add {U} or {R}. Shivan Reef deals 1 damage to you.",
    );
    let oracle = parse_oracle_card(&source);
    let [
        OracleAbility::Activated(colorless),
        OracleAbility::Activated(colored),
    ] = oracle.abilities.as_slice()
    else {
        panic!("the land has two separate mana modes");
    };
    assert!(
        matches!(colorless.effects.as_slice(), [OracleEffect::Mana(mana)] if mana.colorless == 1)
    );
    assert!(matches!(colored.effects.as_slice(), [
        OracleEffect::Mana(mana),
        OracleEffect::Damage { amount: 1, target: DamageTarget::Player, scope: super::super::model::LifeLossScope::SelfOnly }
    ] if mana.choice[1] && mana.choice[3]));
    let lowered = parse_sim_card(&source);
    let modes: Vec<_> = lowered.unlocked_abilities(0).collect();
    assert_eq!(modes.len(), 2);
    assert!(matches!(modes[0].effect_sequence(), [SimEffect::Mana(mana)] if mana.colorless == 1));
    assert!(matches!(modes[1].effect_sequence(), [
        SimEffect::Mana(mana),
        SimEffect::Damage { amount: 1, scope: super::super::model::LifeLossScope::SelfOnly }
    ] if mana.choice[1] && mana.choice[3]));
}

/// Only the complete controller recipient becomes self-directed player damage.
#[test]
fn damage_recipients_keep_controller_and_opponent_scopes_separate() {
    use crate::deck::simulator::model::LifeLossScope;

    for (recipient, expected) in [
        ("you", Some(LifeLossScope::SelfOnly)),
        ("target player", Some(LifeLossScope::TargetPlayer)),
        ("target opponent", Some(LifeLossScope::TargetPlayer)),
        ("each opponent", Some(LifeLossScope::EachOpponent)),
        ("each player", Some(LifeLossScope::EachPlayer)),
        ("your commander", None),
        ("target creature you control", None),
        ("you and target creature", None),
    ] {
        let lowered = parse_sim_card(&row(
            "Scoped source",
            "",
            "Land",
            &format!("{{T}}: Add {{U}} or {{R}}. Scoped source deals 1 damage to {recipient}."),
        ));
        let abilities: Vec<_> = lowered.unlocked_abilities(0).collect();
        if let Some(expected) = expected {
            let [ability] = abilities.as_slice() else {
                panic!("complete mana and damage ability: {recipient}");
            };
            assert!(
                matches!(ability.effect_sequence(), [SimEffect::Mana(_), SimEffect::Damage { amount: 1, scope }] if *scope == expected),
                "{recipient}"
            );
        } else {
            assert!(abilities.is_empty(), "{recipient}");
        }
    }
}
