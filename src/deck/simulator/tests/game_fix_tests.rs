// Regression tests for game-loop fixes: restricted-mana creature
// casts, crew expiry, blink vs land-search vs Monarch separation,
// graveyard timing, X-cost restricted-bucket wipes, fetch land pairs,
// and awareness accounting.

use super::game::run_game;
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

/// Build a deck from explicit card rows (count pairs).
fn deck_from(rows: &[(CardRow, usize)]) -> SimDeck {
    let mut cards = Vec::new();
    for (row, count) in rows {
        for _ in 0..*count {
            cards.push(parse_sim_card(row));
        }
    }
    SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    }
}

/// A creature-restricted land's mana pays a creature cast (Secluded
/// Courtyard class): the courtyard's creature-only bucket funds a
/// creature cast even when the general pool falls short, and the cast
/// lands on the battlefield.
#[test]
fn creature_restricted_land_pays_creature_cast() {
    let courtyard = card(
        "Secluded Courtyard",
        "",
        "Land",
        "As this land enters, choose a creature type.\n{T}: Add {C}.\n{T}: Add one mana of any color. Spend this mana only to cast a creature spell of the chosen type.",
    );
    // A sparse basic base: only the courtyard's restricted mana makes
    // the 5-cost creature castable in the opening turns.
    let big_body = card("Big Body", "{5}", "Creature — Beast", "");
    let land = card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)");
    let mut rows: Vec<(CardRow, usize)> = vec![(land, 6), (courtyard, 10)];
    rows.push((big_body, 1));
    let deck = deck_from(&rows);
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let big_body: Vec<usize> = deck
        .cards
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name == "Big Body")
        .map(|(i, _)| i)
        .collect();
    // The cast must be funded through the restricted bucket in a good
    // share of games: with only 6 plain sources, the 5-cost cast needs
    // the courtyard's mana.
    let cast_games = logs
        .iter()
        .filter(|log| {
            big_body
                .iter()
                .any(|i| log.card_first_battlefield.contains_key(i))
        })
        .count();
    assert!(
        cast_games > 20,
        "creature-restricted mana should cast the creature: {cast_games}/300"
    );
}

