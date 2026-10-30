//! Access and land controls. Companion and readiness near misses are synthetic.

use super::*;
use crate::deck::simulator::cast_pass::play_land;
use crate::deck::simulator::deal::deal_opener;
use crate::deck::simulator::deck::build_sim_deck;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Use the real bench selector without inventing a pinned companion source.
fn companion_deck() -> SimDeck {
    let mut first = body(
        "First synthetic companion",
        "{99}",
        "Companion — Your starting deck contains only cards with even mana values.",
        2,
    );
    first.keywords = "[\"Companion\"]".into();
    let mut second = first.clone();
    second.name = "Second synthetic companion".into();
    let sources = [
        row("Library marker", "{99}", "Sorcery", ""),
        row("Plain bench card", "{0}", "Artifact", ""),
        first,
        second,
    ];
    let list = crate::deck::grammar::Deck::parse(
        "// DECK\n8 Library marker\n// SIDEBOARD\n1 Plain bench card\n1 First synthetic companion\n1 Second synthetic companion\n",
    )
    .expect("synthetic bench deck parses");
    let metadata = sources
        .into_iter()
        .map(|card| (card.name.clone(), card))
        .collect();
    build_sim_deck(&list, &metadata, Some("constructed"))
}

/// The first keyword-bearing bench card stays outside opening zones and costs {3} once.
pub(super) fn assert_companion_access_report() {
    let cards = companion_deck();
    let companion = cards.companion.expect("bench selector found the companion");
    assert_eq!(companion, CardIdx(8));
    assert_eq!(cards.library_len(), 8);
    assert_eq!(cards.cards.len(), 9);
    assert_eq!(cards[companion].name, "First synthetic companion");
    assert!(
        cards
            .cards
            .iter()
            .all(|card| card.name != "Second synthetic companion")
    );
    let mut opening_rng = ChaCha8Rng::seed_from_u64(9);
    for _ in 0..8 {
        let opener = deal_opener(&cards, &mut opening_rng);
        assert!(!opener.hand.contains(&companion));
        assert!(!opener.library.contains(&companion));
        assert_eq!(opener.hand.len() + opener.library.len(), 8);
    }
    for available in [2, 3, 4] {
        let mut cards = companion_deck();
        let source_index = cards.cards.len() as u32;
        cards.cards.extend(
            deck(&[row(
                "Companion payment source",
                "{99}",
                "Artifact",
                &format!("{{T}}: Add {}.", "{C}".repeat(available)),
            )])
            .cards,
        );
        let mut game = state(vec![], (0..8).collect());
        put_permanents(&cards, &mut game, &[source_index]);
        let random = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 6, vec![]);
        let fetched = available >= 3;
        assert_eq!(game.companion_fetched, fetched);
        assert_eq!(
            game.hand
                .iter()
                .filter(|index| **index == companion)
                .count(),
            usize::from(fetched)
        );
        assert_eq!(
            game.card_first_seen.get(&companion.index()),
            fetched.then_some(&1)
        );
        assert!(!game.library.contains(&companion));
        assert!(!game.graveyard.contains(&companion) && !game.exile.contains(&companion));
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(
            game.battlefield[0].card,
            CardRef::Deck(CardIdx(source_index))
        );
        assert!(game.battlefield[0].tapped);
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert_eq!(game.current_phase, GamePhase::EndStep);
        assert_library_rng(&game.library_rng, &random);
        assert_eq!(log.companion_online, fetched.then_some(1));
        assert_eq!(log.mana_spent, [0.0; 6]);
        assert_eq!(
            log.mana_remaining[0],
            (available - usize::from(fetched) * 3) as f64
        );
        assert_eq!(log.mana_remaining[1..], [available as f64; 5]);
        let report = report_log(&cards, &log, 6);
        assert_finished_turn(&report, &log);
        let access = report
            .companion
            .as_ref()
            .expect("companion has an access report");
        assert_eq!(access.in_hand_games, u32::from(fetched));
        assert_eq!(access.p50_online_turn, fetched.then_some(1));
        assert_eq!(
            access.in_hand_by_t6.map(f64::from),
            Some(if fetched { 100.0 } else { 0.0 })
        );
        let estimate = &report.sampling["companion.in_hand_by_t6"];
        assert_eq!(estimate.observed_games, 1);
        assert_eq!(estimate.event_games, Some(u32::from(fetched)));
        assert_assumption(&report, "Its deck condition is assumed legal");
        assert_eq!(report.mana.avg_unused_by_turn[&1], log.mana_remaining[0]);
        assert_eq!(report.mana.avg_unused_by_turn[&6], available as f64);
    }
}

