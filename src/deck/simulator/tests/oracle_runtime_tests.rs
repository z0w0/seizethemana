//! Keep the standard co-located test import for shared simulator helpers.
use super::game::{Activation, ManaPool, fire_on_enter, fire_triggers, new_perm_with};
use super::game_combat::combat_phase;
use super::game_commander::CommanderProfile;
use super::game_effects::activation::resolve_activation;
use super::game_run::{TurnCensus, build_pool, run_turn};
use super::model::SimEffect;
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
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
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
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
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

/// Resolve all effects from one entry trigger in Oracle order.
#[test]
fn compound_entry_trigger_resolves_all_effects_once() {
    let cards = deck(&[
        row(
            "Mixed entry trigger",
            "{U}",
            "Enchantment",
            "When this permanent enters, draw a card and gain two life.",
        ),
        row("Drawn card", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![1]);
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };

    cast(&cards, &mut st, &mut pool);

    assert_eq!(st.hand, [crate::deck::simulator::model::CardIdx(1)]);
    assert_eq!(st.life, 22);
    assert_eq!(st.life_gained, 2);
}

/// Pay one cost and resolve every activation effect once.
#[test]
fn compound_activation_pays_once_and_resolves_all_effects() {
    let cards = deck(&[
        row(
            "Mixed activation",
            "{1}{U}",
            "Creature — Wizard",
            "{1}, {T}: Draw a card and gain one life.",
        ),
        row("Drawn card", "{9}", "Sorcery", ""),
    ]);
    let ability = cards.cards[0]
        .unlocked_abilities(0)
        .find(|ability| ability.kind.is_activated())
        .expect("parsed activated ability")
        .clone();
    let mut st = state(vec![], vec![1]);
    st.battlefield.push(new_perm_with(
        12,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    st.battlefield[0].summoning_sick = false;
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };
    let activation = Activation {
        pos: 0,
        uid: 12,
        ability,
        ability_cost: super::model::Cost {
            generic: 1,
            ..super::model::Cost::default()
        },
        draws: 1,
        sacrifice_uid: None,
        target_uid: None,
    };

    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(pool.total(), 0);
    assert_eq!(st.hand, [crate::deck::simulator::model::CardIdx(1)]);
    assert_eq!(st.life_gained, 1);
    assert!(st.battlefield[0].tapped);
}

/// Check every activation cost before paying any component.
#[test]
fn compound_activation_requires_and_pays_all_costs() {
    let cards = deck(&[
        row(
            "Combined activation",
            "{1}",
            "Artifact",
            "{1}, Pay 2 life, Remove a charge counter from this artifact, {T}: Draw a card and gain one life.",
        ),
        row("Drawn card", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![], vec![1]);
    st.battlefield.push(new_perm_with(
        12,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    st.battlefield[0].counters.charge = 1;
    let activation =
        super::game_effects::activation::pick_best_activation(&cards, &st, &ManaPool::default(), 1);
    assert!(activation.is_none(), "the mana cost is not payable");
    assert_eq!(st.life, 20, "no partial cost is paid");
    assert_eq!(st.battlefield[0].counters.charge, 1);
    assert!(!st.battlefield[0].tapped);

    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };
    let activation = super::game_effects::activation::pick_best_activation(&cards, &st, &pool, 1)
        .expect("all activation costs are payable");
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(pool.total(), 0);
    assert_eq!(st.life, 19, "pay two life, then gain one life");
    assert_eq!(st.life_paid, 2);
    assert_eq!(st.battlefield[0].counters.charge, 0);
    assert!(st.battlefield[0].tapped);
    assert_eq!(st.hand, [crate::deck::simulator::model::CardIdx(1)]);
}

/// Keep player damage out of the direct life-loss measure.
#[test]
fn direct_damage_and_life_loss_use_separate_runtime_counters() {
    let damage_cards = deck(&[
        row(
            "Bolt",
            "{R}",
            "Instant",
            "Bolt deals 3 damage to target player.",
        ),
        row("Library card", "{9}", "Sorcery", ""),
    ]);
    let mut damage_state = state(vec![0], vec![1]);
    let mut damage_pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };
    cast(&damage_cards, &mut damage_state, &mut damage_pool);
    assert_eq!(damage_state.damage_dealt_this_turn, 3);
    assert_eq!(damage_state.opponent_life_lost, 0);

    let life_loss_cards = deck(&[
        row("Drain", "{B}", "Instant", "Target player loses 3 life."),
        row("Library card", "{9}", "Sorcery", ""),
    ]);
    let mut life_loss_state = state(vec![0], vec![1]);
    let mut life_loss_pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };
    cast(&life_loss_cards, &mut life_loss_state, &mut life_loss_pool);
    assert_eq!(life_loss_state.damage_dealt_this_turn, 0);
    assert_eq!(life_loss_state.opponent_life_lost, 3);
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
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
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

    assert!(super::cast_pass::play_land(&cards, &mut st, 1));
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
    assert!(!super::cast_pass::play_land(&cards, &mut no_land, 1));
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

    let no_haste_cards = deck(&[
        row(
            "New attacker",
            "{R}",
            "Creature — Warrior",
            "Whenever this creature attacks, draw a card.",
        ),
        row("First card", "{9}", "Sorcery", ""),
        row("Second card", "{9}", "Sorcery", ""),
    ]);
    let mut no_attack = state(vec![], vec![1, 2]);
    no_attack.battlefield.push(new_perm_with(
        11,
        &no_haste_cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    no_attack.battlefield[0].summoning_sick = true;
    let outcome = combat_phase(&no_haste_cards, &mut no_attack, 1, &[1]);
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

#[test]
fn intervening_if_condition_gates_the_trigger_resolution() {
    let cards = deck(&[
        row(
            "Raid draw",
            "{1}{R}",
            "Enchantment",
            "At the beginning of your end step, if you attacked this turn, draw a card.",
        ),
        row("Drawn card", "{9}", "Sorcery", ""),
    ]);
    let mut not_attacked = state(vec![], vec![1]);
    not_attacked.battlefield.push(new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    fire_triggers(
        &cards,
        &mut not_attacked,
        super::model::SimTrigger::EndStep,
        1,
    );
    assert!(not_attacked.hand.is_empty());

    let mut attacked = state(vec![], vec![1]);
    attacked.battlefield.push(new_perm_with(
        2,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    ));
    attacked.attacked_this_turn = true;
    fire_triggers(&cards, &mut attacked, super::model::SimTrigger::EndStep, 1);
    assert_eq!(attacked.hand, [crate::deck::simulator::model::CardIdx(1)]);
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
        .unlocked_abilities(0)
        .find(|ability| matches!(ability.effect, SimEffect::Draw(1)))
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
    draw_state.battlefield[1].summoning_sick = false;
    let mut pool = ManaPool::default();
    let activation = Activation {
        pos: 1,
        uid: 21,
        ability: draw,
        ability_cost: super::model::Cost::default(),
        draws: 1,
        sacrifice_uid: None,
        target_uid: None,
    };
    resolve_activation(&cards, &mut draw_state, &mut pool, 1, &activation);
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
        .unlocked_abilities(0)
        .find(|ability| matches!(ability.effect, SimEffect::UntapSelf))
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
            ability_cost: super::model::Cost {
                generic: 3,
                ..Default::default()
            },
            draws: 0,
            sacrifice_uid: None,
            target_uid: None,
        };
        resolve_activation(&deck, &mut st, &mut pool, 1, &activation);
    }
    assert_eq!(
        pool.colorless - starting_colorless,
        5,
        "pool after loop: {pool:?}"
    );
    assert_eq!(pool.fixed[1], 1, "the Island supplies blue");
    assert_eq!(pool.fixed[4], 1, "the Forest supplies green");

    let search_ability = deck.cards[0]
        .unlocked_abilities(0)
        .find(|ability| matches!(ability.effect, SimEffect::Search(_)))
        .expect("Kinnan's search ability")
        .clone();
    let search = Activation {
        pos: 0,
        uid: 0,
        ability: search_ability.clone(),
        ability_cost: search_ability
            .activation
            .as_ref()
            .expect("activation cost bundle")
            .mana_cost(),
        draws: 0,
        sacrifice_uid: None,
        target_uid: None,
    };
    resolve_activation(&deck, &mut st, &mut pool, 1, &search);

    assert!(st.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(4))));
    assert!(st.library.is_empty());
    assert_eq!(pool.colorless, 4);
    assert_eq!(pool.fixed, [0; 5]);
}
