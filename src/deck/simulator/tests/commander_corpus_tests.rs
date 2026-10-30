//! Broad source smoke checks do not certify complete card rules.

use super::aggregate::aggregate;
use super::card_report_tests::execute_turn;
use super::commander_test_support::rows;
use super::game::{CardRef, GameState, new_perm_with};
use super::game_run::census::finish_log;
use super::model::{CardIdx, Format, SimDeck};
use super::oracle_lower::parse_sim_card;
use super::scenario_test_support::{deck, row, state};
use crate::db::CardRow;
use std::collections::BTreeSet;

/// Give each source the same library and a five-color commander.
fn controlled_deck(source: &CardRow) -> SimDeck {
    let mut inputs = vec![source.clone()];
    for index in 0..20 {
        inputs.push(row(
            &format!("Control source {index}"),
            "",
            "Land",
            if index % 5 == 0 {
                "{T}: Add {C}."
            } else {
                "{T}: Add one mana of any color."
            },
        ));
    }
    for (name, land_type, mana) in [
        ("Plains", "Plains", "{W}"),
        ("Island", "Island", "{U}"),
        ("Swamp", "Swamp", "{B}"),
        ("Mountain", "Mountain", "{R}"),
        ("Forest", "Forest", "{G}"),
    ] {
        for _ in 0..4 {
            inputs.push(row(
                name,
                "",
                &format!("Basic Land — {land_type}"),
                &format!("{{T}}: Add {mana}."),
            ));
        }
    }
    for index in 0..30 {
        let mut creature = row(&format!("Control creature {index}"), "{3}", "Creature", "");
        creature.power = Some("2".into());
        creature.toughness = Some("2".into());
        inputs.push(creature);
    }
    let mut cards = deck(&inputs);
    cards.format = Format::Commander;
    cards.rules = super::format::rules_for("commander");
    let mut commander = row(
        "Control commander",
        "{W}{U}{B}{R}{G}",
        "Legendary Creature",
        "",
    );
    commander.color_identity = r#"["W","U","B","R","G"]"#.into();
    commander.power = Some("2".into());
    commander.toughness = Some("2".into());
    cards.commanders.push(parse_sim_card(&commander));
    cards
}

/// Start with the source in hand and either no board or twenty mana sources.
fn controlled_state(cards: &SimDeck, established: bool) -> GameState {
    let mut game = state(vec![0], Vec::new());
    game.life = 40;
    game.opponents = super::opponents::OpponentTable::for_format(Format::Commander);
    game.commander_casts = vec![0];
    for index in 1..cards.cards.len() {
        let card = CardIdx(index as u32);
        if established && index <= 20 {
            let uid = game.next_uid;
            game.next_uid += 1;
            game.battlefield
                .push(new_perm_with(uid, cards, card, 0, false));
        } else {
            game.library.push(card);
        }
    }
    game
}

/// Every physical library card must occupy exactly one modeled zone.
fn assert_card_conservation(cards: &SimDeck, game: &GameState) {
    let mut locations: Vec<CardIdx> = game
        .library
        .iter()
        .chain(&game.hand)
        .chain(&game.graveyard)
        .chain(&game.exile)
        .copied()
        .collect();
    locations.extend(game.plotted.iter().map(|(card, _)| *card));
    locations.extend(
        game.battlefield
            .iter()
            .filter_map(|permanent| match permanent.card {
                CardRef::Deck(card) => Some(card),
                _ => None,
            }),
    );
    locations.sort_by_key(|card| card.0);
    assert_eq!(
        locations,
        (0..cards.cards.len())
            .map(|index| CardIdx(index as u32))
            .collect::<Vec<_>>(),
        "{}: a physical card was lost or duplicated",
        cards.cards[0].name
    );
}

/// Compile every captured row without substituting names or Oracle inputs.
#[test]
fn expanded_commander_sources_compile_with_distinct_source_identity() {
    let sources = rows();
    assert!(sources.len() > 1000);
    let mut identities = BTreeSet::new();
    let mut hashes = BTreeSet::new();
    for source in sources {
        assert!(
            identities.insert(source.oracle_id.clone()),
            "{}",
            source.name
        );
        let card = parse_sim_card(&source);
        assert_eq!(card.name, source.name);
        assert!(!card.source_hash.is_empty(), "{}", source.name);
        assert!(hashes.insert(card.source_hash.clone()), "{}", source.name);
        assert_eq!(
            format!("{:?}", card.support),
            format!("{:?}", parse_sim_card(&source).support),
            "{}: compiler evidence changed between equal inputs",
            source.name
        );
    }
}

