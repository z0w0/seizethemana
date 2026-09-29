//! Tests for the simulator model module.

/// A minimal card row for tests.
use super::aggregate::aggregate;
use super::game::run_game;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::cost::parse_cost;
use super::oracle_parser::draw_amount;
use super::oracle_parser::land::parse_tap_yield;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
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
fn card_type_predicates_come_from_the_type_line() {
    let land = parse_sim_card(&card("Plain", "", "Basic Land — Plains", ""));
    let spell = parse_sim_card(&card("Growth", "{G}", "Sorcery", "Draw a card."));
    let walker = parse_sim_card(&card("Guide", "{2}{U}", "Planeswalker — Guide", ""));
    let creature = parse_sim_card(&card("Scout", "{G}", "Creature — Scout", ""));

    assert!(land.is_land);
    assert!(!spell.is_land);
    assert!(walker.is_planeswalker);
    assert!(!creature.is_planeswalker);
}

/// A card row with keywords and power/toughness.
fn card_kw(name: &str, mana_cost: &str, type_line: &str, keywords: &str, text: &str) -> CardRow {
    let mut row = card(name, mana_cost, type_line, text);
    row.keywords = keywords.to_string();
    row
}

/// A deck text with one section.
fn test_perm(card_idx: usize) -> super::game::Permanent {
    super::game::Permanent {
        uid: 0,
        card: super::game::CardRef::Deck(super::model::CardIdx(card_idx as u32)),
        tapped: false,
        summoning_sick: false,
        counters: Default::default(),
        animated: false,
        crewed: false,
        entered_turn: 1,
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
    }
}

#[test]
fn cost_parses_generic_pips_hybrid_and_faces() {
    let c = parse_cost("{2}{W}{W}");
    assert_eq!(c.generic, 2);
    assert_eq!(c.pips[0], 2);
    assert_eq!(c.total(), 4);

    let hybrid = parse_cost("{W/U}");
    assert_eq!(hybrid.hybrid_pips, 1);
    assert!(hybrid.pips.iter().all(|p| *p == 0));

    // Phyrexian {B/P}: payable with black or 2 life (CR 107.4f).
    let phyrexian = parse_cost("{1}{B/P}");
    assert_eq!(phyrexian.generic, 1);
    assert_eq!(phyrexian.pips[2], 0);
    assert_eq!(phyrexian.phyrexian[2], 1);
    assert_eq!(phyrexian.total(), 2);

    let faces = parse_cost("{2}{B} // {B}");
    assert_eq!(faces.generic, 2);
    assert_eq!(faces.pips[2], 2);

    let x = parse_cost("{X}{U}");
    assert_eq!(x.generic, 1);
    assert_eq!(x.pips[1], 1);

    assert_eq!(parse_cost("{10}").generic, 10);
}

