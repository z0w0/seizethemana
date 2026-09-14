// Standard sweep: every Standard fixture must hold the shared invariants and
// show the deck-level insights the report surfaces (velocity, screw bounded,
// castability ordered by cost).

use super::deck_test_support::*;

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
        // Every fixture deck has at least one land and one spell with a cost.
        assert!(stats.land_count > 0, "{name} has no lands");
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
        assert!(
            stats.draw_access_6 >= 0.6,
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
