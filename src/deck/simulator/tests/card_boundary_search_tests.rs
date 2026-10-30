//! Search boundaries retain complete supported rules and reject unmodeled requirements.

use super::aggregate::aggregate;
use super::card_runtime_tests::{attempt, renamed};
use super::card_test_support::{assert_library_rng, main_phase_state, named_row};
use super::game::{GameLog, GameState, ManaPool, new_perm_with};
use super::game_commander::CommanderProfile;
use super::game_effects::activation::{pick_best_activation, resolve_activation, spend_leftover};
use super::game_run::{TurnCensus, census::finish_log, mana::build_pool, run_turn};
use super::model::{
    CardIdx, Cost, SearchCardType, SearchDestination, SearchSpec, SimDeck, SimEffect,
};
use super::oracle_ast::search::{OracleMultiSearch, OracleSearchPlacement};
use super::oracle_ast::{ActivationCost, OracleAbility, OracleEffect};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row};
use super::support::{AbilitySupportStatus, card_source_hash};
use crate::db::CardRow;
use serde_json::Value;

/// Historical split-search sources execute both destinations through paid actions and reports.
#[path = "card_boundary_search_supported_tests.rs"]
mod supported_searches;

/// State the complete printed split search without using parsed output as its expectation.
fn split_search_plan() -> OracleMultiSearch {
    OracleMultiSearch {
        card_type: SearchCardType::BasicLand,
        land_types: [false; 5],
        min_cards: 0,
        max_cards: 2,
        reveal: true,
        destinations: vec![
            OracleSearchPlacement {
                destination: SearchDestination::BattlefieldTapped,
                max_cards: 1,
            },
            OracleSearchPlacement {
                destination: SearchDestination::Hand,
                max_cards: 1,
            },
        ],
        shuffle: true,
    }
}

/// Keep a required name restriction that the runtime cannot check, without changing the source identity.
fn unsupported_split_search(source: &CardRow) -> CardRow {
    let mut changed = source.clone();
    changed.oracle_text = changed
        .oracle_text
        .replace("basic land cards", "basic land cards with different names");
    assert_ne!(changed.oracle_text, source.oracle_text);
    assert_eq!(changed.oracle_id, source.oracle_id);
    assert_eq!(changed.name, source.name);
    assert_ne!(card_source_hash(&changed), card_source_hash(source));
    changed
}

/// Check source evidence and atomic rejection before exercising the ordinary paid cast.
fn assert_rejected_split_search_source(source: &CardRow, linked_reward: bool) {
    super::card_ast_tests::assert_source_compiles(source);
    let oracle = parse_oracle_card(source);
    let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
        panic!("the source has one complete spell ability");
    };
    let [
        OracleEffect::Unsupported(search),
        OracleEffect::Unsupported(placement),
        rewards @ ..,
    ] = spell.effects.as_slice()
    else {
        panic!("the rejected selector and its complete placement remain unsupported");
    };
    assert_eq!(rewards.len(), usize::from(linked_reward));
    let expected = source
        .oracle_text
        .strip_suffix(" You gain 2 life.")
        .unwrap_or(&source.oracle_text);
    assert_eq!(format!("{search}, {placement}."), expected);
    assert!(
        spell.effects[..2]
            .iter()
            .all(|effect| effect.to_runtime().is_none())
    );
    assert!(super::oracle_lower::lower_effect_sequence(&spell.effects).is_none());
    if linked_reward {
        assert!(matches!(
            spell.effects.last(),
            Some(OracleEffect::GainLife(2))
        ));
    }
    let card = parse_sim_card(source);
    assert_eq!(
        card.cost,
        Cost {
            generic: 2,
            pips: [0, 0, 0, 0, 1],
            ..Cost::default()
        }
    );
    assert!(card.spell_data.effects.is_empty());
    assert_eq!(card.spell_data.extra_land_drops_on_cast, 0);
    assert_eq!(card.spell_data.life_gain_on_cast, 0);
    assert_eq!(card.support.len(), 1);
    assert!(
        matches!(&card.support[0].status, AbilitySupportStatus::Unsupported { reason }
        if reason.contains("A required clause blocks this spell's complete resolution"))
    );
    assert_eq!(card.support[0].source, source.oracle_text);
}

