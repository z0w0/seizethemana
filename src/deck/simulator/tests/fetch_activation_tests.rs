use super::*;
use crate::deck::simulator::cast_zone_tests::{deck, row, state};
use crate::deck::simulator::game::CardRef;
use crate::deck::simulator::game_commander::CommanderProfile;
use crate::deck::simulator::game_effects::activation::{pick_best_activation, resolve_activation};
use crate::deck::simulator::game_effects::spend_leftover;
use crate::deck::simulator::game_run::{TurnCensus, run_turn};
use crate::deck::simulator::model::{SimAbility, SimEffect};
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use rand::RngCore;

/// Read a fetch's executable search without using its metadata to grant effects.
fn search_activation(cards: &SimDeck, index: CardIdx) -> &SimAbility {
    cards[index]
        .unlocked_abilities(0)
        .find(|ability| {
            ability.kind.is_activated()
                && ability
                    .effect_sequence()
                    .iter()
                    .any(|effect| matches!(effect, SimEffect::Search(_)))
        })
        .expect("the complete supported Oracle search activation lowers")
}

/// A rejected activation leaves zones, resources, and the library stream intact.
fn assert_activation_unavailable(cards: &SimDeck, st: &mut GameState, pool: &mut ManaPool) {
    let zones = (
        st.library.clone(),
        st.hand.clone(),
        st.graveyard.clone(),
        st.exile.clone(),
    );
    let board = st
        .battlefield
        .iter()
        .map(|source| (source.uid, source.card, source.tapped, source.counters))
        .collect::<Vec<_>>();
    let resources = (
        st.life,
        st.life_paid,
        st.next_uid,
        st.seen,
        st.treasure_bank,
    );
    let expected_pool = pool.clone();
    let expected_random = st.library_rng.clone().next_u64();

    assert!(pick_best_activation(cards, st, pool, 1).is_none());
    spend_leftover(cards, st, pool, 1);

    assert_eq!(
        (&st.library, &st.hand, &st.graveyard, &st.exile),
        (&zones.0, &zones.1, &zones.2, &zones.3)
    );
    assert_eq!(
        st.battlefield
            .iter()
            .map(|source| (source.uid, source.card, source.tapped, source.counters))
            .collect::<Vec<_>>(),
        board
    );
    assert_eq!(
        (
            st.life,
            st.life_paid,
            st.next_uid,
            st.seen,
            st.treasure_bank
        ),
        resources
    );
    assert_eq!(*pool, expected_pool);
    assert_eq!(st.library_rng.clone().next_u64(), expected_random);
}

/// Evolving Wilds enters first and can later fetch a basic land without a type.
#[test]
fn evolving_wilds_activates_separately_and_can_find_wastes() {
    let cards = deck(&[
        row(
            "Evolving Wilds",
            "",
            "Land",
            "{T}, Sacrifice Evolving Wilds: Search your library for a basic land card, put it onto the battlefield tapped, then shuffle.",
        ),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Wastes", "", "Basic Land", "{T}: Add {C}."),
        row(
            "Nonbasic",
            "",
            "Land — Plains Mountain",
            "{T}: Add {W} or {R}.",
        ),
    ]);
    let ability = search_activation(&cards, CardIdx(0));
    let costs = ability.activation.as_ref().expect("typed fetch costs");
    assert!(costs.taps_source());
    assert!(costs.sacrifices_source());
    assert_eq!(costs.mana_cost(), parse_cost(""));
    assert_eq!(costs.life_payment(), 0);
    let SimEffect::Search(spec) = &ability.effect_sequence()[0] else {
        panic!("fetch resolution is the parsed search effect");
    };
    assert_eq!(spec.land_types, [false; 5]);
    assert!(spec.basic_land_only);
    let mut st = state(vec![0], vec![1, 2, 3]);
    let mut pool = ManaPool::default();
    let before_random = st.library_rng.clone().next_u64();

    assert!(play_land(&cards, &mut st, 1));

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert!(!st.battlefield[0].tapped);
    assert!(st.graveyard.is_empty());
    assert_eq!(st.library, [CardIdx(1), CardIdx(2), CardIdx(3)]);
    assert_eq!(st.library_rng.clone().next_u64(), before_random);
    let mut expected_library = vec![CardIdx(1), CardIdx(3)];
    let mut expected_rng = st.library_rng.clone();
    crate::deck::simulator::rng::shuffle(&mut expected_library, &mut expected_rng);

    spend_leftover(&cards, &mut st, &mut pool, 1);

    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(2)));
    assert!(st.battlefield[0].tapped);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.life, 20);
    assert_eq!(st.life_paid, 0);
    assert_eq!(pool, ManaPool::default());
    assert_eq!(st.library, expected_library);
    assert_eq!(st.library_rng.clone().next_u64(), expected_rng.next_u64());
}

