//! Tests for the per-game turn loop.
use super::cast_pass::cast_pass;
use super::deal::{bottom_position, count_lands_in, take_n};
use super::game::{Activation, GameState, ManaPool, fire_on_enter, new_perm_with};
use super::game_commander::CommanderProfile;
use super::game_effects::activation::{pick_best_activation, resolve_activation};
use super::game_effects::spend_leftover;
use super::game_run::{
    TurnCensus, build_pool, run_sagas, run_turn, tap_dorks_for_mana, tap_new_rocks,
};
use super::model::CardIdx;
use super::model::{Format, SimAbility, SimDeck, SimEffect, SimStriation, SimTrigger};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::cost::parse_cost;
/// Keep existing turn-test imports on the shared scenario setup.
pub(super) use super::scenario_test_support::{deck, row, state};
use super::{game, game_effects, game_mana, game_run, model};

/// Mana-source costs, restrictions, and triggered effects.
#[path = "turn_loop_mana_tests.rs"]
mod turn_loop_mana_tests;

/// Additional land allowances during the turn loop.
#[path = "turn_loop_land_tests.rs"]
mod turn_loop_land_tests;

/// Run the turn-one cast phase and return its mana spend.
pub(super) fn cast(deck: &SimDeck, st: &mut GameState, pool: &mut ManaPool) -> [f64; 1] {
    let mut spent = [0.0];
    cast_pass(
        deck,
        st,
        pool,
        1,
        &mut spent,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    );
    spent
}

/// Produce colored-choice mana with one explicit cast restriction.
pub(super) fn restricted_pool(
    amount: u32,
    restriction: super::model::SpendRestriction,
) -> ManaPool {
    let mut pool = ManaPool::default();
    super::game_mana::add_yield(
        &super::model::ManaYield {
            any_pips: amount,
            restriction: Some(restriction),
            ..Default::default()
        },
        &mut pool,
    );
    pool
}

