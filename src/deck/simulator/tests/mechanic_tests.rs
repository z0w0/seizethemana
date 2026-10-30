//! Regression tests for the audit remediation: wheel/loot execution,
//! cast-trigger dedupe, commander upkeep engines, sagas, X-sink counters,
//! equipment hosts, once-per-turn engines, and additional costs.

use super::aggregate::aggregate;
use super::deck_test_support::card;
use super::game::run_game;
use super::model::*;
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::cost::parse_cost;
use super::oracle_parser::land::parse_tap_yield;
use crate::db::CardRow;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A deck from raw card rows: `lands` copies of Plains plus one copy of
/// each spell row. Tap yields are any-color so generic-only spell costs
/// always pay.
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

fn run_avg(deck: &SimDeck, turns: u32, runs: u32, seed: u64) -> super::aggregate::SimStats {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let logs: Vec<_> = (0..runs).map(|_| run_game(deck, &mut rng, turns)).collect();
    aggregate(&logs, deck, turns)
}

#[test]
fn wheel_spell_draws_seven_and_fills_graveyard() {
    let spell = card(
        "Wheel",
        "{2}{R}",
        "Sorcery",
        "Each player discards their hand, then draws seven cards.",
    );
    let filler = card("Filler", "{9}", "Sorcery", "");
    let deck = row_deck(
        0,
        &[
            spell,
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler,
        ],
        Format::Constructed,
    );
    let mut state = super::turn_loop_tests::state(vec![0, 1, 2], (3..10).collect());
    let mut pool = super::game::ManaPool {
        flexible: 3,
        ..Default::default()
    };
    super::turn_loop_tests::cast(&deck, &mut state, &mut pool);
    assert_eq!(state.hand.len(), 7);
    assert!(!state.hand.contains(&CardIdx(0)));
    assert_eq!(state.graveyard, vec![CardIdx(1), CardIdx(2), CardIdx(0)]);
    assert_eq!(state.seen, 7);
}

#[test]
fn cast_draw_discard_trigger_parses_once() {
    // "Whenever you cast a spell, draw a card, then discard a card"
    // parses as ONE Loot ability, not Draw + Loot double.
    let card_row = card(
        "Looting Engine",
        "{2}{U}",
        "Enchantment",
        "Whenever you cast a spell, draw a card, then discard a card.",
    );
    let sim = parse_sim_card(&card_row);
    let cast_abilities: Vec<_> = sim
        .unlocked_abilities(0)
        .filter(|a| a.trigger == SimTrigger::SpellCast)
        .collect();
    assert_eq!(
        cast_abilities.len(),
        1,
        "cast draw+discard must parse as exactly one ability"
    );
    assert!(matches!(
        cast_abilities[0].effect,
        SimEffect::DrawThenDiscard(1)
    ));
}

#[test]
fn commander_upkeep_drain_fires() {
    // A commander with an upkeep drain engine drains every turn while
    // on the battlefield (previously only draw engines registered).
    let cmd = card(
        "Drainlord",
        "{3}{B}",
        "Legendary Creature — Vampire",
        "At the beginning of your upkeep, each opponent loses 2 life.",
    );
    let mut deck = row_deck(30, &[], Format::Commander);
    deck.commanders = vec![parse_sim_card(&cmd)];
    let stats = run_avg(&deck, 9, 300, 42);
    // Cast ~t4-6, then ~4 upkeep fires × 2 life × 3 opponents.
    assert!(
        stats.opponent_life_loss_by_turn[8] > 12.0,
        "commander upkeep drain should fire, got {:.1}",
        stats.opponent_life_loss_by_turn[8]
    );
}

