use super::*;
use crate::deck::simulator::game::{fire_on_enter, fire_triggers, new_perm_with};
use crate::deck::simulator::game_effects::activation::{pick_best_activation, resolve_activation};
use crate::deck::simulator::game_effects::{
    CardZone, apply_effect_at, apply_prepared_spell_effect_at, move_to_graveyard,
    prepare_spell_targets, spell_targets_available,
};
use crate::deck::simulator::model::{SimAbilityCondition, SimTrigger};
use crate::deck::simulator::turn_loop_tests::{cast, deck, row, state};
use rand::RngCore;

/// An entering draw trigger waits until the whole return-and-mill spell ends.
#[test]
fn compound_spell_mills_a_before_returned_creature_draws_b() {
    let cards = deck(&[
        row(
            "Return and mill",
            "{1}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield. Mill a card.",
        ),
        row(
            "Returned creature",
            "{9}",
            "Creature",
            "When this creature enters, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
    ]);
    assert!(matches!(
        cards.cards[0].spell_data.effects.as_slice(),
        [
            SimEffect::ReturnFromGraveyard { to_hand: false, .. },
            SimEffect::Mill(1)
        ]
    ));
    let mut st = state(vec![0], vec![3, 2]);
    st.graveyard.push(CardIdx(1));
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let mut expected_rng = st.library_rng.clone();

    cast(&cards, &mut st, &mut pool);

    assert!(st.graveyard.contains(&CardIdx(2)), "the spell mills A");
    assert_eq!(st.hand, [CardIdx(3)], "the queued ETB draws B");
    assert_eq!(st.milled_self, 1);
    assert_eq!(pool.total(), 0);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
}

/// Library return triggers cannot interrupt a multi-card mill instruction.
#[test]
fn multi_card_mill_finishes_before_library_return_etb_draws() {
    let mut cards = deck(&[
        row(
            "Library creature",
            "{9}",
            "Creature",
            "When this creature enters, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
    ]);
    cards.cards[0].library_graveyard_trigger = Some(LibraryGraveyardTrigger::ReturnToBattlefield);
    let mut st = state(Vec::new(), vec![2, 1, 0]);

    apply_effect_at(&cards, &SimEffect::Mill(2), &mut st, 1, false, None);

    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(st.milled_self, 2);
}

/// Cost-triggered undying cannot return a spell-cast listener before capture.
#[test]
fn cost_undying_source_does_not_draw_for_the_cast_that_sacrificed_it() {
    let cards = deck(&[
        row(
            "Undying engine",
            "{9}",
            "Creature",
            "Undying\nWhenever you cast a spell, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].has_undying);
    let mut st = state(Vec::new(), vec![1]);
    st.next_uid = 7;
    st.battlefield
        .push(new_perm_with(7, &cards, CardIdx(0), 0, false));

    begin_resolution(&mut st);
    let victim = st.battlefield.remove(0);
    move_to_graveyard(&cards, &mut st, CardIdx(0), 1, CardZone::Battlefield);
    queue_undying(&cards, &mut st, &victim, 1);
    assert!(st.battlefield.is_empty(), "cost triggers remain pending");
    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    end_resolution(&cards, &mut st, None);

    assert!(st.hand.is_empty(), "the new object misses the earlier cast");
    assert_eq!(st.library, [CardIdx(1)]);
    assert_eq!(st.battlefield.len(), 1);
    assert_ne!(st.battlefield[0].uid, victim.uid);
    assert_eq!(st.battlefield[0].counters.plus1, 1);

    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    assert_eq!(st.hand, [CardIdx(1)], "a later cast does trigger it");
}

/// A surviving cast listener still receives its reward beside queued undying.
#[test]
fn cost_undying_preserves_listeners_that_were_present_for_the_cast() {
    let cards = deck(&[
        row(
            "Undying engine",
            "{9}",
            "Creature",
            "Undying\nWhenever you cast a spell, draw a card.",
        ),
        row(
            "Present engine",
            "{9}",
            "Artifact",
            "Whenever you cast a spell, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![2]);
    st.next_uid = 2;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));

    begin_resolution(&mut st);
    let victim = st.battlefield.remove(0);
    move_to_graveyard(&cards, &mut st, CardIdx(0), 1, CardZone::Battlefield);
    queue_undying(&cards, &mut st, &victim, 1);
    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    end_resolution(&cards, &mut st, None);

    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.seen, 1);
}

/// The committed cast path captures listeners before its sacrifice returns.
#[test]
fn additional_sacrifice_cast_does_not_reward_its_returned_engine() {
    let cards = deck(&[
        row(
            "Sacrifice spell",
            "{1}",
            "Sorcery",
            "As an additional cost to cast this spell, sacrifice a creature. Gain 1 life.",
        ),
        row(
            "Undying engine",
            "{9}",
            "Creature",
            "Undying\nWhenever you cast a spell, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![2]);
    st.next_uid = 7;
    st.battlefield
        .push(new_perm_with(7, &cards, CardIdx(1), 0, false));
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };

    cast(&cards, &mut st, &mut pool);

    assert!(st.hand.is_empty());
    assert_eq!(st.library, [CardIdx(2)]);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_ne!(st.battlefield[0].uid, 7);
    assert_eq!(st.battlefield[0].counters.plus1, 1);
    assert_eq!(st.spells_cast_this_turn, 1);
    assert_eq!(st.life, 21);
}

/// An activated ability's full ordered sequence finishes before its ETB draws.
#[test]
fn compound_activation_finishes_before_its_returned_creature_draws() {
    let cards = deck(&[
        row(
            "Source",
            "{1}",
            "Artifact",
            "{T}: Draw a card. Return target creature card from your graveyard to the battlefield. Mill a card.",
        ),
        row(
            "Draw creature",
            "{9}",
            "Creature",
            "When this creature enters, draw a card.",
        ),
        row("First draw", "{9}", "Sorcery", ""),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![4, 3, 2]);
    st.graveyard.push(CardIdx(1));
    st.next_uid = 1;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &st, &pool, 1).expect("tap draw is usable");

    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(st.hand, [CardIdx(2), CardIdx(4)]);
    assert_eq!(st.graveyard, [CardIdx(3)]);
    assert!(st.library.is_empty());
}

/// Undying from an activation cost resolves before the activated mill effect.
#[test]
fn activation_cost_undying_settles_before_the_ability_resolves() {
    let cards = deck(&[
        row(
            "Source",
            "{1}",
            "Artifact",
            "Sacrifice a creature: Mill a card.",
        ),
        row(
            "Undying creature",
            "{9}",
            "Creature",
            "Undying\nWhen this creature enters, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![3, 2]);
    st.next_uid = 2;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    let mut pool = ManaPool::default();
    let activation =
        pick_best_activation(&cards, &st, &pool, 1).expect("a creature can pay the cost");

    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(st.hand, [CardIdx(2)], "the undying ETB draws A first");
    assert_eq!(
        st.graveyard,
        [CardIdx(3)],
        "the activated ability then mills B"
    );
}

/// Battlefield recursion leaves a land in its graveyard instead of drawing it.
#[test]
fn battlefield_recursion_has_no_land_to_hand_fallback() {
    let cards = deck(&[
        row(
            "Return",
            "{1}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield.",
        ),
        row("Land", "", "Land", ""),
    ]);
    let mut st = state(Vec::new(), Vec::new());
    st.graveyard.push(CardIdx(1));

    assert!(!spell_targets_available(&cards, &st, &cards.cards[0]));
    apply_effect_at(
        &cards,
        &SimEffect::ReturnFromGraveyard {
            to_hand: false,
            count: 1,
        },
        &mut st,
        1,
        false,
        None,
    );

    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert!(st.hand.is_empty());
    assert!(st.battlefield.is_empty());
}

/// Recursion uses the returned card's entry rules rather than forcing a tap.
#[test]
fn graveyard_return_preserves_printed_tapped_entry() {
    for (text, tapped) in [("", false), ("This creature enters tapped.", true)] {
        let cards = deck(&[row("Creature", "{1}", "Creature", text)]);
        let mut st = state(Vec::new(), Vec::new());
        st.graveyard.push(CardIdx(0));

        apply_effect_at(
            &cards,
            &SimEffect::ReturnFromGraveyard {
                to_hand: false,
                count: 1,
            },
            &mut st,
            1,
            false,
            None,
        );

        assert_eq!(st.battlefield[0].tapped, tapped);
        assert!(st.graveyard.is_empty());
    }
}

/// A targetless required return never pays mana through the normal cast path.
#[test]
fn targetless_recursion_stays_in_hand_and_keeps_its_mana() {
    let cards = deck(&[
        row(
            "Return",
            "{1}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield.",
        ),
        row("Land", "", "Land", ""),
    ]);
    let mut st = state(vec![0], Vec::new());
    st.graveyard.push(CardIdx(1));
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let mut expected_rng = st.library_rng.clone();

    cast(&cards, &mut st, &mut pool);

    assert_eq!(st.hand, [CardIdx(0)]);
    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert_eq!(pool.total(), 1);
    assert_eq!(st.spells_cast_this_turn, 0);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
}

/// Recursion filters missing from the runtime model must not become substitutes.
#[test]
fn unsupported_recursion_filters_do_not_lower_to_unrestricted_returns() {
    for text in [
        "Return target artifact card from your graveyard to the battlefield.",
        "Return target creature card with mana value 3 or less from your graveyard to the battlefield.",
        "Return target creature card from your graveyard to the battlefield tapped.",
        "Return target land card from your graveyard to your hand.",
        "Return target permanent card from your graveyard to the battlefield.",
    ] {
        let cards = deck(&[row("Unsupported return", "{1}", "Sorcery", text)]);
        assert!(
            cards.cards[0]
                .spell_data
                .effects
                .iter()
                .all(|effect| !matches!(effect, SimEffect::ReturnFromGraveyard { .. })),
            "{text}"
        );
    }
}

/// Costs cannot replace a prepared creature target with a new graveyard card.
#[test]
fn prepared_return_keeps_its_pre_cost_target() {
    let cards = deck(&[
        row(
            "Return",
            "{1}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield.",
        ),
        row("Original target", "{9}", "Creature", ""),
        row("Sacrificed creature", "{9}", "Creature", ""),
    ]);
    let mut st = state(Vec::new(), Vec::new());
    st.graveyard.push(CardIdx(1));
    let mut targets = prepare_spell_targets(&cards, &st, &cards.cards[0]).expect("target exists");
    st.graveyard.push(CardIdx(2));

    apply_prepared_spell_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        Some(CardIdx(0)),
        &mut targets,
    );

    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(st.graveyard, [CardIdx(2)]);
}

/// A missing prepared target does not cause resolution to choose another one.
#[test]
fn removed_prepared_target_does_not_retarget() {
    let cards = deck(&[
        row(
            "Return",
            "{1}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield.",
        ),
        row("Original target", "{9}", "Creature", ""),
        row("Other creature", "{9}", "Creature", ""),
    ]);
    let mut st = state(Vec::new(), Vec::new());
    st.graveyard.push(CardIdx(1));
    let mut targets = prepare_spell_targets(&cards, &st, &cards.cards[0]).expect("target exists");
    st.graveyard = vec![CardIdx(2)];
    st.exile.push(CardIdx(1));

    apply_prepared_spell_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        Some(CardIdx(0)),
        &mut targets,
    );

    assert!(st.battlefield.is_empty());
    assert_eq!(st.graveyard, [CardIdx(2)]);
}

/// An ETB return with no target does not prevent casting its permanent.
#[test]
fn permanent_with_missing_etb_target_remains_castable() {
    let cards = deck(&[row(
        "Return creature",
        "{1}",
        "Creature",
        "When this creature enters, return target creature card from your graveyard to your hand.",
    )]);
    let mut st = state(vec![0], Vec::new());
    assert!(spell_targets_available(&cards, &st, &cards.cards[0]));
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };

    cast(&cards, &mut st, &mut pool);

    assert_eq!(st.battlefield.len(), 1);
    assert!(st.hand.is_empty());
}

/// A removed source keeps the captured trigger and its last-known counters.
#[test]
fn removed_source_trigger_uses_last_known_counters() {
    let mut cards = deck(&[row(
        "Source",
        "{1}",
        "Artifact",
        "Whenever you cast a spell, draw a card.",
    )]);
    cards.cards[0].striations[0].abilities[0].effect = SimEffect::BurdenLifeLoss;
    cards.cards[0].striations[0].abilities[0].effects.clear();
    let mut st = state(Vec::new(), Vec::new());
    let mut source = new_perm_with(1, &cards, CardIdx(0), 0, false);
    source.counters.burden = 3;
    st.battlefield.push(source);

    begin_resolution(&mut st);
    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    st.battlefield.clear();
    end_resolution(&cards, &mut st, None);

    assert_eq!(st.life, 17);
    assert_eq!(st.life_paid, 3);
}

/// A once-per-turn limit belongs to event capture, even before settlement.
#[test]
fn pending_events_reserve_once_per_turn_limit() {
    let mut cards = deck(&[
        row(
            "Source",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
    ]);
    cards.cards[0].striations[0].abilities[0].once_per_turn = true;
    let mut st = state(Vec::new(), vec![2, 1]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));

    begin_resolution(&mut st);
    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    end_resolution(&cards, &mut st, None);

    assert_eq!(st.hand, [CardIdx(1)]);
    assert_eq!(st.library, [CardIdx(2)]);
}

/// An intervening condition must hold at both event time and resolution time.
#[test]
fn intervening_conditions_are_checked_before_capture_and_resolution() {
    let mut cards = deck(&[
        row(
            "Source",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
    ]);
    cards.cards[0].striations[0].abilities[0].condition = Some(SimAbilityCondition::Raid);
    let mut st = state(Vec::new(), vec![1]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));

    begin_resolution(&mut st);
    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    st.attacked_this_turn = true;
    end_resolution(&cards, &mut st, None);
    assert!(
        st.hand.is_empty(),
        "a false event-time condition stays false"
    );

    begin_resolution(&mut st);
    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    st.attacked_this_turn = false;
    end_resolution(&cards, &mut st, None);
    assert!(st.hand.is_empty(), "resolution rechecks the condition");
}

/// Listeners that enter later cannot observe a prior battlefield entry.
#[test]
fn entering_listener_does_not_receive_a_prior_entry_event() {
    let cards = deck(&[
        row("Creature", "{1}", "Creature", ""),
        row(
            "Watcher",
            "{1}",
            "Artifact",
            "Whenever a creature enters under your control, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![2]);

    begin_resolution(&mut st);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 1, false));
    fire_on_enter(&cards, &mut st, 0, 1, false);
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 1, false));
    fire_on_enter(&cards, &mut st, 1, 1, false);
    end_resolution(&cards, &mut st, None);

    assert!(st.hand.is_empty());
    assert_eq!(st.library, [CardIdx(2)]);
}

/// Each created token produces one event, and nontoken filters reject them.
#[test]
fn created_tokens_fire_one_matching_entry_event_each() {
    let mut cards = deck(&[
        row(
            "Token watcher",
            "{1}",
            "Artifact",
            "Whenever a creature enters under your control, draw a card.",
        ),
        row(
            "Nontoken watcher",
            "{1}",
            "Artifact",
            "Whenever a nontoken creature enters under your control, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
        row("C", "{9}", "Sorcery", ""),
    ]);
    cards.cards[1].striations[0].abilities[0]
        .event_filter
        .nontoken = true;
    let mut st = state(Vec::new(), vec![4, 3, 2]);
    for (uid, index) in [(1, 0), (2, 1)] {
        st.battlefield
            .push(new_perm_with(uid, &cards, CardIdx(index), 0, false));
    }

    apply_effect_at(&cards, &SimEffect::Tokens(2), &mut st, 1, false, None);

    assert_eq!(st.battlefield.len(), 4);
    assert_eq!(st.hand, [CardIdx(2), CardIdx(3)]);
    assert_eq!(st.library, [CardIdx(4)]);
}

/// Afterlife Spirit tokens use the same entry-event capture path as other tokens.
#[test]
fn afterlife_tokens_fire_matching_entry_listeners() {
    let cards = deck(&[
        row(
            "Token watcher",
            "{1}",
            "Artifact",
            "Whenever a creature enters under your control, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![2, 1]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));

    apply_effect_at(&cards, &SimEffect::Afterlife(2), &mut st, 1, false, None);

    assert_eq!(st.hand, [CardIdx(1), CardIdx(2)]);
    assert_eq!(st.battlefield.len(), 3);
    assert!(
        st.battlefield[1..]
            .iter()
            .all(|token| matches!(token.card, super::super::game::CardRef::Token(_)))
    );
}

/// A trigger created during resolution settles before the next waiting trigger.
#[test]
fn nested_etb_settles_between_separate_waiting_abilities() {
    let mut cards = deck(&[
        row(
            "Return engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, return target creature card from your graveyard to the battlefield.",
        ),
        row(
            "Mill engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, mill a card.",
        ),
        row(
            "Draw creature",
            "{9}",
            "Creature",
            "When this creature enters, draw a card.",
        ),
        row("A", "{9}", "Sorcery", ""),
        row("B", "{9}", "Sorcery", ""),
    ]);
    cards.cards[0].striations[0].abilities[0].effect = SimEffect::ReturnFromGraveyard {
        to_hand: false,
        count: 1,
    };
    let mut st = state(Vec::new(), vec![4, 3]);
    st.graveyard.push(CardIdx(2));
    st.next_uid = 2;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));

    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);

    assert_eq!(st.hand, [CardIdx(3)], "the new trigger draws A");
    assert_eq!(st.graveyard, [CardIdx(4)], "the waiting trigger mills B");
}

/// A recursive supported trigger chain cannot hang the simulation.
#[test]
fn recursive_trigger_chain_stops_at_the_explicit_limit() {
    let mut cards = deck(&[row(
        "Ring engine",
        "{1}",
        "Artifact",
        "Whenever the Ring tempts you, draw a card.",
    )]);
    cards.cards[0].striations[0].abilities[0].effect = SimEffect::RingTempts;
    cards.cards[0].striations[0].abilities[0].effects.clear();
    let mut st = state(Vec::new(), Vec::new());
    st.current_turn = 1;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));

    fire_triggers(&cards, &mut st, SimTrigger::RingTempts, 1);

    assert!(st.resolution.trigger_limit_reached);
    assert_eq!(st.ring_tempts, MAX_TRIGGER_SETTLEMENTS as u32);
    assert!(st.resolution.pending.is_empty());
    assert!(
        matches!(
            &st.completion,
            Some(crate::deck::simulator::game::GameCompletion::Incomplete {
                reason: crate::deck::simulator::game::IncompleteReason::TriggerSettlement {
                    configured_limit: 1024,
                    remaining_work: 1,
                },
                turn: 1,
                phase: crate::deck::simulator::game::GamePhase::Upkeep,
            })
        ),
        "unexpected completion: {:?}",
        st.completion
    );
    let log = crate::deck::simulator::game_run::census::finish_log(
        &cards,
        st,
        crate::deck::simulator::game_run::census::TurnCensus::new(1, cards.cards.len()),
        0,
        false,
        1,
    );
    let stats = crate::deck::simulator::aggregate::aggregate(&[log], &cards, 1);
    assert_eq!(stats.runs, 1);
    assert_eq!(stats.finished_games, 0);
    assert_eq!(stats.incomplete_games, 1);
    assert_eq!(stats.incomplete_reasons["trigger_settlement_limit"], 1);
}

/// A finite queue that drains on the final allowed settlement is complete.
#[test]
fn trigger_queue_finishing_exactly_at_the_limit_is_not_incomplete() {
    let cards = deck(&[row(
        "Finite listener",
        "{1}",
        "Artifact",
        "Whenever the Ring tempts you, gain 1 life.",
    )]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    begin_resolution(&mut st);
    for _ in 0..MAX_TRIGGER_SETTLEMENTS {
        fire_triggers(&cards, &mut st, SimTrigger::RingTempts, 1);
    }
    end_resolution(&cards, &mut st, None);
    assert!(st.completion.is_none());
    assert!(!st.resolution.trigger_limit_reached);
    assert!(st.resolution.pending.is_empty());
    assert_eq!(st.life_gained, MAX_TRIGGER_SETTLEMENTS as u32);
}