/// Arid Mesa pays its printed life cost and cannot find an Island or Wastes.
#[test]
fn arid_mesa_searches_only_its_printed_land_types() {
    for target in [
        "Basic Land — Plains",
        "Basic Land — Mountain",
        "Land — Plains Mountain",
    ] {
        let cards = deck(&[
            row(
                "Arid Mesa",
                "",
                "Land",
                "{T}, Pay 1 life, Sacrifice Arid Mesa: Search your library for a Mountain or Plains card, put it onto the battlefield, then shuffle.",
            ),
            row("Matching land", "", target, ""),
            row("Island", "", "Basic Land — Island", "{T}: Add {U}."),
            row("Wastes", "", "Basic Land", "{T}: Add {C}."),
        ]);
        let mut st = state(vec![0], vec![1, 2, 3]);
        let mut pool = ManaPool::default();
        assert!(play_land(&cards, &mut st, 1));
        assert_eq!(st.life, 20);

        let activation = pick_best_activation(&cards, &st, &pool, 1)
            .expect("the untapped fetch can pay its life cost");
        resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

        assert_eq!(st.battlefield.len(), 1);
        assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
        assert!(!st.battlefield[0].tapped);
        assert_eq!(st.life, 19);
        assert_eq!(st.life_paid, 1);
        assert_eq!(st.graveyard, [CardIdx(0)]);
        assert!(st.library.contains(&CardIdx(2)));
        assert!(st.library.contains(&CardIdx(3)));
    }
}

/// Fetched shock lands pay their separate entry cost or enter tapped.
#[test]
fn fetched_shock_land_keeps_its_own_entry_life_payment() {
    let cards = deck(&[
        row(
            "Arid Mesa",
            "",
            "Land",
            "{T}, Pay 1 life, Sacrifice Arid Mesa: Search your library for a Mountain or Plains card, put it onto the battlefield, then shuffle.",
        ),
        row(
            "Sacred Foundry",
            "",
            "Land — Mountain Plains",
            "As Sacred Foundry enters, you may pay 2 life. If you don't, it enters tapped.\n{T}: Add {R} or {W}.",
        ),
    ]);
    assert_eq!(cards.cards[1].life_to_untap, 2);
    for (starting_life, final_life, paid, tapped) in [(20, 17, 3, false), (2, 1, 1, true)] {
        let mut st = state(vec![0], vec![1]);
        st.life = starting_life;
        assert!(play_land(&cards, &mut st, 1));
        let mut pool = ManaPool::default();
        let activation = pick_best_activation(&cards, &st, &pool, 1)
            .expect("the fetch's one-life activation cost is payable");

        resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

        assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
        assert_eq!(st.battlefield[0].tapped, tapped);
        assert_eq!(st.life, final_life);
        assert_eq!(st.life_paid, paid);
        assert_eq!(st.graveyard, [CardIdx(0)]);
    }
}

/// An instruction that forces tapped entry makes the optional shock payment useless.
#[test]
fn forced_tapped_fetch_does_not_pay_optional_shock_life() {
    let cards = deck(&[
        row(
            "Tapped-result fetch",
            "",
            "Land",
            "{T}, Pay 1 life, Sacrifice this land: Search your library for a Mountain or Plains card, put it onto the battlefield tapped, then shuffle.",
        ),
        row(
            "Sacred Foundry",
            "",
            "Land — Mountain Plains",
            "As Sacred Foundry enters, you may pay 2 life. If you don't, it enters tapped.\n{T}: Add {R} or {W}.",
        ),
    ]);
    let mut st = state(vec![0], vec![1]);
    assert!(play_land(&cards, &mut st, 1));
    let mut pool = ManaPool::default();
    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("the fetch's own one-life cost is payable");

    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 1);
    assert!(st.battlefield[0].tapped);
}

