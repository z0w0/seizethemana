use super::*;
use crate::deck::simulator::game::{new_perm_with, new_token_perm};
use crate::deck::simulator::model::{
    SimAbilityKind, SimActivationCost, SimStriation, SpendRestriction,
};
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::turn_loop_tests::{deck, row, state};

/// Unrestricted creature sacrifices still allow the source itself or another creature.
#[test]
fn unrestricted_sacrifices_keep_legal_native_and_leftover_rewards() {
    for other in [false, true] {
        let cards = deck(&[
            row(
                "Source",
                "{1}",
                if other { "Artifact" } else { "Creature" },
                "Sacrifice a creature: Add {C}.",
            ),
            row("Other creature", "{1}", "Creature", ""),
        ]);
        let mut st = state(vec![], vec![]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
        if other {
            st.battlefield
                .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
        }
        let mut leftover = st.clone();
        let source = st.battlefield[0].clone();
        let mut pool = ManaPool::default();
        assert!(crate::deck::simulator::game_run::mana::activate_mana_mode(
            &cards, &mut st, &source, &mut pool, 1
        ));
        assert_eq!(pool.total(), 1);
        assert_eq!(st.graveyard, [CardIdx(u32::from(other))]);
        let mut leftover_pool = ManaPool::default();
        spend_leftover(&cards, &mut leftover, &mut leftover_pool, 1);
        assert_eq!(leftover_pool.total(), 1);
        assert_eq!(leftover.graveyard.len(), 1);
        assert_eq!(leftover.battlefield.len(), usize::from(other));
        assert!(leftover.completion.is_none());
    }
}

/// Rejecting one incomplete payment does not disable a sibling fully supported mana mode.
#[test]
fn another_creature_cost_does_not_block_supported_sibling_modes() {
    let cards = deck(&[row(
        "Source",
        "{1}",
        "Creature",
        "{T}, Sacrifice another creature: Add {C}.\n{T}: Add {G}.",
    )]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield[0].summoning_sick = false;
    let source = st.battlefield[0].clone();
    let mut pool = ManaPool::default();
    assert!(crate::deck::simulator::game_run::mana::activate_mana_mode(
        &cards, &mut st, &source, &mut pool, 1
    ));
    assert_eq!(pool.fixed[4], 1);
    assert_eq!(pool.colorless, 0);
    assert_eq!(st.battlefield.len(), 1);
    assert!(st.battlefield[0].tapped);
    assert!(st.graveyard.is_empty());
}

/// Paying the last life for an ordinary ability loses before its reward resolves.
#[test]
fn last_life_payment_cannot_be_rescued_by_the_ordinary_reward() {
    let cards = deck(&[row(
        "Life engine",
        "{1}",
        "Artifact",
        "Pay 1 life: Gain 3 life.",
    )]);
    let mut st = state(vec![], vec![]);
    st.life = 1;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    let mut pool = ManaPool::default();
    let activation =
        pick_best_activation(&cards, &st, &pool, 1).expect("exact life can pay the cost");
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);
    assert_eq!(st.life, 0);
    assert_eq!(st.life_paid, 1);
    assert_eq!(st.life_gained, 0);
    assert!(matches!(
        st.completion,
        Some(crate::deck::simulator::game::GameCompletion::PlayerLost { .. })
    ));
}

/// Cost-triggered life gain cannot rescue a loss already reached after paying costs.
#[test]
fn last_life_payment_stops_before_queued_death_rewards() {
    let cards = deck(&[
        row(
            "Cost source",
            "{1}",
            "Artifact",
            "Pay 1 life, Sacrifice a creature: Gain 3 life.",
        ),
        row(
            "Death listener",
            "{1}",
            "Artifact",
            "Whenever a creature you control dies, gain 3 life.",
        ),
        row("Victim", "{1}", "Creature", ""),
    ]);
    let mut st = state(vec![], vec![]);
    st.life = 1;
    for index in 0..3 {
        st.battlefield
            .push(new_perm_with(index + 1, &cards, CardIdx(index), 0, false));
    }
    let mut pool = ManaPool::default();
    let activation =
        pick_best_activation(&cards, &st, &pool, 1).expect("complete life and sacrifice cost");
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);
    assert_eq!(st.life, 0);
    assert_eq!(st.life_gained, 0);
    assert_eq!(st.graveyard, [CardIdx(2)]);
    assert_eq!(st.battlefield.len(), 2);
    assert!(matches!(
        st.completion,
        Some(crate::deck::simulator::game::GameCompletion::PlayerLost { .. })
    ));
}

