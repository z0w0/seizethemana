use super::*;

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

/// A mass sacrifice removes only the original creatures. Death-trigger tokens
/// survive, and both dying sources see the other creature die.
#[test]
fn graveyard_exchange_preserves_tokens_created_by_simultaneous_deaths() {
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
            "Whenever another creature you control dies, create two 1/1 creature tokens.",
        ),
        super::turn_loop_tests::row(
            "Second Death Token Maker",
            "{3}",
            "Creature — Zombie",
            "Whenever another creature you control dies, create two 1/1 creature tokens.",
        ),
    ];
    let cards = super::turn_loop_tests::deck(&rows);
    for index in [3, 4] {
        assert!(
            cards.cards[index]
                .striations
                .iter()
                .flat_map(|tier| &tier.abilities)
                .any(|ability| ability.trigger == super::model::SimTrigger::Dies
                    && matches!(
                        ability.effect_sequence(),
                        [super::model::SimEffect::Tokens(2)]
                    ))
        );
    }
    assert!(cards.cards[1].spell_data.graveyard_creature_exchange);
    let mut st = super::turn_loop_tests::state(vec![0], vec![1, 2]);
    for index in [3, 4] {
        let uid = super::game::take_uid(&mut st);
        st.battlefield.push(super::game::new_perm_with(
            uid,
            &cards,
            crate::deck::simulator::model::CardIdx(index),
            1,
            false,
        ));
    }
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
    assert_eq!(st.battlefield.len(), 4);
    assert!(
        st.battlefield
            .iter()
            .all(|permanent| matches!(permanent.card, super::game::CardRef::Token(_)))
    );
    assert_eq!(st.graveyard, [3, 4, 1, 0].map(super::model::CardIdx));
    assert!(st.exile.is_empty());
}

#[test]
fn empty_library_only_loses_after_a_failed_draw() {
    let empty_deck = super::turn_loop_tests::deck(&[]);
    let mut st = super::turn_loop_tests::state(vec![], vec![]);
    st.current_turn = 4;
    st.current_phase = super::game::GamePhase::Draw;

    super::game_effects::apply_effect_at(&empty_deck, &SimEffect::Mill(1), &mut st, 4, false, None);
    assert_eq!(
        st.completion, None,
        "milling an empty library is not a failed draw"
    );

    assert!(!super::game_effects::draw_one(&empty_deck, &mut st, 4));
    assert_eq!(
        st.completion,
        Some(super::game::GameCompletion::PlayerLost {
            reasons: vec![super::game::PlayerLossReason::FailedDraw],
            turn: 4,
            phase: super::game::GamePhase::Draw,
        })
    );
}

#[test]
fn failed_draw_remains_terminal_after_library_refill_in_the_same_resolution() {
    let deck = super::turn_loop_tests::deck(&[super::turn_loop_tests::row(
        "Returned card",
        "{1}",
        "Sorcery",
        "",
    )]);
    let mut st = super::turn_loop_tests::state(vec![], vec![]);
    st.current_turn = 2;
    st.current_phase = super::game::GamePhase::Main;
    super::resolution::begin_resolution(&mut st);
    assert!(!super::game_effects::draw_one(&deck, &mut st, 2));
    st.library.push(CardIdx(0));
    super::resolution::end_resolution(&deck, &mut st, None);

    assert_eq!(st.library, [CardIdx(0)]);
    assert!(matches!(
        &st.completion,
        Some(super::game::GameCompletion::PlayerLost {
            reasons,
            turn: 2,
            phase: super::game::GamePhase::Main,
        }) if reasons == &[super::game::PlayerLossReason::FailedDraw]
    ));
    let mut census = super::game_run::TurnCensus::new(3, deck.cards.len());
    census.turn_observed[0] = true;
    census.turn_observations[0] = super::game::TurnObservation {
        draw: true,
        main: true,
        combat: true,
        end_step: true,
        end_turn: true,
    };
    census.turn_observations[1].draw = true;
    let log = super::game_run::census::finish_log(&deck, st, census, 0, false, 3);
    assert_eq!(log.turns_observed, 1);
    assert!(matches!(
        &log.completion,
        super::game::GameCompletion::PlayerLost {
            reasons,
            turn: 2,
            phase: super::game::GamePhase::Main,
        } if reasons == &[super::game::PlayerLossReason::FailedDraw]
    ));
    let stats = super::aggregate::aggregate(&[log], &deck, 3);
    assert_eq!(stats.runs, 1);
    assert_eq!(stats.finished_games, 1);
    assert_eq!(stats.player_loss_games, 1);
    assert_eq!(stats.phase_observations[0].end_turn, 1);
    assert_eq!(stats.phase_observations[1].draw, 1);
    assert_eq!(stats.phase_observations[1].main, 0);
    assert_eq!(stats.phase_observations[2].combat, 0);
}

