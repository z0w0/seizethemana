//! Search selector actions and reports. Changed text and constructed tutors are synthetic.

use super::*;
use crate::deck::simulator::model::SimEffect;

/// Build one synthetic paid search without adding a card-name execution rule.
fn search_source(selector: &str) -> CardRow {
    row(
        "Synthetic selector search",
        "{U}",
        "Sorcery",
        &format!(
            "Search your library for {selector} card, put it into your graveyard, then shuffle."
        ),
    )
}

/// Check one complete paid search and its independent ordinary-turn report.
fn assert_paid_search_report(
    source: CardRow,
    targets: Vec<CardRow>,
    library: Vec<usize>,
    found: Option<u32>,
    supported: bool,
) {
    let mut rows = vec![source];
    rows.extend(targets);
    let funding = rows.len() as u32;
    rows.push(row(
        "Search funding Island",
        "",
        "Basic Land - Island",
        "{T}: Add {U}.",
    ));
    let cards = deck(&rows);
    assert_eq!(
        cards.cards[0]
            .spell_data
            .effects
            .iter()
            .any(|effect| matches!(effect, SimEffect::Search(_))),
        supported
    );
    assert_search_payment_rejection(&cards, library.clone());
    let mut game = main_phase_state(vec![0], library.clone());
    let mut expected_library = game.library.clone();
    expected_library.retain(|index| Some(index.0) != found);
    let mut expected_rng = game.library_rng.clone();
    if supported {
        crate::deck::simulator::rng::shuffle(&mut expected_library, &mut expected_rng);
    }
    let mut expected_graveyard: Vec<_> = found.map(CardIdx).into_iter().collect();
    expected_graveyard.push(CardIdx(0));
    let mut pool = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        ..ManaPool::default()
    };
    assert_eq!(cast_action(&cards, &mut game, &mut pool), 1.0);
    assert_eq!(pool.total(), 0);
    assert_eq!(game.library, expected_library, "{}", rows[0].oracle_text);
    assert_eq!(game.graveyard, expected_graveyard);
    assert!(game.hand.is_empty() && game.battlefield.is_empty() && game.exile.is_empty());
    assert_eq!(game.seen, u32::from(found.is_some()));
    assert_eq!(game.life_gained, 0);
    assert_library_rng(&game.library_rng, &expected_rng);

    let mut game = state(vec![0], library);
    put_permanents(&cards, &mut game, &[funding]);
    let funding_uid = game.battlefield[0].uid;
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_mana_report(&log, &json, &[1.0], &[1.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(game.library, expected_library, "{}", rows[0].oracle_text);
    assert_eq!(game.graveyard, expected_graveyard);
    assert!(game.hand.is_empty() && game.exile.is_empty());
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].uid, funding_uid);
    assert_eq!(game.battlefield[0].card.deck_idx(), Some(CardIdx(funding)));
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.life, 20);
    assert_eq!(game.life_gained, 0);
    assert_eq!(log.cards_seen, [u32::from(found.is_some())]);
    assert_eq!(
        json["velocity"]["avg_cards_seen"]["1"],
        f64::from(u32::from(found.is_some()))
    );
    assert_eq!(
        json["velocity"]["library_remaining_by_turn"]["1"],
        expected_library.len() as f64
    );
    assert_eq!(log.library_size, [expected_library.len() as u32]);
    if let Some(index) = found {
        assert_eq!(
            log.card_first_graveyard.get(&CardIdx(index).index()),
            Some(&1)
        );
    }
    let support = json["card_support"]
        .as_array()
        .expect("compiler records")
        .iter()
        .find(|record| record["name"] == rows[0].name)
        .expect("search source remains reported");
    assert_eq!(support["abilities"][0]["source"], rows[0].oracle_text);
    assert_eq!(
        support["abilities"][0]["status"],
        if supported {
            "compiler_only"
        } else {
            "unsupported"
        }
    );
    assert_library_rng(&game.library_rng, &expected_rng);
}

/// A wrong-color payment must not start a search, move cards, or consume randomness.
fn assert_search_payment_rejection(cards: &SimDeck, library: Vec<usize>) {
    let mut rejected = main_phase_state(vec![0], library);
    let before = rejected.clone();
    let mut wrong_pool = ManaPool {
        fixed: [0, 0, 0, 1, 0],
        ..ManaPool::default()
    };
    let before_pool = wrong_pool.clone();
    assert_eq!(cast_action(cards, &mut rejected, &mut wrong_pool), 0.0);
    assert_resources_unchanged(&rejected, &before);
    assert_eq!(wrong_pool, before_pool);
}