/// Put two basics, a nonbasic, and a nonland in an ordered library with occupied other zones.
fn split_search_scenario(source: &CardRow) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row("Library Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Library Island", "", "Basic Land — Island", "{T}: Add {U}."),
        row(
            "Library nonbasic",
            "",
            "Land — Forest Island",
            "{T}: Add {G}.",
        ),
        row("Library nonland", "{99}", "Artifact", ""),
        row("Hand marker", "{99}", "Sorcery", ""),
        row("Graveyard marker", "{99}", "Sorcery", ""),
        row("Exile marker", "{99}", "Sorcery", ""),
        row(
            "Funding Forest one",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ),
        row(
            "Funding Forest two",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ),
        row("Hand Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Hand Island", "", "Basic Land — Island", "{T}: Add {U}."),
    ]);
    let mut game = main_phase_state(vec![0, 5, 10, 11], vec![1, 2, 3, 4]);
    game.graveyard = vec![CardIdx(6)];
    game.exile = vec![CardIdx(7)];
    for index in [8, 9] {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 20;
    assert!(cards[CardIdx(1)].is_basic_land && cards[CardIdx(2)].is_basic_land);
    assert!(cards[CardIdx(3)].is_land && !cards[CardIdx(3)].is_basic_land);
    assert!(!cards[CardIdx(4)].is_land);
    (cards, game)
}

/// Check physical battlefield identities without treating created tokens as missing cards.
fn assert_battlefield(game: &GameState, expected: &[CardIdx]) {
    let actual: Vec<_> = game
        .battlefield
        .iter()
        .map(|permanent| permanent.card.deck_idx())
        .collect();
    let expected: Vec<_> = expected.iter().copied().map(Some).collect();
    assert_eq!(actual, expected);
}

/// Check resources that an inert search or a dropped linked reward must not create.
fn assert_no_search_rewards(game: &GameState) {
    assert_eq!(game.extra_land_drops_this_turn, 0);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_gained, 0);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.treasure_bank, 0);
    assert_eq!(game.awareness_cards, 0);
    assert_eq!(game.milled_self, 0);
    assert_eq!(game.milled_opp, 0);
    assert_eq!(game.extra_turns_queued, 0);
    assert!(!game.failed_draw);
    assert!(game.completion.is_none());
}

/// Require a normal three-mana cast, or an unchanged state when printed payment fails.
fn assert_split_search_cast(source: &CardRow, green: u32, colorless: u32, legal: bool) {
    let (cards, mut game) = split_search_scenario(source);
    let expected_rng = game.library_rng.clone();
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 0, green],
        colorless,
        ..ManaPool::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), legal.then_some(3));
    assert_eq!(pool.fixed, [0, 0, 0, 0, if legal { 0 } else { green }]);
    assert_eq!(pool.colorless, if legal { 0 } else { colorless });
    assert_eq!(pool.total(), if legal { 0 } else { green + colorless });
    assert_eq!(
        game.hand,
        if legal {
            vec![CardIdx(5), CardIdx(10), CardIdx(11)]
        } else {
            vec![CardIdx(0), CardIdx(5), CardIdx(10), CardIdx(11)]
        }
    );
    assert_eq!(
        game.graveyard,
        if legal {
            vec![CardIdx(6), CardIdx(0)]
        } else {
            vec![CardIdx(6)]
        }
    );
    assert_eq!(game.exile, [CardIdx(7)]);
    assert_eq!(
        game.library,
        [CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4)]
    );
    assert_battlefield(&game, &[CardIdx(8), CardIdx(9)]);
    assert!(game.battlefield.iter().all(|permanent| !permanent.tapped));
    assert_eq!(game.next_uid, 20);
    assert_eq!(game.spells_cast_this_turn, u32::from(legal));
    assert_eq!(game.seen, 0);
    assert!(game.card_first_seen.is_empty() && game.battlefield_seen.is_empty());
    assert_eq!(game.graveyard_seen.contains_key(&CardIdx(0)), legal);
    assert_library_rng(&game.library_rng, &expected_rng);
    assert_no_search_rewards(&game);
}

