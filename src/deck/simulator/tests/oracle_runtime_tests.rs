//!! Keep the standard co-located test import for shared simulator helpers.
use super::game::{Activation, Pool, fire_on_enter, new_perm_with};
use super::game_combat::combat_phase;
use super::game_commander::CommanderProfile;
use super::game_effects::resolve_activation_public;
use super::game_run::{TurnCensus, build_pool, run_turn};
use super::model::Effect;
use super::turn_loop_tests::{cast, deck, row, state};
#[allow(unused_imports)]
use super::*;

/// Move exact surveilled copies and fire only library-origin triggers.
#[test]
fn surveil_moves_the_exact_top_cards_and_resolves_library_triggers() {
    let cards = deck(&[
        row("Surveil spell", "{U}", "Instant", "Surveil 2."),
        row(
            "Narcomoeba copy",
            "{1}{U}",
            "Creature — Illusion",
            "When this card is put into your graveyard from your library, you may put it onto the battlefield.",
        ),
        row("Library card", "{9}", "Sorcery", ""),
        row(
            "Narcomoeba copy",
            "{1}{U}",
            "Creature — Illusion",
            "When this card is put into your graveyard from your library, you may put it onto the battlefield.",
        ),
    ]);
    let mut st = state(vec![0], vec![1, 2, 3]);
    let mut pool = Pool {
        flexible: 1,
        ..Pool::default()
    };

    let spent = cast(&cards, &mut st, &mut pool);

    assert_eq!(spent, [1.0]);
    assert!(st.hand.is_empty());
    assert_eq!(
        st.library,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "the unexamined duplicate stays in the library"
    );
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(
        st.battlefield[0].card,
        crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(3)),
        "the top physical copy triggers"
    );
    assert_eq!(
        st.graveyard,
        [2, 0]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "the other examined card and spell resolve"
    );
    assert_eq!(st.seen, 2);
    assert_eq!(st.awareness_cards, 2);
    assert_eq!(st.milled_self, 2);
}

