//! Independent typed sequences cannot drop an unsupported replacement requirement.

use super::*;
use crate::deck::simulator::model::ManaColor;
use crate::deck::simulator::oracle_ast::tribal::{
    OracleCreatureToken, OracleQuantity, OracleTribalEffect,
};
use crate::deck::simulator::oracle_ast::{OracleEventFilter, PlayerScope};

/// Build the printed first token without using the source parser as the expected rule.
fn insect_token() -> OracleEffect {
    OracleEffect::Tribal(OracleTribalEffect::CreateCreatureTokens {
        quantity: OracleQuantity::Fixed(1),
        token: OracleCreatureToken {
            power: 1,
            toughness: 1,
            colors: vec![ManaColor::Green],
            subtypes: vec!["insect".into()],
            tapped: false,
            attacking: false,
        },
    })
}

/// State a complete hand-built land-entry trigger with ordered required effects.
fn land_entry_trigger(effects: Vec<OracleEffect>) -> OracleTriggeredAbility {
    OracleTriggeredAbility {
        event: OracleTriggerEvent::LandEnters(PlayerScope::You),
        effects,
        condition: None,
        resolution_payment: None,
        event_filter: OracleEventFilter::default(),
        once_per_turn: false,
        is_mana_ability: false,
    }
}

/// An unsupported later replacement blocks the first token and every linked reward.
#[test]
fn typed_token_replacement_requirement_blocks_the_whole_trigger() {
    for text in [
        "If you control six or more lands, create a token that's a copy of this creature instead.",
        "Create two Treasure tokens instead.",
        "If you control six or more lands, create two Treasure tokens.",
    ] {
        let effects = vec![
            insect_token(),
            OracleEffect::Unsupported(text.into()),
            OracleEffect::CreateTreasureTokens(2),
            OracleEffect::GainLife(2),
        ];
        assert!(lower_effect_sequence_with_fixed_tokens(&effects).is_none());
        assert!(lower_effect_sequence(&effects).is_none());
        assert!(land_entry_trigger(effects).to_runtime_all().is_empty());
    }
}

/// The control proves only one generic runtime body when there is no required replacement.
#[test]
fn typed_unconditional_token_control_keeps_one_generic_body() {
    let trigger = land_entry_trigger(vec![insect_token()]);
    let lowered = trigger.to_runtime_all();
    let [ability] = lowered.as_slice() else {
        panic!("the simple typed trigger lowers once");
    };
    assert_eq!(ability.trigger, SimTrigger::LandEnters);
    assert_eq!(ability.event_subject, SimEventSubject::Any);
    assert_eq!(ability.condition, None);
    assert_eq!(ability.resolution_payment, None);
    assert!(matches!(ability.effect_sequence(), [SimEffect::Tokens(1)]));
}
