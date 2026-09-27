use super::cast_phase::cast_phase;
use super::deal::london_mulligan;
use super::game::{Activation, GameState, Pool, fire_on_enter, new_perm_with, run_game};
use super::game_commander::CommanderProfile;
use super::game_effects::{pick_best_activation_public, resolve_activation_public, spend_leftover};
use super::game_run::{
    TurnCensus, build_pool, run_sagas, run_turn, tap_dorks_for_mana, tap_new_rocks,
};
use super::model::CardIdx;
use super::model::{Ability, AbilityTiming, Effect, Format, SimDeck, Tier};
use super::oracle_parse::{parse_oracle_cost, parse_sim_card};
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

/// Build a card row for controlled simulator tests.
pub(super) fn row(name: &str, cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.into(),
        oracle_id: String::new(),
        mana_cost: cost.into(),
        cmc: parse_oracle_cost(cost).total() as f64,
        type_line: type_line.into(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.into(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

/// Parse test rows into a constructed simulator deck.
pub(super) fn deck(rows: &[CardRow]) -> SimDeck {
    SimDeck {
        cards: rows.iter().map(parse_sim_card).collect(),
        commanders: Vec::new(),
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    }
}

/// Create a game state with exact hand and library card indexes.
pub(super) fn state(hand: Vec<usize>, library: Vec<usize>) -> GameState {
    let to_idx = |v: Vec<usize>| {
        v.into_iter()
            .map(|i| super::model::CardIdx(i as u32))
            .collect()
    };
    GameState {
        battlefield: Vec::new(),
        library: to_idx(library),
        hand: to_idx(hand),
        seen: 0,
        graveyard: Vec::new(),
        exile: Vec::new(),
        battlefield_seen: HashMap::new(),
        graveyard_seen: HashMap::new(),
        #[cfg(test)]
        alternate_casts: Vec::new(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        drained: 0,
        life_gained: 0,
        flashback_permissions: std::collections::HashSet::new(),
        replay_casts: 0,
        milestones_by_turn: HashMap::new(),
        life_paid: 0,
        life_funded_draws: 0,
        life: 20,
        is_monarch: false,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
    }
}

/// Run the turn-one cast phase and return its mana spend.
pub(super) fn cast(deck: &SimDeck, st: &mut GameState, pool: &mut Pool) -> [f64; 1] {
    let mut spent = [0.0];
    cast_phase(
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

#[test]
fn cantrip_draws_and_casts_payable_spell_in_same_main_phase() {
    let cards = deck(&[
        row("Cantrip", "{U}", "Instant", "Draw a card."),
        row("Follow-up", "{U}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![1]);
    let mut pool = Pool {
        flexible: 2,
        ..Pool::default()
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
    let mut pool = Pool {
        fixed: [0, 0, 0, 1, 0],
        ..Pool::default()
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
    let mut pool = Pool {
        fixed: [0, 0, 0, 1, 0],
        creature_only: 1,
        ..Pool::default()
    };

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
    let mut pool = Pool {
        creature_only: 1,
        ..Pool::default()
    };
    let _ = cast(&cards, &mut restricted_only, &mut pool);
    assert_eq!(restricted_only.battlefield.len(), 1);
    assert_eq!(
        restricted_only.hand,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(pool.creature_only, 0);
}

#[test]
fn newly_cast_rock_taps_once_for_a_follow_up_cast() {
    let cards = deck(&[
        row("Mana Rock", "{1}", "Artifact", "{T}: Add {C}."),
        row("Follow-up", "{1}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0, 1], vec![]);
    let mut pool = Pool {
        colorless: 1,
        ..Pool::default()
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
    st.battlefield[1].sick = true;
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
    let mut pool = Pool::default();
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
    spend_leftover(&cards, &mut draw_state, &mut Pool::default(), 1);
    assert!(!draw_state.infinite_mana_suspected);

    cards.cards[2].station_tiers.push(Tier {
        abilities: vec![Ability {
            trigger: AbilityTiming::Activated,
            effect: Effect::None,
            ..Ability::default()
        }],
        ..Tier::default()
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
    spend_leftover(&cards, &mut no_progress_state, &mut Pool::default(), 1);
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
    let mut one_shot_pool = Pool::default();
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
    let hand = london_mulligan(&cards, &mut library);
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
    let mut cards = deck(&[
        row("Land", "", "Basic Land — Plains", "{T}: Add {W}."),
        row("Blank", "{9}", "Sorcery", ""),
    ]);
    cards.cards = std::iter::repeat_n(cards.cards[0].clone(), 24)
        .chain(std::iter::repeat_n(cards.cards[1].clone(), 36))
        .collect();
    let mut constructed_rng = ChaCha8Rng::seed_from_u64(81);
    let constructed = run_game(&cards, &mut constructed_rng, 1);
    assert_eq!(constructed.cards_seen, [7]);

    let mut commander_deck = cards;
    commander_deck.format = Format::Commander;
    commander_deck.rules = super::format::rules_for("commander");
    let mut commander_rng = ChaCha8Rng::seed_from_u64(81);
    let commander = run_game(&commander_deck, &mut commander_rng, 1);
    assert_eq!(commander.cards_seen, [8]);
}

#[test]
fn nonland_mana_trigger_adds_one_produced_mana() {
    let rows = [
        row(
            "Mana trigger",
            "{G}{U}",
            "Creature — Human Druid",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        ),
        row("Three mana rock", "{3}", "Artifact", "{T}: Add {C}{C}{C}."),
    ];
    let deck = deck(&rows);
    assert!(deck.cards[0].bonus_mana_on_nonland_tap);
    let mut st = state(vec![], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        0,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    st.battlefield.push(super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(1),
        1,
        false,
    ));

    let pool = build_pool(&deck, &mut st, 1);

    assert_eq!(pool.colorless, 4);
    assert!(st.battlefield[1].tapped);
}

#[test]
fn metalcraft_mana_source_needs_three_artifacts() {
    let rows = [
        row(
            "Conditional rock",
            "{0}",
            "Artifact",
            "Metalcraft — {T}: Add one mana of any color. (You have metalcraft if you control three or more artifacts.)",
        ),
        row("Artifact two", "{0}", "Artifact", ""),
        row("Artifact three", "{0}", "Artifact", ""),
        row("Land", "", "Basic Land — Island", "{T}: Add {U}."),
    ];
    let deck = deck(&rows);
    assert!(deck.cards[0].requires_metalcraft);
    for (artifact_count, include_land) in [(1, true), (2, false), (3, false)] {
        let mut st = state(vec![], vec![]);
        for card in 0..artifact_count {
            st.battlefield.push(super::game::new_perm_with(
                card as u32,
                &deck,
                CardIdx(card as u32),
                1,
                false,
            ));
        }
        if include_land {
            st.battlefield.push(super::game::new_perm_with(
                1,
                &deck,
                crate::deck::simulator::model::CardIdx(3),
                1,
                false,
            ));
        }
        let pool = build_pool(&deck, &mut st, 1);
        assert_eq!(pool.total(), u32::from(include_land || artifact_count == 3));
        assert_eq!(st.battlefield[0].tapped, artifact_count == 3);
    }

    let mut active = state(vec![], vec![]);
    for index in 0..3 {
        active.battlefield.push(new_perm_with(
            index as u32,
            &deck,
            crate::deck::simulator::model::CardIdx(index as u32),
            1,
            false,
        ));
    }
    let pool = build_pool(&deck, &mut active, 1);
    assert_eq!(pool.total(), 1);
    active.battlefield.retain(|permanent| permanent.uid != 2);
    for permanent in &mut active.battlefield {
        permanent.tapped = false;
    }
    let pool = build_pool(&deck, &mut active, 2);
    assert_eq!(pool.total(), 0, "Mox Opal stops after an artifact leaves");
    assert!(!active.battlefield[0].tapped);
}

#[test]
fn opponent_dependent_mana_is_generic_only_from_turn_two() {
    let deck = deck(&[row(
        "Fellwar Stone",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
    )]);
    let mut st = state(vec![], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        0,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));

    let turn_one_pool = build_pool(&deck, &mut st, 1);
    assert_eq!(turn_one_pool.total(), 0);
    assert!(!st.battlefield[0].tapped);

    let turn_two_pool = build_pool(&deck, &mut st, 2);
    assert_eq!(turn_two_pool.colorless, 1);
    assert!(st.battlefield[0].tapped);
    assert!(!super::game_mana::pips_ok(
        &super::model::Cost {
            pips: [0, 1, 0, 0, 0],
            ..super::model::Cost::default()
        },
        &turn_two_pool,
    ));
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
        .abilities()
        .find(|ability| matches!(ability.effect, Effect::UntapSelf))
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
        let starting_pool = pool.colorless;
        for _ in 0..5 {
            let activation = Activation {
                pos: basalt_index,
                uid: basalt_index as u32,
                ability: ability.clone(),
                cost: 3,
                draws: 0,
                search: None,
                mana_yield: None,
                counters: 0,
                drain: 0,
                sacrifice_uid: None,
                target_uid: None,
            };
            resolve_activation_public(&deck, &mut st, &mut pool, 1, 1, &activation);
        }
        assert_eq!(pool.colorless, expected_pool);
        assert!(st.battlefield[basalt_index].tapped);
        assert_eq!(pool.colorless > 3, positive);
        assert_eq!(
            pool.colorless - starting_pool,
            if positive { 5 } else { 0 },
            "five untap cycles change the pool by +5 only with Kinnan"
        );
    }
}

#[test]
fn kinnan_search_uses_colored_pips_and_only_the_top_five() {
    let kinnan = row(
        "Mana trigger",
        "{G}{U}",
        "Legendary Creature — Human Druid",
        "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.\n{5}{G}{U}: Look at the top five cards of your library. You may put a non-Human creature card from among them onto the battlefield. Put the rest on the bottom of your library in a random order.",
    );
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
        .abilities()
        .find(|ability| matches!(ability.effect, Effect::Search(_)))
        .expect("Kinnan's activation parses as a filtered search")
        .clone();
    let Effect::Search(spec) = ability.effect else {
        unreachable!();
    };
    assert_eq!(spec.top_count, Some(5));
    assert!(spec.non_human);
    assert_eq!(
        spec.destination,
        super::model::SearchDestination::Battlefield
    );

    let mut st = state(vec![], vec![1, 2, 3, 4, 5]);
    let mut pool = Pool {
        fixed: [0, 1, 0, 0, 1],
        colorless: 5,
        ..Pool::default()
    };
    st.battlefield.push(super::game::new_perm_with(
        10,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    assert!(pick_best_activation_public(&deck, &st, &pool).is_some());
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
    let pool = Pool {
        colorless: 7,
        ..Pool::default()
    };
    assert!(pick_best_activation_public(&deck, &colorless_only, &pool).is_none());
}

#[test]
fn kinnan_search_does_not_enter_a_creature_missing_from_the_top_five() {
    let kinnan = row(
        "Mana trigger",
        "{G}{U}",
        "Legendary Creature — Human Druid",
        "{5}{G}{U}: Look at the top five cards of your library. You may put a non-Human creature card from among them onto the battlefield. Put the rest on the bottom of your library in a random order.",
    );
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
    let mut pool = Pool {
        fixed: [0, 1, 0, 0, 1],
        colorless: 5,
        ..Pool::default()
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
