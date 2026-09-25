use super::Role;

#[test]
fn parse_resolves_every_known_name() {
    // Every name the help line advertises must resolve; a missing arm
    // would print a suggestion that then errors.
    for name in Role::known_names() {
        assert!(Role::parse(name).is_some(), "known name failed: {name}");
    }
}

#[test]
fn parse_normalizes_separators_and_case() {
    assert_eq!(Role::parse("Card-Draw"), Some(Role::Draw));
    assert_eq!(Role::parse("card_draw"), Some(Role::Draw));
    assert_eq!(Role::parse("CARD DRAW"), Some(Role::Draw));
    assert_eq!(Role::parse("board-wipe"), Some(Role::BoardWipe));
}

#[test]
fn parse_unknown_word_is_none() {
    assert_eq!(Role::parse("wibble"), None);
    assert_eq!(Role::parse(""), None);
}

#[test]
fn parse_known_roles_map_to_their_enum() {
    assert_eq!(Role::parse("draw"), Some(Role::Draw));
    assert_eq!(Role::parse("ramp"), Some(Role::Ramp));
    assert_eq!(Role::parse("removal"), Some(Role::Removal));
    assert_eq!(Role::parse("wipe"), Some(Role::BoardWipe));
    assert_eq!(Role::parse("counterspell"), Some(Role::Counterspell));
    assert_eq!(Role::parse("land"), Some(Role::Land));
    assert_eq!(Role::parse("tutor"), Some(Role::Tutor));
    assert_eq!(Role::parse("wincon"), Some(Role::Wincon));
    assert_eq!(Role::parse("stax"), Some(Role::Stax));
    assert_eq!(Role::parse("sac"), Some(Role::Sacrifice));
    assert_eq!(Role::parse("cantrip"), Some(Role::CardSelection));
    assert_eq!(Role::parse("loot"), Some(Role::Discard));
    assert_eq!(Role::parse("mill"), Some(Role::Mill));
    assert_eq!(Role::parse("burn"), Some(Role::Burn));
    assert_eq!(Role::parse("blink"), Some(Role::Blink));
    assert_eq!(Role::parse("landfall"), Some(Role::Landfall));
    assert_eq!(Role::parse("voltron"), Some(Role::Voltron));
    assert_eq!(Role::parse("tribal"), Some(Role::Typal));
    assert_eq!(Role::parse("interaction"), Some(Role::Removal));
}

#[test]
fn every_enum_variant_has_a_parse_name() {
    // A variant with no resolvable name is dead surface: nothing can
    // request it through --role.
    let variants = [
        Role::Draw,
        Role::Removal,
        Role::Ramp,
        Role::Wincon,
        Role::Counterspell,
        Role::Land,
        Role::BoardWipe,
        Role::Tutor,
        Role::Sacrifice,
        Role::Reanimate,
        Role::Recursion,
        Role::Discard,
        Role::Mill,
        Role::Lifegain,
        Role::Burn,
        Role::Token,
        Role::Anthem,
        Role::Equipment,
        Role::Evasion,
        Role::CombatTrick,
        Role::Theft,
        Role::Protection,
        Role::Stax,
        Role::GraveyardHate,
        Role::Combo,
        Role::Storm,
        Role::Blink,
        Role::Landfall,
        Role::Artifact,
        Role::Enchantment,
        Role::Planeswalker,
        Role::Counters,
        Role::Energy,
        Role::Vehicles,
        Role::GroupHug,
        Role::Politics,
        Role::Voltron,
        Role::Spellslinger,
        Role::Typal,
        Role::ExtraTurn,
        Role::ManaSink,
        Role::CardSelection,
    ];
    for variant in variants {
        assert!(
            Role::known_names()
                .iter()
                .any(|name| Role::parse(name) == Some(variant)),
            "variant unreachable from parse: {variant:?}"
        );
    }
}