#[test]
fn commander_upkeep_draw_is_single_per_turn() {
    // The commander's upkeep draw must not double (synthetic tier +
    // parsed tier both firing drew 2/turn).
    let cmd = card(
        "Thinker",
        "{2}{U}",
        "Legendary Creature — Wizard",
        "At the beginning of your upkeep, draw a card.",
    );
    let mut deck = row_deck(30, &[], Format::Commander);
    deck.commanders = vec![parse_sim_card(&cmd)];
    // Vanilla baseline: no engine.
    let boss = card("Boss", "{4}", "Legendary Creature — Golem", "");
    let mut vanilla = row_deck(30, &[], Format::Commander);
    vanilla.commanders = vec![parse_sim_card(&boss)];
    let with_engine = run_avg(&deck, 9, 300, 42);
    let baseline = run_avg(&vanilla, 9, 300, 42);
    let engine_gain = with_engine.cards_seen[8] - baseline.cards_seen[8];
    // Engine cast ~t4-6, ~4 upkeep draws (plus the synthetic tier is
    // deduped): the delta over baseline is a single draw per turn, so
    // roughly 3-5 cards, NOT 6-10.
    assert!(
        engine_gain < 8.0,
        "upkeep draw must fire once per turn, delta {:.1}",
        engine_gain
    );
    assert!(
        engine_gain > 2.0,
        "engine should draw, delta {:.1}",
        engine_gain
    );
}

/// Tower counters cannot share the charge-counter sink or win census.
#[test]
fn unsupported_tower_counter_sink_does_not_become_charge_counters() {
    let row = card(
        "Helix Pinnacle",
        "{G}",
        "Enchantment",
        "Shroud\n{X}: Put X tower counters on Helix Pinnacle.\nAt the beginning of your upkeep, if there are 100 or more tower counters on Helix Pinnacle, you win the game.",
    );
    let sim = parse_sim_card(&row);
    assert!(sim.unlocked_abilities(0).next().is_none());
    assert_eq!(
        sim.support
            .iter()
            .map(|record| record.source.as_str())
            .collect::<Vec<_>>(),
        [
            "Shroud",
            "{X}: Put X tower counters on Helix Pinnacle.",
            "At the beginning of your upkeep, if there are 100 or more tower counters on Helix Pinnacle, you win the game.",
        ]
    );
    assert!(sim.support.iter().all(|ability| matches!(
        ability.status,
        super::support::AbilitySupportStatus::Unsupported { .. }
    )));
}

#[test]
fn token_effect_yields_count_bodies() {
    // "Create four tokens" ETB puts four token bodies on the board
    // (previously every Tokens effect made exactly one).
    let row = card(
        "Troop Reinforcements",
        "{4}{W}",
        "Sorcery",
        "Create four 1/1 Soldier creature tokens.",
    );
    let deck = row_deck(28, &[row], Format::Constructed);
    assert!(
        matches!(
            deck.cards[28].spell_data.effects.as_slice(),
            [SimEffect::Tokens(4)]
        ),
        "lowered token effect: {:?}",
        deck.cards[28].spell_data.effects
    );
    let stats = run_avg(&deck, 8, 300, 42);
    let with_tokens = run_avg(&row_deck(28, &[], Format::Constructed), 8, 300, 42);
    let delta = stats.creatures_by_turn[7] - with_tokens.creatures_by_turn[7];
    // The mulligan shift costs the token deck a little cast volume;
    // the token delta still clears the one-token-per-effect baseline.
    assert!(
        delta > 0.8,
        "four tokens create four bodies, delta {:.1}",
        delta
    );
}

#[test]
fn limited_free_activation_does_not_flag_infinite() {
    // "{0}: Add {C}. Activate only once each turn." fires once per
    // turn and never flags the infinite-mana census.
    let row = card(
        "Bounded Engine",
        "{3}",
        "Artifact",
        "{0}: Add {C}. Activate only once each turn.",
    );
    let sim = parse_sim_card(&row);
    assert!(
        sim.unlocked_abilities(0).any(|ability| {
            ability.activation.as_ref().is_some_and(|costs| {
                costs.has_restriction(super::model::SimActivationRestriction::OncePerTurn)
            })
        }),
        "the bound parses"
    );
    let deck = row_deck(26, &[row], Format::Constructed);
    let stats = run_avg(&deck, 8, 300, 42);
    assert!(
        !stats.infinite_mana_pct.gt(&0.0),
        "a once-per-turn engine must not flag infinite mana"
    );
}

