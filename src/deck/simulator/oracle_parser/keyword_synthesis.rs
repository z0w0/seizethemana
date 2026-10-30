//! Add ability nodes implied by parsed keyword entries.
//!
//! [`synthesize_keyword_abilities`] adds triggered nodes for Mobilize and
//! Afterlife and spell nodes for Amass and Empower Jace. They use the same
//! lowering paths as printed abilities.
//!
//! Other keyword rules become runtime card fields in
//! `oracle_lower::keyword_lower::lower_keywords`. Effect lowering handles
//! keyword-action nodes. Synthesis and keyword lowering read typed syntax,
//! not raw Oracle text.

use super::super::oracle_ast::{
    ObjectSubject, OracleAbility, OracleCard, OracleEffect, OracleKeywordName, OracleSpellAbility,
    OracleTriggerEvent, OracleTriggeredAbility,
};

/// Synthesize the keyword abilities the CR defines as triggered abilities
/// into ordinary AST nodes.
///
/// - Mobilize N (CR 702.181): "Whenever this creature attacks, create
///   N 1/1 red Warrior tokens. Those tokens enter tapped and attacking.
///   Sacrifice them at the beginning of the next end step."
/// - Afterlife N (CR 702.135): "When this permanent is put into a
///   graveyard from the battlefield, create N 1/1 white and black
///   Spirit creature tokens with flying."
///
/// The synthesized nodes use the same shapes the text parser produces,
/// so lowering, triggering, and resolution need no mechanic branches.
pub(crate) fn synthesize_keyword_abilities(oracle: &mut OracleCard) {
    let mobilize = oracle.keyword_number(&OracleKeywordName::Mobilize);
    let afterlife = oracle.keyword_number(&OracleKeywordName::Afterlife);
    if let Some(amount) = mobilize {
        oracle
            .abilities
            .push(OracleAbility::Triggered(OracleTriggeredAbility {
                event: OracleTriggerEvent::Attacks(ObjectSubject::ThisPermanent),
                effects: vec![OracleEffect::Mobilize(amount)],
                is_mana_ability: false,
                once_per_turn: false,
                condition: None,
                event_filter: Default::default(),
                resolution_payment: None,
            }));
    }
    if let Some(amount) = afterlife {
        oracle
            .abilities
            .push(OracleAbility::Triggered(OracleTriggeredAbility {
                event: OracleTriggerEvent::Dies(ObjectSubject::ThisPermanent),
                effects: vec![OracleEffect::Afterlife(amount)],
                is_mana_ability: false,
                once_per_turn: false,
                condition: None,
                event_filter: Default::default(),
                resolution_payment: None,
            }));
    }
    // A spell-shaped keyword action line ("Amass Orcs 2.", "Empower Jace
    // 3.") parses as a standalone keyword entry, not an effect node. Add
    // the matching spell effect so cast resolution executes it.
    let amass = oracle.keyword_number(&OracleKeywordName::Amass);
    if let Some(amount) = amass {
        oracle
            .abilities
            .push(OracleAbility::Spell(OracleSpellAbility {
                effects: vec![OracleEffect::Amass(amount)],
            }));
    }
    if oracle.has_keyword(&OracleKeywordName::EmpowerJace) {
        let amount = oracle
            .keyword_number(&OracleKeywordName::EmpowerJace)
            .unwrap_or(1);
        oracle
            .abilities
            .push(OracleAbility::Spell(OracleSpellAbility {
                effects: vec![OracleEffect::EmpowerJace(amount)],
            }));
    }
}
