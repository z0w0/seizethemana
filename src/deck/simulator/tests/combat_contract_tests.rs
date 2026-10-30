use super::*;
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::turn_loop_tests::{deck, row, state};
use crate::deck::simulator::{aggregate, game, game_run, report};

/// Build a commander scenario with enough physical library cards for full turns.
fn scenario(powers: &[u32], text: &str) -> (SimDeck, GameState) {
    let mut cards = deck(
        &(0..20)
            .map(|index| row(&format!("Library {index}"), "{20}", "Sorcery", ""))
            .collect::<Vec<_>>(),
    );
    cards.format = crate::deck::simulator::model::Format::Commander;
    cards.rules = crate::deck::simulator::format::rules_for("commander");
    cards.commanders = powers
        .iter()
        .enumerate()
        .map(|(slot, power)| {
            let mut source = row(
                &format!("Commander {slot}"),
                "{20}",
                "Legendary Creature",
                text,
            );
            source.power = Some(power.to_string());
            source.toughness = Some("5".into());
            crate::deck::simulator::oracle_lower::parse_sim_card(&source)
        })
        .collect();
    let mut st = state(vec![], (0..20).collect());
    st.opponents = crate::deck::simulator::opponents::OpponentTable::for_rules(cards.rules);
    st.commander_casts = vec![1; powers.len()];
    st.next_uid = 100;
    for slot in 0..powers.len() {
        st.battlefield.push(game::new_commander_perm(
            slot as u32 + 1,
            slot,
            0,
            0,
            true,
            false,
        ));
        st.battlefield[slot].summoning_sick = false;
    }
    (cards, st)
}

/// Attack-trigger counters affect this combat, not only the next combat.
#[test]
fn commander_power_is_measured_after_attack_rewards() {
    let (cards, mut st) = scenario(&[4], "Whenever this creature attacks, proliferate.");
    st.battlefield[0].counters.plus1 = 1;
    let outcome = combat_phase(&cards, &mut st, 1, &[0]);
    assert_eq!(st.battlefield[0].counters.plus1, 2);
    assert_eq!(outcome.commander_power, [6]);
    assert_eq!(outcome.power, 6);
    assert_eq!(
        st.opponents
            .seats
            .iter()
            .map(|seat| seat.life)
            .collect::<Vec<_>>(),
        [40; 3]
    );
}

/// Dead seats permit a capacity measurement but cannot supply attack events.
#[test]
fn no_live_defender_has_no_attack_or_damage_side_effects() {
    let (cards, mut st) = scenario(
        &[4],
        "Lifelink\nWhenever this creature attacks, draw a card.\nWhenever this creature deals combat damage to a player, draw a card.",
    );
    for seat in &mut st.opponents.seats {
        seat.active = false;
    }
    st.ring_bearer = Some(1);
    st.ring_tempts = 4;
    let hand = st.hand.clone();
    let library = st.library.clone();
    let life = st.life;
    let outcome = combat_phase(&cards, &mut st, 1, &[0]);
    assert_eq!(outcome.commander_power, [4]);
    assert_eq!(outcome.power, 4);
    assert_eq!(st.hand, hand);
    assert_eq!(st.library, library);
    assert_eq!(st.life, life);
    assert!(!st.battlefield[0].tapped);
    assert!(!st.attacked_this_turn);
    assert_eq!(st.opponent_life_lost, 0);
}

/// Readiness uses sickness, haste, animation, and the current counter total.
#[test]
fn commander_readiness_and_negative_power_are_not_assumed() {
    let (mut cards, mut st) = scenario(&[4], "");
    st.battlefield[0].entered_turn = 1;
    st.battlefield[0].summoning_sick = true;
    assert_eq!(combat_phase(&cards, &mut st, 1, &[0]).power, 0);
    cards.commanders[0].printed_keywords.insert(Keyword::Haste);
    st.battlefield[0].counters.minus1 = 6;
    assert_eq!(combat_phase(&cards, &mut st, 1, &[0]).power, 0);
    st.battlefield[0].tapped = false;
    st.battlefield[0].counters.plus1 = 3;
    cards.commanders[0]
        .printed_keywords
        .insert(Keyword::DoubleStrike);
    assert_eq!(combat_phase(&cards, &mut st, 1, &[0]).commander_power, [2]);
}

