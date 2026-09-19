// Tests for the simulator model module.

/// A minimal card row for tests.
use super::aggregate::aggregate;
use super::game::run_game;
use super::model::*;
use super::parse::*;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
fn card(name: &str, mana_cost: &str, type_line: &str, text: &str) -> CardRow {
    CardRow {
        name: name.to_string(),
        oracle_id: String::new(),
        mana_cost: mana_cost.to_string(),
        cmc: super::parse::parse_cost(mana_cost).total() as f64,
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

/// A card row with keywords and power/toughness.
fn card_kw(name: &str, mana_cost: &str, type_line: &str, keywords: &str, text: &str) -> CardRow {
    let mut row = card(name, mana_cost, type_line, text);
    row.keywords = keywords.to_string();
    row
}

/// A deck text with one section.
fn test_perm(card_idx: usize) -> super::game::InPlay {
    super::game::InPlay {
        card: card_idx,
        tapped: false,
        sick: false,
        counters: 0,
        animated: false,
        crewed: false,
        entered_turn: 1,
        saga_step: 0,
        fired: false,
        is_commander: false,
        commander_slot: 0,
        blink_pending: false,
        loyalty: 0,
        equipped: false,
        equip_host: None,
    }
}

#[test]
fn cost_parses_generic_pips_hybrid_and_faces() {
    let c = parse_cost("{2}{W}{W}");
    assert_eq!(c.generic, 2);
    assert_eq!(c.pips[0], 2);
    assert_eq!(c.total(), 4);

    let hybrid = parse_cost("{W/U}");
    assert_eq!(hybrid.flex_pips, 1);
    assert!(hybrid.pips.iter().all(|p| *p == 0));

    // Phyrexian {B/P}: payable with black — a single-color pip.
    let phyrexian = parse_cost("{1}{B/P}");
    assert_eq!(phyrexian.generic, 1);
    assert_eq!(phyrexian.pips[2], 1);

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
    // MDFC spell/land ("Instant // Land") plays as a land in the model:
    // one physical card, dominant use is the land face in a goldfish.
    let row3 = card(
        "Sink into Stupor // Soporific Springs",
        "{1}{U}{U} // ",
        "Instant // Land",
        "",
    );
    let sim3 = parse_sim_card(&row3);
    assert_eq!(
        sim3.role,
        super::model::Role::Land,
        "MDFC spell+land plays as a land"
    );
    assert_eq!(sim3.cost.total(), 0, "lands cost nothing");
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
    // Goldfish: opponent-scaled production reads as any-color from turn 2
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
    // Pure improvise shape: 6 generic + improvise. Parse-time floor is
    // −2 (generic 4); a 4-artifact board reaches −3, an 8-artifact board
    // the full −5.
    let act = card(
        "Organic Extinction",
        "{6}{W}{W}",
        "Sorcery",
        "Improvise (Your artifacts can help cast this spell.)\nDestroy all nonartifact creatures.",
    );
    let sim = parse_sim_card(&act);
    assert!(sim.board_discount);
    assert_eq!(sim.min_cost.generic, 4);

    let rock = card("Iron Lump", "{2}", "Artifact", "{T}: Add {C}.");
    let rock_sim = parse_sim_card(&rock);
    let mut battlefield: Vec<super::game::InPlay> = Vec::new();
    let deck = SimDeck {
        cards: vec![sim.clone(), rock_sim],
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    // No artifacts: the flat floor.
    assert_eq!(
        super::game_mana::effective_min_cost(&deck, &deck.cards[0], &battlefield).generic,
        4
    );
    // 4 artifacts: one extra discount step (generic grows toward printed).
    for _ in 0..4 {
        battlefield.push(test_perm(1));
    }
    assert_eq!(
        super::game_mana::effective_min_cost(&deck, &deck.cards[0], &battlefield).generic,
        5
    );
    // 8 artifacts: two extra steps (generic caps at the printed 6).
    for _ in 0..4 {
        battlefield.push(test_perm(1));
    }
    assert_eq!(
        super::game_mana::effective_min_cost(&deck, &deck.cards[0], &battlefield).generic,
        6
    );
    // Creatures do not count toward the discount even in bulk.
    let body = card("Bear Cub", "{1}{G}", "Creature — Bear", "");
    let body_sim = parse_sim_card(&body);
    let deck_bodies = SimDeck {
        cards: vec![sim.clone(), body_sim],
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut creature_board: Vec<super::game::InPlay> = Vec::new();
    for _ in 0..8 {
        creature_board.push(test_perm(1));
    }
    assert_eq!(
        super::game_mana::effective_min_cost(&deck_bodies, &deck_bodies.cards[0], &creature_board)
            .generic,
        4
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
        sim.station_tiers
            .iter()
            .flat_map(|t| t.abilities.iter())
            .any(|a| a.trigger == Trigger::OnUpkeep
                && matches!(a.effect, super::model::Effect::ManaPerCounter(_)))
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

// Station tiers (CR 702.184/721)

#[test]
fn station_single_tier_with_pt_animates() {
    // Galvanizing Sawship: single 3+ tier, becomes a creature.
    let text = "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 3+.)\n3+ | Flying, haste";
    let (tiers, is_station) = parse_station_tiers(text, "Artifact — Spacecraft");
    assert!(is_station);
    assert_eq!(tiers.len(), 1);
    assert_eq!(tiers[0].at, 3);
    assert!(tiers[0].animate);
}

#[test]
fn station_two_tiers_only_last_animates() {
    // Dawnsire: 10+ trigger, 20+ P/T.
    let text = "Station (Tap another creature you control: Put charge counters equal to its power on this Spacecraft. Station only as a sorcery. It's an artifact creature at 20+.)\n10+ | Whenever you attack, Dawnsire deals 100 damage to up to one target creature or planeswalker.\n20+ | Flying";
    let (tiers, _) = parse_station_tiers(text, "Legendary Artifact — Spacecraft");
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
    let (tiers, is_station) = parse_station_tiers(text, "Land — Planet");
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
        parse_station_tiers("Some other text entirely.", "Artifact — Spacecraft");
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
    assert!(sim.mana_on_cast.is_some());

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

// draw_amount

#[test]
fn draw_amount_numerals_win() {
    assert_eq!(draw_amount("draw two cards"), 2);
    assert_eq!(draw_amount("draw three cards"), 3);
    assert_eq!(draw_amount("draw 5 cards"), 5);
    assert_eq!(draw_amount("draw a card"), 1);
    assert_eq!(draw_amount("investigate"), 1);
    assert_eq!(draw_amount("destroy target"), 0);
}

// Game loop: pool math
