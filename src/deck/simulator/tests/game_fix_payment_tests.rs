use super::*;

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
    let mut st = super::turn_loop_tests::state(vec![0], vec![]);
    // Creature-only mana cannot fund the instant: readiness stays off.
    let pool = super::turn_loop_tests::restricted_pool(3, SpendRestriction::Creature);
    super::game_run::census::record_interaction_readiness(&deck, &st, &pool, &mut census, 1);
    assert!(
        !census.interaction_ready[0],
        "creature-only mana never readies an instant"
    );
    // The instant/sorcery bucket covers the pips: readiness turns on.
    let pool = super::turn_loop_tests::restricted_pool(2, SpendRestriction::InstantSorcery);
    super::game_run::census::record_interaction_readiness(&deck, &st, &pool, &mut census, 1);
    assert!(
        census.interaction_ready[0],
        "instant/sorcery mana readies the answer"
    );
    assert_eq!(
        census.interaction_mana_held[0], 0.0,
        "the exact bucket covers the answer with nothing held"
    );
    let mut cast_pool = super::game::ManaPool {
        flexible: 2,
        ..super::game::ManaPool::default()
    };
    super::cast_pass::cast_pass(
        &deck,
        &mut st,
        &mut cast_pool,
        1,
        &mut [0.0],
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    );
    assert_eq!(
        st.hand.len(),
        1,
        "a counterspell stays in hand when no spell is on the stack"
    );
}