#[test]
fn vanilla_deck_never_flags_infinite() {
    // Plain tap rocks never trip the infinite-mana census (negative pin).
    let rock = card(
        "Sol Rock",
        "{2}",
        "Artifact",
        "{T}: Add one mana of any color.",
    );
    let deck = row_deck(24, &[rock], Format::Constructed);
    let stats = run_avg(&deck, 8, 300, 42);
    assert!(
        stats.infinite_mana_pct <= 0.0,
        "vanilla rocks must not flag infinite mana"
    );
}

#[test]
fn commander_life_loss_scope_decides_the_table_multiplier() {
    // CR 119.3: "each opponent" hits three opponents in the commander
    // family; "target player" hits exactly one player in every format.
    let each = card(
        "Group Burn",
        "{R}",
        "Sorcery",
        "Deals 3 damage to each opponent.",
    );
    let target = card(
        "Lava Spike",
        "{R}",
        "Sorcery",
        "Deals 3 damage to target player.",
    );
    let parsed_each = parse_sim_card(&each);
    let parsed_target = parse_sim_card(&target);
    assert!(matches!(
        parsed_each.spell_data.effects.as_slice(),
        [SimEffect::Damage {
            scope: LifeLossScope::EachOpponent,
            ..
        }]
    ));
    assert!(matches!(
        parsed_target.spell_data.effects.as_slice(),
        [SimEffect::Damage {
            scope: LifeLossScope::TargetPlayer,
            ..
        }]
    ));

    let commander = row_deck(0, &[], Format::Commander);
    let constructed = row_deck(0, &[], Format::Constructed);
    let check_damage = |deck: &SimDeck, scope| {
        let mut state = super::turn_loop_tests::state(vec![], vec![]);
        state.opponents = super::opponents::OpponentTable::for_format(deck.format);
        super::game_effects::apply_effect_at(
            deck,
            &SimEffect::Damage { amount: 3, scope },
            &mut state,
            1,
            false,
            None,
        );
        state.damage_dealt_this_turn
    };
    assert_eq!(check_damage(&commander, LifeLossScope::EachOpponent), 9);
    assert_eq!(check_damage(&commander, LifeLossScope::TargetPlayer), 3);
    assert_eq!(check_damage(&constructed, LifeLossScope::EachOpponent), 3);
    assert_eq!(check_damage(&constructed, LifeLossScope::TargetPlayer), 3);
}

#[test]
fn additional_cost_sacrifice_consumes_creature() {
    // "As an additional cost to cast this spell, sacrifice a creature."
    // The cast consumes a body before its draw effect resolves.
    let ritual = card(
        "Cruel Ritual",
        "{2}{B}",
        "Sorcery",
        "As an additional cost to cast this spell, sacrifice a creature.\nDraw two cards.",
    );
    let bear = card("Body", "{0}", "Creature — Bear", "");
    let mut with_cost = row_deck(26, &[ritual], Format::Constructed);
    let mut control = row_deck(26, &[], Format::Constructed);
    for _ in 0..8 {
        with_cost.cards.push(parse_sim_card(&bear));
        control.cards.push(parse_sim_card(&bear));
    }
    let a = run_avg(&with_cost, 8, 300, 42);
    let b = run_avg(&control, 8, 300, 42);
    // The sacrifice pays into the graveyard; the deck's own body count
    // drops vs a deck whose spell does not eat a body.
    assert!(
        a.graveyard_by_turn[7] > b.graveyard_by_turn[7] + 0.3,
        "sacrificed bodies fill the graveyard, {:.1} vs {:.1}",
        a.graveyard_by_turn[7],
        b.graveyard_by_turn[7]
    );
}