/// A renamed fetch pays its whole typed mana, tap, life, and sacrifice cost.
#[test]
fn renamed_paid_fetch_uses_the_shared_activation_payment() {
    let cards = deck(&[
        row(
            "Renamed paid fetch",
            "",
            "Land",
            "{2}{U}, {T}, Pay 3 life, Sacrifice Renamed paid fetch: Search your library for a Forest card, put it onto the battlefield, then shuffle.",
        ),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Mountain", "", "Basic Land — Mountain", "{T}: Add {R}."),
    ]);
    let costs = search_activation(&cards, CardIdx(0))
        .activation
        .as_ref()
        .expect("typed paid fetch costs");
    assert_eq!(costs.mana_cost(), parse_cost("{2}{U}"));
    assert!(costs.taps_source());
    assert!(costs.sacrifices_source());
    assert_eq!(costs.life_payment(), 3);
    let mut st = state(vec![0], vec![1, 2]);
    let mut pool = ManaPool {
        fixed: [0, 1, 0, 0, 0],
        colorless: 3,
        ..Default::default()
    };
    assert!(play_land(&cards, &mut st, 1));
    assert_eq!(pool.total(), 4);
    assert_eq!(st.life, 20);

    let activation = pick_best_activation(&cards, &st, &pool, 1)
        .expect("the shared pool covers every printed activation cost");
    resolve_activation(&cards, &mut st, &mut pool, 1, &activation);

    assert_eq!(pool.fixed, [0; 5]);
    assert_eq!(pool.colorless, 1);
    assert_eq!(pool.total(), 1);
    assert_eq!(st.life, 17);
    assert_eq!(st.life_paid, 3);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert_eq!(st.library, [CardIdx(2)]);
}

/// A tap-only search leaves its source tapped and cannot activate it twice.
#[test]
fn ordinary_land_search_pays_its_tap_cost() {
    let cards = deck(&[
        row(
            "Search land",
            "",
            "Land",
            "{1}, {T}: Search your library for a basic land card, put it onto the battlefield tapped, then shuffle.",
        ),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
        row("Island", "", "Basic Land — Island", "{T}: Add {U}."),
    ]);
    let mut st = state(vec![0], vec![1, 2]);
    let mut pool = ManaPool {
        colorless: 2,
        ..Default::default()
    };
    assert!(play_land(&cards, &mut st, 1));

    spend_leftover(&cards, &mut st, &mut pool, 1);

    assert_eq!(st.battlefield.len(), 2);
    assert!(st.battlefield[0].tapped);
    assert!(st.battlefield[1].tapped);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(0)));
    assert_eq!(st.battlefield[1].card, CardRef::Deck(CardIdx(2)));
    assert_eq!(pool.colorless, 1);
    assert!(st.graveyard.is_empty());
    assert_eq!(st.library, [CardIdx(1)]);
}

/// Fetched ETBs can see and return the source already sacrificed to pay costs.
#[test]
fn fetch_source_reaches_graveyard_before_the_fetched_land_etb() {
    let cards = deck(&[
        row(
            "Fetch",
            "",
            "Land",
            "{T}, Sacrifice this land: Search your library for a basic land card, put it onto the battlefield, then shuffle.",
        ),
        row(
            "Returning land",
            "",
            "Basic Land",
            "When this land enters, return target card from your graveyard to your hand.",
        ),
    ]);
    let mut st = state(vec![0], vec![1]);
    assert!(play_land(&cards, &mut st, 1));

    spend_leftover(&cards, &mut st, &mut ManaPool::default(), 1);

    assert_eq!(
        st.hand,
        [CardIdx(0)],
        "the fetched ETB finds the sacrificed source"
    );
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert!(st.graveyard.is_empty());
    assert!(st.library.is_empty());
}

/// A tapped fetch cannot pay a tap-symbol cost or consume its other resources.
#[test]
fn tapped_fetch_cannot_activate_or_consume_library_randomness() {
    let cards = deck(&[
        row(
            "Tapped fetch",
            "",
            "Land",
            "This land enters tapped.\n{1}, {T}, Pay 2 life, Sacrifice this land: Search your library for a basic land card, put it onto the battlefield, then shuffle.",
        ),
        row("Wastes", "", "Basic Land", "{T}: Add {C}."),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
    ]);
    let mut st = state(vec![0], vec![1, 2]);
    let mut pool = ManaPool {
        colorless: 1,
        ..Default::default()
    };
    assert!(play_land(&cards, &mut st, 1));
    assert!(st.battlefield[0].tapped);

    assert_activation_unavailable(&cards, &mut st, &mut pool);
}

