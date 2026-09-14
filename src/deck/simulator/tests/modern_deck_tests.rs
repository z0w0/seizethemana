// Modern sweep: every Modern fixture must hold the shared invariants. The
// assertions check the simulator's mechanics, not deck quality.

use super::deck_test_support::*;

#[test]
fn sweep_modern_invariants() {
    for name in MODERN_FIXTURES {
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
        assert!(stats.land_count > 0, "{name} has no lands");
    }
}

#[test]
fn sweep_modern_storm_spends_everything() {
    // Ritual shells convert every drop of mana: Ruby Storm's average
    // unspent mana stays low across the mid turns — rituals pay for spells
    // the same turn they hit. The float appears late as the deck runs out
    // of gas; assert the early turns are nearly fully spent.
    let cards = fixture_cards("ruby-storm-2026");
    let deck = fixture_deck("ruby-storm-2026");
    let stats = sim(&deck, &cards, 200, 8);
    for t in 1..5.min(stats.unused_mana.len() - 1) {
        assert!(
            stats.unused_mana[t] <= 1.5,
            "storm floats {:.2} mana at t{}",
            stats.unused_mana[t],
            t + 1
        );
    }
}

#[test]
fn sweep_modern_cheat_decks_pay_full_price_or_never() {
    // Cheat decks (Neobrand, Broodscale) run 7+ MV finishers the deck
    // never plans to hard-cast. The sim must never mark them ready on a
    // curve: first-castable stays deep in the midgame even with the
    // ramp the shell runs (temples, labyrinths), and the target turn is
    // the on-curve ceil(cmc).
    for name in ["neobrand", "broodscale-combo"] {
        let cards = fixture_cards(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 8);
        let fat = stats
            .card_castability
            .iter()
            .filter(|c| c.cmc >= 7.0)
            .collect::<Vec<_>>();
        assert!(
            !fat.is_empty(),
            "{name} fixture has no 7+ MV rows (fixture error)"
        );
        for c in fat {
            // On-curve target: ceil of the discount-aware min cost.
            let floor =
                parse_sim_card(cards.get(&c.name).expect("castability row maps to fixture"))
                    .min_cost
                    .total();
            assert_eq!(
                c.target_turn, floor,
                "{name} {} target turn off-curve",
                c.name
            );
            assert!(
                c.avg_first_castable_turn >= f64::from(c.target_turn) - 5.0,
                "{name} {} (MV {}) castable avg t{:.2} well before curve: ramp model too generous",
                c.name,
                c.cmc,
                c.avg_first_castable_turn
            );
        }
    }
}

#[test]
fn sweep_modern_burn_casts_early() {
    // Boros LD and Boros Energy want cheap interaction online early.
    let cards = fixture_cards("boros-land-destruction");
    let deck = fixture_deck("boros-land-destruction");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.removal_access_5 >= 0.8,
        "Boros LD sees removal by t5 in {:.0}%",
        stats.removal_access_5 * 100.0
    );
}

#[test]
fn sweep_modern_color_screw_bounded() {
    // Mono-color and two-color decks trip at most one color at 10%.
    for name in ["ruby-storm-2026", "dimir-midrange-modern", "living-end"] {
        let cards = fixture_cards(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 8);
        let tripped = stats.color_screw.iter().filter(|p| **p >= 0.10).count();
        assert!(tripped <= 2, "{name} trips {} colors", tripped);
    }
}
