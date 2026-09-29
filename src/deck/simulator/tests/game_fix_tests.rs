//! Regression tests for game-loop fixes: restricted-mana creature
//! casts, crew expiry, blink vs land-search vs Monarch separation,
//! graveyard timing, X-cost restricted-bucket wipes, fetch land pairs,
//! and awareness accounting.

use super::game::run_game;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::cost::parse_cost;
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
        companion: None,
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
    let large_creature_set: Vec<usize> = deck
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
            large_creature_set
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
        exile: Vec::new(),
        battlefield_seen: Default::default(),
        graveyard_seen: Default::default(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        opponent_life_lost: 0,
        damage_dealt_this_turn: 0,
        life_gained: 0,
        flashback_permissions: std::collections::HashSet::new(),
        replay_casts: 0,
        milestones_by_turn: std::collections::HashMap::new(),
        life_paid: 0,
        life_funded_draws: 0,
        life: 20,
        is_monarch: false,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
        player_counters: Default::default(),
        activated_this_turn: Default::default(),
        activated_once: Default::default(),
        triggered_this_turn: Default::default(),
        companion_fetched: false,
        ring_tempts: 0,
        extra_land_drops_this_turn: 0,
        spells_cast_this_turn: 0,
        attacked_this_turn: false,
        plotted: Vec::new(),
        ring_bearer: None,
    };
    st.battlefield.push(super::game::Permanent {
        uid: 1,
        card: crate::deck::simulator::game::CardRef::Token(
            crate::deck::simulator::game::TokenKind::Creature,
        ),
        tapped: false,
        summoning_sick: false,
        counters: Default::default(),
        animated: false,
        crewed: true,
        entered_turn: 0,
        saga_step: 0,
        fired: false,
        returned_trigger_pending: false,
        equipped: false,
        equip_host: None,
        face_down: false,
        saddled: false,
        army: false,
        jace_token: false,
        sacrifice_at_end: false,
        attacking_this_turn: false,
    });
    super::game_run::expire_crew(&mut st);
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
        companion: None,
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
        exile: Vec::new(),
        battlefield_seen: Default::default(),
        graveyard_seen: Default::default(),
        treasure_bank: 0,
        milled_self: 0,
        milled_opp: 0,
        opponent_life_lost: 0,
        damage_dealt_this_turn: 0,
        life_gained: 0,
        flashback_permissions: std::collections::HashSet::new(),
        replay_casts: 0,
        milestones_by_turn: std::collections::HashMap::new(),
        life_paid: 0,
        life_funded_draws: 0,
        life: 20,
        is_monarch: false,
        awareness_cards: 0,
        extra_turns_queued: 0,
        prowess_casts: 0,
        infinite_mana_suspected: false,
        next_uid: 0,
        player_counters: Default::default(),
        activated_this_turn: Default::default(),
        activated_once: Default::default(),
        triggered_this_turn: Default::default(),
        companion_fetched: false,
        ring_tempts: 0,
        extra_land_drops_this_turn: 0,
        spells_cast_this_turn: 0,
        attacked_this_turn: false,
        plotted: Vec::new(),
        ring_bearer: None,
    };
    // Board: host (pos 0), gear (pos 1), a small body (pos 2). Bodies
    // enter unsick so equip can pick its host this turn.
    let mut host_perm = super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        1,
        false,
    );
    host_perm.summoning_sick = false;
    st.battlefield.push(host_perm);
    st.battlefield.push(super::game::new_perm_with(
        2,
        &deck,
        crate::deck::simulator::model::CardIdx(1),
        1,
        false,
    ));
    let mut tiny_perm = super::game::new_perm_with(
        3,
        &deck,
        crate::deck::simulator::model::CardIdx(2),
        1,
        false,
    );
    tiny_perm.summoning_sick = false;
    st.battlefield.push(tiny_perm);
    // Equip: pay the equip cost onto the strongest body.
    let mut pool = super::game::ManaPool::default();
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
    let token = super::game::Permanent {
        uid: 9,
        card: crate::deck::simulator::game::CardRef::Token(
            crate::deck::simulator::game::TokenKind::Creature,
        ),
        tapped: false,
        summoning_sick: false,
        counters: Default::default(),
        animated: false,
        crewed: false,
        entered_turn: 2,
        saga_step: 0,
        fired: false,
        returned_trigger_pending: false,
        equipped: false,
        equip_host: None,
        face_down: false,
        saddled: false,
        army: false,
        jace_token: false,
        sacrifice_at_end: false,
        attacking_this_turn: false,
    };
    st.battlefield.insert(0, token);
    let combat = super::game_combat::combat_phase(&deck, &mut st, 2, &[1, 1, 1, 1, 1, 1, 1, 1]);
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
        log.lands_seen_by_turn_4 <= 9,
        "fetch searches once: {}",
        log.lands_seen_by_turn_4
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
    // Constructed skips the first draw. The Monarch adds one card per
    // turn after acquisition, so games with the Monarch out early still
    // exceed the baseline by turn 5.
    let fed = with_engine
        .iter()
        .filter(|log| log.cards_seen[4] > 12)
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
    let mut rows: Vec<(CardRow, usize)> = vec![(land, 24), (x_spell.clone(), 8)];
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
    // ManaPool-level unit pin: a plain sorcery X spell belongs to no cast
    // class, so every bucket is illegal and discards.
    let mut pool = super::game::ManaPool {
        creature_only: 3,
        legendary_only: 2,
        artifact_only: 1,
        instant_sorcery_only: 2,
        colorless: 5,
        ..Default::default()
    };
    let x = super::cast_pass::cast_resolve::convert_pool_to_x(&parse_sim_card(&x_spell), &mut pool);
    assert_eq!(x, 5, "the X value counts only legally fundable mana");
    assert_eq!(
        pool.creature_only + pool.legendary_only + pool.artifact_only + pool.instant_sorcery_only,
        0
    );
    // A creature X spell spends the creature-only bucket into X: that
    // mana legally funds the cast, so it converts and drains. The other
    // buckets stay illegal and discard.
    let mut creature_card = parse_sim_card(&x_spell);
    creature_card.is_creature = true;
    let mut pool = super::game::ManaPool {
        creature_only: 3,
        legendary_only: 2,
        colorless: 5,
        ..Default::default()
    };
    let x = super::cast_pass::cast_resolve::convert_pool_to_x(&creature_card, &mut pool);
    assert_eq!(x, 8, "the creature bucket funds X with the general pool");
    assert_eq!(pool.creature_only, 0, "the member bucket is spent into X");
    assert_eq!(pool.legendary_only, 0, "unrelated buckets discard");
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
        companion: None,
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

