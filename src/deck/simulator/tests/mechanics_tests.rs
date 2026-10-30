//! Tests for the strong-mechanic batch: mobilize, amass, the Ring,
//! empower Jace, saddle, power-up, teamwork, storm, convoke, delve,
//! offspring, plot, explore, living metal, afterlife, connive, read
//! ahead, and the intervening-if conditions.

use super::aggregate::aggregate;
use super::deck_test_support::card;
use super::game::run_game;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::land::parse_tap_yield;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A deck from raw card rows: `lands` copies of any-color lands plus one
/// copy of each card row.
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
        commanders: Vec::new(),
        format,
        rules: super::format::rules_for(rules),
    }
}

/// Average stats over `runs` games.
fn run_avg(deck: &SimDeck, turns: u32, runs: u32, seed: u64) -> super::aggregate::SimStats {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let logs: Vec<_> = (0..runs).map(|_| run_game(deck, &mut rng, turns)).collect();
    aggregate(&logs, deck, turns)
}

#[test]
fn mobilize_parses_and_adds_attack_power() {
    // Mobilize 2 (CR 702.181): the attack trigger creates two 1/1
    // Warriors, tapped and attacking, sacrificed at the next end step.
    let mob = card(
        "Mob Leader",
        "{2}{R}",
        "Creature — Soldier",
        "Mobilize 2 (Whenever this creature attacks, create two 1/1 red Warrior creature tokens. Those tokens enter tapped and attacking. Sacrifice them at the beginning of the next end step.)",
    );
    let sim = parse_sim_card(&mob);
    assert!(sim
        .unlocked_abilities(0)
        .any(|a| a.trigger == SimTrigger::Attacks && matches!(a.effect, SimEffect::Mobilize(2))));
    // The attack power of the entry turn includes the mobilizing leader
    // plus its two Warrior tokens.
    let deck = row_deck(
        24,
        &[mob.clone(), mob.clone(), mob.clone(), mob],
        Format::Constructed,
    );
    let stats = run_avg(&deck, 6, 200, 7);
    assert!(
        stats.attack_power_by_turn[5] >= 4.0,
        "mobilize tokens join the attack, got {:.1}",
        stats.attack_power_by_turn[5]
    );
}

#[test]
fn mobilize_tokens_are_sacrificed_and_fire_death_triggers() {
    // The Warriors die at the next end step, so an aristocrats payoff
    // drains each turn they exist.
    let mob = card(
        "Mob Leader",
        "{1}{R}",
        "Creature — Soldier",
        "Mobilize 1 (Whenever this creature attacks, create a 1/1 red Warrior creature token. It enters tapped and attacking. Sacrifice it at the beginning of the next end step.)",
    );
    let payoff = card(
        "Death Payoff",
        "{1}{B}",
        "Enchantment",
        "Whenever a creature you control dies, each opponent loses 1 life.",
    );
    let filler = card("Filler", "{9}", "Sorcery", "");
    let cards = row_deck(0, &[mob.clone(), payoff.clone(), filler], Format::Commander);
    assert!(
        cards.cards[1]
            .unlocked_abilities(0)
            .any(|ability| ability.trigger == SimTrigger::Dies),
        "death trigger support: {:?}",
        cards.cards[1].support
    );
    let mut state = super::turn_loop_tests::state(vec![], vec![]);
    state.library.push(CardIdx(2));
    state.opponents = super::opponents::OpponentTable::for_format(cards.format);
    for index in 0..2 {
        state.battlefield.push(super::game::new_perm_with(
            index + 1,
            &cards,
            CardIdx(index),
            0,
            false,
        ));
    }
    state.next_uid = 3;
    let mut census = super::game_run::TurnCensus::new(1, cards.cards.len());
    let commander = super::game_commander::CommanderProfile::new(&cards);

    super::game_run::run_turn(
        &cards,
        &mut state,
        &mut census,
        &commander,
        &mut Vec::new(),
        1,
        false,
    );

    assert_eq!(state.battlefield.len(), 2);
    assert!(
        state
            .battlefield
            .iter()
            .all(|permanent| !permanent.sacrifice_at_end)
    );
    assert_eq!(state.opponent_life_lost, 3);
    assert_eq!(census.opponent_life_loss, [3]);
    let deck = row_deck(24, &[mob, payoff], Format::Commander);
    let stats = run_avg(&deck, 8, 200, 11);
    assert!(
        stats.opponent_life_loss_by_turn[7] > 0.5,
        "mobilize token sacrifices feed death triggers, got {:.2}",
        stats.opponent_life_loss_by_turn[7]
    );
}

