//! Tests for the split counter model, keyword counters and grants,
//! energy, proliferate, companion, reconfigure, and face-down casting.

use super::deal::deal_opener;
use super::game::run_game;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::cost::parse_cost;
use super::oracle_parser::land::parse_tap_yield;
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
        power: Some("2".into()),
        toughness: Some("2".into()),
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

/// A deck from raw card rows: `lands` copies of an any-color Plains plus
/// one copy of each spell row.
fn row_deck(lands: usize, spells: &[CardRow], format: Format) -> SimDeck {
    let mut cards = Vec::new();
    for _ in 0..lands {
        cards.push(SimCard {
            name: "Plains".into(),
            cost: Cost::default(),
            min_cost: Cost::default(),
            tap: Some(parse_tap_yield("{T}: Add one mana of any color.").unwrap()),
            role: Role::Land,
            ..SimCard::default()
        });
    }
    for spell in spells {
        cards.push(parse_sim_card(spell));
    }
    let rules = if format == Format::Commander {
        "commander"
    } else {
        "constructed"
    };
    SimDeck {
        companion: None,
        cards,
        commanders: vec![],
        format,
        rules: super::format::rules_for(rules),
    }
}

/// Average life gained per game over `runs` seeded games.
fn avg_life_gained(deck: &SimDeck, turns: u32, runs: u32, seed: u64) -> f64 {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..runs)
        .map(|_| super::game::run_game(deck, &mut rng, turns).life_gained as f64)
        .sum::<f64>()
        / f64::from(runs)
}

#[test]
fn enter_counters_split_charge_and_plus1() {
    let charge = parse_sim_card(&card(
        "Charge Rock",
        "{2}",
        "Artifact",
        "This artifact enters with three charge counters on it.",
    ));
    assert_eq!(charge.enter_counters, EnterCounters::Charge(3));
    let plus1 = parse_sim_card(&card(
        "Pumped Body",
        "{1}{G}",
        "Creature — Beast",
        "This creature enters with two +1/+1 counters on it.",
    ));
    assert_eq!(plus1.enter_counters, EnterCounters::Plus1(2));
}

#[test]
fn enter_counters_parses_battlefield_phrasing() {
    // "Enters the battlefield with" has no literal "enters with", so the
    // counter clause search must accept the "battlefield with" shape.
    let plus1 = parse_sim_card(&card(
        "Pumped Body",
        "{1}{G}",
        "Creature — Beast",
        "This creature enters the battlefield with three +1/+1 counters on it.",
    ));
    assert_eq!(plus1.enter_counters, EnterCounters::Plus1(3));
    let charge = parse_sim_card(&card(
        "Charge Rock",
        "{2}",
        "Artifact",
        "This artifact enters the battlefield with two charge counters on it.",
    ));
    assert_eq!(charge.enter_counters, EnterCounters::Charge(2));
}

#[test]
fn keyword_counters_grant_keywords() {
    // "Enters with a flying counter" parses as a flying grant.
    let granter = parse_sim_card(&card(
        "Sky Holder",
        "{2}{U}",
        "Creature — Bird",
        "This creature enters with a flying counter on it.",
    ));
    assert!(
        granter
            .flags
            .keyword_grants
            .iter()
            .any(|grant| grant.keyword == Keyword::Flying),
        "flying counter parses as a flying grant"
    );
    // A charge counter is not a keyword counter.
    let rock = parse_sim_card(&card(
        "Charge Rock",
        "{2}",
        "Artifact",
        "This artifact enters with a charge counter on it.",
    ));
    assert!(
        rock.flags.keyword_grants.is_empty(),
        "charge counters grant no keywords"
    );
}

#[test]
fn lifelink_attackers_gain_life() {
    let lifelinker = card("Lifelinker", "{1}{W}", "Creature — Cleric", "Lifelink");
    let plain = card("Plain", "{1}{W}", "Creature — Cleric", "");
    let with_lifelink =
        avg_life_gained(&row_deck(20, &[lifelinker], Format::Constructed), 6, 200, 7);
    let without = avg_life_gained(&row_deck(20, &[plain], Format::Constructed), 6, 200, 7);
    assert!(
        with_lifelink > without,
        "lifelink adds life: {with_lifelink:.2} vs {without:.2}"
    );
}

#[test]
fn energy_gain_parses() {
    // A one-shot spell's energy resolves through the cast spell data.
    let spell = parse_sim_card(&card(
        "Energy Spell",
        "{1}{U}",
        "Sorcery",
        "You get {E}{E} (two energy counters).",
    ));
    assert_eq!(spell.spell_data.energy_on_cast, 2);
    // A repeatable source ("Whenever … you get {E}") lowers to a trigger.
    let engine = parse_sim_card(&card(
        "Energy Engine",
        "{1}{U}",
        "Enchantment",
        "Whenever you cast a spell, you get {E}{E}{E} (three energy counters).",
    ));
    assert!(
        engine
            .unlocked_abilities(0)
            .any(|ability| matches!(ability.effect, SimEffect::Energy(3))),
        "triggered energy gain lowers to an energy effect"
    );
}

