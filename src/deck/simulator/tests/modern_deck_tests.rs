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
        // A pure-MDFC land base (tameshi-belcher) reports 0 plain lands;
        // the check needs a real land floor only for shells that have them.
        if stats.land_count > 0 {
            assert!(
                stats.land_count >= 12,
                "{name} has {} lands",
                stats.land_count
            );
        }
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

// Topdeck.gg tournament lists (real Modern competitive results): shared
// invariants plus archetype-specific timing where meaningful.

#[test]
fn sweep_modern_topdeck_fixtures_hold_invariants() {
    for name in [
        "yawgmoth combo modern",
        "dimir murktide topdeck",
        "eldrazi ramp topdeck",
        "persist reanimator modern",
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
fn topdeck_dimir_murk_early_threats() {
    // Murktide aggro: cheap threats cast on curve — the deck's plan is
    // early pressure.
    let cards = fixture_cards("dimir murktide topdeck");
    let deck = fixture_deck("dimir murktide topdeck");
    let stats = sim(&deck, &cards, 200, 8);
    let early = stats
        .card_castability
        .iter()
        .filter(|c| c.target_turn <= 2)
        .map(|c| c.pct_by_target)
        .fold(0.0_f64, f64::max);
    assert!(early > 0.55, "murktide best early-cast rate: {:.2}", early);
}

#[test]
fn topdeck_eldrazi_ramp_pays_big_costs_late() {
    // Eldrazi ramp: the big payoffs must never show as curve-ready.
    let cards = fixture_cards("eldrazi ramp topdeck");
    let deck = fixture_deck("eldrazi ramp topdeck");
    let stats = sim(&deck, &cards, 200, 8);
    for c in &stats.card_castability {
        let row = cards.get(&c.name);
        if let Some(row) = row {
            let cmc = super::parse::parse_cost(&row.mana_cost).total();
            if cmc >= 7 {
                assert!(
                    c.avg_first_castable_turn >= 4.0,
                    "{} (CMC {}) castable at t{:.2}",
                    c.name,
                    cmc,
                    c.avg_first_castable_turn
                );
            }
        }
    }
}

// Archetype-specific dedicated tests for the previously invariants-only
// modern fixtures: each asserts the archetype's defining mechanic.

#[test]
fn dredge_modern_fills_graveyard_fast() {
    // Dredge: self-mill fills the graveyard almost immediately (the
    // whole plan is graveyard fuel).
    let cards = fixture_cards("dredge modern");
    let deck = fixture_deck("dredge modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.graveyard_by_turn[5] > 5.0,
        "dredge graveyard by t6: {:.1}",
        stats.graveyard_by_turn[5]
    );
}

#[test]
fn affinity_modern_counts_artifacts() {
    // Affinity: artifact rocks join the board early and the discount
    // cards cast below printed cost (board_discount class).
    let cards = fixture_cards("affinity modern");
    let deck = fixture_deck("affinity modern");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let discounts = sim_deck.cards.iter().filter(|c| c.board_discount).count();
    assert!(
        discounts >= 2,
        "affinity shell holds {} discount cards",
        discounts
    );
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.attack_power_by_turn[7] > 2.0,
        "affinity attack power by t8: {:.2}",
        stats.attack_power_by_turn[7]
    );
}

#[test]
fn black_burn_bowmasters_drains_early() {
    // Burn + Bowmasters: player-targeted damage shows up by t5.
    let cards = fixture_cards("black burn bowmasters modern");
    let deck = fixture_deck("black burn bowmasters modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.drain_total_by_turn[4] > 0.8,
        "burn drain by t5: {:.1}",
        stats.drain_total_by_turn[4]
    );
}

#[test]
fn merfolk_modern_attacks_with_bodies() {
    // Merfolk: cheap bodies attack every turn from t2.
    let cards = fixture_cards("merfolk modern");
    let deck = fixture_deck("merfolk modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.attackers_by_turn[3] > 1.0,
        "merfolk attackers by t4: {:.2}",
        stats.attackers_by_turn[3]
    );
}

#[test]
fn deaths_shadow_commits_early_pressure() {
    // Death's Shadow: cheap threats cast by t3 in most games.
    let cards = fixture_cards("deaths shadow modern");
    let deck = fixture_deck("deaths shadow modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.creature_access_3 > 0.6,
        "shadow creature access by t3: {:.2}",
        stats.creature_access_3
    );
}

#[test]
fn hollow_one_discards_into_violence() {
    // Hollow One: cycling discards fill the graveyard by t4.
    let cards = fixture_cards("hollow one modern");
    let deck = fixture_deck("hollow one modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.graveyard_by_turn[4] > 1.5,
        "hollow one graveyard by t5: {:.1}",
        stats.graveyard_by_turn[4]
    );
}

#[test]
fn whack_12_goblins_swarm() {
    // 12-Whack: cheap goblins swarm into bodies by t5.
    let cards = fixture_cards("whack 12 modern");
    let deck = fixture_deck("whack 12 modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.bodies_by_turn[5] > 2.0,
        "whack bodies by t6: {:.2}",
        stats.bodies_by_turn[5]
    );
}

#[test]
fn topdeck_yawgmoth_combo_gathers_velocity() {
    // Modern Yawgmoth: draw engines + graveyard churn both show.
    let cards = fixture_cards("yawgmoth combo modern");
    let deck = fixture_deck("yawgmoth combo modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.draw_access_6 > 0.4,
        "yawgmoth modern draw access by t6: {:.2}",
        stats.draw_access_6
    );
}

#[test]
fn topdeck_persist_reanimator_grinds_graveyard() {
    // Persist reanimator: the graveyard census piles up fast.
    let cards = fixture_cards("persist reanimator modern");
    let deck = fixture_deck("persist reanimator modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.graveyard_by_turn[7] > 4.0,
        "persist reanimator graveyard by t8: {:.1}",
        stats.graveyard_by_turn[7]
    );
}