/// Check complete printed searches and nearby unsupported name restrictions under paid casts.
pub(super) fn assert_split_search_actions(source: &CardRow) {
    supported_searches::assert_split_search_actions(source);
    for source in renamed(source) {
        let changed = unsupported_split_search(&source);
        assert_rejected_split_search_source(&changed, false);
        assert_split_search_cast(&changed, 1, 2, true);
        assert_split_search_cast(&changed, 0, 3, false);
        assert_split_search_cast(&changed, 1, 1, false);
        assert_split_search_cast(&source, 0, 3, false);
        assert_split_search_cast(&source, 1, 1, false);
    }
}

/// A supported reward cannot run after an unsupported required search in the same spell.
pub(super) fn assert_split_search_riders(source: &CardRow) {
    supported_searches::assert_split_search_riders(source);
    for source in renamed(source) {
        let mut source = unsupported_split_search(&source);
        source.oracle_text.push_str(" You gain 2 life.");
        assert_rejected_split_search_source(&source, true);
        assert_split_search_cast(&source, 1, 2, true);
    }
}

/// Execute every production turn phase before producing census-based report data.
fn execute_turn(cards: &SimDeck, game: &mut GameState) -> TurnCensus {
    let mut census = TurnCensus::new(1, cards.cards.len());
    run_turn(
        cards,
        game,
        &mut census,
        &CommanderProfile::new(cards),
        &mut Vec::new(),
        1,
        false,
    );
    assert_eq!(census.turn_observed, [true]);
    census
}

/// Emit the production JSON report from an executed state and its real turn census.
fn report_turn(cards: &SimDeck, game: GameState, census: TurnCensus) -> (GameLog, Value) {
    let log = finish_log(cards, game, census, 0, false, 1);
    let stats = aggregate(std::slice::from_ref(&log), cards, 1);
    let report = super::report::json_report(
        &stats,
        cards,
        "Search safety boundary",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    let json = serde_json::to_value(report).expect("the production report serializes");
    assert_eq!(json["outcomes"]["requested_games"], 1);
    assert_eq!(json["outcomes"]["finished_games"], 1);
    assert_eq!(json["outcomes"]["incomplete_games"], 0);
    assert_eq!(log.turns_observed, 1);
    (log, json)
}

/// Require unsupported evidence with its original wording, exact span, hash, and useful reason.
fn assert_report_limitation(json: &Value, source: &CardRow, text: &str, reason_fragment: &str) {
    let card = json["card_support"]
        .as_array()
        .expect("card support is a list")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("the report includes the source");
    assert_eq!(card["source_hash"], card_source_hash(source));
    let ability = card["abilities"]
        .as_array()
        .expect("ability evidence is a list")
        .iter()
        .find(|ability| ability["source"] == text)
        .expect("the exact source wording is reported");
    assert_eq!(ability["status"], "unsupported");
    assert!(ability["rule_id"].is_null());
    assert!(
        ability["reason"]
            .as_str()
            .expect("unsupported evidence has a reason")
            .contains(reason_fragment)
    );
    let start = source
        .oracle_text
        .find(text)
        .expect("the evidence comes from the source");
    assert_eq!(ability["span_start"], start);
    assert_eq!(ability["span_end"], start + text.len());
    assert!(
        card["note"]
            .as_str()
            .expect("support has a scope note")
            .contains("Compiler evidence only")
    );
}

/// Reports must distinguish complete printed searches from rejected required name restrictions.
pub(super) fn assert_split_search_report(source: &CardRow) {
    supported_searches::assert_split_search_report(source);
    assert_rejected_split_search_report(source);
}

/// A rejected search retains the spare hand land and reports only the real land drop and cast spend.
fn assert_rejected_split_search_report(source: &CardRow) {
    for source in renamed(source) {
        let source = unsupported_split_search(&source);
        let (cards, mut game) = split_search_scenario(&source);
        let expected_rng = game.library_rng.clone();
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.hand, [CardIdx(5), CardIdx(11)]);
        assert_eq!(game.graveyard, [CardIdx(6), CardIdx(0)]);
        assert_eq!(game.exile, [CardIdx(7)]);
        assert_eq!(
            game.library,
            [CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4)]
        );
        assert_battlefield(&game, &[CardIdx(8), CardIdx(9), CardIdx(10)]);
        assert!(game.battlefield.iter().all(|permanent| permanent.tapped));
        assert_eq!(game.next_uid, 21);
        assert_eq!(game.seen, 0);
        assert_eq!(game.spells_cast_this_turn, 1);
        for index in 1..=4 {
            assert!(!game.card_first_seen.contains_key(&index));
            assert!(
                !game
                    .battlefield_seen
                    .contains_key(&CardIdx(u32::try_from(index).expect("small card index")))
            );
        }
        assert_library_rng(&game.library_rng, &expected_rng);
        assert_no_search_rewards(&game);
        let (log, json) = report_turn(&cards, game, census);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.mana_available, [3.0]);
        assert_eq!(log.mana_spent, [3.0]);
        assert_eq!(log.mana_remaining, [0.0]);
        assert_eq!(log.cards_seen, [0]);
        assert_eq!(log.library_size, [4]);
        assert_eq!(log.graveyard_size, [2]);
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 3.0);
        assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], 0.0);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 0.0);
        assert_eq!(json["velocity"]["library_remaining_by_turn"]["1"], 4.0);
        assert_eq!(json["graveyard"]["avg_size_by_turn"]["1"], 2.0);
        assert_report_limitation(
            &json,
            &source,
            &source.oracle_text,
            "A required clause blocks this spell's complete resolution",
        );
        assert!(
            !json["assumptions"]
                .as_array()
                .expect("report assumptions are a list")
                .iter()
                .any(|assumption| assumption.as_str().is_some_and(|text| text
                    .contains("Multiple-result searches such as Cultivate stay unsupported")))
        );
    }
}

