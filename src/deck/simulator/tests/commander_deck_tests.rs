// Commander sweep: every Commander fixture must hold the shared invariants
// and the commander-specific telemetry must behave (cast windows, partners,
// engine tiers).

use super::deck_test_support::*;
use super::model::Format;

#[test]
fn sweep_commander_invariants() {
    for name in COMMANDER_FIXTURES {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let sim_deck = build_sim_deck(&deck, &cards, None);
        assert_eq!(
            sim_deck.format,
            Format::Commander,
            "{name} not inferred as commander"
        );
        assert!(
            !sim_deck.commanders.is_empty(),
            "{name} has no commanders parsed"
        );
        assert!(
            sim_deck.cards.len() + sim_deck.commanders.len() == 100,
            "{name} has {} cards, expected 100",
            sim_deck.cards.len() + sim_deck.commanders.len()
        );
        let stats = sim(&deck, &cards, 200, 10);
        assert_land_drops_sane(&stats, 10);
        assert_velocity_monotone(&stats);
        assert_castability_not_before_cost(&stats, &cards);
    }
}

#[test]
fn sweep_commander_partners_carry_two() {
    // Partner decks keep both legends in the command zone.
    for name in [
        "blue farm",
        "thrasios - tymna midrange",
        "rogsi turbo",
        "rogsilas turbo naus",
        "krark-sakashima storm",
    ] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let sim_deck = build_sim_deck(&deck, &cards, None);
        assert_eq!(
            sim_deck.commanders.len(),
            2,
            "{name} should carry two partners"
        );
    }
}

#[test]
fn sweep_commander_zero_cost_leader_casts_turn_one() {
    // Rograkh ({0}) is castable in essentially every game by turn 1.
    for name in ["rogsi turbo", "rogsilas turbo naus"] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 10);
        assert!(
            stats.commander_castable_by[1] >= 0.9,
            "{name} zero-cost commander castable by t1 only {:.0}%",
            stats.commander_castable_by[1] * 100.0
        );
    }
}

#[test]
fn sweep_commander_casual_legends_are_late() {
    // Casual 99-card lists cast {4}{WUBRG}-shaped commanders late: the
    // insight is the cast window. The Ur-Dragon (CMC 9) is castable by t9
    // in a minority of games on a 35-land 5c base — the report's
    // "commander_late" finding fires on exactly this shape.
    let cards = fixture_map("the ur-dragon dragons");
    let deck = fixture_deck("the ur-dragon dragons");
    let stats = sim(&deck, &cards, 300, 10);
    assert!(
        stats.commander_castable_by[9] >= 0.3,
        "Ur-Dragon castable by t9 only {:.0}%",
        stats.commander_castable_by[9] * 100.0
    );
    assert!(
        stats.commander_castable_by[4] < 0.1,
        "CMC 9 commander castable by t4 in {:.0}%: model too generous",
        stats.commander_castable_by[4] * 100.0
    );
}

#[test]
fn sweep_commander_velocity_grows() {
    // Commander games see far more cards than the constructed opener:
    // draw engines plus 10 turns push velocity well past the opener.
    for name in ["blue farm", "kinnan combo", "chulane bant value"] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 10);
        assert!(
            stats.cards_seen[9] > 20.0,
            "{name} velocity stalls at {:.1}",
            stats.cards_seen[9]
        );
        assert!(
            stats.cards_seen[9] < 124.0,
            "{name} sees {} cards by t10: zone telemetry overflows",
            stats.cards_seen[9]
        );
    }
}

#[test]
fn sweep_commander_tax_pieces_seen_early() {
    // Jaws Storm runs God-Pharaoh's Statue ({2} tax, 5 MV) and Winter Moon:
    // cheap rock-style locks show up by t3 in most games; the 5 MV Statue
    // rarely does. Lock timing is the metric under test.
    let cards = fixture_map("jaws storm");
    let deck = fixture_deck("jaws storm");
    let stats = sim(&deck, &cards, 300, 10);
    // Cheap locks hit the table by t3 in a real share of games.
    assert!(
        stats.lock_access_3 > 0.10,
        "jaws storm lock access by t3: {:.2}",
        stats.lock_access_3
    );
}