/// Tokens with the same source definition never gain commander identity.
#[test]
fn partners_and_attack_tokens_keep_separate_identity() {
    let (cards, mut st) = scenario(&[10, 11], "Mobilize 2");
    let outcome = combat_phase(&cards, &mut st, 1, &[0]);
    assert_eq!(outcome.commander_power, [10, 11]);
    assert_eq!(outcome.attackers, 6);
    assert_eq!(outcome.power, 29);
    assert!(
        st.opponents
            .seats
            .iter()
            .all(|seat| seat.active && seat.life == 40)
    );
}

/// Partner contributions cannot combine to cross one commander's threshold.
#[test]
fn partner_threshold_rates_stay_separate_through_reporting() {
    let (cards, mut st) = scenario(&[10, 11], "");
    let mut census = game_run::census::TurnCensus::with_commander_count(1, cards.cards.len(), 2);
    game_run::run_turn(
        &cards,
        &mut st,
        &mut census,
        &CommanderProfile::new(&cards),
        &mut vec![],
        1,
        false,
    );
    let log = game_run::census::finish_log(&cards, st, census, 0, false, 1);
    let stats = aggregate::aggregate(&[log], &cards, 1);
    let payload = report::json_report(
        &stats,
        &cards,
        "partners",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert_eq!(payload.combat.attack_power_avg_by_turn[&1], 21.0);
    assert_eq!(payload.combat.commander_potential.len(), 2);
    for (slot, damage) in [10.0, 11.0].into_iter().enumerate() {
        let commander = &payload.combat.commander_potential[slot];
        assert_eq!(commander.slot, slot as u32);
        assert_eq!(commander.damage_by_turn[&1], damage);
        assert_eq!(commander.games_reaching_21, 0);
        assert_eq!(
            commander
                .percent_games_reaching_21
                .expect("finished denominator")
                .value(),
            0.0
        );
        assert!(commander.median_turn_to_21.is_none());
    }
}

/// Noncreature commanders contribute only while a supported animation makes them creatures.
#[test]
fn vehicle_commander_requires_supported_animation() {
    let (mut cards, mut st) = scenario(&[7], "");
    cards.commanders[0].is_creature = false;
    cards.commanders[0].crew = Some(3);
    assert_eq!(combat_phase(&cards, &mut st, 1, &[0]).commander_power, [0]);
    st.battlefield[0].crewed = true;
    assert_eq!(combat_phase(&cards, &mut st, 1, &[0]).commander_power, [7]);
}

/// Executed full turns feed the commander threshold through production reporting.
#[test]
fn commander_threshold_boundaries_and_recasts_reach_the_report() {
    for power in [20, 21, 22] {
        let (cards, mut st) = scenario(&[power], "");
        let mut census =
            game_run::census::TurnCensus::with_commander_count(1, cards.cards.len(), 1);
        game_run::run_turn(
            &cards,
            &mut st,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut vec![],
            1,
            false,
        );
        let log = game_run::census::finish_log(&cards, st, census, 0, false, 1);
        let stats = aggregate::aggregate(&[log], &cards, 1);
        let report = report::json_report(
            &stats,
            &cards,
            "boundary",
            1,
            &[],
            Default::default(),
            &Default::default(),
        );
        let row = &report.combat.commander_potential[0];
        assert_eq!(row.damage_by_turn[&1], f64::from(power));
        assert_eq!(row.games_reaching_21, u32::from(power >= 21));
        assert_eq!(row.median_turn_to_21, (power >= 21).then_some(1));
        assert_eq!(report.combat.attack_power_avg_by_turn[&1], f64::from(power));
        let json = serde_json::to_value(&report).expect("serialize current report");
        assert_eq!(
            serde_json::from_value::<crate::deck::simulator::report_schema::SimReport>(json)
                .expect("read current report"),
            report
        );
    }
    let (mut cards, mut st) = scenario(&[7], "Haste");
    cards.commanders[0].cost = crate::deck::simulator::model::Cost::default();
    cards.commanders[0].min_cost = cards.commanders[0].cost;
    for index in 0..2 {
        cards.cards[index] = crate::deck::simulator::oracle_lower::parse_sim_card(&row(
            "Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        ));
        st.battlefield.push(game::new_perm_with(
            80 + index as u32,
            &cards,
            CardIdx(index as u32),
            0,
            false,
        ));
    }
    st.library.retain(|index| index.index() >= 2);
    let mut census = game_run::census::TurnCensus::with_commander_count(3, cards.cards.len(), 1);
    let mut sources = vec![];
    game_run::run_turn(
        &cards,
        &mut st,
        &mut census,
        &CommanderProfile::new(&cards),
        &mut sources,
        1,
        false,
    );
    let old_uid = st.battlefield[0].uid;
    crate::deck::simulator::game_effects::resolve_sacrifice_uid(&cards, &mut st, 1, old_uid);
    for turn in 2..=3 {
        game_run::run_turn(
            &cards,
            &mut st,
            &mut census,
            &CommanderProfile::new(&cards),
            &mut sources,
            turn,
            false,
        );
    }
    let returned = st
        .battlefield
        .iter()
        .find(|perm| matches!(perm.card, CardRef::Commander { slot: 0 }))
        .expect("commander recast with tax");
    assert_ne!(returned.uid, old_uid);
    assert_eq!(st.commander_casts, [2]);
    let log = game_run::census::finish_log(&cards, st, census, 0, false, 3);
    let stats = aggregate::aggregate(&[log], &cards, 3);
    assert_eq!(stats.commander_combat_threshold_games, [1]);
    assert_eq!(stats.commander_combat_threshold_p50, [Some(3)]);
    assert_eq!(
        stats.commander_combat_cumulative_by_turn[0],
        [7.0, 14.0, 21.0]
    );
}

/// Equipment affects only its host and noncombat damage does not become commander potential.
#[test]
fn equipment_and_noncombat_output_have_separate_totals() {
    let (mut cards, mut st) = scenario(
        &[3],
        "Whenever this creature attacks, it deals 2 damage to each opponent.",
    );
    let gear = row(
        "Equipment",
        "{1}",
        "Artifact — Equipment",
        "Equipped creature gets +2/+0.\nEquip {1}",
    );
    cards
        .cards
        .push(crate::deck::simulator::oracle_lower::parse_sim_card(&gear));
    let perm = game::new_perm_with(50, &cards, CardIdx(20), 0, false);
    st.battlefield.push(perm);
    let mut pool = game::ManaPool {
        colorless: 1,
        ..Default::default()
    };
    crate::deck::simulator::game_effects::tap_budget(
        &cards,
        &mut st.battlefield,
        &mut pool,
        &st.hand,
    );
    assert_eq!(st.battlefield[1].equip_host, Some(1));
    assert!(st.battlefield[1].equipped);
    assert_eq!(pool.total(), 0);
    let outcome = combat_phase(&cards, &mut st, 1, &[0]);
    assert_eq!(outcome.power, 5);
    assert_eq!(outcome.commander_power, [5]);
    assert_eq!(st.damage_dealt_this_turn, 6);
    assert!(st.opponents.seats.iter().all(|seat| seat.life == 38));
}

/// First-strike rewards change the second strike's power and lifelink amount.
#[test]
fn double_strike_recomputes_power_between_damage_steps() {
    let (cards, mut st) = scenario(
        &[9],
        "Double strike\nLifelink\nWhenever this creature deals combat damage to a player, proliferate.",
    );
    st.battlefield[0].counters.plus1 = 1;
    let initial_life = st.life;
    let mut census = game_run::census::TurnCensus::with_commander_count(1, cards.cards.len(), 1);
    game_run::run_turn(
        &cards,
        &mut st,
        &mut census,
        &CommanderProfile::new(&cards),
        &mut vec![],
        1,
        false,
    );
    assert_eq!(census.commander_combat_damage[0], [21]);
    assert_eq!(census.attack_power, [21]);
    assert_eq!(st.battlefield[0].counters.plus1, 3);
    assert_eq!(st.life_gained, 21);
    assert_eq!(st.life, initial_life + 21);
    assert!(st.opponents.seats.iter().all(|seat| seat.life == 40));
    let log = game_run::census::finish_log(&cards, st, census, 0, false, 1);
    let stats = aggregate::aggregate(&[log], &cards, 1);
    let payload = report::json_report(
        &stats,
        &cards,
        "double strike",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert_eq!(payload.combat.commander_potential[0].games_reaching_21, 1);
    assert_eq!(
        payload.combat.commander_potential[0].median_turn_to_21,
        Some(1)
    );
    assert_eq!(payload.combat.attack_power_avg_by_turn[&1], 21.0);
}

/// Capacity-only double strike cannot gain a counter or life without a defender.
#[test]
fn double_strike_capacity_does_not_resolve_damage_rewards() {
    let (cards, mut st) = scenario(
        &[9],
        "Double strike\nLifelink\nWhenever this creature deals combat damage to a player, proliferate.",
    );
    st.battlefield[0].counters.plus1 = 1;
    for seat in &mut st.opponents.seats {
        seat.active = false;
    }
    let result = combat_phase(&cards, &mut st, 1, &[0]);
    assert_eq!(result.power, 20);
    assert_eq!(result.commander_power, [20]);
    assert_eq!(st.battlefield[0].counters.plus1, 1);
    assert_eq!(st.life_gained, 0);
    assert!(!st.battlefield[0].tapped);
}

/// A completed hit remains a known threshold event when its reward ends combat with a loss.
#[test]
fn commander_threshold_survives_a_later_combat_trigger_loss() {
    let (cards, mut st) = scenario(
        &[21],
        "Whenever this creature deals combat damage to a player, draw a card.",
    );
    st.library = vec![CardIdx(0)];
    let mut census = game_run::census::TurnCensus::with_commander_count(1, cards.cards.len(), 1);
    game_run::run_turn(
        &cards,
        &mut st,
        &mut census,
        &CommanderProfile::new(&cards),
        &mut vec![],
        1,
        false,
    );
    assert!(matches!(
        st.completion,
        Some(game::GameCompletion::PlayerLost { .. })
    ));
    assert_eq!(census.commander_combat_damage[0], [21]);
    assert!(!census.turn_observations[0].combat);
    let log = game_run::census::finish_log(&cards, st, census, 0, false, 1);
    let stats = aggregate::aggregate(&[log], &cards, 1);
    assert_eq!(stats.commander_combat_threshold_games, [1]);
    assert_eq!(stats.commander_combat_threshold_p50, [Some(1)]);
    let payload = report::json_report(
        &stats,
        &cards,
        "combat loss",
        1,
        &[],
        Default::default(),
        &Default::default(),
    );
    assert_eq!(payload.outcomes.player_loss_games, 1);
    let commander = &payload.combat.commander_potential[0];
    assert_eq!(
        commander
            .percent_games_reaching_21
            .expect("finished loss denominator")
            .value(),
        100.0
    );
    assert_eq!(commander.games_reaching_21, 1);
    assert_eq!(commander.median_turn_to_21, Some(1));
    assert!(commander.damage_by_turn.is_empty());
    assert!(commander.cumulative_damage_by_turn.is_empty());
    assert_eq!(
        payload.sampling["combat.commander_potential[0].percent_games_reaching_21"].event_games,
        Some(1)
    );
}