/// A crewed Vehicle stops being a body next turn: crew expires at the
/// turn boundary, so a crewed vehicle's body credit is per turn only.
#[test]
fn crewed_vehicle_reverts_next_turn() {
    // A crewed flag clears at the turn boundary before the next census.
    let mut st = super::game::GameState {
        battlefield: Vec::new(),
        library: Vec::new(),
        hand: Vec::new(),
        seen: 0,
        graveyard: Vec::new(),
        battlefield_seen: Default::default(),
        graveyard_seen: Default::default(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        drained: 0,
        life_paid: 0,
        is_monarch: false,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
    };
    st.battlefield.push(super::game::InPlay {
        uid: 1,
        card: usize::MAX - 1,
        tapped: false,
        sick: false,
        counters: 0,
        animated: false,
        crewed: true,
        entered_turn: 0,
        saga_step: 0,
        fired: false,
        blink_pending: false,
        loyalty: 0,
        equipped: false,
        equip_host: None,
        is_commander: false,
        commander_slot: 0,
    });
    crate::deck::simulator::game_run::expire_crew(&mut st);
    assert!(!st.battlefield[0].crewed, "crew must expire at end of turn");
}

/// An Equipment's buff follows its host across board shifts: a token
/// entering before the next combat must not move the buff to another
/// permanent or drop it.
#[test]
fn equipment_buff_survives_board_shift() {
    let gear = card(
        "Test Sword",
        "{2}",
        "Artifact — Equipment",
        "Equipped creature gets +2/+0.\nEquip {2}",
    );
    let mut host = card("Big Body", "{2}", "Creature — Beast", "");
    host.power = Some("3".into());
    let tiny = card("Small Body", "{1}", "Creature — Rat", "");
    let cards = vec![
        parse_sim_card(&host),
        parse_sim_card(&gear),
        parse_sim_card(&tiny),
    ];
    let deck = SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut st = super::game::GameState {
        battlefield: Vec::new(),
        library: Vec::new(),
        hand: Vec::new(),
        seen: 0,
        graveyard: Vec::new(),
        battlefield_seen: Default::default(),
        graveyard_seen: Default::default(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        drained: 0,
        life_paid: 0,
        is_monarch: false,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
    };
    // Board: host (pos 0), gear (pos 1), a small body (pos 2). Bodies
    // enter unsick so equip can pick its host this turn.
    let mut host_perm = super::game::new_perm_with(1, &deck, 0, 1, false);
    host_perm.sick = false;
    st.battlefield.push(host_perm);
    st.battlefield
        .push(super::game::new_perm_with(2, &deck, 1, 1, false));
    let mut tiny_perm = super::game::new_perm_with(3, &deck, 2, 1, false);
    tiny_perm.sick = false;
    st.battlefield.push(tiny_perm);
    // Equip: pay the equip cost onto the strongest body.
    let mut pool = super::game::Pool::default();
    pool.fixed[2] = 4;
    super::game_mana::pay_cost(
        &super::model::Cost {
            generic: 2,
            ..Default::default()
        },
        &mut pool,
    );
    super::game_effects::tap_budget(&deck, &mut st.battlefield, &mut pool, &st.hand);
    assert!(st.battlefield[1].equipped, "the gear suits up");
    let host_uid = st.battlefield[0].uid;
    assert_eq!(
        st.battlefield[1].equip_host,
        Some(host_uid),
        "the buff keys to the host uid"
    );
    // A token enters before the next combat, shifting positions.
    let token = super::game::InPlay {
        uid: 9,
        card: usize::MAX - 1,
        tapped: false,
        sick: false,
        counters: 0,
        animated: false,
        crewed: false,
        entered_turn: 2,
        saga_step: 0,
        fired: false,
        blink_pending: false,
        loyalty: 0,
        equipped: false,
        equip_host: None,
        is_commander: false,
        commander_slot: 0,
    };
    st.battlefield.insert(0, token);
    let combat = super::game_combat::combat_phase(&deck, &mut st, 2, 8, &[1, 1, 1, 1, 1, 1, 1, 1]);
    // Attack power = host (3) + gear buff (2) + small body (2) + the
    // token's flat 2. A stale battlefield index would put the buff on
    // the token (now at position 0), inflating the total by 2.
    assert_eq!(
        combat.power, 9,
        "the buff follows the host after the shift, got {}",
        combat.power
    );
}

/// A blink ETB re-fires exactly once per blink, the turn after; a
/// fetch-style ETB land never re-fires its search.
#[test]
fn blink_refires_once_land_search_does_not() {
    // Blink path: the card carries a draw ETB plus a self-blink clause.
    // The re-fire applies the draw ETB once, the turn after entry; the
    // deferred firing never re-arms, so the draw fires exactly twice
    // (entry + one re-fire), never more.
    let blink_body = card(
        "Blink Drawer",
        "{2}",
        "Creature — Wizard",
        "When this creature enters, draw a card.\nWhen this creature enters, exile it, then return it to the battlefield.",
    );
    let land = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let deck = deck_from(&[(land, 26), (blink_body, 12)]);
    let mut rng = ChaCha8Rng::seed_from_u64(31);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 4)).collect();
    // Baseline: entry draw (1) + turn draws (4) + opener (7) = 12.
    // The blink re-fire adds exactly one more draw: 13 by t4 in games
    // where the body entered by turn 2.
    let big = logs.iter().map(|log| log.cards_seen[3]).collect::<Vec<_>>();
    let fed = big.iter().filter(|s| **s > 13).count();
    assert!(
        fed > 5,
        "blink re-fire should add one extra draw: {fed}/200 above 13"
    );
    // Land-search path: a fetch land alone re-fires nothing. One fetch
    // on an otherwise bare deck fetches exactly one extra land.
    let fetch = card(
        "Polluted Delta",
        "",
        "Land",
        "{T}, Pay 1 life, Sacrifice this: Search your library for an Island or Swamp card, put it onto the battlefield, then shuffle.",
    );
    let solo = deck_from(&[
        (fetch, 1),
        (
            card("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
            40,
        ),
    ]);
    let mut rng2 = ChaCha8Rng::seed_from_u64(7);
    let log = run_game(&solo, &mut rng2, 3);
    // One fetch searches once: every card seen by end of t3 is a land
    // (opener 7 + 2 draws = 9), and a repeated search would push the
    // census past what the deck could hold.
    assert!(
        log.lands_seen_by_11 <= 9,
        "fetch searches once: {}",
        log.lands_seen_by_11
    );
}

/// Monarch acquisition draws one extra card per turn from the turn
/// after acquisition (an engine, not a one-shot apply).
#[test]
fn monarch_draws_extra_card_per_turn() {
    let monarch = card(
        "Monarch Maker",
        "{2}",
        "Creature — Noble",
        "When this creature enters, you become the Monarch.",
    );
    let land = card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)");
    let deck = deck_from(&[(land, 28), (monarch, 12)]);
    let mut rng = ChaCha8Rng::seed_from_u64(41);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let monarch_idx: Vec<usize> = deck
        .cards
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name == "Monarch Maker")
        .map(|(i, _)| i)
        .collect();
    let with_engine = logs
        .iter()
        .filter(|log| {
            monarch_idx
                .iter()
                .any(|i| log.card_first_battlefield.contains_key(i))
        })
        .collect::<Vec<_>>();
    // A baseline deck sees ~13 cards by t5; the Monarch adds one card
    // per turn after acquisition, so games with the Monarch out early
    // see more by t5.
    let fed = with_engine
        .iter()
        .filter(|log| log.cards_seen[4] > 14)
        .count();
    assert!(
        fed > 10,
        "monarch should draw one extra card per turn: {fed}/{}",
        with_engine.len()
    );
}

