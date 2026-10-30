//! Landfall and extra-play certificates use physical entries and executed turns.

use crate::db::CardRow;
use crate::deck::simulator::aggregate::aggregate;
use crate::deck::simulator::card_land_claim_tests::{
    assert_land_source_report, assert_pinned_land_source, synthetic_fetch,
};
use crate::deck::simulator::card_report_tests::{execute_turn, report_executed_turn};
use crate::deck::simulator::card_runtime_tests::attempt_index;
use crate::deck::simulator::card_test_support::{assert_library_rng, main_phase_state, named_row};
use crate::deck::simulator::cast_pass::play_land;
use crate::deck::simulator::game::{CardRef, GameLog, GameState, ManaPool, new_perm_with};
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_effects::activation::{
    pick_best_activation, resolve_activation, resolve_sacrifice_uid,
};
use crate::deck::simulator::game_run::mana::build_pool;
use crate::deck::simulator::game_run::{TurnCensus, census::finish_log, run_turn};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use serde_json::Value;

/// Report several executed turns without supplying expected-state census data.
fn report_turns(
    cards: &SimDeck,
    game: GameState,
    census: TurnCensus,
    turns: u32,
) -> (GameLog, Value) {
    let log = finish_log(cards, game, census, 0, false, turns as usize);
    let stats = aggregate(std::slice::from_ref(&log), cards, turns);
    let report = crate::deck::simulator::report::json_report(
        &stats,
        cards,
        "Pinned land clauses",
        0,
        &[],
        Default::default(),
        &Default::default(),
    );
    let report = serde_json::to_value(report).expect("production report serializes");
    assert_eq!(log.turns_observed, turns);
    assert_eq!(report["outcomes"]["finished_games"], 1);
    (log, report)
}

/// Aesi draws only for an actual land entry while its source is visible.
pub(super) fn assert_landfall_draw(source: &CardRow) {
    let wording = "Landfall — Whenever a land you control enters, you may draw a card.";
    assert_pinned_land_source(source, "edhrec_top250", wording);
    assert!(source.keywords.contains("Landfall"));
    for (land, visible, draws) in [
        (true, true, true),
        (false, true, false),
        (true, false, false),
    ] {
        let cards = deck(&[
            source.clone(),
            row(
                "Entered object",
                "{0}",
                if land { "Land" } else { "Artifact" },
                "",
            ),
            row("Bottom physical card", "{99}", "Sorcery", ""),
            row("Top physical draw", "{99}", "Sorcery", ""),
        ]);
        let mut game = main_phase_state(vec![1], vec![2, 3]);
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        game.battlefield[0].face_down = !visible;
        game.next_uid = 8;
        let random = game.library_rng.clone();
        if land {
            assert!(play_land(&cards, &mut game, 1));
        } else {
            assert_eq!(
                attempt_index(&cards, &mut game, &mut ManaPool::default(), CardIdx(1)),
                Some(0)
            );
        }
        assert_eq!(game.hand, if draws { vec![CardIdx(3)] } else { vec![] });
        assert_eq!(
            game.library,
            if draws {
                vec![CardIdx(2)]
            } else {
                vec![CardIdx(2), CardIdx(3)]
            }
        );
        assert_eq!(game.seen, u32::from(draws));
        assert_eq!(game.life, 20);
        assert_eq!(game.treasure_bank, 0);
        assert_library_rng(&game.library_rng, &random);

        let mut game = state(vec![1], vec![2, 3]);
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        game.battlefield[0].face_down = !visible;
        game.next_uid = 8;
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.hand, if draws { vec![CardIdx(3)] } else { vec![] });
        assert_eq!(
            game.library,
            if draws {
                vec![CardIdx(2)]
            } else {
                vec![CardIdx(2), CardIdx(3)]
            }
        );
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [u8::from(land)]);
        assert_eq!(log.cards_seen, [u32::from(draws)]);
        assert_eq!(log.library_size, [if draws { 1 } else { 2 }]);
        assert_eq!(
            report["velocity"]["avg_cards_seen"]["1"],
            f64::from(u32::from(draws))
        );
        assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 0.0);
        assert_land_source_report(source, &report, wording, "supported");
    }
}

