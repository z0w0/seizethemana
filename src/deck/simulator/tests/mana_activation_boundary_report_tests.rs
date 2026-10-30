//! Synthetic activation boundary evidence. No named card receives a certificate here.

use super::*;
use crate::deck::simulator::game_effects::activation::{
    pick_best_activation, resolve_activation, spend_leftover,
};
use crate::deck::simulator::game_run::mana::{
    tap_dorks_for_mana, tap_for_nonspell_payment, tap_new_rocks,
};
use crate::deck::simulator::model::SimAbilityKind;

/// Isolate a paid untap-symbol cast from later combat taps by removing defenders.
fn untap_scenario(haste: bool, funded: bool) -> (SimDeck, GameState) {
    let text = format!(
        "{}This creature enters tapped.\n{{1}}, {{Q}}, Pay 1 life: Add {{C}}{{C}}.",
        if haste { "Haste\n" } else { "" }
    );
    let mut source = row("Synthetic untap-symbol creature", "{0}", "Creature", &text);
    source.keywords = if haste { "[\"Haste\"]" } else { "[]" }.into();
    let cards = deck(&[
        source,
        row("Untap output payment", "{2}", "Sorcery", ""),
        row("Untap funding", "{99}", "Artifact", "{T}: Add {C}."),
    ]);
    let mut game = state(vec![0, 1], vec![]);
    for seat in &mut game.opponents.seats {
        seat.active = false;
    }
    if funded {
        put_permanents(&cards, &mut game, &[2]);
    }
    (cards, game)
}

/// Ready-source and stale-action controls enforce sickness and the full untap payment.
fn assert_untap_actions() {
    for sick in [false, true] {
        for haste in [false, true] {
            for funded in [false, true] {
                let (cards, _) = untap_scenario(haste, funded);
                let mut game = main_phase_state(vec![1], vec![]);
                put_permanents(&cards, &mut game, &[0]);
                game.battlefield[0].tapped = true;
                game.battlefield[0].summoning_sick = sick;
                game.battlefield[0].entered_turn = usize::from(sick);
                let mut pool = ManaPool {
                    colorless: u32::from(funded),
                    ..ManaPool::default()
                };
                let before = game.clone();
                let before_pool = pool.clone();
                let legal = funded && (!sick || haste);
                let activation = pick_best_activation(&cards, &game, &pool, 1);
                assert_eq!(
                    activation.is_some(),
                    legal,
                    "sick={sick} haste={haste} funded={funded}"
                );
                if let Some(activation) = activation {
                    assert!(
                        activation
                            .ability
                            .activation
                            .as_ref()
                            .expect("printed costs")
                            .untaps_source()
                    );
                    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
                    assert_eq!(pool.colorless, 2);
                    assert_eq!(game.life, 19);
                    assert_eq!(game.life_paid, 1);
                    assert!(!game.battlefield[0].tapped);
                    assert_eq!(game.battlefield[0].uid, before.battlefield[0].uid);
                    let after = game.clone();
                    let after_pool = pool.clone();
                    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);
                    assert_resources_unchanged(&game, &after);
                    assert_eq!(pool, after_pool);
                    assert_eq!(cast_action(&cards, &mut game, &mut pool), 2.0);
                    assert_eq!(game.graveyard, [CardIdx(1)]);
                    assert_eq!(pool.total(), 0);
                } else {
                    let ability = cards.cards[0]
                        .unlocked_abilities(0)
                        .find(|ability| ability.kind == SimAbilityKind::ManaActivated)
                        .expect("the complete paid mode lowers");
                    assert!(
                        ability
                            .activation
                            .as_ref()
                            .expect("printed costs")
                            .untaps_source()
                    );
                    let source = game.battlefield[0].clone();
                    assert!(!activate_mana_mode(
                        &cards, &mut game, &source, &mut pool, 1
                    ));
                    let mut ready = before.clone();
                    ready.battlefield[0].summoning_sick = false;
                    ready.battlefield[0].entered_turn = 0;
                    let stale = pick_best_activation(
                        &cards,
                        &ready,
                        &ManaPool {
                            colorless: 1,
                            ..ManaPool::default()
                        },
                        1,
                    )
                    .expect("a mature funded source supplies a real action");
                    resolve_activation(&cards, &mut game, &mut pool, 1, &stale);
                    assert_resources_unchanged(&game, &before);
                    assert_eq!(pool, before_pool);
                    assert_eq!(cast_action(&cards, &mut game, &mut pool), 0.0);
                    assert_resources_unchanged(&game, &before);
                }
                assert_library_rng(&game.library_rng, &before.library_rng);
            }
        }
    }
}