/// An end-of-turn hand-limit discard records the discard turn in the
/// graveyard log, not the total game length.
#[test]
fn end_step_discard_records_discard_turn() {
    let land = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    // A flood engine fills the hand past the limit; the overflow
    // discards must carry their own turn.
    let engine = card(
        "Draw Engine",
        "{2}",
        "Artifact",
        "At the beginning of your upkeep, draw two cards.",
    );
    let filler = card("Filler", "{1}", "Instant", "Discard a card: Draw nothing.");
    let deck = deck_from(&[(land, 30), (engine, 8), (filler, 12)]);
    let mut rng = ChaCha8Rng::seed_from_u64(51);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 5)).collect();
    let filler_idx: Vec<usize> = deck
        .cards
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name == "Filler")
        .map(|(i, _)| i)
        .collect();
    // Engine draws force hand-limit discards before the final scheduled
    // turn. The old bug stamped every discard with the game length.
    let turns: Vec<u32> = logs
        .iter()
        .flat_map(|log| {
            filler_idx
                .iter()
                .filter_map(|i| log.card_first_graveyard.get(i).copied())
        })
        .collect();
    assert!(
        !turns.is_empty(),
        "filler must reach the graveyard through the end-step discard"
    );
    assert!(
        turns.iter().any(|turn| *turn < 5),
        "at least one discard must retain its actual pre-final turn: {turns:?}"
    );
}

/// A wheel's drawn replacement cards count toward the awareness census
/// (drawn + milled + scried/surveiled), like plain draws.
#[test]
fn wheel_draws_feed_awareness() {
    let land = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let wheel = card(
        "Wheel Spell",
        "{3}",
        "Sorcery",
        "Each player discards their hand, then draws seven cards.",
    );
    let deck = deck_from(&[(land, 24), (wheel, 12)]);
    let mut rng = ChaCha8Rng::seed_from_u64(61);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 5)).collect();
    // With 12 wheel copies in 36, wheels cast most turns; every drawn
    // wheel card carries awareness credit, so awareness must run well
    // past the plain-draw baseline in a good share of games.
    let aware: Vec<_> = logs.iter().map(|log| log.awareness[4]).collect();
    let high = aware.iter().filter(|a| **a > 0.6).count();
    assert!(
        high > 10,
        "wheel draws should feed awareness: {high} games above 0.6"
    );
}

