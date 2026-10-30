//! Captured Behold costs, conditional rewards, land entries, and linked fetch results.

use super::cast_pass::{entry_is_tapped, new_entry_permanent, play_land, try_cast_from_hand};
use super::commander_test_support::named_row;
use super::game::{CardRef, GameState, ManaPool, new_perm_with, new_token_perm, take_uid};
use super::game_effects::activation::{pick_best_activation, resolve_activation};
use super::game_effects::behold::untap_search_result;
use super::model::{CardIdx, SearchCardType, SearchDestination, SimDeck, SimEffect};
use super::oracle_ast::{OracleAbility, OracleEffect};
use super::oracle_lower::parse_sim_card;
use super::oracle_parser::parse_oracle_card;
use super::scenario_test_support::{deck, row, state};
use crate::db::CardRow;
use rand::RngCore;

/// Keep the fetched basic land and every nonmatching card physically distinct.
fn passage_cards(source: CardRow) -> SimDeck {
    deck(&[
        source,
        row("Basic result", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Elf witness", "{99}", "Creature — Elf", ""),
        row("Nonbasic miss", "", "Land — Forest", ""),
        row("First marker", "{99}", "Sorcery", ""),
        row("Second marker", "{99}", "Artifact", ""),
    ])
}

/// Commit one selected source activation without running unrelated mana abilities.
fn activate_passage(cards: &SimDeck, game: &mut GameState) {
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(cards, game, &pool, 1)
        .expect("the complete untapped fetch activation can pay one life");
    assert!(matches!(
        activation.ability.effect_sequence(),
        [SimEffect::SearchThenBeholdUntap { .. }]
    ));
    resolve_activation(cards, game, &mut pool, 1, &activation);
    assert_eq!(pool.total(), 0);
}

/// Attempt one physical hand card without letting the sweep cast its Behold witness.
fn cast_one(cards: &SimDeck, game: &mut GameState, pool: &mut ManaPool) -> Option<u32> {
    try_cast_from_hand(
        cards,
        game,
        pool,
        1,
        CardIdx(0),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut [false; 5],
    )
}

/// Elven Passage keeps a complete search, a typed Elf quality, and all source costs.
#[test]
fn captured_passage_preserves_the_ast_and_runtime_compound() {
    for renamed in [false, true] {
        let mut source = named_row("Elven Passage");
        if renamed {
            source.name = "Renamed Behold fetch".into();
        }
        let oracle = parse_oracle_card(&source);
        let [OracleAbility::Activated(ability)] = oracle.abilities.as_slice() else {
            panic!("the captured land has one complete activation");
        };
        let [OracleEffect::SearchThenBeholdUntap { search, spec }] = ability.effects.as_slice()
        else {
            panic!("the optional untap must remain linked to its own search");
        };
        assert_eq!(search.card_type, Some(SearchCardType::BasicLand));
        assert!(search.basic_land_only && search.shuffle);
        assert_eq!(search.destination, SearchDestination::BattlefieldTapped);
        assert_eq!(spec.subtypes, ["elf"]);
        assert!(spec.card_types.is_empty());
        assert_eq!(spec.count, 1);
        let cards = passage_cards(source);
        assert!(cards.cards[0].is_fetch_land);
        assert_eq!(cards.cards[0].fetch_life_cost, 1);
        assert!(cards.cards[0].fetch_basic_only && cards.cards[0].fetch_enters_tapped);
        let witness = cards.cards[0]
            .fetch_behold_untap
            .as_ref()
            .expect("typed fetch quality");
        assert_eq!(witness.subtypes, ["elf"]);
        let runtime = cards.cards[0]
            .unlocked_abilities(0)
            .next()
            .expect("one executable activation");
        assert!(matches!(
            runtime.effect_sequence(),
            [SimEffect::SearchThenBeholdUntap { .. }]
        ));
        let costs = runtime
            .activation
            .as_ref()
            .expect("complete typed source costs");
        assert!(costs.taps_source() && costs.sacrifices_source());
        assert_eq!(costs.life_payment(), 1);
        assert_eq!(costs.mana_cost().total(), 0);
    }
}

/// A revealed Elf stays in hand while the searched land untaps after a real shuffle.
#[test]
fn passage_reveals_an_elf_without_consuming_it() {
    let cards = passage_cards(named_row("Elven Passage"));
    let mut game = state(vec![0, 2], vec![1, 3, 4, 5]);
    let random_before_entry = game.library_rng.clone().next_u64();
    assert!(play_land(&cards, &mut game, 1));
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(game.life_paid, 0);
    assert!(game.graveyard.is_empty());
    assert_eq!(game.library_rng.clone().next_u64(), random_before_entry);
    let mut expected_library = vec![CardIdx(3), CardIdx(4), CardIdx(5)];
    let mut expected_rng = game.library_rng.clone();
    super::rng::shuffle(&mut expected_library, &mut expected_rng);

    activate_passage(&cards, &mut game);

    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert!(game.exile.is_empty());
    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert!(!game.battlefield[0].tapped);
    assert_eq!((game.life, game.life_paid), (19, 1));
    assert_eq!(game.library, expected_library);
    assert_eq!(game.library_rng.clone().next_u64(), expected_rng.next_u64());
}

/// Behold can choose a tapped or summoning-sick Elf without tapping or sacrificing it.
#[test]
fn passage_uses_controlled_elves_without_spending_them() {
    for tapped in [false, true] {
        let cards = passage_cards(named_row("Elven Passage"));
        let mut game = state(vec![0], vec![1, 3, 4, 5]);
        let uid = take_uid(&mut game);
        game.battlefield
            .push(new_perm_with(uid, &cards, CardIdx(2), 1, tapped));
        assert!(play_land(&cards, &mut game, 1));

        activate_passage(&cards, &mut game);

        assert_eq!(game.battlefield.len(), 2);
        let elf = game
            .battlefield
            .iter()
            .find(|permanent| permanent.uid == uid)
            .expect("the Elf stays controlled");
        assert_eq!(elf.tapped, tapped);
        assert!(elf.summoning_sick);
        assert_eq!(elf.card, CardRef::Deck(CardIdx(2)));
        let land = game
            .battlefield
            .iter()
            .find(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
            .expect("the basic land was found");
        assert!(!land.tapped);
        assert_eq!(game.graveyard, [CardIdx(0)]);
        assert_eq!((game.life, game.life_paid), (19, 1));
    }
}

/// A controlled Elf commander qualifies even when its card is absent from the main deck.
#[test]
fn passage_can_behold_a_controlled_commander() {
    let mut cards = passage_cards(named_row("Elven Passage"));
    cards.commanders.push(parse_sim_card(&row(
        "Elf commander",
        "{99}",
        "Legendary Creature — Elf",
        "",
    )));
    let mut game = state(vec![0], vec![1, 3, 4, 5]);
    let uid = take_uid(&mut game);
    let mut commander = new_token_perm(uid, 1);
    commander.card = CardRef::Commander { slot: 0 };
    commander.tapped = true;
    game.battlefield.push(commander);
    assert!(play_land(&cards, &mut game, 1));

    activate_passage(&cards, &mut game);

    assert_eq!(game.battlefield[0].card, CardRef::Commander { slot: 0 });
    assert!(game.battlefield[0].tapped);
    assert!(!game.battlefield[1].tapped);
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(game.graveyard, [CardIdx(0)]);
}

/// An Elf in a hidden or departed zone does not untap the fetched basic land.
#[test]
fn passage_keeps_its_result_tapped_without_a_legal_witness() {
    let cards = passage_cards(named_row("Elven Passage"));
    for zone in ["absent", "library", "graveyard", "exile", "face_down"] {
        let mut game = state(vec![0], vec![1, 3, 4, 5]);
        match zone {
            "library" => game.library.push(CardIdx(2)),
            "graveyard" => game.graveyard.push(CardIdx(2)),
            "exile" => game.exile.push(CardIdx(2)),
            "face_down" => {
                let uid = take_uid(&mut game);
                let mut witness = new_perm_with(uid, &cards, CardIdx(2), 1, false);
                witness.face_down = true;
                game.battlefield.push(witness);
            }
            _ => {}
        }
        assert!(play_land(&cards, &mut game, 1));

        activate_passage(&cards, &mut game);

        let land = game
            .battlefield
            .iter()
            .find(|permanent| permanent.card == CardRef::Deck(CardIdx(1)))
            .expect("the basic land was found");
        assert!(land.tapped, "{zone}");
        assert!(game.graveyard.contains(&CardIdx(0)));
        assert_eq!((game.life, game.life_paid), (19, 1));
    }
}

/// A failed search still pays and shuffles, but cannot untap an earlier land.
#[test]
fn passage_with_no_basic_result_never_reuses_an_old_land() {
    let cards = passage_cards(named_row("Elven Passage"));
    let mut game = state(vec![0, 2], vec![3, 4, 5]);
    let uid = take_uid(&mut game);
    game.battlefield
        .push(new_perm_with(uid, &cards, CardIdx(1), 0, true));
    assert!(play_land(&cards, &mut game, 1));
    let mut expected_library = game.library.clone();
    let mut expected_rng = game.library_rng.clone();
    super::rng::shuffle(&mut expected_library, &mut expected_rng);

    activate_passage(&cards, &mut game);

    assert_eq!(game.battlefield.len(), 1);
    assert_eq!(game.battlefield[0].uid, uid);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!((game.life, game.life_paid), (19, 1));
    assert_eq!(game.library, expected_library);
    assert_eq!(game.library_rng.clone().next_u64(), expected_rng.next_u64());
}

/// A returned physical card is a new permanent, so an older search uid cannot untap it.
#[test]
fn passage_result_identity_does_not_follow_a_removed_permanent() {
    let cards = passage_cards(named_row("Elven Passage"));
    let mut game = state(vec![0, 2], vec![1, 3, 4, 5]);
    assert!(play_land(&cards, &mut game, 1));
    activate_passage(&cards, &mut game);
    let result = game.battlefield.remove(0);
    assert_eq!(result.card, CardRef::Deck(CardIdx(1)));
    let new_uid = take_uid(&mut game);
    game.battlefield
        .push(new_perm_with(new_uid, &cards, CardIdx(1), 1, true));
    let spec = cards.cards[0]
        .fetch_behold_untap
        .as_ref()
        .expect("compiled Elf quality");

    untap_search_result(&cards, &mut game, spec, Some(result.uid));

    assert_eq!(game.battlefield[0].uid, new_uid);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.hand, [CardIdx(2)]);
}

/// A nonland source can resolve the same compound through ordinary paid activation.
#[test]
fn ordinary_tap_activation_uses_the_same_linked_compound() {
    let passage = named_row("Elven Passage");
    let resolution = passage
        .oracle_text
        .split_once(": ")
        .expect("printed fetch separator")
        .1;
    let source = row(
        "Behold search artifact",
        "{2}",
        "Artifact",
        &format!("{{1}}, {{T}}: {resolution}"),
    );
    let cards = passage_cards(source);
    assert!(!cards.cards[0].is_fetch_land);
    let mut game = state(vec![2], vec![1, 3, 4, 5]);
    let uid = take_uid(&mut game);
    game.battlefield
        .push(new_perm_with(uid, &cards, CardIdx(0), 0, false));
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    let activation = pick_best_activation(&cards, &game, &pool, 1)
        .expect("the paid compound is a usable search activation");

    resolve_activation(&cards, &mut game, &mut pool, 1, &activation);

    assert_eq!(pool.total(), 0);
    assert!(game.battlefield[0].tapped);
    assert_eq!(game.battlefield[0].uid, uid);
    assert_eq!(game.battlefield[1].card, CardRef::Deck(CardIdx(1)));
    assert!(!game.battlefield[1].tapped);
    assert!(game.graveyard.is_empty());
    assert_eq!(game.hand, [CardIdx(2)]);
    assert_eq!((game.life, game.life_paid), (20, 0));
}

/// Sanctum's typed entry choice reaches runtime without a card-name exception.
#[test]
fn captured_sanctum_entry_keeps_its_typed_jace_quality() {
    let source = named_row("Theorist's Sanctum");
    let oracle = parse_oracle_card(&source);
    let spec = oracle
        .land
        .behold_untapped_entry
        .as_ref()
        .expect("typed Oracle entry quality");
    assert_eq!(spec.subtypes, ["jace"]);
    assert_eq!(spec.count, 1);
    let card = parse_sim_card(&source);
    let spec = card
        .behold_untapped_entry
        .as_ref()
        .expect("lowered entry quality");
    assert_eq!(spec.subtypes, ["jace"]);
    assert_eq!(spec.count, 1);
}

/// Entry can reveal a Jace or choose a tapped Jace without spending it or paying life.
#[test]
fn sanctum_entry_uses_hand_or_controlled_jace() {
    let mut witness = row("Jace witness", "{99}", "Planeswalker — Jace", "");
    witness.loyalty = Some("3".into());
    let cards = deck(&[named_row("Theorist's Sanctum"), witness]);
    for zone in ["absent", "hand", "battlefield"] {
        let mut game = state(vec![0], Vec::new());
        if zone == "hand" {
            game.hand.push(CardIdx(1));
        } else if zone == "battlefield" {
            let uid = take_uid(&mut game);
            game.battlefield
                .push(new_perm_with(uid, &cards, CardIdx(1), 0, true));
        }
        assert_eq!(
            entry_is_tapped(&cards, &game, CardIdx(0), true),
            zone == "absent"
        );
        assert!(play_land(&cards, &mut game, 1));
        let entered = game.battlefield.last().expect("the Sanctum entered");
        assert_eq!(entered.card, CardRef::Deck(CardIdx(0)));
        assert_eq!(entered.tapped, zone == "absent");
        if zone == "hand" {
            assert_eq!(game.hand, [CardIdx(1)]);
        } else if zone == "battlefield" {
            assert!(game.battlefield[0].tapped);
            assert_eq!(game.battlefield[0].counters.loyalty, 3);
        }
        assert_eq!((game.life, game.life_paid), (20, 0));
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
    }
}

/// Forced tapped placement overrides a successful Behold replacement without spending life.
#[test]
fn forced_sanctum_entry_stays_tapped() {
    let cards = deck(&[
        named_row("Theorist's Sanctum"),
        row("Jace witness", "{99}", "Planeswalker — Jace", ""),
    ]);
    let mut game = state(vec![0, 1], Vec::new());
    assert!(new_entry_permanent(&cards, &mut game, CardIdx(0), 1, true).tapped);
    assert_eq!(game.hand, [CardIdx(0), CardIdx(1)]);
    assert_eq!((game.life, game.life_paid), (20, 0));
}

/// The entering land cannot satisfy its own Behold quality while it is still in hand.
#[test]
fn behold_entry_excludes_the_entering_hand_card() {
    let mut source = named_row("Theorist's Sanctum");
    source.name = "Renamed land-quality entry".into();
    source.oracle_text = source.oracle_text.replace("a Jace", "a land");
    let cards = deck(&[source, row("Other land witness", "", "Land", "")]);
    let mut game = state(vec![0], Vec::new());
    assert!(entry_is_tapped(&cards, &game, CardIdx(0), true));
    assert!(new_entry_permanent(&cards, &mut game, CardIdx(0), 1, false).tapped);
    game.hand.push(CardIdx(1));
    assert!(!entry_is_tapped(&cards, &game, CardIdx(0), true));
    assert!(!new_entry_permanent(&cards, &mut game, CardIdx(0), 1, false).tapped);
    assert_eq!(game.hand, [CardIdx(0), CardIdx(1)]);
    assert_eq!((game.life, game.life_paid), (20, 0));
}

/// Unsupported counted entry choices keep the printed tapped fallback even with one Jace.
#[test]
fn counted_behold_entry_cannot_use_a_singular_fallback() {
    for (amount, count) in [("two", 2), ("three", 3)] {
        let mut source = named_row("Theorist's Sanctum");
        source.oracle_text = source
            .oracle_text
            .replace("behold a Jace", &format!("behold {amount} Jaces"));
        let oracle = parse_oracle_card(&source);
        assert_eq!(
            oracle
                .land
                .behold_untapped_entry
                .as_ref()
                .expect("the AST retains the count")
                .count,
            count
        );
        let cards = deck(&[
            source,
            row("Jace witness", "{99}", "Planeswalker — Jace", ""),
        ]);
        assert!(cards.cards[0].behold_untapped_entry.is_none());
        let mut game = state(vec![0, 1], Vec::new());
        assert!(entry_is_tapped(&cards, &game, CardIdx(0), true));

        assert!(play_land(&cards, &mut game, 1));

        assert!(game.battlefield[0].tapped);
        assert_eq!(game.hand, [CardIdx(1)]);
        assert_eq!((game.life, game.life_paid), (20, 0));
    }
}

/// Sarkhan's captured entry creates a Treasure only after a real Dragon Behold choice.
#[test]
fn captured_sarkhan_entry_keeps_its_reward_conditional() {
    let source = named_row("Sarkhan, Dragon Ascendant");
    let oracle = parse_oracle_card(&source);
    let Some(OracleAbility::Triggered(trigger)) = oracle.abilities.first() else {
        panic!("Sarkhan's first ability is the captured entry trigger");
    };
    assert!(matches!(
        trigger.effects.as_slice(),
        [OracleEffect::Behold { .. }]
    ));
    let cards = deck(&[
        source,
        row("Dragon witness", "{99}", "Creature — Dragon", ""),
    ]);
    for dragon in [false, true] {
        let mut game = state(if dragon { vec![0, 1] } else { vec![0] }, Vec::new());
        let mut pool = ManaPool {
            flexible: 2,
            ..Default::default()
        };

        assert_eq!(cast_one(&cards, &mut game, &mut pool), Some(2));

        assert_eq!(game.treasure_bank + pool.treasures, u32::from(dragon));
        assert_eq!(
            game.hand,
            if dragon { vec![CardIdx(1)] } else { Vec::new() }
        );
        assert_eq!(game.battlefield.len(), 1);
        assert_eq!((game.life, game.life_paid), (20, 0));
        assert!(game.graveyard.is_empty() && game.exile.is_empty());
    }
}

/// A pending Kithkin cannot reveal itself to avoid the printed extra generic payment.
#[test]
fn captured_behold_cost_excludes_the_casting_card() {
    let cards = deck(&[named_row("Kinsbaile Aspirant")]);
    let mut game = state(vec![0], Vec::new());
    let mut pool = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        ..Default::default()
    };

    assert_eq!(cast_one(&cards, &mut game, &mut pool), None);

    assert_eq!(game.hand, [CardIdx(0)]);
    assert_eq!(pool.total(), 1);
    assert!(game.battlefield.is_empty() && game.graveyard.is_empty());
    assert_eq!((game.life, game.life_paid), (20, 0));
}