/// Fetch targets come from parsed Oracle search clauses.
#[test]
fn fetch_targets_come_from_oracle_text() {
    use super::model::BasicLandType::{self, *};
    let cases: &[(&str, &str, &[BasicLandType], bool)] = &[
        (
            "Flooded Strand",
            "a Plains or Island card",
            &[Plains, Island],
            false,
        ),
        (
            "Polluted Delta",
            "an Island or Swamp card",
            &[Island, Swamp],
            false,
        ),
        (
            "Windswept Heath",
            "a Plains or Forest card",
            &[Plains, Forest],
            false,
        ),
        (
            "Wooded Foothills",
            "a Mountain or Forest card",
            &[Mountain, Forest],
            false,
        ),
        (
            "Scalding Tarn",
            "an Island or Mountain card",
            &[Island, Mountain],
            false,
        ),
        (
            "Arid Mesa",
            "a Plains or Mountain card",
            &[Plains, Mountain],
            false,
        ),
        (
            "Marsh Flats",
            "a Plains or Swamp card",
            &[Plains, Swamp],
            false,
        ),
        (
            "Misty Rainforest",
            "a Forest or Island card",
            &[Island, Forest],
            false,
        ),
        (
            "Bloodstained Mire",
            "a Swamp or Mountain card",
            &[Swamp, Mountain],
            false,
        ),
        (
            "Verdant Catacombs",
            "a Swamp or Forest card",
            &[Swamp, Forest],
            false,
        ),
        (
            "Fabled Passage",
            "a basic land card",
            &BasicLandType::ALL,
            true,
        ),
        (
            "Prismatic Vista",
            "a basic land card",
            &BasicLandType::ALL,
            true,
        ),
        (
            "Terramorphic Expanse",
            "a basic land card",
            &BasicLandType::ALL,
            true,
        ),
        (
            "Evolving Wilds",
            "a basic land card",
            &BasicLandType::ALL,
            true,
        ),
        (
            "Escape Tunnel",
            "a basic land card",
            &BasicLandType::ALL,
            true,
        ),
    ];
    for (name, target, expected_types, basic_only) in cases {
        let parsed = parse_sim_card(&card(
            name,
            "",
            "Land",
            &format!(
                "{{T}}, Sacrifice this land: Search your library for {target}, put it onto the battlefield tapped, then shuffle."
            ),
        ));
        assert!(parsed.is_fetch_land, "{name} must parse as a fetch");
        assert_eq!(
            parsed.fetch_target_types, *expected_types,
            "{name} must use only target types named in Oracle text"
        );
        assert_eq!(parsed.fetch_basic_only, *basic_only);
    }

    let oracle_named_fetch = parse_sim_card(&card(
        "Oracle Fetch",
        "",
        "Land",
        "{T}, Sacrifice this land: Search your library for a Forest or Island card, put it onto the battlefield, then shuffle.",
    ));
    assert!(oracle_named_fetch.is_fetch_land);
    assert_eq!(
        oracle_named_fetch.fetch_target_types,
        vec![
            super::model::BasicLandType::Island,
            super::model::BasicLandType::Forest,
        ]
    );
    assert!(!oracle_named_fetch.fetch_basic_only);
    let same_name_without_search = parse_sim_card(&card("Oracle Fetch", "", "Land", ""));
    assert!(!same_name_without_search.is_fetch_land);
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
        companion: None,
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
    // One commander: partner copies of the self-miller in the command
    // zone would each cast for their own slot and starve the library
    // cast. The test needs a battlefield self-mill engine racing an
    // opponent-milling commander.
    let commanders = vec![parse_sim_card(&commander)];
    let mut lib = vec![parse_sim_card(&land); 20];
    lib.push(parse_sim_card(&self_miller));
    lib.push(parse_sim_card(&self_miller));
    lib.push(parse_sim_card(&self_miller));
    let deck = SimDeck {
        companion: None,
        cards: lib,
        commanders,
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

/// The graveyard-exchange sacrifice loop stays bounded while death
/// triggers create tokens: two payoff creatures on the board refill
/// it with token bodies every round while one survives, and the cast
/// still resolves in bounded rounds instead of relying on an
/// unbounded `while` loop.
#[test]
fn graveyard_exchange_sacrifice_loop_terminates_with_death_tokens() {
    // Cascade drives the exchange out of hand (Living End has no mana
    // cost). Two payoff creatures sit on the battlefield; each
    // sacrifice of one fires the survivor's death trigger and adds
    // two token bodies.
    let rows = [
        super::turn_loop_tests::row("Cascade spell", "{2}{U}", "Sorcery", "Cascade."),
        super::turn_loop_tests::row(
            "Living End",
            "",
            "Sorcery",
            "Each player exiles all creature cards from their graveyard, then sacrifices all creatures they control, then puts all cards they exiled this way onto the battlefield.",
        ),
        super::turn_loop_tests::row("Island", "", "Basic Land — Island", "({T}: Add {U}.)"),
        super::turn_loop_tests::row(
            "Death Token Maker",
            "{3}",
            "Creature — Zombie",
            "Whenever another creature you control dies, create two 2/2 creature tokens.",
        ),
    ];
    let cards = super::turn_loop_tests::deck(&rows);
    let mut st = super::turn_loop_tests::state(vec![0, 3, 3], vec![1, 2]);
    // Two payoff creatures on the board (card 3, two copies).
    for _ in 0..2 {
        let uid = super::game::take_uid(&mut st);
        st.battlefield.push(super::game::new_perm_with(
            uid,
            &cards,
            crate::deck::simulator::model::CardIdx(3),
            1,
            false,
        ));
    }
    let mut pool = super::game::ManaPool {
        flexible: 4,
        ..super::game::ManaPool::default()
    };
    // This call is the test: the loop must finish in bounded rounds
    // (the death-token refills keep the board alive for a while; the
    // cap bounds the exchange regardless).
    super::cast_pass::cast_pass(
        &cards,
        &mut st,
        &mut pool,
        1,
        &mut [0.0],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    );
    // The exchange drained every deck body (payoffs sacrificed; refill
    // tokens consumed by later rounds), the cast left the hand, and
    // the exchange card resolved through the cascade.
    assert!(
        st.battlefield.is_empty(),
        "the capped loop drained every body: {:?}",
        st.battlefield.iter().map(|p| p.card).collect::<Vec<_>>()
    );
    assert!(
        st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(3)),
        "the payoffs were sacrificed"
    );
    assert!(
        st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(1)),
        "the exchange card resolved"
    );
}