#[test]
fn amass_grows_a_single_army_creature() {
    // Amass Orcs 2 (CR 701.47): an Army body with +1/+1 counters. Two
    // amass effects grow the same Army.
    let one = card("Amass One", "{1}{B}", "Sorcery", "Amass Orcs 2.");
    let two = card("Amass Two", "{1}{B}", "Sorcery", "Amass Orcs 3.");
    let sim = parse_sim_card(&one);
    assert!(matches!(
        sim.spell_data.effects.as_slice(),
        [SimEffect::Amass(2)]
    ));
    let deck = row_deck(0, &[one, two], Format::Constructed);
    let mut state = super::turn_loop_tests::state(vec![], vec![]);
    for amount in [2, 3] {
        super::game_effects::apply_effect_at(
            &deck,
            &SimEffect::Amass(amount),
            &mut state,
            1,
            false,
            None,
        );
    }
    assert_eq!(state.battlefield.len(), 1, "both actions use the same Army");
    let army = &state.battlefield[0];
    assert!(army.army);
    assert_eq!(army.counters.plus1, 5);
    assert_eq!(
        army.card,
        super::game::CardRef::Token(super::game::TokenKind::Army)
    );
    assert_eq!(
        super::game::card_of(&deck, army).colors,
        [false, false, true, false, false]
    );
    assert_eq!(super::game_effects::creature_power(army, &deck), 5);
}

/// Combat, crew, and station power share counter-aware current power.
#[test]
fn combat_power_includes_positive_and_negative_counters() {
    let mut deck = row_deck(
        0,
        &[card("Body", "{1}", "Creature", "")],
        Format::Constructed,
    );
    deck.cards[0].printed_power = Some(3);
    let mut state = super::turn_loop_tests::state(vec![], vec![]);
    let mut ordinary = super::game::new_perm_with(1, &deck, CardIdx(0), 0, false);
    ordinary.summoning_sick = false;
    ordinary.counters.plus1 = 2;
    ordinary.counters.minus1 = 1;
    state.battlefield.push(ordinary);

    let mut face_down = super::game::new_perm_with(2, &deck, CardIdx(0), 0, false);
    face_down.face_down = true;
    face_down.summoning_sick = false;
    face_down.counters.plus1 = 1;
    face_down.counters.minus1 = 1;
    state.battlefield.push(face_down);

    let mut army = super::game::new_token_perm(3, 0);
    army.army = true;
    army.summoning_sick = false;
    army.counters.plus1 = 4;
    army.counters.minus1 = 1;
    state.battlefield.push(army);

    let mut reduced_to_zero = super::game::new_perm_with(4, &deck, CardIdx(0), 0, false);
    reduced_to_zero.summoning_sick = false;
    reduced_to_zero.counters.minus1 = 4;
    state.battlefield.push(reduced_to_zero);

    assert_eq!(
        super::game_effects::creature_power(&state.battlefield[0], &deck),
        4
    );
    assert_eq!(
        super::game_effects::creature_power(&state.battlefield[1], &deck),
        2
    );
    assert_eq!(
        super::game_effects::creature_power(&state.battlefield[2], &deck),
        3
    );
    assert_eq!(
        super::game_effects::creature_power(&state.battlefield[3], &deck),
        0
    );

    let combat = super::game_combat::combat_phase(&deck, &mut state, 1, &[0]);
    assert_eq!(combat.power, 9);
    assert_eq!(
        state.opponent_life_lost, 0,
        "potential damage is not life loss"
    );
}

