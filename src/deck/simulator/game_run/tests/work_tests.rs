use super::*;
use crate::deck::simulator::turn_loop_tests::{deck, row, state};
use rand::RngCore;

/// A stable state at a work boundary is complete, not automatically capped.
#[test]
fn empty_action_probe_has_no_remaining_work() {
    let cards = deck(&[]);
    let st = state(vec![], vec![]);
    assert!(!remaining_main_work(
        &cards,
        &st,
        &ManaPool::default(),
        &TurnCensus::new(1, 0),
        &CommanderProfile::new(&cards),
        &[],
        1
    ));
}

/// Legal work can be demonstrated without casting, shuffling, or spending in the real game.
#[test]
fn legal_cast_probe_preserves_all_real_resources() {
    let cards = deck(&[
        row(
            "Search",
            "{0}",
            "Sorcery",
            "Search your library for a card, put that card into your hand, then shuffle.",
        ),
        row("Target", "{20}", "Sorcery", ""),
    ]);
    let st = state(vec![0], vec![1]);
    let pool = ManaPool::default();
    let census = TurnCensus::new(1, cards.cards.len());
    let mut expected_rng = st.library_rng.clone();
    assert!(remaining_main_work(
        &cards,
        &st,
        &pool,
        &census,
        &CommanderProfile::new(&cards),
        &[],
        1
    ));
    assert_eq!(st.hand, [CardIdx(0)]);
    assert_eq!(st.library, [CardIdx(1)]);
    assert!(st.graveyard.is_empty());
    assert_eq!(st.spells_cast_this_turn, 0);
    assert_eq!(pool.total(), 0);
    assert_eq!(census.mana_spent, [0.0]);
    let mut actual_rng = st.library_rng.clone();
    assert_eq!(actual_rng.next_u64(), expected_rng.next_u64());
}

/// An older identity land can use a new grant without spending anything during the probe.
#[test]
fn granted_only_old_land_probe_preserves_real_resources() {
    use crate::deck::simulator::card_test_support::{
        assert_library_rng, main_phase_state, named_row, pinned_revision,
    };
    use crate::deck::simulator::library_core_report_tests::{cast_action, put_permanents};

    let source = named_row("Command Tower");
    pinned_revision(&source);
    let cards = deck(&[
        source,
        row(
            "Synthetic new land grant",
            "{99}",
            "Enchantment",
            "Lands you control have \"{T}: Add one mana of any color.\"",
        ),
        row("Synthetic green work", "{G}", "Sorcery", ""),
        row("Retained work library", "{99}", "Sorcery", ""),
        row("Retained work graveyard", "{99}", "Sorcery", ""),
        row("Retained work exile", "{99}", "Sorcery", ""),
    ]);
    assert_eq!(cards.commander_color_identity(), [false; 5]);
    for provider in [false, true] {
        for tapped in [false, true] {
            let mut st = main_phase_state(vec![2], vec![3]);
            put_permanents(&cards, &mut st, &[0]);
            st.battlefield[0].tapped = tapped;
            if provider {
                put_permanents(&cards, &mut st, &[1]);
                st.battlefield[1].entered_turn = 1;
            }
            st.graveyard.push(CardIdx(4));
            st.exile.push(CardIdx(5));
            let before = st.clone();
            let pool = ManaPool::default();
            let census = TurnCensus::new(1, cards.cards.len());
            let sources = Vec::new();
            let legal = provider && !tapped;
            assert_eq!(
                remaining_main_work(
                    &cards,
                    &st,
                    &pool,
                    &census,
                    &CommanderProfile::new(&cards),
                    &sources,
                    1
                ),
                legal
            );
            assert_eq!(st.hand, before.hand);
            assert_eq!(st.library, before.library);
            assert_eq!(st.graveyard, before.graveyard);
            assert_eq!(st.exile, before.exile);
            assert_eq!(
                format!("{:?}", st.battlefield),
                format!("{:?}", before.battlefield)
            );
            assert_eq!(st.life, before.life);
            assert_eq!(st.life_paid, before.life_paid);
            assert_eq!(st.player_counters, before.player_counters);
            assert_eq!(st.activated_this_turn, before.activated_this_turn);
            assert_eq!(st.completion, before.completion);
            assert_eq!(pool, ManaPool::default());
            assert_eq!(census.mana_spent, [0.0]);
            assert!(sources.is_empty());
            assert_library_rng(&st.library_rng, &before.library_rng);
            let mut paid = ManaPool::default();
            crate::deck::simulator::game_effects::activation::spend_leftover(
                &cards, &mut st, &mut paid, 1,
            );
            assert_eq!(paid.total(), u32::from(legal));
            assert_eq!(
                cast_action(&cards, &mut st, &mut paid),
                f64::from(u32::from(legal))
            );
            assert_eq!(paid.total(), 0);
        }
    }
}
