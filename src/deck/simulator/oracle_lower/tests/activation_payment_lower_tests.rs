use super::*;
use crate::deck::simulator::model::{Cost, SimActivationCost};

/// Build typed payment controls without calling the source parser.
fn payment_ability(costs: Vec<ActivationCost>) -> OracleActivatedAbility {
    OracleActivatedAbility {
        costs,
        effects: vec![OracleEffect::Draw(1)],
        restrictions: Vec::new(),
        is_mana_ability: false,
        mana_per_spell_cast: false,
    }
}

/// Multiple typed life payments must become one full runtime amount in every order.
#[test]
fn typed_life_payments_lower_to_one_complete_total() {
    for costs in [
        vec![
            ActivationCost::Tap(ActivationTarget::Source),
            ActivationCost::PayLife(1),
            ActivationCost::PayLife(2),
        ],
        vec![
            ActivationCost::PayLife(2),
            ActivationCost::Tap(ActivationTarget::Source),
            ActivationCost::PayLife(1),
        ],
        vec![
            ActivationCost::PayLife(0),
            ActivationCost::PayLife(1),
            ActivationCost::PayLife(2),
        ],
    ] {
        let runtime = payment_ability(costs).to_runtime_all();
        assert_eq!(runtime.len(), 1);
        let activation = runtime[0]
            .activation
            .as_ref()
            .expect("lowering retains the complete cost");
        assert_eq!(activation.life_payment(), 3);
        assert_eq!(
            activation
                .costs
                .iter()
                .filter(|cost| matches!(cost, SimActivationCost::PayLife(_)))
                .count(),
            1
        );
        assert!(activation.costs.contains(&SimActivationCost::PayLife(3)));
    }
}

/// An overflowing sum or a life amount outside the runtime's signed range must stay inert.
#[test]
fn typed_life_payment_totals_cannot_overflow_or_wrap_the_runtime_life_gate() {
    for costs in [
        vec![
            ActivationCost::PayLife(u32::MAX),
            ActivationCost::PayLife(1),
        ],
        vec![
            ActivationCost::PayLife(i32::MAX as u32),
            ActivationCost::PayLife(1),
        ],
        vec![ActivationCost::PayLife(u32::MAX)],
        vec![
            ActivationCost::Mana(Cost {
                phyrexian: [1, 0, 0, 0, 0],
                ..Cost::default()
            }),
            ActivationCost::PayLife(i32::MAX as u32),
        ],
    ] {
        assert!(payment_ability(costs).to_runtime_all().is_empty());
    }
}

/// A known life payment does not excuse an unknown or unpayable discard requirement.
#[test]
fn typed_unknown_and_discard_costs_disable_the_complete_activation() {
    for extra in [
        ActivationCost::Other("Pay X life".into()),
        ActivationCost::Discard(1),
    ] {
        let ability = payment_ability(vec![
            ActivationCost::PayLife(1),
            ActivationCost::PayLife(2),
            extra,
        ]);
        assert!(ability.to_runtime_all().is_empty());
    }
}

/// The runtime cannot consume just one of several typed counter or sacrifice costs.
#[test]
fn typed_repeated_counter_and_sacrifice_payments_keep_the_whole_mode_inert() {
    for costs in [
        vec![
            ActivationCost::RemoveCounter {
                kind: Some("charge".into()),
                count: 1,
            },
            ActivationCost::RemoveCounter {
                kind: Some("charge".into()),
                count: 1,
            },
        ],
        vec![
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Source,
                exclude_source: false,
            },
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Source,
                exclude_source: false,
            },
        ],
        vec![
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Creature,
                exclude_source: false,
            },
            ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Creature,
                exclude_source: false,
            },
        ],
    ] {
        assert!(payment_ability(costs).to_runtime_all().is_empty());
    }
}

/// Typed compound loyalty costs cannot bypass the parser's complete-cost rejection.
#[test]
fn typed_compound_and_duplicate_loyalty_costs_keep_the_whole_mode_inert() {
    for costs in [
        vec![ActivationCost::Loyalty(-1), ActivationCost::PayLife(1)],
        vec![ActivationCost::PayLife(1), ActivationCost::Loyalty(-1)],
        vec![ActivationCost::Loyalty(0), ActivationCost::PayLife(1)],
        vec![
            ActivationCost::Loyalty(1),
            ActivationCost::Mana(Cost {
                generic: 1,
                ..Cost::default()
            }),
        ],
        vec![ActivationCost::Loyalty(-1), ActivationCost::Loyalty(-1)],
        vec![ActivationCost::Loyalty(1), ActivationCost::Loyalty(-1)],
        vec![ActivationCost::Loyalty(0), ActivationCost::Loyalty(0)],
    ] {
        assert!(payment_ability(costs).to_runtime_all().is_empty());
    }
}

/// Sole positive, zero, and negative loyalty costs retain their exact typed amounts.
#[test]
fn sole_loyalty_costs_keep_their_parser_and_lowering_representation() {
    for (payment, amount) in [("+1", 1), ("0", 0), ("-1", -1), ("−1", -1), ("–1", -1)] {
        let oracle = crate::deck::simulator::oracle_parser::parse_oracle_activated_ability(
            &format!("{payment}: Draw a card."),
        )
        .expect("the sole loyalty activation is retained");
        assert!(
            matches!(oracle.costs.as_slice(), [ActivationCost::Loyalty(value)] if *value == amount)
        );
        let runtime = payment_ability(vec![ActivationCost::Loyalty(amount)]).to_runtime_all();
        assert_eq!(runtime.len(), 1);
        let activation = runtime[0]
            .activation
            .as_ref()
            .expect("the loyalty cost remains typed");
        assert_eq!(activation.costs, [SimActivationCost::Loyalty(amount)]);
    }
}

