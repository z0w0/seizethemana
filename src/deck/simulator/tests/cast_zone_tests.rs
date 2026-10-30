//! Tests for casting from the hand and other zones.
use super::cast_pass::{cast_pass, play_land};
use super::game::{GameState, ManaPool};
use super::model::{Cost, Format, SimDeck};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::cost::parse_cost;
use crate::db::CardRow;
use std::collections::HashMap;

/// Build a card row for cast and zone regressions.
pub(super) fn row(name: &str, cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.into(),
        oracle_id: String::new(),
        mana_cost: cost.into(),
        cmc: parse_cost(cost).total() as f64,
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
        penny_rank: None,
        reserved: None,
    }
}

/// Lower test card rows into a Constructed deck.
pub(super) fn deck(rows: &[CardRow]) -> SimDeck {
    SimDeck {
        companion: None,
        cards: rows.iter().map(parse_sim_card).collect(),
        commanders: Vec::new(),
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    }
}

/// Build an empty board with the chosen hand and library order.
pub(super) fn state(hand: Vec<usize>, library: Vec<usize>) -> GameState {
    let to_idx = |v: Vec<usize>| {
        v.into_iter()
            .map(|i| super::model::CardIdx(i as u32))
            .collect()
    };
    GameState {
        completion: None,
        failed_draw: false,
        current_turn: 0,
        current_phase: super::game::GamePhase::Upkeep,
        graveyard_generations: HashMap::new(),
        resolution: Default::default(),
        library_rng: super::rng::game_rng(0, 0, super::rng::RngDomain::Library),
        battlefield: Vec::new(),
        library: to_idx(library),
        hand: to_idx(hand),
        seen: 0,
        graveyard: Vec::new(),
        exile: Vec::new(),
        battlefield_seen: HashMap::new(),
        graveyard_seen: HashMap::new(),
        card_first_seen: HashMap::new(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        opponent_life_lost: 0,
        damage_dealt_this_turn: 0,
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
        player_counters: Default::default(),
        activated_this_turn: Default::default(),
        activated_once: Default::default(),
        triggered_this_turn: Default::default(),
        companion_fetched: false,
        ring_tempts: 0,
        extra_land_drops_this_turn: 0,
        spells_cast_this_turn: 0,
        attacked_this_turn: false,
        plotted: Vec::new(),
        ring_bearer: None,
        commander_casts: Vec::new(),
        opponents: super::opponents::OpponentTable::for_format(Format::Constructed),
    }
}

#[test]
fn card_drawn_and_cast_in_one_turn_is_recorded_as_seen() {
    let cards = deck(&[row("Free spell", "{0}", "Sorcery", "")]);
    let mut game = state(Vec::new(), vec![0]);

    assert!(super::game_effects::draw_one(&cards, &mut game, 1));
    let mut pool = ManaPool::default();
    let mut mana_spent = [0.0; 12];
    let mut repeatable_sources = Vec::new();
    let mut pip_blocks = Vec::new();
    let mut blocked_colors = [false; 5];
    cast_pass(
        &cards,
        &mut game,
        &mut pool,
        1,
        &mut mana_spent,
        &mut repeatable_sources,
        &mut pip_blocks,
        &mut blocked_colors,
    );

    assert!(game.hand.is_empty());
    assert_eq!(game.graveyard, [super::model::CardIdx(0)]);
    assert_eq!(game.card_first_seen.get(&0), Some(&1));
}

#[test]
fn mana_mode_pays_source_sacrifice_and_uses_its_output() {
    let cards = deck(&[row(
        "Sacrificing mana artifact",
        "",
        "Artifact",
        "{T}: Add {G}.\nSacrifice this artifact: Add {C}{C}.",
    )]);
    let source = &cards.cards[0];
    let sacrifice_mode = source
        .unlocked_abilities(0)
        .find(|ability| {
            ability
                .activation
                .as_ref()
                .is_some_and(|activation| activation.sacrifices_source())
        })
        .expect("source-sacrifice mana mode parses");
    assert!(matches!(
        sacrifice_mode.effect_sequence(),
        [super::model::SimEffect::Mana(yield_)] if yield_.colorless == 2
    ));

    let mut game = state(Vec::new(), Vec::new());
    game.next_uid = 1;
    game.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        super::model::CardIdx(0),
        0,
        false,
    ));

    let pool = super::game_run::build_pool(&cards, &mut game, 1);

    assert_eq!(pool.colorless, 2, "the chosen mode adds two colorless mana");
    assert!(pool.fixed.iter().all(|amount| *amount == 0));
    assert!(
        game.battlefield.is_empty(),
        "the mana source was sacrificed"
    );
    assert_eq!(game.graveyard, [super::model::CardIdx(0)]);
}

