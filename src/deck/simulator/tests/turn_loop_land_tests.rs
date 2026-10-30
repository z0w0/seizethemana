use super::*;

#[test]
fn additional_land_granted_after_land_phase_is_available_later() {
    let land = row("Island", "", "Basic Land — Island", "");
    let deck = deck(&[land]);
    let mut st = state(vec![0], vec![]);
    let mut census = TurnCensus::new(1, 1);
    census.land_drops[0] = 1;
    super::game_effects::apply_effect_at(&deck, &SimEffect::ExtraLand, &mut st, 1, false, None);

    super::game_run::play_late_land(&deck, &mut st, &mut census, 1);

    assert_eq!(census.land_drops[0], 2);
    assert!(st.hand.is_empty());

    let mut no_grant = state(vec![0], vec![]);
    let mut no_grant_census = TurnCensus::new(1, 1);
    no_grant_census.land_drops[0] = 1;
    super::game_run::play_late_land(&deck, &mut no_grant, &mut no_grant_census, 1);
    assert_eq!(no_grant_census.land_drops[0], 1);
    assert_eq!(no_grant.hand, [CardIdx(0)]);
}

#[test]
fn cast_additional_land_spell_uses_remaining_turn_land_allowance() {
    let deck = deck(&[
        row("Island one", "", "Basic Land — Island", "{T}: Add {U}."),
        row("Island two", "", "Basic Land — Island", "{T}: Add {U}."),
        row(
            "Explore",
            "{U}",
            "Sorcery",
            "You may play an additional land this turn. Draw a card.",
        ),
        row("Drawn card", "{9}", "Sorcery", ""),
    ]);
    assert_eq!(deck.cards[2].spell_data.extra_land_drops_on_cast, 1);
    let mut st = state(vec![2, 0, 1], vec![3]);
    let mut census = TurnCensus::new(1, deck.cards.len());
    let commander = CommanderProfile::new(&deck);
    let mut repeatable_sources = Vec::new();

    run_turn(
        &deck,
        &mut st,
        &mut census,
        &commander,
        &mut repeatable_sources,
        1,
        false,
    );

    assert_eq!(census.land_drops[0], 2);
    assert!(!st.hand.contains(&CardIdx(1)));
}