#[test]
fn cantrip_draws_and_casts_payable_spell_in_same_main_phase() {
    let cards = deck(&[
        row("Cantrip", "{U}", "Instant", "Draw a card."),
        row("Follow-up", "{U}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![1]);
    let mut pool = ManaPool {
        flexible: 2,
        ..ManaPool::default()
    };

    let spent = cast(&cards, &mut st, &mut pool);

    assert!(st.hand.is_empty());
    assert_eq!(
        st.graveyard,
        [0, 1]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(spent, [2.0]);
    assert_eq!(pool.total(), 0);
}

#[test]
fn ritual_funds_a_spell_that_was_initially_unaffordable() {
    let cards = deck(&[
        row("Ritual", "{R}", "Instant", "Add {R}{R}."),
        row("Payoff", "{1}{R}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0, 1], vec![]);
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 1, 0],
        ..ManaPool::default()
    };

    let spent = cast(&cards, &mut st, &mut pool);

    assert!(st.hand.is_empty());
    assert_eq!(
        st.graveyard,
        [0, 1]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(spent, [3.0]);
    assert_eq!(pool.total(), 0);
}

#[test]
fn color_and_restricted_mana_remain_available_for_the_right_second_cast() {
    let cards = deck(&[
        row("Creature", "{R}", "Creature — Human", ""),
        row("Instant", "{R}", "Instant", ""),
    ]);
    let mut st = state(vec![0, 1], vec![]);
    let mut pool = restricted_pool(1, super::model::SpendRestriction::Creature);
    pool.fixed[3] = 1;

    let spent = cast(&cards, &mut st, &mut pool);

    assert_eq!(spent, [2.0]);
    assert_eq!(
        st.graveyard,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(
        st.battlefield[0].card,
        crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(0))
    );
    assert_eq!(pool.total(), 0);

    let mut restricted_only = state(vec![0, 1], vec![]);
    let mut pool = restricted_pool(1, super::model::SpendRestriction::Creature);
    let _ = cast(&cards, &mut restricted_only, &mut pool);
    assert_eq!(restricted_only.battlefield.len(), 1);
    assert_eq!(
        restricted_only.hand,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(pool.total(), 0);
}

#[test]
fn newly_cast_rock_taps_once_for_a_follow_up_cast() {
    let cards = deck(&[
        row("Mana Rock", "{1}", "Artifact", "{T}: Add {C}."),
        row("Follow-up", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0, 1], vec![]);
    let mut pool = ManaPool {
        colorless: 1,
        ..ManaPool::default()
    };
    let mut spent = cast(&cards, &mut st, &mut pool);
    assert_eq!(
        st.hand,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(pool.total(), 0);

    tap_new_rocks(&cards, &mut st, &mut pool, 1);
    spent[0] += cast(&cards, &mut st, &mut pool)[0];

    assert_eq!(
        st.graveyard,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(st.hand.is_empty());
    assert_eq!(spent, [2.0]);
    assert!(st.battlefield[0].tapped);
    tap_new_rocks(&cards, &mut st, &mut pool, 1);
    assert_eq!(pool.total(), 0, "the same rock cannot produce mana twice");
}

#[test]
fn tapped_and_summoning_sick_sources_cannot_produce_mana() {
    let cards = deck(&[
        row("Mana Rock", "{1}", "Artifact", "{T}: Add {C}."),
        row("Mana Dork", "{G}", "Creature — Elf", "{T}: Add {G}."),
    ]);
    let mut st = state(vec![], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    st.battlefield.push(super::game::new_perm_with(
        2,
        &cards,
        crate::deck::simulator::model::CardIdx(1),
        1,
        false,
    ));
    st.battlefield[1].summoning_sick = true;
    st.battlefield.push(super::game::new_perm_with(
        3,
        &cards,
        crate::deck::simulator::model::CardIdx(1),
        0,
        false,
    ));
    st.battlefield[2].tapped = true;

    let mut pool = build_pool(&cards, &mut st, 2);
    assert_eq!(pool.total(), 1);
    assert!(st.battlefield[0].tapped);
    tap_dorks_for_mana(&cards, &mut st, &mut pool, 2);
    assert_eq!(pool.total(), 1);
    assert!(
        !st.battlefield[1].tapped,
        "summoning sickness prevents tapping"
    );
    assert!(
        st.battlefield[2].tapped,
        "an already tapped source stays spent"
    );
}

#[test]
fn healthy_dork_taps_when_the_pool_cannot_pay_the_cheapest_spell() {
    let cards = deck(&[
        row("Mana Dork", "{G}", "Creature — Elf", "{T}: Add {G}."),
        row("Bear", "{1}{G}", "Creature — Bear", ""),
    ]);
    let mut st = state(vec![1], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        0,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    st.battlefield[0].summoning_sick = false;
    let mut pool = ManaPool::default();

    tap_dorks_for_mana(&cards, &mut st, &mut pool, 1);

    assert_eq!(pool.fixed[4], 1, "the dork joins the pool on demand");
    assert!(st.battlefield[0].tapped, "the dork is tapped for mana");
}

#[test]
fn only_a_capped_positive_mana_loop_sets_the_infinite_flag() {
    let mut cards = deck(&[
        row("Free Mana", "{1}", "Artifact", "{0}: Add {C}."),
        row("Free Draw", "{1}", "Artifact", "{0}: Draw a card."),
        row("No Progress", "{1}", "Artifact", ""),
    ]);
    let mut mana_state = state(vec![], vec![]);
    mana_state.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    let mut pool = ManaPool::default();
    spend_leftover(&cards, &mut mana_state, &mut pool, 1);
    assert!(mana_state.infinite_mana_suspected);
    assert_eq!(pool.colorless, 24);
    assert!(mana_state.milestones_by_turn[&1].positive_mana_loop);

    let mut draw_state = state(vec![], vec![0; 30]);
    draw_state.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(1),
        0,
        false,
    ));
    spend_leftover(&cards, &mut draw_state, &mut ManaPool::default(), 1);
    assert!(!draw_state.infinite_mana_suspected);

    cards.cards[2].striations.push(SimStriation {
        abilities: vec![SimAbility {
            kind: super::model::SimAbilityKind::Activated,
            trigger: SimTrigger::Never,
            effect: SimEffect::None,
            ..SimAbility::default()
        }],
        ..SimStriation::default()
    });
    let mut no_progress_state = state(vec![], vec![]);
    no_progress_state
        .battlefield
        .push(super::game::new_perm_with(
            1,
            &cards,
            crate::deck::simulator::model::CardIdx(2),
            0,
            false,
        ));
    spend_leftover(&cards, &mut no_progress_state, &mut ManaPool::default(), 1);
    assert!(!no_progress_state.infinite_mana_suspected);

    let one_shot_rocks = (0..24)
        .map(|index| {
            row(
                &format!("One-shot rock {index}"),
                "{0}",
                "Artifact",
                "{T}: Add {C}.",
            )
        })
        .collect::<Vec<_>>();
    let one_shot_deck = deck(&one_shot_rocks);
    let mut one_shot_state = state(Vec::new(), Vec::new());
    for index in 0..one_shot_deck.cards.len() {
        one_shot_state.battlefield.push(new_perm_with(
            index as u32,
            &one_shot_deck,
            CardIdx(index as u32),
            0,
            false,
        ));
    }
    let mut one_shot_pool = ManaPool::default();
    spend_leftover(&one_shot_deck, &mut one_shot_state, &mut one_shot_pool, 1);
    assert_eq!(one_shot_pool.colorless, 24);
    assert!(!one_shot_state.infinite_mana_suspected);
}

#[test]
fn london_bottom_is_drawn_only_after_every_other_card() {
    let cards = deck(&[
        row("Land", "", "Land", "{T}: Add {W}."),
        row("Spell", "{1}", "Sorcery", ""),
        row("Land two", "", "Land", "{T}: Add {W}."),
        row("Spell two", "{1}", "Sorcery", ""),
        row("Land three", "", "Land", "{T}: Add {W}."),
        row("Spell three", "{1}", "Sorcery", ""),
        row("Land four", "", "Land", "{T}: Add {W}."),
        row("Spell four", "{1}", "Sorcery", ""),
    ]);
    // Four lands and three spells: the selected bottom card is a land.
    let mut library = (0..8)
        .map(crate::deck::simulator::model::CardIdx)
        .collect::<Vec<_>>();
    let mut hand = take_n(&mut library, 7);
    let land_count = count_lands_in(&cards, &hand);
    let bottomed_pos = bottom_position(&cards, &hand, land_count);
    let bottomed_card = hand.remove(bottomed_pos);
    library.insert(0, bottomed_card);
    assert_eq!(hand.len(), 6);
    let bottomed = library[0];

    assert_eq!(
        library.pop(),
        Some(crate::deck::simulator::model::CardIdx(0))
    );
    assert_eq!(library.pop(), Some(bottomed));
}

#[test]
fn constructed_skips_first_draw_and_commander_draws_on_turn_one() {
    let mut cards = deck(&vec![row("Blank", "{9}", "Sorcery", ""); 8]);
    for (format, expected) in [(Format::Constructed, 7), (Format::Commander, 8)] {
        cards.format = format;
        cards.rules = super::format::rules_for(if format == Format::Commander {
            "commander"
        } else {
            "constructed"
        });
        let mut st = state((0..7).collect(), vec![7]);
        st.seen = 7;
        let mut census = TurnCensus::new(1, cards.cards.len());
        let commander = CommanderProfile::new(&cards);
        run_turn(
            &cards,
            &mut st,
            &mut census,
            &commander,
            &mut Vec::new(),
            1,
            false,
        );
        assert_eq!(st.seen, expected);
        assert_eq!(census.cards_seen, [expected]);
    }
}

#[test]
fn monarch_who_gains_the_title_on_turn_one_draws_at_that_end_step() {
    let deck = deck(&[row("Card", "{9}", "Sorcery", "")]);
    let mut st = state(vec![], vec![0]);
    st.is_monarch = true;
    let mut census = TurnCensus::new(1, 1);

    super::game_run::ending_phase(&deck, &mut st, &mut census, 1, false);

    assert_eq!(st.hand, [CardIdx(0)]);
}

#[test]
fn end_step_life_loss_is_in_the_current_turn_census() {
    let deck = deck(&[row(
        "End Step Drain",
        "{0}",
        "Enchantment",
        "At the beginning of your end step, each opponent loses 2 life.",
    )]);
    let mut st = state(vec![], vec![]);
    let uid = super::game::take_uid(&mut st);
    st.battlefield
        .push(new_perm_with(uid, &deck, CardIdx(0), 1, false));
    let mut census = TurnCensus::new(1, deck.cards.len());
    let commander = CommanderProfile::new(&deck);
    let mut repeatable_sources = Vec::new();

    run_turn(
        &deck,
        &mut st,
        &mut census,
        &commander,
        &mut repeatable_sources,
        1,
        false,
    );

    assert_eq!(census.opponent_life_loss[0], 2);
    assert_eq!(census.player_damage[0], 0, "life loss is not damage");
}

#[test]
fn newly_entered_creature_vehicle_can_use_its_crew_ability() {
    let deck = deck(&[
        row(
            "Living vehicle",
            "{3}",
            "Artifact — Vehicle",
            "Living metal\nCrew 2",
        ),
        row("Crew member", "{1}", "Creature — Pilot", ""),
    ]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, false));
    st.battlefield
        .push(new_perm_with(2, &deck, CardIdx(1), 1, false));
    assert!(st.battlefield[0].summoning_sick);
    assert!(st.battlefield[1].summoning_sick);

    super::game_effects::tap_budget(&deck, &mut st.battlefield, &mut ManaPool::default(), &[]);

    assert!(st.battlefield[0].crewed);
    assert!(st.battlefield[1].tapped);
}

#[test]
fn haste_allows_a_new_creature_to_pay_its_tap_activation_cost() {
    let deck = deck(&[row(
        "Hasty dork",
        "{G}",
        "Creature — Elf",
        "Haste\n{T}: Add {G}.",
    )]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &deck, CardIdx(0), 1, false));
    st.battlefield[0].summoning_sick = true;

    let selected = pick_best_activation(&deck, &st, &ManaPool::default(), 1);

    assert!(selected.is_some());
}

#[test]
fn basalt_monolith_loop_is_positive_only_with_mana_trigger() {
    let kinnan = row(
        "Mana trigger",
        "{G}{U}",
        "Legendary Creature — Human Druid",
        "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.\n{5}{G}{U}: Look at the top five cards of your library. You may put a non-Human creature card from among them onto the battlefield. Put the rest on the bottom of your library in a random order.",
    );
    let basalt = row(
        "Basalt Monolith",
        "{3}",
        "Artifact",
        "Basalt Monolith doesn't untap during your untap step.\n{T}: Add {C}{C}{C}.\n{3}: Untap this artifact.",
    );
    let ability = parse_sim_card(&basalt)
        .unlocked_abilities(0)
        .find(|ability| matches!(ability.effect, SimEffect::UntapSelf))
        .expect("the source-named untap activation parses")
        .clone();
    for (has_kinnan, expected_pool, positive) in [(false, 3, false), (true, 9, true)] {
        let rows = if has_kinnan {
            vec![kinnan.clone(), basalt.clone()]
        } else {
            vec![basalt.clone()]
        };
        let deck = deck(&rows);
        let basalt_index = usize::from(has_kinnan);
        let mut st = state(vec![], vec![]);
        for index in 0..deck.cards.len() {
            st.battlefield.push(super::game::new_perm_with(
                index as u32,
                &deck,
                CardIdx(index as u32),
                1,
                false,
            ));
        }
        let mut pool = build_pool(&deck, &mut st, 1);
        let starting_pool = pool.total();
        for _ in 0..5 {
            let activation = Activation {
                pos: basalt_index,
                uid: basalt_index as u32,
                ability: ability.clone(),
                ability_cost: super::model::Cost {
                    generic: 3,
                    ..Default::default()
                },
                draws: 0,
                paid_x: 0,
                sacrifice_uid: None,
                target_uid: None,
            };
            resolve_activation(&deck, &mut st, &mut pool, 1, &activation);
            assert!(!st.battlefield[basalt_index].tapped);
            tap_new_rocks(&deck, &mut st, &mut pool, 1);
        }
        assert_eq!(pool.total(), expected_pool);
        assert!(st.battlefield[basalt_index].tapped);
        assert_eq!(pool.total() > 3, positive);
        assert_eq!(
            pool.total() - starting_pool,
            if positive { 5 } else { 0 },
            "five untap cycles change the pool by +5 only with Kinnan"
        );
    }
}

/// Keep complete remainder wording inert and retain a separate bare-selection control.
pub(super) fn bare_bounded_search_control(mut source: crate::db::CardRow) -> crate::db::CardRow {
    let full = deck(&[source.clone()]);
    assert!(full.cards[0].unlocked_abilities(0).all(|ability| {
        !ability
            .effect_sequence()
            .iter()
            .any(|effect| matches!(effect, SimEffect::Search(_)))
    }));
    let (selection, remainder) = source
        .oracle_text
        .split_once(" Put the rest on the bottom of your library")
        .expect("the original control retains its mandatory remainder instruction");
    assert!(matches!(remainder, "." | " in a random order."));
    source.oracle_text = selection.to_string();
    source.name = "Synthetic bare bounded selection".into();
    source
}

/// Unsupported remainder handling cannot disable the supported selector and colored-cost controls.
#[test]
fn kinnan_search_uses_colored_pips_and_only_the_top_five() {
    let kinnan = bare_bounded_search_control(row(
        "Mana trigger",
        "{G}{U}",
        "Legendary Creature — Human Druid",
        "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.\n{5}{G}{U}: Look at the top five cards of your library. You may put a non-Human creature card from among them onto the battlefield. Put the rest on the bottom of your library in a random order.",
    ));
    let creatures = [
        row("Elf", "{1}{G}", "Creature — Elf", ""),
        row("Human one", "{1}{W}", "Creature — Human Soldier", ""),
        row("Bear", "{2}{G}", "Creature — Bear", ""),
        row("Human two", "{2}{W}", "Creature — Human Knight", ""),
        row("Human three", "{3}{W}", "Creature — Human Soldier", ""),
    ];
    let rows = std::iter::once(kinnan.clone())
        .chain(creatures.iter().cloned())
        .collect::<Vec<_>>();
    let deck = deck(&rows);
    let ability = deck.cards[0]
        .unlocked_abilities(0)
        .find(|ability| matches!(ability.effect, SimEffect::Search(_)))
        .expect("the synthetic bare selection lowers as a filtered search")
        .clone();
    let SimEffect::Search(spec) = ability.effect else {
        unreachable!();
    };
    assert_eq!(spec.top_count, Some(5));
    assert!(spec.non_human);
    assert_eq!(
        spec.destination,
        super::model::SearchDestination::Battlefield
    );

    let mut st = state(vec![], vec![1, 2, 3, 4, 5]);
    let mut pool = ManaPool {
        fixed: [0, 1, 0, 0, 1],
        colorless: 5,
        ..ManaPool::default()
    };
    st.battlefield.push(super::game::new_perm_with(
        10,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    assert!(pick_best_activation(&deck, &st, &pool, 1).is_some());
    spend_leftover(&deck, &mut st, &mut pool, 1);
    assert_eq!(
        st.battlefield
            .iter()
            .map(|perm| perm.card.deck_idx().unwrap().index())
            .collect::<Vec<_>>(),
        [0, 3]
    );
    assert_eq!(st.seen, 5);
    assert_eq!(
        st.library,
        [1, 2, 4, 5]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );

    let mut colorless_only = state(vec![], vec![]);
    colorless_only.battlefield.push(super::game::new_perm_with(
        10,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    let pool = ManaPool {
        colorless: 7,
        ..ManaPool::default()
    };
    assert!(pick_best_activation(&deck, &colorless_only, &pool, 1).is_none());
}

/// A bare bounded selection never searches beyond its printed window.
#[test]
fn kinnan_search_does_not_enter_a_creature_missing_from_the_top_five() {
    let kinnan = bare_bounded_search_control(row(
        "Mana trigger",
        "{G}{U}",
        "Legendary Creature — Human Druid",
        "{5}{G}{U}: Look at the top five cards of your library. You may put a non-Human creature card from among them onto the battlefield. Put the rest on the bottom of your library in a random order.",
    ));
    let humans = [
        row("Human one", "{1}{W}", "Creature — Human Soldier", ""),
        row("Human two", "{2}{W}", "Creature — Human Knight", ""),
        row("Human three", "{3}{W}", "Creature — Human Soldier", ""),
        row("Human four", "{4}{W}", "Creature — Human Scout", ""),
        row("Human five", "{5}{W}", "Creature — Human Wizard", ""),
    ];
    let deck = deck(&std::iter::once(kinnan).chain(humans).collect::<Vec<_>>());
    let mut st = state(vec![], vec![1, 2, 3, 4, 5]);
    st.battlefield.push(super::game::new_perm_with(
        10,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    let mut pool = ManaPool {
        fixed: [0, 1, 0, 0, 1],
        colorless: 5,
        ..ManaPool::default()
    };

    spend_leftover(&deck, &mut st, &mut pool, 1);

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(
        st.library,
        [1, 2, 3, 4, 5]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.seen, 5);
}

#[test]
fn extra_turn_runs_a_full_turn_in_its_own_turn_slot() {
    let rows = [
        row(
            "Time Warp",
            "{0}",
            "Sorcery",
            "Take an extra turn after this one.",
        ),
        row("Hasty Creature", "{0}", "Creature — Human Warrior", "Haste"),
        row("Spell One", "{0}", "Instant", ""),
        row("Spell Two", "{0}", "Sorcery", ""),
        row("Island", "", "Basic Land — Island", "{T}: Add {U}."),
        row("Island", "", "Basic Land — Island", "{T}: Add {U}."),
    ];
    let cards = deck(&rows);
    let mut st = state(vec![0, 1, 2, 4, 5], vec![3]);
    st.seen = st.hand.len() as u32 + st.library.len() as u32;
    st.battlefield.push(new_perm_with(
        6,
        &cards,
        crate::deck::simulator::model::CardIdx(1),
        0,
        false,
    ));
    let mut census = TurnCensus::new(2, cards.cards.len());
    let commander = CommanderProfile::new(&cards);
    let mut engines = Vec::new();

    run_turn(
        &cards,
        &mut st,
        &mut census,
        &commander,
        &mut engines,
        1,
        false,
    );

    assert_eq!(st.extra_turns_queued, 1);
    assert_eq!(census.extra_turns[0], 0);
    st.extra_turns_queued = 0;
    run_turn(
        &cards,
        &mut st,
        &mut census,
        &commander,
        &mut engines,
        2,
        true,
    );

    assert_eq!(census.extra_turns[1], 1);
    assert_eq!(census.land_drops[1], 1);
    assert!(census.attackers_turn[1] > 0);
    assert!(
        st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(3)),
        "the extra turn casts its new spell"
    );

    let without_extra = [
        row("No Extra", "{0}", "Sorcery", ""),
        rows[1].clone(),
        rows[2].clone(),
        rows[3].clone(),
        rows[4].clone(),
        rows[5].clone(),
    ];
    let control = deck(&without_extra);
    let mut control_state = state(vec![0, 1, 2, 4, 5], vec![3]);
    control_state.seen = control_state.hand.len() as u32 + control_state.library.len() as u32;
    control_state.battlefield.push(new_perm_with(
        6,
        &control,
        crate::deck::simulator::model::CardIdx(1),
        0,
        false,
    ));
    let mut control_census = TurnCensus::new(1, control.cards.len());
    let control_commander = CommanderProfile::new(&control);
    let mut control_engines = Vec::new();
    run_turn(
        &control,
        &mut control_state,
        &mut control_census,
        &control_commander,
        &mut control_engines,
        1,
        false,
    );
    assert_eq!(control_state.extra_turns_queued, 0);
    assert_eq!(control_census.land_drops, [1]);
    assert_eq!(control_census.attackers_turn.len(), 1);
    assert!(control_census.attackers_turn[0] > 0);
    assert_eq!(
        control_state.library,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert!(
        !control_state
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(3))
    );
}

#[test]
fn saga_chapter_one_fires_on_entry_and_unsupported_chapter_stays_inert() {
    let rows = [
        row(
            "Three Chapter Saga",
            "{2}{U}",
            "Enchantment — Saga",
            "I — Create a 1/1 Soldier creature token.\nII — Draw a card.\nIII — Return target artifact from your graveyard to the battlefield.",
        ),
        row("First Draw", "{1}", "Sorcery", ""),
        row("Second Draw", "{1}", "Sorcery", ""),
    ];
    let cards = deck(&rows);
    assert_eq!(cards.cards[0].chapter_count(), 3);
    assert!(matches!(
        cards.cards[0].saga.chapters[0].as_slice(),
        [SimEffect::Tokens(1)]
    ));
    let mut st = state(vec![], vec![1, 2]);
    st.seen = 2;
    st.battlefield.push(new_perm_with(
        0,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));

    fire_on_enter(&cards, &mut st, 0, 1, false);

    assert_eq!(st.battlefield[0].saga_step, 1);
    assert_eq!(
        st.battlefield.len(),
        2,
        "chapter I creates its token on entry"
    );
    run_sagas(&cards, &mut st, 2);
    assert_eq!(st.battlefield[0].saga_step, 2);
    assert_eq!(
        st.hand,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "chapter II draws the top card"
    );
    run_sagas(&cards, &mut st, 3);
    assert_eq!(
        st.hand,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "unsupported chapter III does not invent a draw"
    );
    assert_eq!(
        st.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "the saga leaves after chapter III"
    );
    assert_eq!(st.battlefield.len(), 1, "only the token remains");
}