/// An opponent mill touches no player zone: only the opponent census
/// moves, the player's library and graveyard stay unchanged.
#[test]
fn opponent_mill_leaves_player_zones_untouched() {
    let opp_mill = card(
        "Opp Miller",
        "{2}{B}",
        "Sorcery",
        "Target opponent mills three cards.",
    );
    let cards = super::turn_loop_tests::deck(&[
        super::turn_loop_tests::row("Filler", "{9}", "Sorcery", ""),
        opp_mill,
    ]);
    let mut st = super::turn_loop_tests::state(vec![1], vec![0]);
    let mut pool = super::game::ManaPool {
        flexible: 4,
        ..super::game::ManaPool::default()
    };
    super::cast_pass::cast_pass(
        &cards,
        &mut st,
        &mut pool,
        1,
        &mut [0.0],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    );
    assert_eq!(st.milled_opp, 3, "the opponent mill feeds the opp census");
    assert_eq!(
        st.milled_self, 0,
        "an opponent mill never feeds the self census"
    );
    assert_eq!(
        st.library,
        [0].iter()
            .map(|i| crate::deck::simulator::model::CardIdx(*i))
            .collect::<Vec<_>>(),
        "the player's library is untouched by an opponent mill"
    );
    assert!(
        !st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0)),
        "the player's graveyard gains nothing from an opponent mill"
    );
}

/// A graveyard cast with an additional life cost gates on life the
/// same way hand casts do: with life at or below the cost, the spell
/// stays in the graveyard.
#[test]
fn graveyard_cast_gates_on_additional_life_cost() {
    // A flashback spell whose cost includes paying 4 life, with the
    // flashback permission already active (a prior discard + enabler).
    let spell = card(
        "Costly Spell",
        "{1}",
        "Instant",
        "As an additional cost to cast this spell, pay 4 life.\nDraw a card.",
    );
    let land = card("Island", "", "Basic Land — Island", "({T}: Add {U}.)");
    let cards = super::turn_loop_tests::deck(&[spell, land]);
    let mut st = super::turn_loop_tests::state(vec![], vec![1]);
    st.graveyard.push(crate::deck::simulator::model::CardIdx(0));
    st.flashback_permissions
        .insert(crate::deck::simulator::model::CardIdx(0));
    st.life = 4;
    let mut pool = super::game::ManaPool {
        flexible: 4,
        ..super::game::ManaPool::default()
    };
    super::cast_pass::cast_graveyard_spells(
        &cards,
        &mut st,
        &mut pool,
        1,
        &mut [0.0],
        &mut Vec::new(),
    );
    // Life at 4 cannot pay the additional 4 (the same gate hand casts
    // apply: life must stay above the additional cost), so the spell
    // stays in the graveyard and no life is paid.
    assert!(
        st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0)),
        "the costly spell stays in the graveyard: {:?}",
        st.graveyard
    );
    assert_eq!(st.life, 4, "no additional life was paid");
    // The gate lifts when life is above the cost: life 5 pays the 4
    // and the spell resolves to exile.
    let mut st_ok = super::turn_loop_tests::state(vec![], vec![1]);
    st_ok
        .graveyard
        .push(crate::deck::simulator::model::CardIdx(0));
    st_ok
        .flashback_permissions
        .insert(crate::deck::simulator::model::CardIdx(0));
    st_ok.life = 5;
    let mut pool_ok = super::game::ManaPool {
        flexible: 4,
        ..super::game::ManaPool::default()
    };
    super::cast_pass::cast_graveyard_spells(
        &cards,
        &mut st_ok,
        &mut pool_ok,
        1,
        &mut [0.0],
        &mut Vec::new(),
    );
    assert!(
        !st_ok
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0)),
        "life above the cost lets the spell cast: {:?}",
        st_ok.graveyard
    );
    assert_eq!(st_ok.life, 1, "the additional cost was paid");
}