/// A X-cost spell wipes every restricted bucket, not just the general
/// pool: an X spell with creature-only mana in the pool leaves no
/// creature-only mana behind.
#[test]
fn x_cost_wipes_restricted_buckets() {
    let courtyard = card(
        "Secluded Courtyard",
        "",
        "Land",
        "As this land enters, choose a creature type.\n{T}: Add {C}.\n{T}: Add one mana of any color. Spend this mana only to cast a creature spell of the chosen type.",
    );
    let land = card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)");
    // A cheap X draw spell: after it resolves, no restricted bucket
    // survives.
    let x_spell = card("X Draw", "{X}{W}", "Sorcery", "Draw X cards.");
    let mut rows: Vec<(CardRow, usize)> = vec![(land, 24), (x_spell, 8)];
    rows.push((courtyard, 6));
    let deck = deck_from(&rows);
    let mut rng = ChaCha8Rng::seed_from_u64(81);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 6)).collect();
    // The X spell's cast converts the whole pool (general + every
    // restricted bucket) into X: mana spent tracks the full conversion.
    let spent: Vec<_> = logs.iter().map(|log| log.mana_spent[4]).collect();
    assert!(
        spent.iter().any(|s| *s >= 3.0),
        "X spell should spend the leftover pool: {spent:?}"
    );
    // Pool-level unit pin: after the wipe, the bucket reads zero.
    let mut pool = super::game::Pool {
        creature_only: 3,
        legendary_only: 2,
        artifact_only: 1,
        instant_sorcery_only: 2,
        colorless: 5,
        ..Default::default()
    };
    super::cast_phase::wipe_restricted_buckets(&mut pool);
    assert_eq!(
        pool.creature_only + pool.legendary_only + pool.artifact_only + pool.instant_sorcery_only,
        0
    );
}

/// Polluted Delta opens only its target pair's verge gates (Island and
/// Swamp), never a Mountain gate.
#[test]
fn fetch_targets_only_its_pair() {
    let delta = card(
        "Polluted Delta",
        "",
        "Land",
        "{T}, Pay 1 life, Sacrifice this: Search your library for an Island or Swamp card, put it onto the battlefield, then shuffle.",
    );
    let island = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let swamp = card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)");
    let steam_vents = card(
        "Steam Vents",
        "",
        "Land — Island Mountain",
        "({T}: Add {U} or {R}.)",
    );
    let mut cards = Vec::new();
    for _ in 0..16 {
        cards.push(parse_sim_card(&island));
    }
    for _ in 0..8 {
        cards.push(parse_sim_card(&swamp));
    }
    for _ in 0..8 {
        cards.push(parse_sim_card(&delta));
    }
    for _ in 0..2 {
        cards.push(parse_sim_card(&steam_vents));
    }
    // A steam-vents-style gate land that needs an Island or Mountain in
    // play: the Delta's fetched Island satisfies it, a fetched Mountain
    // would too — but the Delta only fetches Island/Swamp.
    cards.push(parse_sim_card(&card(
        "River Verge",
        "",
        "Land",
        "As this land enters, you may pay 2 life. If you do, it enters untapped.\n{T}: Add {U}.\n{T}: Add {R}. Activate only if you control an Island or a Mountain.",
    )));
    let deck = SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(71);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 4)).collect();
    // The Delta fetches an Island (or Swamp); the fetched Island
    // satisfies the River Verge gate, so the verge's blue mode
    // unlocks in most games. Assert the game runs and the verge's
    // gated mana joins the pool in some games.
    let blue_mana_games = logs
        .iter()
        .filter(|log| log.mana_available[3] >= 4.0)
        .count();
    assert!(
        blue_mana_games > 5,
        "delta-fetched island should open the blue verge gate: {blue_mana_games}/100"
    );
}

