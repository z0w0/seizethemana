use super::cast_phase::cast_phase;
use super::game::{GameState, Pool};
use super::model::{Format, SimDeck};
use super::oracle_parse::{parse_oracle_cost, parse_sim_card};
use crate::db::CardRow;
use std::collections::HashMap;

fn row(name: &str, cost: &str, type_line: &str, colors: &str, text: &str) -> CardRow {
    CardRow {
        name: name.into(),
        oracle_id: String::new(),
        mana_cost: cost.into(),
        cmc: parse_oracle_cost(cost).total() as f64,
        type_line: type_line.into(),
        colors: colors.into(),
        color_identity: colors.into(),
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

fn cast(deck: &SimDeck, state: &mut GameState, pool: &mut Pool) {
    cast_phase(
        deck,
        state,
        pool,
        1,
        &mut [0.0],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    );
}

#[test]
fn filtered_search_selects_a_matching_card_instead_of_the_library_top() {
    let cards = deck(&[
        row(
            "Tutor",
            "{1}",
            "Artifact",
            "[]",
            "{T}: Search your library for a green creature card with mana value 3 or less, put it into your hand, then shuffle.",
        ),
        row("Red creature", "{2}{R}", "Creature — Beast", "[\"R\"]", ""),
        row("Green creature", "{2}{G}", "Creature — Elf", "[\"G\"]", ""),
        row("Forest", "", "Basic Land — Forest", "[]", ""),
    ]);
    let mut state = state(Vec::new(), vec![1, 2]);
    let search = cards.cards[0]
        .abilities()
        .next()
        .expect("tutor ability parses")
        .effect
        .clone();
    super::game_effects::apply_effect_at(
        &cards,
        &search,
        &mut state,
        1,
        false,
        Some(crate::deck::simulator::model::CardIdx(0)),
    );

    assert_eq!(
        state.hand,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        state.library,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
}

#[test]
fn optional_exact_search_puts_a_basic_land_onto_the_battlefield_tapped() {
    let cards = deck(&[
        row(
            "Land tutor",
            "{1}",
            "Artifact",
            "[]",
            "{T}: Search your library for a basic land card, put it onto the battlefield tapped, then shuffle.",
        ),
        row(
            "Creature tutor",
            "{1}",
            "Artifact",
            "[]",
            "{T}: You may search your library for a creature card with mana value exactly 2, put it into your hand, then shuffle.",
        ),
        row("Forest", "", "Basic Land — Forest", "[]", ""),
        row("Plains", "", "Basic Land — Plains", "[]", ""),
        row(
            "Two-mana creature",
            "{1}{G}",
            "Creature — Elf",
            "[\"G\"]",
            "",
        ),
        row(
            "Three-mana creature",
            "{2}{G}",
            "Creature — Beast",
            "[\"G\"]",
            "",
        ),
        row(
            "Artifact search",
            "{1}",
            "Artifact",
            "[]",
            "{T}: Search your library for a colorless artifact or enchantment card with mana value 4 or more, exile that card, then shuffle.",
        ),
    ]);
    let ability = cards.cards[1]
        .abilities()
        .next()
        .expect("optional search ability parses");
    let super::model::Effect::Search(spec) = ability.effect else {
        panic!("search ability has an explicit filter");
    };
    assert_eq!(spec.card_type, Some(super::model::SearchCardType::Creature));
    assert_eq!(spec.mana_value, Some(2));
    assert!(spec.optional);

    let mut state = state(Vec::new(), vec![2, 3]);
    let land_search = cards.cards[0]
        .abilities()
        .next()
        .expect("land search ability parses")
        .effect
        .clone();
    super::game_effects::apply_effect_at(
        &cards,
        &land_search,
        &mut state,
        1,
        false,
        Some(crate::deck::simulator::model::CardIdx(0)),
    );
    assert_eq!(
        state.library,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(state.battlefield.len(), 1);
    assert_eq!(
        state.battlefield[0].card,
        crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(3))
    );
    assert!(state.battlefield[0].tapped);

    let ability = cards.cards[6]
        .abilities()
        .next()
        .expect("artifact search ability parses");
    let super::model::Effect::Search(spec) = ability.effect else {
        panic!("artifact search uses explicit constraints");
    };
    assert_eq!(
        spec.card_type,
        Some(super::model::SearchCardType::ArtifactOrEnchantment)
    );
    assert!(spec.colorless);
    assert_eq!(spec.min_mana_value, Some(4));
    assert_eq!(spec.destination, super::model::SearchDestination::Exile);
}

#[test]
fn rider_alternate_cost_and_sacrifice_search_put_a_countered_target_into_play() {
    let cards = deck(&[
        row(
            "Rider",
            "{4}{G}{G}{G}",
            "Creature — Elf",
            "[\"G\"]",
            "You may exile two green cards from your hand rather than pay this spell's mana cost.",
        ),
        row(
            "Search spell",
            "{G}{U}",
            "Sorcery",
            "[\"G\",\"U\"]",
            "As an additional cost to cast this spell, sacrifice a creature. Search your library for a creature card with mana value equal to 1 plus the sacrificed creature's mana value, put it onto the battlefield, then shuffle.",
        ),
        row("Green pitch one", "{3}{G}", "Sorcery", "[\"G\"]", ""),
        row("Green pitch two", "{3}{G}", "Sorcery", "[\"G\"]", ""),
        row(
            "Eight-mana target",
            "{4}{B}{B}{B}{B}",
            "Creature — Demon",
            "[\"B\"]",
            "When this creature enters, draw a card.",
        ),
        row("Drawn card", "{1}", "Sorcery", "[]", ""),
    ]);
    assert!(cards.cards[0].riders.alternative_cast_cost.is_some());
    assert_eq!(
        cards.cards[0]
            .riders
            .alternative_cast_cost
            .as_ref()
            .unwrap()
            .count,
        2
    );
    assert_eq!(cards.cards[0].mana_value, 7);
    assert!(cards.cards[1].riders.search_after_sacrifice);
    let mut state = state(vec![0, 1, 2, 3], vec![5, 4]);
    let mut pool = Pool {
        fixed: [0, 1, 0, 0, 1],
        ..Pool::default()
    };

    cast(&cards, &mut state, &mut pool);

    assert_eq!(
        state.exile,
        [2, 3]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "green card colors {:?}",
        cards
            .cards
            .iter()
            .map(|card| (card.name.as_str(), card.colors, card.mana_value))
            .collect::<Vec<_>>()
    );
    assert!(
        state
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0))
    );
    assert!(
        state
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(1))
    );
    assert!(
        state
            .hand
            .contains(&crate::deck::simulator::model::CardIdx(5))
    );
    let target = state
        .battlefield
        .iter()
        .find(|permanent| {
            permanent.card
                == crate::deck::simulator::game::CardRef::Deck(
                    crate::deck::simulator::model::CardIdx(4),
                )
        })
        .expect("the eligible target enters");
    assert_eq!(target.counters, 1);
}