/// Exercise real turn actions while keeping gaps, losses, and limits visible.
#[test]
fn expanded_commander_sources_preserve_cards_and_report_all_outcomes() {
    for source in rows() {
        let cards = controlled_deck(&source);
        for established in [false, true] {
            let mut game = controlled_state(&cards, established);
            assert_card_conservation(&cards, &game);
            let census = execute_turn(&cards, &mut game);
            assert_card_conservation(&cards, &game);
            let config = game.opponents.config.clone();
            let log = finish_log(&cards, game, census, 0, false, 1);
            let stats = aggregate(std::slice::from_ref(&log), &cards, 1);
            let mut report = super::report::json_report(
                &stats,
                &cards,
                "Expanded source smoke check",
                0,
                &[],
                Default::default(),
                &Default::default(),
            );
            super::report::set_opponent_model(&mut report, cards.format, config, &stats);
            assert_eq!(report.provenance.schema_version, 3);
            assert_eq!(report.provenance.model_version, 2);
            assert_eq!(report.provenance.policy_version, 2);
            let json = serde_json::to_value(&report).expect("current report serializes");
            assert_eq!(report.outcomes.requested_games, 1);
            assert_eq!(
                report.outcomes.finished_games + report.outcomes.incomplete_games,
                1
            );
            let record = json["card_support"]
                .as_array()
                .expect("compiler records")
                .iter()
                .find(|record| record["name"] == source.name)
                .expect("the original card remains in the report");
            assert_eq!(record["source_hash"], cards.cards[0].source_hash);
            assert_eq!(
                record["abilities"]
                    .as_array()
                    .expect("ability records")
                    .len(),
                cards.cards[0].support.len()
            );
            assert!(
                !json["assumptions"]
                    .as_array()
                    .expect("model limits")
                    .is_empty()
            );
            assert!(serde_json::from_value::<super::SimReport>(json).is_ok());
        }
    }
}

/// A supported search cannot hide an unsupported mandatory scaled drawback.
#[test]
fn expanded_searches_block_unmodeled_trailing_life_loss() {
    for tail in [
        "You lose 2 life for each land you control.",
        "You lose 2 life unless you control an Elf.",
        "Each opponent loses life equal to the number of lands you control.",
    ] {
        let mut source = super::commander_test_support::named_row("Cultivate");
        source.oracle_text.push(' ');
        source.oracle_text.push_str(tail);
        let cards = controlled_deck(&source);
        assert!(cards.cards[0].spell_data.effects.is_empty(), "{tail}");
        let mut game = controlled_state(&cards, true);
        let before_library = game.library.clone();
        let mut pool = super::game::ManaPool {
            flexible: 20,
            ..Default::default()
        };
        assert_eq!(
            super::card_runtime_tests::attempt(&cards, &mut game, &mut pool),
            Some(3),
            "{tail}"
        );
        assert_eq!(pool.total(), 17);
        assert!(game.hand.is_empty());
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.library, before_library);
        assert_eq!(game.life, 40);
        assert_eq!(game.spells_cast_this_turn, 1);
        assert_card_conservation(&cards, &game);
    }
}

/// Single-result top tutors must retain their printed shuffle-before-placement order.
#[test]
fn single_top_search_rejects_an_unmodeled_trailing_shuffle() {
    for text in [
        "Search your library for a card, put that card on top of your library, then shuffle. You lose 2 life.",
        "Search your library for a card, then shuffle and put that card on top of your library, then shuffle.",
    ] {
        let mut source = super::commander_test_support::named_row("Vampiric Tutor");
        source.oracle_text = text.into();
        let cards = controlled_deck(&source);
        assert!(cards.cards[0].spell_data.effects.is_empty());
        let mut game = controlled_state(&cards, true);
        let before_library = game.library.clone();
        let mut pool = super::game::ManaPool {
            flexible: 20,
            ..Default::default()
        };
        assert_eq!(
            super::card_runtime_tests::attempt(&cards, &mut game, &mut pool),
            Some(1)
        );
        assert_eq!(game.library, before_library);
        assert!(game.hand.is_empty());
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.life, 40);
        assert_eq!(pool.total(), 19);
        assert_card_conservation(&cards, &game);
    }
}

/// Unsupported subjects and scaled counts cannot change the player's library.
#[test]
fn library_choice_mutations_stay_inert_through_paid_casts() {
    for text in [
        "Target opponent may scry 2.",
        "You may surveil 2 for each land you control.",
        "Scry 2 unless you control an Elf.",
    ] {
        let source = row("Unsupported library choice", "{1}", "Sorcery", text);
        let cards = controlled_deck(&source);
        assert!(cards.cards[0].spell_data.effects.is_empty(), "{text}");
        let mut game = controlled_state(&cards, true);
        let library = game.library.clone();
        let mut pool = super::game::ManaPool {
            flexible: 20,
            ..Default::default()
        };
        assert_eq!(
            super::card_runtime_tests::attempt(&cards, &mut game, &mut pool),
            Some(1),
            "{text}"
        );
        assert_eq!(game.library, library);
        assert!(game.hand.is_empty());
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(pool.total(), 19);
        assert_eq!(game.awareness_cards, 0);
        assert_card_conservation(&cards, &game);
    }
}