#[test]
fn zero_life_ends_at_the_resolution_boundary_and_blocks_later_effects() {
    let deck = super::turn_loop_tests::deck(&[]);
    let mut st = super::turn_loop_tests::state(vec![], vec![]);
    st.life = 1;
    st.current_turn = 3;
    st.current_phase = super::game::GamePhase::Main;

    super::game_effects::apply_effect_at(
        &deck,
        &SimEffect::LoseLife {
            amount: 1,
            scope: super::model::LifeLossScope::SelfOnly,
        },
        &mut st,
        3,
        false,
        None,
    );
    assert!(matches!(
        &st.completion,
        Some(super::game::GameCompletion::PlayerLost {
            reasons,
            turn: 3,
            phase: super::game::GamePhase::Main,
        }) if reasons == &[super::game::PlayerLossReason::ZeroLife]
    ));

    super::game_effects::apply_effect_at(&deck, &SimEffect::GainLife(5), &mut st, 3, false, None);
    assert_eq!(st.life, 0);
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
        .insert((crate::deck::simulator::model::CardIdx(0), 0));
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
    // CR 119.4: life at 4 may pay the additional 4 (down to 0), so the
    // spell casts and the cost is paid.
    assert!(
        !st.graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0)),
        "life equal to the cost pays it: {:?}",
        st.graveyard
    );
    assert_eq!(st.life, 0, "the additional cost was paid");
    // One life short: the gate holds and the spell stays in the graveyard.
    let mut st_short = super::turn_loop_tests::state(vec![], vec![1]);
    st_short
        .graveyard
        .push(crate::deck::simulator::model::CardIdx(0));
    st_short
        .flashback_permissions
        .insert((crate::deck::simulator::model::CardIdx(0), 0));
    st_short.life = 3;
    let mut pool_short = super::game::ManaPool {
        flexible: 4,
        ..super::game::ManaPool::default()
    };
    super::cast_pass::cast_graveyard_spells(
        &cards,
        &mut st_short,
        &mut pool_short,
        1,
        &mut [0.0],
        &mut Vec::new(),
    );
    assert!(
        st_short
            .graveyard
            .contains(&crate::deck::simulator::model::CardIdx(0)),
        "life below the cost keeps the spell in the graveyard: {:?}",
        st_short.graveyard
    );
    assert_eq!(st_short.life, 3, "no additional life was paid");
    // Life above the cost also casts (the prior behavior).
    let mut st_ok = super::turn_loop_tests::state(vec![], vec![1]);
    st_ok
        .graveyard
        .push(crate::deck::simulator::model::CardIdx(0));
    st_ok
        .flashback_permissions
        .insert((crate::deck::simulator::model::CardIdx(0), 0));
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
        land_face: false,
        prowess_bonus: 0,
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
    let mut cards = vec![parse_sim_card(&reactor)];
    cards.extend(
        (0..40)
            .map(|_| parse_sim_card(&card("Plains", "", "Basic Land — Plains", "{T}: Add {W}."))),
    );
    let deck = SimDeck {
        companion: None,
        cards,
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
    let mut st = super::turn_loop_tests::state(vec![0], (1..=40).collect());
    let mut census = super::game_run::TurnCensus::new(22, deck.cards.len());
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