/// State the exact supported control grammar and its complete expected runtime search.
fn single_land_search(source: &CardRow) -> CardRow {
    let mut changed = source.clone();
    changed.oracle_text = "Search your library for a basic land card, put that card onto the battlefield tapped, then shuffle.".into();
    let expected = SearchSpec {
        card_type: Some(SearchCardType::BasicLand),
        basic_land_only: true,
        destination: SearchDestination::BattlefieldTapped,
        shuffle: true,
        ..SearchSpec::default()
    };
    let oracle = parse_oracle_card(&changed);
    let [OracleAbility::Spell(spell)] = oracle.abilities.as_slice() else {
        panic!("the single-search control has one spell ability");
    };
    let [OracleEffect::Search(actual)] = spell.effects.as_slice() else {
        panic!("the exact single-search grammar parses");
    };
    assert_search_spec(actual, &expected);
    let card = parse_sim_card(&changed);
    let [SimEffect::Search(actual)] = card.spell_data.effects.as_slice() else {
        panic!("the single-search grammar lowers without extra effects");
    };
    assert_search_spec(actual, &expected);
    changed
}

/// Compare every selector and destination field without ignoring an added restriction.
fn assert_search_spec(actual: &SearchSpec, expected: &SearchSpec) {
    assert_eq!(actual.card_type, expected.card_type);
    assert_eq!(actual.color, expected.color);
    assert_eq!(actual.colorless, expected.colorless);
    assert_eq!(actual.mana_value, expected.mana_value);
    assert_eq!(actual.max_mana_value, expected.max_mana_value);
    assert_eq!(actual.min_mana_value, expected.min_mana_value);
    assert_eq!(actual.destination, expected.destination);
    assert_eq!(actual.optional, expected.optional);
    assert_eq!(actual.top_count, expected.top_count);
    assert_eq!(actual.shuffle, expected.shuffle);
    assert_eq!(actual.non_human, expected.non_human);
    assert_eq!(actual.land_types, expected.land_types);
    assert_eq!(actual.basic_land_only, expected.basic_land_only);
}

