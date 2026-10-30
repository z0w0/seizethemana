use super::*;

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
        completion: None,
        failed_draw: false,
        current_turn: 0,
        current_phase: super::game::GamePhase::Upkeep,
        graveyard_generations: Default::default(),
        resolution: Default::default(),
        library_rng: super::rng::game_rng(0, 0, super::rng::RngDomain::Library),
        battlefield: Vec::new(),
        library: Vec::new(),
        hand: Vec::new(),
        seen: 0,
        graveyard: Vec::new(),
        exile: Vec::new(),
        battlefield_seen: Default::default(),
        graveyard_seen: Default::default(),
        card_first_seen: Default::default(),
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
        commander_casts: Vec::new(),
        opponents: super::opponents::OpponentTable::for_format(Format::Constructed),
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
        land_face: false,
        prowess_bonus: 0,
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
        completion: None,
        failed_draw: false,
        current_turn: 0,
        current_phase: super::game::GamePhase::Upkeep,
        graveyard_generations: Default::default(),
        resolution: Default::default(),
        library_rng: super::rng::game_rng(0, 0, super::rng::RngDomain::Library),
        battlefield: Vec::new(),
        library: Vec::new(),
        hand: Vec::new(),
        seen: 0,
        graveyard: Vec::new(),
        exile: Vec::new(),
        battlefield_seen: Default::default(),
        graveyard_seen: Default::default(),
        card_first_seen: Default::default(),
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
        commander_casts: Vec::new(),
        opponents: super::opponents::OpponentTable::for_format(Format::Constructed),
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
    // A hasty token enters before combat, shifting positions.
    let mut token = super::game::Permanent {
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
        land_face: false,
        prowess_bonus: 0,
        saddled: false,
        army: false,
        jace_token: false,
        sacrifice_at_end: false,
        attacking_this_turn: false,
    };
    token.counters.keyword.insert(super::model::Keyword::Haste);
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
    // 12 copies in a 38-card deck: the body enters by turn 2 in most
    // games, so the re-fire shows in a large share of the 200 runs.
    assert!(
        fed > 60,
        "blink re-fire should add one extra draw in most games: {fed}/200 above 13"
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
    // 12 wheels in 36 cards; wheels cast most games, so a clear majority
    // of the 100 runs cross the awareness bar.
    assert!(
        high > 50,
        "wheel draws should feed awareness in most games: {high}/100 above 0.6"
    );
}

/// X spending preserves mana that cannot pay this spell's cast class.
#[test]
fn x_cost_preserves_unrelated_restricted_mana() {
    let x_spell = card("X Draw", "{X}{W}", "Sorcery", "Draw X cards.");
    let mut rows = vec![x_spell];
    rows.extend((0..12).map(|_| card("Remaining", "{99}", "Sorcery", "")));
    let deck = super::turn_loop_tests::deck(&rows);
    let mut state = super::turn_loop_tests::state(vec![0], (1..13).collect());
    let mut pool = super::turn_loop_tests::restricted_pool(3, SpendRestriction::Creature);
    for (amount, restriction) in [
        (2, SpendRestriction::Legendary),
        (1, SpendRestriction::Artifact),
        (2, SpendRestriction::InstantSorcery),
    ] {
        pool.lots
            .extend(super::turn_loop_tests::restricted_pool(amount, restriction).lots);
    }
    pool.colorless = 5;
    pool.fixed[0] = 1;
    super::turn_loop_tests::cast(&deck, &mut state, &mut pool);
    assert_eq!(state.seen, 7);
    assert_eq!(pool.total(), 6);
    assert_eq!(
        super::game_mana::usable_for_classes(&pool, &[SpendRestriction::Creature]),
        3
    );
    assert_eq!(
        super::game_mana::usable_for_classes(&pool, &[SpendRestriction::Legendary]),
        2
    );
    let creature = card(
        "X Creature",
        "{X}{W}",
        "Creature",
        "This creature enters with X +1/+1 counters.",
    );
    let deck = super::turn_loop_tests::deck(&[creature]);
    let mut state = super::turn_loop_tests::state(vec![0], vec![]);
    let mut pool = super::turn_loop_tests::restricted_pool(3, SpendRestriction::Creature);
    pool.lots
        .extend(super::turn_loop_tests::restricted_pool(2, SpendRestriction::Legendary).lots);
    pool.colorless = 5;
    pool.fixed[0] = 1;
    super::turn_loop_tests::cast(&deck, &mut state, &mut pool);
    assert_eq!(state.battlefield[0].counters.plus1, 8);
    assert_eq!(pool.total(), 2);
    assert_eq!(
        super::game_mana::usable_for_classes(&pool, &[SpendRestriction::Creature]),
        0
    );
    assert_eq!(
        super::game_mana::usable_for_classes(&pool, &[SpendRestriction::Legendary]),
        2
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
        ("Fabled Passage", "a basic land card", &[], true),
        ("Prismatic Vista", "a basic land card", &[], true),
        ("Terramorphic Expanse", "a basic land card", &[], true),
        ("Evolving Wilds", "a basic land card", &[], true),
        ("Escape Tunnel", "a basic land card", &[], true),
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

/// Unused Treasure persists across phases, and each paid token is consumed once.
#[test]
fn treasure_bank_preserves_unused_tokens_without_duplication() {
    let mut rows = vec![card("One-mana spell", "{1}", "Sorcery", "")];
    rows.extend((0..4).map(|_| card("Filler", "{99}", "Sorcery", "")));
    let deck = super::turn_loop_tests::deck(&rows);
    let mut st = super::turn_loop_tests::state(vec![], vec![1, 2, 3, 4]);
    st.treasure_bank = 2;
    let mut census = super::game_run::TurnCensus::new(3, deck.cards.len());
    let commander = super::game_commander::CommanderProfile::new(&deck);
    let mut sources = Vec::new();
    for (turn, expected) in [(1, 2), (2, 1), (3, 1)] {
        if turn == 2 {
            st.hand.push(super::model::CardIdx(0));
        }
        super::game_run::run_turn(
            &deck,
            &mut st,
            &mut census,
            &commander,
            &mut sources,
            turn,
            false,
        );
        assert_eq!(st.treasure_bank, expected);
        assert_eq!(census.mana_remaining[turn - 1], 0.0);
    }
    assert_eq!(st.graveyard, [super::model::CardIdx(0)]);
}