/// Check a live static source, a hidden source, and removal through the production sacrifice path.
fn assert_static_extra_plays(source: &CardRow, wording: &str, extra: u8) {
    assert_pinned_land_source(source, "lands", wording);
    for hidden in [false, true] {
        let mut rows = vec![source.clone()];
        rows.extend(
            (0..4).map(|_| row("Physical land", "", "Basic Land — Forest", "{T}: Add {G}.")),
        );
        let cards = deck(&rows);
        assert_eq!(cards.cards[0].flags.extra_land_drops, u32::from(extra));
        let mut game = state(vec![1, 2, 3, 4], vec![]);
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        game.battlefield[0].face_down = hidden;
        game.next_uid = 8;
        let census = execute_turn(&cards, &mut game);
        let played = 1 + if hidden { 0 } else { extra };
        let expected_board: Vec<_> = (1..=u32::from(played))
            .map(|index| CardRef::Deck(CardIdx(index)))
            .collect();
        assert_eq!(
            game.battlefield
                .iter()
                .skip(1)
                .map(|permanent| permanent.card)
                .collect::<Vec<_>>(),
            expected_board
        );
        assert_eq!(
            game.hand,
            (u32::from(played) + 1..=4).map(CardIdx).collect::<Vec<_>>()
        );
        assert_eq!(game.life, 20);
        assert_eq!(game.extra_land_drops_this_turn, 0);
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [played]);
        assert_eq!(log.lands_seen_by_turn_4, 4);
        assert_eq!(log.cards_seen, [0]);
        assert_eq!(
            report["mana"]["avg_available_by_turn"]["1"],
            f64::from(played)
        );
        assert_eq!(report["mana"]["avg_unused_by_turn"]["1"], f64::from(played));
        assert_eq!(
            report["land_drops"]["percent_games_hitting_all_land_drops_by_turn"]["1"],
            100.0
        );
        assert_land_source_report(source, &report, wording, "unsupported");
    }
    let cards = deck(&[
        source.clone(),
        row("Remaining land one", "", "Land", ""),
        row("Remaining land two", "", "Land", ""),
    ]);
    let mut game = main_phase_state(vec![1, 2], vec![]);
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
    resolve_sacrifice_uid(&cards, &mut game, 1, 7);
    let mut census = TurnCensus::new(1, cards.cards.len());
    crate::deck::simulator::game_run::play_late_land(&cards, &mut game, &mut census, 1);
    assert_eq!(census.land_drops, [1]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.hand, [CardIdx(2)]);
}

/// Azusa contributes exactly two additional plays while its physical source remains live.
pub(super) fn assert_azusa_extra_plays(source: &CardRow) {
    assert_static_extra_plays(
        source,
        "You may play two additional lands on each of your turns.",
        2,
    );
}

/// Exploration contributes exactly one additional play while its physical source remains live.
pub(super) fn assert_exploration_extra_plays(source: &CardRow) {
    assert_static_extra_plays(
        source,
        "You may play an additional land on each of your turns.",
        1,
    );
}

/// Explore pays real green mana, draws the physical top card, and cannot use a wrong-color pool.
pub(super) fn assert_explore_current_turn(source: &CardRow) {
    assert_pinned_land_source(
        source,
        "edhrec_top250",
        "You may play an additional land this turn.\nDraw a card.",
    );
    assert_explore_payment(source);
    assert_explore_turn_reset(source);
}

