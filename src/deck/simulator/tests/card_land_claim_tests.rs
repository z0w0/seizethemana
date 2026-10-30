//! Land certificates cover exact entry branches. Generic fetch evidence is synthetic.

use crate::db::CardRow;
use crate::deck::simulator::card_report_tests::{execute_turn, report_executed_turn};
use crate::deck::simulator::card_test_support::{
    assert_library_rng, corpus, json_hash, named_row, oracle_hash, pinned_revision,
};
use crate::deck::simulator::cast_pass::play_land;
use crate::deck::simulator::game::{CardRef, GameState, ManaPool, new_perm_with};
use crate::deck::simulator::game_effects::activation::{pick_best_activation, resolve_activation};
use crate::deck::simulator::model::{CardIdx, SimDeck};
use crate::deck::simulator::scenario_test_support::{deck, row, state};
use serde_json::Value;

/// Bind a branch to its actual metadata, selection tag, text, and preserved revision.
pub(super) fn assert_pinned_land_source(source: &CardRow, tag: &str, wording: &str) {
    let revision = pinned_revision(source);
    let input = corpus()["cards"]
        .as_array()
        .expect("pinned source cards")
        .iter()
        .find(|card| card["metadata"]["oracle_id"] == source.oracle_id)
        .expect("the original Oracle identity exists")
        .clone();
    assert!(
        input["selection_tags"]
            .as_array()
            .expect("selection tags")
            .iter()
            .any(|value| value == tag)
    );
    assert_eq!(revision["metadata_sha256"], json_hash(&input["metadata"]));
    assert_eq!(
        revision["oracle_text_sha256"],
        oracle_hash(&input["metadata"])
    );
    assert_eq!(source.oracle_text, named_row(&source.name).oracle_text);
    assert!(source.oracle_text.contains(wording));
    assert_eq!(source.type_line, named_row(&source.name).type_line);
}

/// Preserve compiler wording and status without upgrading a card-level field to whole-card support.
pub(super) fn assert_land_source_report(
    source: &CardRow,
    report: &Value,
    wording: &str,
    status: &str,
) {
    let card = report["card_support"]
        .as_array()
        .expect("compiler records")
        .iter()
        .find(|card| card["name"] == source.name)
        .expect("source compiler record");
    assert_eq!(
        card["source_hash"],
        pinned_revision(source)["flattened_input_sha256"]
    );
    assert!(
        card["note"]
            .as_str()
            .expect("compiler limit")
            .contains("Compiler evidence only")
    );
    let ability = card["abilities"]
        .as_array()
        .expect("ability records")
        .iter()
        .find(|ability| {
            ability["source"]
                .as_str()
                .is_some_and(|text| text.contains(wording))
        })
        .expect("the exact original clause remains visible");
    assert_eq!(ability["status"], status);
    if status == "supported" {
        assert!(ability["reason"].is_null());
        assert!(ability["rule_id"].is_null());
    }
    if let (Some(start), Some(end)) = (ability["span_start"].as_u64(), ability["span_end"].as_u64())
    {
        assert_eq!(
            &source.oracle_text[start as usize..end as usize],
            ability["source"].as_str().expect("original wording")
        );
    } else {
        panic!("printed clauses must retain their source span");
    }
    if status == "unsupported" || status == "compiler_only" {
        assert!(
            !ability["reason"]
                .as_str()
                .expect("explicit compiler gap")
                .is_empty()
        );
    }
}

/// Set the number of other lands and active opponent seats without granting extra mana.
fn entry_state(cards: &SimDeck, life: i32, other_lands: usize, opponents: usize) -> GameState {
    let mut game = state(vec![0, 1], vec![]);
    game.life = life;
    for index in 2..2 + other_lands {
        game.battlefield.push(new_perm_with(
            index as u32,
            cards,
            CardIdx(index as u32),
            0,
            false,
        ));
    }
    game.opponents = crate::deck::simulator::opponents::OpponentTable::for_format(
        crate::deck::simulator::model::Format::Commander,
    );
    game.opponents.seats.truncate(opponents);
    game.next_uid = 50;
    game
}