/// Missing mana, life, or the required mana color blocks all fetch costs.
#[test]
fn unaffordable_fetch_leaves_zones_resources_and_randomness_unchanged() {
    let cards = deck(&[
        row(
            "Paid fetch",
            "",
            "Land",
            "{1}{U}, {T}, Pay 2 life, Sacrifice this land: Search your library for a basic land card, put it onto the battlefield, then shuffle.",
        ),
        row("Wastes", "", "Basic Land", "{T}: Add {C}."),
        row("Forest", "", "Basic Land — Forest", "{T}: Add {G}."),
    ]);
    for (life, mut pool) in [
        (20, ManaPool::default()),
        (
            20,
            ManaPool {
                colorless: 2,
                ..Default::default()
            },
        ),
        (
            1,
            ManaPool {
                colorless: 1,
                fixed: [0, 1, 0, 0, 0],
                ..Default::default()
            },
        ),
    ] {
        let mut st = state(vec![0], vec![1, 2]);
        st.life = life;
        assert!(play_land(&cards, &mut st, 1));

        assert_activation_unavailable(&cards, &mut st, &mut pool);
    }
}

/// Fetch metadata cannot bypass an unsupported required activation cost.
#[test]
fn unsupported_required_fetch_cost_does_not_grant_a_free_search() {
    for extra in ["Discard a card", "Remove a quest counter from this land"] {
        let cards = deck(&[
            row(
                "Arid Mesa",
                "",
                "Land",
                &format!(
                    "{{T}}, Pay 1 life, {extra}, Sacrifice Arid Mesa: Search your library for a Mountain or Plains card, put it onto the battlefield, then shuffle."
                ),
            ),
            row("Mountain", "", "Basic Land — Mountain", "{T}: Add {R}."),
            row("Plains", "", "Basic Land — Plains", "{T}: Add {W}."),
            row("Discard card", "{9}", "Sorcery", ""),
        ]);
        let mut st = state(vec![0, 3], vec![1, 2]);
        assert!(play_land(&cards, &mut st, 1));
        assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(0)));
        assert_eq!(st.life, 20);
        assert_eq!(st.library, [CardIdx(1), CardIdx(2)]);

        assert_activation_unavailable(&cards, &mut st, &mut ManaPool::default());
    }
}

/// An untapped fetched land can fund a later cast through the normal turn loop.
#[test]
fn normal_turn_loop_activates_fetch_and_casts_with_the_fetched_land() {
    let cards = deck(&[
        row(
            "Arid Mesa",
            "",
            "Land",
            "{T}, Pay 1 life, Sacrifice Arid Mesa: Search your library for a Mountain or Plains card, put it onto the battlefield, then shuffle.",
        ),
        row("Spell", "{R}", "Sorcery", "Gain 2 life."),
        row("Mountain", "", "Basic Land — Mountain", "{T}: Add {R}."),
    ]);
    let mut st = state(vec![0, 1], vec![2]);
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
    assert_eq!(st.graveyard, [CardIdx(0), CardIdx(1)]);
    assert_eq!(st.battlefield.len(), 1);
    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(2)));
    assert!(st.battlefield[0].tapped);
    assert_eq!(st.life, 21);
    assert_eq!(st.life_paid, 1);
    assert_eq!(st.spells_cast_this_turn, 1);
    assert_eq!(census.mana_spent, [1.0]);
}

/// A typed search with no matching land can fail after its costs are paid.
#[test]
fn legal_fetch_with_no_match_still_pays_its_printed_costs() {
    let cards = deck(&[
        row(
            "Arid Mesa",
            "",
            "Land",
            "{T}, Pay 1 life, Sacrifice Arid Mesa: Search your library for a Mountain or Plains card, put it onto the battlefield, then shuffle.",
        ),
        row("Island", "", "Basic Land — Island", "{T}: Add {U}."),
        row("Wastes", "", "Basic Land", "{T}: Add {C}."),
    ]);
    let mut st = state(vec![0], vec![1, 2]);
    assert!(play_land(&cards, &mut st, 1));
    let activation = pick_best_activation(&cards, &st, &ManaPool::default(), 1)
        .expect("a hidden-library search does not require a matching target");

    resolve_activation(&cards, &mut st, &mut ManaPool::default(), 1, &activation);

    assert!(st.battlefield.is_empty());
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.life, 19);
    assert_eq!(st.life_paid, 1);
    assert_eq!(st.library.len(), 2);
    assert!(st.library.contains(&CardIdx(1)));
    assert!(st.library.contains(&CardIdx(2)));
}