/// Interaction readiness checks colored pips and chooses a payable answer
/// when the cheapest answer in hand cannot use the available colors.
#[test]
fn interaction_readiness_requires_pips_and_checks_each_answer() {
    let double_blue = card(
        "Double Blue Answer",
        "{U}{U}",
        "Instant",
        "Counter target spell.",
    );
    let blue_and_generic = card("Blue Answer", "{1}{U}", "Instant", "Counter target spell.");
    let cards = vec![
        parse_sim_card(&double_blue),
        parse_sim_card(&blue_and_generic),
    ];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let st = super::turn_loop_tests::state(vec![0, 1], vec![]);
    let mut census = super::game_run::TurnCensus::new(1, 2);

    let missing_blue = super::game::ManaPool {
        colorless: 2,
        ..super::game::ManaPool::default()
    };
    super::game_run::census::record_interaction_readiness(
        &deck,
        &st,
        &missing_blue,
        &mut census,
        1,
    );
    assert!(
        !census.interaction_ready[0],
        "colorless mana cannot pay two blue pips"
    );

    let blue_and_generic_pool = super::game::ManaPool {
        fixed: [0, 1, 0, 0, 0],
        colorless: 1,
        ..super::game::ManaPool::default()
    };
    super::game_run::census::record_interaction_readiness(
        &deck,
        &st,
        &blue_and_generic_pool,
        &mut census,
        1,
    );
    assert!(
        census.interaction_ready[0],
        "the one-blue answer is payable even though the cheaper answer is not"
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
    let cards = vec![
        parse_sim_card(&looting),
        parse_sim_card(&card("First flashback draw", "{99}", "Sorcery", "")),
        parse_sim_card(&card("Second flashback draw", "{99}", "Sorcery", "")),
    ];
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
    let mut st = super::turn_loop_tests::state(vec![], vec![1, 2]);
    st.graveyard.push(crate::deck::simulator::model::CardIdx(0));
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
    assert_eq!(st.exile, [CardIdx(0)]);
    assert_eq!(st.hand, [CardIdx(2)]);
    assert_eq!(st.graveyard, [CardIdx(1)]);
    assert!(st.library.is_empty());
    assert_eq!((st.seen, st.awareness_cards, st.milled_self), (2, 2, 0));
    assert!(!st.failed_draw);
    assert!(st.completion.is_none());
    assert_eq!(mana_spent, [3.0]);
    assert_eq!(pool.total(), 0);
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
    assert_eq!(st.life, 0, "a cast at exactly the pip's life cost pays it");
    assert!(
        !st.hand.contains(&crate::deck::simulator::model::CardIdx(0)),
        "the cast resolves and leaves the hand"
    );
    // One life short of the pip cost: the cast stays in hand.
    let mut st = super::turn_loop_tests::state(vec![0], vec![]);
    st.life = 1;
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
    assert_eq!(st.life, 1, "a life-starved cast never pays the pip");
    assert!(
        st.hand.contains(&crate::deck::simulator::model::CardIdx(0)),
        "the unpayable cast stays in hand"
    );
}

/// A phyrexian pip and an additional life cost pay from one total
/// (CR 119.4): a spell with both needs life for the whole amount, so a
/// pool short of the sum cannot drive life negative.
#[test]
fn combined_life_costs_gate_on_the_total() {
    let spell = card(
        "Life-Paid Spell",
        "{1}{B/P}",
        "Sorcery",
        "As an additional cost to cast this spell, pay 2 life. Draw a card.",
    );
    let cards = vec![parse_sim_card(&spell)];
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    assert_eq!(
        deck.cards[0].spell_data.additional_cost_life, 2,
        "the pay-2-life cost parses"
    );
    let run = |life: i32| {
        let mut st = super::turn_loop_tests::state(vec![0], vec![]);
        st.life = life;
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
        st
    };
    let paid = run(4);
    assert_eq!(paid.life, 0, "the combined 4-life cost was paid exactly");
    assert_eq!(paid.life_paid, 4);
    assert!(
        paid.hand.is_empty(),
        "the cast resolves and leaves the hand"
    );
    let short = run(3);
    assert_eq!(short.life, 3, "a life-starved cast never partially pays");
    assert!(
        short
            .hand
            .contains(&crate::deck::simulator::model::CardIdx(0)),
        "a cast short of the combined cost stays in hand"
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
    // CR 119.4: life exactly equal to the pip's life charge pays it (down
    // to 0), so the activation still fires.
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
        super::game_effects::activation::pick_best_activation(&deck, &low, &pool, 1).is_some(),
        "life equal to the pip charge still fires the activation"
    );
    // One life short: the activation skips instead of driving life below
    // zero.
    let mut very_low = super::turn_loop_tests::state(vec![0], vec![]);
    very_low.seen = very_low.hand.len() as u32 + very_low.library.len() as u32;
    very_low.battlefield.push(super::game::new_perm_with(
        2,
        &deck,
        crate::deck::simulator::model::CardIdx(0),
        0,
        false,
    ));
    very_low.battlefield[0].summoning_sick = false;
    let pool = super::game::ManaPool {
        colorless: 6,
        ..super::game::ManaPool::default()
    };
    very_low.life = 1;
    assert!(
        super::game_effects::activation::pick_best_activation(&deck, &very_low, &pool, 1).is_none(),
        "life below the pip charge cannot fire the activation"
    );
}

/// CR 704.5j: the legend rule keeps one legendary permanent per name. A
/// second copy of the same legend leaves the battlefield (fires Dies
/// triggers) instead of stacking.
#[test]
fn legend_rule_keeps_one_copy_per_name() {
    let legend = card(
        "Legendary Bear",
        "{2}{G}",
        "Legendary Creature — Bear",
        "When this creature dies, draw a card.",
    );
    let filler = card("Bystander", "{1}", "Creature — Human", "");
    let cards = deck_from(&[(legend, 2), (filler, 1)]);
    let mut st = super::turn_loop_tests::state(vec![], vec![2]);
    st.battlefield
        .push(super::game::new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(super::game::new_perm_with(2, &cards, CardIdx(1), 0, false));
    st.battlefield
        .push(super::game::new_perm_with(3, &cards, CardIdx(2), 0, false));
    super::game_effects::enforce_legend_rule(&cards, &mut st, 1);
    assert_eq!(st.battlefield.len(), 2, "one duplicate legend was removed");
    assert_eq!(
        st.graveyard,
        [CardIdx(1)].to_vec(),
        "the later legend copy went to the graveyard"
    );
    // The surviving copy is the first (uid 1), not the removed uid 2.
    assert!(st.battlefield.iter().any(|perm| perm.uid == 1));
    assert!(!st.battlefield.iter().any(|perm| perm.uid == 2));
    // The Dies trigger on the removed legend fired (CR 603.6).
    assert_eq!(st.hand.len(), 1, "the legend's Dies draw fired");
}

/// Distinct legendary names coexist: the legend rule is per name.
#[test]
fn legend_rule_allows_distinct_names() {
    let first = card("Legend One", "{2}{G}", "Legendary Creature — Bear", "");
    let second = card("Legend Two", "{2}{G}", "Legendary Creature — Wolf", "");
    let cards = deck_from(&[(first, 1), (second, 1)]);
    let mut st = super::turn_loop_tests::state(vec![], vec![]);
    st.battlefield
        .push(super::game::new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(super::game::new_perm_with(2, &cards, CardIdx(1), 0, false));
    super::game_effects::enforce_legend_rule(&cards, &mut st, 1);
    assert_eq!(st.battlefield.len(), 2);
    assert!(st.graveyard.is_empty());
}