/// Check entry before mana taps, then independently run the same branch through the report pipeline.
fn assert_entry_branch(
    source: &CardRow,
    life: i32,
    other_lands: usize,
    opponents: usize,
    tapped: bool,
    paid: u32,
) {
    let color = if source.color_identity.contains('W') {
        "{W}"
    } else if source.color_identity.contains('U') {
        "{U}"
    } else {
        "{G}"
    };
    let mut rows = vec![
        source.clone(),
        row("Entry-funded spell", color, "Sorcery", ""),
    ];
    rows.extend((0..other_lands).map(|_| row("Other physical land", "", "Land", "")));
    let cards = deck(&rows);
    let mut game = entry_state(&cards, life, other_lands, opponents);
    let random = game.library_rng.clone();
    assert!(play_land(&cards, &mut game, 1));
    let entered = game.battlefield.last().expect("physical land entered");
    assert_eq!(entered.card, CardRef::Deck(CardIdx(0)));
    assert_eq!(entered.uid, 51);
    assert_eq!(game.next_uid, 51);
    assert_eq!(entered.entered_turn, 1);
    assert_eq!(
        entered.tapped, tapped,
        "{}: other lands={other_lands}, opponents={opponents}",
        source.name
    );
    assert_eq!(game.life, life - paid as i32);
    assert_eq!(game.life_paid, paid);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert!(game.library.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
    assert_library_rng(&game.library_rng, &random);

    let mut game = entry_state(&cards, life, other_lands, opponents);
    let census = execute_turn(&cards, &mut game);
    assert_eq!(game.life, life - paid as i32);
    assert_eq!(game.life_paid, paid);
    assert_eq!(
        game.graveyard,
        if tapped { vec![] } else { vec![CardIdx(1)] }
    );
    assert_eq!(game.hand, if tapped { vec![CardIdx(1)] } else { vec![] });
    let (log, report) = report_executed_turn(&cards, game, census);
    assert_eq!(log.land_drops, [1]);
    assert_eq!(log.cards_seen, [0]);
    assert_eq!(log.life_paid, paid);
    assert_eq!(log.mana_spent, [f64::from(u32::from(!tapped))]);
    assert_eq!(
        report["land_drops"]["percent_games_hitting_all_land_drops_by_turn"]["1"],
        100.0
    );
    assert_eq!(
        report["mana"]["avg_available_by_turn"]["1"],
        f64::from(u32::from(!tapped))
    );
    assert_eq!(
        report["mana"]["avg_cast_spent_by_turn"]["1"],
        f64::from(u32::from(!tapped))
    );
    assert_eq!(report["velocity"]["avg_cards_seen"]["1"], 0.0);
    assert_eq!(report["draw"]["avg_life_paid"], f64::from(paid));
    let wording = source
        .oracle_text
        .lines()
        .find(|line| line.contains("enters"))
        .expect("entry clause");
    assert_land_source_report(source, &report, wording, "unsupported");
}

/// A selected three-life MDFC pays for its land face, or enters tapped when life is short.
pub(super) fn assert_three_life_mdfc(source: &CardRow) {
    let wording = "As this land enters, you may pay 3 life. If you don't, it enters tapped.";
    assert_pinned_land_source(source, "edhrec_top250", wording);
    assert!(source.type_line.contains(" // Land"));
    assert_entry_branch(source, 20, 0, 1, false, 3);
    assert_entry_branch(source, 2, 0, 1, true, 0);
    let cards = deck(&[source.clone(), row("Ordinary land", "", "Basic Land", "")]);
    let mut game = state(vec![0, 1], vec![]);
    assert!(play_land(&cards, &mut game, 1));
    assert_eq!(game.hand, [CardIdx(0)]);
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(game.life_paid, 0);
}

/// The selected tapped MDFC cannot fund a spell on its entry turn.
pub(super) fn assert_tapped_mdfc(source: &CardRow) {
    assert_pinned_land_source(source, "edhrec_top250", "This land enters tapped.");
    assert!(source.type_line.contains("Sorcery // Land"));
    assert_entry_branch(source, 20, 0, 1, true, 0);
}

/// The wider shock source pays two life independently of its mana ability.
pub(super) fn assert_shock_entry(source: &CardRow) {
    assert_pinned_land_source(
        source,
        "lathril_selected",
        "As this land enters, you may pay 2 life. If you don't, it enters tapped.",
    );
    assert!(source.type_line.contains("Swamp Forest"));
    assert_entry_branch(source, 20, 0, 1, false, 2);
    assert_entry_branch(source, 1, 0, 1, true, 0);
}

/// The wider fast land checks two versus three other physical lands.
pub(super) fn assert_fast_land_entry(source: &CardRow) {
    assert_pinned_land_source(
        source,
        "lathril_selected",
        "This land enters tapped unless you control two or fewer other lands.",
    );
    assert_entry_branch(source, 20, 0, 1, false, 0);
    assert_entry_branch(source, 20, 2, 1, false, 0);
    assert_entry_branch(source, 20, 3, 1, true, 0);
}

/// The wider slow land checks one versus two other physical lands.
pub(super) fn assert_slow_land_entry(source: &CardRow) {
    assert_pinned_land_source(
        source,
        "lathril_selected",
        "This land enters tapped unless you control two or more other lands.",
    );
    assert_entry_branch(source, 20, 1, 1, true, 0);
    assert_entry_branch(source, 20, 2, 1, false, 0);
}

/// The wider multiplayer land checks one versus two active opponent seats.
pub(super) fn assert_multiplayer_land_entry(source: &CardRow) {
    assert_pinned_land_source(
        source,
        "lathril_selected",
        "This land enters tapped unless you have two or more opponents.",
    );
    assert_entry_branch(source, 20, 0, 1, true, 0);
    assert_entry_branch(source, 20, 0, 2, false, 0);
}

/// Build a synthetic generic fetch; this is not a selected-source certificate.
pub(super) fn synthetic_fetch(forced_tapped: bool) -> CardRow {
    row(
        "Synthetic typed fetch",
        "",
        "Land",
        &format!(
            "{{T}}, Pay 1 life, Sacrifice this land: Search your library for a Swamp or Forest card, put it onto the battlefield{}, then shuffle.",
            if forced_tapped { " tapped" } else { "" }
        ),
    )
}

/// Generic fetch actions pay life and preserve type misses while destination rules govern shock entry.
pub(super) fn assert_synthetic_fetch_actions() {
    assert_synthetic_fetch_type_boundaries();
    for (forced, life, matching, paid, tapped) in [
        (false, 20, true, 3, false),
        (false, 2, true, 1, true),
        (true, 20, true, 1, true),
        (false, 20, false, 1, false),
    ] {
        let target = if matching {
            named_row("Overgrown Tomb")
        } else {
            row("Type miss", "", "Land — Island", "")
        };
        let cards = deck(&[
            synthetic_fetch(forced),
            target,
            row("Second type miss", "", "Basic Land — Mountain", ""),
        ]);
        let mut game = state(vec![0], vec![1, 2]);
        game.life = life;
        assert!(play_land(&cards, &mut game, 1));
        assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(game.life_paid, 0);
        let mut expected_library = if matching {
            vec![CardIdx(2)]
        } else {
            vec![CardIdx(1), CardIdx(2)]
        };
        let mut random = game.library_rng.clone();
        crate::deck::simulator::rng::shuffle(&mut expected_library, &mut random);
        let mut pool = ManaPool::default();
        let activation =
            pick_best_activation(&cards, &game, &pool, 1).expect("complete generic fetch cost");
        resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
        assert_eq!(game.life, life - paid as i32);
        assert_eq!(game.life_paid, paid);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.library, expected_library);
        assert_library_rng(&game.library_rng, &random);
        assert_eq!(game.battlefield.len(), usize::from(matching));
        if matching {
            assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(1)));
            assert_eq!(game.battlefield[0].tapped, tapped);
        }
        assert!(game.hand.is_empty() && game.exile.is_empty());
        assert_eq!(pool.total(), 0);
        assert_eq!(game.treasure_bank, 0);
    }
}

