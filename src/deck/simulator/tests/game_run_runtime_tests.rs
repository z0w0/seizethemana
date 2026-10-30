use super::*;

/// Printed additional-land counts stack and disappear with their sources.
#[test]
fn additional_land_rules_keep_counts_instead_of_booleans() {
    let cards = deck(&[
        row(
            "Two extra lands",
            "{2}{G}",
            "Legendary Creature",
            "You may play two additional lands on each of your turns.",
        ),
        row(
            "One extra land",
            "{G}",
            "Enchantment",
            "You may play an additional land on each of your turns.",
        ),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
    ]);
    let mut st = state((2..7).collect(), vec![]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    let mut census = TurnCensus::new(1, cards.cards.len());
    play_land_drops(&cards, &mut st, &mut census, 1);
    assert_eq!(census.land_drops, [4]);
    assert_eq!(st.hand.len(), 1);
    st.battlefield.retain(|source| source.uid != 1);
    play_late_land(&cards, &mut st, &mut census, 1);
    assert_eq!(census.land_drops, [4]);
    assert_eq!(st.hand.len(), 1);
}
use crate::deck::simulator::game::{CardRef, new_perm_with};
use crate::deck::simulator::game_effects::resolve_sacrifice_uid;
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::turn_loop_tests::{deck, row, state};

/// Unequal upkeep draw/discard effects register and execute as commander engines.
#[test]
fn commander_unequal_upkeep_draw_registers_and_runs_on_the_next_turn() {
    let mut cards = deck(
        &(0..6)
            .map(|index| row(&format!("Library {index}"), "{100}", "Sorcery", ""))
            .collect::<Vec<_>>(),
    );
    cards.commanders = deck(&[row(
        "Commander",
        "{0}",
        "Legendary Creature",
        "At the beginning of your upkeep, draw three cards, then discard a card.",
    )])
    .cards;
    assert!(cards.commanders[0].unlocked_abilities(0).any(|ability| {
        ability.effect_sequence().iter().any(|effect| {
            matches!(
                effect,
                SimEffect::DrawDiscard {
                    draw: 3,
                    discard: 1
                }
            )
        })
    }));
    let profile = CommanderProfile::new(&cards);
    assert_eq!(profile.has_other_upkeep_effect, [true]);
    let mut st = state(vec![], (0..6).collect());
    let mut census = TurnCensus::with_commander_count(2, cards.cards.len(), 1);
    let mut sources = vec![];
    run_turn(
        &cards,
        &mut st,
        &mut census,
        &profile,
        &mut sources,
        1,
        false,
    );
    assert_eq!(sources.len(), 1);
    assert_eq!(st.hand.len(), 0);
    run_turn(
        &cards,
        &mut st,
        &mut census,
        &profile,
        &mut sources,
        2,
        false,
    );
    assert_eq!(st.hand.len(), 3);
    assert_eq!(st.graveyard.len(), 1);
    assert_eq!(st.library.len(), 2);
    assert_eq!(st.seen, 4);
    assert!(st.completion.is_none());
}

/// Object limits retain partial state and explain exactly what remains.
#[test]
fn token_effect_cap_requires_uncreated_objects() {
    for count in [8, 9] {
        let cards = deck(&[]);
        let mut st = state(vec![], vec![]);
        crate::deck::simulator::game_effects::apply_effect_at(
            &cards,
            &SimEffect::Tokens(count),
            &mut st,
            1,
            false,
            None,
        );
        assert_eq!(st.battlefield.len(), 8);
        assert_eq!(st.completion.is_some(), count > 8);
        if count > 8 {
            assert!(matches!(
                st.completion,
                Some(crate::deck::simulator::game::GameCompletion::Incomplete {
                    reason: crate::deck::simulator::game::IncompleteReason::EffectWork {
                        configured_limit: 8,
                        remaining_work: 1
                    },
                    ..
                })
            ));
        }
    }
}

/// All linked upkeep effects finish before checking zero life.
#[test]
fn linked_upkeep_life_effects_use_one_resolution_boundary() {
    let mut cards = deck(&[row(
        "Upkeep source",
        "{1}",
        "Artifact",
        "At the beginning of your upkeep, gain 1 life.",
    )]);
    let ability = &mut cards.cards[0].striations[0].abilities[0];
    ability.effects = vec![
        SimEffect::Damage {
            amount: 1,
            scope: crate::deck::simulator::model::LifeLossScope::EachPlayer,
        },
        SimEffect::GainLife(3),
    ];
    let mut st = state(vec![], vec![]);
    st.life = 1;
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    run_upkeep(&cards, &mut st, &mut vec![(1, 0)], 1);
    assert_eq!(st.life, 3);
    assert!(st.completion.is_none());
}

/// A scaled draw executes all nine observed matching objects, without an eight-card ceiling.
#[test]
fn scaled_draws_above_eight_are_not_silently_truncated() {
    let mut rows = vec![row("Scaled source", "{1}", "Artifact", "")];
    rows.extend((0..20).map(|index| row(&format!("Spell {index}"), "{20}", "Sorcery", "")));
    let mut cards = deck(&rows);
    cards.cards[0].draws_per_matching = Some(crate::deck::simulator::model::DrawMatch::Artifacts);
    let mut st = state(vec![], (1..21).collect());
    for uid in 1..=9 {
        st.battlefield
            .push(new_perm_with(uid, &cards, CardIdx(0), 0, false));
    }
    run_scaling_draw(&cards, &mut st, Some(CardIdx(0)), 1, 1);
    assert_eq!(st.hand.len(), 9);
    assert_eq!(st.library.len(), 11);
    assert!(st.completion.is_none());
}

/// A commander cannot enter, fire entry rewards, or cast its partner after a last-life cost.
#[test]
fn last_life_commander_payment_stops_before_entry_and_partner_casts() {
    let mut cards = deck(&[]);
    cards.commanders = deck(&[
        row(
            "Life commander",
            "{B/P}",
            "Legendary Creature",
            "When this creature enters, gain 3 life.",
        ),
        row("Partner", "{0}", "Legendary Creature", ""),
    ])
    .cards;
    let mut st = state(vec![], vec![]);
    st.life = 2;
    st.commander_casts = vec![0, 0];
    let mut census = TurnCensus::with_commander_count(1, 0, 2);
    commander_phase(
        &cards,
        &mut st,
        &mut ManaPool::default(),
        &mut census,
        &CommanderProfile::new(&cards),
        1,
        &mut vec![],
        true,
    );
    assert_eq!(st.life, 0);
    assert_eq!(st.life_paid, 2);
    assert_eq!(st.life_gained, 0);
    assert_eq!(st.commander_casts, [1, 0]);
    assert!(st.battlefield.is_empty());
    assert!(matches!(
        st.completion,
        Some(crate::deck::simulator::game::GameCompletion::PlayerLost { .. })
    ));
}

/// A plotted spell resolves all its normal effects and counts as a cast.
#[test]
fn plotted_spell_uses_normal_cast_effects() {
    let cards = deck(&[
        row(
            "Plotted spell",
            "{7}",
            "Sorcery",
            "Draw a card. Gain three life. Add {U}{U}.",
        ),
        row("Drawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![], vec![1]);
    st.plotted.push((CardIdx(0), 1));
    let mut pool = ManaPool::default();
    let mut spent = [0.0; 2];

    cast_plotted_cards(&cards, &mut st, &mut pool, 2, &mut spent, &mut Vec::new());

    assert_eq!(st.hand, [CardIdx(1)]);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert!(st.plotted.is_empty());
    assert_eq!(st.life, 23);
    assert_eq!(pool.total(), 2);
    assert_eq!(spent, [0.0, 0.0]);
    assert_eq!(st.spells_cast_this_turn, 1);
}

/// Plot waives only the mana cost, not a required creature sacrifice.
#[test]
fn plotted_spell_waits_for_its_additional_cost() {
    let cards = deck(&[row(
        "Plotted spell",
        "{7}",
        "Sorcery",
        "As an additional cost to cast this spell, sacrifice a creature. Draw two cards.",
    )]);
    let mut st = state(vec![], vec![]);
    st.plotted.push((CardIdx(0), 1));

    cast_plotted_cards(
        &cards,
        &mut st,
        &mut ManaPool::default(),
        2,
        &mut [0.0; 2],
        &mut Vec::new(),
    );

    assert_eq!(st.plotted, [(CardIdx(0), 1)]);
    assert!(st.graveyard.is_empty());
    assert_eq!(st.spells_cast_this_turn, 0);
}

/// A free plotted planeswalker still enters with printed loyalty.
#[test]
fn plotted_planeswalker_keeps_its_starting_loyalty() {
    let mut walker = row(
        "Walker",
        "{7}",
        "Planeswalker",
        "+1: Create a 2/2 creature token.",
    );
    walker.loyalty = Some("5".into());
    let cards = deck(&[walker]);
    let mut st = state(vec![], vec![]);
    st.plotted.push((CardIdx(0), 1));

    cast_plotted_cards(
        &cards,
        &mut st,
        &mut ManaPool::default(),
        2,
        &mut [0.0; 2],
        &mut Vec::new(),
    );

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].counters.loyalty, 5);
}

/// Command-zone cast counts survive the commander's departure.
#[test]
fn commander_recasts_pay_increasing_tax() {
    let mut cards = deck(&[]);
    cards.commanders = deck(&[row("Commander", "{0}", "Legendary Creature", "")]).cards;
    let profile = CommanderProfile::new(&cards);
    let mut st = state(vec![], vec![]);
    let mut census = TurnCensus::new(3, 0);
    let mut pool = ManaPool {
        colorless: 2,
        ..ManaPool::default()
    };
    let mut repeatable = Vec::new();

    commander_phase(
        &cards,
        &mut st,
        &mut pool,
        &mut census,
        &profile,
        1,
        &mut repeatable,
        true,
    );
    assert_eq!(pool.total(), 2);
    assert_eq!(st.commander_casts, [1]);
    let uid = st.battlefield[0].uid;
    resolve_sacrifice_uid(&cards, &mut st, 1, uid);
    commander_phase(
        &cards,
        &mut st,
        &mut pool,
        &mut census,
        &profile,
        2,
        &mut repeatable,
        true,
    );
    assert_eq!(pool.total(), 0);
    assert_eq!(st.commander_casts, [2]);
    assert_eq!(census.mana_spent, [0.0, 2.0, 0.0]);
    let uid = st.battlefield[0].uid;
    resolve_sacrifice_uid(&cards, &mut st, 2, uid);
    pool.colorless = 3;
    commander_phase(
        &cards,
        &mut st,
        &mut pool,
        &mut census,
        &profile,
        3,
        &mut repeatable,
        true,
    );
    assert!(st.battlefield.is_empty());
    assert_eq!(st.commander_casts, [2]);
    assert_eq!(pool.total(), 3);
}

/// A tapped animated permanent cannot be declared as an attacker.
#[test]
fn tapped_animated_permanent_does_not_attack() {
    let cards = deck(&[row("Vehicle", "{3}", "Artifact — Vehicle", "Crew 1")]);
    let mut st = state(vec![], vec![]);
    let mut vehicle = new_perm_with(1, &cards, CardIdx(0), 0, true);
    vehicle.animated = true;
    st.battlefield.push(vehicle);

    let combat = crate::deck::simulator::game_combat::combat_phase(&cards, &mut st, 1, &[0]);

    assert_eq!(combat.attackers, 0);
    assert_eq!(combat.power, 0);
}

/// Ordinary attackers tap while vigilance leaves an attacker untapped.
#[test]
fn attacking_taps_only_creatures_without_vigilance() {
    let cards = deck(&[
        row("Ordinary", "{1}", "Creature", ""),
        row("Vigilant", "{1}", "Creature", "Vigilance"),
    ]);
    let mut st = state(vec![], vec![]);
    for index in 0..2 {
        let mut permanent = new_perm_with(index + 1, &cards, CardIdx(index), 0, false);
        permanent.summoning_sick = false;
        st.battlefield.push(permanent);
    }

    let combat = crate::deck::simulator::game_combat::combat_phase(&cards, &mut st, 1, &[0]);

    assert_eq!(combat.attackers, 2);
    assert!(st.battlefield[0].tapped);
    assert!(!st.battlefield[1].tapped);
    assert_eq!(st.battlefield[1].card, CardRef::Deck(CardIdx(1)));
}

/// Build one source's output with a parsed nonland mana-tap bonus present.
fn mana_with_bonus(output: super::super::model::ManaYield, demand: &str) -> ManaPool {
    let cards = deck(&[
        row("Mana source", "{1}", "Artifact", ""),
        row(
            "Mana bonus",
            "{G}{U}",
            "Creature",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        ),
        row("Planned spell", demand, "Sorcery", ""),
    ]);
    assert!(cards.cards[1].flags.bonus_mana_on_nonland_tap);
    let mut st = state(vec![2], vec![]);
    for index in 0..2 {
        st.battlefield
            .push(new_perm_with(index + 1, &cards, CardIdx(index), 0, false));
    }
    let source = st.battlefield[0].clone();
    let mut pool = ManaPool::default();
    assert!(add_nonland_mana(
        &cards, &st, &source, &output, &mut pool, 2
    ));
    pool
}

/// One chosen source color also fixes the bonus color for that activation.
#[test]
fn mana_bonus_cannot_split_a_source_choice_into_different_colors() {
    use crate::deck::simulator::game_mana::payment_plan;
    use crate::deck::simulator::model::ManaYield;
    use crate::deck::simulator::oracle_parser::cost::parse_cost;
    for output in [
        ManaYield {
            any_pips: 1,
            ..Default::default()
        },
        ManaYield {
            choice: [false, true, false, false, true],
            ..Default::default()
        },
    ] {
        let pool = mana_with_bonus(output, "{U}{G}");
        assert_eq!(pool.total(), 2);
        assert!(payment_plan(&parse_cost("{U}{G}"), &pool, &[]).is_none());
        assert!(payment_plan(&parse_cost("{U}{U}"), &pool, &[]).is_some());
    }
    let pool = mana_with_bonus(
        ManaYield {
            any_pips: 3,
            one_color: true,
            ..Default::default()
        },
        "{U}{U}{U}{G}",
    );
    assert_eq!(pool.total(), 4);
    assert!(payment_plan(&parse_cost("{U}{U}{U}{G}"), &pool, &[]).is_none());
    assert!(payment_plan(&parse_cost("{U}{U}{U}{U}"), &pool, &[]).is_some());
}

/// Restricted source mana and an unrestricted bonus share their chosen type.
#[test]
fn mana_bonus_keeps_source_restrictions_separate() {
    use crate::deck::simulator::game_mana::payment_plan;
    use crate::deck::simulator::model::{ManaYield, SpendRestriction};
    use crate::deck::simulator::oracle_parser::cost::parse_cost;
    let pool = mana_with_bonus(
        ManaYield {
            any_pips: 1,
            restriction: Some(SpendRestriction::Creature),
            ..Default::default()
        },
        "{U}{G}",
    );
    assert!(payment_plan(&parse_cost("{U}{G}"), &pool, &[SpendRestriction::Creature]).is_none());
    assert!(payment_plan(&parse_cost("{U}{U}"), &pool, &[SpendRestriction::Creature]).is_some());
    assert!(payment_plan(&parse_cost("{U}{U}"), &pool, &[]).is_none());
    assert!(payment_plan(&parse_cost("{U}"), &pool, &[]).is_some());
}

/// Generic-only opponent-color credit does not become colored or colorless mana.
#[test]
fn mana_bonus_preserves_generic_only_credit() {
    use crate::deck::simulator::game_mana::payment_plan;
    use crate::deck::simulator::model::ManaYield;
    use crate::deck::simulator::oracle_parser::cost::parse_cost;
    let pool = mana_with_bonus(
        ManaYield {
            any_pips: 1,
            opponent_any: true,
            ..Default::default()
        },
        "{C}",
    );
    assert_eq!(pool.generic_credit, 2);
    assert_eq!(pool.colorless, 0);
    assert!(payment_plan(&parse_cost("{2}"), &pool, &[]).is_some());
    assert!(payment_plan(&parse_cost("{C}"), &pool, &[]).is_none());
    assert!(payment_plan(&parse_cost("{U}"), &pool, &[]).is_none());
}

/// Role access includes a card drawn and cast before the hand is sampled again.
#[test]
fn role_sightings_include_same_turn_drawn_and_cast_cards() {
    let cards = deck(&[
        row("Cantrip", "{0}", "Instant", "Draw a card."),
        row("Drawn creature", "{0}", "Creature", ""),
    ]);
    let mut st = state(vec![0], vec![1]);
    let mut census = TurnCensus::new(1, cards.cards.len());
    let commander = CommanderProfile::new(&cards);

    run_turn(
        &cards,
        &mut st,
        &mut census,
        &commander,
        &mut Vec::new(),
        1,
        false,
    );

    assert!(st.hand.is_empty());
    assert_eq!(st.card_first_seen.get(&1), Some(&1));
    assert_eq!(census.first_seen.get(&cards.cards[1].role), Some(&1));
    assert_eq!(census.first_creature, Some(1));
}

/// The earliest event turn wins even when the card is no longer in hand.
#[test]
fn role_sightings_keep_the_earliest_recorded_event_turn() {
    let cards = deck(&[row("Seen creature", "{0}", "Creature", "")]);
    let mut st = state(vec![], vec![]);
    st.card_first_seen.insert(0, 2);
    let mut census = TurnCensus::new(5, 1);
    census.first_seen.insert(cards.cards[0].role, 4);

    census::record_hand_sightings(&cards, &st, &mut census, 5);

    assert_eq!(census.first_seen.get(&cards.cards[0].role), Some(&2));
    assert_eq!(census.first_creature, Some(2));
}

/// Activity counters come from actual rounds, excluding the final horizon.
#[test]
fn game_log_records_only_executed_opponent_rounds() {
    use crate::deck::simulator::model::Format;
    use crate::deck::simulator::opponents::{OpponentActivity, OpponentConfig};
    use rand::SeedableRng;
    let mut cards = deck(&vec![
        row(
            "Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}."
        );
        24
    ]);
    cards.format = Format::Commander;
    cards.rules = crate::deck::simulator::format::rules_for("commander");
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let one_turn = run_game(&cards, &mut rng, 1);
    assert_eq!(one_turn.opponent_activity, OpponentActivity::default());
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let active = run_game(&cards, &mut rng, 3);
    assert_eq!(active.opponent_activity.draws, 6);
    assert_eq!(active.opponent_activity.casts, 3);
    let config = OpponentConfig {
        active: false,
        ..Default::default()
    };
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let inactive = run_game_with_config(
        &cards,
        &mut rng,
        3,
        Some(&config),
        crate::deck::simulator::rng::game_rng(
            1,
            0,
            crate::deck::simulator::rng::RngDomain::Library,
        ),
    );
    assert_eq!(inactive.opponent_activity, OpponentActivity::default());
    assert_eq!(active.cards_seen, inactive.cards_seen);
}

/// An extra turn starts without an intervening synthetic opponent round.
#[test]
fn extra_turns_do_not_generate_opponent_activity() {
    use crate::deck::simulator::model::Format;
    use crate::deck::simulator::opponents::OpponentActivity;
    use rand::SeedableRng;
    let mut cards = deck(&vec![
        row(
            "Extra turn",
            "{0}",
            "Sorcery",
            "Take an extra turn after this one."
        );
        24
    ]);
    cards.format = Format::Commander;
    cards.rules = crate::deck::simulator::format::rules_for("commander");
    let mut rng = ChaCha8Rng::seed_from_u64(1);

    assert!(
        cards.cards[0]
            .spell_data
            .effects
            .iter()
            .any(|effect| matches!(effect, SimEffect::ExtraTurn)),
        "effects={:?}, role={:?}",
        cards.cards[0].spell_data.effects,
        cards.cards[0].role
    );
    let log = run_game(&cards, &mut rng, 3);

    assert_eq!(log.extra_turns, [0, 1, 1]);
    assert_eq!(log.opponent_activity, OpponentActivity::default());
}

/// Attack and combat-damage listeners reject a Goblin when their filter is Elf.
#[test]
fn combat_event_filters_accept_elf_and_reject_goblin() {
    for (text, trigger) in [
        (
            "Whenever an Elf you control attacks, draw a card.",
            SimTrigger::Attacks,
        ),
        (
            "Whenever an Elf you control deals combat damage to a player, draw a card.",
            SimTrigger::CombatDamage,
        ),
    ] {
        let cards = deck(&[
            row("Elf listener", "{1}", "Enchantment", text),
            row("Elf", "{1}", "Creature — Elf", ""),
            row("Goblin", "{1}", "Creature — Goblin", ""),
            row("Remaining", "{9}", "Sorcery", ""),
            row("Drawn", "{9}", "Sorcery", ""),
        ]);
        assert!(
            cards.cards[0]
                .unlocked_abilities(0)
                .any(|ability| ability.trigger == trigger)
        );
        let mut st = state(vec![], vec![3, 4]);
        for index in 0..3 {
            let mut permanent = new_perm_with(index + 1, &cards, CardIdx(index), 0, false);
            permanent.summoning_sick = false;
            st.battlefield.push(permanent);
        }

        let combat = crate::deck::simulator::game_combat::combat_phase(&cards, &mut st, 1, &[0]);

        assert_eq!(combat.attackers, 2);
        assert_eq!(st.hand, [CardIdx(4)], "{text}");
        assert_eq!(st.library, [CardIdx(3)], "{text}");
        assert_eq!(st.seen, 1);
    }
}

/// Pip-only mana can fund interaction without underflowing spare-mana metrics.
#[test]
fn interaction_readiness_handles_mana_that_cannot_pay_generic_costs() {
    let cards = deck(&[row(
        "Counterspell",
        "{U}{U}",
        "Instant",
        "Counter target spell.",
    )]);
    let st = state(vec![0], vec![]);
    let pool = ManaPool {
        fixed: [0, 2, 0, 0, 0],
        fixed_no_generic: [0, 2, 0, 0, 0],
        ..Default::default()
    };
    let mut census = TurnCensus::new(1, 1);

    census::record_interaction_readiness(&cards, &st, &pool, &mut census, 1);

    assert_eq!(census.interaction_ready, [true]);
    assert_eq!(census.interaction_mana_held, [0.0]);
    assert_eq!(pool.total(), 2);
}

/// Missing commander data must not become a free command-zone cast.
#[test]
fn missing_commander_data_does_not_cast_as_a_zero_cost_card() {
    let mut cards = deck(&[]);
    cards
        .commanders
        .push(crate::deck::simulator::model::SimCard::default());
    let profile = CommanderProfile::new(&cards);
    let mut st = state(vec![], vec![]);
    let mut census = TurnCensus::new(1, 0);
    let mut pool = ManaPool {
        colorless: 2,
        ..Default::default()
    };

    commander_phase(
        &cards,
        &mut st,
        &mut pool,
        &mut census,
        &profile,
        1,
        &mut Vec::new(),
        true,
    );

    assert!(st.battlefield.is_empty());
    assert_eq!(st.commander_casts, [0]);
    assert_eq!(st.spells_cast_this_turn, 0);
    assert_eq!(pool.total(), 2);
    assert_eq!(census.commander_castable, None);
}