#[test]
fn split_card_faces_take_cheapest_face() {
    // Dusk // Dawn: faces {2}{W}{W} and {4}{W}; the model pays {2}{W}{W}.
    let row = card(
        "Dusk // Dawn",
        "{2}{W}{W} // {4}{W}",
        "Instant // Instant",
        "",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(
        sim.cost.total(),
        4,
        "Dusk // Dawn should cost its cheaper face"
    );
    // Bramble Familiar // Fetch Quest: spell face {1}{G} (2 total).
    let row2 = card(
        "Bramble Familiar // Fetch Quest",
        "{1}{G} // {5}{G}{G}",
        "Creature — Treefolk // Sorcery — Adventure",
        "",
    );
    let sim2 = parse_sim_card(&row2);
    assert_eq!(
        sim2.cost.total(),
        2,
        "Adventure creature face is the playable face"
    );
    // MDFC spell/land ("Instant // Land") is one card with two uses: the
    // sim models the spell face (role + cast cost), and the land rule
    // plays the land face when no other land is in hand.
    let row3 = card(
        "Sink into Stupor // Soporific Springs",
        "{1}{U}{U} // ",
        "Instant // Land",
        "",
    );
    let sim3 = parse_sim_card(&row3);
    assert!(sim3.is_mdfc_spell, "MDFC spell+land detected");
    assert_eq!(
        sim3.role,
        super::model::Role::Other,
        "MDFC spell face carries the spell role"
    );
    assert_eq!(sim3.cost.total(), 3, "the spell face is the cast");
}

// Tap yields: choice vs fixed vs any vs colorless

#[test]
fn tap_yield_or_is_choice() {
    // Shock dual: one tap, pick one of two colors.
    let y = parse_tap_yield("{T}: Add {G} or {U}.").unwrap();
    assert_eq!(y.total(), 1);
    assert!(y.choice[4] && y.choice[1]);
    assert!(y.fixed.iter().all(|p| *p == 0));
}

#[test]
fn tap_yield_fixed_set_is_simultaneous() {
    // Jegantha: one tap produces all five at once.
    let y = parse_tap_yield("{T}: Add {W}{U}{B}{R}{G}.");
    assert!(y.is_some());
    let y = y.unwrap();
    assert_eq!(y.total(), 5);
    assert!(y.choice.iter().all(|c| !c));
    for p in y.fixed {
        assert_eq!(p, 1);
    }
}

#[test]
fn tap_yield_any_color_prose() {
    let y = parse_tap_yield("{T}: Add one mana of any color.");
    assert!(y.is_some_and(|y| y.any_pips == 1 && y.total() == 1));
}

#[test]
fn tap_yield_colorless() {
    let y = parse_tap_yield("{T}: Add {C}.");
    assert!(y.is_some_and(|y| y.colorless == 1 && y.total() == 1));
}

#[test]
fn tap_yield_double_colorless() {
    // Sol Ring produces {C}{C} on one tap.
    let y = parse_tap_yield("{T}: Add {C}{C}.");
    assert!(y.is_some_and(|y| y.colorless == 2 && y.total() == 2));
}

#[test]
fn tap_yield_none_for_non_mana() {
    assert!(parse_tap_yield("Destroy target creature.").is_none());
}

#[test]
fn tap_yield_opponent_dependent_flags_any() {
    // Opponent-dependent production stays unavailable in a goldfish game.
    // (add_yield_turns gates the turn), not nothing.
    let orchard = "{T}: Add one mana of any color that a land an opponent controls could produce.";
    let y = parse_tap_yield(orchard).expect("opponent yield parses");
    assert!(y.opponent_any);
    assert_eq!(y.any_pips, 1);
    assert!(parse_tap_yield("{T}: Add {G}.").is_some());
}

#[test]
fn type_granted_land_taps_for_any_color() {
    // Multiversal Passage: no add clause; "This land is the chosen type"
    // makes it a basic of the player's choice.
    let passage = card(
        "Multiversal Passage",
        "",
        "Land",
        "As this land enters, choose a basic land type. Then you may pay 2 life. If you don't, it enters tapped.\nThis land is the chosen type.",
    );
    let sim = parse_sim_card(&passage);
    let tap = sim.tap.expect("type-granted land taps for mana");
    assert!(tap.any_pips == 1 && tap.total() == 1);
    // Regular add-clause lands are unaffected.
    let plain = card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)");
    assert!(parse_sim_card(&plain).tap.is_some());
}