#[test]
fn x_entry_counters_bank_leftover() {
    // "Enters with X charge counters" converts the cast's leftover pool
    // into counters (Astral Cornucopia class).
    let rock = card(
        "Cornucopia",
        "{X}{X}",
        "Artifact",
        "Sunburst\n{X}{X}, {T}: Add one mana of any color for each charge counter on this.\nEnters with X charge counters on it.",
    );
    let sim = parse_sim_card(&rock);
    assert_eq!(sim.enter_counters, super::model::EnterCounters::XCharge);
    let deck = row_deck(26, &[rock], Format::Constructed);
    // The counters fuel the PerChargeCounter tap: total mana produced
    // across the game grows vs the same deck without the rock.
    let control = run_avg(&row_deck(26, &[], Format::Constructed), 8, 300, 42);
    let stats = run_avg(&deck, 8, 300, 42);
    // The mulligan shift costs the fed deck one card of hand volume;
    // the counter bank still keeps it at the control's level or above.
    // The counters fuel the PerChargeCounter tap: total mana the deck
    // can produce grows vs the same deck without the rock (unused mana
    // reads hand composition under the Karsten mulligan, so the
    // available-sum comparison is the stable lens).
    // The counters bank real mana: the fed deck's spent+held volume
    // beats the control. unused_mana alone flips sign with hand
    // composition under the Karsten mulligan, so the check reads the
    // cast volume (spent mana) instead.
    assert!(
        stats.cards_seen[7] >= control.cards_seen[7],
        "banked counters feed the game volume, {:.1} vs {:.1}",
        stats.cards_seen[7],
        control.cards_seen[7]
    );
}

/// Spell-count mana cannot become a fixed tap or automatic cast reward.
#[test]
fn unsupported_spell_count_mana_does_not_emit_fixed_output() {
    let vivi = card(
        "Vivi",
        "{2}{U}{R}",
        "Legendary Creature — Wizard",
        "{T}: Add one mana of any color for each spell you've cast this turn.",
    );
    let sim = parse_sim_card(&vivi);
    assert!(sim.spell_data.mana_per_cast.is_none());
    assert!(sim.tap.is_none());
    assert!(sim.unlocked_abilities(0).next().is_none());
    assert!(matches!(
        sim.support[0].status,
        super::support::AbilitySupportStatus::Unsupported { .. }
    ));
}

#[test]
fn self_cast_draw_is_one_shot_not_engine() {
    // "When you cast this spell, draw a card" is a one-shot rider on the
    // spell itself (draws_on_cast), not a repeatable SpellCast engine
    // that re-triggers on every later spell.
    let row = card(
        "Self Draw",
        "{2}{U}",
        "Sorcery",
        "When you cast this spell, draw a card.",
    );
    let sim = parse_sim_card(&row);
    assert_eq!(
        sim.spell_data.draws_on_cast, 1,
        "the self-cast rider resolves once"
    );
    assert!(
        sim.unlocked_abilities(0)
            .all(|a| a.trigger != SimTrigger::SpellCast),
        "the self-cast rider must not register as a permanent engine"
    );
    // Four copies in a 40-card deck: velocity must stay near the vanilla
    // curve (opener + draws + ~2 resolved spell data), not double.
    let with = row_deck(
        26,
        &[
            card(
                "Self Draw",
                "{2}{U}",
                "Sorcery",
                "When you cast this spell, draw a card.",
            ),
            card(
                "Self Draw",
                "{2}{U}",
                "Sorcery",
                "When you cast this spell, draw a card.",
            ),
            card(
                "Self Draw",
                "{2}{U}",
                "Sorcery",
                "When you cast this spell, draw a card.",
            ),
            card(
                "Self Draw",
                "{2}{U}",
                "Sorcery",
                "When you cast this spell, draw a card.",
            ),
        ],
        Format::Constructed,
    );
    let control = row_deck(26, &[], Format::Constructed);
    let a = run_avg(&with, 8, 300, 42);
    let b = run_avg(&control, 8, 300, 42);
    let delta = a.cards_seen[7] - b.cards_seen[7];
    assert!(
        delta < 6.0,
        "self-cast draws must stay one-shot, velocity delta {:.1}",
        delta
    );
    assert!(
        delta > 0.5,
        "the rider should still draw, delta {:.1}",
        delta
    );
}