/// Synthetic fetches accept either printed subtype and reject an unrelated subtype or typeless basic.
pub(super) fn assert_synthetic_fetch_type_boundaries() {
    for (target_type, matching) in [
        ("Land — Swamp", true),
        ("Land — Forest", true),
        ("Land — Island", false),
        ("Basic Land", false),
    ] {
        let cards = deck(&[
            synthetic_fetch(false),
            row("Synthetic typed target", "", target_type, ""),
        ]);
        let mut game = state(vec![0], vec![1]);
        assert!(play_land(&cards, &mut game, 1));
        let mut pool = ManaPool::default();
        let activation = pick_best_activation(&cards, &game, &pool, 1)
            .expect("hidden-library search may fail after paying its costs");
        resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!(game.life, 19);
        assert_eq!(game.life_paid, 1);
        assert_eq!(
            game.library,
            if matching { vec![] } else { vec![CardIdx(1)] }
        );
        assert_eq!(game.battlefield.len(), usize::from(matching));
        if matching {
            assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(1)));
            assert!(!game.battlefield[0].tapped);
        }
        let mut game = state(vec![0], vec![1]);
        let census = execute_turn(&cards, &mut game);
        assert_eq!(
            game.library,
            if matching { vec![] } else { vec![CardIdx(1)] }
        );
        assert_eq!(game.graveyard, [CardIdx(0)]);
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.lands_seen_by_turn_4, 1 + u32::from(matching));
        assert_eq!(log.library_size, [u32::from(!matching)]);
        assert_eq!(log.cards_seen, [u32::from(matching)]);
        assert_eq!(report["draw"]["avg_life_paid"], 1.0);
        assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 0.0);
        assert_eq!(
            report["velocity"]["avg_cards_seen"]["1"],
            f64::from(u32::from(matching))
        );
    }
}

