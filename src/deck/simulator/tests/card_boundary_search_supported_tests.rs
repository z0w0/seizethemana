use super::*;
use crate::deck::simulator::model::MultiSearchSpec;
use crate::deck::simulator::{card_ast_tests, oracle_lower, rng};

/// Compare every runtime field with the independently written historical split rule.
fn assert_runtime_plan(actual: &MultiSearchSpec) {
    let expected = split_search_plan();
    assert_eq!(actual.card_type, expected.card_type);
    assert_eq!(actual.land_types, expected.land_types);
    assert_eq!(actual.min_cards, expected.min_cards);
    assert_eq!(actual.max_cards, expected.max_cards);
    assert_eq!(actual.reveal, expected.reveal);
    assert_eq!(actual.shuffle, expected.shuffle);
    assert_eq!(actual.destinations.len(), expected.destinations.len());
    for (actual, expected) in actual.destinations.iter().zip(&expected.destinations) {
        assert_eq!(actual.destination, expected.destination);
        assert_eq!(actual.max_cards, expected.max_cards);
    }
}

/// Require complete syntax, complete lowering, and source-bound selection evidence.
fn assert_split_search_source(source: &CardRow, linked_reward: bool) {
    card_ast_tests::assert_source_compiles(source);
    let oracle = parse_oracle_card(source);
    let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
        panic!("the historical source has one complete spell ability");
    };
    assert_eq!(spell.effects.len(), 1 + usize::from(linked_reward));
    assert!(
        matches!(spell.effects.first(), Some(OracleEffect::MultiSearch(plan)) if *plan == split_search_plan())
    );
    let SimEffect::MultiSearch(plan) = spell.effects[0].to_runtime().expect("the full plan lowers")
    else {
        panic!("the split search cannot lower as a single-result search");
    };
    assert_runtime_plan(&plan);
    let sequence = oracle_lower::lower_effect_sequence(&spell.effects)
        .expect("the complete printed sequence lowers");
    assert_eq!(sequence.len(), 1 + usize::from(linked_reward));
    let card = parse_sim_card(source);
    assert_eq!(
        card.cost,
        Cost {
            generic: 2,
            pips: [0, 0, 0, 0, 1],
            ..Default::default()
        }
    );
    assert_eq!(card.spell_data.effects.len(), sequence.len());
    let SimEffect::MultiSearch(plan) = &card.spell_data.effects[0] else {
        panic!("the complete split plan must reach spell resolution");
    };
    assert_runtime_plan(plan);
    if linked_reward {
        assert!(matches!(
            spell.effects.last(),
            Some(OracleEffect::GainLife(2))
        ));
        assert!(matches!(
            card.spell_data.effects.last(),
            Some(SimEffect::GainLife(2))
        ));
    }
    assert_eq!(card.spell_data.extra_land_drops_on_cast, 0);
    assert_eq!(
        card.spell_data.life_gain_on_cast,
        if linked_reward { 2 } else { 0 }
    );
    assert_eq!(card.support.len(), 1);
    assert!(matches!(
        &card.support[0].status,
        AbilitySupportStatus::CompilerOnly { reason }
        if reason == "Ordered effects lowered, but their ordinary runtime resolution path is not verified."
    ));
    assert_eq!(card.support[0].source, source.oracle_text);
}

/// Count physical cards across all occupied zones without counting tokens as cards.
fn physical_cards(game: &GameState) -> Vec<CardIdx> {
    let mut cards: Vec<_> = game
        .hand
        .iter()
        .chain(&game.library)
        .chain(&game.graveyard)
        .chain(&game.exile)
        .copied()
        .chain(
            game.battlefield
                .iter()
                .filter_map(|permanent| permanent.card.deck_idx()),
        )
        .collect();
    cards.sort_unstable_by_key(|index| index.0);
    cards
}

/// A paid historical search can find zero, one, or two cards without duplicating a physical result.
fn assert_paid_split_search(source: &CardRow, matches: usize, linked_reward: bool) {
    let (cards, mut game) = split_search_scenario(source);
    game.library
        .retain(|index| index.0 > 2 || index.index() <= matches);
    let before = physical_cards(&game);
    let mut expected_rng = game.library_rng.clone();
    let mut expected_library = vec![CardIdx(3), CardIdx(4)];
    rng::shuffle(&mut expected_library, &mut expected_rng);
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 0, 1],
        colorless: 2,
        ..Default::default()
    };

    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(3));

    let mut expected_hand = vec![CardIdx(5), CardIdx(10), CardIdx(11)];
    if matches == 2 {
        expected_hand.push(CardIdx(1));
    }
    let mut expected_battlefield = vec![CardIdx(8), CardIdx(9)];
    if matches > 0 {
        expected_battlefield.push(CardIdx(
            u32::try_from(matches).expect("at most two results"),
        ));
    }
    assert_eq!(pool.total(), 0);
    assert_eq!(game.hand, expected_hand);
    assert_eq!(game.graveyard, [CardIdx(6), CardIdx(0)]);
    assert_eq!(game.exile, [CardIdx(7)]);
    assert_eq!(game.library, expected_library);
    assert_battlefield(&game, &expected_battlefield);
    assert!(
        game.battlefield[..2]
            .iter()
            .all(|permanent| !permanent.tapped)
    );
    assert!(
        game.battlefield[2..]
            .iter()
            .all(|permanent| permanent.tapped)
    );
    assert_eq!(game.next_uid, 20 + u32::from(matches > 0));
    assert_eq!(game.spells_cast_this_turn, 1);
    assert_eq!(
        game.seen,
        u32::try_from(matches).expect("at most two results")
    );
    assert_eq!(game.awareness_cards, game.seen);
    assert_eq!(game.card_first_seen.len(), usize::from(matches == 2));
    if matches == 2 {
        assert_eq!(game.card_first_seen.get(&1), Some(&1));
    }
    assert_eq!(game.battlefield_seen.len(), usize::from(matches > 0));
    if matches > 0 {
        let index = CardIdx(u32::try_from(matches).expect("at most two results"));
        assert_eq!(game.battlefield_seen.get(&index), Some(&1));
    }
    assert!(game.graveyard_seen.contains_key(&CardIdx(0)));
    assert_library_rng(&game.library_rng, &expected_rng);
    assert_eq!(physical_cards(&game), before);
    assert_search_resources(&game, linked_reward);
}