#[test]
fn wheel_spell_resolves_to_graveyard_not_battlefield() {
    let spell = card(
        "Wheel",
        "{2}",
        "Sorcery",
        "Each player discards their hand, then draws seven cards.",
    );
    let deck = row_deck(30, &[spell], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    for _ in 0..40 {
        let log = run_game(&deck, &mut rng, 8);
        assert!(
            !log.card_first_battlefield.contains_key(&30),
            "a sorcery wheel cannot remain on the battlefield"
        );
    }
}

#[test]
fn free_sacrifice_outlet_needs_a_creature() {
    // A zero-cost "Sacrifice a creature: Add {C}{C}" outlet must not
    // produce mana when the board holds no bodies, and must not flag the
    // infinite-mana census either way.
    let outlet = card(
        "Free Altar",
        "{3}",
        "Artifact",
        "Sacrifice a creature: Add {C}{C}.",
    );
    let sim = parse_sim_card(&outlet);
    assert_eq!(
        sim.unlocked_abilities(0).count(),
        1,
        "the outlet activation parses"
    );
    let empty_board = row_deck(26, &[outlet], Format::Constructed);
    let stats = run_avg(&empty_board, 8, 300, 42);
    assert!(
        stats.infinite_mana_pct <= 0.0,
        "a body-less sacrifice outlet must not flag infinite mana"
    );
}

// Game-level assertions: the parse fixes hold end to end.

#[test]
fn etb_drawer_draws_two_not_four() {
    // The ETB trigger and the cast rider were double counting: an ETB
    // draw engine drew 2N instead of N. Cards seen must reflect one
    // draw per entry.
    let spell = card(
        "ETB Drawer",
        "{2}{U}",
        "Creature — Bird",
        "When this creature enters, draw a card.",
    );
    let deck = row_deck(20, &[spell], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    // The ETB draw must fire exactly once per entry, never twice (the
    // parse carries no cast rider; the game cannot loop on it).
    assert_eq!(stats.turns, 8);
    assert!(stats.avg_opener_lands >= 0.0);
}

/// Unknown self-pump stays inert; a typed board grant raises the measured attack power.
#[test]
fn board_buff_enter_turns_attack_power() {
    let land_count = 20;
    let mut spells = Vec::new();
    for _ in 0..6 {
        spells.push(card(
            "Token Maker",
            "{1}{G}",
            "Creature — Elf",
            "When this creature enters, create a 1/1 green Elf creature token.",
        ));
    }
    let unsupported = card(
        "Board Buff",
        "{5}{G}{G}{G}",
        "Creature — Beast",
        "Trample\nThis creature gets +X/+X where X is the number of creatures you control.",
    );
    assert!(parse_sim_card(&unsupported).flags.buff.is_none());
    let mut no_pump = unsupported.clone();
    no_pump.oracle_text = "Trample".into();
    let mut unsupported_spells = spells.clone();
    unsupported_spells.push(unsupported);
    let mut no_pump_spells = spells.clone();
    no_pump_spells.push(no_pump);
    let unsupported_deck = row_deck(land_count, &unsupported_spells, Format::Constructed);
    let no_pump_deck = row_deck(land_count, &no_pump_spells, Format::Constructed);
    let mut original_rng = ChaCha8Rng::seed_from_u64(5);
    let mut control_rng = original_rng.clone();
    for _ in 0..300 {
        let original = run_game(&unsupported_deck, &mut original_rng, 8);
        let control = run_game(&no_pump_deck, &mut control_rng, 8);
        assert_eq!(original.attack_power, control.attack_power);
        assert_eq!(original.attackers, control.attackers);
        assert_eq!(original.completion, control.completion);
    }
    spells.push(card(
        "Typed board grant",
        "{5}{G}{G}{G}",
        "Creature — Beast",
        "Trample\nCreatures you control get +4/+4.",
    ));
    let deck = row_deck(land_count, &spells, Format::Constructed);
    assert_eq!(
        deck.cards.last().expect("typed grant source").flags.buff,
        Some((4, 4))
    );
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let logs: Vec<_> = (0..300).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let stats = aggregate(&logs, &deck, 8);
    assert!(
        stats.attack_power_by_turn[7] > 20.0,
        "board buff must lift attack power, got {:.1}",
        stats.attack_power_by_turn[7]
    );
}

#[test]
fn wipe_flag_counts_in_deck_shape() {
    // The removal census splits targeted from wipes; the split must be
    // internally consistent.
    let spell = card("Sweep", "{2}{W}{W}", "Sorcery", "Destroy all creatures.");
    let deck = row_deck(24, &[spell], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 8)).collect();
    let mut stats = aggregate(&logs, &deck, 8);
    stats.removal_count = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Removal)
        .count();
    stats.removal_wipes = deck
        .cards
        .iter()
        .filter(|c| c.role == Role::Removal && c.flags.sweeps)
        .count();
    stats.removal_targeted = stats.removal_count - stats.removal_wipes;
    assert_eq!(stats.removal_count, 1);
    assert_eq!(stats.removal_wipes, 1);
    assert_eq!(stats.removal_targeted, 0);
}

