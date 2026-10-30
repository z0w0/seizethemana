use super::*;
use crate::deck::simulator::game::new_perm_with;
use crate::deck::simulator::game_run::mana::{activate_mana_mode, tap_for_nonspell_payment};
use crate::deck::simulator::model::SimActivationCost;
use crate::deck::simulator::oracle_ast::{ActivationCost, CostObject};
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::oracle_parser::parse_oracle_activated_ability;
use crate::deck::simulator::turn_loop_tests::{deck, row, state};
use rand::RngCore;

/// Build a ready creature source with an optional second creature and draw targets.
fn sacrifice_board(text: &str, other: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row("Source", "{1}", "Creature", text),
        row("Other creature", "{1}", "Creature", ""),
        row("First draw", "{9}", "Sorcery", ""),
        row("Second draw", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![], vec![2, 3]);
    st.current_turn = 1;
    st.current_phase = crate::deck::simulator::game::GamePhase::Main;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield[0].summoning_sick = false;
    if other {
        st.battlefield
            .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    }
    st.next_uid = 2;
    (cards, st)
}

/// Rejected payments preserve resources, usage limits, zones, and library randomness.
fn assert_failed_payment_unchanged(st: &GameState, before: &GameState) {
    assert_eq!(st.life, before.life);
    assert_eq!(st.life_paid, before.life_paid);
    assert_eq!(st.life_gained, before.life_gained);
    assert_eq!(st.player_counters.energy, before.player_counters.energy);
    assert_eq!(st.hand, before.hand);
    assert_eq!(st.library, before.library);
    assert_eq!(st.graveyard, before.graveyard);
    assert_eq!(st.exile, before.exile);
    assert_eq!(st.seen, before.seen);
    assert_eq!(st.activated_once, before.activated_once);
    assert_eq!(st.activated_this_turn, before.activated_this_turn);
    assert_eq!(st.battlefield.len(), before.battlefield.len());
    for (permanent, original) in st.battlefield.iter().zip(&before.battlefield) {
        assert_eq!(permanent.uid, original.uid);
        assert_eq!(permanent.counters, original.counters);
        assert_eq!(permanent.tapped, original.tapped);
        assert_eq!(permanent.fired, original.fired);
    }
    let mut rng = st.library_rng.clone();
    let mut original_rng = before.library_rng.clone();
    for _ in 0..4 {
        assert_eq!(rng.next_u64(), original_rng.next_u64());
    }
    assert_eq!(st.completion, before.completion);
}

/// Only the printed word "another" excludes the source from creature payment.
#[test]
fn sacrifice_source_exclusion_matches_the_printed_cost() {
    for (selector, exclude_source) in [
        ("a creature", false),
        ("another creature", true),
        ("a creature you control", false),
        ("another creature you control", true),
    ] {
        let text = format!("Sacrifice {selector}: Add {{C}}.");
        let ast = parse_oracle_activated_ability(&text).expect("sacrifice activation");
        assert!(matches!(
            ast.costs.as_slice(),
            [ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Creature,
                exclude_source: flag,
            }] if *flag == exclude_source
        ));
        let (cards, _) = sacrifice_board(&text, false);
        let costs = cards.cards[0]
            .unlocked_abilities(0)
            .next()
            .expect("fully supported creature cost")
            .activation
            .as_ref()
            .expect("typed activation");
        assert_eq!(costs.creature_sacrifices(), 1);
        assert_eq!(costs.creature_sacrifice_excludes_source(), exclude_source);
    }
}

/// Native, nonspell, and leftover payments use the source only when permitted.
#[test]
fn sole_source_pays_only_an_unrestricted_creature_cost() {
    for selector in ["a creature", "another creature"] {
        for route in ["native", "nonspell", "leftover"] {
            let text = format!("{{1}}, Pay 1 life, Sacrifice {selector}: Add {{C}}{{C}}.");
            let (cards, mut st) = sacrifice_board(&text, false);
            let before = st.clone();
            let source = st.battlefield[0].clone();
            let mut pool = ManaPool {
                colorless: 1,
                ..Default::default()
            };
            let allowed = selector == "a creature";
            match route {
                "native" => assert_eq!(
                    activate_mana_mode(&cards, &mut st, &source, &mut pool, 1),
                    allowed
                ),
                "nonspell" => assert_eq!(
                    tap_for_nonspell_payment(&cards, &mut st, &source, &mut pool, 1),
                    allowed
                ),
                "leftover" => {
                    assert_eq!(
                        pick_best_activation(&cards, &st, &pool, 1).is_some(),
                        allowed
                    );
                    spend_leftover(&cards, &mut st, &mut pool, 1);
                }
                _ => unreachable!("fixed payment route"),
            }
            if allowed {
                assert!(st.battlefield.is_empty());
                assert_eq!(st.graveyard, [CardIdx(0)]);
                assert_eq!(st.life, 19);
                assert_eq!(st.life_paid, 1);
                assert_eq!(pool.total(), 2);
            } else {
                assert_failed_payment_unchanged(&st, &before);
                assert_eq!(pool.total(), 1);
            }
        }
    }
}

