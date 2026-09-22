// Tests for the simulator parse module.

/// A minimal card row for tests.
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
#[test]
fn etb_draw_trigger_parses() {
    let row = card(
        "Atraxa, Grand Unifier",
        "{3}{G}{W}{U}{B}",
        "Legendary Creature — Phyrexian Angel",
        "When Atraxa enters, reveal the top ten cards of your library. For each card type, you may put a card of that type from among the revealed cards into your hand.",
    );
    let sim = parse_sim_card(&row);
    assert!(
        sim.abilities()
            .any(|a| a.trigger == Trigger::OnEnter
                && matches!(a.effect, super::model::Effect::Tutor))
    );
}

#[test]
fn etb_tokens_parse_for_named_enters() {
    // "When <Cardname> enters, create …" (IGS shape).
    let row = card(
        "Breya, Etherium Shaper",
        "{W}{U}{B}{R}",
        "Legendary Artifact Creature — Human",
        "When Breya enters, create two 1/1 blue Thopter artifact creature tokens with flying.",
    );
    let sim = parse_sim_card(&row);
    assert!(
        sim.abilities().any(|a| a.trigger == Trigger::OnEnter
            && matches!(a.effect, super::model::Effect::Tokens(_)))
    );
}

#[test]
fn upkeep_draw_engine_parses() {
    let row = card(
        "Phyrexian Arena",
        "{1}{B}",
        "Enchantment",
        "At the beginning of your upkeep, you draw a card and you lose 1 life.",
    );
    let sim = parse_sim_card(&row);
    assert!(sim.abilities().any(|a| a.trigger == Trigger::OnUpkeep));
}

#[test]
fn attack_draw_trigger_parses() {
    let row = card(
        "Smuggler's Copter",
        "{2}",
        "Artifact — Vehicle",
        "Flying\nWhenever this Vehicle attacks or blocks, you may draw a card. If you do, discard a card.\nCrew 1",
    );
    let sim = parse_sim_card(&row);
    assert!(sim.abilities().any(
        |a| a.trigger == Trigger::OnAttack && matches!(a.effect, super::model::Effect::Draw(1))
    ));
}

#[test]
fn on_cast_spell_engine_parses() {
    let row = card(
        "Jhoira, Weatherlight Captain",
        "{2}{U}{R}",
        "Legendary Creature — Human Artificer",
        "Whenever you cast a historic spell, draw a card.",
    );
    let sim = parse_sim_card(&row);
    assert!(
        sim.abilities().any(|a| a.trigger == Trigger::OnCastSpell
            && matches!(a.effect, super::model::Effect::Draw(1)))
    );
}

#[test]
fn activation_draw_parses_cost_and_tap() {
    let ab = parse_ability("{1}, {T}: Draw two cards.");
    assert!(ab.is_some());
    let ab = ab.unwrap();
    assert_eq!(ab.cost.total(), 1);
    assert!(ab.taps);
    assert!(matches!(ab.effect, super::model::Effect::Draw(2)));
    // Draw + discard in one activation is a loot.
    let loot = parse_ability("{1}, {T}: Draw two cards, then discard a card.").unwrap();
    assert!(matches!(loot.effect, super::model::Effect::Loot(2)));
}

#[test]
fn planeswalker_loyalty_activation_costs_no_mana() {
    let ab = parse_ability(
        "−3: Search your library for an artifact card with mana value 1 or less, reveal it, put it into your hand, then shuffle.",
    );
    assert!(
        ab.is_some_and(|a| a.cost.total() == 0 && matches!(a.effect, super::model::Effect::Tutor))
    );
}

#[test]
fn activation_mana_effect_parses() {
    let ab = parse_ability("{0}: Add X mana in any combination of {U} and/or {R}.");
    // Vivi's scaling ability approximates to a mana activation.
    assert!(ab.is_some_and(|a| matches!(a.effect, super::model::Effect::Mana(_))));
}

// Enters-tapped, verge gates, Leyline, cost reductions

#[test]
fn shock_dual_stays_untapped() {
    let row = card(
        "Breeding Pool",
        "",
        "Land — Forest Island",
        "({T}: Add {G} or {U}.)\nAs this land enters, you may pay 2 life. If you don't, it enters tapped.",
    );
    let sim = parse_sim_card(&row);
    assert!(!sim.enters_tapped);
}