/// A present zero loyalty symbol differs from no loyalty cost or a zero mana cost.
#[test]
fn loyalty_cost_presence_keeps_zero_loyalty_distinct_from_zero_mana() {
    let loyalty = SimActivation {
        costs: vec![SimActivationCost::Loyalty(0)],
        restrictions: Vec::new(),
    };
    let mana = SimActivation {
        costs: vec![SimActivationCost::Mana(Cost::default())],
        restrictions: Vec::new(),
    };
    assert!(loyalty.has_loyalty_cost());
    assert_eq!(loyalty.loyalty_change(), 0);
    assert!(!mana.has_loyalty_cost());
    assert!(!SimActivation::default().has_loyalty_cost());
}

/// Only explicit targeted returns with fully modeled selectors and destinations may lower.
#[test]
fn graveyard_return_near_misses_keep_their_source_and_block_linked_rewards() {
    for instruction in [
        "Return a creature card from your graveyard to the battlefield",
        "Return a card from your graveyard to your hand",
        "You may return a creature card from your graveyard to the battlefield",
        "Return one creature card from your graveyard to the battlefield",
        "Return up to one target creature card from your graveyard to the battlefield",
        "Return target creature card from your graveyard to the battlefield tapped",
        "Return target creature card from your graveyard to the battlefield under your control",
        "Return target creature card with mana value 3 or less from your graveyard to the battlefield",
        "Return target card from your graveyard to the battlefield",
        "Return target creature card from your graveyard to your hand",
        "Return target card from your graveyard to its owner's hand",
        "Return target creature card from a graveyard to the battlefield",
        "Return target artifact card from your graveyard to the battlefield",
        "Return two target creature cards from your graveyard to the battlefield",
    ] {
        let source = format!("Sacrifice a creature: {instruction}. Gain 3 life.");
        let oracle = crate::deck::simulator::oracle_parser::parse_oracle_text(&source, &[]);
        assert_eq!(oracle.ability_sources.len(), 1);
        assert_eq!(oracle.ability_sources[0].text, source);
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
            panic!("the complete activated statement remains typed: {instruction}");
        };
        assert!(
            matches!(ability.effects.as_slice(), [OracleEffect::Unsupported(text), OracleEffect::GainLife(3)] if text == instruction),
            "{instruction}: {:?}",
            ability.effects
        );
        assert!(ability.to_runtime_all().is_empty(), "{instruction}");
        assert!(lower_activation_effects(ability).is_none(), "{instruction}");
    }
}

/// Unsupported return data blocks gains on either side of the return without source parsing.
#[test]
fn typed_unsupported_graveyard_choices_cannot_emit_partial_linked_gains() {
    for instruction in [
        "Return a creature card from your graveyard to the battlefield",
        "Return a card from your graveyard to your hand",
    ] {
        for unsupported_first in [true, false] {
            let mut ability = payment_ability(vec![ActivationCost::Sacrifice {
                count: 1,
                object: CostObject::Creature,
                exclude_source: false,
            }]);
            ability.effects = if unsupported_first {
                vec![
                    OracleEffect::Unsupported(instruction.into()),
                    OracleEffect::GainLife(3),
                ]
            } else {
                vec![
                    OracleEffect::GainLife(3),
                    OracleEffect::Unsupported(instruction.into()),
                ]
            };
            assert!(ability.to_runtime_all().is_empty());
            assert!(lower_activation_effects(&ability).is_none());
        }
    }
}

/// Explicit targeted return data retains its one-card destination and linked gain.
#[test]
fn explicit_targeted_graveyard_returns_preserve_their_complete_lowering() {
    for (instruction, to_hand) in [
        ("Return target card from your graveyard to your hand", true),
        (
            "Return target creature card from your graveyard to the battlefield",
            false,
        ),
        (
            "You may return target creature card from your graveyard to the battlefield",
            false,
        ),
    ] {
        let source = format!("{{T}}: {instruction}. Gain 3 life.");
        let oracle = crate::deck::simulator::oracle_parser::parse_oracle_activated_ability(&source)
            .expect("the complete target instruction is recognized");
        assert!(
            matches!(oracle.effects.as_slice(), [OracleEffect::ReturnFromGraveyard { to_hand: destination, count: 1 }, OracleEffect::GainLife(3)] if *destination == to_hand)
        );
        let mut ability = payment_ability(vec![ActivationCost::Tap(ActivationTarget::Source)]);
        ability.effects = vec![
            OracleEffect::ReturnFromGraveyard { to_hand, count: 1 },
            OracleEffect::GainLife(3),
        ];
        let runtime = ability.to_runtime_all();
        assert_eq!(runtime.len(), 1);
        assert!(
            matches!(runtime[0].effect_sequence(), [SimEffect::ReturnFromGraveyard { to_hand: destination, count: 1 }, SimEffect::GainLife(3)] if *destination == to_hand)
        );
    }
}