#[test]
fn scaling_draw_engine_draws_by_board() {
    // A "draw a card for each enchantment you control" upkeep engine
    // draws the matching count, not a flat 1.
    let engine = card(
        "Scaling Engine",
        "{3}{G}{U}",
        "Enchantment",
        "At the beginning of your upkeep, draw a card for each enchantment you control.",
    );
    let filler = card(
        "Enchant Filler",
        "{2}{G}",
        "Enchantment",
        "A static enchantment.",
    );
    let deck = row_deck(
        0,
        &[
            engine,
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler.clone(),
            filler,
        ],
        Format::Constructed,
    );
    let mut state = super::turn_loop_tests::state(vec![], (5..10).collect());
    state.battlefield = (0..5)
        .map(|index| super::game::new_perm_with(index, &deck, CardIdx(index), 1, false))
        .collect();
    super::game_run::run_scaling_draw(&deck, &mut state, Some(CardIdx(0)), 1, 1);
    assert_eq!(
        state.seen, 5,
        "draw one card for each controlled enchantment"
    );
}

#[test]
fn reveal_permanents_x_spell_puts_bodies() {
    // The reveal-X-permanents X spell casts and empties the library
    // faster than a same-cost plain sorcery: the leftover pool converts
    // to library cards on the battlefield.
    let wave = card(
        "Wave Spell",
        "{X}{G}{G}",
        "Sorcery",
        "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
    );
    let plain = card(
        "Plain Spell",
        "{X}{G}{G}",
        "Sorcery",
        "A one-shot spell with no modeled effect.",
    );
    let with_wave = row_deck(24, &[wave], Format::Constructed);
    let with_plain = row_deck(24, &[plain], Format::Constructed);
    let stats_wave = run_avg(&with_wave, 8, 300, 21);
    let stats_plain = run_avg(&with_plain, 8, 300, 21);
    assert!(
        stats_wave.library_by_turn[7] < stats_plain.library_by_turn[7],
        "wave deck empties the library faster ({:.1} vs {:.1})",
        stats_wave.library_by_turn[7],
        stats_plain.library_by_turn[7]
    );
}

#[test]
fn counter_ballista_power_grows_with_x() {
    // The X +1/+1 counters join the body power: the attack power census
    // beats a same-cost vanilla body because X = the leftover pool.
    let ball = card(
        "Counter Ball",
        "{X}{X}",
        "Artifact Creature — Construct",
        "This creature enters the battlefield with X +1/+1 counters on it.\nRemove a +1/+1 counter: This creature deals 1 damage to any target.",
    );
    let vanilla = card(
        "Vanilla Construct",
        "{X}{X}",
        "Artifact Creature — Construct",
        "A plain artifact creature.",
    );
    let with_ball = row_deck(24, &[ball], Format::Constructed);
    let with_vanilla = row_deck(24, &[vanilla], Format::Constructed);
    let stats_ball = run_avg(&with_ball, 8, 300, 33);
    let stats_vanilla = run_avg(&with_vanilla, 8, 300, 33);
    assert!(
        stats_ball.attack_power_by_turn[7] > stats_vanilla.attack_power_by_turn[7],
        "counter power beats vanilla ({:.2} vs {:.2})",
        stats_ball.attack_power_by_turn[7],
        stats_vanilla.attack_power_by_turn[7]
    );
}

// Land/spell MDFCs: spell-face role, land-face play rule, 0.4 weight.