/// Full synthetic fetch turns report a land play, real life costs, and only available follow-up mana.
pub(super) fn assert_synthetic_fetch_report() {
    for (forced, life, matching, paid, can_cast) in [
        (false, 20, true, 3, true),
        (false, 2, true, 1, false),
        (true, 20, true, 1, false),
        (false, 20, false, 1, false),
    ] {
        let target = if matching {
            named_row("Overgrown Tomb")
        } else {
            row("Type miss", "", "Land — Island", "")
        };
        let cards = deck(&[
            synthetic_fetch(forced),
            target,
            row("Fetch-funded spell", "{G}", "Sorcery", ""),
        ]);
        let mut game = state(vec![0, 2], vec![1]);
        game.life = life;
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.life, life - paid as i32);
        assert_eq!(
            game.graveyard,
            if can_cast {
                vec![CardIdx(0), CardIdx(2)]
            } else {
                vec![CardIdx(0)]
            }
        );
        assert_eq!(game.hand, if can_cast { vec![] } else { vec![CardIdx(2)] });
        assert_eq!(
            game.library,
            if matching { vec![] } else { vec![CardIdx(1)] }
        );
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.lands_seen_by_turn_4, 1 + u32::from(matching));
        assert_eq!(log.life_paid, paid);
        assert_eq!(log.cards_seen, [u32::from(matching)]);
        assert_eq!(report["draw"]["avg_life_paid"], f64::from(paid));
        assert_eq!(
            report["mana"]["avg_cast_spent_by_turn"]["1"],
            f64::from(u32::from(can_cast))
        );
        assert_eq!(
            report["velocity"]["avg_cards_seen"]["1"],
            f64::from(u32::from(matching))
        );
        assert_eq!(
            report["land_drops"]["percent_games_hitting_all_land_drops_by_turn"]["1"],
            100.0
        );
        let generic = report["card_support"]
            .as_array()
            .expect("compiler records")
            .iter()
            .find(|card| card["name"] == "Synthetic typed fetch")
            .expect("synthetic record");
        assert_eq!(generic["abilities"][0]["status"], "approximated");
        assert_eq!(
            generic["abilities"][0]["rule_id"],
            "bounded_search_selection"
        );
    }
}