/// Changing the wording, while keeping either source name, enables a real paid search and shuffle.
pub(super) fn assert_split_search_control(source: &CardRow) {
    for source in renamed(source) {
        let changed = single_land_search(&source);
        let (cards, mut game) = split_search_scenario(&changed);
        let mut expected_library = vec![CardIdx(1), CardIdx(3), CardIdx(4)];
        let mut expected_rng = game.library_rng.clone();
        super::rng::shuffle(&mut expected_library, &mut expected_rng);
        assert!(expected_rng.get_word_pos() > game.library_rng.get_word_pos());
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 0, 1],
            colorless: 2,
            ..ManaPool::default()
        };
        assert_eq!(attempt(&cards, &mut game, &mut pool), Some(3));
        assert_eq!(pool.total(), 0);
        assert_eq!(game.hand, [CardIdx(5), CardIdx(10), CardIdx(11)]);
        assert_eq!(game.graveyard, [CardIdx(6), CardIdx(0)]);
        assert_eq!(game.exile, [CardIdx(7)]);
        assert_battlefield(&game, &[CardIdx(8), CardIdx(9), CardIdx(2)]);
        assert!(game.battlefield[2].tapped);
        assert_eq!(game.library, expected_library);
        assert_eq!(game.seen, 1);
        assert_library_rng(&game.library_rng, &expected_rng);
        let (cards, mut game) = split_search_scenario(&changed);
        let census = execute_turn(&cards, &mut game);
        assert_battlefield(&game, &[CardIdx(8), CardIdx(9), CardIdx(10), CardIdx(2)]);
        assert_eq!(game.hand, [CardIdx(5), CardIdx(11)]);
        assert_eq!(game.library, expected_library);
        assert_library_rng(&game.library_rng, &expected_rng);
        let (log, json) = report_turn(&cards, game, census);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.mana_spent, [3.0]);
        assert_eq!(log.library_size, [3]);
        assert_eq!(log.cards_seen, [1]);
        assert_eq!(json["velocity"]["library_remaining_by_turn"]["1"], 3.0);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 1.0);
    }
}

/// Keep the genuine source, a renamed source, and isolated unsupported selector controls.
fn sisay_sources(source: &CardRow) -> Vec<CardRow> {
    let mut renamed = source.clone();
    renamed.name = "Renamed search source".into();
    renamed.oracle_text = source.oracle_text.replace("Sisay", &renamed.name);
    let mut sources = vec![source.clone(), renamed];
    for selector in [
        "a legendary permanent card",
        "a permanent card with mana value less than 2",
        "a permanent card with mana value less than Sisay's power",
    ] {
        let mut changed = source.clone();
        changed.oracle_text = format!(
            "{{W}}{{U}}{{B}}{{R}}{{G}}: Search your library for {selector}, put that card onto the battlefield, then shuffle."
        );
        sources.push(changed);
    }
    sources
}

/// Retain exact WUBRG payment and original source evidence while declining the whole activation.
fn assert_sisay_source(source: &CardRow) {
    super::card_ast_tests::assert_source_compiles(source);
    super::card_ast_tests::assert_sisay_inert(source);
    let oracle = parse_oracle_card(source);
    let (index, activation) = oracle
        .abilities
        .iter()
        .enumerate()
        .find_map(|(index, ability)| match ability {
            OracleAbility::Activated(activation) => Some((index, activation)),
            _ => None,
        })
        .expect("the printed activation parses");
    let expected_cost = Cost {
        pips: [1; 5],
        ..Cost::default()
    };
    assert!(
        matches!(activation.costs.as_slice(), [ActivationCost::Mana(cost)]
        if *cost == expected_cost)
    );
    assert!(matches!(
        activation.effects.as_slice(),
        [OracleEffect::Unsupported(_)]
    ));
    assert!(activation.to_runtime_all().is_empty());
    let card = parse_sim_card(source);
    assert!(
        card.unlocked_abilities(0)
            .all(|ability| !ability.kind.is_activated())
    );
    let evidence = &card.support[index];
    assert_eq!(
        evidence.source,
        source
            .oracle_text
            .lines()
            .last()
            .expect("the source has an activation")
    );
    assert!(matches!(
        evidence.status,
        AbilitySupportStatus::Unsupported { .. }
    ));
}