#[test]
fn proliferate_parses() {
    let spell = parse_sim_card(&card(
        "Proliferator",
        "{2}{G}",
        "Creature — Druid",
        "At the beginning of your end step, proliferate.",
    ));
    assert!(
        spell
            .unlocked_abilities(0)
            .any(|ability| matches!(ability.effect, SimEffect::Proliferate)),
        "proliferate parses as an effect"
    );
}

#[test]
fn companion_parses_from_bench() {
    let mut row = card(
        "Lurrus",
        "{1}{W}{B}",
        "Legendary Creature — Cat Nightmare",
        "Companion — Each permanent card in your starting deck has mana value 2 or less.\nLifelink",
    );
    row.keywords = "Companion Lifelink".into();
    assert!(parse_sim_card(&row).has_companion);
    let plain = card("Bear", "{1}{G}", "Creature — Bear", "");
    assert!(!parse_sim_card(&plain).has_companion);
}

/// A deck whose last card is a companion: `row_deck` plus the companion
/// index, matching how `build_sim_deck` appends the bench companion.
fn deck_with_companion(lands: usize, spells: &[CardRow]) -> SimDeck {
    let mut deck = row_deck(lands, spells, Format::Constructed);
    let mut lurrus = card(
        "Lurrus",
        "{1}{W}{B}",
        "Legendary Creature — Cat Nightmare",
        "Companion — Each permanent card in your starting deck has mana value 2 or less.\nLifelink",
    );
    lurrus.keywords = "Companion Lifelink".into();
    let companion = parse_sim_card(&lurrus);
    let idx = CardIdx(deck.cards.len() as u32);
    deck.cards.push(companion);
    deck.companion = Some(idx);
    deck
}

#[test]
fn companion_stays_out_of_the_shuffled_library() {
    let deck = deck_with_companion(20, &[card("Bear", "{1}{G}", "Creature — Bear", "")]);
    let companion = deck.companion.expect("test deck sets a companion");
    assert_eq!(deck.library_len(), deck.cards.len() - 1);
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    for _ in 0..50 {
        let opener = deal_opener(&deck, &mut rng);
        assert!(
            !opener.hand.contains(&companion),
            "companion starts outside the game (CR 702.139a)"
        );
        assert!(
            !opener.library.contains(&companion),
            "companion never shuffles into the library"
        );
    }
}

#[test]
fn companion_fetches_once_when_three_is_payable() {
    let deck = deck_with_companion(20, &[card("Bear", "{1}{G}", "Creature — Bear", "")]);
    let mut rng = ChaCha8Rng::seed_from_u64(4);
    let log = run_game(&deck, &mut rng, 8);
    assert!(
        log.companion_online.is_some(),
        "the fetch fires once {{3}} is payable"
    );
}

#[test]
fn reconfigure_gear_parses_its_cost() {
    let row = card(
        "Reconfigured Gear",
        "{1}{R}",
        "Artifact Creature — Equipment Rhino",
        "Reconfigure—Pay {2} or {E}{E}{E}.\nEquipped creature gets +2/+2.",
    );
    let sim = parse_sim_card(&row);
    let gear = sim
        .flags
        .equipment
        .expect("reconfigure parses as Equipment");
    assert!(gear.reconfigure);
    assert_eq!(gear.cost, 2);
}

#[test]
fn morph_cost_parses() {
    let sim = parse_sim_card(&card(
        "Shifty Body",
        "{1}{U}",
        "Creature — Bird",
        "Flying\nMorph {2}{U}",
    ));
    assert_eq!(
        sim.morph_cost,
        Some(Cost {
            generic: 2,
            pips: [0, 1, 0, 0, 0],
            ..Cost::default()
        })
    );
    let plain = parse_sim_card(&card("Bear", "{1}{G}", "Creature — Bear", ""));
    assert_eq!(plain.morph_cost, None);
}

#[test]
fn megamorph_and_disguise_costs_parse() {
    let mega = parse_sim_card(&card(
        "Grown Body",
        "{2}{G}",
        "Creature — Beast",
        "Megamorph {1}{G}",
    ));
    assert_eq!(
        mega.morph_cost,
        Some(Cost {
            generic: 1,
            pips: [0, 0, 0, 0, 1],
            ..Cost::default()
        })
    );
    let disguise = parse_sim_card(&card(
        "Hidden Body",
        "{2}{U}",
        "Creature — Bird",
        "Disguise {1}{U}",
    ));
    assert_eq!(
        disguise.morph_cost,
        Some(Cost {
            generic: 1,
            pips: [0, 1, 0, 0, 0],
            ..Cost::default()
        })
    );
}