/// Explore's payment boundary uses actual land mana and preserves an unpaid spell.
fn assert_explore_payment(source: &CardRow) {
    for green in [true, false] {
        let cards = deck(&[
            source.clone(),
            row(
                "Funding land one",
                "",
                if green {
                    "Basic Land — Forest"
                } else {
                    "Basic Land — Island"
                },
                if green {
                    "{T}: Add {G}."
                } else {
                    "{T}: Add {U}."
                },
            ),
            row(
                "Funding land two",
                "",
                "Basic Land — Island",
                "{T}: Add {U}.",
            ),
            row("Drawn physical card", "{99}", "Sorcery", ""),
        ]);
        let mut game = main_phase_state(vec![0], vec![3]);
        for index in 1..=2 {
            game.battlefield
                .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
        }
        let mut pool = build_pool(&cards, &mut game, 1);
        assert_eq!(pool.total(), 2);
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
            green.then_some(2)
        );
        assert_eq!(game.extra_land_drops_this_turn, u32::from(green));
        assert_eq!(
            game.hand,
            if green {
                vec![CardIdx(3)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(game.library, if green { vec![] } else { vec![CardIdx(3)] });
        assert_eq!(
            game.graveyard,
            if green { vec![CardIdx(0)] } else { vec![] }
        );
        assert_eq!(pool.total(), if green { 0 } else { 2 });
    }
}

/// Two executed turns separate Explore's one-turn permission from the normal turn draw.
fn assert_explore_turn_reset(source: &CardRow) {
    let mut rows = vec![source.clone()];
    rows.extend((0..4).map(|_| {
        row(
            "Physical Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        )
    }));
    rows.extend((0..3).map(|_| row("Physical draw", "{99}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut game = state(vec![0, 2, 3, 4], vec![5, 6, 7]);
    game.battlefield
        .push(new_perm_with(7, &cards, CardIdx(1), 0, false));
    game.next_uid = 8;
    let mut census = TurnCensus::new(2, cards.cards.len());
    let commander = CommanderProfile::new(&cards);
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut Vec::new(),
        1,
        false,
    );
    assert_eq!(game.extra_land_drops_this_turn, 1);
    assert_eq!(game.hand, [CardIdx(4), CardIdx(7)]);
    assert_eq!(game.library, [CardIdx(5), CardIdx(6)]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut Vec::new(),
        2,
        false,
    );
    assert_eq!(game.extra_land_drops_this_turn, 0);
    assert_eq!(game.hand, [CardIdx(7), CardIdx(6)]);
    assert_eq!(game.library, [CardIdx(5)]);
    let (log, report) = report_turns(&cards, game, census, 2);
    assert_eq!(log.land_drops, [2, 1]);
    assert_eq!(log.cards_seen, [1, 2]);
    assert_eq!(log.mana_spent, [2.0, 0.0]);
    assert_eq!(report["velocity"]["avg_cards_seen"]["1"], 1.0);
    assert_eq!(report["velocity"]["avg_cards_seen"]["2"], 2.0);
    assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 2.0);
    assert_land_source_report(
        source,
        &report,
        "You may play an additional land this turn.",
        "compiler_only",
    );
    assert_land_source_report(source, &report, "Draw a card.", "compiler_only");
}

/// Independent live sources stack with Explore; removing Azusa and starting a new turn removes both grants.
pub(super) fn assert_stacked_land_play_sources() {
    let azusa = named_row("Azusa, Lost but Seeking");
    let explore = named_row("Explore");
    let exploration = named_row("Exploration");
    assert_pinned_land_source(&azusa, "lands", &azusa.oracle_text);
    assert_pinned_land_source(&explore, "edhrec_top250", &explore.oracle_text);
    assert_pinned_land_source(&exploration, "lands", &exploration.oracle_text);
    let mut rows = vec![azusa.clone(), explore.clone(), exploration.clone()];
    rows.extend((0..8).map(|_| {
        row(
            "Physical Forest",
            "",
            "Basic Land — Forest",
            "{T}: Add {G}.",
        )
    }));
    rows.extend((0..3).map(|_| row("Physical draw", "{99}", "Sorcery", "")));
    let cards = deck(&rows);
    let mut game = state(vec![1, 3, 4, 5, 6, 7, 8, 9, 10], vec![11, 12, 13]);
    game.battlefield
        .push(new_perm_with(70, &cards, CardIdx(0), 0, false));
    game.battlefield
        .push(new_perm_with(71, &cards, CardIdx(2), 0, false));
    game.next_uid = 72;
    let mut census = TurnCensus::new(2, cards.cards.len());
    let commander = CommanderProfile::new(&cards);
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut Vec::new(),
        1,
        false,
    );
    assert_eq!(census.land_drops[0], 5);
    assert_eq!(game.extra_land_drops_this_turn, 1);
    assert_eq!(
        game.hand,
        [CardIdx(8), CardIdx(9), CardIdx(10), CardIdx(13)]
    );
    assert_eq!(game.library, [CardIdx(11), CardIdx(12)]);
    assert_eq!(
        game.battlefield
            .iter()
            .skip(2)
            .map(|permanent| permanent.card)
            .collect::<Vec<_>>(),
        (3..=7)
            .map(|index| CardRef::Deck(CardIdx(index)))
            .collect::<Vec<_>>()
    );
    resolve_sacrifice_uid(&cards, &mut game, 1, 70);
    assert_eq!(game.graveyard, [CardIdx(1), CardIdx(0)]);
    crate::deck::simulator::game_run::play_late_land(&cards, &mut game, &mut census, 1);
    assert_eq!(census.land_drops[0], 5);
    assert_eq!(
        game.hand,
        [CardIdx(8), CardIdx(9), CardIdx(10), CardIdx(13)]
    );
    run_turn(
        &cards,
        &mut game,
        &mut census,
        &commander,
        &mut Vec::new(),
        2,
        false,
    );
    assert_eq!(game.extra_land_drops_this_turn, 0);
    assert_eq!(game.hand, [CardIdx(10), CardIdx(13), CardIdx(12)]);
    assert_eq!(game.library, [CardIdx(11)]);
    assert_eq!(game.life, 20);
    let (log, report) = report_turns(&cards, game, census, 2);
    assert_eq!(log.land_drops, [5, 2]);
    assert_eq!(log.lands_seen_by_turn_4, 8);
    assert_eq!(log.cards_seen, [1, 2]);
    assert_eq!(log.mana_spent, [2.0, 0.0]);
    assert_eq!(report["mana"]["avg_available_by_turn"]["1"], 4.0);
    assert_eq!(report["mana"]["avg_available_by_turn"]["2"], 7.0);
    assert_land_source_report(&azusa, &report, &azusa.oracle_text, "unsupported");
    assert_land_source_report(
        &exploration,
        &report,
        &exploration.oracle_text,
        "unsupported",
    );
    assert_land_source_report(&explore, &report, "Draw a card.", "compiler_only");
}

/// The selected Cobra creates mana only when a real entry resolves with a live pool.
pub(super) fn assert_cobra_live_pool(source: &CardRow) {
    let wording = "Landfall — Whenever a land you control enters, add one mana of any color.";
    assert_pinned_land_source(source, "lands", wording);
    assert!(source.keywords.contains("Landfall"));
    for (fetch, target_cost, legal) in [
        (true, "{U}", true),
        (true, "{C}", false),
        (false, "{U}", false),
    ] {
        let entry = if fetch {
            synthetic_fetch(false)
        } else {
            row("No-pool physical land", "", "Land", "")
        };
        let cards = deck(&[
            source.clone(),
            entry,
            row("Fetched physical land", "", "Land — Forest", ""),
            row("Cobra-funded spell", target_cost, "Sorcery", ""),
        ]);
        let mut game = main_phase_state(vec![1, 3], if fetch { vec![2] } else { vec![] });
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        game.next_uid = 8;
        assert!(play_land(&cards, &mut game, 1));
        let mut pool = ManaPool::default();
        assert_eq!(game.treasure_bank, 0);
        if fetch {
            let activation = pick_best_activation(&cards, &game, &pool, 1)
                .expect("synthetic fetch has a supported activation");
            resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
        }
        assert_eq!(pool.total(), u32::from(fetch));
        assert_eq!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(3)),
            legal.then_some(1)
        );
        assert_eq!(pool.total(), u32::from(fetch && !legal));
        assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(3)] });
        assert_eq!(game.treasure_bank, 0);

        let mut game = state(vec![1, 3], if fetch { vec![2] } else { vec![] });
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(0), 0, false));
        game.next_uid = 8;
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(3)] });
        assert_eq!(game.life_paid, u32::from(fetch));
        assert_eq!(game.treasure_bank, 0);
        assert!(game.library.is_empty());
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.cards_seen, [u32::from(fetch)]);
        assert_eq!(log.mana_spent, [f64::from(u32::from(legal))]);
        assert_eq!(
            report["mana"]["avg_cast_spent_by_turn"]["1"],
            f64::from(u32::from(legal))
        );
        assert_eq!(
            report["mana"]["avg_unused_by_turn"]["1"],
            f64::from(u32::from(fetch && !legal))
        );
        assert_eq!(report["draw"]["avg_life_paid"], f64::from(u32::from(fetch)));
        assert_eq!(
            report["velocity"]["avg_cards_seen"]["1"],
            f64::from(u32::from(fetch))
        );
        assert!(
            report["assumptions"]
                .as_array()
                .expect("scope limits")
                .iter()
                .any(|limit| limit
                    .as_str()
                    .is_some_and(|text| text.contains("live mana pool")))
        );
        assert_land_source_report(source, &report, wording, "supported");
    }
}

/// A selected landfall draw source executes land and nonland controls through both report layers.
#[test]
fn pinned_landfall_draw_claim() {
    assert_landfall_draw(&named_row("Aesi, Tyrant of Gyre Strait"));
}

/// Static extra plays have separate exact source identities and amounts.
#[test]
fn pinned_static_extra_land_play_claims() {
    assert_azusa_extra_plays(&named_row("Azusa, Lost but Seeking"));
    assert_exploration_extra_plays(&named_row("Exploration"));
}

/// Explore's paid current-turn grant expires before the next turn's land phase.
#[test]
fn pinned_explore_current_turn_claim() {
    assert_explore_current_turn(&named_row("Explore"));
}

/// Source counts and a one-turn effect stack without surviving source removal or turn reset.
#[test]
fn stacked_land_play_sources_and_reset_claim() {
    assert_stacked_land_play_sources();
}

/// Cobra's supported live-pool branch does not invent mana during a no-pool land phase.
#[test]
fn pinned_cobra_live_pool_claim() {
    assert_cobra_live_pool(&named_row("Lotus Cobra"));
}
