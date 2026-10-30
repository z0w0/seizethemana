//! Source searches and recursion retain physical targets and deterministic pilot choices.

use super::{attempt, renamed};
use crate::db::CardRow;
use crate::deck::simulator::card_test_support::{assert_library_rng, main_phase_state as state};
use crate::deck::simulator::game::{GameState, ManaPool, new_perm_with};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row};
use rand_chacha::ChaCha8Rng;

/// Physical setup and seeded shuffle expectations shared by tutor action and turn cases.
pub(in crate::deck::simulator) struct TopTutorScenario {
    pub(in crate::deck::simulator) cards: SimDeck,
    pub(in crate::deck::simulator) game: GameState,
    pub(in crate::deck::simulator) selected: Option<CardIdx>,
    pub(in crate::deck::simulator) expected_library: Vec<CardIdx>,
    pub(in crate::deck::simulator) expected_exile: Vec<CardIdx>,
    pub(in crate::deck::simulator) expected_rng: ChaCha8Rng,
}

/// Put Enlightened Tutor's sole target below four misses; Vampiric Tutor uses the known top-first pilot.
pub(in crate::deck::simulator) fn top_tutor_scenario(
    source: &CardRow,
    vampiric: bool,
    has_target: bool,
) -> TopTutorScenario {
    let candidate_kind = if has_target { "Artifact" } else { "Sorcery" };
    let (land, mana) = if vampiric {
        ("Swamp", "{T}: Add {B}.")
    } else {
        ("Plains", "{T}: Add {W}.")
    };
    let mut rows = vec![
        source.clone(),
        row("Bottom candidate", "{99}", candidate_kind, ""),
    ];
    rows.extend((0..4).map(|_| row("Other physical card", "{99}", "Sorcery", "")));
    rows.push(row(
        "Funding land",
        "",
        &format!("Basic Land — {land}"),
        mana,
    ));
    let cards = deck(&rows);
    let mut game = state(
        vec![0],
        if vampiric && !has_target {
            vec![]
        } else {
            vec![1, 2, 3, 4, 5]
        },
    );
    if vampiric && !has_target {
        game.exile = (1..=5).map(CardIdx).collect();
    }
    game.library_rng = crate::deck::simulator::rng::game_rng(
        7319,
        0,
        crate::deck::simulator::rng::RngDomain::Library,
    );
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(6), 0, false));
    let selected = has_target.then_some(if vampiric { CardIdx(5) } else { CardIdx(1) });
    if has_target && !vampiric {
        assert_ne!(game.library.last(), selected.as_ref());
    }
    let mut expected_library = game.library.clone();
    if let Some(selected) = selected {
        expected_library.retain(|index| *index != selected);
    }
    let mut expected_rng = game.library_rng.clone();
    crate::deck::simulator::rng::shuffle(&mut expected_library, &mut expected_rng);
    if let Some(selected) = selected {
        expected_library.push(selected);
        assert_eq!(
            expected_library,
            if vampiric {
                vec![CardIdx(4), CardIdx(2), CardIdx(1), CardIdx(3), CardIdx(5)]
            } else {
                vec![CardIdx(5), CardIdx(3), CardIdx(2), CardIdx(4), CardIdx(1)]
            }
        );
    }
    let expected_words = if vampiric && !has_target {
        0
    } else if has_target {
        6
    } else {
        8
    };
    assert_eq!(expected_rng.get_word_pos(), expected_words);
    if !game.library.is_empty() {
        assert_ne!(
            expected_library, game.library,
            "the seeded case must detect a skipped search"
        );
        assert_ne!(expected_rng.get_word_pos(), game.library_rng.get_word_pos());
    }
    let expected_exile = game.exile.clone();
    TopTutorScenario {
        cards,
        game,
        selected,
        expected_library,
        expected_exile,
        expected_rng,
    }
}

/// Top tutors retain library placement, filters, shuffle scope, and linked life loss.
pub(in crate::deck::simulator) fn assert_top_tutor(source: &CardRow) {
    crate::deck::simulator::card_ast_tests::assert_top_tutor(source);
    let vampiric = source.name == "Vampiric Tutor";
    for source in renamed(source) {
        for has_target in [true, false] {
            let TopTutorScenario {
                cards,
                mut game,
                selected,
                expected_library,
                expected_exile,
                expected_rng,
            } = top_tutor_scenario(&source, vampiric, has_target);
            let mut pool = ManaPool {
                flexible: 1,
                ..Default::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), Some(1));
            assert_eq!(game.library, expected_library);
            if let Some(selected) = selected {
                assert_eq!(game.library.last(), Some(&selected));
            }
            assert_library_rng(&game.library_rng, &expected_rng);
            assert!(game.hand.is_empty());
            assert_eq!(game.exile, expected_exile);
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(6)));
            assert!(!game.battlefield[0].tapped);
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(game.life, if vampiric { 18 } else { 20 });
            assert_eq!(game.life_paid, 0);
            assert_eq!(game.life_gained, 0);
            assert_eq!(game.seen, u32::from(has_target));
            assert_eq!(game.spells_cast_this_turn, 1);
            assert_eq!(game.treasure_bank, 0);
            assert!(!game.failed_draw);
            assert!(game.completion.is_none());
            assert_eq!(pool.total(), 0);
            for index in 1..=5 {
                assert!(!game.card_first_seen.contains_key(&index));
            }
        }
    }
    let mut changed = source.clone();
    changed.oracle_text =
        "Search your library for a card, put it into your hand, then shuffle.".into();
    let cards = deck(&[changed, row("Hand result", "{99}", "Sorcery", "")]);
    let mut game = state(vec![0], vec![1]);
    let mut pool = ManaPool {
        flexible: 1,
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(1));
    assert_eq!(game.hand, [CardIdx(1)]);
    assert!(game.library.is_empty());
    assert_eq!(game.life, 20);
}

