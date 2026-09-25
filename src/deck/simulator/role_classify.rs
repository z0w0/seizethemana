// Functional role classification for the simulator: a parsed card's
// deckbuilding job (Land, Rock, Removal, Draw, Lock, ...), read from the
// card row and the oracle text. Split from parse.rs to keep files small.

use super::super::stats::{is_dork, is_rock};
use super::model::TapYield;
use super::parse::damage_removal_shape;
use crate::db::CardRow;

/// Functional role classification.
pub fn classify(
    row: &CardRow,
    text: &str,
    tap: &Option<TapYield>,
    mana_on_cast: &Option<TapYield>,
    land: bool,
    station: bool,
) -> super::model::Role {
    use super::model::Role;
    if land {
        return Role::Land;
    }
    if is_rock(row) {
        return Role::Rock;
    }
    if is_dork(row) {
        return Role::Dork;
    }
    if tap.is_some() || mana_on_cast.is_some() {
        return Role::RampSpell;
    }
    // Removal first: "Destroy target creature. Draw a card." is a
    // removal spell with a rider, not a draw engine. Draw-role counts
    // would skew without this order. Protection grants ("gains
    // hexproof") are not removal: they answer nothing in a goldfish.
    let removal = text.contains("destroy target")
        || text.contains("destroy all")
        || text.contains("exile target")
        || text.contains("counter target")
        || text.contains("return target")
            && (text.contains("to its owner's hand") || text.contains("to their owner's hand"))
        || text.contains("prevent all combat damage")
        || text.contains("prevent the next") && text.contains("damage")
        || text.contains("creatures with power") && text.contains("can't attack")
        || text.contains("can't attack or block")
        || text.contains("regenerate target")
        || damage_removal_shape(&row.oracle_text);
    if removal {
        return Role::Removal;
    }
    // Draw: repeatable engines and one-shot draws; tutors and look-at-top
    // effects refuel the hand.
    let draws = text.contains("draw ")
        || text.contains("drawn")
        || text.contains("investigate")
        || text.contains("surveil")
        || text.contains("search your library")
        || (text.contains("look at the top") && text.contains("into your hand"));
    if draws && !text.contains("opponent") {
        return Role::Draw;
    }
    if station {
        // Spacecraft/planets with no draw text are wincons when big.
        if row.cmc >= 5.0 {
            return Role::Wincon;
        }
        return Role::Other;
    }
    // Static tax/restriction pieces (stax): their timing is the question.
    // "This spell costs {1} more to cast for each target" is a multi-target
    // rider (multi-target X spells), not a tax: exclude "for each
    // target" shapes.
    let lock = (text.contains("cost")
        && text.contains("more to cast")
        && !text.contains("for each target")
        && !text.contains("for each additional target"))
        || text.contains("players can't cast more than")
        || text.contains("can't untap")
        || text.contains("doesn't untap")
        || text.contains("don't untap")
        || text.contains("skip your draw step") && !row.type_line.contains("Creature")
        || (text.contains("activated abilities")
            && text.contains("unless")
            && !row.type_line.contains("Creature"));
    if lock {
        return Role::Lock;
    }
    // Equipment, auras, and pump spells: the suit-up cadence question.
    let booster = row.type_line.contains("Equipment")
        || row.type_line.contains("Aura")
        || (text.contains("target creature")
            && text.contains("gets +")
            && (text.contains("until end of turn") || row.type_line.contains("Enchantment")));
    if booster {
        return Role::Booster;
    }
    let big_threat = row.cmc >= 5.0
        || text.contains("win the game")
        || text.contains("loses the game")
        || (row.type_line.contains("Creature")
            && row
                .power
                .as_deref()
                .and_then(|p| p.trim_end_matches('*').parse::<i32>().ok())
                .is_some_and(|p| p >= 5));
    if big_threat {
        return Role::Wincon;
    }
    Role::Other
}