/// Fabled Passage's linked untap clause blocks the whole activation rather than granting a partial fetch.
pub(super) fn assert_fabled_passage_safety(source: &CardRow) {
    let wording = "{T}, Sacrifice this land: Search your library for a basic land card, put it onto the battlefield tapped, then shuffle. Then if you control four or more lands, untap that land.";
    assert_pinned_land_source(source, "lathril_selected", wording);
    for other_lands in [0, 3, 4] {
        let mut rows = vec![
            source.clone(),
            row("Basic target", "", "Basic Land — Forest", "{T}: Add {G}."),
        ];
        rows.extend((0..other_lands).map(|_| row("Other physical land", "", "Land", "")));
        let cards = deck(&rows);
        let mut game = state(vec![0], vec![1]);
        for index in 2..2 + other_lands {
            game.battlefield.push(new_perm_with(
                index as u32,
                &cards,
                CardIdx(index as u32),
                0,
                false,
            ));
        }
        game.next_uid = 50;
        let random = game.library_rng.clone();
        assert!(play_land(&cards, &mut game, 1));
        let pool = ManaPool::default();
        assert!(pick_best_activation(&cards, &game, &pool, 1).is_none());
        assert_eq!(game.library, [CardIdx(1)]);
        assert!(game.graveyard.is_empty());
        assert_library_rng(&game.library_rng, &random);
        let mut game = state(vec![0], vec![1]);
        for index in 2..2 + other_lands {
            game.battlefield.push(new_perm_with(
                index as u32,
                &cards,
                CardIdx(index as u32),
                0,
                false,
            ));
        }
        game.next_uid = 50;
        let census = execute_turn(&cards, &mut game);
        assert_eq!(game.library, [CardIdx(1)]);
        assert!(game.hand.is_empty() && game.graveyard.is_empty());
        assert_eq!(game.life_paid, 0);
        assert_library_rng(&game.library_rng, &random);
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.cards_seen, [0]);
        assert_eq!(report["mana"]["avg_cast_spent_by_turn"]["1"], 0.0);
        assert_eq!(report["draw"]["avg_life_paid"], 0.0);
        assert_land_source_report(source, &report, wording, "unsupported");
    }
}

/// Selected MDFC sources and wider shock entry execute their exact supported branches.
#[test]
fn pinned_mdfc_and_shock_entry_claims() {
    assert_three_life_mdfc(&named_row("Sink into Stupor // Soporific Springs"));
    assert_tapped_mdfc(&named_row("Bala Ged Recovery // Bala Ged Sanctuary"));
    assert_shock_entry(&named_row("Overgrown Tomb"));
}

/// Exact fast-land thresholds cannot be replaced with a fixed early-game assumption.
#[test]
fn pinned_fast_land_entry_claim() {
    assert_fast_land_entry(&named_row("Blooming Marsh"));
}

/// Exact slow-land thresholds cannot be replaced with unconditional tapped entry.
#[test]
fn pinned_slow_land_entry_claim() {
    assert_slow_land_entry(&named_row("Deathcap Glade"));
}

/// Active opponent count controls the multiplayer land's entry state.
#[test]
fn pinned_multiplayer_land_entry_claim() {
    assert_multiplayer_land_entry(&named_row("Undergrowth Stadium"));
}

/// Synthetic generic fetch coverage does not add a selected positive fetch identity.
#[test]
fn synthetic_fetch_action_and_report_claims() {
    assert_synthetic_fetch_actions();
    assert_synthetic_fetch_report();
}

/// The wider linked fetch source contributes safety evidence only.
#[test]
fn pinned_fabled_passage_safety_claim() {
    assert_fabled_passage_safety(&named_row("Fabled Passage"));
}

/// The supported number predicates have exact inclusive boundaries.
#[test]
fn land_entry_conditions_use_exact_state_counts() {
    use crate::deck::simulator::model::LandEntryCondition;
    let at_most = LandEntryCondition::OtherLandsAtMost(2);
    assert!(at_most.is_met_with_land_types(0, 0, [false; 5]));
    assert!(at_most.is_met_with_land_types(2, 0, [false; 5]));
    assert!(!at_most.is_met_with_land_types(3, 3, [false; 5]));
    let at_least = LandEntryCondition::OtherLandsAtLeast(2);
    assert!(!at_least.is_met_with_land_types(1, 3, [false; 5]));
    assert!(at_least.is_met_with_land_types(2, 0, [false; 5]));
    let opponents = LandEntryCondition::OpponentsAtLeast(2);
    assert!(!opponents.is_met_with_land_types(9, 1, [false; 5]));
    assert!(opponents.is_met_with_land_types(0, 2, [false; 5]));
}

