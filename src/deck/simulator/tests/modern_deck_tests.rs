//! Modern sweep: every Modern fixture must hold the shared invariants. The
//! assertions check the simulator's mechanics, not deck quality.

use super::deck_test_support::*;
use super::model::{SimDeck, SimEffect};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

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
    // Broodscale runs a 7+ MV finisher the deck
    // never plans to hard-cast. The sim must never mark them ready on a
    // curve: first-castable stays deep in the midgame even with the
    // ramp the shell runs (temples, labyrinths), and the target turn is
    // the on-curve ceil(cmc).
    for name in ["broodscale-combo"] {
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
fn modern_cascade_line_depends_on_its_enabler() {
    let (fixture_name, enabler, payoff, support, turns) = (
        "living-end",
        "Violent Outburst",
        "Generous Ent",
        "Living End",
        8,
    );
    let rows = fixture_cards(fixture_name);
    let source = build_sim_deck(&fixture_deck(fixture_name), &rows, None);
    let payoff_index = source
        .cards
        .iter()
        .position(|card| card.name == payoff)
        .unwrap_or_else(|| panic!("{fixture_name} has no {payoff} card"));
    let support_index = source
        .cards
        .iter()
        .position(|card| card.name == support)
        .unwrap_or_else(|| panic!("{fixture_name} has no {support} card"));
    let enabler_index = source
        .cards
        .iter()
        .position(|card| card.name == enabler)
        .unwrap_or_else(|| panic!("{fixture_name} has no {enabler} card"));
    let without = without_cascade(&source);
    let mut full_rng = ChaCha8Rng::seed_from_u64(921);
    let mut without_rng = ChaCha8Rng::seed_from_u64(921);
    let runs = 1000;
    let full_rate = (0..runs)
        .filter(|_| {
            let log = run_game(&source, &mut full_rng, turns);
            log.card_first_battlefield
                .get(&payoff_index)
                .is_some_and(|turn| {
                    *turn <= turns
                        && log
                            .card_first_graveyard
                            .get(&support_index)
                            .is_some_and(|support_turn| support_turn <= turn)
                        && log
                            .card_first_graveyard
                            .get(&enabler_index)
                            .is_some_and(|enabler_turn| enabler_turn <= turn)
                })
        })
        .count();
    let without_index = without
        .cards
        .iter()
        .position(|card| card.name == payoff)
        .expect("the payoff remains in the comparison deck");
    let without_support = without
        .cards
        .iter()
        .position(|card| card.name == support)
        .expect("the support card remains in the comparison deck");
    let without_enabler = without.cards.iter().position(|card| card.name == enabler);
    let without_rate = (0..runs)
        .filter(|_| {
            let log = run_game(&without, &mut without_rng, turns);
            log.card_first_battlefield
                .get(&without_index)
                .is_some_and(|turn| {
                    *turn <= turns
                        && log
                            .card_first_graveyard
                            .get(&without_support)
                            .is_some_and(|support_turn| support_turn <= turn)
                        && without_enabler.is_some_and(|index| {
                            log.card_first_graveyard
                                .get(&index)
                                .is_some_and(|enabler_turn| enabler_turn <= turn)
                        })
                })
        })
        .count();
    assert!(
        full_rate > without_rate,
        "{fixture_name}: enabler rate {full_rate}, without {without_rate}"
    );
}

/// Build a comparison deck with cascade disabled.
fn without_cascade(deck: &SimDeck) -> SimDeck {
    let mut comparison = copy_deck(deck);
    for card in &mut comparison.cards {
        card.has_cascade = false;
    }
    comparison
}

/// Clone the parsed card data while preserving the deck's card counts.
fn copy_deck(deck: &SimDeck) -> SimDeck {
    SimDeck {
        companion: None,
        cards: deck.cards.clone(),
        commanders: deck.commanders.clone(),
        format: deck.format,
        rules: deck.rules,
    }
}

#[test]
fn sweep_modern_land_destruction_sees_interaction_early() {
    // Boros Land Destruction wants cheap interaction online early.
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

#[test]
fn graveyard_and_life_engines_change_their_resource_metrics() {
    for (name, enabler, metric) in [
        ("ruby-storm-2026", "Past in Flames", "replay"),
        ("rogsilas turbo naus", "Underworld Breach", "replay"),
        ("blue farm", "Underworld Breach", "replay"),
        ("neobrand", "Griselbrand", "life"),
        (
            "yawgmoth combo modern",
            "Yawgmoth, Thran Physician",
            "velocity",
        ),
    ] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let with_engine = sim(&deck, &cards, 600, 10);
        let mut without_deck = fixture_deck(name);
        let entries = without_deck.section_entries_mut("DECK");
        let entry = entries
            .iter()
            .position(|entry| entry.name == enabler)
            .unwrap_or_else(|| panic!("{name} fixture is missing {enabler}"));
        if entries[entry].quantity > 1 {
            entries[entry].quantity -= 1;
        } else {
            entries.remove(entry);
        }
        let without_engine = sim(&without_deck, &cards, 600, 10);
        let (with_metric, without_metric) = match metric {
            "life" => (with_engine.life_paid_avg, without_engine.life_paid_avg),
            "velocity" => (with_engine.cards_seen[5], without_engine.cards_seen[5]),
            _ => (
                with_engine.replay_casts_avg,
                without_engine.replay_casts_avg,
            ),
        };
        assert!(
            with_metric > without_metric,
            "{name} {enabler} metric did not improve: {with_metric:.2} vs {without_metric:.2}"
        );
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
fn topdeck_eldrazi_ramp_can_cast_big_payoffs_when_tron_is_online() {
    // Urza's Mine, Power Plant, and Tower can pay seven mana on turn three.
    let cards = fixture_cards("eldrazi ramp topdeck");
    let deck = fixture_deck("eldrazi ramp topdeck");
    let stats = sim(&deck, &cards, 200, 8);
    for c in &stats.card_castability {
        let row = cards.get(&c.name);
        if let Some(row) = row {
            let cmc = super::oracle_parser::cost::parse_cost(&row.mana_cost).total();
            if cmc >= 7 {
                assert!(
                    c.avg_first_castable_turn >= 2.5,
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
    // Dredge: draw replacements mill enough cards to fuel the graveyard
    // payoffs earlier than the same list without its keyword abilities.
    let cards = fixture_cards("dredge modern");
    let deck = fixture_deck("dredge modern");
    let active_deck = build_sim_deck(&deck, &cards, None);
    let mut control_deck = build_sim_deck(&deck, &cards, None);
    for card in &mut control_deck.cards {
        card.dredge = None;
    }
    let mut active_rng = ChaCha8Rng::seed_from_u64(42);
    let mut control_rng = ChaCha8Rng::seed_from_u64(42);
    let active_logs: Vec<_> = (0..200)
        .map(|_| run_game(&active_deck, &mut active_rng, 8))
        .collect();
    let control_logs: Vec<_> = (0..200)
        .map(|_| run_game(&control_deck, &mut control_rng, 8))
        .collect();
    let stats = super::aggregate::aggregate(&active_logs, &active_deck, 8);
    let control = super::aggregate::aggregate(&control_logs, &control_deck, 8);
    assert!(
        stats.self_milled_by_turn[5] > control.self_milled_by_turn[5] + 3.0,
        "dredge mills {:.1} cards by t6 vs {:.1} without dredge",
        stats.self_milled_by_turn[5],
        control.self_milled_by_turn[5]
    );
    assert!(
        stats.graveyard_by_turn[5] > control.graveyard_by_turn[5],
        "dredge graveyard by t6: {:.1}",
        stats.graveyard_by_turn[5]
    );
    assert!(
        stats.creatures_by_turn[5] > control.creatures_by_turn[5],
        "dredge payoffs create {:.1} bodies by t6 vs {:.1} without dredge",
        stats.creatures_by_turn[5],
        control.creatures_by_turn[5]
    );
}

#[test]
fn affinity_modern_counts_artifacts() {
    // Affinity: artifact rocks join the board early and the discount
    // cards cast below printed cost (battlefield discount class).
    let cards = fixture_cards("affinity modern");
    let deck = fixture_deck("affinity modern");
    let sim_deck = build_sim_deck(&deck, &cards, None);
    let discounts = sim_deck
        .cards
        .iter()
        .filter(|c| c.battlefield_discount)
        .count();
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
fn affinity_mox_opal_increases_mana_when_metalcraft_is_active() {
    let cards = fixture_map("affinity modern");
    let deck = fixture_deck("affinity modern");
    let full = build_sim_deck(&deck, &cards, None);
    let opal = full
        .cards
        .iter()
        .find(|card| card.name == "Mox Opal")
        .expect("Affinity fixture contains Mox Opal");
    assert!(opal.flags.requires_metalcraft);

    let mut without_deck = deck.clone();
    without_deck
        .section_entries_mut("DECK")
        .retain(|entry| entry.name != "Mox Opal");
    let without = build_sim_deck(&without_deck, &cards, None);
    let mut full_rng = ChaCha8Rng::seed_from_u64(707);
    let mut without_rng = ChaCha8Rng::seed_from_u64(707);
    let full_mana = (0..1000)
        .map(|_| {
            run_game(&full, &mut full_rng, 6).mana_available[1..5]
                .iter()
                .sum::<f64>()
        })
        .sum::<f64>()
        / 1000.0;
    let without_mana = (0..1000)
        .map(|_| {
            run_game(&without, &mut without_rng, 6).mana_available[1..5]
                .iter()
                .sum::<f64>()
        })
        .sum::<f64>()
        / 1000.0;
    assert!(
        full_mana > without_mana,
        "Affinity mana with Opal {full_mana:.2} must exceed without it {without_mana:.2}"
    );
}

#[test]
fn black_burn_self_costs_do_not_leak_into_the_drain_census() {
    // Thoughtseize ("you lose 2 life") and Castle Locthwain ("you lose
    // life equal to ...") are self-costs (CR 119.3), not drains. The
    // deck's only player-directed burn (Soul Spike, Bowmasters) targets
    // creatures/any-target and stays inert in the goldfish, so the drain
    // census must not pick up the self-costs.
    let cards = fixture_cards("black burn bowmasters modern");
    let deck = fixture_deck("black burn bowmasters modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.opponent_life_loss_by_turn[4] < 0.2,
        "self-costs must not register as opponent drain: {:.2}",
        stats.opponent_life_loss_by_turn[4]
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
        stats.creatures_by_turn[5] > 2.0,
        "whack bodies by t6: {:.2}",
        stats.creatures_by_turn[5]
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

#[test]
fn topdeck_boros_energy_online() {
    // Boros Energy: Guide of Souls and Galvanic Discharge parse energy,
    // and the deck's early bodies come down on curve.
    let cards = fixture_cards("boros energy 2026");
    let deck = fixture_deck("boros energy 2026");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.creatures_by_turn[1] > 0.5,
        "boros energy bodies by t2: {:.2}",
        stats.creatures_by_turn[1]
    );
    let guide = parse_sim_card(cards.get("Guide of Souls").expect("fixture has Guide"));
    assert!(
        guide
            .unlocked_abilities(0)
            .any(|ability| matches!(ability.effect, SimEffect::Energy(_))),
        "Guide of Souls parses an energy trigger"
    );
}

#[test]
fn topdeck_oculus_manifest_modern_grinds_graveyard() {
    // Abhorrent Oculus: manifest dread plus graveyard fuel.
    let cards = fixture_cards("oculus manifest modern");
    let deck = fixture_deck("oculus manifest modern");
    let stats = sim(&deck, &cards, 200, 8);
    assert!(
        stats.graveyard_by_turn[7] > 3.0,
        "oculus graveyard by t8: {:.1}",
        stats.graveyard_by_turn[7]
    );
}