/// Run one cast pass and return its mana spend.
pub(super) fn cast_once(deck: &SimDeck, st: &mut GameState, pool: &mut ManaPool) -> [f64; 1] {
    let mut spent = [0.0; 1];
    let mut engines = Vec::new();
    let mut pip_blocks = Vec::new();
    let mut blocked_colors = [false; 5];
    cast_pass(
        deck,
        st,
        pool,
        1,
        &mut spent,
        &mut engines,
        &mut pip_blocks,
        &mut blocked_colors,
    );
    spent
}

#[test]
fn delve_pays_generic_cost_with_graveyard_cards_and_resolves() {
    let mut rows = vec![row(
        "Delve Draw",
        "{5}{U}",
        "Sorcery",
        "Delve (Each card you exile from your graveyard while casting this spell pays for {1}.)\nDraw three cards.",
    )];
    rows.extend((0..8).map(|index| row(&format!("Filler {index}"), "{10}", "Sorcery", "")));
    let cards = deck(&rows);
    assert!(cards.cards[0].keyword_abilities.delve);
    assert_eq!(cards.cards[0].spell_data.draws_on_cast, 3);

    let mut game = state(vec![0], (6..9).collect());
    game.graveyard = (1..6)
        .map(|index| super::model::CardIdx(index as u32))
        .collect();
    let mut pool = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        ..ManaPool::default()
    };
    let spent = cast_once(&cards, &mut game, &mut pool);

    assert_eq!(
        game.exile,
        (1..6)
            .rev()
            .map(|index| super::model::CardIdx(index as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        game.hand.len(),
        3,
        "the spell draws after it resolves; library {:?}, exile {:?}, graveyard {:?}",
        game.library,
        game.exile,
        game.graveyard
    );
    assert_eq!(game.seen, 3);
    assert_eq!(spent[0], 1.0, "Delve leaves only the blue pip to pay");
    assert!(game.graveyard.contains(&super::model::CardIdx(0)));

    let mut short_game = state(vec![0], (6..9).collect());
    short_game.graveyard = (1..5)
        .map(|index| super::model::CardIdx(index as u32))
        .collect();
    let mut short_pool = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        ..ManaPool::default()
    };
    cast_once(&cards, &mut short_game, &mut short_pool);
    assert_eq!(short_game.hand, [super::model::CardIdx(0)]);
    assert!(
        short_game.exile.is_empty(),
        "an unpaid delve cast stays in hand"
    );
}

