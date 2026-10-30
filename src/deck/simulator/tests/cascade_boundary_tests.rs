use super::*;
use crate::deck::simulator::cast_pass::cast_pass;
use crate::deck::simulator::combos::{ComboCandidate, PieceRef, PieceZone, measure};
use crate::deck::simulator::game::{
    CardRef, GameCompletion, GamePhase, GameState, ManaPool, PlayerLossReason,
};
use crate::deck::simulator::game_run::{TurnCensus, census::finish_log};
use crate::deck::simulator::model::{CardIdx, LifeLossScope, SimDeck, SimEffect};
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// Parse a paid cascade parent with entry rewards that must wait for its child.
fn cascade_deck(child_type: &str, child_text: &str) -> SimDeck {
    let cards = deck(&[
        row(
            "Cascade parent",
            "{3}",
            "Artifact",
            "As an additional cost to cast this spell, pay 1 life.\nCascade\nWhen this artifact enters, gain 7 life.\nAt the beginning of your upkeep, draw a card.",
        ),
        row("Cascade child", "{1}", child_type, child_text),
        row("Library marker", "{99}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].has_cascade);
    assert_eq!(cards.cards[0].spell_data.additional_cost_life, 1);
    cards
}

/// Cast through the ordinary hand pass and keep committed payment telemetry.
fn cast_parent(cards: &SimDeck, st: &mut GameState) -> (TurnCensus, Vec<(u32, u32)>) {
    st.current_turn = 1;
    st.current_phase = GamePhase::Main;
    let mut census = TurnCensus::new(3, cards.cards.len());
    let mut sources = Vec::new();
    let mut pool = ManaPool {
        flexible: 4,
        ..ManaPool::default()
    };
    cast_pass(
        cards,
        st,
        &mut pool,
        1,
        &mut census.mana_spent,
        &mut sources,
        &mut census.pip_blocks,
        &mut census.blocked_colors,
    );
    assert_eq!(census.mana_spent, [3.0, 0.0, 0.0]);
    assert_eq!(pool.total(), 1);
    assert_eq!(st.life_paid, 1);
    assert_eq!(st.spells_cast_this_turn, 2);
    assert!(!st.hand.contains(&CardIdx(0)));
    assert!(!st.graveyard.contains(&CardIdx(0)));
    assert!(!st.exile.contains(&CardIdx(0)));
    (census, sources)
}

/// A stopped parent cannot create a battlefield visit or register an upkeep engine.
fn assert_parent_stopped(st: &GameState, sources: &[(u32, u32)]) {
    assert!(
        st.battlefield
            .iter()
            .all(|permanent| permanent.card != CardRef::Deck(CardIdx(0)))
    );
    assert!(!st.battlefield_seen.contains_key(&CardIdx(0)));
    assert!(sources.is_empty());
}

/// Use the production log and combo reducer to check the parent's first battlefield visit.
fn assert_parent_access(cards: &SimDeck, st: GameState, census: TurnCensus, entered: bool) {
    let incomplete = matches!(st.completion, Some(GameCompletion::Incomplete { .. }));
    let log = finish_log(cards, st, census, 0, false, 3);
    assert_eq!(log.card_first_battlefield.get(&0), entered.then_some(&1));
    assert_eq!(log.life_paid, 1);
    assert_eq!(log.mana_spent, [3.0, 0.0, 0.0]);
    let candidate = ComboCandidate {
        id: "cascade-parent-access".into(),
        produces: Vec::new(),
        mana_value_needed: 3,
        bracket_tag: None,
        complete: true,
        missing: Vec::new(),
        pieces: vec![PieceRef {
            name: cards.cards[0].name.clone(),
            indices: vec![0],
            starts_in_command_zone: false,
            starts_in_library: false,
            zones: vec![PieceZone::Battlefield],
        }],
    };
    let assembly = measure(&[candidate], &[log], cards, 3);
    let access = &assembly.complete[0].access;
    assert_eq!(access.observed_games, u32::from(!incomplete));
    assert_eq!(
        access.event_games,
        (!incomplete).then_some(u32::from(entered))
    );
}

/// A child's failed entry draw ends the game before the parent can enter.
#[test]
fn cascade_failed_draw_stops_parent_entry_and_combo_access() {
    let cards = cascade_deck("Artifact", "When this artifact enters, draw a card.");
    let mut st = state(vec![0], vec![1]);

    let (census, sources) = cast_parent(&cards, &mut st);

    assert_eq!(
        st.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::FailedDraw],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert!(st.failed_draw);
    assert_eq!(st.life, 19);
    assert_eq!(st.life_gained, 0);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(st.battlefield_seen.get(&CardIdx(1)), Some(&1));
    assert_eq!(st.next_uid, 1);
    assert_parent_stopped(&st, &sources);
    assert_parent_access(&cards, st, census, false);
}

/// A lethal child spell keeps its graveyard visit without resolving the parent.
#[test]
fn cascade_lethal_child_stops_parent_entry_and_entry_triggers() {
    let cards = cascade_deck("Sorcery", "You lose 1 life.");
    assert!(matches!(
        cards.cards[1].spell_data.effects.as_slice(),
        [SimEffect::LoseLife {
            amount: 1,
            scope: LifeLossScope::SelfOnly,
        }]
    ));
    let mut st = state(vec![0], vec![1]);
    st.life = 2;

    let (census, sources) = cast_parent(&cards, &mut st);

    assert_eq!(
        st.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::ZeroLife],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert_eq!(st.life, 0);
    assert_eq!(st.life_gained, 0);
    assert!(!st.failed_draw);
    assert!(st.battlefield.is_empty());
    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert_eq!(st.graveyard_seen.get(&CardIdx(1)), Some(&1));
    assert_eq!(st.next_uid, 0);
    assert_parent_stopped(&st, &sources);
    assert_parent_access(&cards, st, census, false);
}

/// A parsed recursive child trigger cannot let an incomplete game resolve its parent.
#[test]
fn cascade_incomplete_child_stops_parent_entry() {
    let cards = cascade_deck(
        "Artifact",
        "When this artifact enters, the Ring tempts you.\nWhenever the Ring tempts you, the Ring tempts you.",
    );
    let mut st = state(vec![0], vec![1]);

    let (census, sources) = cast_parent(&cards, &mut st);

    assert_eq!(
        st.completion,
        Some(GameCompletion::Incomplete {
            reason: game::IncompleteReason::TriggerSettlement {
                configured_limit: 1024,
                remaining_work: 1
            },
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert!(st.resolution.trigger_limit_reached);
    assert_eq!(st.ring_tempts, 1024);
    assert_eq!(st.life, 19);
    assert_eq!(st.life_gained, 0);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(st.next_uid, 1);
    assert_parent_stopped(&st, &sources);
    assert_parent_access(&cards, st, census, false);
}

/// A successful child draw still permits the parent entry and upkeep registration.
#[test]
fn cascade_nonterminal_child_preserves_parent_entry() {
    let cards = cascade_deck("Artifact", "When this artifact enters, draw a card.");
    let mut st = state(vec![0], vec![2, 1]);

    let (census, sources) = cast_parent(&cards, &mut st);

    assert!(st.completion.is_none());
    assert!(!st.failed_draw);
    assert_eq!(st.life, 26);
    assert_eq!(st.life_gained, 7);
    assert_eq!(st.hand, [CardIdx(2)]);
    assert!(st.library.is_empty());
    assert_eq!(st.battlefield.len(), 2);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(st.battlefield[1].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(sources, [(st.battlefield[1].uid, 1)]);
    assert_parent_access(&cards, st, census, true);
}

/// Linked child effects may restore life before its single loss boundary.
#[test]
fn cascade_child_finishes_linked_life_effects_before_parent_resolution() {
    let cards = cascade_deck("Sorcery", "You lose 1 life. Gain three life.");
    assert!(matches!(
        cards.cards[1].spell_data.effects.as_slice(),
        [
            SimEffect::LoseLife {
                amount: 1,
                scope: LifeLossScope::SelfOnly
            },
            SimEffect::GainLife(3),
        ]
    ));
    let mut st = state(vec![0], vec![1]);
    st.life = 2;

    let (census, sources) = cast_parent(&cards, &mut st);

    assert!(st.completion.is_none());
    assert_eq!(st.life, 10);
    assert_eq!(st.life_gained, 10);
    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(sources, [(st.battlefield[0].uid, 1)]);
    assert_parent_access(&cards, st, census, true);
}

/// A failed draw does not interrupt linked child rewards, but does stop the parent.
#[test]
fn cascade_failed_draw_finishes_linked_child_effects_only() {
    let cards = cascade_deck("Sorcery", "Draw a card. Gain three life.");
    assert!(matches!(
        cards.cards[1].spell_data.effects.as_slice(),
        [SimEffect::Draw(1), SimEffect::GainLife(3)]
    ));
    let mut st = state(vec![0], vec![1]);

    let (census, sources) = cast_parent(&cards, &mut st);

    assert_eq!(
        st.completion,
        Some(GameCompletion::PlayerLost {
            reasons: vec![PlayerLossReason::FailedDraw],
            turn: 1,
            phase: GamePhase::Main,
        })
    );
    assert_eq!(st.life, 22);
    assert_eq!(st.life_gained, 3);
    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert!(st.battlefield.is_empty());
    assert_parent_stopped(&st, &sources);
    assert_parent_access(&cards, st, census, false);
}