/// Raw single-face colors outrank costs, while merged multiface colors remain unknown.
pub(super) fn assert_search_colors_report() {
    for (word, symbol) in [
        ("white", "W"),
        ("blue", "U"),
        ("black", "B"),
        ("red", "R"),
        ("green", "G"),
    ] {
        let mut indicator = row("Synthetic stored-color hit", "{99}", "Creature", "");
        indicator.colors = format!("[\"{symbol}\"]");
        let cost_only = row(
            "Synthetic cost-only near miss",
            &format!("{{99}}{{{symbol}}}"),
            "Creature",
            "",
        );
        let mut merged = row(
            "Synthetic unknown front // back",
            "{99} // {99}",
            "Creature // Creature",
            "",
        );
        merged.colors = format!("[\"{symbol}\"]");
        for has_hit in [false, true] {
            assert_paid_search_report(
                search_source(&format!("a {word} creature")),
                vec![indicator.clone(), cost_only.clone(), merged.clone()],
                if has_hit { vec![1, 2, 3] } else { vec![2, 3] },
                has_hit.then_some(1),
                true,
            );
        }
    }
    let devoid = row(
        "Synthetic stored-colorless hit",
        "{99}{G}",
        "Creature",
        "Devoid",
    );
    let mut colored = row("Synthetic indicator near miss", "{99}", "Creature", "");
    colored.colors = "[\"G\"]".into();
    let unknown = row(
        "Synthetic unknown colorless front // back",
        "{99} // {99}",
        "Creature // Creature",
        "",
    );
    for has_hit in [false, true] {
        assert_paid_search_report(
            search_source("a colorless creature"),
            vec![devoid.clone(), colored.clone(), unknown.clone()],
            if has_hit { vec![1, 2, 3] } else { vec![2, 3] },
            has_hit.then_some(1),
            true,
        );
    }
    let green = named_row("Llanowar Elves");
    let merged = named_row("Etali, Primal Conqueror // Etali, Primal Sickness");
    pinned_revision(&green);
    pinned_revision(&merged);
    for has_hit in [false, true] {
        assert_paid_search_report(
            search_source("a green creature"),
            vec![green.clone(), merged.clone()],
            if has_hit { vec![1, 2] } else { vec![2] },
            has_hit.then_some(1),
            true,
        );
    }
}

/// Each supported type and union rejects the same type when it appears only on the back.
fn assert_front_type_reports() {
    for (selector, matching_types, wrong_type) in [
        ("a creature", &["Creature"][..], "Instant"),
        ("a land", &["Land"][..], "Instant"),
        ("an artifact", &["Artifact"][..], "Instant"),
        ("an enchantment", &["Enchantment"][..], "Instant"),
        ("an instant", &["Instant"][..], "Creature"),
        ("a sorcery", &["Sorcery"][..], "Creature"),
        ("a planeswalker", &["Planeswalker"][..], "Instant"),
        (
            "an artifact or enchantment",
            &["Artifact", "Enchantment"][..],
            "Creature",
        ),
        (
            "an instant or sorcery",
            &["Instant", "Sorcery"][..],
            "Creature",
        ),
        (
            "a permanent",
            &[
                "Creature",
                "Land",
                "Artifact",
                "Enchantment",
                "Planeswalker",
                "Battle",
            ][..],
            "Instant",
        ),
        (
            "a basic land",
            &["Basic Land - Forest"][..],
            "Land - Forest",
        ),
    ] {
        for matching in matching_types {
            let front = row(
                "Synthetic front-face hit",
                "{2} // {99}",
                &format!("{matching} // {wrong_type}"),
                "",
            );
            let wrong = row("Synthetic other-type near miss", "{2}", wrong_type, "");
            let back = row(
                "Synthetic back-face near miss",
                "{2} // {99}",
                &format!("{wrong_type} // {matching}"),
                "",
            );
            for has_hit in [false, true] {
                assert_paid_search_report(
                    search_source(selector),
                    vec![front.clone(), wrong.clone(), back.clone()],
                    if has_hit { vec![1, 2, 3] } else { vec![2, 3] },
                    has_hit.then_some(1),
                    true,
                );
            }
        }
    }
}

/// Exact, inclusive minimum, and inclusive maximum bounds run through complete paid searches.
fn assert_mana_value_reports() {
    for (relation, value, matches) in [
        ("2", 1, false),
        ("2", 2, true),
        ("2", 3, false),
        ("equal to 2", 2, true),
        ("exactly 2", 2, true),
        ("2 or more", 1, false),
        ("2 or more", 2, true),
        ("2 or more", 3, true),
        ("2 or greater", 2, true),
        ("2 or less", 1, true),
        ("2 or less", 2, true),
        ("2 or less", 3, false),
        ("0", 0, true),
        ("0", 1, false),
    ] {
        let source = row(
            "Synthetic mana-value search",
            "{U}",
            "Sorcery",
            &format!(
                "Search your library for an artifact card with mana value {relation}, put it into your graveyard, then shuffle."
            ),
        );
        assert_paid_search_report(
            source,
            vec![
                row(
                    "Synthetic mana-value boundary",
                    &format!("{{{value}}}"),
                    "Artifact",
                    "",
                ),
                row(
                    "Synthetic wrong-type same-value",
                    &format!("{{{value}}}"),
                    "Creature",
                    "",
                ),
            ],
            vec![1, 2],
            matches.then_some(1),
            true,
        );
    }
}

