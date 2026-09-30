//! Tests that real deck fixtures exercise parsed mechanics.
use super::deck_test_support::*;
use super::model::{Role, SimEffect};
use super::oracle_lower::parse_sim_card;

#[test]
fn target_fixtures_have_parsed_mechanics() {
    let living_end = fixture_map("living-end");
    let living_end_card = parse_sim_card(
        living_end
            .get("Living End")
            .expect("Living End fixture includes its no-cost payoff"),
    );
    assert!(!living_end_card.has_mana_cost);
    assert_eq!(living_end_card.role, Role::Other);
    assert!(living_end_card.is_instant_or_sorcery);
    assert!(living_end_card.spell_data.graveyard_creature_exchange);

    let neoform = fixture_map("neobrand");
    assert_eq!(
        parse_sim_card(neoform.get("Neoform").expect("Neoform is in its fixture"))
            .spell_data
            .additional_cost_creatures,
        1
    );
    assert!(
        parse_sim_card(neoform.get("Neoform").expect("Neoform is in its fixture"))
            .spell_data
            .search_after_sacrifice
    );

    let ruby = fixture_map("ruby-storm-2026");
    assert!(
        parse_sim_card(
            ruby.get("Past in Flames")
                .expect("Ruby Storm has Past in Flames")
        )
        .spell_data
        .grants_flashback
    );

    let rogsi = fixture_map("rogsilas turbo naus");
    assert!(
        parse_sim_card(
            rogsi
                .get("Underworld Breach")
                .expect("RogSi has Underworld Breach")
        )
        .spell_data
        .grants_escape
    );
    assert!(
        parse_sim_card(rogsi.get("Ad Nauseam").expect("RogSi has Ad Nauseam"))
            .spell_data
            .reveal_rule
            .is_some()
    );

    let yawgmoth = fixture_map("yawgmoth combo modern");
    let yawgmoth = parse_sim_card(
        yawgmoth
            .get("Yawgmoth, Thran Physician")
            .expect("Yawgmoth fixture has its namesake"),
    );
    let yawgmoth_ast = super::oracle_parser::parse_oracle_text(
        "Pay 1 life, Sacrifice another creature: Put a -1/-1 counter on up to one target creature and draw a card.",
        &[],
    );
    assert!(matches!(
        yawgmoth_ast.abilities.as_slice(),
        [super::oracle_ast::OracleAbility::Activated(_)]
    ));
    let yawgmoth_abilities = yawgmoth.unlocked_abilities(0).collect::<Vec<_>>();
    assert!(
        yawgmoth_abilities.iter().any(|ability| {
            ability
                .activation
                .as_ref()
                .is_some_and(|costs| costs.life_payment() == 1 && costs.creature_sacrifices() == 1)
                && matches!(ability.effect, SimEffect::DrawAndMinusCounter)
        }),
        "parsed Yawgmoth abilities: {yawgmoth_abilities:?}"
    );

    let mut has_fetch = false;
    for name in ["dredge modern", "living-end", "neobrand", "ruby-storm-2026"] {
        has_fetch |= fixture_map(name)
            .values()
            .map(parse_sim_card)
            .any(|card| card.is_fetch_land);
    }
    assert!(has_fetch, "the target fixtures include a parsed fetch land");

    let kinnan = fixture_map("kinnan combo");
    let kinnan_deck = build_sim_deck(&fixture_deck("kinnan combo"), &kinnan, None);
    assert!(
        kinnan_deck
            .commanders
            .iter()
            .any(|card| card.name == "Kinnan, Bonder Prodigy")
    );
    let kinnan = &kinnan_deck.commanders[0];
    assert!(kinnan.flags.bonus_mana_on_nonland_tap);
    assert!(kinnan.unlocked_abilities(0).any(|ability| {
        matches!(ability.effect, SimEffect::Search(spec) if spec.top_count == Some(5) && spec.non_human)
    }));
}