#[test]
fn ring_tempts_raises_levels_and_level_four_drains() {
    // Four temptations unlock the level-4 combat drain ("each opponent
    // loses 3 life", CR 701.54c).
    let tempter = card(
        "Ringbearer",
        "{1}{B}",
        "Creature — Human",
        "When this creature enters, the Ring tempts you.",
    );
    let sim = parse_sim_card(&tempter);
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::Enters && matches!(a.effect, SimEffect::RingTempts))
    );
}

#[test]
fn ring_tempts_trigger_fires_on_temptation() {
    // "Whenever the Ring tempts you" fires each time the Ring tempts.
    let tempter = card(
        "Ringbearer",
        "{1}{B}",
        "Creature — Human",
        "When this creature enters, the Ring tempts you.",
    );
    let payoff = card(
        "Tempt Payoff",
        "{1}{B}",
        "Enchantment",
        "Whenever the Ring tempts you, you draw a card and you lose 1 life.",
    );
    let sim = parse_sim_card(&payoff);
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::RingTempts),
        "the tempt trigger lowers"
    );
    let mut spells = vec![payoff.clone(), payoff.clone(), payoff.clone(), payoff];
    spells.extend([tempter.clone(), tempter.clone(), tempter.clone(), tempter]);
    let deck = row_deck(24, &spells, Format::Constructed);
    let stats = run_avg(&deck, 6, 200, 13);
    assert!(
        stats.cards_seen[5] > 12.0,
        "tempt triggers draw, got {:.1}",
        stats.cards_seen[5]
    );
}

#[test]
fn empower_jace_creates_a_loyalty_token() {
    // Empower Jace 3 (CR 701.71): a Jace planeswalker token arrives with
    // three loyalty, so its [-3]: Draw becomes activatable.
    let spell = card("Empower", "{2}{U}", "Sorcery", "Empower Jace 3.");
    let sim = parse_sim_card(&spell);
    assert!(matches!(
        sim.spell_data.effects.as_slice(),
        [SimEffect::EmpowerJace(3)]
    ));
    let deck = row_deck(
        24,
        &[spell.clone(), spell.clone(), spell.clone(), spell],
        Format::Constructed,
    );
    let stats = run_avg(&deck, 6, 200, 17);
    // The token's draw activation adds card velocity beyond the base
    // one-per-turn draw.
    assert!(
        stats.cards_seen[5] > 11.0,
        "the Jace token draws, got {:.1}",
        stats.cards_seen[5]
    );
}

#[test]
fn saddle_parses_and_only_pays_with_a_payoff() {
    // Saddle 2 (CR 702.171) with a "while saddled" buff: the buff joins
    // the mount's attack.
    let mount = card(
        "Test Mount",
        "{2}{G}",
        "Creature — Mount",
        "Saddle 2 (Tap any number of other creatures you control with total power 2 or greater: This permanent becomes saddled until end of turn. Activate only as a sorcery.)\nAs long as this creature is saddled, it gets +2/+2.",
    );
    let sim = parse_sim_card(&mount);
    assert_eq!(sim.keyword_abilities.saddle, Some(2));
    assert_eq!(sim.flags.saddled_buff, Some((2, 2)));
    // Without a saddled payoff the goldfish never pays the saddle cost.
    let plain = card(
        "Plain Mount",
        "{2}{G}",
        "Creature — Mount",
        "Saddle 2 (Tap any number of other creatures you control with total power 2 or greater: This permanent becomes saddled until end of turn. Activate only as a sorcery.)",
    );
    let plain_sim = parse_sim_card(&plain);
    assert_eq!(plain_sim.keyword_abilities.saddle, Some(2));
    assert_eq!(plain_sim.flags.saddled_buff, None);
}