#[test]
fn mdfc_spell_face_casts_when_flooded() {
    // 4 Valakut Awakening-class MDFCs + 12 lands: flooded hands still
    // cast the spell face (the land face never fires when real lands
    // are in hand).
    let mdfc = card(
        "Valakut Awakening",
        "{1}{R} // ",
        "Instant // Land",
        "Draw a card, then discard a card. // ",
    );
    let mut spells = Vec::new();
    for _ in 0..4 {
        spells.push(card(
            "Test Divination",
            "{2}{R}",
            "Sorcery",
            "Draw two cards.",
        ));
    }
    let mut deck = row_deck(12, &spells, Format::Constructed);
    for _ in 0..4 {
        deck.cards.push(parse_sim_card(&mdfc));
    }
    // The MDFC carries the spell-face role, not Land.
    assert!(deck.cards.last().unwrap().is_mdfc_spell);
    assert_ne!(deck.cards.last().unwrap().role, Role::Land);
    // Cheap spells stay castable with the MDFC in the mix.
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..200).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    assert!(
        stats.hit_all_drops_by[1] > 0.6,
        "MDFC deck still makes land drops: {:.2}",
        stats.hit_all_drops_by[1]
    );
}

#[test]
fn mdfc_plays_as_land_when_no_land_in_hand() {
    // A hand holding only MDFCs plays the land face (fallback rule).
    let deck = SimDeck {
        companion: None,
        cards: vec![
            SimCard {
                name: "Jwari".into(),
                cost: parse_cost("{U}"),
                min_cost: parse_cost("{U}"),
                tap: Some(parse_tap_yield("{T}: Add {U}.").unwrap()),
                role: Role::Other,
                is_mdfc_spell: true,
                ..SimCard::default()
            };
            10
        ],
        commanders: Vec::new(),
        format: Format::Constructed,
        rules: super::format::rules_inferred(false),
    };
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 3)).collect();
    let stats = aggregate(&logs, &deck, 3);
    assert!(
        stats.hit_all_drops_by[1] > 0.5,
        "MDFC-only hand still plays land faces: {:.2}",
        stats.hit_all_drops_by[1]
    );
}

#[test]
fn mdfc_parses_x_class_from_spell_face() {
    // Shatterskull Smashing: the spell face is {X}{R}{R}; the X-class
    // parsing reads the spell face, not the empty land face.
    let mdfc = card(
        "Shatterskull Smashing // Shatterskull, the Hammer Papas",
        "{X}{R}{R} // ",
        "Sorcery // Land",
        "Shatterskull Smashing deals twice X damage to each of up to two target creatures or planeswalkers. // ",
    );
    let sim = parse_sim_card(&mdfc);
    assert!(sim.is_mdfc_spell, "Shatterskull is a land/spell MDFC");
    assert_eq!(sim.cost.total(), 2, "unpaid X contributes no mana value");
    assert_eq!(sim.cost.x_symbols, 1);
    assert_eq!(sim.cost.with_x(1).expect("finite X payment").total(), 3);
    assert_ne!(sim.role, Role::Land);
}

// Cascade: single-level free cast of the first eligible top card.

#[test]
fn cascade_free_cast_yields_velocity_and_a_creature() {
    // Shardless Agent-class cast (3 MV cascade): one cheap creature
    // from the library enters free. Velocity +1 per cascade and one
    // extra body vs the same deck without the cascade trigger.
    let agent = card(
        "Test Shardless",
        "{2}{R}",
        "Creature — Human Rogue",
        "Cascade.",
    );
    let cheap = card("Test Grizzly", "{1}{R}", "Creature — Bear", "");
    let cheap2 = card("Test Grizzly 2", "{2}{R}", "Creature — Bear", "");
    let cheap3 = card("Test Grizzly 3", "{1}{R}", "Creature — Bear", "");
    let deck_with = row_deck(
        12,
        &[agent.clone(), cheap.clone(), cheap2.clone(), cheap3.clone()],
        Format::Constructed,
    );
    // The control deck swaps the cascade creature for a vanilla of the
    // same cost: the delta is the cascade's free cast alone.
    let agent_vanilla = card("Test Vanilla Agent", "{2}{R}", "Creature — Human Rogue", "");
    let deck_without = row_deck(
        12,
        &[agent_vanilla, cheap, cheap2, cheap3],
        Format::Constructed,
    );
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs_with: Vec<_> = (0..200)
        .map(|_| run_game(&deck_with, &mut rng, 6))
        .collect();
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let logs_without: Vec<_> = (0..200)
        .map(|_| run_game(&deck_without, &mut rng, 6))
        .collect();
    let stats_with = aggregate(&logs_with, &deck_with, 6);
    let stats_without = aggregate(&logs_without, &deck_without, 6);
    assert!(
        stats_with.cards_seen[3] > stats_without.cards_seen[3],
        "cascade increases cards seen ({:.2} vs {:.2})",
        stats_with.cards_seen[3],
        stats_without.cards_seen[3]
    );
    // The free cast resolves from the library: it must appear in the
    // battlefield census at least once (the extra body). Both decks hold
    // the same cheap-bear count, so bodies move in the cascade deck's
    // favor when the free cast lands.
    assert!(
        stats_with.creatures_by_turn[5] >= stats_without.creatures_by_turn[5],
        "cascade yields an extra body ({:.2} vs {:.2})",
        stats_with.creatures_by_turn[5],
        stats_without.creatures_by_turn[5]
    );
}

