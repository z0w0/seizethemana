use super::mana_modes_tests::with_land;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::game::{GameState, new_perm_with};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_mana::payable;
use crate::deck::simulator::game_run::mana::{build_pool, tap_new_rocks};
use crate::deck::simulator::game_run::{TurnCensus, run_turn};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::scenario_test_support::{deck, row};
use crate::deck::simulator::turn_loop_tests::cast;

/// Put ready one-mana creatures beside a typed same-type nonland mana bonus.
fn bonus_board(mana: &str, cost: &str, dorks: u32) -> (SimDeck, GameState) {
    let cards = deck(&[
        named_row("Adarkar Wastes"),
        row("Dork", "{1}", "Creature", &format!("{{T}}: Add {mana}.")),
        row(
            "Mana bonus",
            "{9}",
            "Enchantment",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        ),
        row("Spell", cost, "Sorcery", ""),
    ]);
    assert!(cards.cards[2].flags.bonus_mana_on_nonland_tap);
    let mut st = with_land(&cards, vec![3]);
    for uid in 2..dorks + 2 {
        let mut dork = new_perm_with(uid, &cards, CardIdx(1), 0, false);
        dork.summoning_sick = false;
        st.battlefield.push(dork);
    }
    st.battlefield
        .push(new_perm_with(dorks + 2, &cards, CardIdx(2), 0, false));
    st.next_uid = dorks + 2;
    (cards, st)
}

/// Real bonus mana can avoid pain only when it can pay the missing symbol.
#[test]
fn real_bonus_mana_is_checked_before_a_painful_land_mode() {
    for (mana, cost, life) in [("{U}", "{U}{U}", 20), ("{G}", "{1}{U}", 19)] {
        let (cards, mut st) = bonus_board(mana, cost, 1);
        let mut pool = build_pool(&cards, &mut st, 1);
        assert_eq!(st.life, life, "{mana} bonus for {cost}");
        assert_eq!(st.life_paid, 0);
        assert_eq!(pool.total(), 3);
        assert_eq!(pool.colorless, u32::from(mana == "{U}"));
        assert!(st.battlefield[0].tapped && st.battlefield[1].tapped);
        assert!(!st.battlefield[2].tapped);
        assert!(payable(&parse_cost(cost), &pool));
        assert_eq!(payable(&parse_cost("{U}{U}"), &pool), mana == "{U}");
        assert_eq!(cast(&cards, &mut st, &mut pool), [2.0]);
        assert_eq!(pool.total(), 1);
        assert_eq!(st.life, life);
        assert!(st.hand.is_empty());
        assert_eq!(st.graveyard, [CardIdx(3)]);
    }
}

/// The normal turn uses two produced blue mana before considering colored land damage.
#[test]
fn full_turn_bonus_mana_avoids_pain_and_preserves_nonmatching_color_costs() {
    for (mana, cost, life) in [("{U}", "{U}{U}", 20), ("{G}", "{1}{U}", 19)] {
        let (cards, mut st) = bonus_board(mana, cost, 1);
        let mut census = TurnCensus::new(1, cards.cards.len());
        run_turn(
            &cards,
            &mut st,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut Vec::new(),
            1,
            false,
        );
        assert!(st.completion.is_none());
        assert!(st.hand.is_empty());
        assert_eq!(st.graveyard, [CardIdx(3)]);
        assert!(st.library.is_empty() && st.exile.is_empty());
        assert_eq!(st.life, life, "{mana} bonus for {cost}");
        assert_eq!(st.life_paid, 0);
        assert!(st.battlefield[0].tapped && st.battlefield[1].tapped);
        assert!(!st.battlefield[2].tapped);
        assert_eq!(census.mana_available, [3.0]);
        assert_eq!(census.mana_spent, [2.0]);
        assert_eq!(census.mana_remaining, [1.0]);
    }
}

/// Replanning after the first bonus prevents a second planned dork activation.
#[test]
fn a_painless_bonus_replaces_a_later_dork_tap_in_the_same_plan() {
    let (cards, mut st) = bonus_board("{U}", "{U}{U}", 2);
    let mut pool = build_pool(&cards, &mut st, 1);
    let tapped_dorks = |st: &GameState| {
        st.battlefield
            .iter()
            .filter(|source| source.card.deck_idx() == Some(CardIdx(1)) && source.tapped)
            .count()
    };
    assert_eq!(tapped_dorks(&st), 1);
    assert_eq!(st.life, 20);
    assert_eq!(st.life_paid, 0);
    assert_eq!(pool.total(), 3);
    assert_eq!(pool.colorless, 1);
    let before = pool.clone();
    tap_new_rocks(&cards, &mut st, &mut pool, 1);
    assert_eq!(pool, before);
    assert_eq!(tapped_dorks(&st), 1);
    assert_eq!(cast(&cards, &mut st, &mut pool), [2.0]);
    assert_eq!(pool.colorless, 1);
    assert_eq!(pool.total(), 1);
    assert!(st.hand.is_empty());
    tap_new_rocks(&cards, &mut st, &mut pool, 1);
    assert_eq!(pool.total(), 1);
    assert_eq!(tapped_dorks(&st), 1);
    assert_eq!(st.life, 20);
}