/// Supply lower and equal mana values, a nonlegendary candidate, and five real colored sources.
fn sisay_scenario(source: &CardRow) -> (SimDeck, GameState) {
    let cards = deck(&[
        source.clone(),
        row("Low legendary candidate", "{1}", "Legendary Artifact", ""),
        row("Equal legendary candidate", "{2}", "Legendary Artifact", ""),
        row("Low nonlegendary candidate", "{1}", "Artifact", ""),
        row("High legendary candidate", "{3}", "Legendary Artifact", ""),
        row("Hand marker", "{99}", "Sorcery", ""),
        row("Graveyard marker", "{99}", "Sorcery", ""),
        row("Exile marker", "{99}", "Sorcery", ""),
        row("Funding Plains", "", "Basic Land — Plains", "{T}: Add {W}."),
        row("Funding Island", "", "Basic Land — Island", "{T}: Add {U}."),
        row("Funding Swamp", "", "Basic Land — Swamp", "{T}: Add {B}."),
        row(
            "Funding Mountain",
            "",
            "Basic Land — Mountain",
            "{T}: Add {R}.",
        ),
        row("Funding Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
    ]);
    let mut game = main_phase_state(vec![5], vec![1, 2, 3, 4]);
    game.graveyard = vec![CardIdx(6)];
    game.exile = vec![CardIdx(7)];
    for index in [0, 8, 9, 10, 11, 12] {
        game.battlefield
            .push(new_perm_with(index + 20, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 40;
    assert_eq!(cards[CardIdx(0)].printed_power, Some(2));
    assert!(cards[CardIdx(1)].is_legendary && cards[CardIdx(2)].is_legendary);
    assert_eq!(cards[CardIdx(1)].mana_value, 1);
    assert_eq!(cards[CardIdx(2)].mana_value, 2);
    assert!(!cards[CardIdx(3)].is_legendary);
    assert_eq!(cards[CardIdx(3)].mana_value, 1);
    (cards, game)
}

/// Tap the five native funding lands before selecting a nonmana activation.
fn sisay_pool(cards: &SimDeck, game: &mut GameState) -> ManaPool {
    let pool = build_pool(cards, game, 1);
    assert_eq!(pool.fixed, [1; 5]);
    assert_eq!(pool.total(), 5);
    assert!(!game.battlefield[0].tapped && !game.battlefield[0].fired);
    assert!(
        game.battlefield[1..]
            .iter()
            .all(|permanent| permanent.tapped && !permanent.fired)
    );
    pool
}

/// Require all candidates and occupied zones to survive an unsupported activation pass.
fn assert_sisay_inert_state(game: &GameState) {
    assert_eq!(game.hand, [CardIdx(5)]);
    assert_eq!(
        game.library,
        [CardIdx(1), CardIdx(2), CardIdx(3), CardIdx(4)]
    );
    assert_eq!(game.graveyard, [CardIdx(6)]);
    assert_eq!(game.exile, [CardIdx(7)]);
    assert_battlefield(
        game,
        &[
            CardIdx(0),
            CardIdx(8),
            CardIdx(9),
            CardIdx(10),
            CardIdx(11),
            CardIdx(12),
        ],
    );
    assert_eq!(game.next_uid, 40);
    assert_eq!(game.seen, 0);
    assert_eq!(game.spells_cast_this_turn, 0);
    assert!(game.activated_this_turn.is_empty() && game.activated_once.is_empty());
    for index in 1..=4 {
        assert!(!game.card_first_seen.contains_key(&index));
        assert!(
            !game
                .battlefield_seen
                .contains_key(&CardIdx(u32::try_from(index).expect("small card index")))
        );
    }
    assert_no_search_rewards(game);
}

/// Missing grammar cannot choose or pay an activation even when every color and a legal-looking hit exist.
pub(super) fn assert_sisay_actions(source: &CardRow) {
    for source in sisay_sources(source) {
        assert_sisay_source(&source);
        let (cards, mut game) = sisay_scenario(&source);
        let expected_rng = game.library_rng.clone();
        let funding = pick_best_activation(&cards, &game, &ManaPool::default(), 1)
            .expect("an untapped funding land has a native mana activation");
        assert_eq!(funding.uid, 28);
        assert_eq!(
            game.battlefield[funding.pos].card.deck_idx(),
            Some(CardIdx(8))
        );
        let mut pool = sisay_pool(&cards, &mut game);
        let before = pool.clone();
        let remaining = pick_best_activation(&cards, &game, &pool, 1);
        assert!(
            remaining.is_none(),
            "unexpected activation from source uid {:?}",
            remaining.as_ref().map(|activation| activation.uid)
        );
        spend_leftover(&cards, &mut game, &mut pool, 1);
        assert_eq!(pool, before);
        assert!(!game.battlefield[0].tapped && !game.battlefield[0].fired);
        assert!(
            game.battlefield[1..]
                .iter()
                .all(|permanent| permanent.tapped && !permanent.fired)
        );
        assert_sisay_inert_state(&game);
        assert_library_rng(&game.library_rng, &expected_rng);
    }
}

/// Full-turn reports must expose the blocked source while retaining all five unused mana.
pub(super) fn assert_sisay_report(source: &CardRow) {
    for source in sisay_sources(source) {
        let (cards, mut game) = sisay_scenario(&source);
        let expected_rng = game.library_rng.clone();
        let census = execute_turn(&cards, &mut game);
        assert_sisay_inert_state(&game);
        assert!(
            game.battlefield[1..]
                .iter()
                .all(|permanent| permanent.tapped)
        );
        assert_library_rng(&game.library_rng, &expected_rng);
        let (log, json) = report_turn(&cards, game, census);
        assert_eq!(log.land_drops, [0]);
        assert_eq!(log.mana_available, [5.0]);
        assert_eq!(log.mana_spent, [0.0]);
        assert_eq!(log.mana_remaining, [5.0]);
        assert_eq!(log.cards_seen, [0]);
        assert_eq!(log.library_size, [4]);
        assert_eq!(log.graveyard_size, [1]);
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], 0.0);
        assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], 5.0);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 0.0);
        assert_eq!(json["velocity"]["library_remaining_by_turn"]["1"], 4.0);
        assert_report_limitation(
            &json,
            &source,
            source
                .oracle_text
                .lines()
                .last()
                .expect("the source has an activation"),
            "cannot lower as a complete ability",
        );
    }
}

