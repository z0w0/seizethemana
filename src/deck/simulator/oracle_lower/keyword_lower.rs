//! Lower typed keyword entries into runtime card fields.

use super::super::model::KeywordAbilities;
use super::super::oracle_ast::{OracleCard, OracleKeywordName};

/// Lower parsed keyword abilities into runtime card fields.
pub(super) fn lower_keywords(oracle: &OracleCard) -> KeywordAbilities {
    KeywordAbilities {
        saddle: oracle.keyword_number(&OracleKeywordName::Saddle),
        convoke: oracle.has_keyword(&OracleKeywordName::Convoke),
        delve: oracle.has_keyword(&OracleKeywordName::Delve),
        storm: oracle.has_keyword(&OracleKeywordName::Storm),
        offspring: oracle.keyword_cost(&OracleKeywordName::Offspring),
        plot: oracle.keyword_cost(&OracleKeywordName::Plot),
        living_metal: oracle.has_keyword(&OracleKeywordName::LivingMetal),
        teamwork: oracle.keyword_number(&OracleKeywordName::Teamwork),
    }
}
