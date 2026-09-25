// Game-level assertions for the new mechanics: haste, landfall engines,
// planeswalker activations, X-costs, extra land drops, extra-turn drops,
// per-cast mana, upkeep drains, infinite-mana census, and the London
// mulligan policy alignment.

use super::aggregate::aggregate;
use super::game::run_game;
use super::game_tests::stub_deck;
use super::model::*;
use super::parse::*;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A minimal card row for tests.
fn card(name: &str, mana_cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: mana_cost.to_string(),
        cmc: parse_cost(mana_cost).total() as f64,
        type_line: type_line.to_string(),
        colors: "[]".into(),
        color_identity: "[]".into(),
        keywords: "[]".into(),
        power: None,
        toughness: None,
        loyalty: None,
        oracle_text: text.to_string(),
        rarity: "common".into(),
        edhrec_rank: None,
        legalities: "{}".into(),
        set_code: String::new(),
        collector_number: String::new(),
        scryfall_id: String::new(),
        released_at: String::new(),
        game_changer: None,
    }
}

#[test]
fn hasted_creature_attacks_entry_turn() {
    // One hasted one-drop in a land deck: bodies and attack power rise
    // the same turn it enters, not the turn after.
    let rows = [
        card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"),
        card("Swift Body", "{W}", "Creature — Human Soldier", "Haste"),
    ];
    let mut cards = Vec::new();
    for _ in 0..24 {
        cards.push(rows[0].clone());
    }
    for _ in 0..12 {
        cards.push(rows[1].clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 5)).collect();
    let stats = aggregate(&logs, &deck, 5);
    // Haste means the creature attacks the turn it enters; t2 attack
    // power must be non-zero in a healthy share of games.
    assert!(
        stats.attack_power_by_turn[1] > 0.5,
        "hasted one-drop should attack on t2, got {:.2}",
        stats.attack_power_by_turn[1]
    );
}

