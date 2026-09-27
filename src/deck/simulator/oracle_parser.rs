//! Grammar parser for the supported subset of Magic Oracle text.

/// Parse activated-ability costs and timing restrictions.
mod activation;
/// Parse effects and their supported clause shapes.
mod effects;
/// Parse standalone keyword abilities.
pub(crate) mod keywords;
/// Split Oracle text and classify ability statements.
mod statements;
/// Number-word and numeral extraction from Oracle text.
mod text;
/// Parse triggered abilities and Saga chapter statements.
mod triggers;

use super::oracle_ast::*;
use effects::parse_oracle_effects;
use keywords::{add_keyword, keyword_line_list, parse_oracle_keyword, standalone_keyword};
use statements::{
    is_spell_statement, is_static_statement, oracle_segments, strip_ability_word, unsupported_kind,
};
use triggers::{parse_oracle_saga_chapter, parse_oracle_trigger_event, parse_oracle_triggered};

/// Keep the activated parser available at its existing simulator path.
pub(super) use activation::parse_oracle_activated_ability;
pub(super) use text::{amount_after, draw_amount};

/// Parse Oracle text and its card-data keyword list into typed syntax nodes.
pub fn parse_oracle_text(oracle_text: &str, keywords: &[String]) -> OracleCard {
    let mut card = OracleCard {
        keywords: keywords
            .iter()
            .map(|keyword| parse_oracle_keyword(keyword))
            .collect(),
        abilities: Vec::new(),
    };

    for source in oracle_segments(oracle_text, &card.keywords) {
        if let Some(keywords) = keyword_line_list(&source, &card.keywords) {
            for keyword in keywords {
                add_keyword(&mut card.keywords, keyword);
            }
            continue;
        }
        if let Some(keyword) = standalone_keyword(&source, &card.keywords) {
            add_keyword(&mut card.keywords, parse_oracle_keyword(&keyword));
            continue;
        }
        if let Some(chapter) = parse_oracle_saga_chapter(&source) {
            card.abilities.push(OracleAbility::SagaChapter(chapter));
        } else if let Some(ability) = parse_oracle_activated_ability(strip_ability_word(&source)) {
            card.abilities.push(OracleAbility::Activated(ability));
        } else if let Some(event) = parse_oracle_trigger_event(&source) {
            card.abilities
                .push(OracleAbility::Triggered(parse_oracle_triggered(
                    source, event,
                )));
        } else if is_static_statement(&source) {
            card.abilities
                .push(OracleAbility::Static(statements::parse_oracle_static(
                    &source,
                )));
        } else if is_spell_statement(&source) {
            card.abilities.push(OracleAbility::Spell(SpellAbility {
                effects: parse_oracle_effects(&source),
            }));
        } else {
            card.abilities
                .push(OracleAbility::Unsupported(UnsupportedAbility {
                    kind: unsupported_kind(&source),
                    source,
                }));
        }
    }
    card
}
