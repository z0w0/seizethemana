use super::*;

/// Losing the event's opponent does not remove already-triggered player rewards.
#[test]
fn eliminated_opponent_cannot_cancel_pending_treasure() {
    let mut drain = engine(SimTrigger::OpponentDraw, 1);
    drain.striations[0].abilities[0].effect = SimEffect::LoseLife {
        amount: 2,
        scope: crate::deck::simulator::model::LifeLossScope::TargetPlayer,
    };
    let mut treasure = engine(SimTrigger::OpponentDraw, 2);
    treasure.striations[0].abilities[0].effect = SimEffect::Treasures(1);
    let mut cards = crate::deck::simulator::turn_loop_tests::deck(&[]);
    cards.cards = vec![drain, treasure];
    let mut st = crate::deck::simulator::turn_loop_tests::state(vec![], vec![]);
    st.opponents = super::super::OpponentTable::for_format(Format::Commander);
    st.opponents.seats[0].life = 2;
    for index in 0..2 {
        st.battlefield.push(new_perm_with(
            index + 1,
            &cards,
            crate::deck::simulator::model::CardIdx(index),
            1,
            false,
        ));
    }
    resolve_event(&cards, &mut st, 0, OpponentEvent::Draw, 2);
    assert!(!st.opponents.seats[0].active);
    assert_eq!(st.treasure_bank, 1);
}

/// Spell-only production cannot displace an unrestricted upkeep mode.
#[test]
fn nonspell_payment_uses_unrestricted_source_mode() {
    use crate::deck::simulator::turn_loop_tests::{deck, row, state};
    let cards = deck(&[
        row(
            "Choice land",
            "",
            "Land",
            "{T}: Add {C}.\n{T}: Add one mana of any color. Spend this mana only to cast a legendary spell.",
        ),
        row("Legend", "{U}", "Legendary Creature", ""),
    ]);
    let mut st = state(vec![1], vec![]);
    let source = new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    );
    st.battlefield.push(source.clone());
    let mut pool = ManaPool::default();
    assert!(
        crate::deck::simulator::game_run::mana::tap_for_nonspell_payment(
            &cards, &mut st, &source, &mut pool, 2
        )
    );
    assert_eq!(pool.colorless, 1);
    assert!(crate::deck::simulator::game_mana::payable(
        &Cost {
            generic: 1,
            ..Default::default()
        },
        &pool
    ));
}

/// Turning a creature face down removes its printed mana modes.
#[test]
fn facedown_source_cannot_pay_upkeep_with_printed_mana() {
    use crate::deck::simulator::turn_loop_tests::{deck, row, state};
    let cards = deck(&[row(
        "Hidden source",
        "{1}",
        "Creature",
        "{T}: Add {G}.\nMorph {1}",
    )]);
    let mut st = state(vec![], vec![]);
    let mut source = new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    );
    source.face_down = true;
    source.summoning_sick = false;
    st.battlefield.push(source.clone());
    let mut pool = ManaPool::default();
    assert!(
        !crate::deck::simulator::game_run::mana::tap_for_nonspell_payment(
            &cards, &mut st, &source, &mut pool, 2
        )
    );
    assert_eq!(pool.total(), 0);
    assert!(!st.battlefield[0].tapped);
}
use crate::deck::simulator::{
    game::new_perm_with,
    model::{Format, SimAbility, SimAbilityKind, SimCard, SimEffect, SimStriation},
};

/// Construct an engine directly so runtime tax semantics do not depend on parsing.
fn engine(trigger: SimTrigger, tax: u32) -> SimCard {
    SimCard {
        name: format!("engine {tax}"),
        is_permanent: true,
        is_enchantment: true,
        striations: vec![SimStriation {
            at: 0,
            animate: false,
            abilities: vec![SimAbility {
                kind: SimAbilityKind::Triggered,
                trigger,
                opponent_tax: Some(OpponentTax::Fixed(tax)),
                effect: SimEffect::Draw(1),
                ..SimAbility::default()
            }],
        }],
        ..SimCard::default()
    }
}