/// Run payable, unpayable, wrong-speed, discarded, and restricted-mana answer controls.
pub(super) fn assert_interaction_readiness_report() {
    let answer = named_row("Counterspell");
    pinned_revision(&answer);
    for (label, mana, speed, discard, ready, held, unused) in [
        (
            "payable",
            "{T}: Add {U}{U}{U}.",
            true,
            false,
            true,
            1.0,
            3.0,
        ),
        ("short mana", "{T}: Add {U}.", true, false, false, 0.0, 1.0),
        (
            "wrong color",
            "{T}: Add {G}{G}{G}.",
            true,
            false,
            false,
            0.0,
            3.0,
        ),
        (
            "wrong speed",
            "{T}: Add {U}{U}{U}.",
            false,
            false,
            false,
            0.0,
            3.0,
        ),
        (
            "no longer held",
            "{T}: Add {U}{U}{U}.",
            true,
            true,
            false,
            0.0,
            3.0,
        ),
        (
            "creature restricted",
            "{T}: Add {U}{U}{U}. Spend this mana only to cast creature spells.",
            true,
            false,
            false,
            0.0,
            3.0,
        ),
        (
            "instant restricted",
            "{T}: Add {U}{U}{U}. Spend this mana only to cast instant or sorcery spells.",
            true,
            false,
            true,
            1.0,
            3.0,
        ),
    ] {
        let source = if speed {
            answer.clone()
        } else {
            row(
                "Synthetic sorcery answer",
                "{U}{U}",
                "Sorcery",
                "Counter target spell.",
            )
        };
        let cards = deck(&[
            source,
            row("Readiness mana", "{99}", "Artifact", mana),
            row(
                "Discard action",
                "{0}",
                "Sorcery",
                "As an additional cost to cast this spell, discard a card.\nGain 1 life.",
            ),
            row("Library remainder", "{99}", "Sorcery", ""),
        ]);
        assert!(cards.cards[0].flags.is_interaction, "{label}");
        assert_eq!(cards.cards[0].flags.is_instant_speed, speed, "{label}");
        assert_readiness_turn(&cards, label, discard, ready, held, unused);
    }
}

/// Keep readiness tied to the hand and payment state after all real main actions.
fn assert_readiness_turn(
    cards: &SimDeck,
    label: &str,
    discard: bool,
    ready: bool,
    held: f64,
    unused: f64,
) {
    let mut game = state(if discard { vec![0, 2] } else { vec![0] }, vec![3]);
    put_permanents(cards, &mut game, &[1]);
    let random = game.library_rng.clone();
    let (game, log) = execute_turns(cards, game, 1, vec![]);
    assert_eq!(
        game.hand,
        if discard { vec![] } else { vec![CardIdx(0)] },
        "{label}"
    );
    assert_eq!(
        game.graveyard,
        if discard {
            vec![CardIdx(0), CardIdx(2)]
        } else {
            vec![]
        },
        "{label}"
    );
    assert_eq!(game.library, [CardIdx(3)]);
    assert!(game.exile.is_empty());
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].uid, 1);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.spells_cast_this_turn, u32::from(discard));
    assert_eq!(game.life, 20 + i32::from(discard));
    assert_eq!(game.opponents.seats[0].life, 20);
    assert_eq!(game.opponent_life_lost, 0);
    assert_eq!(game.life_paid, 0);
    assert_library_rng(&game.library_rng, &random);
    assert_eq!(log.interaction_ready, [ready], "{label}");
    assert_eq!(log.interaction_mana_held, [held], "{label}");
    assert_eq!(log.mana_spent, [0.0]);
    assert_eq!(log.mana_remaining, [unused]);
    let report = report_log(cards, &log, 1);
    assert_finished_turn(&report, &log);
    assert_eq!(
        report.interaction.ready_turn_observations,
        usize::from(ready),
        "{label}"
    );
    assert_eq!(
        report.interaction.mana_held_avg,
        ready.then_some(held),
        "{label}"
    );
    assert_eq!(
        report
            .interaction
            .percent_games_with_ready_interaction_by_turn[&1]
            .value(),
        if ready { 100.0 } else { 0.0 },
        "{label}"
    );
    let estimate = &report.sampling["interaction.percent_games_with_ready_interaction_by_turn.1"];
    assert_eq!(estimate.observed_games, 1);
    assert_eq!(estimate.event_games, Some(u32::from(ready)));
    assert_eq!(report.win_conditions.opponent_life_loss_by_turn[&1], 0.0);
    assert_eq!(report.combat.player_damage_avg_by_turn[&1], 0.0);
    assert_eq!(report.mana.avg_unused_by_turn[&1], unused);
    assert_eq!(report.interaction.note, "capacity, not events");
    assert_assumption(
        &report,
        "Readiness requires an instant-speed answer still in hand",
    );
}