/// Trophy Mage's entry rejects both a wrong mana value and a wrong card type.
pub(in crate::deck::simulator) fn assert_trophy_mage(source: &CardRow) {
    crate::deck::simulator::card_ast_tests::assert_trophy_mage(source);
    for source in renamed(source) {
        for (cost, kind, matching) in [
            ("{3}", "Artifact", true),
            ("{2}", "Artifact", false),
            ("{3}", "Creature", false),
        ] {
            let cards = deck(&[source.clone(), row("Candidate", cost, kind, "")]);
            let mut game = state(vec![0], vec![1]);
            let mut pool = ManaPool {
                flexible: 3,
                ..Default::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), Some(3));
            assert_eq!(game.hand, if matching { vec![CardIdx(1)] } else { vec![] });
            assert_eq!(
                game.library,
                if matching { vec![] } else { vec![CardIdx(1)] }
            );
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
            assert!(game.graveyard.is_empty() && game.exile.is_empty());
            assert_eq!(pool.total(), 0);
            assert_eq!(game.life, 20);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = changed.oracle_text.replace("mana value 3", "mana value 2");
    let cards = deck(&[changed, row("Two-value artifact", "{2}", "Artifact", "")]);
    let mut game = state(vec![0], vec![1]);
    let mut pool = ManaPool {
        flexible: 3,
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(3));
    assert_eq!(game.hand, [CardIdx(1)]);
}

/// Eternal Witness returns the actual graveyard card and does nothing when none exists.
pub(in crate::deck::simulator) fn assert_eternal_witness(source: &CardRow) {
    crate::deck::simulator::card_ast_tests::assert_recursion_source(source);
    for source in renamed(source) {
        for has_target in [true, false] {
            let cards = deck(&[source.clone(), row("Return target", "{99}", "Artifact", "")]);
            let mut game = state(vec![0], vec![]);
            if has_target {
                game.graveyard.push(CardIdx(1));
            }
            let mut pool = ManaPool {
                flexible: 3,
                ..Default::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), Some(3));
            assert_eq!(
                game.hand,
                if has_target { vec![CardIdx(1)] } else { vec![] }
            );
            assert!(game.graveyard.is_empty() && game.exile.is_empty() && game.library.is_empty());
            assert_eq!(game.battlefield.len(), 1);
            assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
            assert_eq!(pool.total(), 0);
            assert_eq!(game.life, 20);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = "When this creature enters, draw a card.".into();
    let cards = deck(&[
        changed,
        row("Unreturned", "{99}", "Artifact", ""),
        row("Drawn", "{99}", "Sorcery", ""),
    ]);
    let mut game = state(vec![0], vec![2]);
    game.graveyard.push(CardIdx(1));
    let mut pool = ManaPool {
        flexible: 3,
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(3));
    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!(game.graveyard, [CardIdx(1)]);
}

/// Entry ramp distinguishes a Forest subtype from a basic-land restriction.
pub(in crate::deck::simulator) fn assert_entry_land_search(source: &CardRow) {
    let basic_only = source.name == "Farhaven Elf";
    for source in renamed(source) {
        for matching in [true, false] {
            let target = entry_land_target(basic_only, matching);
            let cards = deck(&[source.clone(), target]);
            let mut game = state(vec![0], vec![1]);
            let mut pool = ManaPool {
                fixed: [0, 0, 0, 0, 1],
                colorless: 2,
                ..Default::default()
            };
            assert_eq!(attempt(&cards, &mut game, &mut pool), Some(3));
            assert_eq!(game.battlefield.len(), 1 + usize::from(matching));
            assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
            if matching {
                assert_eq!(game.battlefield[1].card.deck_idx(), Some(CardIdx(1)));
                assert_eq!(game.battlefield[1].tapped, basic_only);
            }
            assert_eq!(
                game.library,
                if matching { vec![] } else { vec![CardIdx(1)] }
            );
            assert!(game.hand.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
            assert_eq!(game.seen, u32::from(matching));
            assert_eq!(pool.total(), 0);
            assert_eq!(game.life, 20);
        }
    }
    let mut changed = source.clone();
    changed.oracle_text = changed.oracle_text.replace(
        if basic_only {
            "onto the battlefield tapped"
        } else {
            "onto the battlefield"
        },
        "into your hand",
    );
    assert_ne!(changed.oracle_text, source.oracle_text);
    let cards = deck(&[changed, entry_land_target(basic_only, true)]);
    let mut game = state(vec![0], vec![1]);
    let mut pool = ManaPool {
        flexible: 3,
        ..Default::default()
    };
    assert_eq!(attempt(&cards, &mut game, &mut pool), Some(3));
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.battlefield.len(), 1);
    assert!(game.library.is_empty());
}

/// Nonbasic Forests match Wood Elves; Farhaven accepts a basic Swamp instead.
pub(in crate::deck::simulator) fn entry_land_target(basic_only: bool, matching: bool) -> CardRow {
    let kind = match (basic_only, matching) {
        (true, true) => "Basic Land — Swamp",
        (true, false) | (false, true) => "Land — Forest",
        (false, false) => "Land — Swamp",
    };
    row("Entry-search candidate", "", kind, "")
}