/// Two listeners must not each spend the same last opponent mana source.
#[test]
fn multiple_engines_share_one_cast_payment_ledger() {
    let deck = crate::deck::simulator::model::SimDeck {
        cards: vec![
            engine(SimTrigger::OpponentSpellCast, 1),
            engine(SimTrigger::OpponentNoncreatureCast, 4),
            SimCard::default(),
        ],
        commanders: Vec::new(),
        companion: None,
        format: Format::Commander,
        rules: crate::deck::simulator::format::rules_for("commander"),
    };
    let mut state = crate::deck::simulator::turn_loop_tests::state(Vec::new(), vec![2]);
    state.opponents = super::super::OpponentTable::for_format(Format::Commander);
    state.opponents.seats[0].unspent_sources = 4;
    state.battlefield.push(new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    state.battlefield.push(new_perm_with(
        2,
        &deck,
        crate::deck::simulator::model::CardIdx(1),
        1,
        false,
    ));
    resolve_event(
        &deck,
        &mut state,
        0,
        OpponentEvent::Cast {
            noncreature: true,
            first_noncreature: true,
        },
        1,
    );
    assert_eq!(state.opponents.payments, 1);
    assert_eq!(state.opponents.seats[0].unspent_sources, 3);
    assert_eq!(state.hand.len(), 1);
}

/// Draw rewards create a banked token rather than spending it during the event.
#[test]
fn unpaid_draw_tax_banks_treasure() {
    let mut card = engine(SimTrigger::OpponentDraw, 2);
    card.striations[0].abilities[0].effect = SimEffect::Treasures(1);
    let deck = crate::deck::simulator::model::SimDeck {
        cards: vec![card],
        commanders: Vec::new(),
        companion: None,
        format: Format::Commander,
        rules: crate::deck::simulator::format::rules_for("commander"),
    };
    let mut state = crate::deck::simulator::turn_loop_tests::state(Vec::new(), Vec::new());
    state.opponents = super::super::OpponentTable::for_format(Format::Commander);
    state.battlefield.push(new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    resolve_event(&deck, &mut state, 0, OpponentEvent::Draw, 1);
    assert_eq!(state.treasure_bank, 1);
    assert_eq!(state.opponents.payments, 0);
}

/// Declared land capability supplies only the listed colors, never explicit C.
#[test]
fn declared_opponent_colors_override_generic_credit() {
    let row = crate::deck::simulator::turn_loop_tests::row(
        "Foreign mana rock",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
    );
    let deck = crate::deck::simulator::turn_loop_tests::deck(&[row]);
    let mut state = crate::deck::simulator::turn_loop_tests::state(Vec::new(), Vec::new());
    state.opponents = super::super::OpponentTable::for_format(Format::Commander);
    state.opponents.config.land_colors = Some(vec!["U".into(), "".into(), "".into()]);
    state.opponents.seats[0].lands = 1;
    let source = new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    );
    state.battlefield.push(source.clone());
    let mut pool = ManaPool::default();
    assert!(crate::deck::simulator::game_run::mana::activate_mana_mode(
        &deck, &mut state, &source, &mut pool, 2
    ));
    assert_eq!(pool.total(), 1);
    let blue = crate::deck::simulator::oracle_parser::cost::parse_cost("{U}");
    let black = crate::deck::simulator::oracle_parser::cost::parse_cost("{B}");
    let colorless = crate::deck::simulator::oracle_parser::cost::parse_cost("{C}");
    assert!(crate::deck::simulator::game_mana::payable(&blue, &pool));
    assert!(!crate::deck::simulator::game_mana::payable(&black, &pool));
    assert!(!crate::deck::simulator::game_mana::payable(
        &colorless, &pool
    ));
}

/// Native and leftover activation paths reject opponent mana without a legal source.
#[test]
fn opponent_mana_needs_an_active_seat_and_declared_color() {
    use crate::deck::simulator::game_effects::activation::spend_leftover;
    use crate::deck::simulator::game_run::mana::activate_mana_mode;
    use crate::deck::simulator::turn_loop_tests::{deck, row, state};

    let cards = deck(&[row(
        "Foreign mana rock",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
    )]);
    for active in [true, false] {
        for leftover in [false, true] {
            let mut st = state(Vec::new(), Vec::new());
            st.opponents = super::super::OpponentTable::for_format(Format::Commander);
            st.opponents.config.land_colors = Some(vec![String::new(); st.opponents.seats.len()]);
            for seat in &mut st.opponents.seats {
                seat.active = active;
                seat.lands = u32::from(active);
            }
            let source = new_perm_with(
                1,
                &cards,
                crate::deck::simulator::model::CardIdx(0),
                1,
                false,
            );
            st.battlefield.push(source.clone());
            let mut pool = ManaPool::default();

            if leftover {
                spend_leftover(&cards, &mut st, &mut pool, 2);
            } else {
                assert!(!activate_mana_mode(&cards, &mut st, &source, &mut pool, 2,));
            }

            assert_eq!(pool.total(), 0, "active={active}, leftover={leftover}");
            assert!(
                !st.battlefield[0].tapped,
                "active={active}, leftover={leftover}"
            );
        }
    }
}

/// Opponent-round damage remains in the cumulative player-damage census.
#[test]
fn opponent_round_damage_is_added_after_the_turn_census() {
    use crate::deck::simulator::{
        game_run::{TurnCensus, census::record_opponent_round_damage},
        opponents::run_opponent_round,
        turn_loop_tests::{deck, state},
    };

    let mut source = engine(SimTrigger::OpponentDraw, 0);
    source.striations[0].abilities[0].opponent_tax = None;
    source.striations[0].abilities[0].effect = SimEffect::Damage {
        amount: 1,
        scope: crate::deck::simulator::model::LifeLossScope::EachOpponent,
    };
    let mut cards = deck(&[]);
    cards.cards = vec![source];
    let mut state = state(vec![], vec![]);
    state.opponents = super::super::OpponentTable::for_format(Format::Commander);
    state.battlefield.push(new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    let mut census = TurnCensus::new(2, 1);
    census.player_damage[0] = 4;
    let before = state.damage_dealt_this_turn;

    run_opponent_round(&cards, &mut state, 1);
    record_opponent_round_damage(&mut census, 1, before, state.damage_dealt_this_turn);

    assert_eq!(census.player_damage, [13, 0]);
}

/// Cumulative upkeep consumes real sources and sacrifices an unpaid engine.
#[test]
fn cumulative_upkeep_increases_and_does_not_float_main_phase_mana() {
    use crate::deck::simulator::turn_loop_tests::{deck, row, state};
    let remora = row(
        "Upkeep engine",
        "{U}",
        "Enchantment",
        "Cumulative upkeep {1}\nWhenever an opponent casts a noncreature spell, you may draw a card unless that player pays {4}.",
    );
    let island = row("Island", "", "Basic Land — Island", "{T}: Add {U}.");
    let cards = deck(&[remora, island.clone(), island.clone(), island]);
    let mut st = state(Vec::new(), Vec::new());
    for index in 0..4 {
        st.battlefield.push(new_perm_with(
            index + 1,
            &cards,
            crate::deck::simulator::model::CardIdx(index),
            1,
            false,
        ));
    }
    pay_cumulative_upkeep(&cards, &mut st, 2);
    assert_eq!(
        st.battlefield.iter().filter(|source| source.tapped).count(),
        1
    );
    for source in &mut st.battlefield {
        source.tapped = false;
    }
    pay_cumulative_upkeep(&cards, &mut st, 3);
    assert_eq!(
        st.battlefield.iter().filter(|source| source.tapped).count(),
        2
    );
    st.battlefield.retain(|source| source.uid == 1);
    pay_cumulative_upkeep(&cards, &mut st, 4);
    assert!(st.battlefield.is_empty());
    assert_eq!(
        st.graveyard,
        vec![crate::deck::simulator::model::CardIdx(0)]
    );
}
