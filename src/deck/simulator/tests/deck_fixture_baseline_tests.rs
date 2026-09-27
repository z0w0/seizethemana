use super::deck_test_support::*;
use super::model::{Effect, Role};
use super::oracle_parse::parse_sim_card;

#[test]
fn target_fixtures_have_seeded_baselines_and_parsed_inventory() {
    for (name, turns) in [
        ("dredge modern", 8),
        ("living-end", 8),
        ("neobrand", 8),
        ("ruby-storm-2026", 8),
        ("yawgmoth combo modern", 8),
        ("kinnan combo", 10),
        ("rogsilas turbo naus", 10),
    ] {
        let rows = fixture_map(name);
        let deck = fixture_deck(name);
        let sim_deck = build_sim_deck(&deck, &rows, None);
        let stats = sim(&deck, &rows, 1000, turns);
        for card in sim_deck.cards.iter().filter(|card| card.role != Role::Land) {
            let printed_cost = rows
                .get(&card.name)
                .map(|row| row.mana_cost.as_str())
                .unwrap_or("missing row");
            let abilities = card
                .abilities()
                .map(|ability| format!("{:?}:{:?}", ability.trigger, ability.effect))
                .collect::<Vec<_>>()
                .join("|");
            let destination = if card.is_instant_or_sorcery {
                if card.exile_on_resolve {
                    "exile"
                } else {
                    "graveyard"
                }
            } else {
                "battlefield"
            };
            println!(
                "inventory {name} card={} printed_cost={} parsed_cost={} min_cost={} role={:?} riders=draw:{} life:{} tokens:{} drain:{} mill:{} abilities=[{}] destination={destination}",
                card.name,
                printed_cost,
                card.cost.total(),
                card.min_cost.total(),
                card.role,
                card.riders.draws_on_cast,
                card.riders.life_gain_on_cast,
                card.riders.tokens_on_cast,
                card.riders.drain_on_cast,
                card.riders.mills_on_enter,
                abilities,
            );
        }
        println!(
            "baseline {name} seed=42 runs=1000 turns={turns} lands={} seen_t4={:.2} bodies_t4={:.2} replay={:.2} life_draws={:.2} infinite={:.3}",
            stats.land_count,
            stats.cards_seen[3.min(turns as usize - 1)],
            stats.bodies_by_turn[3.min(turns as usize - 1)],
            stats.replay_casts_avg,
            stats.life_funded_draws_avg,
            stats.infinite_mana_pct,
        );
        assert!(
            sim_deck.cards.iter().any(|card| card.role != Role::Land),
            "{name} fixture must contain castable spells"
        );
    }

    let living_end = fixture_map("living-end");
    let living_end_card = parse_sim_card(
        living_end
            .get("Living End")
            .expect("Living End fixture includes its no-cost payoff"),
    );
    assert!(!living_end_card.has_mana_cost);
    assert_eq!(living_end_card.role, Role::Other);
    assert!(living_end_card.is_instant_or_sorcery);
    assert!(living_end_card.riders.graveyard_creature_exchange);

    let neoform = fixture_map("neobrand");
    assert_eq!(
        parse_sim_card(neoform.get("Neoform").expect("Neoform is in its fixture"))
            .riders
            .additional_cost_bodies,
        1
    );
    assert!(
        parse_sim_card(neoform.get("Neoform").expect("Neoform is in its fixture"))
            .riders
            .search_after_sacrifice
    );

    let ruby = fixture_map("ruby-storm-2026");
    assert!(
        parse_sim_card(
            ruby.get("Past in Flames")
                .expect("Ruby Storm has Past in Flames")
        )
        .riders
        .grants_flashback
    );

    let rogsi = fixture_map("rogsilas turbo naus");
    assert!(
        parse_sim_card(
            rogsi
                .get("Underworld Breach")
                .expect("RogSi has Underworld Breach")
        )
        .riders
        .grants_escape
    );
    assert!(
        parse_sim_card(rogsi.get("Ad Nauseam").expect("RogSi has Ad Nauseam"))
            .riders
            .reveal_rule
            .is_some()
    );

    let yawgmoth = fixture_map("yawgmoth combo modern");
    let yawgmoth = parse_sim_card(
        yawgmoth
            .get("Yawgmoth, Thran Physician")
            .expect("Yawgmoth fixture has its namesake"),
    );
    assert!(super::oracle_parse::parse_oracle_ability(
        "Pay 1 life, Sacrifice another creature: Put a -1/-1 counter on up to one target creature and draw a card."
    )
    .is_some());
    let yawgmoth_abilities = yawgmoth.abilities().collect::<Vec<_>>();
    assert!(
        yawgmoth_abilities.iter().any(|ability| {
            ability.life_cost == 1
                && ability.sacrifice_bodies == 1
                && matches!(ability.effect, Effect::DrawAndMinusCounter)
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
    assert!(kinnan.abilities().any(|ability| {
        matches!(ability.effect, Effect::Search(spec) if spec.top_count == Some(5) && spec.non_human)
    }));
}