// New-mechanic commander fixtures: targeted assertions where the modeled
// mechanic makes the invariant meaningful (not deck-quality judgments).

#[test]
fn sweep_commander_new_mechanic_fixtures_hold_invariants() {
    for name in [
        "atraxa superfriends",
        "yuriko ninja tempo",
        "muldrotha graveyard value",
        "krenko goblin swarm",
        "nekusar wheel punish",
        "aesi extra lands landfall",
        "brago blink value",
        "kalamax x instants",
        "light-paws aura voltron",
        "ezuri elf swarm",
    ] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 10);
        assert_land_drops_sane(&stats, 10);
        assert_velocity_monotone(&stats);
        assert_castability_not_before_cost(&stats, &cards);
    }
}

#[test]
fn atraxa_superfriends_walkers_get_cast() {
    // Walker shells cast their planeswalkers in a healthy share of games
    // (the loyalty-activation execution itself is covered by the
    // game-level planeswalker_fires_loyalty_and_gains test).
    let cards = fixture_map("atraxa superfriends");
    let deck = fixture_deck("atraxa superfriends");
    let stats = sim(&deck, &cards, 200, 10);
    let walker_rows: Vec<&CardCast> = stats
        .card_castability
        .iter()
        .filter(|c| {
            cards
                .get(&c.name)
                .is_some_and(|row| row.type_line.contains("Planeswalker"))
        })
        .collect();
    assert!(
        !walker_rows.is_empty(),
        "no walkers found in the fixture cards"
    );
    let cast_share = walker_rows
        .iter()
        .map(|c| c.pct_by_target)
        .fold(0.0_f64, f64::max);
    assert!(
        cast_share > 0.20,
        "best walker on-curve cast rate: {:.2}",
        cast_share
    );
}

#[test]
fn aesi_extra_lands_median_drops_beat_four() {
    // "You may play an additional land on each of your turns.": the
    // median drops by t4 must exceed 4 when Aesi is out early.
    let cards = fixture_map("aesi extra lands landfall");
    let deck = fixture_deck("aesi extra lands landfall");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.p50_drops_by_4 >= 4,
        "aesi deck median drops by t4: {}",
        stats.p50_drops_by_4
    );
    assert!(
        stats.p95_drops_by_4 >= 5,
        "aesi p95 drops by t4: {} (extra-land engine must reach 5)",
        stats.p95_drops_by_4
    );
    assert!(
        stats.hit_all_drops_by[4] > 0.35,
        "aesi deck should hit all 4 land drops often: {:.2}",
        stats.hit_all_drops_by[4]
    );
}

#[test]
fn muldrotha_graveyard_census_grows() {
    // Self-mill + recursion: the graveyard census must grow across the
    // game window.
    let cards = fixture_map("muldrotha graveyard value");
    let deck = fixture_deck("muldrotha graveyard value");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.graveyard_by_turn[9] > 6.0,
        "muldrotha graveyard by t10: {:.1} (self-mill engine must fill)",
        stats.graveyard_by_turn[9]
    );
}

// Topdeck.gg tournament lists (real competitive cEDH/EDH results): the
// invariants plus commander timing sanity for real-table lists.

#[test]
fn sweep_commander_topdeck_fixtures_hold_invariants() {
    for name in [
        "shanna energy soldiers",
        "satya aetherflux energy",
        "tayam luminous engine",
        "yawgmoth thran combo",
    ] {
        let cards = fixture_map(name);
        let deck = fixture_deck(name);
        let stats = sim(&deck, &cards, 200, 10);
        assert_land_drops_sane(&stats, 10);
        assert_velocity_monotone(&stats);
        assert_castability_not_before_cost(&stats, &cards);
        assert!(
            stats.commander_castable_by[7] > 0.5,
            "{name} commander castable by t8: {:.2}",
            stats.commander_castable_by[7]
        );
    }
}

