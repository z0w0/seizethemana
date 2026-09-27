use super::cast_phase::{cast_phase, play_land};
use super::game::{GameState, Pool};
use super::model::{Cost, Format, SimDeck};
use super::oracle_parse::{parse_oracle_cost, parse_sim_card};
use crate::db::CardRow;
use std::collections::HashMap;

fn row(name: &str, cost: &str, type_line: &str, text: &str) -> CardRow {
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

fn deck(rows: &[CardRow]) -> SimDeck {
    SimDeck {
        cards: rows.iter().map(parse_sim_card).collect(),
        commanders: Vec::new(),
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    }
}

fn state(hand: Vec<usize>, library: Vec<usize>) -> GameState {
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

fn cast_once(deck: &SimDeck, st: &mut GameState, pool: &mut Pool) {
    let mut spent = [0.0; 1];
    let mut engines = Vec::new();
    let mut pip_blocks = Vec::new();
    let mut blocked_colors = [false; 5];
    cast_phase(
        deck,
        st,
        pool,
        1,
        &mut spent,
        &mut engines,
        &mut pip_blocks,
        &mut blocked_colors,
    );
}

#[test]
fn resolved_spells_reach_the_graveyard_and_permanents_stay_in_play() {
    let cards = deck(&[
        row("Test Draw", "{U}", "Instant", "Draw a card."),
        row("Bear", "{1}", "Creature — Bear", ""),
    ]);
    let mut spell_state = state(vec![0], vec![]);
    let mut pool = Pool {
        flexible: 1,
        ..Pool::default()
    };
    cast_once(&cards, &mut spell_state, &mut pool);
    assert!(spell_state.hand.is_empty());
    assert_eq!(
        spell_state.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(spell_state.battlefield.is_empty());

    let mut permanent_state = state(vec![1], vec![]);
    let mut pool = Pool {
        flexible: 1,
        ..Pool::default()
    };
    cast_once(&cards, &mut permanent_state, &mut pool);
    assert!(permanent_state.graveyard.is_empty());
    assert_eq!(permanent_state.battlefield.len(), 1);
    assert!(permanent_state.battlefield[0].sick);
}

#[test]
fn missing_additional_cost_resources_do_not_spend_mana_or_cards() {
    let rows = [
        row(
            "Neoform",
            "{G}{U}",
            "Sorcery",
            "As an additional cost to cast this spell, sacrifice a creature. Search your library for a creature card.",
        ),
        row(
            "Cathartic Reunion",
            "{1}{R}",
            "Sorcery",
            "As an additional cost to cast this spell, discard two cards. Draw three cards.",
        ),
    ];
    let cards = deck(&rows);
    let mut no_creature = state(vec![0], vec![]);
    let mut pool = Pool {
        flexible: 2,
        ..Pool::default()
    };
    cast_once(&cards, &mut no_creature, &mut pool);
    assert_eq!(
        no_creature.hand,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(pool.flexible, 2);

    let mut no_discard_fodder = state(vec![1], vec![]);
    let mut pool = Pool {
        flexible: 2,
        ..Pool::default()
    };
    cast_once(&cards, &mut no_discard_fodder, &mut pool);
    assert_eq!(
        no_discard_fodder.hand,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(pool.flexible, 2);
    assert_eq!(cards.cards[1].riders.additional_cost_discards, 2);
}

#[test]
fn wheel_sees_current_hand_and_resolves_the_cast_copy_once() {
    let cards = deck(&[
        row(
            "Wheel",
            "{0}",
            "Sorcery",
            "Each player discards their hand, then draws seven cards.",
        ),
        row("Filler", "{7}", "Sorcery", ""),
        row("Filler", "{7}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0, 1, 2], vec![]);
    let mut pool = Pool::default();
    cast_once(&cards, &mut st, &mut pool);
    assert_eq!(
        st.graveyard,
        [1, 2, 0]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(st.hand.is_empty());
    assert!(st.battlefield.is_empty());
}

#[test]
fn life_cost_must_leave_the_player_alive_and_is_paid_on_cast() {
    let cards = deck(&[row(
        "Life Cost Spell",
        "{B}",
        "Sorcery",
        "As an additional cost to cast this spell, pay 5 life. Draw a card.",
    )]);
    let mut state_at_five = state(vec![0], vec![]);
    state_at_five.life = 5;
    let mut pool = Pool {
        flexible: 1,
        ..Pool::default()
    };
    cast_once(&cards, &mut state_at_five, &mut pool);
    assert_eq!(
        state_at_five.hand,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(state_at_five.life, 5);
    assert_eq!(pool.flexible, 1);

    let mut state_at_six = state(vec![0], vec![]);
    state_at_six.life = 6;
    let mut pool = Pool {
        flexible: 1,
        ..Pool::default()
    };
    cast_once(&cards, &mut state_at_six, &mut pool);
    assert_eq!(state_at_six.life, 1);
    assert_eq!(state_at_six.life_paid, 5);
    assert_eq!(
        state_at_six.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
}

#[test]
fn fetch_sacrifices_itself_and_searches_only_a_matching_land_type() {
    let cards = deck(&[
        row(
            "Misty Rainforest",
            "",
            "Land",
            "{T}, Pay 1 life, Sacrifice this land: Search your library for a Forest or Island card, put it onto the battlefield, then shuffle.",
        ),
        row(
            "Breeding Pool",
            "",
            "Land — Forest Island",
            "This land enters tapped.\n{T}: Add {G} or {U}.",
        ),
        row("Mountain", "", "Basic Land — Mountain", "{T}: Add {R}."),
        row(
            "Evolving Wilds",
            "",
            "Land",
            "{T}, Sacrifice Evolving Wilds: Search your library for a basic land card, put it onto the battlefield tapped, then shuffle.",
        ),
        row("Island", "", "Basic Land — Island", "{T}: Add {U}."),
    ]);
    let mut st = state(vec![0], vec![1, 2]);
    assert!(play_land(&cards, &mut st, 1));
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(
        st.battlefield[0].card,
        crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(1))
    );
    assert!(st.battlefield[0].tapped);
    assert_eq!(
        st.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.life, 19);
    assert_eq!(
        st.library,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );

    let mut no_target = state(vec![0], vec![2]);
    assert!(play_land(&cards, &mut no_target, 1));
    assert!(no_target.battlefield.is_empty());
    assert_eq!(
        no_target.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        no_target.library,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );

    let mut no_life_cost = state(vec![3], vec![4]);
    assert!(play_land(&cards, &mut no_life_cost, 1));
    assert_eq!(no_life_cost.life, 20);
    assert_eq!(no_life_cost.life_paid, 0);
    assert_eq!(
        no_life_cost.graveyard,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        no_life_cost.battlefield[0].card,
        crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(4))
    );
    assert!(no_life_cost.battlefield[0].tapped);
}

#[test]
fn shock_land_pays_life_to_enter_untapped() {
    let cards = deck(&[row(
        "Steam Vents",
        "",
        "Land — Island Mountain",
        "({T}: Add {U} or {R}.)\nAs Steam Vents enters, you may pay 2 life. If you don't, it enters tapped.",
    )]);
    let mut paid = state(vec![0], vec![]);
    assert!(play_land(&cards, &mut paid, 1));
    assert_eq!(paid.life, 18);
    assert_eq!(paid.life_paid, 2);
    assert!(!paid.battlefield[0].tapped);

    let mut cannot_pay = state(vec![0], vec![]);
    cannot_pay.life = 2;
    assert!(play_land(&cards, &mut cannot_pay, 1));
    assert_eq!(cannot_pay.life, 2);
    assert!(cannot_pay.battlefield[0].tapped);
}

#[test]
fn no_mana_cost_spell_is_not_a_zero_cost_hand_cast() {
    let cards = deck(&[
        row("Living End", "", "Sorcery", "Suspend 3—{2}{B}{B}."),
        row(
            "Ornithopter",
            "{0}",
            "Artifact Creature — Thopter",
            "Flying.",
        ),
    ]);
    assert!(!cards.cards[0].has_mana_cost);
    assert!(cards.cards[1].has_mana_cost);
    let mut st = state(vec![0, 1], vec![]);
    let mut pool = Pool::default();
    cast_once(&cards, &mut st, &mut pool);
    assert_eq!(
        st.hand,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(
        st.battlefield[0].card,
        crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(1))
    );
    assert!(st.graveyard.is_empty());
}

#[test]
fn resolved_self_exiling_spell_goes_to_exile_and_sacrifice_triggers_fire() {
    let cards = deck(&[
        row(
            "Eldritch Evolution",
            "{1}{G}{G}",
            "Sorcery",
            "As an additional cost to cast this spell, sacrifice a creature. Search your library for a creature card with mana value X or less, where X is 2 plus the sacrificed creature's mana value. Put that card onto the battlefield, then shuffle. Exile Eldritch Evolution.",
        ),
        row("Bear", "{1}", "Creature — Bear", ""),
        row(
            "Death Dealer",
            "{2}{B}",
            "Creature — Human",
            "Whenever another creature you control dies, create a 1/1 Soldier creature token.",
        ),
    ]);
    let mut st = state(vec![0], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(1),
        0,
        false,
    ));
    st.battlefield.push(super::game::new_perm_with(
        2,
        &cards,
        crate::deck::simulator::model::CardIdx(2),
        0,
        false,
    ));
    let mut pool = Pool {
        flexible: 3,
        ..Pool::default()
    };
    cast_once(&cards, &mut st, &mut pool);
    assert_eq!(
        st.exile,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        st.graveyard,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.battlefield.len(), 2);
    assert_eq!(
        st.battlefield[1].card,
        crate::deck::simulator::game::CardRef::Token
    );
    assert_eq!(
        cards.cards[0].cost,
        Cost {
            generic: 1,
            pips: [0, 0, 0, 0, 2],
            ..Cost::default()
        }
    );
    assert_eq!(st.life_paid, 0);
}

#[test]
fn flashback_grants_only_the_graveyard_instances_present_at_resolution() {
    let cards = deck(&[
        row(
            "Flashback grant",
            "{3}{R}",
            "Sorcery",
            "Until end of turn, instant and sorcery cards in your graveyard gain flashback. The flashback cost is equal to its mana cost.",
        ),
        row("Ritual", "{R}", "Instant", "Add {R}{R}."),
        row("Later spell", "{R}", "Instant", "Draw a card."),
        row("Drawn", "{0}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].riders.grants_flashback);
    let mut st = state(vec![0], vec![3]);
    st.graveyard.push(crate::deck::simulator::model::CardIdx(1));
    let mut pool = Pool {
        fixed: [0, 0, 0, 1, 0],
        flexible: 6,
        ..Pool::default()
    };
    cast_once(&cards, &mut st, &mut pool);

    assert_eq!(st.replay_casts, 1);
    assert_eq!(st.milestones_by_turn[&1].graveyard_casts, 1);
    assert_eq!(
        st.exile,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(pool.fixed[3], 2);
    assert!(st.flashback_permissions.is_empty());

    st.graveyard.push(crate::deck::simulator::model::CardIdx(2));
    cast_once(&cards, &mut st, &mut Pool::default());
    assert_eq!(st.replay_casts, 1);
    assert!(
        st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(2))
    );
}

#[test]
fn escape_pays_mana_and_exiles_three_other_graveyard_instances() {
    let cards = deck(&[
        row(
            "Escape grant",
            "{1}{R}",
            "Enchantment",
            "Nonland cards in your graveyard have escape. The escape cost is equal to the card's mana cost plus exile three other cards from your graveyard.",
        ),
        row("First spell", "{R}", "Instant", "Draw a card."),
        row("Second spell", "{R}", "Instant", "Draw a card."),
        row("Fuel one", "{9}", "Sorcery", ""),
        row("Fuel two", "{9}", "Sorcery", ""),
        row("Fuel three", "{9}", "Sorcery", ""),
        row("Fuel four", "{9}", "Sorcery", ""),
        row("Fuel five", "{9}", "Sorcery", ""),
        row("Fuel six", "{9}", "Sorcery", ""),
        row("Draw one", "{0}", "Sorcery", ""),
        row("Draw two", "{0}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].riders.grants_escape);
    let mut st = state(vec![0], vec![]);
    st.graveyard.extend(
        [1, 3, 4, 5, 2, 6, 7, 8]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i)),
    );
    let mut pool = Pool {
        fixed: [0, 0, 0, 3, 0],
        flexible: 1,
        ..Pool::default()
    };
    cast_once(&cards, &mut st, &mut pool);

    assert_eq!(pool.fixed[3], 0);
    assert_eq!(st.replay_casts, 2);
    assert_eq!(st.milestones_by_turn[&1].graveyard_casts, 2);
    assert!(
        st.exile
            .contains(&crate::deck::simulator::model::CardIdx(1))
    );
    assert!(
        st.exile
            .contains(&crate::deck::simulator::model::CardIdx(2))
    );
    assert_eq!(st.exile.len(), 8);
    assert!(st.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(0))));

    let mut no_fuel = state(Vec::new(), vec![]);
    no_fuel.battlefield.push(super::game::new_perm_with(
        10,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    no_fuel.graveyard.extend(
        [1, 3, 4]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i)),
    );
    cast_once(&cards, &mut no_fuel, &mut Pool::default());
    assert!(
        no_fuel
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(1))
    );
    assert!(no_fuel.exile.is_empty());
    assert_eq!(no_fuel.replay_casts, 0);
}

#[test]
fn sacrificed_escape_artifact_can_be_escaped_again_with_remaining_fuel() {
    let cards = deck(&[
        row(
            "Escape grant",
            "{1}{R}",
            "Enchantment",
            "Nonland cards in your graveyard have escape. The escape cost is equal to the card's mana cost plus exile three other cards from your graveyard.",
        ),
        row(
            "Mana artifact",
            "{0}",
            "Artifact",
            "Sacrifice this permanent: Add one mana of any color.",
        ),
        row("Fuel one", "{9}", "Sorcery", ""),
        row("Fuel two", "{9}", "Sorcery", ""),
        row("Fuel three", "{9}", "Sorcery", ""),
        row("Fuel four", "{9}", "Sorcery", ""),
        row("Fuel five", "{9}", "Sorcery", ""),
        row("Fuel six", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), Vec::new());
    assert!(cards.cards[1].sacrifices_for_mana);
    assert!(cards.cards[1].tap.is_some());
    st.next_uid = 1;
    st.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    st.graveyard.extend(
        [1, 2, 3, 4, 5, 6, 7]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i)),
    );
    let mut pool = Pool::default();

    cast_once(&cards, &mut st, &mut pool);
    assert!(st.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(1))));
    assert_eq!(
        st.battlefield
            .iter()
            .find(|permanent| permanent.card
                == crate::deck::simulator::game::CardRef::Deck(
                    crate::deck::simulator::model::CardIdx(1)
                ))
            .map(|permanent| permanent.entered_turn),
        Some(1)
    );
    let permanent = st
        .battlefield
        .iter()
        .find(|permanent| {
            permanent.card
                == crate::deck::simulator::game::CardRef::Deck(
                    crate::deck::simulator::model::CardIdx(1),
                )
        })
        .expect("the escaped artifact stays in play");
    let parsed_artifact = super::game::card_of(&cards, permanent);
    assert!(!parsed_artifact.is_creature);
    assert!(parsed_artifact.sacrifices_for_mana);
    assert!(parsed_artifact.gate_types.is_empty());
    assert!(
        parsed_artifact
            .tap
            .as_ref()
            .is_some_and(|yield_| yield_.scaling.is_none())
    );
    assert!(!permanent.tapped);
    pool = super::game_run::build_pool(&cards, &mut st, 2);
    assert!(!st.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(1))));
    assert!(
        st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(1))
    );
    assert_eq!(pool.total(), 1);

    cast_once(&cards, &mut st, &mut pool);

    assert_eq!(st.replay_casts, 2);
    pool = super::game_run::build_pool(&cards, &mut st, 3);
    assert_eq!(st.exile.len(), 6);
    assert!(!st.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(1))));
    assert_eq!(
        st.graveyard,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(pool.total(), 1);
}

#[test]
fn ad_nauseam_reveals_in_order_and_can_cause_a_lethal_reveal() {
    let cards = deck(&[
        row(
            "Reveal spell",
            "{3}{B}{B}",
            "Sorcery",
            "Reveal the top card of your library and put that card into your hand. You lose life equal to its mana value. Repeat this process any number of times.",
        ),
        row("Zero", "{0}", "Sorcery", ""),
        row("One", "{1}", "Sorcery", ""),
        row("Seven", "{7}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].riders.reveal_rule.is_some());
    let mut st = state(vec![0], vec![1, 2, 3]);
    st.life = 5;
    let mut pool = Pool {
        fixed: [0, 0, 2, 0, 0],
        flexible: 3,
        ..Pool::default()
    };

    cast_once(&cards, &mut st, &mut pool);

    assert_eq!(
        st.hand,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.seen, 1);
    assert_eq!(st.life, -2);
    assert_eq!(
        st.library,
        [1, 2]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.life_funded_draws, 1);
    assert_eq!(st.milestones_by_turn[&1].life_funded_draws, 1);
}

#[test]
fn griselbrand_repeats_life_paid_draws_only_while_life_can_pay() {
    let mut rows = vec![row(
        "Draw engine",
        "{4}{B}{B}{B}{B}",
        "Creature — Demon",
        "Pay 7 life: Draw seven cards.",
    )];
    rows.extend((0..14).map(|_| row("Filler", "{0}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut st = state(Vec::new(), (1..15).collect());
    st.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));

    super::game_effects::spend_leftover(&cards, &mut st, &mut Pool::default(), 1);

    assert_eq!(st.hand.len(), 14);
    assert_eq!(st.life, 6);
    assert_eq!(st.life_paid, 14);
    assert_eq!(st.seen, 14);
    assert_eq!(st.life_funded_draws, 14);
    assert_eq!(st.milestones_by_turn[&1].life_funded_draws, 14);

    let mut rows = vec![
        row(
            "Draw engine",
            "{4}{B}{B}{B}{B}",
            "Creature — Demon",
            "Pay 7 life: Draw seven cards.",
        ),
        row("Life spell", "{0}", "Sorcery", "You gain 7 life."),
    ];
    rows.extend((0..14).map(|_| row("Filler", "{0}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut st = state(vec![1], (2..16).collect());
    st.life = 13;
    st.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    cast_once(&cards, &mut st, &mut Pool::default());
    super::game_effects::spend_leftover(&cards, &mut st, &mut Pool::default(), 1);

    assert_eq!(st.life_gained, 7);
    assert_eq!(st.life, 6);
    assert_eq!(st.hand.len(), 14);
    assert_eq!(st.life_paid, 14);
    assert_eq!(st.life_funded_draws, 14);
}

#[test]
fn sacrifice_draw_resolves_undying_and_cancels_the_targets_counter() {
    let cards = deck(&[
        row(
            "Sacrifice engine",
            "{2}{B}{B}",
            "Creature — Human Cleric",
            "Pay 1 life, Sacrifice another creature: Draw a card, then put a -1/-1 counter on up to one target creature.",
        ),
        row("Fodder", "{G}", "Creature — Wolf", "Undying"),
        row("Target", "{G}", "Creature — Wolf", "Undying"),
        row("Drawn", "{0}", "Sorcery", ""),
        row("Plain fodder", "{G}", "Creature — Wolf", ""),
    ]);
    let mut st = state(Vec::new(), vec![3]);
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
        0,
        false,
    ));
    let mut target = super::game::new_perm_with(
        3,
        &cards,
        crate::deck::simulator::model::CardIdx(2),
        0,
        false,
    );
    target.counters = 1;
    st.battlefield.push(target);
    let ability = cards.cards[0]
        .abilities()
        .next()
        .expect("the text parses as an activation")
        .clone();
    assert_eq!(ability.life_cost, 1);
    assert_eq!(ability.sacrifice_bodies, 1);
    assert_eq!(ability.cost.total(), 0);
    assert!(matches!(
        ability.effect,
        super::model::Effect::DrawAndMinusCounter
    ));
    let activation =
        super::game_effects::pick_best_activation_public(&cards, &st, &Pool::default())
            .expect("the ability has a legal sacrifice and target");

    super::game_effects::resolve_activation_public(
        &cards,
        &mut st,
        &mut Pool::default(),
        1,
        1,
        &activation,
    );

    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 1);
    assert_eq!(
        st.hand,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(st.life_funded_draws, 1);
    assert!(st.battlefield.iter().any(|permanent| {
        permanent.card
            == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(
                1,
            ))
            && permanent.counters == 1
    }));
    assert!(st.battlefield.iter().any(|permanent| {
        permanent.card
            == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(
                2,
            ))
            && permanent.counters == 0
    }));

    let mut no_target = state(Vec::new(), vec![3]);
    no_target.battlefield.push(super::game::new_perm_with(
        10,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    no_target.battlefield.push(super::game::new_perm_with(
        11,
        &cards,
        crate::deck::simulator::model::CardIdx(4),
        0,
        false,
    ));
    let activation =
        super::game_effects::pick_best_activation_public(&cards, &no_target, &Pool::default())
            .expect("the optional target can be absent");
    super::game_effects::resolve_activation_public(
        &cards,
        &mut no_target,
        &mut Pool::default(),
        1,
        1,
        &activation,
    );
    assert_eq!(no_target.life_paid, 1);
    assert_eq!(no_target.life_funded_draws, 1);
    assert_eq!(
        no_target.hand,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(no_target.battlefield.iter().all(|permanent| permanent.card
        != crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(4))));
}