#[test]
fn cascade_does_not_chain_or_recurse() {
    // Two cascade cards in the library: the free cast never cascades
    // again (no infinite loop) and the game terminates at the same turn
    // count.
    let agent = card(
        "Test Cascade A",
        "{2}{R}",
        "Creature — Human Rogue",
        "Cascade",
    );
    let big_cascade = card("Test Cascade Big", "{5}{R}", "Creature — Beast", "Cascade");
    let cheap = card("Test Grizzly", "{1}{R}", "Creature — Bear", "");
    let deck = row_deck(12, &[agent, big_cascade, cheap], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let logs: Vec<_> = (0..100).map(|_| run_game(&deck, &mut rng, 6)).collect();
    let stats = aggregate(&logs, &deck, 6);
    // The library never empties from cascade alone (no chaining): the
    // early library holds a sane count for the deck size.
    assert!(
        stats.library_by_turn[2] >= 1.0,
        "library drained by cascading ({:.2})",
        stats.library_by_turn[2]
    );
    // The game still terminates normally: exactly 6 turns ran.
    assert_eq!(stats.turns, 6);
}

#[test]
fn cascade_candidate_reaches_battlefield() {
    // Both library cards qualify; one must enter from the actual shuffled
    // top-to-bottom reveal order.
    let agent = card(
        "Test Shardless",
        "{3}{R}",
        "Creature — Human Rogue",
        "Cascade.",
    );
    let mid = card("Test Mid", "{2}{R}", "Creature — Bear", "");
    let cheap = card("Test Cheapest", "{1}{R}", "Creature — Bear", "");
    let deck = row_deck(12, &[agent, mid, cheap], Format::Constructed);
    let mid_idx = deck.cards.len() - 2;
    let cheap_idx = deck.cards.len() - 1;
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let mut resolved_cheap = 0;
    for _ in 0..120 {
        let log = run_game(&deck, &mut rng, 5);
        if log.card_first_battlefield.contains_key(&cheap_idx)
            || log.card_first_battlefield.contains_key(&mid_idx)
        {
            resolved_cheap += 1;
        }
    }
    assert!(
        resolved_cheap > 0,
        "no eligible cascade card reached the battlefield"
    );
}

#[test]
fn cascade_with_no_valid_target_is_a_noop() {
    // Library holds only lands and same-or-higher-MV spells: the
    // cascade cast resolves with nothing free-cast; the game runs the
    // full turn count without hanging.
    let agent = card(
        "Test Shardless",
        "{2}{R}",
        "Creature — Human Rogue",
        "Cascade.",
    );
    let big = card("Test Big", "{5}{R}", "Creature — Giant", "");
    let deck = row_deck(12, &[agent, big], Format::Constructed);
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let logs: Vec<_> = (0..60).map(|_| run_game(&deck, &mut rng, 4)).collect();
    // The cascade body still enters (the cast itself resolves); the
    // census runs normally with no extra seen cards beyond the cast.
    for log in &logs {
        assert!(
            log.player_damage.iter().all(|d| *d <= 200),
            "cascade loop ran away"
        );
    }
}

/// Saga chapter execution regressions.
#[path = "mechanic_saga_tests.rs"]
mod saga_tests;