/// Spending the only other creature cannot enable a second activation with the source.
#[test]
fn another_creature_payment_consumes_the_other_victim_only_once() {
    for route in ["native", "nonspell", "leftover"] {
        let (cards, mut st) = sacrifice_board(
            "{1}, Pay 1 life, Sacrifice another creature: Add {C}{C}.",
            true,
        );
        let source = st.battlefield[0].clone();
        let mut pool = ManaPool {
            colorless: 1,
            ..Default::default()
        };
        match route {
            "native" => assert!(activate_mana_mode(&cards, &mut st, &source, &mut pool, 1)),
            "nonspell" => assert!(tap_for_nonspell_payment(
                &cards, &mut st, &source, &mut pool, 1
            )),
            "leftover" => spend_leftover(&cards, &mut st, &mut pool, 1),
            _ => unreachable!("fixed payment route"),
        }
        assert_eq!(st.battlefield.len(), 1);
        assert_eq!(st.battlefield[0].uid, source.uid);
        assert_eq!(st.graveyard, [CardIdx(1)]);
        assert_eq!(st.life, 19);
        assert_eq!(st.life_paid, 1);
        assert_eq!(pool.total(), 2);
        assert!(!st.infinite_mana_suspected);
        assert!(st.completion.is_none());

        let before = st.clone();
        let pool_before = pool.clone();
        assert!(!activate_mana_mode(&cards, &mut st, &source, &mut pool, 1));
        assert!(!tap_for_nonspell_payment(
            &cards, &mut st, &source, &mut pool, 1
        ));
        assert!(pick_best_activation(&cards, &st, &pool, 1).is_none());
        spend_leftover(&cards, &mut st, &mut pool, 1);
        assert_failed_payment_unchanged(&st, &before);
        assert_eq!(pool, pool_before);
    }
}

/// Ordinary draw engines stop once the source is the only remaining creature.
#[test]
fn another_creature_draw_activation_stops_after_the_only_victim() {
    let (cards, mut st) =
        sacrifice_board("Pay 1 life, Sacrifice another creature: Draw a card.", true);
    let mut pool = ManaPool::default();
    spend_leftover(&cards, &mut st, &mut pool, 1);
    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].uid, 1);
    assert_eq!(st.hand.len(), 1);
    assert_eq!(st.library.len(), 1);
    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 1);
    assert_eq!(st.life_funded_draws, 1);
    assert!(st.completion.is_none());
    assert!(pick_best_activation(&cards, &st, &pool, 1).is_none());
}

/// A separate source-sacrifice cost cannot also pay the creature selection.
#[test]
fn source_sacrifice_requires_a_distinct_creature_for_both_source_flags() {
    for selector in ["a creature", "another creature"] {
        for other in [false, true] {
            let text = format!("Sacrifice {selector}: Add {{C}}.");
            let (mut cards, st) = sacrifice_board(&text, other);
            cards.cards[0].striations[0].abilities[0]
                .activation
                .as_mut()
                .expect("typed creature sacrifice")
                .costs
                .push(SimActivationCost::SacrificeSource);
            for native in [false, true] {
                let mut st = st.clone();
                let before = st.clone();
                let source = st.battlefield[0].clone();
                let mut pool = ManaPool::default();
                if native {
                    assert_eq!(
                        activate_mana_mode(&cards, &mut st, &source, &mut pool, 1),
                        other
                    );
                } else {
                    assert_eq!(pick_best_activation(&cards, &st, &pool, 1).is_some(), other);
                    spend_leftover(&cards, &mut st, &mut pool, 1);
                }
                if other {
                    assert!(st.battlefield.is_empty());
                    assert_eq!(st.graveyard, [CardIdx(0), CardIdx(1)]);
                    assert_eq!(pool.total(), 1);
                } else {
                    assert_failed_payment_unchanged(&st, &before);
                    assert_eq!(pool.total(), 0);
                }
            }
        }
    }
}