#[test]
fn convoke_uses_matching_creature_colors_and_tokens_for_generic_costs() {
    let mut bear = row("Green Bear", "{G}", "Creature — Bear", "");
    bear.colors = "[\"G\"]".to_string();
    let mut green_rows = vec![
        row(
            "Green Convoke",
            "{G}",
            "Sorcery",
            "Convoke (Your creatures can help cast this spell.)\nDraw a card.",
        ),
        bear,
    ];
    green_rows.extend((0..2).map(|index| row(&format!("Filler {index}"), "{10}", "Sorcery", "")));
    let green_deck = deck(&green_rows);
    let mut green_game = state(vec![0], vec![2, 3]);
    green_game.battlefield.push(super::game::new_perm_with(
        0,
        &green_deck,
        super::model::CardIdx(1),
        0,
        false,
    ));
    let green_spent = cast_once(&green_deck, &mut green_game, &mut ManaPool::default());
    assert!(green_game.graveyard.contains(&super::model::CardIdx(0)));
    assert!(green_game.battlefield[0].tapped);
    assert_eq!(green_spent[0], 0.0, "convoke pays the colored pip");

    let mut white_bear = row("White Bear", "{W}", "Creature — Bear", "");
    white_bear.colors = "[\"W\"]".to_string();
    let mut restricted_deck = deck(&[
        row(
            "Restricted Convoke",
            "{W}{U}",
            "Instant",
            "Convoke (Your creatures can help cast this spell.)\nDraw a card.",
        ),
        white_bear,
        row("Filler E", "{10}", "Sorcery", ""),
        row("Filler F", "{10}", "Sorcery", ""),
    ]);
    assert!(restricted_deck.cards[0].keyword_abilities.convoke);
    assert_eq!(restricted_deck.cards[0].cost.pips, [1, 1, 0, 0, 0]);
    restricted_deck.cards[0].flags.is_interaction = true;
    let mut restricted_game = state(vec![0], vec![2, 3]);
    restricted_game.battlefield.push(super::game::new_perm_with(
        0,
        &restricted_deck,
        super::model::CardIdx(1),
        0,
        false,
    ));
    assert!(super::game::is_creature_permanent(
        &restricted_deck,
        &restricted_game.battlefield[0]
    ));
    assert_eq!(
        super::game::card_of(&restricted_deck, &restricted_game.battlefield[0]).colors,
        [true, false, false, false, false]
    );
    let mut restricted_pool =
        super::turn_loop_tests::restricted_pool(1, super::model::SpendRestriction::InstantSorcery);
    let restricted_spent = cast_once(&restricted_deck, &mut restricted_game, &mut restricted_pool);
    assert!(
        restricted_game
            .graveyard
            .contains(&super::model::CardIdx(0)),
        "restricted convoke did not cast: hand {:?}, pool {:?}, battlefield {:?}, cost {:?}",
        restricted_game.hand,
        restricted_pool,
        restricted_game.battlefield,
        restricted_deck.cards[0].cost
    );
    assert!(restricted_game.battlefield[0].tapped);
    assert_eq!(
        restricted_spent[0], 1.0,
        "the restricted pool pays the blue pip"
    );

    let mut red_bear = row("Red Bear", "{R}", "Creature — Bear", "");
    red_bear.colors = "[\"R\"]".to_string();
    let red_deck = deck(&[
        green_rows[0].clone(),
        red_bear,
        row("Filler C", "{10}", "Sorcery", ""),
        row("Filler D", "{10}", "Sorcery", ""),
    ]);
    let mut red_game = state(vec![0], vec![2, 3]);
    red_game.battlefield.push(super::game::new_perm_with(
        0,
        &red_deck,
        super::model::CardIdx(1),
        0,
        false,
    ));
    cast_once(&red_deck, &mut red_game, &mut ManaPool::default());
    assert_eq!(red_game.hand, [super::model::CardIdx(0)]);
    assert!(!red_game.battlefield[0].tapped);

    let generic_deck = deck(&[
        row(
            "Generic Convoke",
            "{1}",
            "Sorcery",
            "Convoke (Your creatures can help cast this spell.)",
        ),
        row("Filler A", "{10}", "Sorcery", ""),
        row("Filler B", "{10}", "Sorcery", ""),
    ]);
    let mut generic_game = state(vec![0], vec![1, 2]);
    let mut spirit = super::game::new_token_perm(0, 0);
    spirit.card = super::game::CardRef::Token(super::game::TokenKind::Spirit);
    generic_game.battlefield.push(spirit);
    let generic_spent = cast_once(&generic_deck, &mut generic_game, &mut ManaPool::default());
    assert!(generic_game.graveyard.contains(&super::model::CardIdx(0)));
    assert!(generic_game.battlefield[0].tapped);
    assert_eq!(generic_spent[0], 0.0, "the Spirit pays generic mana");

    let mut hybrid_bear = row("White Hybrid Bear", "{W}", "Creature — Bear", "");
    hybrid_bear.colors = "[\"W\"]".to_string();
    let hybrid_deck = deck(&[
        row(
            "Hybrid Convoke",
            "{W/U}",
            "Sorcery",
            "Convoke (Your creatures can help cast this spell.)",
        ),
        hybrid_bear,
        row("Filler G", "{10}", "Sorcery", ""),
        row("Filler H", "{10}", "Sorcery", ""),
    ]);
    let mut hybrid_game = state(vec![0], vec![2, 3]);
    hybrid_game.battlefield.push(super::game::new_perm_with(
        0,
        &hybrid_deck,
        super::model::CardIdx(1),
        0,
        false,
    ));
    cast_once(&hybrid_deck, &mut hybrid_game, &mut ManaPool::default());
    assert!(hybrid_game.hand.is_empty());
    assert_eq!(hybrid_game.graveyard, [super::model::CardIdx(0)]);
    assert!(hybrid_game.battlefield[0].tapped);
}