#[test]
fn alternate_cast_needs_two_other_green_cards() {
    let cards = deck(&[
        row(
            "Rider",
            "{4}{G}{G}{G}",
            "Creature — Elf",
            "[\"G\"]",
            "You may exile two green cards from your hand rather than pay this spell's mana cost.",
        ),
        row("Green pitch", "{G}", "Sorcery", "[\"G\"]", ""),
    ]);
    let mut state = state(vec![0, 1], Vec::new());
    cast(&cards, &mut state, &mut Pool::default());
    assert_eq!(
        state.hand,
        [0, 1]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(state.battlefield.is_empty());
    assert!(state.exile.is_empty());
}

#[test]
fn sacrifice_search_casts_even_when_no_creature_has_the_required_mana_value() {
    let cards = deck(&[
        row(
            "Rider",
            "{4}{G}{G}{G}",
            "Creature — Elf",
            "[\"G\"]",
            "You may exile two green cards from your hand rather than pay this spell's mana cost.",
        ),
        row(
            "Search spell",
            "{G}{U}",
            "Sorcery",
            "[\"G\",\"U\"]",
            "As an additional cost to cast this spell, sacrifice a creature. Search your library for a creature card with mana value equal to 1 plus the sacrificed creature's mana value, put it onto the battlefield, then shuffle.",
        ),
        row("Green pitch one", "{3}{G}", "Sorcery", "[\"G\"]", ""),
        row("Green pitch two", "{3}{G}", "Sorcery", "[\"G\"]", ""),
        row(
            "Wrong-value target",
            "{3}{B}{B}{B}",
            "Creature — Demon",
            "[\"B\"]",
            "",
        ),
    ]);
    let mut state = state(vec![0, 1, 2, 3], vec![4]);
    cast(
        &cards,
        &mut state,
        &mut Pool {
            fixed: [0, 1, 0, 0, 1],
            ..Pool::default()
        },
    );

    assert!(
        state
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0))
    );
    assert!(
        state
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(1))
    );
    assert!(!state.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(4))));
    assert_eq!(
        state.library,
        [4].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
}