/// Known-hand turn-two payments use one other land and the format's default opponent seats.
#[test]
fn known_hand_entry_conditions_use_the_same_thresholds() {
    use crate::deck::simulator::hand_payment::known_hand_payments;
    use crate::deck::simulator::model::Format;
    for (condition, format, payable) in [
        (
            "you control 0 or fewer other lands",
            Format::Constructed,
            false,
        ),
        (
            "you control 1 or fewer other lands",
            Format::Constructed,
            true,
        ),
        (
            "you control 1 or more other lands",
            Format::Constructed,
            true,
        ),
        (
            "you control two or more other lands",
            Format::Constructed,
            false,
        ),
        ("you have two or more opponents", Format::Constructed, false),
        ("you have two or more opponents", Format::Commander, true),
        (
            "you control two or fewer other lands and an Elf",
            Format::Commander,
            false,
        ),
    ] {
        let text = format!("This land enters tapped unless {condition}.\n{{T}}: Add {{G}}.");
        let rows = [
            row("First conditional land", "", "Land", &text),
            row("Second conditional land", "", "Land", &text),
            row("Two-source spell", "{G}{G}", "Sorcery", ""),
        ];
        let mut cards = deck(&rows);
        cards.format = format;
        cards.rules = crate::deck::simulator::format::rules_for(if format == Format::Commander {
            "commander"
        } else {
            "constructed"
        });
        let metadata = rows
            .into_iter()
            .map(|row| (row.name.clone(), row))
            .collect();
        let result = known_hand_payments(&cards, &[CardIdx(0), CardIdx(1), CardIdx(2)], &metadata);
        assert_eq!(
            result.payable_spells,
            usize::from(payable),
            "{condition}, {format:?}"
        );
    }
}

/// Renamed sources keep their parsed rule; changed wording changes the threshold under the same name.
#[test]
fn conditional_entry_actions_follow_text_not_card_names() {
    for (name, old, new, lands, opponents) in [
        ("Blooming Marsh", "two or fewer", "three or fewer", 3, 1),
        ("Deathcap Glade", "two or more", "one or more", 1, 1),
        ("Undergrowth Stadium", "two or more", "one or more", 0, 1),
    ] {
        let source = named_row(name);
        let mut renamed = source.clone();
        renamed.name = "Renamed entry control".into();
        let mut changed = source.clone();
        changed.oracle_text = changed.oracle_text.replace(old, new);
        for (control, tapped) in [(source, true), (renamed, true), (changed, false)] {
            let mut rows = vec![control, row("Uncast control", "{99}", "Sorcery", "")];
            rows.extend((0..lands).map(|_| row("Other land", "", "Land", "")));
            let cards = deck(&rows);
            let mut game = entry_state(&cards, 20, lands, opponents);
            assert!(play_land(&cards, &mut game, 1));
            assert_eq!(
                game.battlefield.last().expect("entered source").tapped,
                tapped
            );
            assert_eq!(game.hand, [CardIdx(1)]);
            assert_eq!(game.life, 20);
            assert_eq!(game.life_paid, 0);
        }
    }
}

/// Scheduling external activity does not add or remove an opponent for entry conditions.
#[test]
fn multiplayer_entry_counts_active_seats_without_activity_gating() {
    let cards = deck(&[
        named_row("Undergrowth Stadium"),
        row("Uncast control", "{99}", "Sorcery", ""),
    ]);
    for (activity, active_seats, tapped) in [
        (false, 1, true),
        (false, 2, false),
        (true, 1, true),
        (true, 2, false),
    ] {
        let mut game = entry_state(&cards, 20, 0, 3);
        game.opponents.config.active = activity;
        for (index, seat) in game.opponents.seats.iter_mut().enumerate() {
            seat.active = index < active_seats;
        }
        assert!(play_land(&cards, &mut game, 1));
        assert_eq!(game.battlefield[0].tapped, tapped);
        assert_eq!(game.life, 20);
    }
}

/// A controlled MDFC has one front entry trigger and one land-face mana mode.
fn face_control_row() -> CardRow {
    let mut source = row(
        "Creature face // Land face",
        "{0}",
        "Creature // Land",
        "When this creature enters, draw a card.\n//\n{T}: Add {G}.",
    );
    source.keywords = "[\"Haste\"]".into();
    source
}