#[test]
fn landfall_engine_draws_on_land_drops() {
    // "Landfall — Whenever a land you control enters, draw a card." on a
    // cheap body: velocity rises with the land count.
    let landfall_body = card(
        "Tatyova-class",
        "{1}{G}",
        "Creature — Elemental",
        "Landfall — Whenever a land you control enters, you gain 1 life and draw a card.",
    );
    let mut cards = Vec::new();
    for _ in 0..28 {
        cards.push(card("Forest", "", "Basic Land — Forest", "({T}: Add {G}.)"));
    }
    for _ in 0..12 {
        cards.push(landfall_body.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // 6 turns = 6 land drops + the opener; each land drop draws. Cards
    // seen by t6 must beat the plain draw expectation (1/turn + 7).
    assert!(
        stats.cards_seen[5] > 12.0,
        "landfall engine should draw per land drop, got {:.2} by t6",
        stats.cards_seen[5]
    );
}

#[test]
fn planeswalker_fires_loyalty_and_gains() {
    // A walker with +1 draw and −3 draw: loyalty activates fire, the
    // ultimate (loyalty 6) becomes online in some games.
    let walker = card(
        "Test Walker",
        "{2}{W}",
        "Legendary Planeswalker — Test",
        "+1: Draw a card.\n−3: Draw two cards.\n−7: Draw five cards.",
    );
    let mut cmd = walker.clone();
    cmd.loyalty = Some("4".into());
    let mut cards = Vec::new();
    for _ in 0..35 {
        cards.push(card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"));
    }
    for _ in 0..12 {
        cards.push(card("Bear", "{2}", "Creature — Bear", "Vanilla."));
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![parse_sim_card(&cmd)],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // With +1 each turn from cast turn, loyalty 4 reaches 6+ within a
    // few turns in most games: ultimate_online must fire in >30%.
    assert!(
        stats.ultimate_online_pct > 0.30,
        "walker ultimate should flag online, got {:.2}",
        stats.ultimate_online_pct
    );
}

#[test]
fn x_spell_pays_leftover_and_drains() {
    // {X}{B}{B} drain spell in a mono-black land deck: once the board
    // floats mana, the X spell converts it to drain.
    let x_drain = card(
        "Torment-lite",
        "{X}{B}{B}",
        "Sorcery",
        "Target player loses X life.",
    );
    let mut cards = Vec::new();
    for _ in 0..26 {
        cards.push(card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)"));
    }
    for _ in 0..10 {
        cards.push(x_drain.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(13);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.drain_total_by_turn[7] > 10.0,
        "X drain spell should convert floated mana to drain, got {:.1}",
        stats.drain_total_by_turn[7]
    );
}

#[test]
fn extra_land_drop_engine_ramps() {
    // "You may play an additional land on each of your turns." on a
    // cheap creature: median drops by t4 exceed 4.
    let aesi_shape = card(
        "Aesi-lite",
        "{1}{G}{U}",
        "Creature — Merfolk",
        "You may play an additional land on each of your turns.",
    );
    let mut cards = Vec::new();
    for _ in 0..18 {
        cards.push(card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"));
        cards.push(card("Forest", "", "Basic Land — Forest", "({T}: Add {G}.)"));
    }
    for _ in 0..12 {
        cards.push(aesi_shape.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(17);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 5)).collect();
    let stats = aggregate(&logs, &deck, 5);
    assert!(
        stats.p50_drops_by_4 >= 5,
        "extra-land engine should push median drops by t4 past 4, got {}",
        stats.p50_drops_by_4
    );
}

#[test]
fn extra_turn_replays_land_drop_and_engines() {
    // "Take an extra turn" spell: the replayed turn records its land
    // drop in land_drops and keeps upkeep engines firing.
    let time_walk = card(
        "Twilight-lite",
        "{3}{U}",
        "Sorcery",
        "Take an extra turn after this one.",
    );
    let engine = card(
        "Upkeep Engine",
        "{2}{U}",
        "Enchantment",
        "At the beginning of your upkeep, draw a card.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"));
    }
    for _ in 0..6 {
        cards.push(time_walk.clone());
    }
    for _ in 0..4 {
        cards.push(engine.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(19);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.extra_turns_pct > 0.20,
        "extra turns should fire, got {:.2}",
        stats.extra_turns_pct
    );
}

#[test]
fn per_cast_mana_engine_feeds_pool() {
    // Vivi-class: "{T}: Add one mana of any color for each instant or
    // sorcery spell you've cast this turn." Cheap cantrips + this =
    // mana multiplication. Unused mana must stay low while cantrips last.
    let vivi = card(
        "Vivi-lite",
        "{1}{U}{R}",
        "Legendary Creature — Wizard",
        "{T}: Add one mana of any color for each instant or sorcery spell you've cast this turn.",
    );
    let cantrip = card("Cheap Draw", "{U}", "Instant", "Draw a card.");
    let mut cards = Vec::new();
    for _ in 0..28 {
        cards.push(card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"));
    }
    for _ in 0..2 {
        cards.push(vivi.clone());
    }
    for _ in 0..10 {
        cards.push(cantrip.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(23);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // The per-cast engine + cheap cantrips should keep the deck casting:
    // velocity by t6 must beat the plain 1/turn + 7 expectation clearly.
    assert!(
        stats.cards_seen[5] > 14.0,
        "per-cast mana should fuel extra casts, got {:.2} by t6",
        stats.cards_seen[5]
    );
}

#[test]
fn infinite_mana_engine_flags_census() {
    // Zero-cost untapped activation producing 1 mana per activation:
    // the pass caps it and flags the census.
    let engine = card(
        "Loop Rock",
        "{2}",
        "Artifact",
        "{T}: Add {C}.\n{0}: Add {C}.",
    );
    let mut cards = Vec::new();
    for _ in 0..20 {
        cards.push(card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)"));
    }
    for _ in 0..8 {
        cards.push(engine.clone());
    }
    for _ in 0..8 {
        cards.push(card("Bear", "{2}", "Creature — Bear", "Vanilla."));
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(29);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.infinite_mana_pct > 0.10,
        "zero-cost loop engine should flag the census, got {:.2}",
        stats.infinite_mana_pct
    );
}

#[test]
fn upkeep_drain_engine_resolves() {
    // "At the beginning of your upkeep, each opponent loses 1 life."
    // registers and drains per turn.
    let engine = card(
        "Pain Engine",
        "{2}{B}",
        "Enchantment",
        "At the beginning of your upkeep, each opponent loses 1 life.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)"));
    }
    for _ in 0..6 {
        cards.push(engine.clone());
    }
    for _ in 0..4 {
        cards.push(card("Bear", "{2}", "Creature — Bear", "Vanilla."));
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(31);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 7)).collect();
    let stats = aggregate(&logs, &deck, 7);
    // Constructed: x1 per engine per turn. 6 copies x ~5 turns.
    assert!(
        stats.drain_total_by_turn[6] > 2.0,
        "upkeep drain engine should resolve, got {:.1}",
        stats.drain_total_by_turn[6]
    );
}

#[test]
fn constructed_drain_is_x1_not_x3() {
    // A single burn spell in constructed drains its printed amount,
    // not the commander-family triple.
    let burn = card(
        "Burn",
        "{1}{R}",
        "Sorcery",
        "Deal 3 damage to target player.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(card(
            "Mountain",
            "",
            "Basic Land — Mountain",
            "({T}: Add {R}.)",
        ));
    }
    for _ in 0..6 {
        cards.push(burn.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(37);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // 6 copies x 3 damage, each cast once at most: total ≤ 18 avg.
    assert!(
        stats.drain_total_by_turn[7] <= 20.0,
        "constructed drain should be x1, got {:.1}",
        stats.drain_total_by_turn[7]
    );
}

#[test]
fn kicker_pays_from_spare_mana() {
    // Kicker {2} burn: when the pool covers the kick, the drain bumps.
    let kicked = card(
        "Kicked Bolt",
        "{1}{R}",
        "Sorcery",
        "Kicker {2}\nKicked Bolt deals 3 damage to target player.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(card(
            "Mountain",
            "",
            "Basic Land — Mountain",
            "({T}: Add {R}.)",
        ));
    }
    for _ in 0..8 {
        cards.push(kicked.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(41);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // Kicked casts drain 5, unkicked 3. Either way, drain happens.
    assert!(
        stats.drain_total_by_turn[7] > 3.0,
        "kicker burn should drain, got {:.1}",
        stats.drain_total_by_turn[7]
    );
}

#[test]
fn kicker_ignores_restricted_bucket_mana() {
    // Kicker {4} burn with mostly creature-only mana: bucket mana must
    // not fund the kick (only the general pool pays). With the old
    // pool.total() check the kick fires free and the drain bumps.
    let kicked = card(
        "Kicked Drain",
        "{1}{R}",
        "Sorcery",
        "Kicker {4}\nKicked Drain deals 2 damage to each opponent.",
    );
    let courtyard = card(
        "Secluded Courtyard",
        "",
        "Land",
        "As this land enters, choose a creature type.\n{T}: Add {C}.\n{T}: Add one mana of any color. Spend this mana only to cast a creature spell of the chosen type.",
    );
    let island = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let mut cards = Vec::new();
    for _ in 0..26 {
        cards.push(parse_sim_card(&courtyard));
    }
    for _ in 0..2 {
        cards.push(parse_sim_card(&island));
    }
    for _ in 0..8 {
        cards.push(parse_sim_card(&kicked));
    }
    let deck = SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(41);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    // The kicked drain pays 5 total from a general pool of 2 lands' mana:
    // by turn 6 the pool has at most ~5 general mana, so with the old
    // bucket-inflated check the drain compounded past the real spend.
    // With the fix, turn-5 drain stays at the unkicked rate mostly.
    let over_drain = logs.iter().filter(|log| log.drain_total[5] > 10).count();
    assert_eq!(
        over_drain, 0,
        "restricted-bucket mana must not fund a free kick: {over_drain}/200 games over-drain"
    );
}

#[test]
fn saga_chapters_fire_payoffs() {
    // Chapter III "Draw two cards" runs after two upkeep steps.
    let saga = card(
        "Tales of Learning",
        "{1}{U}",
        "Enchantment — Saga",
        "(As this Saga enters and after each of your upkeep steps, add a lore counter.)\nI — Draw a card.\nII — Draw a card.\nIII — Draw two cards.",
    );
    let mut cards = Vec::new();
    for _ in 0..34 {
        cards.push(card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"));
    }
    for _ in 0..6 {
        cards.push(saga.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(43);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 7)).collect();
    let stats = aggregate(&logs, &deck, 7);
    // 4 sagas entering early each draw 1+1+2 over four turns on top of
    // the normal draw.
    assert!(
        stats.cards_seen[6] > 13.0,
        "saga chapters should draw, got {:.2} by t7",
        stats.cards_seen[6]
    );
}

#[test]
fn token_count_etb_feeds_bodies() {
    // "create four …tokens" puts 4 bodies, not the legacy 2.
    let maker = card(
        "Quad Maker",
        "{3}{W}",
        "Creature — Soldier",
        "When Quad Maker enters, create four 1/1 Soldier creature tokens.",
    );
    let mut cards = Vec::new();
    for _ in 0..28 {
        cards.push(card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)"));
    }
    for _ in 0..12 {
        cards.push(maker.clone());
    }
    let deck = SimDeck {
        cards: cards.iter().map(parse_sim_card).collect(),
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(47);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // Each cast Quad Maker = 1 body + 4 tokens; bodies by t6 should
    // exceed what 2-token ETBs would give.
    assert!(
        stats.bodies_by_turn[5] > 3.0,
        "four-token ETB should fill the board, got {:.2}",
        stats.bodies_by_turn[5]
    );
}

#[test]
fn loyalty_token_plus1_counts_as_engine() {
    // A planeswalker +1 that creates tokens is a repeatable once-per-turn
    // engine: it registers in engines_online the turn after the first
    // activation (Liliana, Dreadhorde General class).
    let mut deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let mut row = card(
        "Walker",
        "{3}{W}{W}",
        "Legendary Planeswalker — Human",
        "+1: Create a 2/2 black Zombie creature token.",
    );
    row.loyalty = Some("4".to_string());
    let walker = parse_sim_card(&row);
    assert!(
        walker
            .abilities()
            .any(|a| a.loyalty_gain == 1 && matches!(a.effect, Effect::Tokens(_)))
    );
    deck.cards.push(walker);
    let mut rng = ChaCha8Rng::seed_from_u64(89);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 10)).collect();
    let stats = aggregate(&logs, &deck, 10);
    // Token engines only exist in games where the walker was drawn and
    // cast (1 copy in 66); a low bar: some games show an engine online.
    assert!(
        stats.engines_by_turn[9] > 0.0,
        "loyalty +1 token engines should register: {:.3}",
        stats.engines_by_turn[9]
    );
}

// 5.4 mulligan policy alignment: Karsten's London model.

#[test]
fn london_policy_lowers_screw_vs_ship_on_zero() {
    // The Karsten policy (redraw 0/1-land openers, bottom toward 3)
    // should drop the screw rate for a 24-land deck vs shipping only
    // zero-land openers: same deck, fewer ≤2-land-by-t4 games. The
    // spell list fills the deck to its real size.
    let deck = stub_deck(24, &[("Bear", 2, Role::Other); 5]);
    let mut rng = ChaCha8Rng::seed_from_u64(21);
    let logs: Vec<_> = (0..400).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // 24/60: P(0-land opener) ≈ 6% + P(1 land) ≈ 20%: the redraw band
    // rescues roughly a quarter of would-be screw games. The screw rate
    // must sit clearly below the no-mulligan baseline (~20% for 24 lands
    // at 2-or-fewer by t4).
    assert!(
        stats.screw_pct < 0.16,
        "24-land screw rate too high under Karsten policy: {:.3}",
        stats.screw_pct
    );
    assert!(
        stats.hit_all_drops_by[4] > 0.55,
        "24-land deck should hit 4 drops reasonably: {:.2}",
        stats.hit_all_drops_by[4]
    );
}

#[test]
fn sparse_deck_still_reports_screw() {
    // A 17-land deck has no mulligan rescue for 2-land openers: the
    // screw rate stays high. The spell list fills the deck to ~60 cards
    // so the land ratio is the real one.
    let deck = stub_deck(17, &[("Bear", 2, Role::Other); 6]);
    let mut rng = ChaCha8Rng::seed_from_u64(21);
    let logs: Vec<_> = (0..400).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.screw_pct > 0.20,
        "17-land deck must still read as screwed: {:.3}",
        stats.screw_pct
    );
}