#[test]
fn resolved_spells_reach_the_graveyard_and_permanents_stay_in_play() {
    let cards = deck(&[
        row("Test Draw", "{U}", "Instant", "Draw a card."),
        row("Bear", "{1}", "Creature — Bear", ""),
    ]);
    let mut spell_state = state(vec![0], vec![]);
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
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
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };
    cast_once(&cards, &mut permanent_state, &mut pool);
    assert!(permanent_state.graveyard.is_empty());
    assert_eq!(permanent_state.battlefield.len(), 1);
    assert!(permanent_state.battlefield[0].summoning_sick);
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
    let mut pool = ManaPool {
        flexible: 2,
        ..ManaPool::default()
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
    let mut pool = ManaPool {
        flexible: 2,
        ..ManaPool::default()
    };
    cast_once(&cards, &mut no_discard_fodder, &mut pool);
    assert_eq!(
        no_discard_fodder.hand,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(pool.flexible, 2);
    assert_eq!(cards.cards[1].spell_data.additional_cost_discards, 2);
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
    let mut pool = ManaPool::default();
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
    // CR 119.4: exactly 5 life may pay a 5-life cost (down to 0), so the
    // spell resolves and the player survives at 0.
    let mut state_at_five = state(vec![0], vec![]);
    state_at_five.life = 5;
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };
    cast_once(&cards, &mut state_at_five, &mut pool);
    assert!(state_at_five.hand.is_empty());
    assert_eq!(state_at_five.life, 0);
    assert_eq!(state_at_five.life_paid, 5);

    // One life short of the cost: the cast stays in hand.
    let mut state_at_four = state(vec![0], vec![]);
    state_at_four.life = 4;
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
    };
    cast_once(&cards, &mut state_at_four, &mut pool);
    assert_eq!(
        state_at_four.hand,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(state_at_four.life, 4);
    assert_eq!(pool.flexible, 1);

    let mut state_at_six = state(vec![0], vec![]);
    state_at_six.life = 6;
    let mut pool = ManaPool {
        flexible: 1,
        ..ManaPool::default()
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
            "Breeding ManaPool",
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
    assert_eq!(
        st.battlefield[0].card,
        super::game::CardRef::Deck(super::model::CardIdx(0))
    );
    assert!(!st.battlefield[0].tapped);
    assert!(st.graveyard.is_empty());
    assert_eq!(st.life, 20);
    super::game_effects::spend_leftover(&cards, &mut st, &mut ManaPool::default(), 1);
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
    assert_eq!(
        no_target.battlefield[0].card,
        super::game::CardRef::Deck(super::model::CardIdx(0))
    );
    super::game_effects::spend_leftover(&cards, &mut no_target, &mut ManaPool::default(), 1);
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
    assert_eq!(
        no_life_cost.battlefield[0].card,
        super::game::CardRef::Deck(super::model::CardIdx(3))
    );
    assert!(no_life_cost.graveyard.is_empty());
    super::game_effects::spend_leftover(&cards, &mut no_life_cost, &mut ManaPool::default(), 1);
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
fn sacrifice_mana_activation_requires_and_consumes_a_creature() {
    let cards = deck(&[
        row(
            "Altar",
            "{3}",
            "Artifact",
            "Sacrifice a creature: Add {C}{C}.",
        ),
        row("Bear", "{1}{G}", "Creature — Bear", ""),
    ]);
    let mut without_body = state(Vec::new(), Vec::new());
    without_body.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    assert!(
        super::game_effects::activation::pick_best_activation(
            &cards,
            &without_body,
            &ManaPool::default(),
            1,
        )
        .is_none()
    );

    let mut with_body = state(Vec::new(), Vec::new());
    with_body.battlefield.push(super::game::new_perm_with(
        1,
        &cards,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    with_body.battlefield.push(super::game::new_perm_with(
        2,
        &cards,
        crate::deck::simulator::model::CardIdx(1),
        0,
        false,
    ));
    let activation = super::game_effects::activation::pick_best_activation(
        &cards,
        &with_body,
        &ManaPool::default(),
        1,
    )
    .expect("the creature pays the activation cost");
    let mut pool = ManaPool::default();
    super::game_effects::activation::resolve_activation(
        &cards,
        &mut with_body,
        &mut pool,
        1,
        &activation,
    );

    assert_eq!(pool.colorless, 2);
    assert_eq!(with_body.battlefield.len(), 1);
    assert_eq!(
        with_body.graveyard,
        [crate::deck::simulator::model::CardIdx(1)]
    );
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

    // CR 119.4: a player may pay life equal to their total, so at exactly
    // 2 life the shock can still enter untapped.
    let mut exact = state(vec![0], vec![]);
    exact.life = 2;
    assert!(play_land(&cards, &mut exact, 1));
    assert_eq!(exact.life, 0);
    assert!(!exact.battlefield[0].tapped);

    let mut cannot_pay = state(vec![0], vec![]);
    cannot_pay.life = 1;
    assert!(play_land(&cards, &mut cannot_pay, 1));
    assert_eq!(cannot_pay.life, 1);
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
    let mut pool = ManaPool::default();
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
    assert!(
        cards.cards[2]
            .striations
            .iter()
            .flat_map(|tier| &tier.abilities)
            .any(|ability| ability.trigger == super::model::SimTrigger::Dies
                && matches!(
                    ability.effect_sequence(),
                    [super::model::SimEffect::Tokens(1)]
                ))
    );
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
    let mut pool = ManaPool {
        flexible: 3,
        ..ManaPool::default()
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
        crate::deck::simulator::game::CardRef::Token(
            crate::deck::simulator::game::TokenKind::Creature,
        )
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