/// Execute multiple spell effects parsed from separate Oracle clauses.
#[test]
fn compound_spell_executes_parsed_draw_and_life_effects() {
    let cards = deck(&[
        row(
            "Mixed effect spell",
            "{U}",
            "Instant",
            "Draw two cards. Gain three life.",
        ),
        row("First card", "{9}", "Sorcery", ""),
        row("Second card", "{9}", "Sorcery", ""),
        row("Last card", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![1, 2, 3]);
    let mut pool = Pool {
        flexible: 1,
        ..Pool::default()
    };

    cast(&cards, &mut st, &mut pool);

    assert_eq!(
        st.hand,
        [3, 2]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        st.library,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        st.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.life, 23);
    assert_eq!(st.life_gained, 3);
    assert_eq!(st.seen, 2);
}

/// Keep unsupported spell text inert after a legal cast.
#[test]
fn unsupported_spell_text_stays_inert_after_a_legal_cast() {
    let cards = deck(&[
        row(
            "Unsupported spell",
            "{U}",
            "Instant",
            "Venture into the dungeon.",
        ),
        row("Unchanged library card", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![1]);
    let mut pool = Pool {
        flexible: 1,
        ..Pool::default()
    };

    cast(&cards, &mut st, &mut pool);

    assert!(st.hand.is_empty());
    assert_eq!(
        st.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        st.library,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.life, 20);
    assert_eq!(st.seen, 0);
    assert_eq!(st.awareness_cards, 0);
}

/// Fire landfall once for land entry and not for a nonland entry.
#[test]
fn landfall_draws_only_when_a_land_enters() {
    let cards = deck(&[
        row(
            "Landfall engine",
            "{1}{G}",
            "Creature — Elemental",
            "Landfall — Whenever a land you control enters, draw a card.",
        ),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("First draw", "{9}", "Sorcery", ""),
        row("Second draw", "{9}", "Sorcery", ""),
        row("Ordinary creature", "{1}{G}", "Creature — Bear", ""),
    ]);
    let mut st = state(vec![1], vec![2, 3]);
    st.battlefield.push(new_perm_with(
        10,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));

    assert!(super::cast_phase::play_land(&cards, &mut st, 1));
    assert_eq!(
        st.hand,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        st.library,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.seen, 1);

    st.battlefield.push(new_perm_with(
        11,
        &cards,
        crate::deck::simulator::model::CardIdx(4),
        1,
        false,
    ));
    let nonland_position = st.battlefield.len() - 1;
    fire_on_enter(&cards, &mut st, nonland_position, 1, false);
    assert_eq!(
        st.hand,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "a nonland entry does not repeat landfall"
    );
    assert_eq!(
        st.library,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );

    let mut no_land = state(vec![], vec![2, 3]);
    no_land.battlefield.push(new_perm_with(
        12,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    assert!(!super::cast_phase::play_land(&cards, &mut no_land, 1));
    assert!(no_land.hand.is_empty());
    assert_eq!(
        no_land.library,
        [2, 3]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(no_land.seen, 0);
}

/// Resolve attack and combat-damage triggers only when a creature attacks.
#[test]
fn attack_and_combat_damage_triggers_fire_only_for_an_attacker() {
    let cards = deck(&[
        row(
            "Connecting attacker",
            "{R}",
            "Creature — Warrior",
            "Haste\nWhenever this creature attacks, draw a card.\nWhenever this creature deals combat damage to a player, draw a card.",
        ),
        row("First card", "{9}", "Sorcery", ""),
        row("Second card", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![], vec![1, 2]);
    st.battlefield.push(new_perm_with(
        10,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));

    let outcome = combat_phase(&cards, &mut st, 1, &[1]);

    assert_eq!(outcome.attackers, 1);
    assert_eq!(
        st.hand,
        [2, 1]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert!(st.library.is_empty());
    assert_eq!(st.seen, 2);

    let mut no_attack = state(vec![], vec![1, 2]);
    no_attack.battlefield.push(new_perm_with(
        11,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    no_attack.battlefield[0].sick = true;
    let outcome = combat_phase(&cards, &mut no_attack, 1, &[1]);
    assert_eq!(outcome.attackers, 0);
    assert!(no_attack.hand.is_empty());
    assert_eq!(
        no_attack.library,
        [1, 2]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(no_attack.seen, 0);
}

/// Fire first-main and end-step triggers at their correct turn boundaries.
#[test]
fn first_main_and_end_step_triggers_fire_at_their_turn_boundaries() {
    let cards = deck(&[
        row(
            "First main engine",
            "{2}{U}",
            "Enchantment",
            "At the beginning of your first main phase, draw a card.",
        ),
        row(
            "End step engine",
            "{2}{U}",
            "Enchantment",
            "At the beginning of your end step, draw a card.",
        ),
        row("First land", "", "Basic Land — Island", "{T}: Add {U}."),
        row("Second land", "", "Basic Land — Forest", "{T}: Add {G}."),
    ]);
    let mut st = state(vec![], vec![2, 3]);
    st.battlefield.push(new_perm_with(
        10,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    st.battlefield.push(new_perm_with(
        11,
        &cards,
        crate::deck::simulator::model::CardIdx(1),
        0,
        false,
    ));
    let mut census = TurnCensus::new(1, cards.cards.len());
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

    assert_eq!(census.land_drops, [1]);
    assert_eq!(
        st.battlefield
            .iter()
            .map(|permanent| permanent.card.deck_idx().unwrap().index())
            .collect::<Vec<_>>(),
        [0, 1, 3]
    );
    assert_eq!(
        st.hand,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "end-step draw waits for the next land phase"
    );
    assert!(st.library.is_empty());
}

/// Reject Kinnan bonuses from land taps and nonmana activations.
#[test]
fn kinnan_bonus_ignores_land_taps_and_nonmana_taps() {
    let kinnan = row(
        "Mana trigger",
        "{G}{U}",
        "Legendary Creature — Human Druid",
        "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
    );
    let forest = row("Forest", "", "Basic Land — Forest", "{T}: Add {G}.");
    let land_deck = deck(&[kinnan.clone(), forest]);
    let mut land_state = state(vec![], vec![]);
    land_state.battlefield.push(new_perm_with(
        10,
        &land_deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    land_state.battlefield.push(new_perm_with(
        11,
        &land_deck,
        crate::deck::simulator::model::CardIdx(1),
        1,
        false,
    ));

    let land_pool = build_pool(&land_deck, &mut land_state, 1);
    assert_eq!(land_pool.fixed[4], 1);
    assert_eq!(land_pool.colorless, 0, "a land tap gets no Kinnan bonus");

    let cards = deck(&[
        kinnan,
        row(
            "Looter",
            "{1}{U}",
            "Creature — Merfolk",
            "{T}: Draw a card.",
        ),
        row("Drawn card", "{9}", "Sorcery", ""),
    ]);
    let draw = cards.cards[1]
        .abilities()
        .find(|ability| matches!(ability.effect, Effect::Draw(1)))
        .expect("looter activation")
        .clone();
    let mut draw_state = state(vec![], vec![2]);
    draw_state.battlefield.push(new_perm_with(
        20,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    draw_state.battlefield.push(new_perm_with(
        21,
        &cards,
        crate::deck::simulator::model::CardIdx(1),
        1,
        false,
    ));
    draw_state.battlefield[1].sick = false;
    let mut pool = Pool::default();
    let activation = Activation {
        pos: 1,
        uid: 21,
        ability: draw,
        cost: 0,
        draws: 1,
        search: None,
        mana_yield: None,
        counters: 0,
        drain: 0,
        sacrifice_uid: None,
        target_uid: None,
    };
    resolve_activation_public(&cards, &mut draw_state, &mut pool, 1, 1, &activation);
    assert_eq!(
        draw_state.hand,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>()
    );
    assert_eq!(pool.total(), 0, "a nonmana tap gets no Kinnan bonus");
}

/// Spend verified Basalt loop mana on Kinnan's colored search activation.
#[test]
fn proven_basalt_loop_funds_kinnans_colored_search_activation() {
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
        "{T}: Add {C}{C}{C}.\n{3}: Untap this artifact.",
    );
    let rows = [
        kinnan,
        basalt,
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Island", "", "Basic Land — Island", "{T}: Add {U}."),
        row("Bear", "{2}{G}", "Creature — Bear", ""),
    ];
    let deck = deck(&rows);
    let untap = deck.cards[1]
        .abilities()
        .find(|ability| matches!(ability.effect, Effect::UntapSelf))
        .expect("Basalt untap ability")
        .clone();
    let mut st = state(vec![], vec![4]);
    for index in 0..4 {
        st.battlefield.push(new_perm_with(
            index as u32,
            &deck,
            crate::deck::simulator::model::CardIdx(index as u32),
            1,
            false,
        ));
    }
    let mut pool = build_pool(&deck, &mut st, 1);
    assert_eq!(
        pool.colorless, 4,
        "Basalt starts with three mana plus Kinnan's bonus"
    );
    let starting_colorless = pool.colorless;

    for _ in 0..5 {
        let activation = Activation {
            pos: 1,
            uid: 1,
            ability: untap.clone(),
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
    assert_eq!(
        pool.colorless - starting_colorless,
        5,
        "pool after loop: {pool:?}"
    );
    assert_eq!(pool.fixed[1], 1, "the Island supplies blue");
    assert_eq!(pool.fixed[4], 1, "the Forest supplies green");

    let search_ability = deck.cards[0]
        .abilities()
        .find(|ability| matches!(ability.effect, Effect::Search(_)))
        .expect("Kinnan's search ability")
        .clone();
    let spec = match &search_ability.effect {
        Effect::Search(spec) => *spec,
        _ => panic!("expected search effect"),
    };
    let search = Activation {
        pos: 0,
        uid: 0,
        ability: search_ability,
        cost: 7,
        draws: 0,
        search: Some(spec),
        mana_yield: None,
        counters: 0,
        drain: 0,
        sacrifice_uid: None,
        target_uid: None,
    };
    resolve_activation_public(&deck, &mut st, &mut pool, 1, 1, &search);

    assert!(st.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(4))));
    assert!(st.library.is_empty());
    assert_eq!(pool.colorless, 4);
    assert_eq!(pool.fixed, [0; 5]);
}

#[test]
fn debug_selfmill2() {
    let self_miller = super::turn_loop_tests::row(
        "Self Miller",
        "{2}",
        "Creature — Zombie",
        "At the beginning of your upkeep, mill three cards.",
    );
    let opp = super::turn_loop_tests::row(
        "Opp Miller",
        "{1}{B}{B}",
        "Legendary Creature — Horror",
        "At the beginning of your upkeep, each opponent mills two cards.",
    );
    let land = super::turn_loop_tests::row("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)");
    let cards = vec![
        super::oracle_parse::parse_sim_card(&opp),
        super::oracle_parse::parse_sim_card(&self_miller),
        super::oracle_parse::parse_sim_card(&self_miller),
        super::oracle_parse::parse_sim_card(&self_miller),
    ];
    let mut lib = vec![super::oracle_parse::parse_sim_card(&land); 8];
    lib.push(super::oracle_parse::parse_sim_card(&self_miller));
    lib.push(super::oracle_parse::parse_sim_card(&self_miller));
    lib.push(super::oracle_parse::parse_sim_card(&self_miller));
    let deck = super::model::SimDeck {
        cards: lib,
        commanders: cards,
        format: super::model::Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(5);
    let log = super::game::run_game(&deck, &mut rng, 4);
    eprintln!("self: {:?} opp: {:?}", log.self_milled, log.opp_milled);
    eprintln!("bf: {:?}", log.card_first_battlefield);
    eprintln!("seen: {:?}", log.card_first_seen);
    eprintln!("engines: {:?}", log.engines_online);
}

#[test]
fn debug_selfmill3() {
    let self_miller = super::turn_loop_tests::row(
        "Self Miller",
        "{2}",
        "Creature — Zombie",
        "At the beginning of your upkeep, mill three cards.",
    );
    let opp = super::turn_loop_tests::row(
        "Opp Miller",
        "{1}{B}{B}",
        "Legendary Creature — Horror",
        "At the beginning of your upkeep, each opponent mills two cards.",
    );
    let land = super::turn_loop_tests::row("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)");
    let cards = vec![
        super::oracle_parse::parse_sim_card(&opp),
        super::oracle_parse::parse_sim_card(&self_miller),
        super::oracle_parse::parse_sim_card(&self_miller),
        super::oracle_parse::parse_sim_card(&self_miller),
    ];
    let mut lib = vec![super::oracle_parse::parse_sim_card(&land); 4];
    lib.push(super::oracle_parse::parse_sim_card(&self_miller));
    lib.push(super::oracle_parse::parse_sim_card(&self_miller));
    lib.push(super::oracle_parse::parse_sim_card(&self_miller));
    for (i, c) in lib.iter().enumerate() {
        if c.name != "Swamp" {
            eprintln!(
                "lib[{i}]: upkeep_abilities={:?}",
                c.abilities()
                    .filter(|a| a.trigger == super::model::AbilityTiming::OnUpkeep)
                    .collect::<Vec<_>>()
            );
        }
    }
    let deck = super::model::SimDeck {
        cards: lib,
        commanders: cards,
        format: super::model::Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(5);
    let log = super::game::run_game(&deck, &mut rng, 4);
    eprintln!(
        "self: {:?} opp: {:?} bf: {:?} seen: {:?} eng: {:?} grave: {:?}",
        log.self_milled,
        log.opp_milled,
        log.card_first_battlefield,
        log.card_first_seen,
        log.engines_online,
        log.card_first_graveyard
    );
}
