//! Assert narrow real-card behavior from pinned sources, with renamed controls.

use super::game::ManaPool;
use super::game_mana::{add_yield, pips_ok};
use super::model::CardIdx;
use super::oracle_ast::{OracleAbility, OracleEffect};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::{cost::parse_cost, parse_oracle_card};
use super::popular_corpus_tests::pinned_row;
use super::turn_loop_tests::{cast, deck, row, state};

/// Verify Sol Ring's exact production without treating it as colored mana.
#[test]
fn popular_sol_ring_produces_two_colorless_not_colored_mana() {
    for name in ["Sol Ring", "Renamed colorless rock"] {
        let mut source = pinned_row("Sol Ring");
        source.name = name.into();
        let card = parse_sim_card(&source);
        let mut pool = ManaPool::default();
        add_yield(
            card.tap.as_ref().expect("Sol Ring has a tap ability"),
            &mut pool,
        );
        assert_eq!(pool.colorless, 2);
        assert_eq!(pool.fixed, [0; 5]);
        assert_eq!(pool.flexible, 0);
        assert!(pips_ok(&parse_cost("{C}{C}"), &pool));
        assert!(!pips_ok(&parse_cost("{1}{G}"), &pool));
    }
}

/// Keep a native green mana ability distinct from an unrestricted color choice.
#[test]
fn popular_llanowar_elves_produces_one_green_and_no_other_color() {
    for name in ["Llanowar Elves", "Renamed green creature"] {
        let mut source = pinned_row("Llanowar Elves");
        source.name = name.into();
        let card = parse_sim_card(&source);
        let mut pool = ManaPool::default();
        add_yield(
            card.tap.as_ref().expect("Llanowar Elves has a tap ability"),
            &mut pool,
        );
        assert_eq!(pool.fixed, [0, 0, 0, 0, 1]);
        assert_eq!(pool.colorless, 0);
        assert_eq!(pool.flexible, 0);
        assert!(pips_ok(&parse_cost("{G}"), &pool));
        assert!(!pips_ok(&parse_cost("{W}"), &pool));
    }
}

/// Pay for Dark Ritual and resolve exactly three black mana into its owner's pool.
#[test]
fn popular_dark_ritual_pays_black_and_resolves_to_the_graveyard() {
    for name in ["Dark Ritual", "Renamed black ritual"] {
        let mut source = pinned_row("Dark Ritual");
        source.name = name.into();
        let cards = deck(&[source]);
        let mut game = state(vec![0], vec![]);
        let mut pool = ManaPool {
            fixed: [0, 0, 1, 0, 0],
            ..ManaPool::default()
        };
        let spent = cast(&cards, &mut game, &mut pool);
        assert_eq!(spent, [1.0]);
        assert_eq!(pool.fixed, [0, 0, 3, 0, 0]);
        assert_eq!(pool.total(), 3);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert!(game.hand.is_empty());
        assert!(game.battlefield.is_empty());
        assert_eq!(game.life, 20);
        assert_eq!(game.seen, 0);
    }
}

/// Reject a color-near-miss payment without giving Dark Ritual's mana reward.
#[test]
fn popular_dark_ritual_does_not_resolve_from_green_mana() {
    let cards = deck(&[pinned_row("Dark Ritual")]);
    let mut game = state(vec![0], vec![]);
    let mut pool = ManaPool {
        fixed: [0, 0, 0, 0, 1],
        ..ManaPool::default()
    };
    assert_eq!(cast(&cards, &mut game, &mut pool), [0.0]);
    assert_eq!(game.hand, [CardIdx(0)]);
    assert!(game.graveyard.is_empty());
    assert_eq!(pool.fixed, [0, 0, 0, 0, 1]);
    assert_eq!(game.life, 20);
}

/// Draw exactly one physical card on Elvish Visionary's own entry.
#[test]
fn popular_elvish_visionary_draws_once_and_keeps_its_body() {
    for name in ["Elvish Visionary", "Renamed entry creature"] {
        let mut source = pinned_row("Elvish Visionary");
        source.name = name.into();
        let cards = deck(&[source, row("Expensive card", "{99}", "Sorcery", "")]);
        let mut game = state(vec![0], vec![1]);
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 0, 1],
            colorless: 1,
            ..ManaPool::default()
        };
        assert_eq!(cast(&cards, &mut game, &mut pool), [2.0]);
        assert_eq!(pool.total(), 0);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert!(game.library.is_empty());
        assert!(game.graveyard.is_empty());
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(0)));
        assert_eq!(game.seen, 1);
        assert_eq!(game.awareness_cards, 1);
        assert_eq!(game.life, 20);
    }
}

/// Check parser-only land filters without claiming that search execution is complete.
#[test]
fn popular_land_search_parser_keeps_matching_and_nonmatching_types() {
    for (name, expected) in [
        ("Nature's Lore", [false, false, false, false, true]),
        ("Three Visits", [false, false, false, false, true]),
        ("Farseek", [true, true, true, true, false]),
    ] {
        let source = pinned_row(name);
        let card = parse_oracle_card(&source);
        let search = card
            .abilities
            .iter()
            .filter_map(|ability| match ability {
                OracleAbility::Spell(spell) => Some(&spell.effects),
                _ => None,
            })
            .flatten()
            .find_map(|effect| match effect {
                OracleEffect::Search(spec) => Some(spec),
                _ => None,
            })
            .expect("pinned ramp spell parses its search");
        assert_eq!(search.land_types, expected, "{name}");
        assert!(
            !search.basic_land_only,
            "{name} may find nonbasic lands with the named type"
        );
    }
}