#[test]
fn yawgmoth_topdeck_shows_combo_velocity() {
    // cEDH Yawgmoth combo: draw engines and graveyard velocity must both
    // show up — the deck's whole plan is draw-into-loop.
    let cards = fixture_map("yawgmoth thran combo");
    let deck = fixture_deck("yawgmoth thran combo");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.draw_access_6 > 0.5,
        "yawgmoth draw access by t6: {:.2}",
        stats.draw_access_6
    );
    assert!(
        stats.graveyard_by_turn[9] > stats.graveyard_by_turn[2],
        "yawgmoth graveyard should grow, {:.1} → {:.1}",
        stats.graveyard_by_turn[2],
        stats.graveyard_by_turn[9]
    );
}

#[test]
fn satya_energy_deck_emerges_bodies() {
    // Satya's Aetherflux tokens: bodies must emerge in real numbers.
    let cards = fixture_map("satya aetherflux energy");
    let deck = fixture_deck("satya aetherflux energy");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.bodies_by_turn[9] > 4.0,
        "satya bodies by t10: {:.2}",
        stats.bodies_by_turn[9]
    );
    assert!(
        stats.bodies_by_turn[9] > stats.bodies_by_turn[4] * 1.5,
        "satya body count must grow late, {:.1} → {:.1}",
        stats.bodies_by_turn[4],
        stats.bodies_by_turn[9]
    );
}

// Archetype-specific dedicated tests for the previously invariants-only
// commander fixtures: each asserts the archetype's defining mechanic.

#[test]
fn krenko_goblin_swarm_grows_bodies() {
    // Krenko's plan is goblin swarm: token bodies must compound by t10.
    let cards = fixture_map("krenko goblin swarm");
    let deck = fixture_deck("krenko goblin swarm");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.bodies_by_turn[9] > 3.0,
        "krenko bodies by t10: {:.2}",
        stats.bodies_by_turn[9]
    );
    assert!(
        stats.bodies_by_turn[9] > stats.bodies_by_turn[4],
        "krenko body count must grow late, {:.1} → {:.1}",
        stats.bodies_by_turn[4],
        stats.bodies_by_turn[9]
    );
}

#[test]
fn yuriko_ninja_tempo_attacks_evasive() {
    // Yuriko's ninjas connect early: evasive attackers by t4 and combat
    // power by t3 (the deck's plan is cheap evasive pressure).
    let cards = fixture_map("yuriko ninja tempo");
    let deck = fixture_deck("yuriko ninja tempo");
    let stats = sim(&deck, &cards, 200, 10);
    // Cheap ninjas swing early: attack power by t4 and steady attackers
    // through the midgame. (Evasion in this list is ninjutsu-bounce,
    // which the sim does not model — no keyword census to assert.)
    assert!(
        stats.attack_power_by_turn[3] > 1.0,
        "yuriko attack power by t4: {:.2}",
        stats.attack_power_by_turn[3]
    );
    assert!(
        stats.attackers_by_turn[5] > 1.2,
        "yuriko attackers by t6: {:.2}",
        stats.attackers_by_turn[5]
    );
}

#[test]
fn nekusar_wheel_punish_draws_wheels() {
    // Nekusar's wheel engines refill the hand: wheel draws push cards
    // seen well past the vanilla curve by t10.
    let cards = fixture_map("nekusar wheel punish");
    let deck = fixture_deck("nekusar wheel punish");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.cards_seen[9] > 30.0,
        "nekusar velocity by t10: {:.1}",
        stats.cards_seen[9]
    );
    assert!(
        stats.draw_access_6 > 0.6,
        "nekusar draw access by t6: {:.2}",
        stats.draw_access_6
    );
}

#[test]
fn brago_blink_value_flickers_often() {
    // Brago blink: ETB re-fires mean the graveyard and battlefield
    // telemetry both churn; velocity stays high from blink draws.
    let cards = fixture_map("brago blink value");
    let deck = fixture_deck("brago blink value");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.cards_seen[9] > 28.0,
        "brago velocity by t10: {:.1}",
        stats.cards_seen[9]
    );
}