/// Replace the unsupported selector with an exact supported artifact, value, and hand destination.
fn sisay_supported_search(source: &CardRow) -> CardRow {
    let mut changed = source.clone();
    changed.oracle_text = "{W}{U}{B}{R}{G}: Search your library for an artifact card with mana value 1, put it into your hand, then shuffle.".into();
    let expected = SearchSpec {
        card_type: Some(SearchCardType::Artifact),
        mana_value: Some(1),
        destination: SearchDestination::Hand,
        shuffle: true,
        ..SearchSpec::default()
    };
    let oracle = parse_oracle_card(&changed);
    let [OracleAbility::Activated(activation)] = oracle.abilities.as_slice() else {
        panic!("the supported control has one activation");
    };
    let [OracleEffect::Search(actual)] = activation.effects.as_slice() else {
        panic!("the exact artifact-search grammar parses");
    };
    assert_search_spec(actual, &expected);
    let card = parse_sim_card(&changed);
    let abilities: Vec<_> = card.unlocked_abilities(0).collect();
    assert_eq!(abilities.len(), 1);
    let [SimEffect::Search(actual)] = abilities[0].effect_sequence() else {
        panic!("the artifact-search grammar lowers without extra effects");
    };
    assert_search_spec(actual, &expected);
    assert_eq!(
        abilities[0]
            .activation
            .as_ref()
            .expect("activation payment lowers")
            .mana_cost(),
        Cost {
            pips: [1; 5],
            ..Cost::default()
        }
    );
    assert_eq!(card.support.len(), 1);
    assert!(matches!(
        card.support[0].status,
        AbilitySupportStatus::Approximated {
            rule_id: "bounded_search_selection"
        }
    ));
    changed
}