/// Summoning-sick bodies may crew and station: crewing taps other
/// creatures (CR 702.122a); CR 302.6 only blocks a sick creature's own
/// tap-symbol abilities and attacking.
#[test]
fn summoning_sick_bodies_crew_and_station() {
    let vehicle = card("Test Copter", "{2}", "Artifact — Vehicle", "Crew 2");
    let elf = card("Fresh Elf", "{G}", "Creature — Elf", "");
    let cards = vec![parse_sim_card(&vehicle), parse_sim_card(&elf)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut st = super::turn_loop_tests::state(vec![], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    st.battlefield.push(super::game::new_perm_with(
        2,
        &deck,
        crate::deck::simulator::model::CardIdx(1),
        1,
        false,
    ));
    st.battlefield[1].summoning_sick = true;
    let mut pool = super::game::ManaPool::default();
    let empty: Vec<crate::deck::simulator::model::CardIdx> = Vec::new();
    super::game_effects::tap_budget(&deck, &mut st.battlefield, &mut pool, &empty);
    assert!(
        st.battlefield[0].crewed,
        "a summoning-sick body may crew (CR 702.122a)"
    );
    assert!(
        st.battlefield[1].tapped,
        "the sick crew member is tapped by the crew"
    );
    assert!(
        st.battlefield[0].entered_turn < 2,
        "the vehicle's entry turn stays recorded for the attack gate"
    );
}

/// A Vehicle crewed the turn it entered cannot attack that turn (CR
/// 302.6): it becomes a creature only during that turn's main phase.
#[test]
fn entry_turn_crewed_vehicle_waits_one_turn_to_attack() {
    let vehicle = card("Test Copter", "{2}", "Artifact — Vehicle", "Crew 2");
    let elf = card("Elf", "{G}", "Creature — Elf", "");
    let cards = vec![parse_sim_card(&vehicle), parse_sim_card(&elf)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    // A crewed vehicle that entered this turn (turn 2) cannot attack in
    // turn 2's combat.
    let mut st = super::turn_loop_tests::state(vec![], vec![]);
    st.battlefield.push(super::game::Permanent {
        uid: 1,
        card: crate::deck::simulator::game::CardRef::Deck(crate::deck::simulator::model::CardIdx(
            0,
        )),
        tapped: false,
        summoning_sick: false,
        counters: Default::default(),
        animated: false,
        crewed: true,
        entered_turn: 2,
        saga_step: 0,
        fired: false,
        returned_trigger_pending: false,
        equipped: false,
        equip_host: None,
        face_down: false,
        saddled: false,
        army: false,
        jace_token: false,
        sacrifice_at_end: false,
        attacking_this_turn: false,
    });
    let combat = super::game_combat::combat_phase(&deck, &mut st, 2, &[0]);
    assert_eq!(
        combat.attackers, 0,
        "an entry-turn crewed vehicle cannot attack (CR 302.6)"
    );
    // The same vehicle crewed on a later turn attacks.
    st.battlefield[0].entered_turn = 1;
    let combat = super::game_combat::combat_phase(&deck, &mut st, 2, &[0]);
    assert_eq!(
        combat.attackers, 1,
        "a crewed vehicle controlled since turn start attacks"
    );
}

#[test]
fn once_each_turn_trigger_fires_once() {
    let bounded = card(
        "Once Landfall",
        "{2}{G}",
        "Creature — Elemental",
        "Landfall — Whenever a land you control enters, draw a card. This ability triggers only once each turn.",
    );
    let unbounded = card(
        "Per Landfall",
        "{2}{G}",
        "Creature — Elemental",
        "Landfall — Whenever a land you control enters, draw a card.",
    );
    let cards = vec![parse_sim_card(&bounded), parse_sim_card(&unbounded)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let draws = |bounded_case: bool| -> usize {
        let mut st = super::turn_loop_tests::state(vec![], vec![30, 31]);
        st.seen = st.hand.len() as u32 + st.library.len() as u32;
        st.battlefield.push(super::game::new_perm_with(
            1,
            &deck,
            crate::deck::simulator::model::CardIdx(u32::from(!bounded_case)),
            0,
            false,
        ));
        // Two land drops in the same turn fire the landfall trigger
        // twice; the bounded engine may draw only once.
        let first = st.hand.len();
        super::game::fire_triggers(&deck, &mut st, super::model::SimTrigger::LandEnters, 2);
        let second = st.hand.len();
        super::game::fire_triggers(&deck, &mut st, super::model::SimTrigger::LandEnters, 2);
        (second - first) + (st.hand.len() - second)
    };
    assert_eq!(
        draws(true),
        1,
        "a once-each-turn landfall trigger draws once on a two-drop turn"
    );
    assert_eq!(
        draws(false),
        2,
        "an unbounded landfall trigger draws per land drop"
    );
}

/// A once-each-turn trigger firing does not gate the same permanent's
/// paid activation, and an activation resolving does not gate a later
/// once-each-turn trigger: the two ledgers never cross.
#[test]
fn trigger_and_activation_once_flags_do_not_cross() {
    let hybrid = card(
        "Once and Tap",
        "{2}{U}",
        "Creature — Wizard",
        "At the beginning of your upkeep, draw a card. This ability triggers only once each turn.\n{2}, {T}: Draw a card.",
    );
    let cards = vec![parse_sim_card(&hybrid)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut st = super::turn_loop_tests::state(vec![10], vec![]);
    st.seen = st.hand.len() as u32 + st.library.len() as u32;
    st.battlefield.push(super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    // The turn boundary cleared summoning sickness.
    st.battlefield[0].summoning_sick = false;
    let mut pool = super::game::ManaPool {
        colorless: 6,
        ..super::game::ManaPool::default()
    };
    // The once-each-turn upkeep trigger fires; the paid activation must
    // stay usable.
    super::game::fire_triggers(&deck, &mut st, super::model::SimTrigger::Upkeep, 2);
    assert!(
        super::game_effects::activation::pick_best_activation(&deck, &st, &pool, 2).is_some(),
        "the trigger's once-per-turn marker never gates the activation"
    );
    // The activation fires (its own ledger marks the permanent); a
    // later once-each-turn trigger still fires.
    let activation = super::game_effects::activation::pick_best_activation(&deck, &st, &pool, 2)
        .expect("the activation is usable");
    super::game_effects::activation::resolve_activation(&deck, &mut st, &mut pool, 2, &activation);
    assert!(st.battlefield[0].tapped, "the activation pays its tap cost");
    let hand = st.hand.len();
    super::game::fire_triggers(&deck, &mut st, super::model::SimTrigger::EndStep, 2);
    assert_eq!(
        st.hand.len() - hand,
        0,
        "no end-step trigger exists on this card"
    );
    let hand = st.hand.len();
    super::game::fire_triggers(&deck, &mut st, super::model::SimTrigger::Upkeep, 2);
    assert_eq!(
        st.hand.len() - hand,
        0,
        "the upkeep trigger is once per turn: a same-turn refire draws nothing"
    );
}

/// An unbounded sibling trigger keeps firing after the card's
/// once-each-turn trigger has fired: one card with a bounded landfall
/// draw and a plain landfall draw draws three across two land drops
/// (1 bounded + 2 unbounded), not two.
#[test]
fn unbounded_sibling_fires_beside_the_once_trigger() {
    let twin = card(
        "Twin Landfall",
        "{3}{G}",
        "Creature — Elemental",
        "Landfall — Whenever a land you control enters, draw a card. This ability triggers only once each turn.\nLandfall — Whenever a land you control enters, draw a card.",
    );
    let cards = vec![parse_sim_card(&twin)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut st = super::turn_loop_tests::state(vec![], vec![40, 41, 42]);
    st.seen = st.hand.len() as u32 + st.library.len() as u32;
    st.battlefield.push(super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    let first = st.hand.len();
    super::game::fire_triggers(&deck, &mut st, super::model::SimTrigger::LandEnters, 2);
    let second = st.hand.len();
    super::game::fire_triggers(&deck, &mut st, super::model::SimTrigger::LandEnters, 2);
    assert_eq!(
        second - first,
        2,
        "the first land drop fires both the bounded and the unbounded draw"
    );
    assert_eq!(
        st.hand.len() - second,
        1,
        "the second land drop still fires the unbounded sibling"
    );
}

/// A counter-pump win condition works through normal casting and upkeep.
#[test]
fn counter_pump_win_threshold_fires() {
    let reactor = card(
        "Darksteel Reactor",
        "{0}",
        "Artifact",
        "Indestructible (Effects that say \"destroy\" don't destroy this artifact.)\nAt the beginning of your upkeep, you may put a charge counter on this artifact.\nWhen this artifact has twenty or more charge counters on it, you win the game.",
    );
    let deck = SimDeck {
        companion: None,
        cards: vec![parse_sim_card(&reactor)],
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    // The reactor's pump and win trigger parse to the runtime model.
    let sim = &deck.cards[0];
    let pump = sim
        .unlocked_abilities(0)
        .find(|a| matches!(a.effect, SimEffect::Counters(_)))
        .expect("the upkeep pump parses");
    assert!(
        matches!(pump.effect, SimEffect::Counters(1)),
        "the upkeep pump lowers: {:?}",
        pump.effect
    );
    let win = sim
        .unlocked_abilities(0)
        .find(|a| matches!(a.effect, SimEffect::WinThreshold { .. }))
        .expect("the state-trigger win lowers");
    assert_eq!(
        win.trigger,
        super::model::SimTrigger::Upkeep,
        "a state-trigger win check normalizes to the upkeep check"
    );
    let mut st = super::turn_loop_tests::state(vec![0], vec![]);
    let mut census = super::game_run::TurnCensus::new(22, 1);
    let commander = super::game_commander::CommanderProfile::new(&deck);
    let mut repeatable_sources = Vec::new();
    for turn in 1..=22 {
        super::game_run::run_turn(
            &deck,
            &mut st,
            &mut census,
            &commander,
            &mut repeatable_sources,
            turn,
            false,
        );
    }
    assert_eq!(
        repeatable_sources.len(),
        1,
        "the upkeep source registers once"
    );
    assert_eq!(st.battlefield[0].counters.charge, 21);
    assert_eq!(
        census.win_threshold_turn,
        Some(21),
        "the reactor wins the game at twenty charge counters"
    );
}

/// A counter-placement trigger is not a win-state trigger: "Whenever
/// one or more counters are put on this permanent, draw a card." must
/// not lower to a fabricated win threshold (any counter would "win")
/// and keeps its real effect.
#[test]
fn one_or_more_counters_trigger_is_not_a_win() {
    let watcher = card(
        "Counter Watcher",
        "{2}{U}",
        "Creature",
        "Whenever one or more +1/+1 counters are put on this creature, draw a card.",
    );
    let sim = parse_sim_card(&watcher);
    assert!(
        !sim.unlocked_abilities(0)
            .any(|a| matches!(a.effect, SimEffect::WinThreshold { .. })),
        "a counter-placement trigger never parses as a win threshold"
    );
    // The shape also gates a win-worded event on the state form: a
    // threshold event must name "counters on it" and resolve in a win.
    let nonwin = parse_sim_card(&card(
        "Counter Keeper",
        "{2}",
        "Artifact",
        "When this artifact has ten or more charge counters on it, sacrifice it.",
    ));
    assert!(
        !nonwin
            .unlocked_abilities(0)
            .any(|a| matches!(a.effect, SimEffect::WinThreshold { .. })),
        "a non-win threshold trigger never parses as a win"
    );
}

/// Interaction readiness spends only legal mana: a creature-only
/// bucket never marks an instant ready, and an instant/sorcery bucket
/// + general mana does.
#[test]
fn interaction_readiness_spends_only_instant_legal_mana() {
    let answer = card(
        "Test Counterspell",
        "{U}{U}",
        "Instant",
        "Counter target spell.",
    );
    let cards = vec![parse_sim_card(&answer)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut census = super::game_run::TurnCensus::new(1, 1);
    let st = super::turn_loop_tests::state(vec![0], vec![]);
    // Creature-only mana cannot fund the instant: readiness stays off.
    let pool = super::game::ManaPool {
        creature_only: 3,
        ..super::game::ManaPool::default()
    };
    super::game_run::census::record_interaction_readiness(&deck, &st, &pool, &mut census, 1);
    assert!(
        !census.interaction_ready[0],
        "creature-only mana never readies an instant"
    );
    // The instant/sorcery bucket covers the pips: readiness turns on.
    let pool = super::game::ManaPool {
        instant_sorcery_only: 2,
        ..super::game::ManaPool::default()
    };
    super::game_run::census::record_interaction_readiness(&deck, &st, &pool, &mut census, 1);
    assert!(
        census.interaction_ready[0],
        "instant/sorcery mana readies the answer"
    );
    assert_eq!(
        census.interaction_mana_held[0], 0.0,
        "the exact bucket covers the answer with nothing held"
    );
}

/// A card's own printed flashback cost casts it from the graveyard at
/// the flashback cost, then exiles it (CR 702.34).
#[test]
fn own_flashback_casts_from_the_graveyard() {
    let looting = card(
        "Test Looting",
        "{1}{R}",
        "Sorcery",
        "Draw two cards, then discard a card.\nFlashback {2}{R}",
    );
    let cards = vec![parse_sim_card(&looting)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let sim = &deck.cards[0];
    assert_eq!(
        sim.spell_data
            .own_flashback
            .as_ref()
            .map(|cost| cost.total()),
        Some(3),
        "the printed flashback cost parses"
    );
    // The instance in the graveyard casts for the flashback cost.
    let mut st = super::turn_loop_tests::state(vec![], vec![1]);
    st.graveyard.push(crate::deck::simulator::model::CardIdx(0));
    st.seen = st.hand.len() as u32 + st.library.len() as u32;
    let mut pool = super::game::ManaPool {
        flexible: 3,
        ..super::game::ManaPool::default()
    };
    let mut mana_spent = [0.0];
    let mut engines = Vec::new();
    super::cast_pass::cast_graveyard_spells(
        &deck,
        &mut st,
        &mut pool,
        1,
        &mut mana_spent,
        &mut engines,
    );
    assert!(
        !st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0)),
        "the flashback cast exiles the instance"
    );
    assert_eq!(st.replay_casts, 1, "the flashback cast counts as a replay");
    assert!(
        st.exile
            .contains(&crate::deck::simulator::model::CardIdx(0))
    );
}

/// A card's own printed escape cost casts it from the graveyard by
/// paying the escape cost and exiling three other cards (CR 702.138).
#[test]
fn own_escape_pays_the_printed_cost_and_exiles_fodder() {
    let ogre = card(
        "Test Ogre",
        "{2}{R}",
        "Creature — Ogre",
        "Trample\nEscape—{3}{R}, Exile three other cards from your graveyard.",
    );
    let fodder = card("Fodder", "{1}", "Creature — Rat", "");
    let cards = vec![
        parse_sim_card(&ogre),
        parse_sim_card(&fodder),
        parse_sim_card(&fodder),
        parse_sim_card(&fodder),
    ];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let ogre_sim = &deck.cards[0];
    assert_eq!(
        ogre_sim
            .spell_data
            .own_escape
            .as_ref()
            .map(|cost| cost.total()),
        Some(4),
        "the printed escape cost parses"
    );
    let mut st = super::turn_loop_tests::state(vec![], vec![30, 31, 32, 33]);
    st.graveyard = vec![
        crate::deck::simulator::model::CardIdx(0),
        crate::deck::simulator::model::CardIdx(1),
        crate::deck::simulator::model::CardIdx(2),
        crate::deck::simulator::model::CardIdx(3),
    ];
    st.seen = st.hand.len() as u32 + st.library.len() as u32;
    let mut pool = super::game::ManaPool {
        flexible: 4,
        ..super::game::ManaPool::default()
    };
    let mut mana_spent = [0.0];
    let mut engines = Vec::new();
    super::cast_pass::cast_graveyard_spells(
        &deck,
        &mut st,
        &mut pool,
        1,
        &mut mana_spent,
        &mut engines,
    );
    assert_eq!(st.replay_casts, 1, "the escape cast counts as a replay");
    assert!(
        !st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0)),
        "the escaped creature left the graveyard"
    );
    assert_eq!(
        st.exile.len(),
        3,
        "the escape cost exiled three other cards, not the creature"
    );
    // The creature joined the battlefield.
    assert!(
        st.battlefield.iter().any(|perm| perm
            .card
            .deck_idx()
            .is_some_and(|idx| idx == crate::deck::simulator::model::CardIdx(0))),
        "the escaped creature entered the battlefield"
    );
}

/// A grant shape ("...gain flashback") without a printed cost never
/// parses a self cost, and the granted path still casts at the card's
/// own cost.
#[test]
fn flashback_grant_text_does_not_read_a_self_cost() {
    let granter = card(
        "Test Past",
        "{4}{R}",
        "Sorcery",
        "Exile all cards from your graveyard. Until end of turn, you may play cards exiled this way.",
    );
    let looting = card("Test Looting", "{1}{R}", "Sorcery", "Flashback {2}{R}");
    let cards = vec![parse_sim_card(&granter), parse_sim_card(&looting)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    assert!(
        deck.cards[0].spell_data.own_flashback.is_none(),
        "grant shapes carry no self cost"
    );
    assert_eq!(
        deck.cards[1]
            .spell_data
            .own_flashback
            .as_ref()
            .map(|cost| cost.total()),
        Some(3)
    );
}

/// A multi-effect trigger lowers one runtime ability per supported
/// effect: "draw a card and create a Treasure token" banks the draw
/// and the token; "each opponent loses 1 life and you gain 1 life"
/// drains and gains.
#[test]
fn multi_effect_triggers_lower_every_clause() {
    let treasure = card(
        "Test Cache",
        "{2}{G}",
        "Creature — Elf",
        "When this creature enters, draw a card and create a Treasure token.",
    );
    let drain = card(
        "Test Sting",
        "{2}{B}",
        "Creature — Zombie",
        "Whenever this creature enters, each opponent loses 1 life and you gain 1 life.",
    );
    let cards = vec![parse_sim_card(&treasure), parse_sim_card(&drain)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Commander,
        rules: super::format::rules_for("commander"),
    };
    let cache = &deck.cards[0];
    let entries: Vec<_> = cache
        .unlocked_abilities(0)
        .filter(|a| a.trigger == super::model::SimTrigger::Enters)
        .collect();
    assert_eq!(entries.len(), 1, "one trigger has one runtime identity");
    assert!(matches!(
        entries[0].effect_sequence(),
        [SimEffect::Draw(1), SimEffect::Treasures(1)]
    ));
    let sting = &deck.cards[1];
    let entries: Vec<_> = sting
        .unlocked_abilities(0)
        .filter(|a| a.trigger == super::model::SimTrigger::Enters)
        .collect();
    assert_eq!(entries.len(), 1);
    assert!(matches!(
        entries[0].effect_sequence(),
        [SimEffect::LoseLife { .. }, SimEffect::GainLife(1)]
    ));
}

/// A phyrexian pip pays with 2 life (CR 107.4f): `{1}{B/P}` casts with
/// colorless-only mana, charging 2 life; a life-starved cast stays in
/// hand.
#[test]
fn phyrexian_pip_pays_life() {
    let spell = card("Life-Paid Spell", "{1}{B/P}", "Sorcery", "Draw a card.");
    let cards = vec![parse_sim_card(&spell)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let sim = &deck.cards[0];
    assert_eq!(sim.cost.phyrexian[2], 1, "the phyrexian pip parses");
    // Colorless-only mana + 2 life covers the cast.
    let mut st = super::turn_loop_tests::state(vec![0], vec![]);
    st.life = 10;
    let mut pool = super::game::ManaPool {
        colorless: 1,
        ..super::game::ManaPool::default()
    };
    let mut engines = Vec::new();
    let mut mana_spent = [0.0];
    let mut pip_blocks = Vec::new();
    let mut blocked_colors = [false; 5];
    super::cast_pass::cast_pass(
        &deck,
        &mut st,
        &mut pool,
        1,
        &mut mana_spent,
        &mut engines,
        &mut pip_blocks,
        &mut blocked_colors,
    );
    assert_eq!(st.life, 8, "the phyrexian pip charged 2 life");
    assert_eq!(st.life_paid, 2, "the life payment lands in the life ledger");
    assert!(
        st.battlefield_seen.is_empty() && st.hand.is_empty(),
        "the cast left the hand"
    );
    // A life-starved agent (2 life) cannot pay the pip.
    let mut st = super::turn_loop_tests::state(vec![0], vec![]);
    st.life = 2;
    let mut pool = super::game::ManaPool {
        colorless: 1,
        ..super::game::ManaPool::default()
    };
    let mut engines = Vec::new();
    let mut mana_spent = [0.0];
    let mut pip_blocks = Vec::new();
    let mut blocked_colors = [false; 5];
    super::cast_pass::cast_pass(
        &deck,
        &mut st,
        &mut pool,
        1,
        &mut mana_spent,
        &mut engines,
        &mut pip_blocks,
        &mut blocked_colors,
    );
    assert_eq!(st.life, 2, "a cast at lethal life never pays the pip");
    assert!(
        st.hand.contains(&crate::deck::simulator::model::CardIdx(0)),
        "the unpayable cast stays in hand"
    );
}

/// A "doesn't untap during your untap step" static keeps the permanent
/// tapped across the turn boundary (Basalt Monolith class): no free
/// mana from an untap the card never gets.
#[test]
fn doesnt_untap_static_survives_the_untap_step() {
    let basalt = card(
        "Basalt Monolith",
        "{3}",
        "Artifact",
        "Basalt Monolith doesn't untap during your untap step.\n{T}: Add {C}{C}{C}.\n{3}: Untap this artifact.",
    );
    let cards = vec![parse_sim_card(&basalt)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    assert!(
        deck.cards[0].flags.doesnt_untap,
        "the doesn't-untap static parses into a flag"
    );
    let mut st = super::turn_loop_tests::state(vec![], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    st.battlefield[0].tapped = true;
    // The turn-boundary untap leaves a doesn't-untap permanent tapped:
    // the pool builds nothing from it the next turn.
    let pool = super::game_run::build_pool(&deck, &mut st, 2);
    assert_eq!(
        pool.total(),
        0,
        "a tapped doesn't-untap permanent yields nothing while tapped"
    );
}

/// A static mana grant converts every matching permanent's tap: five
/// lands with a Chromatic Lantern yield five any-color pips (the same
/// tap, one mana), not fixed pips plus a capped bonus.
#[test]
fn static_grant_converts_every_land_tap() {
    let lantern = card(
        "Test Lantern",
        "{2}",
        "Artifact",
        "Lands you control have \"{T}: Add one mana of any color.\"\n{T}: Add one mana of any color.",
    );
    let land = card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)");
    let cards = vec![
        parse_sim_card(&lantern),
        parse_sim_card(&land),
        parse_sim_card(&land),
        parse_sim_card(&land),
        parse_sim_card(&land),
        parse_sim_card(&land),
    ];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut st = super::turn_loop_tests::state(vec![], vec![]);
    st.battlefield.push(super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    for i in 1..6 {
        st.battlefield.push(super::game::new_perm_with(
            1 + i,
            &deck,
            crate::deck::simulator::model::CardIdx(i),
            0,
            false,
        ));
    }
    let pool = super::game_run::build_pool(&deck, &mut st, 1);
    assert_eq!(
        pool.flexible, 6,
        "the lantern and the five lands each yield one any-color pip"
    );
    assert_eq!(
        pool.fixed.iter().sum::<u32>(),
        0,
        "the grant replaces the lands' printed fixed pips"
    );
}

/// A phyrexian activation pip pays with 2 life (CR 107.4f): the
/// resolution charges the life and spends no pool mana for the pip,
/// and a life-starved board skips the activation.
#[test]
fn phyrexian_activation_charges_life() {
    let drain_mage = card(
        "Pip Mage",
        "{2}{B}",
        "Creature — Wizard",
        "{2}{B/P}, {T}: Draw a card.",
    );
    let cards = vec![parse_sim_card(&drain_mage)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut st = super::turn_loop_tests::state(vec![0], vec![10, 11]);
    st.seen = st.hand.len() as u32 + st.library.len() as u32;
    st.battlefield.push(super::game::new_perm_with(
        1,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    st.battlefield[0].summoning_sick = false;
    let mut pool = super::game::ManaPool {
        colorless: 6,
        ..super::game::ManaPool::default()
    };
    // Cost {2}{B/P}: gate needs 2 mana + life above 2.
    let hand_before = st.hand.len();
    let activation = super::game_effects::activation::pick_best_activation(&deck, &st, &pool, 1)
        .expect("the pip activation is usable with 6 mana and full life");
    super::game_effects::activation::resolve_activation(&deck, &mut st, &mut pool, 1, &activation);
    assert_eq!(st.life, 18, "the phyrexian pip charged 2 life");
    assert_eq!(
        pool.colorless, 4,
        "only the generic part came from the pool"
    );
    assert_eq!(
        st.hand.len(),
        hand_before + 1,
        "the activation resolved its draw"
    );
    // Life at the charge level: the activation skips instead of
    // driving life to zero.
    let mut low = super::turn_loop_tests::state(vec![0], vec![]);
    low.seen = low.hand.len() as u32 + low.library.len() as u32;
    low.battlefield.push(super::game::new_perm_with(
        2,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    low.battlefield[0].summoning_sick = false;
    let pool = super::game::ManaPool {
        colorless: 6,
        ..super::game::ManaPool::default()
    };
    low.life = 2;
    assert!(
        super::game_effects::activation::pick_best_activation(&deck, &low, &pool, 1).is_none(),
        "life at the charge level cannot fire the pip activation"
    );
}
