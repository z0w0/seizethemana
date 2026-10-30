use super::*;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::game::{
    GameCompletion, GamePhase, ManaPool, PlayerLossReason, new_perm_with,
};
use crate::deck::simulator::game_effects::{DrawOutcome, apply_effect_at, draw_one_outcome};
use crate::deck::simulator::model::{SimEffect, SimTrigger};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use crate::deck::simulator::turn_loop_tests::cast;
use rand::RngCore;

/// Resolve the compiled rules of one source as one complete spell action.
fn resolve_source(cards: &SimDeck, st: &mut GameState, source: usize, turn: u32) {
    begin_resolution(st);
    for effect in &cards.cards[source].spell_data.effects {
        apply_effect_at(cards, effect, st, turn, false, Some(CardIdx(source as u32)));
    }
    end_resolution(cards, st, None);
}

/// Scry can reorder only exposed cards and put rejected cards below the hidden library.
#[test]
fn scry_keeps_needed_lands_and_cheap_spells_without_draw_or_mill_credit() {
    let cards = deck(&[
        row("Expensive", "{9}", "Sorcery", ""),
        row("Land", "", "Land", ""),
        row("Cheap", "{1}", "Sorcery", ""),
        row("Hidden", "{3}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![3, 0, 2, 1]);
    let mut expected_rng = st.library_rng.clone();

    scry(&cards, &mut st, 3, 2);

    assert_eq!(st.library, [CardIdx(0), CardIdx(3), CardIdx(2), CardIdx(1)]);
    assert_eq!(st.awareness_cards, 3);
    assert_eq!(st.seen, 0);
    assert_eq!(st.milled_self, 0);
    assert!(st.hand.is_empty());
    assert!(st.graveyard.is_empty());
    assert!(st.card_first_seen.is_empty());
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
}

/// Surveil preserves the original order of kept cards and counts each exposed card once.
#[test]
fn surveil_rejects_only_unwanted_cards_and_preserves_kept_order() {
    let cards = deck(&[
        row("First kept", "{1}", "Sorcery", ""),
        row("Rejected", "{9}", "Sorcery", ""),
        row("Second kept", "{0}", "Sorcery", ""),
        row("Hidden", "{4}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![3, 2, 1, 0]);

    surveil(&cards, &mut st, 3, 4);

    assert_eq!(st.library, [CardIdx(3), CardIdx(2), CardIdx(0)]);
    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert_eq!(st.graveyard_seen[&CardIdx(1)], 4);
    assert_eq!(st.awareness_cards, 3);
    assert_eq!(st.seen, 1);
    assert_eq!(st.milled_self, 1);
    assert!(st.card_first_seen.is_empty());
}

/// A rejected surveil card still enters from the library and can fire its written trigger.
#[test]
fn surveil_uses_library_graveyard_events_but_discard_does_not() {
    let cards = deck(&[row(
        "Returner",
        "{9}",
        "Creature",
        "When Returner is put into your graveyard from your library, you may put it onto the battlefield.",
    )]);
    let mut milled = state(Vec::new(), vec![0]);
    surveil(&cards, &mut milled, 1, 2);
    assert!(milled.graveyard.is_empty());
    assert_eq!(milled.battlefield.len(), 1);
    assert_eq!(milled.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
    assert_eq!((milled.milled_self, milled.awareness_cards), (1, 1));

    let mut discarded = state(vec![0], Vec::new());
    draw_discard(&cards, &mut discarded, 0, 1, 2);
    assert_eq!(discarded.graveyard, [CardIdx(0)]);
    assert!(discarded.battlefield.is_empty());
    assert_eq!((discarded.milled_self, discarded.awareness_cards), (0, 0));
}

/// The land target counts visible battlefield and hand lands, not library lands.
#[test]
fn preference_stops_prioritizing_lands_at_five_visible_lands() {
    let cards = deck(&[
        row("Land A", "", "Land", ""),
        row("Land B", "", "Land", ""),
        row("Land C", "", "Land", ""),
        row("Land D", "", "Land", ""),
        row("Land E", "", "Land", ""),
        row("Candidate land", "", "Land", ""),
        row("Cheap", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0, 1, 2, 3], vec![6, 5]);
    assert!(preference(&cards, &st, CardIdx(5)) < preference(&cards, &st, CardIdx(6)));
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(4), 0, false));
    assert!(preference(&cards, &st, CardIdx(6)) < preference(&cards, &st, CardIdx(5)));

    scry(&cards, &mut st, 1, 1);

    assert_eq!(st.library, [CardIdx(5), CardIdx(6)]);
}

/// Candidate preference does not inspect the hidden library or consume random numbers.
#[test]
fn preference_and_exposed_reordering_ignore_hidden_library_order() {
    let cards = deck(&[
        row("Hidden land", "", "Land", ""),
        row("Hidden spell", "{0}", "Sorcery", ""),
        row("Shown expensive", "{9}", "Sorcery", ""),
        row("Shown cheap", "{1}", "Sorcery", ""),
    ]);
    let mut first = state(Vec::new(), vec![0, 1, 3, 2]);
    let mut second = state(Vec::new(), vec![1, 0, 3, 2]);
    assert_eq!(
        preference(&cards, &first, CardIdx(2)),
        preference(&cards, &second, CardIdx(2))
    );
    look_at_top(&cards, &mut first, 2, true, 1);
    look_at_top(&cards, &mut second, 2, true, 1);

    assert_eq!(&first.library[..2], &[CardIdx(0), CardIdx(1)]);
    assert_eq!(&second.library[..2], &[CardIdx(1), CardIdx(0)]);
    assert_eq!(&first.library[2..], &[CardIdx(2), CardIdx(3)]);
    assert_eq!(&second.library[2..], &[CardIdx(2), CardIdx(3)]);
    assert_eq!(first.library_rng.next_u64(), second.library_rng.next_u64());
}

/// Optional shuffles use the Library stream only when all exposed candidates are rejected.
#[test]
fn rejected_top_selection_shuffles_the_whole_library_with_its_own_stream() {
    let cards = deck(&[
        row("Hidden cheap", "{0}", "Sorcery", ""),
        row("Shown A", "{9}", "Sorcery", ""),
        row("Shown B", "{8}", "Sorcery", ""),
        row("Shown C", "{7}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![0, 1, 2, 3]);
    let mut expected = st.library.clone();
    let mut expected_rng = st.library_rng.clone();
    crate::deck::simulator::rng::shuffle(&mut expected, &mut expected_rng);

    look_at_top(&cards, &mut st, 3, true, 1);

    assert_eq!(st.library, expected);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
    assert_eq!(st.awareness_cards, 3);
    assert_eq!(st.seen, 0);
    assert_eq!(st.milled_self, 0);
    assert!(st.hand.is_empty());
}

/// A useful exposed card prevents a policy shuffle and hidden cards stay in place.
#[test]
fn useful_top_selection_reorders_without_shuffling() {
    let cards = deck(&[
        row("Hidden", "{9}", "Sorcery", ""),
        row("Expensive", "{8}", "Sorcery", ""),
        row("Land", "", "Land", ""),
        row("Cheap", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![0, 1, 2, 3]);
    let mut expected_rng = st.library_rng.clone();

    look_at_top(&cards, &mut st, 3, true, 1);

    assert_eq!(st.library, [CardIdx(0), CardIdx(1), CardIdx(3), CardIdx(2)]);
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
}

/// Looking at fewer cards than requested does not create draw failures or excess awareness.
#[test]
fn short_and_empty_libraries_bound_selection_work_and_awareness() {
    let cards = deck(&[
        row("Cheap", "{1}", "Sorcery", ""),
        row("Expensive", "{9}", "Sorcery", ""),
    ]);
    let mut looked = state(Vec::new(), vec![0, 1]);
    look_at_top(&cards, &mut looked, 5, false, 1);
    assert_eq!(looked.library, [CardIdx(1), CardIdx(0)]);
    assert_eq!(looked.awareness_cards, 2);
    assert!(!looked.failed_draw);

    let mut scried = state(Vec::new(), vec![0]);
    scry(&cards, &mut scried, 5, 1);
    assert_eq!(scried.awareness_cards, 1);
    assert_eq!(scried.library, [CardIdx(0)]);

    let mut empty = state(Vec::new(), Vec::new());
    let mut expected_rng = empty.library_rng.clone();
    look_at_top(&cards, &mut empty, 3, true, 1);
    surveil(&cards, &mut empty, 3, 1);
    scry(&cards, &mut empty, 3, 1);
    assert_eq!(empty.awareness_cards, 0);
    assert!(!empty.failed_draw);
    assert!(empty.completion.is_none());
    assert_eq!(empty.library_rng.next_u64(), expected_rng.next_u64());
}

/// Equal-count loot cannot dredge a card that is discarded later in the same effect.
#[test]
fn loot_finishes_all_draws_before_discarding_a_dredger() {
    let cards = deck(&[
        row("Dredger", "{9}", "Creature", "Dredge 1"),
        row("First draw", "{1}", "Sorcery", ""),
        row("Second draw", "{1}", "Sorcery", ""),
        row("Untouched", "{9}", "Sorcery", ""),
    ]);
    assert_eq!(cards.cards[0].dredge, Some(1));
    for effect in [
        SimEffect::DrawThenDiscard(2),
        SimEffect::DrawDiscard {
            draw: 2,
            discard: 2,
        },
    ] {
        let mut st = state(vec![0], vec![3, 1, 2]);

        apply_effect_at(&cards, &effect, &mut st, 3, false, None);

        assert_eq!(st.hand, [CardIdx(2)]);
        assert_eq!(st.graveyard, [CardIdx(0), CardIdx(1)]);
        assert_eq!(st.library, [CardIdx(3)]);
        assert_eq!(st.seen, 2);
        assert_eq!(st.milled_self, 0);
        assert!(
            st.milestones_by_turn
                .get(&3)
                .is_none_or(|turn| turn.dredge_uses == 0)
        );
    }
}

/// Fixed unequal counts remain unequal, and discards choose from the completed hand.
#[test]
fn unequal_loot_draws_two_then_discards_one_worst_card() {
    let cards = deck(&[
        row("Expensive", "{9}", "Sorcery", ""),
        row("Cheap", "{1}", "Sorcery", ""),
        row("Land", "", "Land", ""),
    ]);
    let mut st = state(vec![0], vec![1, 2]);

    apply_effect_at(
        &cards,
        &SimEffect::DrawDiscard {
            draw: 2,
            discard: 1,
        },
        &mut st,
        1,
        false,
        None,
    );

    assert_eq!(st.hand, [CardIdx(2), CardIdx(1)]);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (2, 2, 0));
}

/// A pending failed draw allows discards, but a completed loss stops later actions.
#[test]
fn failed_draw_allows_remaining_loot_instructions_before_the_loss_check() {
    let cards = deck(&[
        row("Old hand", "{9}", "Sorcery", ""),
        row("Only draw", "{1}", "Sorcery", ""),
        row("Kept land", "", "Land", ""),
    ]);
    let mut st = state(vec![0, 2], vec![1]);
    st.current_turn = 1;
    st.current_phase = GamePhase::Main;

    begin_resolution(&mut st);
    draw_discard(&cards, &mut st, 2, 2, 1);

    assert!(st.failed_draw);
    assert!(st.completion.is_none());
    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.graveyard, [CardIdx(0), CardIdx(1)]);
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (1, 1, 0));

    end_resolution(&cards, &mut st, None);

    assert_eq!(
        st.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::FailedDraw],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    put_hand_on_top(&cards, &mut st, 2, 1);
    scry(&cards, &mut st, 3, 1);
    surveil(&cards, &mut st, 3, 1);
    look_at_top(&cards, &mut st, 3, true, 1);
    draw_discard(&cards, &mut st, 0, 2, 1);
    assert_eq!(st.hand, [CardIdx(2)]);
    assert!(st.library.is_empty());
    assert_eq!(st.graveyard, [CardIdx(0), CardIdx(1)]);
    assert_eq!((st.seen, st.awareness_cards), (1, 1));
}

/// Later instructions can refill and draw from the library without canceling a pending loss.
#[test]
fn failed_draw_does_not_block_a_later_hand_return_and_draw_in_the_same_resolution() {
    let cards = deck(&[row("Returned card", "{1}", "Sorcery", "")]);
    let mut st = state(vec![0], Vec::new());
    begin_resolution(&mut st);

    assert!(matches!(
        draw_one_outcome(&cards, &mut st, 1),
        DrawOutcome::NotDrawn
    ));
    assert!(st.failed_draw);
    assert!(st.completion.is_none());
    put_hand_on_top(&cards, &mut st, 1, 1);
    assert!(st.hand.is_empty());
    assert_eq!(st.library, [CardIdx(0)]);
    assert!(matches!(
        draw_one_outcome(&cards, &mut st, 1),
        DrawOutcome::Drawn
    ));
    assert_eq!(st.hand, [CardIdx(0)]);
    assert!(st.library.is_empty());
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (1, 1, 0));
    assert!(st.completion.is_none());

    end_resolution(&cards, &mut st, None);

    assert!(matches!(
        st.completion,
        Some(GameCompletion::PlayerLost { .. })
    ));
}

/// Captured Brainstorm draws three before returning two from the full current hand.
#[test]
fn captured_brainstorm_returns_cards_without_discard_or_extra_draw_credit() {
    let cards = deck(&[
        named_row("Brainstorm"),
        row("Old expensive", "{9}", "Sorcery", ""),
        row("Old cheap", "{1}", "Sorcery", ""),
        row("Draw A", "{1}", "Sorcery", ""),
        row("Draw land", "", "Land", ""),
        row("Draw B", "{1}", "Sorcery", ""),
    ]);
    assert!(matches!(
        cards.cards[0].spell_data.effects.as_slice(),
        [SimEffect::Draw(3), SimEffect::PutHandOnTop(2)]
    ));
    let mut st = state(vec![1, 2], vec![3, 4, 5]);

    resolve_source(&cards, &mut st, 0, 2);

    assert_eq!(st.hand, [CardIdx(2), CardIdx(5), CardIdx(4)]);
    assert_eq!(st.library, [CardIdx(1), CardIdx(3)]);
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (3, 3, 0));
    assert!(st.graveyard.is_empty());
    assert!(st.graveyard_seen.is_empty());
}

/// Brainstorm draw actions can dredge, but hand-to-top moves cannot.
#[test]
fn captured_brainstorm_dredges_only_during_its_three_draw_actions() {
    let cards = deck(&[
        named_row("Brainstorm"),
        row("Dredger", "{9}", "Creature", "Dredge 2"),
        row("Draw A", "{1}", "Sorcery", ""),
        row("Draw B", "{1}", "Sorcery", ""),
        row("Mill A", "{9}", "Sorcery", ""),
        row("Mill B", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![2, 3, 4, 5]);
    st.graveyard.push(CardIdx(1));

    resolve_source(&cards, &mut st, 0, 2);

    assert_eq!(st.hand, [CardIdx(3)]);
    assert_eq!(st.library, [CardIdx(1), CardIdx(2)]);
    assert_eq!(st.graveyard, [CardIdx(5), CardIdx(4)]);
    assert_eq!(st.milestones_by_turn[&2].dredge_uses, 1);
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (4, 4, 2));
}

/// A full Brainstorm cast returns cards and leaves the stack before failed-draw loss.
#[test]
fn captured_brainstorm_finishes_its_hand_returns_before_short_library_loss() {
    let cards = deck(&[
        named_row("Brainstorm"),
        row("Old hand", "{9}", "Sorcery", ""),
        row("Only draw", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0, 1], vec![2]);
    st.current_turn = 1;
    st.current_phase = GamePhase::Main;
    let mut pool = ManaPool {
        flexible: 1,
        ..Default::default()
    };

    assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);

    assert!(st.failed_draw);
    assert_eq!(
        st.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::FailedDraw],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert!(st.hand.is_empty());
    assert_eq!(st.library, [CardIdx(1), CardIdx(2)]);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.graveyard_seen[&CardIdx(0)], 1);
    assert_eq!(st.card_first_seen[&2], 1);
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (1, 1, 0));
    assert_eq!(st.spells_cast_this_turn, 1);
    assert_eq!(pool.total(), 0);
}

/// A captured Ponder keeps all looked cards in the library before its separate draw.
#[test]
fn captured_ponder_reorders_only_its_exposed_cards_before_drawing() {
    let cards = deck(&[
        named_row("Ponder"),
        row("Hidden", "{0}", "Sorcery", ""),
        row("Expensive", "{9}", "Sorcery", ""),
        row("Land", "", "Land", ""),
        row("Cheap", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![1, 2, 3, 4]);
    let mut expected_rng = st.library_rng.clone();

    resolve_source(&cards, &mut st, 0, 1);

    assert_eq!(st.hand, [CardIdx(3)]);
    assert_eq!(st.library, [CardIdx(1), CardIdx(2), CardIdx(4)]);
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (1, 4, 0));
    assert!(st.graveyard.is_empty());
    assert_eq!(st.library_rng.next_u64(), expected_rng.next_u64());
}

/// Captured scry and surveil cantrips use physical selection before their draw action.
#[test]
fn captured_scry_and_surveil_cantrips_change_the_next_draw_legally() {
    let cards = deck(&[
        named_row("Preordain"),
        named_row("Consider"),
        row("Expensive", "{9}", "Sorcery", ""),
        row("Land", "", "Land", ""),
        row("Cheap", "{1}", "Sorcery", ""),
    ]);
    let mut scried = state(Vec::new(), vec![2, 3, 4]);
    resolve_source(&cards, &mut scried, 0, 1);
    assert_eq!(scried.hand, [CardIdx(3)]);
    assert_eq!(scried.library, [CardIdx(2), CardIdx(4)]);
    assert_eq!((scried.seen, scried.awareness_cards), (1, 3));

    let mut surveilled = state(Vec::new(), vec![4, 2]);
    resolve_source(&cards, &mut surveilled, 1, 1);
    assert_eq!(surveilled.hand, [CardIdx(4)]);
    assert_eq!(surveilled.graveyard, [CardIdx(2)]);
    assert_eq!(
        (
            surveilled.seen,
            surveilled.awareness_cards,
            surveilled.milled_self
        ),
        (2, 2, 1)
    );
}

/// Controller and self-targeted captured draw spells charge only the player's life.
#[test]
fn captured_draw_life_spells_draw_two_and_charge_only_self() {
    for name in ["Night's Whisper", "Sign in Blood"] {
        let cards = deck(&[
            named_row(name),
            row("Draw A", "{1}", "Sorcery", ""),
            row("Draw B", "{1}", "Sorcery", ""),
        ]);
        let mut st = state(Vec::new(), vec![1, 2]);

        resolve_source(&cards, &mut st, 0, 1);

        assert_eq!(st.hand, [CardIdx(2), CardIdx(1)], "{name}");
        assert_eq!(st.life, 18, "{name}");
        assert_eq!(st.opponent_life_lost, 0, "{name}");
        assert_eq!(st.seen, 2, "{name}");
    }
}

/// A different name uses the same compiled selection and draw-life rules.
#[test]
fn renamed_captured_sources_keep_the_same_runtime_behavior() {
    for name in ["Brainstorm", "Ponder", "Night's Whisper", "Sign in Blood"] {
        let mut source = named_row(name);
        source.name = "Unrelated source name".into();
        let cards = deck(&[
            source,
            row("Draw A", "{1}", "Sorcery", ""),
            row("Draw B", "{1}", "Sorcery", ""),
            row("Land", "", "Land", ""),
        ]);
        let mut st = state(Vec::new(), vec![1, 2, 3]);

        resolve_source(&cards, &mut st, 0, 1);

        let expected_hand_size = if name == "Night's Whisper" || name == "Sign in Blood" {
            2
        } else {
            1
        };
        assert_eq!(st.hand.len(), expected_hand_size, "{name}");
        assert_eq!(st.hand.len() + st.library.len(), 3, "{name}");
        assert!(st.graveyard.is_empty(), "{name}");
    }
}

/// Opponent-only draws cannot grant cards to the controller through the selection grammar.
#[test]
fn opponent_draw_subjects_do_not_become_self_draws() {
    let cards = deck(&[
        row(
            "Opponent draw",
            "{1}",
            "Sorcery",
            "Target opponent draws two cards and loses 2 life.",
        ),
        row("Untouched", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![1]);

    resolve_source(&cards, &mut st, 0, 1);

    assert!(st.hand.is_empty());
    assert_eq!(st.library, [CardIdx(1)]);
    assert_eq!(st.life, 20);
    assert_eq!(st.seen, 0);
}

/// The captured equal-count loot spell reaches the same all-draws-first helper.
#[test]
fn captured_faithless_looting_discards_from_the_completed_hand() {
    let cards = deck(&[
        named_row("Faithless Looting"),
        row("Old expensive", "{9}", "Sorcery", ""),
        row("Draw cheap", "{1}", "Sorcery", ""),
        row("Draw land", "", "Land", ""),
    ]);
    let mut st = state(vec![1], vec![2, 3]);

    resolve_source(&cards, &mut st, 0, 1);

    assert_eq!(st.hand, [CardIdx(3)]);
    assert_eq!(st.graveyard, [CardIdx(1), CardIdx(2)]);
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (2, 2, 0));
}

/// A full loot cast completes its discards and leaves the stack before failed-draw loss.
#[test]
fn captured_faithless_looting_finishes_discards_before_short_library_loss() {
    let cards = deck(&[
        named_row("Faithless Looting"),
        row("Old expensive", "{9}", "Sorcery", ""),
        row("Only draw", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0, 1], vec![2]);
    st.current_turn = 1;
    st.current_phase = GamePhase::Main;
    let mut pool = ManaPool {
        flexible: 1,
        ..Default::default()
    };

    assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);

    assert!(st.failed_draw);
    assert_eq!(
        st.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::FailedDraw],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert!(st.hand.is_empty());
    assert!(st.library.is_empty());
    assert_eq!(st.graveyard, [CardIdx(1), CardIdx(2), CardIdx(0)]);
    for index in [CardIdx(0), CardIdx(1), CardIdx(2)] {
        assert_eq!(st.graveyard_seen[&index], 1);
    }
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (1, 1, 0));
    assert_eq!(st.spells_cast_this_turn, 1);
    assert_eq!(pool.total(), 0);
}

/// The captured Arena trigger retains both effects, including lethal self-life loss.
#[test]
fn captured_arena_draws_before_its_lethal_life_loss() {
    let cards = deck(&[
        named_row("Phyrexian Arena"),
        row("Drawn", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![1]);
    st.life = 1;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));

    crate::deck::simulator::game::fire_triggers(&cards, &mut st, SimTrigger::Upkeep, 1);

    assert_eq!(st.hand, [CardIdx(1)]);
    assert_eq!(st.life, 0);
    assert_eq!(st.opponent_life_lost, 0);
    assert!(matches!(
        st.completion,
        Some(GameCompletion::PlayerLost { .. })
    ));
}

/// Returning cards to the top is not a draw and does not create a dredge opportunity.
#[test]
fn hand_returns_preserve_zones_and_do_not_change_draw_metadata() {
    let cards = deck(&[
        row("Worst", "{9}", "Sorcery", ""),
        row("Cheap", "{1}", "Sorcery", ""),
        row("Land", "", "Land", ""),
        row("Graveyard dredger", "{8}", "Creature", "Dredge 1"),
    ]);
    let mut st = state(vec![0, 1, 2], Vec::new());
    st.graveyard.push(CardIdx(3));

    put_hand_on_top(&cards, &mut st, 2, 1);

    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.library, [CardIdx(0), CardIdx(1)]);
    assert_eq!(st.graveyard, [CardIdx(3)]);
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (0, 0, 0));
    assert!(st.milestones_by_turn.is_empty());
    assert!(st.graveyard_seen.is_empty());
    assert!(matches!(
        draw_one_outcome(&cards, &mut st, 1),
        DrawOutcome::Replaced
    ));
    assert_eq!(st.milestones_by_turn[&1].dredge_uses, 1);
}