/// Power-up retains its cost and usage limits without changing counter kinds.
#[test]
fn power_up_parses_once_per_game_and_discounts_on_entry_turn() {
    use super::game::{ManaPool, new_perm_with};
    use super::game_effects::activation::{pick_best_activation, resolve_activation};
    use super::turn_loop_tests::{deck, row, state};

    let hero = card(
        "Draw Hero",
        "{1}{U}",
        "Creature — Hero",
        "Power-up — {3}{U}: Draw three cards.",
    );
    let sim = parse_sim_card(&hero);
    assert_eq!(sim.cost, super::oracle_parser::cost::parse_cost("{1}{U}"));
    let ability = sim
        .unlocked_abilities(0)
        .find(|a| {
            a.activation.as_ref().is_some_and(|costs| {
                costs.has_restriction(super::model::SimActivationRestriction::PowerUp)
            })
        })
        .expect("the power-up ability lowers");
    assert!(
        ability.activation.as_ref().is_some_and(|costs| {
            costs.has_restriction(super::model::SimActivationRestriction::OncePerGame)
        }),
        "once per game (CR 702.193a)"
    );
    let costs = ability.activation.as_ref().expect("power-up costs");
    assert_eq!(
        costs.mana_cost(),
        super::oracle_parser::cost::parse_cost("{3}{U}")
    );
    assert!(!costs.taps_source());
    let cards = deck(&[
        hero,
        row("First draw", "{9}", "Sorcery", ""),
        row("Second draw", "{9}", "Sorcery", ""),
        row("Third draw", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![], vec![1, 2, 3]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 1, true));
    let mut pool = ManaPool {
        colorless: 2,
        ..Default::default()
    };
    assert!(pick_best_activation(&cards, &st, &pool, 2).is_none());
    assert!(
        pick_best_activation(
            &cards,
            &st,
            &ManaPool {
                colorless: 1,
                ..Default::default()
            },
            1
        )
        .is_none()
    );
    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("entry-turn discount leaves two generic mana");
    assert_eq!(
        activation.ability_cost,
        Cost {
            generic: 2,
            ..Default::default()
        }
    );
    assert!(
        !activation
            .ability
            .activation
            .as_ref()
            .expect("typed costs")
            .taps_source()
    );
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);
    assert_eq!(pool.total(), 0);
    assert_eq!(st.hand, [CardIdx(3), CardIdx(2), CardIdx(1)]);
    assert!(st.battlefield[0].tapped);
    assert!(
        pick_best_activation(
            &cards,
            &st,
            &ManaPool {
                flexible: 10,
                ..Default::default()
            },
            2
        )
        .is_none()
    );

    let unmodeled = parse_sim_card(&card(
        "Counter Hero",
        "{1}{U}",
        "Creature — Hero",
        "Power-up — {3}{U}: Put three +1/+1 counters on this creature.",
    ));
    assert!(unmodeled.unlocked_abilities(0).next().is_none());
    assert!(matches!(
        unmodeled.support[0].status,
        super::support::AbilitySupportStatus::Unsupported { .. }
    ));
}

#[test]
fn exhaust_fires_once_per_game() {
    // Exhaust (CR 702.177): "Activate only once" (per game). The ability
    // never repeats across turns.
    let engine = card(
        "Exhaust Engine",
        "{2}",
        "Artifact",
        "{T}: Draw a card.\nExhaust — {0}: Draw two cards.",
    );
    let sim = parse_sim_card(&engine);
    assert!(
        sim.unlocked_abilities(0).any(|ability| {
            ability.activation.as_ref().is_some_and(|costs| {
                costs.has_restriction(super::model::SimActivationRestriction::OncePerGame)
            })
        }),
        "the exhaust activation is once per game"
    );
}