/// The same source identity can run a real supported search only after its wording changes.
pub(super) fn assert_sisay_control(source: &CardRow) {
    for source in sisay_sources(source).into_iter().take(2) {
        let changed = sisay_supported_search(&source);
        assert_eq!(changed.oracle_id, source.oracle_id);
        assert_eq!(changed.name, source.name);
        assert_ne!(card_source_hash(&source), card_source_hash(&changed));
        let (cards, mut game) = sisay_scenario(&changed);
        let mut expected_library = vec![CardIdx(1), CardIdx(2), CardIdx(4)];
        let mut expected_rng = game.library_rng.clone();
        super::rng::shuffle(&mut expected_library, &mut expected_rng);
        assert!(expected_rng.get_word_pos() > game.library_rng.get_word_pos());
        let mut pool = sisay_pool(&cards, &mut game);
        let activation =
            pick_best_activation(&cards, &game, &pool, 1).expect("supported search is selectable");
        assert_eq!(activation.uid, 20);
        assert_eq!(
            activation.ability_cost,
            Cost {
                pips: [1; 5],
                ..Cost::default()
            }
        );
        resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
        assert_eq!(pool.fixed, [0; 5]);
        assert_eq!(pool.total(), 0);
        assert!(!game.battlefield[0].tapped && !game.battlefield[0].fired);
        assert!(
            game.battlefield[1..]
                .iter()
                .all(|permanent| permanent.tapped)
        );
        assert_eq!(game.hand, [CardIdx(5), CardIdx(3)]);
        assert_eq!(game.library, expected_library);
        assert_eq!(game.graveyard, [CardIdx(6)]);
        assert_eq!(game.exile, [CardIdx(7)]);
        assert_battlefield(
            &game,
            &[
                CardIdx(0),
                CardIdx(8),
                CardIdx(9),
                CardIdx(10),
                CardIdx(11),
                CardIdx(12),
            ],
        );
        assert_eq!(game.seen, 1);
        assert_eq!(game.next_uid, 40);
        assert_no_search_rewards(&game);
        assert_library_rng(&game.library_rng, &expected_rng);
        let (cards, mut game) = sisay_scenario(&changed);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.hand, [CardIdx(5), CardIdx(3)]);
        assert_eq!(game.library, expected_library);
        assert_eq!(game.graveyard, [CardIdx(6)]);
        assert_eq!(game.exile, [CardIdx(7)]);
        assert_battlefield(
            &game,
            &[
                CardIdx(0),
                CardIdx(8),
                CardIdx(9),
                CardIdx(10),
                CardIdx(11),
                CardIdx(12),
            ],
        );
        assert_no_search_rewards(&game);
        assert_library_rng(&game.library_rng, &expected_rng);
        let (log, json) = report_turn(&cards, game, census);
        assert_eq!(log.mana_spent, [0.0]);
        assert_eq!(log.mana_remaining, [0.0]);
        assert_eq!(log.cards_seen, [1]);
        assert_eq!(log.library_size, [3]);
        assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], 0.0);
        assert_eq!(json["velocity"]["avg_cards_seen"]["1"], 1.0);
        let support = json["card_support"]
            .as_array()
            .expect("support is a list")
            .iter()
            .find(|card| card["name"] == changed.name)
            .expect("the changed source is reported");
        assert_eq!(support["source_hash"], card_source_hash(&changed));
        assert_eq!(support["abilities"][0]["source"], changed.oracle_text);
        assert_eq!(support["abilities"][0]["status"], "approximated");
        assert_eq!(
            support["abilities"][0]["rule_id"],
            "bounded_search_selection"
        );
        assert!(support["abilities"][0]["reason"].is_null());
        assert_eq!(support["abilities"][0]["span_start"], 0);
        assert_eq!(
            support["abilities"][0]["span_end"],
            changed.oracle_text.len()
        );
    }
}

/// Both pinned spells execute complete plans; required name restrictions still block movement.
#[test]
fn paid_split_search_casts_preserve_supported_and_rejected_boundaries() {
    for name in ["Cultivate", "Kodama's Reach"] {
        assert_split_search_actions(&named_row(name));
    }
}

/// Linked rewards run after complete searches, but not after unsupported name restrictions.
#[test]
fn split_search_riders_cannot_pay_out_after_unsupported_search() {
    for name in ["Cultivate", "Kodama's Reach"] {
        assert_split_search_riders(&named_row(name));
    }
}

/// Executed turns report real search movement and exact rejected-source evidence separately.
#[test]
fn split_search_turns_report_spend_without_phantom_ramp() {
    for name in ["Cultivate", "Kodama's Reach"] {
        assert_split_search_report(&named_row(name));
    }
}

/// The exact supported single-search wording changes the same source's actual behavior.
#[test]
fn split_search_changed_text_executes_supported_single_search() {
    for name in ["Cultivate", "Kodama's Reach"] {
        assert_split_search_control(&named_row(name));
    }
}

/// Unsupported Sisay wording cannot spend its fully available native colored mana.
#[test]
fn sisay_unsupported_selectors_never_activate_with_all_five_colors() {
    assert_sisay_actions(&named_row("Sisay, Weatherlight Captain"));
}

/// Executed Sisay turns retain both unsupported evidence and unused mana.
#[test]
fn sisay_full_turn_reports_keep_unsupported_library_and_mana() {
    assert_sisay_report(&named_row("Sisay, Weatherlight Captain"));
}

/// Supported wording enables paid movement under the declared search approximation.
#[test]
fn sisay_changed_text_runs_a_paid_supported_search() {
    assert_sisay_control(&named_row("Sisay, Weatherlight Captain"));
}
