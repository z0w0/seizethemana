//! Lower keyword entries into runtime fields, and synthesize the keyword
//! abilities the CR defines as triggered abilities into ordinary AST nodes.
//!
//! Layering contract (AST first, then lowering):
//!
//! 1. `oracle_parser` builds the typed AST: keyword entries with typed
//!    [`KeywordArgument`]s, and effect nodes for keyword actions.
//! 2. [`synthesize_keyword_abilities`] turns `mobilize` and `afterlife`
//!    into ordinary [`OracleTriggeredAbility`] nodes, so they run through the
//!    same trigger/effect pipeline as printed abilities.
//! 3. The remaining keyword abilities are continuous or payment rules
//!    (`convoke`, `delve`, `storm`, `living metal`, `offspring`, `plot`,
//!    `saddle`, `teamwork`); [`lower_keyword_abilities`] maps them onto
//!    runtime card fields.
//! 4. Keyword actions (`amass`, `explore`, `connive`, the Ring, empower
//!    Jace) stay [`OracleEffect`] nodes and lower with the effect
//!    lowering.
//!
//! No step re-scans raw Oracle text: everything reads the AST.

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