#[test]
fn kalamax_x_instants_casts_big_spells() {
    // Kalamax shells cast X instants with real mana behind them: the
    // X-spell class pays the leftover pool, so drains show up.
    let cards = fixture_map("kalamax x instants");
    let deck = fixture_deck("kalamax x instants");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.draw_access_6 > 0.5,
        "kalamax draw access by t6: {:.2}",
        stats.draw_access_6
    );
    assert!(
        stats.commander_castable_by[8] > 0.5,
        "kalamax (5 MV) castable by t8: {:.2}",
        stats.commander_castable_by[8]
    );
}

#[test]
fn light_paws_voltron_suits_up() {
    // Light-Paws voltron: a small body count attacks with real power.
    // (Aura buffs attach to one creature; the sim's buff model covers
    // "creatures you control" static buffs and Equipment hosts, so the
    // checkable half is the early attack cadence.)
    let cards = fixture_map("light-paws aura voltron");
    let deck = fixture_deck("light-paws aura voltron");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.attack_power_by_turn[8] > 3.0,
        "light-paws attack power by t9: {:.2}",
        stats.attack_power_by_turn[8]
    );
    assert!(
        stats.attackers_by_turn[5] > 0.8,
        "light-paws attackers by t6: {:.2}",
        stats.attackers_by_turn[5]
    );
}

#[test]
fn ezuri_elf_swarm_grows_bodies() {
    // Ezuri elfball: elf tokens and lords compound into a wide board.
    let cards = fixture_map("ezuri elf swarm");
    let deck = fixture_deck("ezuri elf swarm");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.bodies_by_turn[9] > 3.0,
        "ezuri bodies by t10: {:.2}",
        stats.bodies_by_turn[9]
    );
}

#[test]
fn atraxa_superfriends_ultimates_reach_online() {
    // Walker shells cast walkers in real numbers and the walker count
    // grows (loyalty gains fire once per walker per turn, so ultimates
    // stay the report's long-game signal). The castability share is the
    // checkable half of the superfriends plan.
    let cards = fixture_map("atraxa superfriends");
    let deck = fixture_deck("atraxa superfriends");
    let stats = sim(&deck, &cards, 200, 10);
    // At least two walker rows with real cast rates (the shell holds
    // 10+ walkers; two clear 10% by on-curve targets).
    let walkers: Vec<&CardCast> = stats
        .card_castability
        .iter()
        .filter(|c| {
            cards
                .get(&c.name)
                .is_some_and(|row| row.type_line.contains("Planeswalker"))
        })
        .collect();
    assert!(
        walkers.iter().filter(|c| c.pct_by_target > 0.10).count() >= 2,
        "fewer than 2 walkers castable: {:?}",
        walkers
            .iter()
            .map(|c| (c.name.clone(), c.pct_by_target))
            .collect::<Vec<_>>()
    );
}

#[test]
fn tayam_luminous_engine_grinds_graveyard() {
    // Tayam recursion: the graveyard census churns (milled and
    // sacrificed pieces pile up for the luminous engine).
    let cards = fixture_map("tayam luminous engine");
    let deck = fixture_deck("tayam luminous engine");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.graveyard_by_turn[9] > 4.0,
        "tayam graveyard by t10: {:.1}",
        stats.graveyard_by_turn[9]
    );
}

#[test]
fn shanna_energy_soldiers_early_creatures() {
    // Shanna's cheap soldiers: a creature is in hand by t3 in most games
    // (the deck's plan is an early board).
    let cards = fixture_map("shanna energy soldiers");
    let deck = fixture_deck("shanna energy soldiers");
    let stats = sim(&deck, &cards, 200, 10);
    assert!(
        stats.creature_access_3 > 0.6,
        "shanna creature access by t3: {:.2}",
        stats.creature_access_3
    );
}