#[test]
fn cascade_reveals_in_order_and_resolves_living_end_for_the_player() {
    let cards = deck(&[
        row("Cascade spell", "{2}{U}", "Sorcery", "[\"U\"]", "Cascade."),
        row(
            "Living End",
            "",
            "Sorcery",
            "[]",
            "Suspend 3—{2}{B}{B}\nEach player exiles all creature cards from their graveyard, then sacrifices all creatures they control, then puts all cards they exiled this way onto the battlefield.",
        ),
        row("Island", "", "Basic Land — Island", "[]", ""),
        row("Higher-value card", "{3}{R}", "Sorcery", "[\"R\"]", ""),
        row(
            "Cycler one",
            "{10}{G}",
            "Creature — Beast",
            "[\"G\"]",
            "Cycling {1} ({1}, Discard this card: Draw a card.)",
        ),
        row(
            "Cycler two",
            "{10}{G}",
            "Creature — Beast",
            "[\"G\"]",
            "Cycling {1} ({1}, Discard this card: Draw a card.)",
        ),
        row(
            "Cycler three",
            "{10}{G}",
            "Creature — Beast",
            "[\"G\"]",
            "Cycling {1} ({1}, Discard this card: Draw a card.)",
        ),
        row("Drawn spell one", "{9}", "Sorcery", "[]", ""),
        row("Drawn spell two", "{9}", "Sorcery", "[]", ""),
        row("Drawn spell three", "{9}", "Sorcery", "[]", ""),
        row(
            "Board creature",
            "{1}{W}",
            "Creature — Soldier",
            "[\"W\"]",
            "",
        ),
    ]);
    assert!(cards.cards[1].riders.graveyard_creature_exchange);
    assert!(!cards.cards[1].has_mana_cost);
    assert_eq!(cards.cards[0].mana_value, 3);
    assert_eq!(cards.cards[1].mana_value, 0);
    assert_eq!(cards.cards[2].role, super::model::Role::Land);
    assert_eq!(cards.cards[3].mana_value, 4);
    assert_eq!(cards.cards[0].riders.mills_on_enter, 0);
    let mut state = state(vec![0, 4, 5, 6], vec![1, 2, 3, 7, 8, 9]);
    let mut pool = Pool {
        flexible: 6,
        ..Pool::default()
    };
    state.battlefield.push(super::game::Permanent {
        uid: 10,
        card: crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(
            10,
        )),
        tapped: false,
        sick: false,
        counters: 0,
        animated: false,
        crewed: false,
        entered_turn: 0,
        saga_step: 0,
        fired: false,
        blink_pending: false,
        loyalty: 0,
        equipped: false,
        equip_host: None,
    });
    cast(&cards, &mut state, &mut pool);
    assert!(
        state
            .exile
            .contains(&crate::deck::simulator::model::CardIdx(4))
    );
    assert!(
        state
            .exile
            .contains(&crate::deck::simulator::model::CardIdx(5))
    );
    assert!(
        state
            .exile
            .contains(&crate::deck::simulator::model::CardIdx(6))
    );
    assert!(state.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(4))));
    assert!(state.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(5))));
    assert!(state.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(6))));
    assert!(!state.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(
            10
        ))));
    assert_eq!(
        state.library,
        [3, 2]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "reveal order preserved at the bottom (first-revealed deepest): seen {} hand {:?} graveyard {:?}",
        state.seen,
        state.hand,
        state.graveyard
    );
}