#[test]
fn improvise_discount_grows_with_artifacts() {
    // Pure improvise shape (CR 702.126a): 6 generic + improvise. The
    // cost drops one generic per artifact, capped at the printed
    // generic (zero here); creatures and enchantments never count.
    let act = card(
        "Organic Extinction",
        "{6}{W}{W}",
        "Sorcery",
        "Improvise (Your artifacts can help cast this spell.)\nDestroy all nonartifact creatures.",
    );
    let sim = parse_sim_card(&act);
    assert!(sim.battlefield_discount);
    assert_eq!(
        sim.min_cost.generic, 6,
        "no parse-time floor: the board decides"
    );

    let rock = card("Iron Lump", "{2}", "Artifact", "{T}: Add {C}.");
    let rock_sim = parse_sim_card(&rock);
    let mut battlefield: Vec<super::game::Permanent> = Vec::new();
    let deck = SimDeck {
        companion: None,
        cards: vec![sim.clone(), rock_sim],
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    // No artifacts: the printed cost.
    assert_eq!(
        super::game_mana::effective_min_cost(&deck, &deck.cards[0], &battlefield).generic,
        6
    );
    // 4 artifacts: four generic less.
    for _ in 0..4 {
        battlefield.push(test_perm(1));
    }
    assert_eq!(
        super::game_mana::effective_min_cost(&deck, &deck.cards[0], &battlefield).generic,
        2
    );
    // 8 artifacts: the generic floor (zero), pips stay.
    for _ in 0..4 {
        battlefield.push(test_perm(1));
    }
    let eff = super::game_mana::effective_min_cost(&deck, &deck.cards[0], &battlefield);
    assert_eq!(eff.generic, 0);
    assert_eq!(eff.pips[0], 2, "the white pips never change");
    // Creatures do not count toward the discount even in bulk.
    let creature = card("Bear Cub", "{1}{G}", "Creature — Bear", "");
    let creature_sim = parse_sim_card(&creature);
    let deck_bodies = SimDeck {
        companion: None,
        cards: vec![sim.clone(), creature_sim],
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut creature_board: Vec<super::game::Permanent> = Vec::new();
    for _ in 0..8 {
        creature_board.push(test_perm(1));
    }
    assert_eq!(
        super::game_mana::effective_min_cost(&deck_bodies, &deck_bodies.cards[0], &creature_board)
            .generic,
        6
    );
}

#[test]
fn commander_relic_banks_and_releases() {
    // Coalition Relic: charge, then release counters × any-color at upkeep.
    let relic = card(
        "Coalition Relic",
        "{3}",
        "Artifact",
        "{T}: Add one mana of any color.\n{T}: Put a charge counter on this artifact.\nAt the beginning of your first main phase, remove all charge counters from this artifact. Add one mana of any color for each charge counter removed this way.",
    );
    let sim = parse_sim_card(&relic);
    assert!(
        sim.striations
            .iter()
            .flat_map(|t| t.abilities.iter())
            .any(|a| a.trigger == SimTrigger::PrecombatMain
                && matches!(a.effect, super::model::SimEffect::ManaPerCounter(_)))
    );
    let mut cards = Vec::new();
    for _ in 0..20 {
        cards.push(parse_sim_card(&card(
            "Island",
            "",
            "Basic Land — Island",
            "({T}: Add {U}.)",
        )));
    }
    for _ in 0..10 {
        cards.push(sim.clone());
    }
    let deck = SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(77);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // Relic-charged turns overproduce relative to lands alone (release).
    assert!(
        stats.unused_mana[5] > 0.0,
        "relic banking should add mana over the land baseline, got {:.2}",
        stats.unused_mana[5]
    );
}

// Station tiers

/// Parse station text through the Oracle AST and runtime lowering layers.
fn parse_striations(text: &str, type_line: &str) -> (Vec<SimStriation>, bool) {
    let row = card("Station Test", "", type_line, text);
    let oracle = super::oracle_parser::parse_oracle_card(&row);
    (
        super::oracle_lower::lower_striations(&oracle.stations),
        oracle.stations.is_station_card,
    )
}

#[test]
fn station_single_tier_with_pt_animates() {
    // Galvanizing Sawship: single 3+ tier, becomes a creature.
    let text = "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 3+.)\n3+ | Flying, haste";
    let (tiers, is_station) = parse_striations(text, "Artifact — Spacecraft");
    assert!(is_station);
    assert_eq!(tiers.len(), 1);
    assert_eq!(tiers[0].at, 3);
    assert!(tiers[0].animate);
}

#[test]
fn station_two_tiers_only_last_animates() {
    // Dawnsire: 10+ trigger, 20+ P/T.
    let text = "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 20+.)\n10+ | Whenever you attack, Dawnsire deals 100 damage to up to one target creature or planeswalker.\n20+ | Flying";
    let (tiers, _) = parse_striations(text, "Legendary Artifact — Spacecraft");
    assert_eq!(tiers.len(), 2);
    assert_eq!(tiers[0].at, 10);
    assert!(!tiers[0].animate);
    assert_eq!(tiers[1].at, 20);
    assert!(tiers[1].animate);
}

#[test]
fn station_planet_never_animates() {
    // Uthros, Titanic Godcore: 12+ mana ability, no P/T box.
    let text = "This land enters tapped.\n{T}: Add {U}.\nStation (Tap another creature you control: Put charge counters equal to its power on this Planet. Station only as a sorcery.)\n12+ | {U}, {T}: Add {U} for each artifact you control.";
    let (tiers, is_station) = parse_striations(text, "Land — Planet");
    assert!(is_station);
    assert!(tiers.iter().all(|t| !t.animate));
    // The 12+ tier has the mana ability.
    assert!(
        tiers
            .iter()
            .find(|t| t.at == 12)
            .is_some_and(|t| !t.abilities.is_empty())
    );
}

#[test]
fn station_no_tiers_when_no_markers() {
    let (tiers, is_station) =
        parse_striations("Some other text entirely.", "Artifact — Spacecraft");
    assert!(is_station);
    assert!(tiers.is_empty());
}

// Crew

#[test]
fn crew_parses_reminder_cost() {
    let row = card_kw(
        "Smuggler's Copter",
        "{2}",
        "Artifact — Vehicle",
        r#"["Flying","Crew"]"#,
        "Flying\nCrew 1 (Tap any number of creatures you control with total power 1 or more: This Vehicle becomes an artifact creature until end of turn.)",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.crew, Some(1));
    assert!(!sim.is_station_card);
}

#[test]
fn crew_defaults_to_one_without_number() {
    let row = card_kw(
        "Esika's Chariot",
        "{4}",
        "Legendary Artifact — Vehicle",
        r#"["Crew"]"#,
        "Crew 4",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.crew, Some(4));
}

#[test]
fn non_vehicle_has_no_crew() {
    let row = card_kw("Bear", "{2}", "Creature — Bear", "[]", "");
    assert!(parse_sim_card(&row).crew.is_none());
}

/// Cascade, undying, companion, and read ahead flags come from their
/// typed keywords: a grant to other objects stays inert on the granter.
#[test]
fn flag_keywords_require_the_keyword_not_the_text() {
    let cascade = card_kw(
        "Shardless Agent",
        "{2}{G}",
        "Creature — Human Rogue",
        r#"["Cascade"]"#,
        "Cascade (When you cast this spell, exile cards from the top of your library.)",
    );
    assert!(parse_sim_card(&cascade).has_cascade);
    // "The first spell you cast from exile each turn has cascade" grants
    // cascade to other spells, not to this card (Wild-Magic Sorcerer).
    let granter = card(
        "Wild-Magic Sorcerer",
        "{3}{R}",
        "Creature — Orc Shaman Sorcerer",
        "The first spell you cast from exile each turn has cascade.",
    );
    assert!(!parse_sim_card(&granter).has_cascade);

    let undying = card_kw(
        "Butcher Ghoul",
        "{1}{B}",
        "Creature — Zombie",
        r#"["Undying"]"#,
        "Undying (When this creature dies, if it had no +1/+1 counters on it, return it to the battlefield with a +1/+1 counter on it.)",
    );
    assert!(parse_sim_card(&undying).has_undying);
    // Mikaeus grants undying to other creatures, not to itself.
    let granter = card(
        "Mikaeus, the Unhallowed",
        "{3}{B}{B}{B}",
        "Legendary Creature — Zombie Cleric",
        "Other non-Human creatures you control get +1/+1 and have undying.",
    );
    assert!(!parse_sim_card(&granter).has_undying);

    let read_ahead = card_kw(
        "Read Ahead Saga",
        "{1}{U}",
        "Enchantment — Saga",
        r#"["Read ahead"]"#,
        "Read ahead (Choose a chapter and start with that many lore counters.)\nI — Draw a card.",
    );
    assert!(parse_sim_card(&read_ahead).read_ahead);
    let no_read_ahead = card("Saga", "{1}{U}", "Enchantment — Saga", "I — Draw a card.");
    assert!(!parse_sim_card(&no_read_ahead).read_ahead);
}

/// Crew and dredge numbers come from typed keyword arguments, so a card
/// whose text spells a number for another object stays inert.
#[test]
fn crew_and_dredge_require_the_keyword_not_the_text() {
    let crew = card_kw(
        "Smuggler's Copter",
        "{2}",
        "Artifact — Vehicle",
        r#"["Crew"]"#,
        "Crew 1 (Tap any number of creatures you control with total power 1 or more.)",
    );
    assert_eq!(parse_sim_card(&crew).crew, Some(1));
    // "Vehicles you control have crew 2" grants crew to other Vehicles;
    // it must not grant crew to this card itself (Kotori, Pilot Prodigy).
    let granter = card(
        "Kotori, Pilot Prodigy",
        "{1}{W}{U}",
        "Legendary Creature — Moonfolk Pilot",
        "Vehicles you control have crew 2.",
    );
    assert!(parse_sim_card(&granter).crew.is_none());

    let dredge = card_kw(
        "Dredger",
        "{2}{G}",
        "Creature — Troll",
        r#"["Dredge"]"#,
        "Dredge 5 (If you would draw a card, you may mill five cards instead.)",
    );
    assert_eq!(parse_sim_card(&dredge).dredge, Some(5));
    let no_dredge = card("Troll", "{2}{G}", "Creature — Troll", "");
    assert!(parse_sim_card(&no_dredge).dredge.is_none());
}

/// Warp, affinity, and improvise lower from their typed keywords: the
/// alternative cost and board-discount flag require the keyword.
#[test]
fn warp_affinity_improvise_lower_from_keywords() {
    let warp = card_kw(
        "Mightform Harmonizer",
        "{2}{G}{G}",
        "Creature — Insect Druid",
        r#"["Warp"]"#,
        "Warp {2}{G} (You may cast this card from your hand for its warp cost.)",
    );
    let warp_sim = parse_sim_card(&warp);
    assert_eq!(warp_sim.min_cost.total(), 3);
    assert!(!warp_sim.battlefield_discount);
    // A "warp" mention that is not the Warp keyword stays inert.
    let no_warp = card(
        "Warped Riders",
        "{2}{G}{G}",
        "Creature",
        "If a spell was warped this turn, this creature gets +1/+1.",
    );
    assert_eq!(parse_sim_card(&no_warp).min_cost.total(), 4);

    let improvise = card_kw(
        "Organic Extinction",
        "{6}{W}{W}",
        "Sorcery",
        r#"["Improvise"]"#,
        "Improvise (Your artifacts can help cast this spell.)",
    );
    assert!(parse_sim_card(&improvise).battlefield_discount);

    let affinity = card_kw(
        "Frogmite",
        "{4}",
        "Artifact Creature — Frog",
        r#"["Affinity"]"#,
        "Affinity for artifacts (This spell costs {1} less to cast for each artifact you control.)",
    );
    assert!(parse_sim_card(&affinity).battlefield_discount);
    // No affinity/improvise keyword: the artifact discount stays off.
    let plain = card("Artifact", "{4}", "Artifact", "");
    assert!(!parse_sim_card(&plain).battlefield_discount);
}

// Abilities: ETB, upkeep, attack, cast engines, activations

#[test]
fn roles_classify_correctly() {
    let land = card("Plains", "", "Basic Land — Plains", "({T}: Add {W}.)");
    assert_eq!(parse_sim_card(&land).role, Role::Land);

    let rock = card("Sol Ring", "{1}", "Artifact", "{T}: Add {C}{C}.");
    assert_eq!(parse_sim_card(&rock).role, Role::Rock);

    let dork = card(
        "Llanowar Elves",
        "{G}",
        "Creature — Elf Druid",
        "{T}: Add {G}.",
    );
    assert_eq!(parse_sim_card(&dork).role, Role::Dork);

    let ritual = card("Dark Ritual", "{B}", "Instant", "Add {B}{B}{B}.");
    let sim = parse_sim_card(&ritual);
    assert_eq!(sim.role, Role::RampSpell);
    assert!(sim.spell_data.mana_on_cast.is_some());

    let draw = card("Divination", "{2}{U}", "Sorcery", "Draw two cards.");
    assert_eq!(parse_sim_card(&draw).role, Role::Draw);

    let removal = card(
        "Swords to Plowshares",
        "{W}",
        "Instant",
        "Exile target creature.",
    );
    assert_eq!(parse_sim_card(&removal).role, Role::Removal);

    let big = card("Dragon", "{7}", "Creature — Dragon", "Flying.");
    assert_eq!(parse_sim_card(&big).role, Role::Wincon);

    // Tutors count as draw sources (Fabricate shape).
    let tutor = card(
        "Fabricate",
        "{2}{U}",
        "Sorcery",
        "Search your library for an artifact card, reveal it, put it into your hand, then shuffle.",
    );
    assert_eq!(parse_sim_card(&tutor).role, Role::Draw);
}

#[test]
fn station_crafts_classify_wincon_or_other() {
    let big = card(
        "Dawnsire, Sunstar Dreadnought",
        "{5}",
        "Legendary Artifact — Spacecraft",
        "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 20+.)\n10+ | Whenever you attack, Dawnsire deals 100 damage.\n20+ | Flying",
    );
    assert_eq!(parse_sim_card(&big).role, Role::Wincon);
}

#[test]
fn saga_and_planeswalker_flags() {
    let saga = card(
        "Urza's Saga",
        "",
        "Enchantment Land — Urza's Saga",
        "(As this Saga enters and after your draw step, add a lore counter.)",
    );
    // Urza's Saga is a land face, so saga staging stays off.
    assert!(!parse_sim_card(&saga).is_saga);
    let real_saga = card(
        "Esper Origins",
        "{1}{G}",
        "Sorcery // Enchantment Creature — Saga Elemental",
        "Surveil 2. // (As this Saga enters and after your draw step, add a lore counter.)",
    );
    // Multi-face: the second face's type line makes it a Saga.
    assert!(parse_sim_card(&real_saga).is_saga);
}

// draw_amount (parser-layer text helper)

#[test]
fn draw_amount_numerals_win() {
    assert_eq!(draw_amount("draw two cards"), 2);
    assert_eq!(draw_amount("draw three cards"), 3);
    assert_eq!(draw_amount("draw 5 cards"), 5);
    assert_eq!(draw_amount("draw a card"), 1);
    assert_eq!(draw_amount("investigate"), 1);
    assert_eq!(draw_amount("destroy target"), 0);
}

#[test]
fn draw_amount_six_and_seven() {
    assert_eq!(draw_amount("draw six cards"), 6);
    assert_eq!(draw_amount("draw seven cards"), 7);
    assert_eq!(draw_amount("each player draws seven cards"), 7);
}

/// Verify enum conversions for all colors and basic land types.
#[test]
fn color_and_basic_land_enums_cover_wubrg() {
    assert_eq!(
        ManaColor::ALL.map(ManaColor::symbol),
        ['W', 'U', 'B', 'R', 'G']
    );
    for (index, color) in ManaColor::ALL.into_iter().enumerate() {
        assert_eq!(color.index(), index);
        assert_eq!(ManaColor::from_symbol(color.symbol()), Some(color));
    }
    assert_eq!(
        BasicLandType::ALL.map(BasicLandType::name),
        ["Plains", "Island", "Swamp", "Mountain", "Forest"]
    );
    for (index, land) in BasicLandType::ALL.into_iter().enumerate() {
        assert_eq!(land.index(), index);
    }
}

// Game loop: pool math
