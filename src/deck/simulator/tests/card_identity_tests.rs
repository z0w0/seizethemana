//! Source-bound native commander-identity payments share the ordinary card action and report path.

use crate::db::CardRow;
use crate::deck::simulator::card_report_tests::{execute_turn, report_executed_turn};
use crate::deck::simulator::card_runtime_tests::{attempt_index, renamed};
use crate::deck::simulator::card_test_support::{assert_library_rng, main_phase_state};
use crate::deck::simulator::game::{GameState, ManaPool, new_perm_with};
use crate::deck::simulator::game_run::mana::activate_mana_mode;
use crate::deck::simulator::model::{CardIdx, Format, SimDeck};
use crate::deck::simulator::oracle_lower::parse_sim_card;
use crate::deck::simulator::scenario_test_support::{deck, row};

/// Keep off-cost identity colors distinct from the commander's colorless printed cost.
fn identity_scenario(
    source: &CardRow,
    identities: &[&str],
    target_cost: &str,
) -> (SimDeck, GameState) {
    let mut cards = deck(&[
        source.clone(),
        row("Identity payment target", target_cost, "Sorcery", ""),
        row("First turn draw", "{99}", "Sorcery", ""),
    ]);
    for (index, identity) in identities.iter().enumerate() {
        let mut commander = row(
            &format!("Stored identity commander {index}"),
            "{99}",
            "Legendary Creature",
            "",
        );
        commander.colors = "[]".into();
        if !identity.is_empty() {
            let symbols = identity
                .chars()
                .map(|color| format!("{{{color}}}"))
                .collect::<String>();
            commander.oracle_text = format!("{{T}}: Add {symbols}.");
        }
        commander.color_identity = serde_json::to_string(
            &identity
                .chars()
                .map(|color| color.to_string())
                .collect::<Vec<_>>(),
        )
        .expect("declared test identity");
        cards.commanders.push(parse_sim_card(&commander));
    }
    cards.format = Format::Commander;
    cards.rules = crate::deck::simulator::format::rules_for("commander");
    let mut game = main_phase_state(vec![1], vec![2]);
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
    game.battlefield[0].summoning_sick = false;
    game.next_uid = 8;
    (cards, game)
}

/// Native identity mana funds only eligible costs and preserves a rejected spell and remaining pool.
fn assert_identity_payment(source: &CardRow, identities: &[&str], cost: &str, legal: bool) {
    let (cards, mut game) = identity_scenario(source, identities, cost);
    let before_rng = game.library_rng.clone();
    let identity_available = identities.iter().any(|identity| !identity.is_empty());
    let permanent = game.battlefield[0].clone();
    let mut pool = ManaPool::default();
    assert_eq!(
        activate_mana_mode(&cards, &mut game, &permanent, &mut pool, 1),
        identity_available
    );
    assert_eq!(pool.total(), u32::from(identity_available));
    assert_eq!(game.battlefield[0].tapped, identity_available);
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
        legal.then_some(1)
    );
    assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(1)] });
    assert_eq!(
        game.graveyard,
        if legal { vec![CardIdx(1)] } else { vec![] }
    );
    assert_eq!(game.library, [CardIdx(2)]);
    assert!(game.exile.is_empty());
    assert_eq!(pool.total(), u32::from(identity_available && !legal));
    assert_eq!(game.life, 20);
    assert_library_rng(&game.library_rng, &before_rng);
}

/// Pinned and renamed sources preserve stored identities, partner union, empty identities, and text changes.
pub(super) fn assert_identity_actions(source: &CardRow) {
    crate::deck::simulator::card_ast_tests::assert_identity_mana_source(source);
    for source in renamed(source) {
        for (cost, legal) in [
            ("{U}", true),
            ("{R}", true),
            ("{1}", true),
            ("{G}", false),
            ("{W}", false),
            ("{B}", false),
            ("{C}", false),
        ] {
            assert_identity_payment(&source, &["UR"], cost, legal);
        }
        for identities in [vec![], vec![""]] {
            for cost in ["{1}", "{U}", "{C}"] {
                assert_identity_payment(&source, &identities, cost, false);
            }
        }
        assert_identity_payment(&source, &["U", "R"], "{R}", true);
        let mut changed = source.clone();
        changed.oracle_text = "{T}: Add {G}.".into();
        let (cards, mut game) = identity_scenario(&changed, &[], "{G}");
        let permanent = game.battlefield[0].clone();
        let mut pool = ManaPool::default();
        assert!(activate_mana_mode(
            &cards, &mut game, &permanent, &mut pool, 1
        ));
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(1)),
            Some(1)
        );
        assert_eq!(game.graveyard, [CardIdx(1)]);
        assert_eq!(pool.total(), 0);
    }
}

/// Full turns report stored-identity capacity and spend without granting excluded colors or empty-identity mana.
pub(super) fn assert_identity_report(source: &CardRow) {
    for (identities, cost, available, legal) in [
        (vec!["UR"], "{U}", 1.0, true),
        (vec!["U", "R"], "{R}", 1.0, true),
        (vec!["UR"], "{G}", 1.0, false),
        (vec![""], "{1}", 0.0, false),
        (vec![], "{C}", 0.0, false),
    ] {
        let (cards, mut game) = identity_scenario(source, &identities, cost);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.graveyard,
            if legal { vec![CardIdx(1)] } else { vec![] }
        );
        assert_eq!(game.battlefield[0].tapped, available > 0.0);
        assert_eq!(game.life, 20);
        let (log, json) = report_executed_turn(&cards, game, census);
        let spent = if legal { 1.0 } else { 0.0 };
        assert_eq!(log.mana_available, [available]);
        assert_eq!(log.mana_spent, [spent]);
        assert_eq!(log.mana_remaining, [available - spent]);
        assert_eq!(json["mana"]["avg_available_by_turn"]["1"], available);
        assert_eq!(json["mana"]["avg_cast_spent_by_turn"]["1"], spent);
        assert_eq!(json["mana"]["avg_unused_by_turn"]["1"], available - spent);
    }
}