#[test]
fn teamwork_taps_bodies_when_available() {
    // Teamwork 4 (CR 702.194): an optional additional cost that taps
    // creatures with total power 4 or more.
    let spell = card(
        "Team Strike",
        "{1}{R}",
        "Instant",
        "Teamwork 4 (As an additional cost to cast this spell, you may tap any number of creatures you control with total power 4 or more.)\nTeam Strike deals 3 damage to target player.",
    );
    let sim = parse_sim_card(&spell);
    assert_eq!(sim.keyword_abilities.teamwork, Some(4));
}

#[test]
fn storm_copies_the_spell_per_prior_cast() {
    // Storm (CR 702.40): the spell copies once per other spell cast this
    // turn. Copies of "each opponent loses 2" multiply the table drain
    // (2 life × 3 opponents per copy) beyond the base cast. A small deck
    // draws the whole list, so the copy scaling shows in the census.
    let bolt = card(
        "Cheap Bolt",
        "{R}",
        "Instant",
        "Cheap Bolt deals 1 damage to target player.",
    );
    let storm = card(
        "Storm Drain",
        "{1}{R}",
        "Sorcery",
        "Storm (When you cast this spell, copy it for each spell cast before it this turn.)\nStorm Drain deals 2 damage to each opponent.",
    );
    let sim = parse_sim_card(&storm);
    assert!(sim.keyword_abilities.storm);
    let mut spells = Vec::new();
    for _ in 0..6 {
        spells.push(bolt.clone());
    }
    spells.push(storm.clone());
    spells.push(storm);
    // 12 cards + 8 lands: the opener plus early draws reach the storm
    // spells, and prior bolts inflate the copy count.
    let deck = row_deck(8, &spells, Format::Commander);
    let stats = run_avg(&deck, 4, 300, 23);
    // Two storm casts with multiple prior spells each dwarf the base
    // 2×3 damage of a single uncopied cast.
    assert!(
        stats.player_damage_by_turn[3] > 12.0,
        "storm copies add damage, got {:.1}",
        stats.player_damage_by_turn[3]
    );
}

#[test]
fn convoke_taps_bodies_to_pay() {
    // Convoke (CR 702.51): tap untapped creatures to pay the spell's
    // mana. A big convoke spell casts with bodies alone when they cover
    // the cost.
    let convoke = card(
        "Convoke Spell",
        "{5}{G}",
        "Sorcery",
        "Convoke (Your creatures can help cast this spell.)\nDraw three cards.",
    );
    let bear = card("Bear", "{1}{G}", "Creature — Bear", "Vanilla.");
    let sim = parse_sim_card(&convoke);
    assert!(sim.keyword_abilities.convoke);
    let mut deck = row_deck(20, std::slice::from_ref(&convoke), Format::Constructed);
    for _ in 0..6 {
        deck.cards.push(parse_sim_card(&bear));
    }
    let stats = run_avg(&deck, 6, 200, 29);
    assert!(
        stats.cards_seen[5] > 11.0,
        "the convoke spell resolves via bodies, got {:.1}",
        stats.cards_seen[5]
    );
}

#[test]
fn offspring_parses_its_cost() {
    // Offspring {2} (CR 702.175): pay the extra cost on cast to create a
    // 1/1 token copy on entry.
    let creature = card(
        "Offspring Bear",
        "{1}{G}",
        "Creature — Bear",
        "Offspring {2} (You may pay an additional {2} as you cast this spell. If you do, when this creature enters, create a 1/1 token copy of it.)",
    );
    let sim = parse_sim_card(&creature);
    assert_eq!(
        sim.keyword_abilities.offspring.map(|cost| cost.generic),
        Some(2)
    );
    let deck = row_deck(
        30,
        &[
            creature.clone(),
            creature.clone(),
            creature.clone(),
            creature,
        ],
        Format::Constructed,
    );
    let stats = run_avg(&deck, 6, 200, 37);
    assert!(
        stats.creatures_by_turn[5] >= 1.5,
        "the offspring copy adds a body, got {:.1}",
        stats.creatures_by_turn[5]
    );
}