/// Failed native and stale leftover payments cannot consume any bundled resource.
#[test]
fn missing_other_creature_preserves_every_payment_resource_and_rng() {
    for reward in ["Add {C}{C}.", "Draw a card."] {
        let text = format!(
            "{{1}}, {{T}}, Pay 1 life, Pay {{E}}, Remove a charge counter from this creature, Sacrifice another creature: {reward}"
        );
        let (cards, mut st) = sacrifice_board(&text, true);
        st.battlefield[0].counters.charge = 2;
        st.player_counters.energy = 3;
        let source = st.battlefield[0].clone();
        let mut pool = ManaPool {
            colorless: 3,
            ..Default::default()
        };
        let mut activation = pick_best_activation(&cards, &st, &pool, 1)
            .expect("all resources cover the complete payment");
        let costs = activation_data(&activation.ability);
        assert_eq!(costs.mana_cost(), parse_cost("{1}"));
        assert!(costs.taps_source());
        assert_eq!(costs.life_payment(), 1);
        assert_eq!(costs.energy_payment(), 1);
        assert_eq!(costs.charge_counter_payment(), 1);
        assert_eq!(costs.creature_sacrifices(), 1);
        assert!(costs.creature_sacrifice_excludes_source());
        assert_eq!(activation.sacrifice_uid, Some(2));
        for route in ["native", "nonspell", "leftover"] {
            if reward == "Draw a card." && route != "leftover" {
                continue;
            }
            let mut successful = st.clone();
            let mut paid_pool = pool.clone();
            match route {
                "native" => assert!(activate_mana_mode(
                    &cards,
                    &mut successful,
                    &source,
                    &mut paid_pool,
                    1
                )),
                "nonspell" => assert!(tap_for_nonspell_payment(
                    &cards,
                    &mut successful,
                    &source,
                    &mut paid_pool,
                    1
                )),
                "leftover" => spend_leftover(&cards, &mut successful, &mut paid_pool, 1),
                _ => unreachable!("fixed payment route"),
            }
            assert_eq!(successful.battlefield.len(), 1);
            assert_eq!(successful.battlefield[0].uid, source.uid);
            assert!(successful.battlefield[0].tapped);
            assert_eq!(successful.battlefield[0].counters.charge, 1);
            assert_eq!(successful.player_counters.energy, 2);
            assert_eq!(successful.life, 19);
            assert_eq!(successful.life_paid, 1);
            assert_eq!(successful.graveyard, [CardIdx(1)]);
            let draws = usize::from(reward == "Draw a card.");
            assert_eq!(successful.hand.len(), draws);
            assert_eq!(successful.library.len(), 2 - draws);
            assert_eq!(paid_pool.total(), if draws == 1 { 2 } else { 4 });
            assert!(successful.completion.is_none());
        }
        st.battlefield.pop();
        activation.sacrifice_uid = Some(source.uid);
        let before = st.clone();
        let pool_before = pool.clone();

        assert!(!activate_mana_mode(&cards, &mut st, &source, &mut pool, 1));
        assert!(!tap_for_nonspell_payment(
            &cards, &mut st, &source, &mut pool, 1
        ));
        assert!(pick_best_activation(&cards, &st, &pool, 1).is_none());
        resolve_activation(&cards, &mut st, &mut pool, 1, &activation);
        spend_leftover(&cards, &mut st, &mut pool, 1);

        assert_failed_payment_unchanged(&st, &before);
        assert_eq!(pool, pool_before);
    }
}

/// Unknown qualifiers and multiple sacrifice clauses disable the complete ability.
#[test]
fn unsupported_sacrifice_requirements_remain_inert() {
    for cost in [
        "Sacrifice a nontoken creature",
        "Sacrifice another nontoken creature",
        "Sacrifice a tapped creature",
        "Sacrifice a creature with power 2 or less",
        "Sacrifice another creature with power 2 or less",
        "Sacrifice another creature other than a Scout",
        "Sacrifice a creature or an artifact",
        "Sacrifice a creature, Sacrifice a creature",
        "Sacrifice this creature, Sacrifice another creature",
        "Sacrifice a creature and sacrifice another creature",
    ] {
        for reward in ["Add {C}.", "Draw a card."] {
            let text = format!("{{1}}, {{T}}, Pay 1 life, {cost}: {reward}");
            let ast = parse_oracle_activated_ability(&text).expect("preserved activation");
            assert!(
                ast.costs
                    .iter()
                    .any(|cost| matches!(cost, ActivationCost::Other(_))),
                "{text}"
            );
            let (cards, mut st) = sacrifice_board(&text, true);
            assert!(
                !cards.cards[0]
                    .unlocked_abilities(0)
                    .any(|ability| ability.kind.is_activated()),
                "{text}"
            );
            assert!(cards.cards[0].tap.is_none(), "{text}");
            let before = st.clone();
            let source = st.battlefield[0].clone();
            let mut pool = ManaPool {
                colorless: 3,
                ..Default::default()
            };
            let pool_before = pool.clone();
            assert!(!activate_mana_mode(&cards, &mut st, &source, &mut pool, 1));
            assert!(!tap_for_nonspell_payment(
                &cards, &mut st, &source, &mut pool, 1
            ));
            assert!(pick_best_activation(&cards, &st, &pool, 1).is_none());
            spend_leftover(&cards, &mut st, &mut pool, 1);
            assert_failed_payment_unchanged(&st, &before);
            assert_eq!(pool, pool_before);
        }
    }
}