#[test]
fn planewide_tapped_land_enters_tapped() {
    let row = card(
        "Uthros, Titanic Godcore",
        "",
        "Land — Planet",
        "This land enters tapped.\n{T}: Add {U}.\nStation (Tap another creature you control: Put charge counters equal to its power on this Planet. Station only as a sorcery.)\n12+ | {U}, {T}: Add {U} for each artifact you control.",
    );
    assert!(parse_sim_card(&row).enters_tapped);
}

#[test]
fn verge_gate_parses_both_types() {
    let row = card(
        "Blazemire Verge",
        "",
        "Land",
        "{T}: Add {B}.\n{T}: Add {R}. Activate only if you control a Swamp or a Mountain.",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.gate_types, vec!["Swamp", "Mountain"]);
}

#[test]
fn leyline_opens_in_play() {
    let row = card(
        "Leyline of the Guildpact",
        "{G/W}{G/U}{B/G}{R/G}",
        "Enchantment",
        "If this card is in your opening hand, you may begin the game with it on the battlefield.\nEach nonland permanent you control is all colors.",
    );
    assert!(parse_sim_card(&row).opens_in_play);
}

#[test]
fn warp_reduces_min_cost() {
    let row = card(
        "Mightform Harmonizer",
        "{2}{G}{G}",
        "Creature — Insect Druid",
        "Landfall — Whenever a land you control enters, double the power of target creature you control until end of turn.\nWarp {2}{G} (You may cast this card from your hand for its warp cost.)",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.min_cost.total(), 3);
    assert_eq!(sim.cost.total(), 4);
}

#[test]
fn cost_reduction_approximates() {
    let row = card(
        "Enthusiastic Mechanaut",
        "{U}{R}",
        "Artifact Creature — Goblin Artificer",
        "Flying\nArtifact spells you cast cost {1} less to cast.",
    );
    let sim = parse_sim_card(&row);
    // Its own cost is {U}{R}; no generic to cut, so min stays 2.
    assert_eq!(sim.min_cost.total(), 2);
    let big = card(
        "Kappa Cannoneer",
        "{5}{U}",
        "Artifact Creature — Turtle Warrior",
        "Improvise (Your artifacts can help cast this spell.)",
    );
    assert_eq!(parse_sim_card(&big).min_cost.total(), 4);
}

// Enters with charge counters, counter injection