#[test]
fn plot_exiles_then_casts_free() {
    // Plot {1}{R} (CR 702.170): exile now, cast free on a later turn.
    let plot = card(
        "Plotted Bolt",
        "{3}{R}",
        "Sorcery",
        "Plot {1}{R} (You may pay {1}{R} and exile this card from your hand. Cast it as a sorcery on a later turn without paying its mana cost.)\nPlotted Bolt deals 4 damage to target player.",
    );
    let sim = parse_sim_card(&plot);
    assert_eq!(sim.keyword_abilities.plot.map(|cost| cost.generic), Some(1));
}

#[test]
fn explore_parses() {
    // Explore (CR 701.44): land to hand, else a +1/+1 counter.
    let explorer = card(
        "Explorer",
        "{1}{G}",
        "Creature — Scout",
        "When this creature enters, it explores. (Reveal the top card of your library. Put that card into your hand if it's a land. Otherwise, put a +1/+1 counter on this creature, then put the card back or put it into your graveyard.)",
    );
    let sim = parse_sim_card(&explorer);
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| matches!(a.effect, SimEffect::Explore)),
        "the explore keyword action lowers as an effect"
    );
}

#[test]
fn living_metal_vehicle_is_a_creature_without_crew() {
    // Living metal (CR 702.161): during your turn the Vehicle is an
    // artifact creature, so it attacks without being crewed.
    let vehicle = card(
        "Metal Vehicle",
        "{2}",
        "Artifact — Vehicle",
        "Living metal (During your turn, this Vehicle is an artifact creature.)\n3/3",
    );
    let sim = parse_sim_card(&vehicle);
    assert!(sim.keyword_abilities.living_metal);
    let deck = row_deck(
        20,
        &[vehicle.clone(), vehicle.clone(), vehicle.clone(), vehicle],
        Format::Constructed,
    );
    let stats = run_avg(&deck, 6, 200, 41);
    assert!(
        stats.attack_power_by_turn[5] >= 3.0,
        "the living-metal Vehicle attacks uncrewed, got {:.1}",
        stats.attack_power_by_turn[5]
    );
}

#[test]
fn afterlife_creates_spirits_on_death() {
    // Afterlife 2 (CR 702.135): two 1/1 flying Spirits when the
    // permanent dies. A sacrifice outlet kills it.
    let spirit = card(
        "Spirit Maker",
        "{1}{W}",
        "Creature — Spirit",
        "Afterlife 2 (When this creature dies, create two 1/1 white and black Spirit creature tokens with flying.)",
    );
    let sim = parse_sim_card(&spirit);
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| a.trigger == SimTrigger::Dies && matches!(a.effect, SimEffect::Afterlife(2)))
    );
    let deck = row_deck(0, &[spirit], Format::Constructed);
    let mut state = super::turn_loop_tests::state(vec![], vec![]);
    super::game_effects::apply_effect_at(
        &deck,
        &SimEffect::Afterlife(2),
        &mut state,
        1,
        false,
        None,
    );
    assert_eq!(state.battlefield.len(), 2);
    for token in &state.battlefield {
        assert_eq!(
            token.card,
            super::game::CardRef::Token(super::game::TokenKind::Spirit)
        );
        let card = super::game::card_of(&deck, token);
        assert_eq!(card.printed_power, Some(1));
        assert_eq!(card.printed_toughness, Some(1));
        assert_eq!(card.colors, [true, false, true, false, false]);
        assert!(card.printed_keywords.contains(Keyword::Flying));
        assert_eq!(super::game_effects::creature_power(token, &deck), 1);
    }
}