#[test]
fn cascade_free_cast_resolves_a_permanent_etb_and_no_cost_spell_stays_uncastable() {
    let cards = deck(&[
        row("Cascade spell", "{2}{U}", "Sorcery", "[\"U\"]", "Cascade."),
        row(
            "Free creature",
            "{1}{G}",
            "Creature — Elf",
            "[\"G\"]",
            "When this creature enters, draw a card.",
        ),
        row("Drawn card", "{1}", "Sorcery", "[]", ""),
        row("No-cost spell", "", "Sorcery", "[]", ""),
    ]);
    let mut game_state = state(vec![0], vec![2, 1]);
    cast(
        &cards,
        &mut game_state,
        &mut Pool {
            flexible: 3,
            ..Pool::default()
        },
    );
    assert!(game_state.battlefield.iter().any(|permanent| permanent.card
        == crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(1))));
    assert!(
        game_state
            .hand
            .contains(&crate::deck::simulator::model::CardIdx(2))
    );
    assert_eq!(
        game_state.milestones_by_turn[&1].free_cast_permanents_entered,
        1
    );

    let mut hand_state = state(vec![3], Vec::new());
    cast(&cards, &mut hand_state, &mut Pool::default());
    assert_eq!(
        hand_state.hand,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
}

#[test]
fn cycling_discards_and_draws_while_landcycling_finds_the_named_land_type() {
    let cards = deck(&[
        row(
            "Life cycler",
            "{3}{B}{B}",
            "Creature — Wraith",
            "[\"B\"]",
            "Cycling—Pay 2 life. (Pay 2 life, Discard this card: Draw a card.)",
        ),
        row(
            "Forest cycler",
            "{5}{G}",
            "Creature — Treefolk",
            "[\"G\"]",
            "Forestcycling {1} ({1}, Discard this card: Search your library for a Forest card, reveal it, put it into your hand, then shuffle.)",
        ),
        row("Draw", "{1}", "Sorcery", "[]", ""),
        row("Forest", "", "Basic Land — Forest", "[]", ""),
        row("Island", "", "Basic Land — Island", "[]", ""),
    ]);
    assert_eq!(cards.cards[0].riders.cycling_life, 2);
    assert_eq!(cards.cards[1].riders.landcycling_type, Some('G'));
    let mut life_state = state(vec![0], vec![2]);
    cast(&cards, &mut life_state, &mut Pool::default());
    assert_eq!(
        life_state.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        life_state.hand,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(life_state.life_paid, 2);
    assert_eq!(life_state.life, 18);

    let mut state = state(vec![1], vec![4, 3]);
    cast(
        &cards,
        &mut state,
        &mut Pool {
            colorless: 1,
            ..Pool::default()
        },
    );
    assert_eq!(
        state.graveyard,
        [1].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        state.hand,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        state.library,
        [4].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
}

#[test]
fn dredge_replaces_a_draw_only_when_the_full_mill_amount_is_available() {
    let cards = deck(&[
        row(
            "Dredger",
            "{2}{G}",
            "Creature — Troll",
            "[\"G\"]",
            "Dredge 5 (If you would draw a card, instead you may mill five cards. If you do, return this card from your graveyard to your hand.)",
        ),
        row("Milled one", "{1}", "Sorcery", "[]", ""),
        row("Milled two", "{1}", "Sorcery", "[]", ""),
        row("Milled three", "{1}", "Sorcery", "[]", ""),
        row("Milled four", "{1}", "Sorcery", "[]", ""),
        row("Milled five", "{1}", "Sorcery", "[]", ""),
    ]);
    assert_eq!(cards.cards[0].dredge, Some(5));

    let mut full_library = state(Vec::new(), vec![1, 2, 3, 4, 5]);
    full_library
        .graveyard
        .push(crate::deck::simulator::model::CardIdx(0));
    assert!(super::game_effects::draw_one(&cards, &mut full_library, 3));
    assert_eq!(
        full_library.hand,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(full_library.library.is_empty());
    assert_eq!(
        full_library.graveyard,
        [5, 4, 3, 2, 1]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(full_library.seen, 5);
    assert_eq!(full_library.awareness_cards, 5);
    assert_eq!(full_library.milestones_by_turn[&3].dredge_uses, 1);

    let mut short_library = state(Vec::new(), vec![1, 2, 3, 4]);
    short_library
        .graveyard
        .push(crate::deck::simulator::model::CardIdx(0));
    assert!(super::game_effects::draw_one(&cards, &mut short_library, 3));
    assert_eq!(
        short_library.hand,
        [4].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        short_library.library,
        [1, 2, 3]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        short_library.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(short_library.seen, 1);
    assert!(!short_library.milestones_by_turn.contains_key(&3));
}

#[test]
fn later_draws_can_use_dredgers_milled_by_an_earlier_draw() {
    let cards = deck(&[
        row(
            "Five dredger",
            "{2}{G}",
            "Creature — Troll",
            "[\"G\"]",
            "Dredge 5 (If you would draw a card, instead you may mill five cards. If you do, return this card from your graveyard to your hand.)",
        ),
        row(
            "Three dredger",
            "{1}{G}",
            "Creature — Troll",
            "[\"G\"]",
            "Dredge 3 (If you would draw a card, instead you may mill three cards. If you do, return this card from your graveyard to your hand.)",
        ),
        row("Card one", "{1}", "Sorcery", "[]", ""),
        row("Card two", "{1}", "Sorcery", "[]", ""),
        row("Card three", "{1}", "Sorcery", "[]", ""),
        row("Card four", "{1}", "Sorcery", "[]", ""),
        row("Card five", "{1}", "Sorcery", "[]", ""),
        row("Card six", "{1}", "Sorcery", "[]", ""),
        row("Card seven", "{1}", "Sorcery", "[]", ""),
        row("Card eight", "{1}", "Sorcery", "[]", ""),
        row("Card nine", "{1}", "Sorcery", "[]", ""),
    ]);
    let mut game_state = state(Vec::new(), vec![2, 3, 4, 5, 6, 1, 7, 8, 9]);
    game_state
        .graveyard
        .push(crate::deck::simulator::model::CardIdx(0));

    super::game_effects::apply_effect_at(
        &cards,
        &super::model::Effect::Draw(3),
        &mut game_state,
        2,
        false,
        None,
    );

    assert_eq!(
        game_state.hand,
        [0, 1, 2]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(game_state.library.is_empty());
    assert_eq!(game_state.seen, 9);
    assert_eq!(game_state.awareness_cards, 9);
}

#[test]
fn discard_cost_draws_resolve_individually_and_recheck_dredge() {
    let spell = row(
        "Discard draw spell",
        "{1}{R}",
        "Sorcery",
        "[\"R\"]",
        "As an additional cost to cast this spell, discard two cards. Draw three cards.",
    );
    let oracle = super::oracle_parse::parse_oracle_card(&spell);
    assert!(
        oracle.abilities.iter().any(|ability| matches!(
            ability,
            super::oracle_ast::OracleAbility::Spell(spell)
                if spell.effects.iter().any(|effect| matches!(effect, super::oracle_ast::OracleEffect::Draw(3)))
        )),
        "draw rider AST: {oracle:#?}"
    );
    let cards = deck(&[
        spell,
        row(
            "Five dredger",
            "{2}{G}",
            "Creature — Troll",
            "[\"G\"]",
            "Dredge 5 (If you would draw a card, instead you may mill five cards. If you do, return this card from your graveyard to your hand.)",
        ),
        row(
            "Three dredger",
            "{1}{G}",
            "Creature — Troll",
            "[\"G\"]",
            "Dredge 3 (If you would draw a card, instead you may mill three cards. If you do, return this card from your graveyard to your hand.)",
        ),
        row("Discard one", "", "Basic Land — Plains", "[]", ""),
        row("Discard two", "", "Basic Land — Island", "[]", ""),
        row("Drawn one", "{1}", "Sorcery", "[]", ""),
        row("Milled one", "{1}", "Sorcery", "[]", ""),
        row("Milled two", "{1}", "Sorcery", "[]", ""),
        row("Milled three", "{1}", "Sorcery", "[]", ""),
        row("Milled four", "{1}", "Sorcery", "[]", ""),
        row("Milled five", "{1}", "Sorcery", "[]", ""),
        row("Milled six", "{1}", "Sorcery", "[]", ""),
        row("Milled seven", "{1}", "Sorcery", "[]", ""),
    ]);
    assert_eq!(cards.cards[0].riders.additional_cost_discards, 2);
    assert_eq!(
        cards.cards[0].riders.draws_on_cast, 3,
        "parsed card: {:#?}",
        cards.cards[0]
    );
    let mut game_state = state(vec![0, 3, 4], vec![5, 6, 7, 8, 9, 2, 10, 11, 12]);
    game_state
        .graveyard
        .push(crate::deck::simulator::model::CardIdx(1));
    let mut pool = Pool {
        fixed: [0, 0, 0, 1, 0],
        flexible: 1,
        ..Pool::default()
    };

    cast(&cards, &mut game_state, &mut pool);

    assert_eq!(
        game_state.hand,
        [1, 2, 5]
            .iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(game_state.seen, 9);
    assert_eq!(game_state.awareness_cards, 9);
    assert!(
        game_state
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(3))
    );
    assert!(
        game_state
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(4))
    );
    assert!(
        game_state
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0))
    );
}

#[test]
fn library_mill_triggers_return_creatures_and_exile_drain_cards() {
    let cards = deck(&[
        row(
            "Library returner",
            "{1}{U}",
            "Creature — Illusion",
            "[\"U\"]",
            "When Library Returner is put into your graveyard from your library, you may put it onto the battlefield.",
        ),
        row(
            "Library returner",
            "{1}{U}",
            "Creature — Illusion",
            "[\"U\"]",
            "When Library Returner is put into your graveyard from your library, you may put it onto the battlefield.",
        ),
        row(
            "Life drain card",
            "{2}{B}",
            "Sorcery",
            "[\"B\"]",
            "When Life Drain Card is put into your graveyard from your library, each opponent loses 3 life and you gain 3 life.",
        ),
        row("Blank", "{1}", "Sorcery", "[]", ""),
    ]);
    assert!(cards.cards[0].library_graveyard_trigger.is_some());
    assert!(cards.cards[1].library_graveyard_trigger.is_some());
    let mut game_state = state(Vec::new(), vec![3, 2, 1, 0]);

    super::game_effects::apply_effect_at(
        &cards,
        &super::model::Effect::Mill(3),
        &mut game_state,
        4,
        false,
        None,
    );

    assert_eq!(game_state.milled_self, 3);
    assert_eq!(game_state.seen, 3);
    assert_eq!(game_state.awareness_cards, 3);
    assert_eq!(
        game_state
            .battlefield
            .iter()
            .map(|permanent| permanent.card.deck_idx().unwrap().index())
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(
        game_state.exile,
        [2].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert!(game_state.graveyard.is_empty());
    assert_eq!(
        game_state.library,
        [3].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(game_state.drained, 3);
    assert_eq!(game_state.life_gained, 3);
    assert_eq!(game_state.life, 23);
}

#[test]
fn discarding_a_library_trigger_card_does_not_fire_its_mill_ability() {
    let cards = deck(&[
        row(
            "Life drain card",
            "{2}{B}",
            "Sorcery",
            "[\"B\"]",
            "When Life Drain Card is put into your graveyard from your library, each opponent loses 3 life and you gain 3 life.",
        ),
        row("Draw", "{1}", "Sorcery", "[]", ""),
    ]);
    let mut game_state = state(vec![0], vec![1]);

    super::game_effects::apply_effect_at(
        &cards,
        &super::model::Effect::Wheel,
        &mut game_state,
        1,
        false,
        None,
    );

    assert_eq!(
        game_state.graveyard,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(game_state.drained, 0);
    assert_eq!(game_state.life_gained, 0);
}