/// A separate Kithkin reveals without leaving hand or changing the printed white cost.
#[test]
fn captured_behold_cost_accepts_a_separate_matching_hand_card() {
    let cards = deck(&[
        named_row("Kinsbaile Aspirant"),
        row("Kithkin witness", "{99}", "Creature — Kithkin", ""),
    ]);
    let mut game = state(vec![0, 1], Vec::new());
    let mut pool = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        ..Default::default()
    };

    assert_eq!(cast_one(&cards, &mut game, &mut pool), Some(1));

    assert_eq!(pool.total(), 0);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert!(game.graveyard.is_empty() && game.exile.is_empty());
    assert_eq!((game.life, game.life_paid), (20, 0));
}

/// Without a matching witness, the extra generic payment stays payable as real mana.
#[test]
fn captured_behold_cost_pays_the_generic_alternative() {
    let cards = deck(&[named_row("Kinsbaile Aspirant")]);
    let mut game = state(vec![0], Vec::new());
    let mut pool = ManaPool {
        fixed: [1, 0, 0, 0, 0],
        colorless: 2,
        ..Default::default()
    };

    assert_eq!(cast_one(&cards, &mut game, &mut pool), Some(3));

    assert_eq!(pool.total(), 0);
    assert!(game.hand.is_empty());
    assert_eq!(game.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!((game.life, game.life_paid), (20, 0));
}

/// An unsupported counted alternative cannot lose its cost and become a free cast.
#[test]
fn counted_behold_or_pay_cost_blocks_the_whole_cast() {
    for (amount, count) in [("two", 2), ("three", 3)] {
        let mut source = named_row("Kinsbaile Aspirant");
        source.oracle_text = source.oracle_text.replace(
            "behold a Kithkin or pay {2}",
            &format!("behold {amount} Kithkin or pay {{2}}"),
        );
        let oracle = parse_oracle_card(&source);
        assert_eq!(
            oracle
                .spell_data
                .behold_alternative
                .as_ref()
                .expect("the AST retains the counted cost")
                .spec
                .count,
            count
        );
        assert!(oracle.spell_data.unsupported_additional_cost);
        let cards = deck(&[
            source,
            row("Kithkin witness", "{99}", "Creature — Kithkin", ""),
        ]);
        assert!(cards.cards[0].spell_data.unsupported_additional_cost);
        assert!(cards.cards[0].spell_data.behold_alternative.is_none());
        let mut game = state(vec![0, 1], Vec::new());
        let mut pool = ManaPool {
            fixed: [1, 0, 0, 0, 0],
            colorless: 2,
            ..Default::default()
        };

        assert_eq!(cast_one(&cards, &mut game, &mut pool), None);

        assert_eq!(pool.total(), 3);
        assert_eq!(game.hand, [CardIdx(0), CardIdx(1)]);
        assert!(game.battlefield.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
        assert_eq!((game.life, game.life_paid), (20, 0));
    }
}

/// Unsupported champion exile costs cannot leave a free creature or linked trigger.
#[test]
fn captured_mandatory_behold_exile_cost_remains_blocked() {
    let cards = deck(&[
        named_row("Champions of the Perfect"),
        row("Elf witness", "{99}", "Creature — Elf", ""),
    ]);
    assert!(cards.cards[0].spell_data.unsupported_additional_cost);
    let mut game = state(vec![0, 1], Vec::new());
    let mut pool = ManaPool {
        flexible: 20,
        ..Default::default()
    };

    assert_eq!(cast_one(&cards, &mut game, &mut pool), None);

    assert_eq!(pool.total(), 20);
    assert_eq!(game.hand, [CardIdx(0), CardIdx(1)]);
    assert!(game.battlefield.is_empty() && game.graveyard.is_empty() && game.exile.is_empty());
}

/// Kindle cannot lose its three distinct Behold choices and retain only a mana replay cost.
#[test]
fn captured_counted_behold_flashback_has_no_partial_replay_cost() {
    let source = named_row("Kindle the Inner Flame");
    assert!(source.oracle_text.contains("Behold three Elementals"));
    let cards = deck(&[
        source,
        row(
            "Single Elemental witness",
            "{99}",
            "Creature — Elemental",
            "",
        ),
        row("Library marker", "{99}", "Sorcery", ""),
    ]);
    assert!(cards.cards[0].spell_data.own_flashback.is_none());
    let mut game = state(vec![1], vec![2]);
    game.graveyard.push(CardIdx(0));
    let mut pool = ManaPool {
        flexible: 20,
        ..Default::default()
    };

    let spent = super::turn_loop_tests::cast(&cards, &mut game, &mut pool);

    assert_eq!(spent, [0.0]);
    assert_eq!(pool.total(), 20);
    assert_eq!(game.hand, [CardIdx(1)]);
    assert_eq!(game.graveyard, [CardIdx(0)]);
    assert_eq!(game.library, [CardIdx(2)]);
    assert!(game.battlefield.is_empty() && game.exile.is_empty());
}
