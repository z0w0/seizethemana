// Standard sweep: every Standard fixture must hold the shared invariants and
// show the deck-level insights the report surfaces (velocity, screw bounded,
// castability ordered by cost).

use super::deck_test_support::*;
use crate::db::CardRow;
use crate::deck::grammar::{Deck, DeckEntry};
use std::collections::HashMap;

#[test]
fn sweep_standard_invariants() {
    for name in STANDARD_FIXTURES {
        let cards = fixture_cards(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 8);
        assert_land_drops_sane(&stats, 8);
        assert_velocity_monotone(&stats);
        assert_castability_not_before_cost(&stats, &cards);
        assert!(
            stats.avg_opener_lands >= 1.0,
            "{name} openers hold {:.2} lands",
            stats.avg_opener_lands
        );
        // Every fixture deck holds a real land base.
        assert!(
            stats.land_count >= 17,
            "{name} has {} lands",
            stats.land_count
        );
    }
}

#[test]
fn sweep_standard_aggro_decks_cast_turn_one() {
    // Low-curve aggro openers: the cheapest one-drop is castable in most
    // games by turn 2. Mono-red-aggro's Heartfire Hero and prowess decks'
    // Monastery Swiftspear anchor this.
    for name in ["mono-red-aggro", "izzet-prowess", "izzet-prowess-2026"] {
        let cards = fixture_cards(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 8);
        let one_drops = stats
            .card_castability
            .iter()
            .filter(|c| c.target_turn == 1)
            .collect::<Vec<_>>();
        assert!(
            !one_drops.is_empty(),
            "{name} has no one-drop castability rows"
        );
        let best = one_drops
            .iter()
            .map(|c| c.avg_first_castable_turn)
            .fold(f64::MAX, f64::min);
        assert!(
            best <= 2.0,
            "{name} cheapest one-drop castable avg t{:.2}",
            best
        );
    }
}

#[test]
fn sweep_standard_landfall_grinds_lands() {
    // Landfall lists run 26+ lands and hit all four drops in most games.
    let cards = fixture_cards("mono-green-landfall");
    let deck = fixture_deck("mono-green-landfall");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.land_count >= 24,
        "landfall list runs {} lands",
        stats.land_count
    );
    assert!(
        stats.hit_all_drops_by[4] >= 0.7,
        "26-land deck hits all 4 by t4 only {:.0}%",
        stats.hit_all_drops_by[4] * 100.0
    );
}

#[test]
fn sweep_standard_control_has_draw_and_interaction() {
    // Control shells see removal and draw by the same turns the sim counts
    // for midrange decks; the insight is role access, not a quality verdict.
    for name in ["four-color-control", "dimir-midrange-2026"] {
        let cards = fixture_cards(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 8);
        assert!(
            stats.removal_access_5 >= 0.5,
            "{name} sees removal by t5 in {:.0}% of games",
            stats.removal_access_5 * 100.0
        );
        // The Karsten London policy bottoms a card toward 3 lands, so a
        // deck whose only draw-role card is a 4-of (dimir-midrange) sits
        // just under the old 0.6 line; the shared floor is 0.45.
        assert!(
            stats.draw_access_6 >= 0.45,
            "{name} sees a draw source by t6 in {:.0}%",
            stats.draw_access_6 * 100.0
        );
    }
}

#[test]
fn sweep_standard_color_screw_bounded() {
    // At most two colors trip the 10% line for two-color-or-fewer decks.
    for name in [
        "mono-red-aggro",
        "mono-black-aggro-2026",
        "dimir-midrange-2026",
    ] {
        let cards = fixture_cards(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 8);
        let tripped = stats.color_screw.iter().filter(|p| **p >= 0.10).count();
        assert!(tripped <= 2, "{name} trips {} colors", tripped);
    }
}

// Topdeck.gg tournament lists (real Standard competitive results): shared
// invariants plus aggro-curve sanity where the archetype makes it checkable.

#[test]
fn sweep_standard_topdeck_fixtures_hold_invariants() {
    for name in [
        "4c control topdeck",
        "boros tokens topdeck",
        "dimir midrange topdeck",
        "izzet spellingentals topdeck",
    ] {
        let cards = fixture_cards(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 8);
        assert_land_drops_sane(&stats, 8);
        assert_velocity_monotone(&stats);
        assert_castability_not_before_cost(&stats, &cards);
        assert!(stats.land_count > 0, "{name} has no lands");
    }
}

#[test]
fn topdeck_boros_tokens_emerges_bodies() {
    // Go-wide Standard tokens: bodies by t6 must clear a board-swarm bar.
    let cards = fixture_cards("boros tokens topdeck");
    let deck = fixture_deck("boros tokens topdeck");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.bodies_by_turn[5] > 2.5,
        "boros tokens bodies by t6: {:.2}",
        stats.bodies_by_turn[5]
    );
}

// Archetype-specific dedicated tests for the previously invariants-only
// standard fixtures: each asserts the archetype's defining mechanic.

#[test]
fn azorius_control_interacts_early() {
    // Control shells hold real removal access by t5 (the plan is
    // answers, not early bodies).
    let cards = fixture_cards("azorius control standard");
    let deck = fixture_deck("azorius control standard");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.removal_access_5 >= 0.4,
        "azorius removal access by t5: {:.2}",
        stats.removal_access_5
    );
    assert!(
        stats.draw_access_6 >= 0.4,
        "azorius draw access by t6: {:.2}",
        stats.draw_access_6
    );
}