/// Mana abilities finish their immediate resolution before the next loss check.
#[test]
fn mana_ability_finishes_linked_rewards_before_the_loss_boundary() {
    let mut cards = deck(&[row(
        "Life mana engine",
        "{1}",
        "Artifact",
        "Pay 1 life: Add {C}.",
    )]);
    let ability = &mut cards.cards[0].striations[0].abilities[0];
    ability.effects = vec![ability.effect.clone(), SimEffect::GainLife(3)];
    assert_eq!(ability.kind, SimAbilityKind::ManaActivated);
    let mut st = state(vec![], vec![]);
    st.life = 1;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &st, &pool, 1).expect("exact life mana payment");
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);
    assert_eq!(st.life, 3);
    assert_eq!(st.life_paid, 1);
    assert_eq!(st.life_gained, 3);
    assert_eq!(pool.total(), 1);
    assert!(st.completion.is_none());
}

/// A finite activation sequence is complete at the cap and incomplete above it.
#[test]
fn activation_cap_requires_a_real_remaining_activation() {
    let limit = crate::deck::simulator::model::MAX_LOOP_PASSES;
    for remaining in [0, 1] {
        let mut st = state(vec![], vec![]);
        // Keep one life after the final available activation by using a finite counter cost.
        let mut finite = deck(&[row(
            "Finite life engine",
            "{1}",
            "Artifact",
            "Pay 1 life: Create a Treasure token.",
        )]);
        finite.cards[0].striations[0].abilities[0]
            .activation
            .as_mut()
            .expect("typed activation")
            .costs = vec![SimActivationCost::RemoveChargeCounters(1)];
        let mut source = new_perm_with(1, &finite, CardIdx(0), 0, false);
        source.counters.charge = limit + remaining;
        st.battlefield.push(source);
        spend_leftover(&finite, &mut st, &mut ManaPool::default(), 1);
        assert_eq!(st.treasure_bank, limit);
        assert_eq!(st.battlefield[0].counters.charge, remaining);
        assert!(!st.infinite_mana_suspected);
        assert_eq!(st.completion.is_some(), remaining > 0);
        if remaining > 0 {
            assert!(matches!(
                st.completion,
                Some(crate::deck::simulator::game::GameCompletion::Incomplete {
                    reason: crate::deck::simulator::game::IncompleteReason::ActivationPass {
                        remaining_work: 1,
                        ..
                    },
                    ..
                })
            ));
        }
    }
}

/// Summoning sickness applies to an untap symbol as well as a tap symbol.
#[test]
fn sick_creature_cannot_pay_an_untap_symbol() {
    let cards = deck(&[row("Untap creature", "{G}", "Creature", "{Q}: Add {G}.")]);
    let mut st = state(vec![], vec![]);
    let mut source = new_perm_with(1, &cards, CardIdx(0), 1, true);
    source.summoning_sick = true;
    st.battlefield.push(source);
    let mut pool = ManaPool::default();

    assert!(pick_best_activation(&cards, &st, &pool, 1).is_none());
    st.battlefield[0].summoning_sick = false;
    let activation = pick_best_activation(&cards, &st, &pool, 2)
        .expect("an older tapped creature can pay its untap symbol");
    resolve_activation(&cards, &mut st, &mut pool, 2, &activation);

    assert!(!st.battlefield[0].tapped);
    assert_eq!(pool.fixed[4], 1);
}