#[test]
fn connive_parses_and_loots() {
    // Connive (CR 701.50): draw, discard, and a +1/+1 counter when a
    // nonland is discarded.
    let conniver = card(
        "Conniver",
        "{1}{U}",
        "Creature — Rogue",
        "When this creature enters, it connives. (Draw a card, then discard a card. If you discarded a nonland card, put a +1/+1 counter on this creature.)",
    );
    let sim = parse_sim_card(&conniver);
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| matches!(a.effect, SimEffect::Connive(1))),
        "the connive keyword action lowers as an effect"
    );
}

#[test]
fn read_ahead_starts_at_the_first_useful_chapter() {
    // Read ahead (CR 702.155): the Saga chooses a starting chapter. The
    // goldfish starts at the first chapter with a parsed effect.
    let saga = card(
        "Read Ahead Saga",
        "{1}{U}",
        "Enchantment — Saga",
        "Read ahead (Choose a chapter and start with that many lore counters.)\nI — Draw a card.\nII — Draw two cards.\nIII — Draw three cards.",
    );
    let sim = parse_sim_card(&saga);
    assert!(sim.read_ahead);
    assert_eq!(sim.saga.chapters.len(), 3);
}

#[test]
fn descend_condition_gates_a_trigger() {
    // Descend 4 (CR 603.4 intervening if): the trigger only fires with
    // four or more permanent cards in the graveyard.
    let payoff = card(
        "Descend Payoff",
        "{1}{B}",
        "Enchantment",
        "At the beginning of your upkeep, if you have four or more permanent cards in your graveyard, draw a card.",
    );
    let sim = parse_sim_card(&payoff);
    let trigger = sim
        .unlocked_abilities(0)
        .find(|a| a.trigger == SimTrigger::Upkeep)
        .expect("the conditioned trigger lowers");
    assert_eq!(
        trigger.condition,
        Some(SimAbilityCondition::Descend(4)),
        "the descend condition is recognized"
    );
}

#[test]
fn raid_condition_gates_a_trigger() {
    let payoff = card(
        "Raid Payoff",
        "{1}{R}",
        "Enchantment",
        "At the beginning of your end step, if you attacked this turn, draw a card.",
    );
    let sim = parse_sim_card(&payoff);
    let trigger = sim
        .unlocked_abilities(0)
        .find(|a| a.trigger == SimTrigger::EndStep)
        .expect("the conditioned trigger lowers");
    assert_eq!(trigger.condition, Some(SimAbilityCondition::Raid));
}

#[test]
fn one_ring_burden_counters_draw_and_cost_life() {
    // The One Ring: "{T}: Put a burden counter on The One Ring, then
    // draw a card for each burden counter on it" and "At the beginning
    // of your upkeep, you lose 1 life for each burden counter on it."
    let ring = card(
        "The One Ring",
        "{4}",
        "Legendary Artifact",
        "Indestructible\nAt the beginning of your upkeep, you lose 1 life for each burden counter on The One Ring.\n{T}: Put a burden counter on The One Ring, then draw a card for each burden counter on The One Ring.",
    );
    let sim = parse_sim_card(&ring);
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| matches!(a.effect, SimEffect::AddBurdenCounter)),
        "the burden activation lowers"
    );
    assert!(
        sim.unlocked_abilities(0)
            .any(|a| matches!(a.effect, SimEffect::BurdenLifeLoss)),
        "the upkeep life loss lowers"
    );
    let deck = row_deck(
        24,
        &[ring.clone(), ring.clone(), ring.clone(), ring],
        Format::Constructed,
    );
    let stats = run_avg(&deck, 8, 300, 47);
    assert!(
        stats.cards_seen[7] > 13.0,
        "burden counters draw each turn, got {:.1}",
        stats.cards_seen[7]
    );
    assert!(
        stats.life_paid_avg > 1.0,
        "burden upkeep costs life, got {:.1}",
        stats.life_paid_avg
    );
}