/// Playing the land face suppresses front creature behavior; casting the spell face suppresses land mana.
#[test]
fn mdfc_entry_uses_the_chosen_physical_face() {
    use crate::deck::simulator::game::{
        is_creature_permanent, is_land_permanent, permanent_abilities,
    };
    for as_land in [true, false] {
        let cards = deck(&[
            face_control_row(),
            row("Physical draw", "{99}", "Sorcery", ""),
            row("Ordinary land", "", "Land", ""),
        ]);
        let mut game = state(if as_land { vec![0] } else { vec![0, 2] }, vec![1]);
        let census = execute_turn(&cards, &mut game);
        let source = game
            .battlefield
            .iter()
            .find(|permanent| permanent.card == CardRef::Deck(CardIdx(0)))
            .expect("MDFC entered through the chosen route");
        assert_eq!(source.land_face, as_land);
        assert_eq!(is_land_permanent(source), as_land);
        assert_eq!(is_creature_permanent(&cards, source), !as_land);
        assert!(
            permanent_abilities(&cards, source)
                .all(|ability| ability.kind.is_triggered() != as_land)
        );
        assert_eq!(game.hand, if as_land { vec![] } else { vec![CardIdx(1)] });
        assert_eq!(
            game.library,
            if as_land { vec![CardIdx(1)] } else { vec![] }
        );
        assert!(game.graveyard.is_empty());
        let (log, report) = report_executed_turn(&cards, game, census);
        assert_eq!(log.land_drops, [1]);
        assert_eq!(log.creatures, [u32::from(!as_land)]);
        assert_eq!(log.lands_seen_by_turn_4, 1);
        assert_eq!(log.cards_seen, [u32::from(!as_land)]);
        assert_eq!(
            report["mana"]["avg_unused_by_turn"]["1"],
            f64::from(u32::from(as_land))
        );
    }
}

/// A land-played MDFC counts toward the threshold; a cast MDFC and a face-down land do not.
#[test]
fn conditional_entry_counts_only_physical_land_faces() {
    use crate::deck::simulator::card_runtime_tests::attempt_index;
    let cards = deck(&[
        named_row("Deathcap Glade"),
        face_control_row(),
        row("Other land", "", "Land", ""),
        row("Other artifact", "", "Artifact", ""),
        row("Entry draw", "{99}", "Sorcery", ""),
        row("Other land two", "", "Land", ""),
    ]);
    for as_land in [true, false] {
        let mut game =
            crate::deck::simulator::card_test_support::main_phase_state(vec![1], vec![4]);
        for index in 2..=3 {
            game.battlefield
                .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
        }
        game.next_uid = 10;
        if as_land {
            assert!(play_land(&cards, &mut game, 1));
        } else {
            assert_eq!(
                attempt_index(&cards, &mut game, &mut ManaPool::default(), CardIdx(1)),
                Some(0)
            );
        }
        game.hand.push(CardIdx(0));
        assert!(play_land(&cards, &mut game, 1));
        assert_eq!(
            game.battlefield.last().expect("conditional entry").tapped,
            !as_land
        );
    }
    let mut game = state(vec![0], vec![]);
    for index in [2, 5] {
        let mut land = new_perm_with(index, &cards, CardIdx(index), 0, false);
        land.face_down = index == 5;
        game.battlefield.push(land);
    }
    assert!(play_land(&cards, &mut game, 1));
    assert!(game.battlefield.last().expect("conditional entry").tapped);
}