/// Ordinary casts keep fresh untap-symbol creatures sick unless haste permits payment.
fn assert_untap_turn_reports() {
    for haste in [false, true] {
        for funded in [false, true] {
            let (cards, game) = untap_scenario(haste, funded);
            let rng = game.library_rng.clone();
            let (game, log, json) = executed_report(&cards, game, 1, vec![]);
            let legal = haste && funded;
            assert_mana_report(
                &log,
                &json,
                &[f64::from(u32::from(funded))],
                &[if legal { 2.0 } else { 0.0 }],
            );
            assert_eq!(
                log.mana_remaining,
                [if legal {
                    0.0
                } else {
                    f64::from(u32::from(funded))
                }]
            );
            let source = game
                .battlefield
                .iter()
                .find(|source| source.card.deck_idx() == Some(CardIdx(0)))
                .expect("cast physical creature remains");
            assert_eq!(source.entered_turn, 1);
            assert_eq!(source.summoning_sick, !haste);
            assert_eq!(source.tapped, !legal);
            assert!(!game.attacked_this_turn);
            assert_eq!(game.life_paid, u32::from(legal));
            assert_eq!(json["draw"]["avg_life_paid"], f64::from(u32::from(legal)));
            assert_eq!(game.life, if legal { 19 } else { 20 });
            assert_eq!(game.hand, if legal { vec![] } else { vec![CardIdx(1)] });
            assert_eq!(
                game.graveyard,
                if legal { vec![CardIdx(1)] } else { vec![] }
            );
            assert!(game.library.is_empty() && game.exile.is_empty());
            assert!(!game.infinite_mana_suspected);
            assert_library_rng(&game.library_rng, &rng);
            assert!(
                json["assumptions"]
                    .as_array()
                    .expect("assumptions")
                    .iter()
                    .any(|item| item
                        .as_str()
                        .is_some_and(|text| text.contains("untap-symbol costs without haste")))
            );
        }
    }
}

/// A fresh creature cannot pay its own untap-symbol cost without haste.
pub(super) fn assert_synthetic_untap_sickness_report() {
    assert_untap_actions();
    assert_untap_turn_reports();
}

/// Give an unknown-cost source enough visible resources to expose a partial payment.
fn unknown_cost_scenario(payment: &str) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic mandatory cost",
            "{99}",
            "Artifact",
            &format!("{{1}}, {{T}}, {payment}: Add {{C}}{{C}}{{C}}{{C}}."),
        ),
        row("Unknown mode payment", "{4}", "Sorcery", ""),
        row("Retained physical hand", "{99}", "Sorcery", ""),
        row("Retained physical library", "{99}", "Sorcery", ""),
        row("Retained physical graveyard", "{99}", "Sorcery", ""),
        row("Retained physical exile", "{99}", "Sorcery", ""),
        row(
            "External activation funding",
            "{99}",
            "Artifact",
            "{T}: Add {C}.",
        ),
    ]);
    let mut game = main_phase_state(vec![1, 2], vec![3]);
    game.graveyard.push(CardIdx(4));
    game.exile.push(CardIdx(5));
    put_permanents(&cards, &mut game, &[6, 0]);
    (cards, game)
}