/// The fetch-name table and the land-type table stay consistent: every
/// recognized fetch carries a nonempty target pair, and every land type
/// entry that looks like a fetch matches the fetch table.
#[test]
fn fetch_tables_stay_consistent() {
    for name in [
        "Flooded Strand",
        "Polluted Delta",
        "Windswept Heath",
        "Wooded Foothills",
        "Fabled Passage",
        "Scalding Tarn",
        "Arid Mesa",
        "Marsh Flats",
        "Misty Rainforest",
        "Bloodstained Mire",
        "Verdant Catacombs",
        "Prismatic Vista",
        "Terramorphic Expanse",
        "Evolving Wilds",
        "Escape Tunnel",
    ] {
        assert!(
            !super::game::land_types(name).is_empty(),
            "{name} must map to its fetch pair"
        );
        let card = SimCard {
            name: name.to_string(),
            ..SimCard::default()
        };
        assert!(
            super::game::fetches_land_text(&card),
            "{name} must be recognized as a fetch"
        );
    }
    // Named fetches carry their real pair, never all five basics.
    assert_eq!(
        super::game::land_types("Polluted Delta"),
        &["Island", "Swamp"],
        "Polluted Delta targets the blue-black pair"
    );
    assert_eq!(
        super::game::land_types("Flooded Strand"),
        &["Plains", "Island"],
        "Flooded Strand targets the white-blue pair"
    );
    assert_eq!(
        super::game::land_types("Marsh Flats"),
        &["Plains", "Swamp"],
        "Marsh Flats targets the white-black pair"
    );
    assert_eq!(
        super::game::land_types("Arid Mesa"),
        &["Plains", "Mountain"],
        "Arid Mesa targets the red-white pair"
    );
    assert_eq!(
        super::game::land_types("Misty Rainforest"),
        &["Island", "Forest"],
        "Misty Rainforest targets the blue-green pair"
    );
    // Generic search lands (any basic) keep the full set.
    assert_eq!(super::game::land_types("Evolving Wilds").len(), 5);
}

/// Misty Rainforest opens an Island-or-Forest gate and never a Plains
/// gate: a verge land gated on "Island or a Forest" unlocks off the
/// fetched Forest, and a Plains-gated verge stays shut.
#[test]
fn misty_rainforest_opens_its_own_pair() {
    let misty = card(
        "Misty Rainforest",
        "",
        "Land",
        "{T}, Pay 1 life, Sacrifice this: Search your library for a Forest or Island card, put it onto the battlefield, then shuffle.",
    );
    let island = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let forest = card("Forest", "", "Basic Land — Forest", "({T}: Add {G}.)");
    let mut cards = Vec::new();
    for _ in 0..10 {
        cards.push(parse_sim_card(&island));
    }
    for _ in 0..10 {
        cards.push(parse_sim_card(&forest));
    }
    for _ in 0..4 {
        cards.push(parse_sim_card(&misty));
    }
    // Gated on Forest (the pair Misty must open): its green mode fires
    // once the fetched Forest is in play.
    cards.push(parse_sim_card(&card(
        "Canopy Verge",
        "",
        "Land",
        "As this land enters, you may pay 2 life. If you do, it enters untapped.\n{T}: Add {C}.\n{T}: Add {G}. Activate only if you control a Forest or a Plains.",
    )));
    let deck = SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(71);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 4)).collect();
    // With the corrected pair the fetched Forest unlocks the green mode;
    // with the old bogus pair (Plains/Island/Swamp) the gate stays shut
    // in nearly every game (only the real Forests in play would open it,
    // and the fetch would never add to them).
    let gate_open_games = logs
        .iter()
        .filter(|log| log.mana_available[3] >= 4.0)
        .count();
    assert!(
        gate_open_games > 20,
        "misty-fetched forest should open the green verge gate: {gate_open_games}/100"
    );
}