/// A search does not grant unrelated resources or a second copy of its linked life gain.
fn assert_search_resources(game: &GameState, linked_reward: bool) {
    assert_eq!(game.extra_land_drops_this_turn, 0);
    assert_eq!(game.life, if linked_reward { 22 } else { 20 });
    assert_eq!(game.life_gained, if linked_reward { 2 } else { 0 });
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.treasure_bank, 0);
    assert_eq!(game.milled_self, 0);
    assert_eq!(game.milled_opp, 0);
    assert_eq!(game.extra_turns_queued, 0);
    assert!(!game.failed_draw);
    assert!(game.completion.is_none());
}

/// Original and renamed historical sources execute every supported result count.
pub(super) fn assert_split_search_actions(source: &CardRow) {
    for source in renamed(source) {
        assert_split_search_source(&source, false);
        for matches in 0..=2 {
            assert_paid_split_search(&source, matches, false);
        }
    }
}

/// A later supported reward executes only once after the complete search plan.
pub(super) fn assert_split_search_riders(source: &CardRow) {
    for mut source in renamed(source) {
        source.oracle_text.push_str(" You gain 2 life.");
        assert_split_search_source(&source, true);
        assert_paid_split_search(&source, 2, true);
    }
}

/// Report complete typed lowering under the existing conservative spell-support policy.
fn assert_report_evidence(json: &Value, source: &CardRow) {
    let support = json["card_support"]
        .as_array()
        .expect("card support is a list")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("the report includes the historical source");
    assert_eq!(support["source_hash"], card_source_hash(source));
    assert_eq!(
        support["abilities"]
            .as_array()
            .expect("ability evidence is a list")
            .len(),
        1
    );
    let ability = &support["abilities"][0];
    assert_eq!(ability["source"], source.oracle_text);
    assert_eq!(ability["status"], "compiler_only");
    assert!(ability["rule_id"].is_null());
    assert_eq!(
        ability["reason"],
        "Ordered effects lowered, but their ordinary runtime resolution path is not verified."
    );
    assert_eq!(ability["span_start"], 0);
    assert_eq!(ability["span_end"], source.oracle_text.len());
    assert!(
        support["note"]
            .as_str()
            .expect("support has a scope note")
            .contains("Compiler evidence only")
    );
}

/// A complete turn reports two found cards, one fetched permanent, and only one normal land drop.
pub(super) fn assert_split_search_report(source: &CardRow) {
    for source in renamed(source) {
        let (cards, mut game) = split_search_scenario(&source);
        let before = physical_cards(&game);
        let mut expected_rng = game.library_rng.clone();
        let mut expected_library = vec![CardIdx(3), CardIdx(4)];
        rng::shuffle(&mut expected_library, &mut expected_rng);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.hand, [CardIdx(5), CardIdx(11), CardIdx(1)]);
        assert_eq!(game.graveyard, [CardIdx(6), CardIdx(0)]);
        assert_eq!(game.exile, [CardIdx(7)]);
        assert_eq!(game.library, expected_library);
        assert_battlefield(&game, &[CardIdx(8), CardIdx(9), CardIdx(10), CardIdx(2)]);
        assert!(game.battlefield.iter().all(|permanent| permanent.tapped));
        assert_eq!(game.next_uid, 22);
        assert_eq!(game.seen, 2);
        assert_eq!(game.awareness_cards, 2);
        assert_eq!(game.spells_cast_this_turn, 1);
        assert_library_rng(&game.library_rng, &expected_rng);
        assert_eq!(physical_cards(&game), before);
        assert_search_resources(&game, false);
        assert_split_search_log(&cards, game, census, &source);
    }
}

/// Derive report expectations from the completed turn rather than from a hand-built log.
fn assert_split_search_log(cards: &SimDeck, game: GameState, census: TurnCensus, source: &CardRow) {
    let (log, json) = report_turn(cards, game, census);
    assert_eq!(log.land_drops, [1]);
    assert_eq!(log.mana_available, [3.0]);
    assert_eq!(log.mana_spent, [3.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(log.cards_seen, [2]);
    assert_eq!(log.library_size, [2]);
    assert_eq!(log.graveyard_size, [2]);
    assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 3.0);
    assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], 0.0);
    assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 2.0);
    assert_eq!(json["velocity"]["library_remaining_by_turn"]["1"], 2.0);
    assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 2.0);
    assert_report_evidence(&json, source);
}