/// Unknown required costs block complete native modes and retain a report limitation.
pub(super) fn assert_unknown_activation_cost_report() {
    for payment in [
        "Discard a card at random",
        "Exile a card from your hand",
        "Pay X life",
    ] {
        let (cards, mut game) = unknown_cost_scenario(payment);
        let before = game.clone();
        let mut pool = ManaPool {
            colorless: 1,
            ..ManaPool::default()
        };
        let before_pool = pool.clone();
        let source = game.battlefield[1].clone();
        assert!(!activate_mana_mode(
            &cards, &mut game, &source, &mut pool, 1
        ));
        assert_resources_unchanged(&game, &before);
        assert_eq!(pool, before_pool);
        assert_eq!(cast_action(&cards, &mut game, &mut pool), 0.0);
        assert_resources_unchanged(&game, &before);
        assert_eq!(pool, before_pool);

        let (cards, game) = unknown_cost_scenario(payment);
        let rng = game.library_rng.clone();
        let (game, log, json) = executed_report(&cards, game, 1, vec![]);
        assert_mana_report(&log, &json, &[1.0], &[0.0]);
        assert_eq!(log.mana_remaining, [1.0]);
        assert_eq!(game.hand, [CardIdx(1), CardIdx(2)]);
        assert_eq!(game.library, [CardIdx(3)]);
        assert_eq!(game.graveyard, [CardIdx(4)]);
        assert_eq!(game.exile, [CardIdx(5)]);
        assert!(game.battlefield[0].tapped);
        assert!(!game.battlefield[1].tapped);
        assert_eq!(game.battlefield[1].card.deck_idx(), Some(CardIdx(0)));
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert_eq!(json["draw"]["avg_life_paid"], 0.0);
        assert_library_rng(&game.library_rng, &rng);
        let record = json["card_support"]
            .as_array()
            .expect("compiler records")
            .iter()
            .find(|record| record["name"] == "Synthetic mandatory cost")
            .expect("source report");
        assert_eq!(record["abilities"][0]["status"], "unsupported");
        assert_eq!(
            record["abilities"][0]["source"],
            cards.cards[0].support[0].source
        );
        assert!(
            record["abilities"][0]["reason"]
                .as_str()
                .expect("limitation reason")
                .contains("required cost")
        );
    }
    let (cards, game) = unknown_cost_scenario("Pay 1 life");
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_mana_report(&log, &json, &[4.0], &[4.0]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!(game.graveyard, [CardIdx(4), CardIdx(1)]);
    assert_eq!(game.library, [CardIdx(3)]);
    assert_eq!(game.exile, [CardIdx(5)]);
    assert_eq!(game.life_paid, 1);
    assert_eq!(json["draw"]["avg_life_paid"], 1.0);
    assert_eq!(game.life, 19);
    assert!(game.battlefield.iter().all(|source| source.tapped));
    assert_library_rng(&game.library_rng, &rng);
}

/// Build a sorcery-only hand whose land receives a separate unrestricted tap ability.
fn granted_land_scenario(text: &str, granted: bool) -> (SimDeck, GameState) {
    let cards = deck(&[
        row(
            "Synthetic grant admission source",
            "{99}",
            "Artifact Land",
            text,
        ),
        row(
            "Synthetic admission land grant",
            "{99}",
            "Enchantment",
            "Lands you control have \"{T}: Add one mana of any color.\"",
        ),
        row("Synthetic admission sorcery", "{U}", "Sorcery", ""),
        row("Retained admission library", "{99}", "Sorcery", ""),
    ]);
    let mut game = main_phase_state(vec![2, 2], vec![3]);
    game.life = 1;
    for seat in &mut game.opponents.seats {
        seat.active = false;
    }
    game.opponents.config.land_colors = Some(vec![String::new()]);
    put_permanents(&cards, &mut game, &[0]);
    if granted {
        put_permanents(&cards, &mut game, &[1]);
    }
    (cards, game)
}

/// Exercise each production admission route with the same physical source.
fn collect_granted_mana(
    cards: &SimDeck,
    game: &mut GameState,
    pool: &mut ManaPool,
    route: &str,
) -> bool {
    let source = game.battlefield[0].clone();
    let before = pool.total();
    match route {
        "pool" => *pool = build_pool(cards, game, 1),
        "native" => {
            return activate_mana_mode(cards, game, &source, pool, 1);
        }
        "nonspell" => {
            return tap_for_nonspell_payment(cards, game, &source, pool, 1);
        }
        "dork" => tap_dorks_for_mana(cards, game, pool, 1),
        "new" => tap_new_rocks(cards, game, pool, 1),
        "leftover" => spend_leftover(cards, game, pool, 1),
        _ => panic!("unknown admission route"),
    }
    pool.total() > before
}

/// A chosen grant pays only its tap cost, even when the native ability requires more.
fn assert_granted_land_action(text: &str, route: &str) {
    let (cards, mut game) = granted_land_scenario(text, true);
    if route == "nonspell" {
        game.hand.clear();
    }
    if route == "new" {
        game.battlefield[0].entered_turn = 1;
    }
    let before = game.clone();
    let stale_source = game.battlefield[0].clone();
    let mut pool = ManaPool::default();
    assert!(
        collect_granted_mana(&cards, &mut game, &mut pool, route),
        "{route}: {text}"
    );
    assert_eq!(
        pool,
        ManaPool {
            flexible: 1,
            ..ManaPool::default()
        }
    );
    let mut expected = before.clone();
    expected.battlefield[0].tapped = true;
    assert_resources_unchanged(&game, &expected);
    if route == "nonspell" {
        assert!(crate::deck::simulator::game_mana::pay_cost(
            &parse_cost("{1}"),
            &mut pool
        ));
        assert_resources_unchanged(&game, &expected);
    } else {
        assert_eq!(cast_action(&cards, &mut game, &mut pool), 1.0);
        assert_eq!(game.hand, [CardIdx(2)]);
        assert_eq!(game.graveyard, [CardIdx(2)]);
        assert_eq!(game.battlefield.len(), 2);
        assert_eq!(game.battlefield[0].uid, stale_source.uid);
        assert_eq!(game.battlefield[0].counters, stale_source.counters);
        assert_eq!(game.life, 1);
        assert_eq!(game.life_paid, 0);
        assert_library_rng(&game.library_rng, &before.library_rng);
    }
    assert_eq!(pool, ManaPool::default());
    let after = game.clone();
    assert!(!activate_mana_mode(
        &cards,
        &mut game,
        &stale_source,
        &mut pool,
        1
    ));
    assert!(!tap_for_nonspell_payment(
        &cards,
        &mut game,
        &stale_source,
        &mut pool,
        1
    ));
    assert_eq!(build_pool(&cards, &mut game, 1), ManaPool::default());
    tap_new_rocks(&cards, &mut game, &mut pool, 1);
    spend_leftover(&cards, &mut game, &mut pool, 1);
    assert_eq!(cast_action(&cards, &mut game, &mut pool), 0.0);
    assert_resources_unchanged(&game, &after);
    assert_eq!(pool, ManaPool::default());
}

/// Reports distinguish an independent grant from native costs, conditions, and source sacrifice.
fn assert_granted_land_report(text: &str, granted: bool) {
    let (cards, game) = granted_land_scenario(text, granted);
    if text.contains("three or more artifacts") {
        assert!(cards.cards[0].flags.requires_metalcraft);
    }
    if text.is_empty() {
        assert!(cards.cards[0].tap.is_none());
    }
    let rng = game.library_rng.clone();
    let sacrifices_native = !granted && text.contains("Sacrifice this land");
    let late_native_mana = !granted && text.ends_with("creature spells.");
    let native_pip_only = !granted && text.contains("generic mana costs");
    let available = f64::from(u32::from(granted || sacrifices_native || native_pip_only));
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_mana_report(&log, &json, &[available], &[f64::from(u32::from(granted))]);
    assert_eq!(
        log.mana_remaining,
        [f64::from(u32::from(
            sacrifices_native || late_native_mana || native_pip_only
        ))]
    );
    assert_eq!(
        game.hand,
        if granted {
            vec![CardIdx(2)]
        } else {
            vec![CardIdx(2), CardIdx(2)]
        }
    );
    assert_eq!(
        game.graveyard,
        if granted {
            vec![CardIdx(2)]
        } else if sacrifices_native {
            vec![CardIdx(0)]
        } else {
            vec![]
        }
    );
    assert_eq!(
        game.battlefield.len(),
        if granted {
            2
        } else {
            usize::from(!sacrifices_native)
        }
    );
    if !sacrifices_native {
        assert_eq!(game.battlefield[0].uid, 1);
        assert_eq!(
            game.battlefield[0].tapped,
            granted || late_native_mana || native_pip_only
        );
        assert_eq!(game.battlefield[0].counters.charge, 0);
    }
    assert_eq!(game.life, 1);
    assert_eq!(game.life_paid, 0);
    assert_eq!(json["draw"]["avg_life_paid"], 0.0);
    assert_eq!(game.player_counters.energy, 0);
    assert_eq!(game.library, [CardIdx(3)]);
    assert!(game.exile.is_empty());
    assert_library_rng(&game.library_rng, &rng);
}

/// Admission uses the granted ability rather than any native payment or requirement.
pub(super) fn assert_granted_admission_routes() {
    for text in [
        "{T}: Add {G}. Spend this mana only to cast creature spells.",
        "{T}: Add {G}. This mana can't be spent to pay generic mana costs.",
        "{2}, {T}: Add {G}.",
        "{T}, Pay 2 life: Add {G}.",
        "{T}, Sacrifice this land: Add {G}.",
        "{T}, Sacrifice another creature: Add {G}.",
        "{T}, Remove a charge counter from this artifact: Add {G}.",
        "{T}, Pay {E}: Add {G}.",
        "{T}: Add one mana of any color that a land an opponent controls could produce.",
        "{T}: Add one mana of any color. Activate only if you control three or more artifacts.",
        "{T}, Discard a card at random: Add {G}.",
        "",
    ] {
        for route in ["pool", "native", "nonspell", "new", "leftover"] {
            assert_granted_land_action(text, route);
        }
        for granted in [false, true] {
            assert_granted_land_report(text, granted);
        }
    }
    assert_granted_creature_routes();
    assert_leftover_grant_action_limit();
}

/// Give a creature a supported tap grant and optional exact nonland tap rewards.
fn granted_creature_scenario(text: &str, haste: bool, bonus: bool) -> (SimDeck, GameState) {
    let mut source = row("Synthetic grant creature", "{0}", "Artifact Creature", text);
    if haste {
        source.oracle_text = format!("Haste\n{text}");
        source.keywords = "[\"Haste\"]".into();
    }
    let cards = deck(&[
        source,
        row(
            "Synthetic creature mana grant",
            "{99}",
            "Enchantment",
            "Creatures you control have \"{T}: Add one mana of any color.\"",
        ),
        row(
            "Synthetic creature-funded sorcery",
            if bonus { "{U}{U}" } else { "{U}" },
            "Sorcery",
            "",
        ),
        row(
            "Synthetic granted mana bonus",
            "{99}",
            "Enchantment",
            "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        ),
        row(
            "Synthetic granted tap reward",
            "{99}",
            "Enchantment",
            "Whenever you tap a nonland permanent for mana, you gain 1 life.",
        ),
    ]);
    let mut game = main_phase_state(vec![2, 2], vec![]);
    game.life = 1;
    for seat in &mut game.opponents.seats {
        seat.active = false;
    }
    put_permanents(&cards, &mut game, &[0, 1]);
    if bonus {
        put_permanents(&cards, &mut game, &[3, 4]);
    }
    (cards, game)
}

/// Creature grant admission enforces readiness and visibility across spell and nonspell routes.
fn assert_granted_creature_actions(text: &str, haste: bool, bonus: bool) {
    let (cards, initial) = granted_creature_scenario(text, haste, bonus);
    assert!(cards.cards[1].flags.grant.is_some());
    for sick in [false, true] {
        for hidden in [false, true] {
            for hidden_grant in [false, true] {
                for route in ["native", "nonspell", "dork", "leftover"] {
                    let mut game = initial.clone();
                    game.battlefield[0].summoning_sick = sick;
                    game.battlefield[0].entered_turn = usize::from(sick);
                    game.battlefield[0].face_down = hidden;
                    game.battlefield[1].face_down = hidden_grant;
                    let before = game.clone();
                    let mut pool = ManaPool::default();
                    let legal = !hidden_grant && (!sick || haste && !hidden);
                    assert_eq!(
                        collect_granted_mana(&cards, &mut game, &mut pool, route),
                        legal,
                        "{route} sick={sick} haste={haste} hidden={hidden} hidden_grant={hidden_grant}: {text}"
                    );
                    if legal {
                        assert_granted_creature_payment(&cards, &mut game, &mut pool, route, bonus);
                        assert_eq!(game.battlefield[0].uid, before.battlefield[0].uid);
                        assert_eq!(game.battlefield[0].counters, before.battlefield[0].counters);
                        assert_library_rng(&game.library_rng, &before.library_rng);
                    } else {
                        assert_resources_unchanged(&game, &before);
                        assert_eq!(pool, ManaPool::default());
                    }
                }
            }
        }
    }
}

/// One nonland tap creates its mana and rewards once, then cannot fund another tap.
fn assert_granted_creature_payment(
    cards: &SimDeck,
    game: &mut GameState,
    pool: &mut ManaPool,
    route: &str,
    bonus: bool,
) {
    let output = if bonus { 2 } else { 1 };
    assert_eq!(pool.total(), output);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.life_gained, u32::from(bonus));
    assert_eq!(game.life, 1 + i32::from(bonus));
    if bonus {
        assert_eq!(pool.lots.len(), 2);
        assert!(
            pool.lots
                .iter()
                .all(|lot| lot.types == 2 && lot.amount == 1 && lot.restriction.is_none())
        );
    } else {
        assert_eq!(pool.flexible, 1);
    }
    if route == "nonspell" {
        assert!(crate::deck::simulator::game_mana::pay_cost(
            &parse_cost("{1}"),
            pool
        ));
        assert_eq!(pool.total(), output - 1);
        assert_eq!(game.hand, [CardIdx(2), CardIdx(2)]);
        assert!(game.graveyard.is_empty());
    } else {
        assert_eq!(cast_action(cards, game, pool), f64::from(output));
        assert_eq!(pool.total(), 0);
        assert_eq!(game.hand, [CardIdx(2)]);
        assert_eq!(game.graveyard, [CardIdx(2)]);
    }
    let before = game.clone();
    let before_pool = pool.clone();
    let source = game.battlefield[0].clone();
    assert!(!activate_mana_mode(cards, game, &source, pool, 1));
    assert!(!tap_for_nonspell_payment(cards, game, &source, pool, 1));
    tap_dorks_for_mana(cards, game, pool, 1);
    spend_leftover(cards, game, pool, 1);
    assert_resources_unchanged(game, &before);
    assert_eq!(*pool, before_pool);
}

/// Granted taps obey the action limit without charging a probe to real resources or randomness.
fn assert_leftover_grant_action_limit() {
    use crate::deck::simulator::game::{GameCompletion, GamePhase, IncompleteReason};
    use crate::deck::simulator::model::MAX_LOOP_PASSES;

    let cards = deck(&[
        row("Synthetic finite grant source", "", "Land", ""),
        row(
            "Synthetic finite tap grant",
            "{99}",
            "Enchantment",
            "Lands you control have \"{T}: Add one mana of any color.\"",
        ),
        row("Retained finite-grant demand", "{1000}", "Sorcery", ""),
    ]);
    for source_count in [MAX_LOOP_PASSES, MAX_LOOP_PASSES + 1] {
        let mut game = main_phase_state(vec![2], vec![]);
        put_permanents(&cards, &mut game, &[1]);
        for _ in 0..source_count {
            put_permanents(&cards, &mut game, &[0]);
        }
        let rng = game.library_rng.clone();
        let mut pool = ManaPool::default();
        spend_leftover(&cards, &mut game, &mut pool, 1);
        assert_eq!(
            pool,
            ManaPool {
                flexible: MAX_LOOP_PASSES,
                ..ManaPool::default()
            }
        );
        assert!(!game.battlefield[0].tapped);
        assert_eq!(
            game.battlefield
                .iter()
                .filter(|source| source.tapped)
                .count(),
            MAX_LOOP_PASSES as usize
        );
        assert_eq!(
            game.battlefield
                .last()
                .expect("last physical source")
                .tapped,
            source_count == MAX_LOOP_PASSES
        );
        assert_eq!(game.battlefield.len(), source_count as usize + 1);
        assert_eq!(game.hand, [CardIdx(2)]);
        assert!(game.library.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!(game.life, 20);
        assert_eq!(game.life_paid, 0);
        assert!(!game.infinite_mana_suspected);
        if source_count == MAX_LOOP_PASSES {
            assert!(game.completion.is_none());
        } else {
            assert_eq!(
                game.completion,
                Some(GameCompletion::Incomplete {
                    reason: IncompleteReason::ActivationPass {
                        configured_limit: MAX_LOOP_PASSES,
                        remaining_work: 1
                    },
                    turn: 1,
                    phase: GamePhase::Main,
                })
            );
        }
        assert_library_rng(&game.library_rng, &rng);
    }
}

/// A real entry-turn cast cannot use the granted creature tap without haste.
fn assert_granted_creature_report(text: &str, haste: bool, bonus: bool) {
    let (cards, mut game) = granted_creature_scenario(text, haste, bonus);
    game.battlefield.remove(0);
    game.hand.insert(0, CardIdx(0));
    let expected_uid = game.next_uid + 1;
    let rng = game.library_rng.clone();
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    let output = if bonus { 2.0 } else { 1.0 };
    assert_mana_report(&log, &json, &[0.0], &[if haste { output } else { 0.0 }]);
    assert_eq!(log.mana_remaining, [0.0]);
    assert_eq!(
        game.hand,
        if haste {
            vec![CardIdx(2)]
        } else {
            vec![CardIdx(2), CardIdx(2)]
        }
    );
    assert_eq!(
        game.graveyard,
        if haste { vec![CardIdx(2)] } else { vec![] }
    );
    let source = game.battlefield.last().expect("cast creature remains");
    assert_eq!(source.uid, expected_uid);
    assert_eq!(source.card.deck_idx(), Some(CardIdx(0)));
    assert_eq!(source.entered_turn, 1);
    assert_eq!(source.tapped, haste);
    assert_eq!(game.life_paid, 0);
    assert_eq!(game.life_gained, u32::from(haste && bonus));
    assert_eq!(json["draw"]["avg_life_paid"], 0.0);
    assert_eq!(
        json["draw"]["avg_life_gained"],
        f64::from(u32::from(haste && bonus))
    );
    assert!(game.library.is_empty() && game.exile.is_empty());
    assert_library_rng(&game.library_rng, &rng);
}

/// Nativeless and extra-cost creatures share the grant's readiness and tap rules.
fn assert_granted_creature_routes() {
    for text in ["", "{T}, Pay 2 life: Add {G}{G}{G}."] {
        for haste in [false, true] {
            for bonus in [false, true] {
                assert_granted_creature_actions(text, haste, bonus);
                assert_granted_creature_report(text, haste, bonus);
            }
        }
    }
    assert_hidden_grant_bonus_report();
    assert_face_down_grant_recipient_report();
}

/// A hidden creature can use an external grant, but cannot expose its own haste or mana.
fn assert_face_down_grant_recipient_report() {
    for fresh in [false, true] {
        for hidden_provider in [false, true] {
            let (cards, mut game) =
                granted_creature_scenario("{T}, Pay 2 life: Add {G}{G}{G}.", true, false);
            game.battlefield[0].face_down = true;
            game.battlefield[0].entered_turn = usize::from(fresh);
            game.battlefield[0].summoning_sick = fresh;
            game.battlefield[1].face_down = hidden_provider;
            let rng = game.library_rng.clone();
            let legal = !fresh && !hidden_provider;
            let case = format!(
                "face-down grant recipient: fresh={fresh}, hidden_provider={hidden_provider}"
            );
            let mut initial = game.clone();
            let initial_pool = build_pool(&cards, &mut initial, 1);
            assert_eq!(
                initial_pool,
                ManaPool {
                    flexible: u32::from(legal),
                    ..ManaPool::default()
                },
                "{case}: initial pool comes only from the separate grant"
            );
            assert_eq!(
                initial.battlefield[0].tapped, legal,
                "{case}: initial recipient tap"
            );
            assert!(
                initial.battlefield[0].face_down,
                "{case}: native abilities remain hidden"
            );
            assert_eq!(initial.battlefield[1].face_down, hidden_provider, "{case}");
            assert_eq!(initial.life, 1, "{case}: native life cost is not paid");
            assert_eq!(initial.life_paid, 0, "{case}");
            assert_eq!(initial.hand, game.hand, "{case}: planning does not cast");
            assert!(initial.graveyard.is_empty() && initial.exile.is_empty());
            let (game, log, json) = executed_report(&cards, game, 1, vec![]);
            let available = f64::from(u32::from(legal));
            assert_eq!(log.mana_available, [available], "{case}");
            assert_mana_report(&log, &json, &[available], &[f64::from(u32::from(legal))]);
            assert_eq!(log.mana_remaining, [0.0]);
            assert_eq!(
                game.hand,
                if legal {
                    vec![CardIdx(2)]
                } else {
                    vec![CardIdx(2), CardIdx(2)]
                }
            );
            assert_eq!(
                game.graveyard,
                if legal { vec![CardIdx(2)] } else { vec![] }
            );
            assert_eq!(game.battlefield[0].uid, 1);
            assert!(game.battlefield[0].face_down);
            assert_eq!(game.battlefield[0].tapped, legal);
            assert_eq!(game.battlefield[1].face_down, hidden_provider);
            assert_eq!(game.life, 1);
            assert_eq!(game.life_paid, 0);
            assert_eq!(json["draw"]["avg_life_paid"], 0.0);
            assert_library_rng(&game.library_rng, &rng);
        }
    }
}

/// A face-down bonus source cannot add its hidden mana while visible tap rewards still fire.
fn assert_hidden_grant_bonus_report() {
    let (cards, mut game) = granted_creature_scenario("", true, true);
    game.battlefield[2].face_down = true;
    game.battlefield[2].entered_turn = 1;
    game.battlefield[2].summoning_sick = true;
    let rng = game.library_rng.clone();
    let source = game.battlefield[0].clone();
    let mut action_game = game.clone();
    let mut pool = ManaPool::default();
    assert!(activate_mana_mode(
        &cards,
        &mut action_game,
        &source,
        &mut pool,
        1
    ));
    assert_eq!(
        pool,
        ManaPool {
            flexible: 1,
            ..ManaPool::default()
        }
    );
    assert_eq!(action_game.life_gained, 1);
    assert!(action_game.battlefield[2].face_down);
    let before = action_game.clone();
    assert!(!activate_mana_mode(
        &cards,
        &mut action_game,
        &source,
        &mut pool,
        1
    ));
    assert_resources_unchanged(&action_game, &before);
    let (game, log, json) = executed_report(&cards, game, 1, vec![]);
    assert_mana_report(&log, &json, &[0.0], &[0.0]);
    assert_eq!(log.mana_remaining, [1.0]);
    assert_eq!(game.hand, [CardIdx(2), CardIdx(2)]);
    assert!(game.graveyard.is_empty() && game.library.is_empty() && game.exile.is_empty());
    assert!(game.battlefield[0].tapped && game.battlefield[2].face_down);
    assert_eq!(game.life, 2);
    assert_eq!(game.life_gained, 1);
    assert_eq!(json["draw"]["avg_life_gained"], 1.0);
    assert_library_rng(&game.library_rng, &rng);
}