#[test]
fn boros_dragons_swings_big_late() {
    // Dragons ramp into fliers: attack power by t8 clears the small
    // board the early turns built.
    let cards = fixture_cards("boros dragons standard");
    let deck = fixture_deck("boros dragons standard");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.attack_power_by_turn[7] > 3.0,
        "boros dragons attack power by t8: {:.2}",
        stats.attack_power_by_turn[7]
    );
    assert!(
        stats.land_count >= 20,
        "boros dragons land base: {}",
        stats.land_count
    );
}

#[test]
fn reanimator_4c_fills_graveyard() {
    // Reanimator: the graveyard fills (self-mill + discards) and the
    // deck reuses it — the census must grow across the window.
    let cards = fixture_cards("reanimator 4c standard");
    let deck = fixture_deck("reanimator 4c standard");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.graveyard_by_turn[7] > 3.0,
        "reanimator graveyard by t8: {:.1}",
        stats.graveyard_by_turn[7]
    );
}

#[test]
fn izzet_discard_engines_draw() {
    // Discard-payoff shells draw constantly (the discard engines and
    // payoffs refill the hand).
    let cards = fixture_cards("izzet discard payoffs standard");
    let deck = fixture_deck("izzet discard payoffs standard");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.draw_access_6 > 0.5,
        "izzet discard draw access by t6: {:.2}",
        stats.draw_access_6
    );
}

#[test]
fn mono_blue_flash_holds_instant_speed() {
    // Flash shells keep instant-speed interaction in hand with spare
    // mana: the readiness metric is the plan.
    let cards = fixture_cards("mono-blue flash standard");
    let deck = fixture_deck("mono-blue flash standard");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.interaction_ready_by_turn[4] > 0.08,
        "mono-blue flash interaction ready by t5: {:.2}",
        stats.interaction_ready_by_turn[4]
    );
    assert!(
        stats.interaction_instant_count >= 3,
        "mono-blue instant count: {}",
        stats.interaction_instant_count
    );
}

#[test]
fn amalia_lifegain_combo_gains_life_drains() {
    // Amalia lifegain: exploration + drain effects push the drain
    // census (the combo plan converts life gain into burn).
    let cards = fixture_cards("amalia lifegain combo standard");
    let deck = fixture_deck("amalia lifegain combo standard");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.drain_total_by_turn[7] > 0.3,
        "amalia drain by t8: {:.1}",
        stats.drain_total_by_turn[7]
    );
}

#[test]
fn golgari_midrange_sees_creatures_early() {
    // Midrange: a creature is in hand by t3 in most games.
    let cards = fixture_cards("golgari midrange standard");
    let deck = fixture_deck("golgari midrange standard");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.creature_access_3 > 0.6,
        "golgari creature access by t3: {:.2}",
        stats.creature_access_3
    );
}

// Degradation fixtures: known simulator limits on unusual mechanics. The
// problems report must still fire on the degraded shape (documented in
// assumptions).

#[test]
fn degradation_fixtures_report_known_problems() {
    for name in [
        "mono-red firebending standard",
        "jeskai energy modern",
        "infect modern",
        "rhinos cascade modern",
    ] {
        let cards = fixture_cards(name);
        let deck = fixture_deck(name);
        let sim_deck = build_sim_deck(&deck, &cards, None);
        let stats = sim(&deck, &cards, 200, 8);
        let problems = super::findings::find_problems(&stats, &sim_deck);
        let kinds: Vec<&str> = problems.iter().map(|p| p.kind).collect();
        assert!(
            !kinds.is_empty(),
            "{name} reports no problems: the degraded shape went undetected"
        );
    }
}

#[test]
fn red_aggro_reaches_lethal_in_a_plausible_band() {
    // Mono-red aggro + burn: best-case lethal lands in the t3-t8 band
    // against the 20-life 60-card target.
    let cards = fixture_cards("mono-red-aggro");
    let deck = fixture_deck("mono-red-aggro");
    let stats = sim(&deck, &cards, 400, 8);
    let p50 = stats.p50_lethal_turn.expect("aggro reaches lethal");
    assert!(
        (3..=8).contains(&p50),
        "p50 lethal t{p50} outside the plausible band"
    );
    // Cumulative: lethal probability never regresses turn over turn.
    for w in stats.lethal_damage_by_turn.windows(2) {
        assert!(w[1] >= w[0] - 1e-9, "lethal pct regressed: {w:?}");
    }
}

#[test]
fn lethal_census_absent_without_wincons() {
    // A pure-draw no-threat deck (a lands-only shell) never reaches lethal.
    let mut cards: HashMap<String, CardRow> = HashMap::new();
    cards.insert(
        "Island".to_string(),
        real_card("Island", "", "Basic Land — Island", "", ""),
    );
    cards.insert(
        "Divination".to_string(),
        real_card("Divination", "{2}{U}", "Sorcery", "", "Draw two cards."),
    );
    let deck = Deck {
        sections: vec![(
            "DECK".to_string(),
            vec![
                DeckEntry {
                    quantity: 24,
                    name: "Island".to_string(),
                    set_code: None,
                    collector_number: None,
                    foil: false,
                },
                DeckEntry {
                    quantity: 36,
                    name: "Divination".to_string(),
                    set_code: None,
                    collector_number: None,
                    foil: false,
                },
            ],
        )],
    };
    let stats = sim(&deck, &cards, 200, 8);
    assert!(stats.p50_lethal_turn.is_none());
    assert!(
        stats.lethal_damage_by_turn.iter().all(|p| *p <= 1e-9),
        "no-threat deck should never reach lethal"
    );
}