/// Search, reveal, and reanimation entries share the normal land's state rule.
#[test]
fn effect_entry_paths_use_the_supported_land_condition() {
    use crate::deck::simulator::card_runtime_tests::attempt_index;
    for (kind, tapped) in [
        ("search", false),
        ("forced search", true),
        ("reveal", false),
        ("reanimate", false),
    ] {
        let (cost, text) = match kind {
            "search" => (
                "{0}",
                "Search your library for a land card, put it onto the battlefield, then shuffle.",
            ),
            "forced search" => (
                "{0}",
                "Search your library for a land card, put it onto the battlefield tapped, then shuffle.",
            ),
            "reveal" => (
                "{X}",
                "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
            ),
            "reanimate" => (
                "{0}",
                "Return target creature card from your graveyard to the battlefield.",
            ),
            _ => unreachable!("the test lists only supported entry routes"),
        };
        let mut target = named_row("Deathcap Glade");
        if kind == "reanimate" {
            target.type_line = "Land Creature".into();
        }
        let cards = deck(&[
            row("Entry effect", cost, "Sorcery", text),
            target,
            row("Other land one", "", "Land", "{T}: Add {G}."),
            row("Other land two", "", "Land", "{T}: Add {G}."),
        ]);
        let mut game = crate::deck::simulator::card_test_support::main_phase_state(
            vec![0],
            if kind == "reanimate" { vec![] } else { vec![1] },
        );
        if kind == "reanimate" {
            game.graveyard.push(CardIdx(1));
        }
        for index in 2..=3 {
            game.battlefield
                .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
        }
        game.next_uid = 10;
        let mut pool = crate::deck::simulator::game_run::mana::build_pool(&cards, &mut game, 1);
        assert!(
            attempt_index(&cards, &mut game, &mut pool, CardIdx(0)).is_some(),
            "{kind}"
        );
        let entered = game
            .battlefield
            .iter()
            .find(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
            .expect("effect moved the physical target");
        assert_eq!(entered.tapped, tapped, "{kind}");
        assert!(crate::deck::simulator::game::is_land_permanent(entered));
        assert_eq!(game.life_paid, 0);
        assert!(game.library.is_empty());
    }
}

/// Lands entering in one reveal batch cannot satisfy each other's pre-entry threshold.
#[test]
fn simultaneous_reveal_entries_share_the_pre_entry_land_count() {
    use crate::deck::simulator::card_runtime_tests::attempt_index;
    let cards = deck(&[
        row(
            "Reveal batch",
            "{X}",
            "Sorcery",
            "Reveal the top X cards of your library. You may put any number of permanent cards with mana value X or less from among them onto the battlefield, then put the rest into your graveyard.",
        ),
        named_row("Deathcap Glade"),
        named_row("Deathcap Glade"),
        row("Funding land", "", "Land", "{T}: Add {G}."),
        row("Funding artifact", "", "Artifact", "{T}: Add {G}."),
    ]);
    let mut game = crate::deck::simulator::card_test_support::main_phase_state(vec![0], vec![1, 2]);
    for index in 3..=4 {
        game.battlefield
            .push(new_perm_with(index, &cards, CardIdx(index), 0, false));
    }
    game.next_uid = 10;
    let mut pool = crate::deck::simulator::game_run::mana::build_pool(&cards, &mut game, 1);
    assert_eq!(
        attempt_index(&cards, &mut game, &mut pool, CardIdx(0)),
        Some(2)
    );
    let entries: Vec<_> = game
        .battlefield
        .iter()
        .filter(|permanent| matches!(permanent.card.deck_idx(), Some(CardIdx(1) | CardIdx(2))))
        .collect();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.tapped));
}

/// A synthetic landcycling life cost cannot consume shuffle randomness after terminal payment.
#[test]
fn landcycling_rejected_and_terminal_payments_do_not_shuffle() {
    let mut cards = deck(&[
        row(
            "Synthetic life-paid Forest cycler",
            "{99}",
            "Creature",
            "Forestcycling {1}",
        ),
        row("Forest target", "", "Land — Forest", ""),
        row("Other card", "{99}", "Artifact", ""),
        row("Funding land", "", "Land", "{T}: Add {C}."),
    ]);
    assert_eq!(
        cards.cards[0].spell_data.landcycling_type,
        Some(crate::deck::simulator::model::BasicLandType::Forest)
    );
    assert_eq!(
        cards.cards[0]
            .spell_data
            .cycling_cost
            .as_ref()
            .map(|cost| cost.total()),
        Some(1)
    );
    cards.cards[0].spell_data.cycling_life = 2;
    for life in [1, 2] {
        let mut game = state(vec![0], vec![1, 2]);
        game.life = life;
        game.battlefield
            .push(new_perm_with(7, &cards, CardIdx(3), 0, false));
        game.next_uid = 8;
        let random = game.library_rng.clone();
        let _census = execute_turn(&cards, &mut game);
        assert_eq!(game.library, [CardIdx(1), CardIdx(2)]);
        assert_library_rng(&game.library_rng, &random);
        assert_eq!(game.life_paid, if life == 2 { 2 } else { 0 });
        assert_eq!(game.hand, if life == 2 { vec![] } else { vec![CardIdx(0)] });
        assert_eq!(
            game.graveyard,
            if life == 2 { vec![CardIdx(0)] } else { vec![] }
        );
        assert_eq!(game.completion.is_some(), life == 2);
        if life == 2 {
            assert_eq!(
                game.completion,
                Some(crate::deck::simulator::game::GameCompletion::PlayerLost {
                    reasons: vec![crate::deck::simulator::game::PlayerLossReason::ZeroLife],
                    turn: 1,
                    phase: crate::deck::simulator::game::GamePhase::Main,
                })
            );
        }
    }
}

/// Parser threshold tests share the land claim test fixtures.
#[path = "card_land_entry_parser_tests.rs"]
mod card_land_entry_parser_tests;
