//! X-cost markers and unsupported spell-count mana scaling.

use super::*;
use crate::deck::simulator::support::AbilitySupportStatus;

/// Supported X clauses select the matching runtime substitution rule.
#[test]
fn x_cost_class_parses() {
    let drain = parse_sim_card(&card(
        "X Drain",
        "{X}{B}{B}",
        "Sorcery",
        "Target player loses X life.",
    ));
    assert_eq!(drain.spell_data.x_class, Some(XClass::Drain));
    let draw = parse_sim_card(&card("X Draw", "{X}{U}{U}", "Instant", "Draw X cards."));
    assert_eq!(draw.spell_data.x_class, Some(XClass::Draw));
    let tokens = parse_sim_card(&card(
        "X Tokens",
        "{X}{W}{W}",
        "Sorcery",
        "Create X 1/1 white Soldier creature tokens.",
    ));
    assert_eq!(tokens.spell_data.x_class, Some(XClass::Tokens));
    let none = parse_sim_card(&card(
        "Unmodeled X",
        "{X}{R}",
        "Sorcery",
        "Exile the top card.",
    ));
    assert_eq!(none.spell_data.x_class, None);
}

/// A variable clause has no extra fixed reward on top of the paid X.
#[test]
fn x_clause_fixed_rider_is_not_double_counted() {
    let drain = parse_sim_card(&card(
        "X Drain",
        "{X}{B}{B}",
        "Sorcery",
        "Target player loses X life.",
    ));
    assert_eq!(drain.spell_data.x_class, Some(XClass::Drain));
    assert_eq!(drain.spell_data.life_loss_on_resolve, 0);
    assert!(matches!(
        drain.spell_data.effects.as_slice(),
        [SimEffect::LoseLife {
            amount: 0,
            scope: LifeLossScope::TargetPlayer
        }]
    ));
    let draw = parse_sim_card(&card("X Draw", "{X}{U}{U}", "Instant", "Draw X cards."));
    assert_eq!(draw.spell_data.x_class, Some(XClass::Draw));
    assert_eq!(draw.spell_data.draws_on_cast, 0);
    assert!(matches!(
        draw.spell_data.effects.as_slice(),
        [SimEffect::Draw(0)]
    ));
    let tokens = parse_sim_card(&card(
        "X Tokens",
        "{X}{W}{W}",
        "Sorcery",
        "Create X 1/1 white Soldier creature tokens.",
    ));
    assert_eq!(tokens.spell_data.x_class, Some(XClass::Tokens));
    assert_eq!(tokens.spell_data.tokens_on_cast, 0);
    assert!(matches!(
        tokens.spell_data.effects.as_slice(),
        [SimEffect::Tokens(0)]
    ));
}

/// Unmodeled spell-count mana must not become a fixed mana yield.
#[test]
fn per_cast_mana_scaling_stays_unsupported() {
    let vivi = parse_sim_card(&card(
        "Spell Count Engine",
        "{1}{U}{R}",
        "Creature — Wizard",
        "{T}: Add one mana of any color for each instant or sorcery spell you've cast this turn.",
    ));
    assert!(vivi.spell_data.mana_per_cast.is_none());
    assert!(vivi.tap.is_none());
    assert!(vivi.unlocked_abilities(0).next().is_none());
    assert!(matches!(
        vivi.support[0].status,
        AbilitySupportStatus::Unsupported { .. }
    ));
    let plain = parse_sim_card(&card("Bear", "{1}{G}", "Creature — Bear", "A bear."));
    assert!(plain.spell_data.mana_per_cast.is_none());
}