/// Unsupported unions and repeated relations cannot execute a partial search or linked reward.
fn assert_rejected_selector_reports() {
    for selector in [
        "an artifact or creature card",
        "a creature or enchantment card",
        "a snow artifact card",
        "an artifact card with mana value 2 and mana value 3",
        "an artifact card with mana value 2 or mana value 3",
        "an artifact card with mana value 2 or 3",
        "an artifact card with mana value 2 and 3",
        "an artifact card with mana value 2 or less and 1 or more",
        "an artifact card with mana value equal to 2 or equal to 3",
    ] {
        let source = row(
            "Synthetic rejected selector",
            "{U}",
            "Sorcery",
            &format!(
                "Search your library for {selector}, put it into your graveyard, then shuffle. You gain 2 life."
            ),
        );
        assert_paid_search_report(
            source,
            vec![
                row(
                    "Synthetic otherwise eligible artifact",
                    "{2}",
                    "Artifact",
                    "",
                ),
                row(
                    "Synthetic otherwise eligible creature",
                    "{3}",
                    "Creature",
                    "",
                ),
            ],
            vec![1, 2],
            None,
            false,
        );
    }
}

/// Pinned union tutors execute their printed top-library placement after their real shuffle.
fn assert_pinned_union_reports() {
    for (name, cost, matching, wrong) in [
        ("Mystical Tutor", [0, 1, 0, 0, 0], "Instant", "Creature"),
        ("Mystical Tutor", [0, 1, 0, 0, 0], "Sorcery", "Creature"),
        ("Enlightened Tutor", [1, 0, 0, 0, 0], "Artifact", "Creature"),
        (
            "Enlightened Tutor",
            [1, 0, 0, 0, 0],
            "Enchantment",
            "Creature",
        ),
    ] {
        let source = named_row(name);
        pinned_revision(&source);
        let color = if cost[0] == 1 { "W" } else { "U" };
        let cards = deck(&[
            source.clone(),
            row("Synthetic union hit", "{99}", matching, ""),
            row("Synthetic union near miss", "{99}", wrong, ""),
            row(
                "Synthetic union back-face near miss",
                "{99} // {99}",
                &format!("{wrong} // {matching}"),
                "",
            ),
            row(
                "Pinned tutor funding",
                "{99}",
                "Artifact",
                &format!("{{T}}: Add {{{color}}}."),
            ),
        ]);
        for has_hit in [false, true] {
            let mut game = state(vec![0], if has_hit { vec![1, 2, 3] } else { vec![2, 3] });
            put_permanents(&cards, &mut game, &[4]);
            let mut expected = vec![CardIdx(2), CardIdx(3)];
            let mut expected_rng = game.library_rng.clone();
            crate::deck::simulator::rng::shuffle(&mut expected, &mut expected_rng);
            if has_hit {
                expected.push(CardIdx(1));
            }
            let (game, log, json) = executed_report(&cards, game, 1, vec![]);
            assert_mana_report(&log, &json, &[1.0], &[1.0]);
            assert_eq!(log.mana_remaining, [0.0]);
            assert_eq!(game.graveyard, [CardIdx(0)]);
            assert_eq!(game.library, expected);
            assert!(game.hand.is_empty() && game.exile.is_empty());
            assert_eq!(log.cards_seen, [u32::from(has_hit)]);
            assert_eq!(
                json["velocity"]["avg_cards_seen"]["1"],
                f64::from(u32::from(has_hit))
            );
            assert_eq!(
                json["velocity"]["library_remaining_by_turn"]["1"],
                expected.len() as f64
            );
            assert_library_rng(&game.library_rng, &expected_rng);
            assert_report_clause(&json, &source, "Search your library", "compiler_only", None);
        }
    }
}

/// Supported types and mana-value relations select real front-face hits only.
pub(super) fn assert_search_types_relations_report() {
    assert_front_type_reports();
    assert_mana_value_reports();
    assert_rejected_selector_reports();
    assert_pinned_union_reports();
}

/// Check relation reports even when an independent front-type regression fails.
#[test]
fn synthetic_mana_value_relations_execute_and_report() {
    assert_mana_value_reports();
}

/// Check complete rejection reports apart from supported selector execution.
#[test]
fn synthetic_rejected_search_selectors_execute_and_report() {
    assert_rejected_selector_reports();
}

/// Keep pinned union placement checks independent of synthetic type regressions.
#[test]
fn pinned_search_unions_execute_and_report() {
    assert_pinned_union_reports();
}