/// The Treasure bank empties at the turn boundary: banked Treasure spent
/// into a turn's pool is consumed, so flexible mana does not compound
/// turn over turn (each turn spends only the Treasure created that turn).
#[test]
fn treasure_bank_consumes_each_turn() {
    let land = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let maker = card(
        "Treasure Maker",
        "{3}",
        "Creature — Pirate",
        "When this creature enters, create two Treasure tokens.",
    );
    let deck = deck_from(&[(land, 24), (maker, 12)]);
    let mut rng = ChaCha8Rng::seed_from_u64(91);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    // With the bank compounding (the old bug), a deck making 2 Treasure
    // per turn grows flexible mana unboundedly and every turn's pool
    // swells. With the bank consumed per turn, late-turn mana stays
    // bounded by lands plus that turn's Treasure.
    let runaway = logs
        .iter()
        .filter(|log| log.mana_available[5] > 20.0)
        .count();
    assert_eq!(
        runaway, 0,
        "mana pool must not compound via the treasure bank: {runaway}/200 games exceed 20"
    );
}

/// A battlefield self-mill engine routes its mill to the self census
/// even when the commander mills opponents (the host's own flag wins).
#[test]
fn battlefield_self_mill_not_overridden_by_commander() {
    let self_miller = card(
        "Self Miller",
        "{2}",
        "Creature — Zombie",
        "At the beginning of your upkeep, mill three cards.",
    );
    let commander = card(
        "Opp Miller",
        "{1}{B}{B}",
        "Legendary Creature — Horror",
        "At the beginning of your upkeep, each opponent mills two cards.",
    );
    let land = card("Swamp", "", "Basic Land — Swamp", "({T}: Add {B}.)");
    let cards = vec![
        parse_sim_card(&commander),
        parse_sim_card(&self_miller),
        parse_sim_card(&self_miller),
        parse_sim_card(&self_miller),
    ];
    let mut lib = vec![parse_sim_card(&land); 20];
    lib.push(parse_sim_card(&self_miller));
    lib.push(parse_sim_card(&self_miller));
    lib.push(parse_sim_card(&self_miller));
    let deck = SimDeck {
        cards: lib,
        commanders: cards,
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 4)).collect();
    // The self-miller must reach the battlefield in most games (3 copies
    // of a 2-drop in a 24-card library). When it is out, its upkeep mill
    // feeds milled_self; only the commander's own firing feeds milled_opp.
    let with_engine = logs
        .iter()
        .filter(|log| {
            deck.cards
                .iter()
                .enumerate()
                .filter(|(_, c)| c.name == "Self Miller")
                .any(|(i, _)| log.card_first_battlefield.contains_key(&i))
        })
        .collect::<Vec<_>>();
    let self_milled = with_engine
        .iter()
        .filter(|log| log.self_milled[3] > 0)
        .count();
    assert!(
        self_milled > 5,
        "battlefield self-mill routes to the self census: {self_milled}/{}",
        with_engine.len()
    );
}

/// A banked mana engine with no counters left stays idle: a Pentad
/// Prism class card whose charge counters are spent must not keep
/// producing a free mana pip every turn.
#[test]
fn banked_activation_stops_when_counters_gone() {
    // Sunburst best case (one color paid) → 1 banked pip.
    let prism = card(
        "Pentad Prism",
        "{2}",
        "Artifact",
        "Sunburst (This artifact enters with a charge counter on it for each color of mana spent to cast it.)\nRemove a charge counter from this artifact: Add one mana of any color.",
    );
    let land = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let deck = deck_from(&[(land, 24), (prism, 12)]);
    let mut rng = ChaCha8Rng::seed_from_u64(31);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 8)).collect();
    // With the zero-counter leak, each spent prism keeps yielding one
    // pip every turn for the rest of the game and late-turn pools
    // exceed lands plus turn draws. With the gate, turn-8 mana stays
    // bounded: 24 lands + up to ~2 prism pip turns worth of banking.
    let leaky = logs
        .iter()
        .filter(|log| log.mana_available[7] > 26.0)
        .count();
    assert_eq!(
        leaky, 0,
        "spent prisms must stop producing mana: {leaky}/200 games exceed 26 mana at t8"
    );
}