#[test]
fn enter_counters_parse_words_and_digits() {
    let row = card(
        "Reckoner Bankbuster",
        "{2}",
        "Artifact — Vehicle",
        "This Vehicle enters with three charge counters on it.\n{2}, {T}, Remove a charge counter from this Vehicle: Draw a card.\nCrew 3",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(sim.enter_counters, 3);
}

#[test]
fn counter_injection_on_cast() {
    let row = card(
        "Drill Too Deep",
        "{1}{R}",
        "Instant",
        "Choose one —\n• Put five charge counters on target Spacecraft or Planet you control.\n• Destroy target artifact.",
    );
    assert_eq!(parse_sim_card(&row).counters_on_cast, 5);
}

// Role classification

#[test]
fn cantrip_draws_on_cast() {
    let row = card("Opt", "{U}", "Instant", "Scry 1.\nDraw a card.");
    assert_eq!(parse_sim_card(&row).draws_on_cast, 1);
    let div = card("Divination", "{2}{U}", "Sorcery", "Draw two cards.");
    assert_eq!(parse_sim_card(&div).draws_on_cast, 2);
}

#[test]
fn mill_shapes_parse() {
    let etb = parse_sim_card(&card(
        "Mill Fiend",
        "{2}{U}",
        "Creature — Horror",
        "When Mill Fiend enters, mill three cards.",
    ));
    assert!(
        etb.abilities()
            .any(|a| a.trigger == Trigger::OnEnter && matches!(a.effect, Effect::Mill(3)))
    );
    let upkeep = parse_sim_card(&card(
        "Slow Mill",
        "{1}{U}",
        "Creature — Frog Horror",
        "At the beginning of your upkeep, mill two cards.",
    ));
    assert!(
        upkeep
            .abilities()
            .any(|a| a.trigger == Trigger::OnUpkeep && matches!(a.effect, Effect::Mill(2)))
    );
}

#[test]
fn graveyard_return_shapes_parse() {
    let hand_return = parse_ability("{T}: Return a card from your graveyard to your hand.");
    assert!(matches!(
        hand_return.unwrap().effect,
        Effect::ReturnFromGraveyard {
            to_hand: true,
            count: 1
        }
    ));
    let board_return = parse_sim_card(&card(
        "Reanimator",
        "{2}{B}",
        "Creature — Zombie",
        "When Reanimator enters, return a creature card from your graveyard to the battlefield.",
    ));
    assert!(board_return.abilities().any(|a| matches!(
        a.effect,
        Effect::ReturnFromGraveyard {
            to_hand: false,
            count: 1
        }
    )));
}

#[test]
fn wheel_and_loot_shapes_parse() {
    let wheel = parse_sim_card(&card(
        "Wheel",
        "{2}{R}",
        "Sorcery",
        "Each player discards their hand, then draws seven cards.",
    ));
    assert!(
        matches!(wheel.draws_on_cast, 0),
        "wheel is an effect, not a plain draw"
    );
    let loot = parse_sim_card(&card(
        "Looter",
        "{1}{U}",
        "Creature — Merfolk",
        "{T}: Draw a card, then discard a card.",
    ));
    assert!(
        loot.abilities()
            .any(|a| matches!(a.effect, Effect::Loot(1)))
    );
}

#[test]
fn sacrifice_outlet_parses() {
    let outlet = parse_ability("{1}, Sacrifice a creature: Draw a card.");
    let ab = outlet.expect("outlet parses");
    assert_eq!(ab.sacrifice_bodies, 1);
    assert!(matches!(ab.effect, Effect::Draw(1)));
}

#[test]
fn death_trigger_parses() {
    let payoff = parse_sim_card(&card(
        "Death Dealer",
        "{2}{B}",
        "Creature — Human",
        "Whenever another creature you control dies, draw a card.",
    ));
    assert!(
        payoff
            .abilities()
            .any(|a| a.trigger == Trigger::OnDeath && matches!(a.effect, Effect::Draw(1)))
    );
}

#[test]
fn creature_only_restriction_parses() {
    let courtyard = parse_sim_card(&card(
        "Secluded Courtyard",
        "",
        "Land",
        "As this land enters, choose a creature type.\n{T}: Add {C}.\n{T}: Add one mana of any color. Spend this mana only to cast a creature spell of the chosen type.",
    ));
    let y = courtyard.tap.expect("courtyard taps");
    assert_eq!(
        y.restriction,
        Some(Restriction::Creature),
        "restriction captured"
    );
    let tower = parse_sim_card(&card(
        "Command Tower",
        "",
        "Land",
        "{T}: Add one mana of any color.",
    ));
    let y = tower.tap.expect("tower taps");
    assert_eq!(y.restriction, None);
}

#[test]
fn printed_power_parsed_for_bodies() {
    let mut row = card("Big Body", "{4}{G}", "Creature — Beast", "Vanilla.");
    row.power = Some("6".to_string());
    row.toughness = Some("6".to_string());
    let big = parse_sim_card(&row);
    assert_eq!(big.printed_power, Some(6));
}

#[test]
fn multi_tap_abilities_yield_one_mana() {
    // Plaza of Heroes: {C} + two any-color modes = one tap, one mana.
    let plaza = card(
        "Plaza of Heroes",
        "",
        "Land",
        "{T}: Add {C}.\n{T}: Add one mana of any color. Spend this mana only to cast a legendary spell.\n{T}: Add one mana of any color among legendary permanents you control.",
    );
    let sim = parse_sim_card(&plaza);
    let tap = sim.tap.expect("Plaza taps for mana");
    assert!(tap.alternatives, "merged abilities are alternatives");
    assert_eq!(tap.total(), 1, "one tap = one mana, not the sum");
}

#[test]
fn open_verge_two_abilities_yield_one_mana() {
    // Blazemire Verge with gates open: B or R, one mana per tap.
    let verge = card(
        "Blazemire Verge",
        "",
        "Land",
        "{T}: Add {B}.\n{T}: Add {R}. Activate only if you control a Swamp or a Mountain.",
    );
    let sim = parse_sim_card(&verge);
    let tap = sim.tap.expect("Verge taps for mana");
    assert_eq!(tap.total(), 1, "verge is one tap = one mana of B or R");
    // Reach covers both colors.
    assert!(tap.choice[2] && tap.choice[3] || tap.fixed[2] > 0 || tap.fixed[3] > 0);
}

#[test]
fn tier_tap_lines_do_not_double_count() {
    // Uthros: base {T}: Add {U} + "12+ | {U}, {T}: Add {U} for each artifact…".
    // The tier line must not merge into the base tap.
    let uthros = card(
        "Uthros, Titanic Godcore",
        "",
        "Land — Planet",
        "This land enters tapped.\n{T}: Add {U}.\nStation (Tap another creature you control: Put charge counters equal to its power on this Planet. Station only as a sorcery.)\n12+ | {U}, {T}: Add {U} for each artifact you control.",
    );
    let sim = parse_sim_card(&uthros);
    let tap = sim.tap.expect("Uthros base tap");
    assert_eq!(tap.total(), 1, "base tap is one mana");
    assert!(tap.fixed[1] == 1 || tap.choice[1], "base tap is U");
    // The 12+ tier still carries the mana ability.
    let tier = sim
        .station_tiers
        .iter()
        .find(|t| t.at == 12)
        .expect("12+ tier exists");
    assert!(
        tier.abilities
            .iter()
            .any(|a| matches!(a.effect, super::model::Effect::Mana(_)))
    );
}

#[test]
fn plaza_pool_yields_one_mana_per_tap() {
    // End-to-end: a deck of only Plaza-style lands produces 1 mana per
    // land per turn, never more.
    let plaza = card(
        "Plaza of Heroes",
        "",
        "Land",
        "{T}: Add {C}.\n{T}: Add one mana of any color.\n{T}: Add one mana of any color among legendary permanents you control.",
    );
    let mut cards = Vec::new();
    for _ in 0..30 {
        cards.push(parse_sim_card(&plaza));
    }
    for _ in 0..30 {
        cards.push(super::model::SimCard {
            name: "Cheap".into(),
            cost: super::model::Cost {
                generic: 1,
                ..super::model::Cost::default()
            },
            min_cost: super::model::Cost {
                generic: 1,
                ..super::model::Cost::default()
            },
            role: super::model::Role::Other,
            ..super::model::SimCard::default()
        });
    }
    let deck = SimDeck {
        cards,
        commanders: vec![],
        format: Format::Constructed,
        rules: super::format::rules_for("constructed"),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(51);
    let log = run_game(&deck, &mut rng, 6);
    // Overproduction check: the turn pool equals the cumulative land count
    // exactly (one tap = one mana). The old model doubled it.
    let lands_played: Vec<u32> = log
        .land_drops
        .iter()
        .scan(0u32, |acc, d| {
            *acc += u32::from(*d);
            Some(*acc)
        })
        .collect();
    for (turn, &lands) in lands_played.iter().enumerate() {
        assert_eq!(
            log.mana_available[turn],
            f64::from(lands),
            "plaza pool must equal land count on turn {}",
            turn + 1
        );
    }
}

#[test]
fn lock_and_booster_roles_classify() {
    let stax = parse_sim_card(&card(
        "Tax Lock",
        "{2}{W}",
        "Enchantment",
        "Spells your opponents cast cost 1 more to cast.",
    ));
    assert_eq!(stax.role, Role::Lock);
    let untap_lock = parse_sim_card(&card(
        "Winter Orb",
        "{2}",
        "Artifact",
        "Marsh creatures don't untap during their controllers' untap steps.",
    ));
    assert_eq!(untap_lock.role, Role::Lock);
    let equipment = parse_sim_card(&card_kw(
        "Sword",
        "{3}",
        "Artifact — Equipment",
        "Equip {2}",
        "Equipped creature gets +2/+2.",
    ));
    assert_eq!(equipment.role, Role::Booster);
    let pump = parse_sim_card(&card(
        "Giant Growth",
        "{G}",
        "Instant",
        "Target creature gets +3/+3 until end of turn.",
    ));
    assert_eq!(pump.role, Role::Booster);
}

#[test]
fn gilded_lotus_parses_three_any_pips() {
    // "Add three mana of any one color" = 3 flexible pips, not 1.
    let lotus = parse_sim_card(&card(
        "Gilded Lotus",
        "Artifact",
        "",
        "{T}: Add three mana of any one color.",
    ));
    let y = lotus.tap.expect("lotus taps");
    assert_eq!(y.any_pips, 3);
    assert_eq!(y.total(), 3);
}

#[test]
fn any_combination_parses() {
    // "Add three mana in any combination of colors" = 3 flexible pips
    // (Golden Throne's sacrifice activation).
    let throne = parse_sim_card(&card(
        "The Golden Throne",
        "Artifact",
        "",
        "{T}, Sacrifice a creature: Add three mana in any combination of colors.",
    ));
    assert!(
        throne
            .abilities()
            .any(|a| matches!(a.effect, Effect::Mana(ref y) if y.any_pips == 3)),
        "sacrifice activation parses with the full 3-pip amount"
    );
}

#[test]
fn fellwar_stone_parses_opponent_any() {
    let fellwar = parse_sim_card(&card(
        "Fellwar Stone",
        "Artifact",
        "",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
    ));
    let y = fellwar.tap.expect("fellwar taps");
    assert!(y.opponent_any);
    assert_eq!(y.any_pips, 1);
}

#[test]
fn faeburrow_elder_parses_colors_present_scaling() {
    let mut elder_row = card(
        "Faeburrow Elder",
        "{1}{G}{W}",
        "Creature — Treefolk Druid",
        "Vigilance\nThis creature gets +1/+1 for each color among permanents you control.\n{T}: For each color among permanents you control, add one mana of that color.",
    );
    elder_row.colors = r#"["G","W"]"#.into();
    let elder = parse_sim_card(&elder_row);
    let y = elder.tap.expect("elder taps");
    assert_eq!(y.scaling, Some(Scale::ColorsPresent));
    assert_eq!(elder.colors, [true, false, false, false, true], "GW");
}

#[test]
fn plaza_of_heroes_legendary_restriction_parses() {
    let plaza = parse_sim_card(&card(
        "Plaza of Heroes",
        "Land",
        "",
        "{T}: Add {C}.\n{T}: Add one mana of any color. Spend this mana only to cast a legendary spell.",
    ));
    let y = plaza.tap.expect("plaza taps");
    assert_eq!(y.restriction, Some(Restriction::Legendary));
}

#[test]
fn steelswarm_operator_artifact_restriction_parses() {
    let op = parse_sim_card(&card(
        "Steelswarm Operator",
        "Artifact Creature",
        "U",
        "Flying\n{T}: Add {U}. Spend this mana only to cast an artifact spell.",
    ));
    let y = op.tap.expect("operator taps");
    assert_eq!(y.restriction, Some(Restriction::Artifact));
}

#[test]
fn opponent_any_yields_nothing_turn_1_any_from_turn_2() {
    // Fellwar Stone: turn 1 produces nothing (no opponent lands yet),
    // turn 2+ produces one flexible pip (best-case reading). Structural
    // check: the parsed yield carries the opponent gate; the turn gate
    // lives in add_yield_turns.
    let fellwar = parse_sim_card(&card(
        "Fellwar Stone",
        "Artifact",
        "",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
    ));
    let y = fellwar.tap.expect("fellwar taps");
    assert!(y.opponent_any);
    assert_eq!(y.any_pips, 1);
}

#[test]
fn astral_cornucopia_parses_per_counter_scaling() {
    let cornucopia = parse_sim_card(&card(
        "Astral Cornucopia",
        "{X}{X}{X}",
        "Artifact",
        "This artifact enters with X charge counters on it.\n{T}: Choose a color. Add one mana of that color for each charge counter on this artifact.",
    ));
    let y = cornucopia.tap.expect("cornucopia taps");
    assert_eq!(y.scaling, Some(Scale::PerChargeCounter));
}

#[test]
fn mox_amber_conditional_any_parses() {
    // "Add one mana of any color among legendary creatures and
    // planeswalkers you control" — parsed as scaling (conditional).
    let mox = parse_sim_card(&card(
        "Mox Amber",
        "{0}",
        "Artifact",
        "{T}: Add one mana of any color among legendary creatures and planeswalkers you control.",
    ));
    let y = mox.tap.expect("mox taps");
    assert_eq!(y.scaling, Some(Scale::ColorsPresent));
}

#[test]
fn spend_restriction_instant_sorcery_parses() {
    let hydro = parse_sim_card(&card(
        "Hydro-Channeler",
        "Creature",
        "U",
        "{T}: Add {U}. Spend this mana only to cast an instant or sorcery spell.",
    ));
    let y = hydro.tap.expect("channeler taps");
    assert_eq!(y.restriction, Some(Restriction::InstantSorcery));
}

#[test]
fn pentad_prism_parses_banked_activation() {
    let prism = card(
        "Pentad Prism",
        "{2}",
        "Artifact",
        "Sunburst (This artifact enters with a charge counter on it for each color of mana spent to cast it.)\nRemove a charge counter from this artifact: Add one mana of any color.",
    );
    let sim = parse_sim_card(&prism);
    // Sunburst best-case: two colors paid → 2 banked pips.
    assert_eq!(sim.enter_counters, 2);
    let banked = sim
        .abilities()
        .find(|a| a.uses_counters)
        .expect("banked activation parsed");
    assert!(!banked.taps, "banked activations do not tap");
    assert!(matches!(banked.effect, Effect::Mana(ref y) if y.any_pips == 1));
}

#[test]
fn enduring_vitality_parses_creature_grant() {
    let mut row = card(
        "Enduring Vitality",
        "{1}{G}{W}",
        "Enchantment",
        "Creatures you control have \"{T}: Add one mana of any color.\"\nWhen Enduring Vitality dies, if it was a creature, return it to the battlefield under its owner's control. It's an enchantment.",
    );
    row.colors = r#"["G","W"]"#.into();
    let vital = parse_sim_card(&row);
    assert_eq!(vital.grant, Some(Grant::Creatures));
}

#[test]
fn chromatic_lantern_parses_land_grant() {
    let lantern = parse_sim_card(&card(
        "Chromatic Lantern",
        "{3}",
        "Artifact",
        "Lands you control have \"{T}: Add one mana of any color.\"\n{T}: Add one mana of any color.",
    ));
    assert_eq!(lantern.grant, Some(Grant::Lands));
}

#[test]
fn treasure_creator_flags() {
    let exec = parse_sim_card(&card(
        "Stark Industries Executive",
        "{2}",
        "Artifact Creature",
        "{2}, {T}: Create a Treasure token.",
    ));
    assert!(exec.treasures_on_token);
    let plain = parse_sim_card(&card("Bear", "{1}{G}", "Creature — Bear", "A bear."));
    assert!(!plain.treasures_on_token);
}

#[test]
fn helix_pinnacle_threshold_is_100_not_10() {
    let pinnacle = card(
        "Helix Pinnacle",
        "{G}",
        "Enchantment",
        "Shield counter on enchanted permanent.\nEnchanted permanent has hexproof.\n{X}: Put X tower counters on Helix Pinnacle.\nIf there are 100 or more tower counters on Helix Pinnacle, you win the game.",
    );
    let sim = parse_sim_card(&pinnacle);
    let threshold = sim
        .abilities()
        .find_map(|a| match a.effect {
            Effect::WinThreshold { counters } => Some(counters),
            _ => None,
        })
        .expect("threshold parsed");
    assert_eq!(threshold, 100);
}

#[test]
fn twobrid_costs_two_generic() {
    // {2/W} costs 2 mana either way; the sim models the generic payment.
    let cost = parse_cost("{2/W}");
    assert_eq!(cost.generic, 2);
    assert_eq!(cost.total(), 2);
    // Spectral Procession: three symbols, six mana total.
    let procession = parse_cost("{2/W}{2/W}{2/W}");
    assert_eq!(procession.generic, 6);
    assert_eq!(procession.total(), 6);
    // No pips leak from the colored half.
    assert!(cost.pips.iter().all(|p| *p == 0));
    assert_eq!(cost.flex_pips, 0);
}