/// Unknown predicates and partial combined predicates remain tapped after a real land play.
pub(super) fn assert_land_entry_safety_report() {
    for (text, tapped) in [
        (
            "This land enters tapped unless you control two or fewer other lands.",
            false,
        ),
        (
            "This land enters tapped unless you control two or fewer other lands and an Elf.",
            true,
        ),
        (
            "This land enters tapped unless you control two or fewer other lands or have two or more opponents.",
            true,
        ),
        (
            "This land enters tapped unless you have two or more opponents and control a Forest.",
            true,
        ),
        (
            "This land enters tapped unless you control two or fewer other lands or you may pay 2 life.",
            true,
        ),
        (
            "This land enters tapped unless you control exactly two other lands.",
            true,
        ),
        (
            "This land enters tapped unless you control 4294967296 or more other lands.",
            true,
        ),
        (
            "This land enters tapped unless you control two or fewer other lands. This land enters tapped unless you have two or more opponents.",
            true,
        ),
    ] {
        let cards = deck(&[
            row(
                "Synthetic conditional land",
                "",
                "Land",
                &format!("{text}\n{{T}}: Add {{G}}."),
            ),
            row("Entry-funded spell", "{G}", "Sorcery", "Gain 1 life."),
            row("Library remainder", "{99}", "Sorcery", ""),
        ]);
        let mut game = main_phase_state(vec![0, 1], vec![2]);
        let random = game.library_rng.clone();
        assert!(play_land(&cards, &mut game, 1));
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(game.battlefield[0].tapped, tapped, "{text}");
        assert!(game.battlefield[0].land_face);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert_library_rng(&game.library_rng, &random);
        let (game, log) = execute_turns(&cards, state(vec![0, 1], vec![2]), 1, vec![]);
        assert_eq!(game.hand, if tapped { vec![CardIdx(1)] } else { vec![] });
        assert_eq!(
            game.graveyard,
            if tapped { vec![] } else { vec![CardIdx(1)] }
        );
        assert_eq!(game.library, [CardIdx(2)]);
        assert!(game.exile.is_empty());
        assert!(game.battlefield[0].tapped);
        assert_eq!(game.life, 20 + i32::from(!tapped));
        assert_eq!(game.life_paid, 0);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.mana_available, [f64::from(!tapped)]);
        assert_eq!(log.mana_spent, [f64::from(!tapped)]);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(report.mana.avg_cast_spent_by_turn[&1], f64::from(!tapped));
        assert_eq!(report.mana.avg_available_by_turn[&1], f64::from(!tapped));
        assert_eq!(report.draw.avg_life_paid, Some(0.0));
        assert_eq!(report.draw.avg_life_gained, Some(f64::from(!tapped)));
        assert_assumption(
            &report,
            "Unmodeled entry conditions are not exact rules support",
        );
    }
}

/// A real Aesi observes a milled physical land without receiving a land-entry reward.
pub(super) fn assert_landfall_library_move_report() {
    let source = named_row("Aesi, Tyrant of Gyre Strait");
    pinned_revision(&source);
    for land in [true, false] {
        let cards = deck(&[
            source.clone(),
            row("Synthetic mill action", "{0}", "Sorcery", "Mill a card."),
            row("Library remainder", "{99}", "Sorcery", ""),
            row(
                "Physical mill target",
                if land { "" } else { "{99}" },
                if land { "Land" } else { "Sorcery" },
                "",
            ),
        ]);
        let mut game = state(vec![1], vec![2, 3]);
        put_permanents(&cards, &mut game, &[0]);
        let random = game.library_rng.clone();
        let (game, log) = execute_turns(&cards, game, 1, vec![]);
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert!(!game.battlefield[0].face_down);
        assert_eq!(game.graveyard, [CardIdx(3), CardIdx(1)]);
        assert_eq!(game.library, [CardIdx(2)]);
        assert!(game.hand.is_empty() && game.exile.is_empty());
        assert_eq!(game.seen, 1);
        assert_eq!(game.milled_self, 1);
        assert_eq!(game.life, 20);
        assert_library_rng(&game.library_rng, &random);
        assert_eq!(log.cards_seen, [1]);
        assert_eq!(log.land_drops, [0]);
        assert_eq!(log.library_size, [1]);
        assert_eq!(log.self_milled, [1]);
        let report = report_log(&cards, &log, 1);
        assert_finished_turn(&report, &log);
        assert_eq!(report.velocity.avg_cards_seen[&1], 1.0);
        assert_eq!(report.velocity.self_milled_by_turn[&1], 1.0);
        assert_eq!(report.velocity.library_remaining_by_turn[&1], 1.0);
        assert_eq!(report.draw.avg_life_gained, Some(0.0));
        let json = serde_json::to_value(report).expect("report serializes");
        crate::deck::simulator::library_core_report_tests::assert_report_clause(
            &json,
            &source,
            "Landfall",
            "supported",
            None,
        );
    }
    crate::deck::simulator::card_landfall_claim_tests::assert_landfall_draw(&source);
}