/// Every selected creature is consumed by a multi-creature sacrifice cost.
#[test]
fn activation_consumes_every_required_creature() {
    let mut cards = deck(&[
        row("Sacrifice source", "{1}", "Artifact", ""),
        row("First creature", "{1}", "Creature", ""),
        row("Second creature", "{1}", "Creature", ""),
    ]);
    cards.cards[0].striations = vec![SimStriation {
        abilities: vec![SimAbility {
            kind: SimAbilityKind::Activated,
            trigger: SimTrigger::Never,
            activation: Some(SimActivation {
                costs: vec![SimActivationCost::SacrificeCreatures {
                    count: 2,
                    exclude_source: false,
                }],
                restrictions: Vec::new(),
            }),
            effect: SimEffect::GainLife(3),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let mut st = state(vec![], vec![]);
    for index in 0..3 {
        st.battlefield.push(new_perm_with(
            index + 1,
            &cards,
            CardIdx(index),
            0,
            index == 1,
        ));
    }
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("two creature permanents cover the complete sacrifice cost");

    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.graveyard, [CardIdx(1), CardIdx(2)]);
    assert_eq!(st.life, 23);
}

/// A nontoken death filter ignores a token but sees a creature card die.
#[test]
fn death_filter_distinguishes_tokens_from_cards() {
    let mut cards = deck(&[
        row(
            "Death draw",
            "{1}",
            "Enchantment",
            "Whenever another creature you control dies, draw a card.",
        ),
        row("Creature", "{1}", "Creature", ""),
        row("Drawn", "{9}", "Sorcery", ""),
    ]);
    cards.cards[0].striations[0].abilities[0]
        .event_filter
        .nontoken = true;
    let mut st = state(vec![], vec![2]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    st.battlefield.push(new_token_perm(3, 0));
    st.next_uid = 3;

    resolve_sacrifice_uid(&cards, &mut st, 1, 3);
    assert!(st.hand.is_empty());
    resolve_sacrifice_uid(&cards, &mut st, 1, 2);

    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert_eq!(st.seen, 1);
}

/// Build a battlefield source from a parsed activation cost and effect sequence.
fn counter_source(cost: &str, effects: &str) -> (SimDeck, GameState) {
    let cards = deck(&[row(
        "Counter source",
        "{1}",
        "Artifact",
        &format!("{cost}: {effects}"),
    )]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.next_uid = 1;
    (cards, st)
}

/// Every X symbol uses one chosen value, including zero, before mana is spent.
#[test]
fn x_counter_activations_pay_one_chosen_value_for_every_x_symbol() {
    for (cost, available, expected_x, remaining) in [
        ("{X}", 0, 0, 0),
        ("{X}", 1, 1, 0),
        ("{X}", 6, 6, 0),
        ("{X}{X}", 0, 0, 0),
        ("{X}{X}", 1, 0, 1),
        ("{X}{X}", 5, 2, 1),
        ("{2}{X}{X}", 7, 2, 1),
    ] {
        let (cards, mut st) = counter_source(cost, "Put X charge counters on this artifact.");
        let mut pool = ManaPool {
            colorless: available,
            ..Default::default()
        };
        let before = pool.clone();

        let activation = pick_best_activation(&cards, &st, &pool, 1)
            .expect("the fixed cost is payable, including a legal X of zero");

        assert_eq!(pool, before, "selection must not spend mana: {cost}");
        assert_eq!(activation.paid_x, expected_x, "{cost}");
        assert_eq!(
            activation.ability_cost,
            parse_cost(cost).with_x(expected_x).expect("bounded X cost"),
            "{cost}"
        );
        resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

        assert_eq!(st.battlefield[0].counters.charge, expected_x, "{cost}");
        assert_eq!(pool.colorless, remaining, "{cost}");
        assert_eq!(pool.total(), remaining, "{cost}");
    }
}

/// Spell-only mana cannot increase activation X or disappear during resolution.
#[test]
fn x_counter_activation_preserves_all_cast_restricted_mana() {
    for available in [0, 3] {
        let (cards, mut st) = counter_source("{X}", "Put X charge counters on this artifact.");
        let mut pool = ManaPool {
            colorless: available,
            lots: [
                SpendRestriction::Creature,
                SpendRestriction::Artifact,
                SpendRestriction::Legendary,
                SpendRestriction::InstantSorcery,
            ]
            .into_iter()
            .map(|restriction| crate::deck::simulator::game_mana::ManaLot {
                amount: 7,
                types: 32,
                restriction: Some(restriction),
                cannot_pay_generic: false,
                one_color: false,
            })
            .collect(),
            ..Default::default()
        };
        let mut expected = pool.clone();
        expected.colorless = 0;

        let activation = pick_best_activation(&cards, &st, &pool, 1)
            .expect("unrestricted mana covers the selected X, including zero");
        assert_eq!(activation.paid_x, available);
        resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

        assert_eq!(st.battlefield[0].counters.charge, available);
        assert_eq!(pool, expected);
    }
}

/// Generic credit and colored mana cannot replace an explicit colorless pip.
#[test]
fn x_counter_activation_reserves_explicit_colorless_mana() {
    for mut pool in [
        ManaPool {
            generic_credit: 4,
            ..Default::default()
        },
        ManaPool {
            flexible: 4,
            ..Default::default()
        },
    ] {
        let (cards, mut st) = counter_source("{X}{C}", "Put X charge counters on this artifact.");
        let before = pool.clone();
        assert!(pick_best_activation(&cards, &st, &pool, 1).is_none());
        assert_eq!(pool, before);

        pool.colorless = 1;
        let activation = pick_best_activation(&cards, &st, &pool, 1)
            .expect("one colorless mana pays the fixed pip before four generic mana pays X");
        assert_eq!(activation.paid_x, 4);
        assert_eq!(activation.ability_cost.generic, 4);
        assert_eq!(activation.ability_cost.colorless, 1);
        resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

        assert_eq!(st.battlefield[0].counters.charge, 4);
        assert_eq!(pool.total(), 0);
    }
}

/// Pip-only mana pays a matching fixed pip but cannot increase generic X.
#[test]
fn x_counter_activation_preserves_unused_pip_only_mana() {
    let (cards, mut st) = counter_source("{U}{X}", "Put X charge counters on this artifact.");
    let mut pool = ManaPool {
        colorless: 2,
        fixed: [0, 4, 0, 0, 0],
        fixed_no_generic: [0, 4, 0, 0, 0],
        ..Default::default()
    };
    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("the blue pip and two generic mana are separately payable");
    assert_eq!(activation.paid_x, 2);
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(st.battlefield[0].counters.charge, 2);
    assert_eq!(
        pool,
        ManaPool {
            fixed: [0, 3, 0, 0, 0],
            fixed_no_generic: [0, 3, 0, 0, 0],
            ..Default::default()
        }
    );
}

/// Mana produced during resolution does not change the X already paid.
#[test]
fn x_counter_amount_does_not_include_resolution_mana() {
    let (cards, mut st) = counter_source(
        "{X}",
        "Add {C}{C}, then put X charge counters on this artifact.",
    );
    let mut pool = ManaPool {
        colorless: 3,
        ..Default::default()
    };
    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("the ordered mana and charge-counter effects lower");
    assert!(matches!(
        activation.ability.effect_sequence(),
        [SimEffect::Mana(_), SimEffect::Counters(0)]
    ));
    assert_eq!(activation.paid_x, 3);
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(st.battlefield[0].counters.charge, 3);
    assert_eq!(pool.colorless, 2);
}

/// Fixed charge-counter amounts do not depend on available or remaining mana.
#[test]
fn fixed_counter_activation_does_not_scale_with_remaining_mana() {
    for (cost, expected_x, remaining) in [("{1}, {T}", 0, 4), ("{X}, {T}", 5, 0)] {
        let (cards, mut st) = counter_source(cost, "Put one charge counter on this artifact.");
        let mut pool = ManaPool {
            colorless: 5,
            ..Default::default()
        };
        let activation = pick_best_activation(&cards, &st, &pool, 1)
            .expect("the artifact can pay its mana and tap costs");
        assert_eq!(activation.paid_x, expected_x);
        resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

        assert_eq!(st.battlefield[0].counters.charge, 1);
        assert!(st.battlefield[0].tapped);
        assert_eq!(pool.colorless, remaining);
    }
}

/// Unsupported counter kinds must not become charge counters or consume mana.
#[test]
fn unknown_x_counter_kinds_stay_inert() {
    for counter in ["tower", "age", "+1/+1"] {
        let (cards, st) = counter_source(
            "{X}",
            &format!("Put X {counter} counters on this artifact."),
        );
        let pool = ManaPool {
            colorless: 5,
            ..Default::default()
        };

        assert!(
            cards.cards[0].unlocked_abilities(0).next().is_none(),
            "{counter}"
        );
        assert!(
            pick_best_activation(&cards, &st, &pool, 1).is_none(),
            "{counter}"
        );
        assert_eq!(st.battlefield[0].counters.charge, 0);
        assert_eq!(pool.colorless, 5);
    }
}

/// Large pools use a bounded search and reject overflowing candidate costs.
#[test]
fn repeated_x_activation_handles_the_largest_pool_capacity() {
    let (cards, mut st) = counter_source("{9}{X}{X}", "Put X charge counters on this artifact.");
    let mut pool = ManaPool {
        colorless: u32::MAX,
        ..Default::default()
    };
    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("the largest bounded X fits the complete activation cost");

    assert_eq!(activation.paid_x, (u32::MAX - 9) / 2);
    assert_eq!(activation.ability_cost.generic, u32::MAX);
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(st.battlefield[0].counters.charge, (u32::MAX - 9) / 2);
    assert_eq!(pool.total(), 0);
}

/// Build the visible artifact count used by Metalcraft activation checks.
fn artifact_board(text: &str, artifact_count: u32) -> (SimDeck, GameState) {
    let cards = deck(&[
        row("Conditional source", "{0}", "Artifact", text),
        row("Second artifact", "{0}", "Artifact", ""),
        row("Third artifact", "{0}", "Artifact", ""),
    ]);
    let mut st = state(vec![], vec![]);
    for index in 0..artifact_count {
        st.battlefield
            .push(new_perm_with(index + 1, &cards, CardIdx(index), 0, false));
    }
    st.next_uid = artifact_count;
    (cards, st)
}

/// The activation pass must enforce the same Metalcraft gate as normal mana taps.
#[test]
fn metalcraft_activation_requires_three_artifacts() {
    for artifact_count in [1, 2, 3] {
        let (cards, mut st) = artifact_board(
            "Metalcraft — {T}: Add one mana of any color. Activate only if you control three or more artifacts.",
            artifact_count,
        );
        assert!(cards.cards[0].unlocked_abilities(0).any(|ability| {
            ability.condition
                == Some(crate::deck::simulator::model::SimAbilityCondition::Metalcraft)
        }));
        let mut pool = ManaPool::default();
        assert_eq!(
            pick_best_activation(&cards, &st, &pool, 1).is_some(),
            artifact_count == 3
        );

        spend_leftover(&cards, &mut st, &mut pool, 1);

        assert_eq!(pool.flexible, u32::from(artifact_count == 3));
        assert_eq!(pool.total(), u32::from(artifact_count == 3));
        assert_eq!(st.battlefield[0].tapped, artifact_count == 3);
    }
}

/// A condition lost after selection must block activation before any cost is paid.
#[test]
fn activation_rechecks_metalcraft_before_committing_costs() {
    let (cards, mut st) = artifact_board(
        "Metalcraft — {T}: Add one mana of any color. Activate only if you control three or more artifacts.",
        3,
    );
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("three artifacts satisfy the activation condition");
    st.battlefield.pop();

    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(pool.total(), 0);
    assert!(!st.battlefield[0].tapped);
    assert!(st.activated_once.is_empty());
    assert!(st.activated_this_turn.is_empty());
}

/// A legal activation may consume the permanent that satisfied its condition.
#[test]
fn activation_condition_is_not_rechecked_after_paying_its_cost() {
    let (cards, mut st) = artifact_board(
        "Metalcraft — Sacrifice this artifact: Add {G}. Activate only if you control three or more artifacts.",
        3,
    );
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("three artifacts satisfy the condition before paying the sacrifice cost");

    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(st.battlefield.len(), 2);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(pool.fixed[4], 1);
    assert_eq!(pool.total(), 1);
}

/// The activation uses its own creature to pay a sacrifice cost when allowed.
#[test]
fn creature_sacrifice_activation_can_sacrifice_its_source_once() {
    let cards = deck(&[
        row(
            "Sacrifice source",
            "{1}",
            "Creature",
            "Sacrifice a creature: Add {G}.",
        ),
        row(
            "Death listener",
            "{1}",
            "Enchantment",
            "Whenever a creature you control dies, gain 1 life.",
        ),
    ]);
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    st.next_uid = 2;
    let source = st.battlefield[0].clone();
    let mut pool = ManaPool::default();

    assert!(crate::deck::simulator::game_run::mana::activate_mana_mode(
        &cards, &mut st, &source, &mut pool, 2,
    ));

    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.life, 21);
    assert_eq!(pool.fixed[4], 1);
    assert_eq!(pool.total(), 1);

    let before = pool.clone();
    assert!(!crate::deck::simulator::game_run::mana::activate_mana_mode(
        &cards,
        &mut st,
        &new_perm_with(1, &cards, CardIdx(0), 0, false),
        &mut pool,
        2,
    ));
    assert_eq!(pool, before, "the removed source cannot activate twice");
}

/// A source sacrifice and creature sacrifice use two distinct objects.
#[test]
fn mana_activation_pays_source_and_creature_sacrifices_once_each() {
    let mut cards = deck(&[
        row("Sacrifice source", "{1}", "Artifact Creature", ""),
        row("Other creature", "{1}", "Creature", ""),
    ]);
    cards.cards[0].striations = vec![SimStriation {
        abilities: vec![crate::deck::simulator::model::SimAbility {
            kind: SimAbilityKind::ManaActivated,
            activation: Some(crate::deck::simulator::model::SimActivation {
                costs: vec![
                    SimActivationCost::SacrificeSource,
                    SimActivationCost::SacrificeCreatures {
                        count: 1,
                        exclude_source: false,
                    },
                ],
                restrictions: Vec::new(),
            }),
            effect: SimEffect::Mana(crate::deck::simulator::model::ManaYield {
                fixed: [0, 0, 0, 0, 1],
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    let source = st.battlefield[0].clone();
    let mut pool = ManaPool::default();

    assert!(crate::deck::simulator::game_run::mana::activate_mana_mode(
        &cards, &mut st, &source, &mut pool, 2,
    ));

    assert!(st.battlefield.is_empty());
    assert_eq!(st.graveyard.len(), 2);
    assert_eq!(pool.fixed[4], 1);
    assert_eq!(pool.total(), 1);
}

/// A missing sacrifice target keeps every other payment and reward unchanged.
#[test]
fn native_mana_activation_rejects_incomplete_sacrifice_payment_atomically() {
    let mut cards = deck(&[row("Sacrifice source", "{1}", "Artifact Creature", "")]);
    cards.cards[0].striations = vec![SimStriation {
        abilities: vec![crate::deck::simulator::model::SimAbility {
            kind: SimAbilityKind::ManaActivated,
            activation: Some(crate::deck::simulator::model::SimActivation {
                costs: vec![
                    SimActivationCost::Mana(parse_cost("{1}")),
                    SimActivationCost::SacrificeCreatures {
                        count: 2,
                        exclude_source: false,
                    },
                ],
                restrictions: Vec::new(),
            }),
            effect: SimEffect::Mana(crate::deck::simulator::model::ManaYield {
                fixed: [0, 0, 0, 0, 1],
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let mut st = state(vec![], vec![]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    let source = st.battlefield[0].clone();
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };

    assert!(!crate::deck::simulator::game_run::mana::activate_mana_mode(
        &cards, &mut st, &source, &mut pool, 2,
    ));

    assert_eq!(pool.colorless, 1);
    assert_eq!(pool.total(), 1);
    assert_eq!(st.battlefield.len(), 1);
    assert!(!st.battlefield[0].tapped);
    assert!(st.graveyard.is_empty());
    assert_eq!(st.life, 20);
}

/// A stale activation cannot pay a missing sacrifice or apply its effect.
#[test]
fn failed_activation_payment_leaves_state_and_pool_unchanged() {
    let cards = deck(&[
        row(
            "Sacrifice source",
            "{1}",
            "Artifact",
            "Sacrifice a creature: Draw a card.",
        ),
        row("Sacrifice target", "{1}", "Creature", ""),
        row("Drawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![], vec![2]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    st.next_uid = 2;
    let mut pool = ManaPool::default();
    let activation =
        pick_best_activation(&cards, &st, &pool, 2).expect("the source can sacrifice the creature");
    st.battlefield.pop();
    let life_before = st.life;
    let hand_before = st.hand.clone();
    let pool_before = pool.clone();

    resolve_activation(&cards, &mut st, &mut pool, 2, &activation);

    assert_eq!(pool, pool_before);
    assert_eq!(st.life, life_before);
    assert_eq!(st.hand, hand_before);
    assert!(!st.battlefield[0].tapped);
}

/// A face-down card loses its printed subtype but remains a nontoken creature.
#[test]
fn death_filters_use_face_down_victims_last_known_characteristics() {
    for face_down in [false, true] {
        let cards = deck(&[
            row(
                "Goblin listener",
                "{1}",
                "Enchantment",
                "Whenever a Goblin you control dies, draw a card.",
            ),
            row(
                "Creature listener",
                "{1}",
                "Enchantment",
                "Whenever a nontoken creature you control dies, draw a card.",
            ),
            row("Goblin", "{1}", "Creature — Goblin", ""),
            row("First draw", "{9}", "Sorcery", ""),
            row("Second draw", "{9}", "Sorcery", ""),
        ]);
        let mut st = state(vec![], vec![3, 4]);
        for index in 0..3 {
            st.battlefield
                .push(new_perm_with(index + 1, &cards, CardIdx(index), 0, false));
        }
        st.battlefield[2].face_down = face_down;
        st.next_uid = 3;

        resolve_sacrifice_uid(&cards, &mut st, 1, 3);

        let expected = if face_down {
            vec![CardIdx(4)]
        } else {
            vec![CardIdx(4), CardIdx(3)]
        };
        assert_eq!(st.hand, expected, "face_down={face_down}");
        assert_eq!(st.seen, if face_down { 1 } else { 2 });
        assert_eq!(st.graveyard, [CardIdx(2)]);
    }
}

/// Face-down permanents have no printed undying ability as they die.
#[test]
fn face_down_victim_does_not_use_its_printed_undying() {
    let cards = deck(&[row(
        "Hidden creature",
        "{1}",
        "Creature — Goblin",
        "Undying",
    )]);
    assert!(cards.cards[0].has_undying);
    let mut st = state(vec![], vec![]);
    let mut victim = new_perm_with(1, &cards, CardIdx(0), 0, false);
    victim.face_down = true;
    st.battlefield.push(victim);
    st.next_uid = 1;

    resolve_sacrifice_uid(&cards, &mut st, 1, 1);

    assert!(st.battlefield.is_empty());
    assert_eq!(st.graveyard, [CardIdx(0)]);
}

/// A dying listener still sees the other creatures in its simultaneous death batch.
#[test]
fn simultaneous_deaths_keep_last_known_listener_sources() {
    let cards = deck(&[
        row(
            "Death listener",
            "{1}",
            "Creature",
            "Whenever another creature you control dies, draw a card.",
        ),
        row("Other creature", "{1}", "Creature", ""),
        row("Remaining artifact", "{1}", "Artifact", ""),
        row("Drawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![], vec![3]);
    for index in 0..3 {
        st.battlefield
            .push(new_perm_with(index + 1, &cards, CardIdx(index), 0, false));
    }
    st.next_uid = 3;

    resolve_sacrifice_batch(&cards, &mut st, 1, &[1, 2]);

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(2)));
    assert_eq!(st.graveyard, [CardIdx(0), CardIdx(1)]);
    assert_eq!(st.hand, [CardIdx(3)]);
    assert_eq!(st.seen, 1);
}
