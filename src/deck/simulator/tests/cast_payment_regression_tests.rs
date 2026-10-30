use super::*;
use crate::deck::simulator::game::new_perm_with;
use crate::deck::simulator::model::{
    AlternativeCastCost, AlternativeCostPayoff, CardFilter, ManaColor, SacrificeSearch,
    SpendRestriction,
};
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::turn_loop_tests::{cast, deck, restricted_pool, row, state};

/// Paid X keeps supported player scopes and leaves unsupported recipients inert.
#[test]
fn paid_x_damage_keeps_its_target_scope() {
    for (recipient, own_life, damage) in [
        ("target player", 20, 3),
        ("target opponent", 20, 3),
        ("each opponent", 20, 3),
        ("you", 20, 0),
        ("target creature", 20, 0),
    ] {
        let cards = deck(&[row(
            "Variable fire",
            "{X}{R}",
            "Sorcery",
            &format!("Variable fire deals X damage to {recipient}."),
        )]);
        let mut st = state(vec![0], vec![]);
        let mut pool = ManaPool {
            colorless: 3,
            fixed: [0, 0, 0, 1, 0],
            ..ManaPool::default()
        };

        assert_eq!(cast(&cards, &mut st, &mut pool), [4.0], "{recipient}");
        assert_eq!(st.life, own_life, "{recipient}");
        assert_eq!(st.damage_dealt_this_turn, damage, "{recipient}");
        assert_eq!(st.opponent_life_lost, 0, "{recipient}");
    }
}

/// A fixed zero and an unpaid X never become damage.
#[test]
fn zero_damage_has_no_invented_payoff() {
    for (cost, text) in [
        ("{X}{R}", "Zero fire deals X damage to target player."),
        ("{R}", "Zero fire deals 0 damage to target player."),
    ] {
        let cards = deck(&[row("Zero fire", cost, "Sorcery", text)]);
        let mut st = state(vec![0], vec![]);
        let mut pool = ManaPool {
            fixed: [0, 0, 0, 1, 0],
            ..ManaPool::default()
        };

        cast(&cards, &mut st, &mut pool);

        assert_eq!(st.damage_dealt_this_turn, 0);
        assert_eq!(st.opponent_life_lost, 0);
    }
}

/// Convoke reroutes the flexible creature to blue and reserves white for white.
#[test]
fn convoke_exact_matching_preserves_the_largest_x() {
    let mut multicolor = row("Two colors", "{W}{U}", "Creature", "");
    multicolor.colors = "[\"W\",\"U\"]".into();
    let mut white = row("White", "{W}", "Creature", "");
    white.colors = "[\"W\"]".into();
    let cards = deck(&[
        row(
            "Convoke draw",
            "{X}{W}{U}",
            "Sorcery",
            "Convoke\nDraw X cards.",
        ),
        multicolor,
        white,
        row("Drawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![3]);
    for (uid, index) in [(1, 1), (2, 2)] {
        st.battlefield
            .push(new_perm_with(uid, &cards, CardIdx(index), 0, false));
    }
    st.next_uid = 2;
    let mut pool = ManaPool {
        colorless: 1,
        ..ManaPool::default()
    };

    assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);
    assert_eq!(st.hand, [CardIdx(3)]);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert!(st.battlefield.iter().all(|permanent| permanent.tapped));
    assert_eq!(pool.total(), 0);
}

/// Convoke pays a matching hybrid color, but never an unrelated or colorless pip.
#[test]
fn convoke_matches_hybrid_colors_without_creating_colorless_mana() {
    for (cost, tapped, accepted) in [
        ("{W/U}", false, true),
        ("{U/B}", false, false),
        ("{C}", false, false),
        ("{W/U}", true, false),
        ("{W}{W}", false, false),
    ] {
        let mut white = row("White", "{W}", "Creature", "");
        white.colors = "[\"W\"]".into();
        let cards = deck(&[row("Convoke", cost, "Sorcery", "Convoke"), white]);
        let mut st = state(vec![0], vec![]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(1), 0, tapped));
        st.next_uid = 1;

        cast(&cards, &mut st, &mut ManaPool::default());

        assert_eq!(st.hand.is_empty(), accepted, "{cost}, tapped={tapped}");
        assert_eq!(st.battlefield[0].tapped, tapped || accepted);
    }
}

/// A failed cast commit leaves real mana and delve cards untouched.
#[test]
fn stale_tap_resource_rejects_the_whole_cast_payment() {
    let mut white = row("White", "{W}", "Creature", "");
    white.colors = "[\"W\"]".into();
    let cards = deck(&[
        row("Payment", "{2}{W}", "Sorcery", "Convoke\nDelve"),
        white,
        row("Delve card", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(vec![0], vec![]);
    st.graveyard.push(CardIdx(2));
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(1), 0, false));
    let mut pool = ManaPool {
        colorless: 1,
        ..ManaPool::default()
    };
    let prepared =
        payment::prepare_cast_payment(&cards, &st, &pool, CardIdx(0), &cards.cards[0].cost)
            .expect("the original untapped resource covers the white symbol");
    assert!(!st.battlefield[0].tapped);
    assert_eq!(st.graveyard, [CardIdx(2)]);
    st.battlefield[0].tapped = true;

    assert!(!prepared.commit(&mut st, &mut pool));

    assert_eq!(pool.colorless, 1);
    assert_eq!(st.graveyard, [CardIdx(2)]);
    assert!(st.exile.is_empty());
    assert_eq!(st.hand, [CardIdx(0)]);
}

/// Improvise taps an artifact once; its tapped state cannot discount another spell.
#[test]
fn improvise_consumes_an_untapped_artifact_once() {
    for tapped in [false, true] {
        let cards = deck(&[
            row("First", "{1}", "Sorcery", "Improvise"),
            row("Second", "{1}", "Sorcery", "Improvise"),
            row("Artifact", "{9}", "Artifact", ""),
        ]);
        let mut st = state(vec![0, 1], vec![]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(2), 0, tapped));
        st.next_uid = 1;

        assert_eq!(cast(&cards, &mut st, &mut ManaPool::default()), [0.0]);
        assert_eq!(st.spells_cast_this_turn, u32::from(!tapped));
        assert_eq!(st.hand.len(), if tapped { 2 } else { 1 });
        assert!(st.battlefield[0].tapped);
    }
}

/// An artifact creature is one shared tap resource, not two payments.
#[test]
fn convoke_and_improvise_share_each_permanent() {
    for (mana, accepted) in [(0, false), (1, true)] {
        let mut blue = row("Blue artifact", "{U}", "Artifact Creature", "");
        blue.colors = "[\"U\"]".into();
        let cards = deck(&[row("Both", "{1}{U}", "Sorcery", "Convoke\nImprovise"), blue]);
        let mut st = state(vec![0], vec![]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(1), 0, false));
        st.next_uid = 1;
        let mut pool = ManaPool {
            colorless: mana,
            ..ManaPool::default()
        };

        cast(&cards, &mut st, &mut pool);

        assert_eq!(st.hand.is_empty(), accepted);
        assert_eq!(st.battlefield[0].tapped, accepted);
        assert_eq!(pool.total(), 0);
    }
}

/// Improvise cannot pay either a colored or explicit colorless symbol.
#[test]
fn improvise_only_pays_generic_symbols() {
    for cost in ["{C}", "{W}", "{W/U}"] {
        let cards = deck(&[
            row("Improvise", cost, "Sorcery", "Improvise"),
            row("Artifact", "{9}", "Artifact", ""),
        ]);
        let mut st = state(vec![0], vec![]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(1), 0, false));

        cast(&cards, &mut st, &mut ManaPool::default());

        assert_eq!(st.hand, [CardIdx(0)], "{cost}");
        assert!(!st.battlefield[0].tapped);
    }
}

/// Kicker can spend a matching cast-restricted lot, but not an unrelated lot.
#[test]
fn kicker_uses_only_eligible_cast_restricted_mana() {
    for (restriction, expected_damage, remaining) in [
        (SpendRestriction::InstantSorcery, 4, 0),
        (SpendRestriction::Creature, 2, 1),
    ] {
        let cards = deck(&[row(
            "Kicked burn",
            "{0}",
            "Sorcery",
            "Kicker {1}\nKicked burn deals 2 damage to target player.",
        )]);
        let mut st = state(vec![0], vec![]);
        let mut pool = restricted_pool(1, restriction);

        assert_eq!(cast(&cards, &mut st, &mut pool), [f64::from(1 - remaining)]);
        assert_eq!(st.damage_dealt_this_turn, expected_damage);
        assert_eq!(pool.total(), remaining);
    }
}

/// Offspring is part of the creature cast and accepts creature-only mana.
#[test]
fn offspring_uses_only_eligible_cast_restricted_mana() {
    for (restriction, count, remaining) in [
        (SpendRestriction::Creature, 2, 0),
        (SpendRestriction::InstantSorcery, 1, 1),
    ] {
        let cards = deck(&[row("Parent", "{0}", "Creature", "Offspring {1}")]);
        let mut st = state(vec![0], vec![]);
        let mut pool = restricted_pool(1, restriction);

        cast(&cards, &mut st, &mut pool);

        assert_eq!(st.battlefield.len(), count);
        assert_eq!(pool.total(), remaining);
    }
}

/// Resolution mana cannot retroactively pay the spell's optional cost.
#[test]
fn kicker_is_chosen_before_the_spell_adds_mana() {
    let cards = deck(&[row(
        "Late mana",
        "{0}",
        "Sorcery",
        "Kicker {1}\nAdd {R}. Late mana deals 2 damage to target player.",
    )]);
    let mut st = state(vec![0], vec![]);
    let mut pool = ManaPool::default();

    cast(&cards, &mut st, &mut pool);

    assert_eq!(st.damage_dealt_this_turn, 2);
    assert_eq!(pool.total(), 1);
}

/// Build a fixed-count exile cost whose resolution gains the exiled mana value.
fn life_gain_alternative() -> AlternativeCastCost {
    AlternativeCastCost {
        filter: CardFilter {
            color: Some(ManaColor::Green),
        },
        count: 2,
        payoff: AlternativeCostPayoff::GainLifeEqualToExiledManaValue,
    }
}

/// The payoff does not change a fixed alternate cost's required card count.
#[test]
fn life_gain_alternative_exiles_exactly_the_required_count() {
    for count in [1, 3] {
        let mut cards = deck(&[
            row("Alternate", "{99}", "Sorcery", ""),
            row("Largest", "{10}", "Sorcery", ""),
            row("Next", "{9}", "Sorcery", ""),
            row("Kept", "{8}", "Sorcery", ""),
        ]);
        cards.cards[0].spell_data.alternative_cast_cost = Some(life_gain_alternative());
        for card in &mut cards.cards[1..] {
            card.colors[4] = true;
        }
        let mut st = state((0..=count).collect(), vec![]);

        cast(&cards, &mut st, &mut ManaPool::default());

        if count == 1 {
            assert_eq!(st.hand, [CardIdx(0), CardIdx(1)]);
            assert!(st.exile.is_empty());
            assert_eq!(st.life_gained, 0);
        } else {
            assert_eq!(st.hand, [CardIdx(3)]);
            assert_eq!(st.exile, [CardIdx(1), CardIdx(2)]);
            assert_eq!(st.life_gained, 19);
        }
    }
}

/// An alternate cast must not waive an unsupported mandatory extra cost.
#[test]
fn alternate_cast_rejects_unsupported_additional_costs() {
    let mut cards = deck(&[
        row("Alternate", "{99}", "Sorcery", "Gain three life."),
        row("First pitch", "{9}", "Sorcery", ""),
        row("Second pitch", "{9}", "Sorcery", ""),
    ]);
    cards.cards[0].spell_data.alternative_cast_cost = Some(life_gain_alternative());
    cards.cards[0].spell_data.unsupported_additional_cost = true;
    cards.cards[1].colors[4] = true;
    cards.cards[2].colors[4] = true;
    let mut st = state(vec![0, 1, 2], vec![]);

    cast(&cards, &mut st, &mut ManaPool::default());

    assert_eq!(st.hand, [CardIdx(0), CardIdx(1), CardIdx(2)]);
    assert!(st.exile.is_empty());
    assert_eq!(st.life_gained, 0);
}

/// Cascade waives unsupported printed mana, but not unsupported extra costs.
#[test]
fn cascade_distinguishes_unsupported_mana_and_additional_costs() {
    for unsupported_additional in [false, true] {
        let mut cards = deck(&[
            row("Cascade", "{1}", "Sorcery", "Cascade"),
            row("Free hit", "{0}", "Sorcery", "Gain three life."),
        ]);
        cards.cards[1].cost.unsupported = true;
        cards.cards[1].min_cost.unsupported = true;
        cards.cards[1].spell_data.unsupported_additional_cost = unsupported_additional;
        let mut st = state(vec![0], vec![1]);
        let mut pool = ManaPool {
            colorless: 1,
            ..ManaPool::default()
        };

        cast(&cards, &mut st, &mut pool);

        assert_eq!(st.life_gained, if unsupported_additional { 0 } else { 3 });
        assert_eq!(st.library.contains(&CardIdx(1)), unsupported_additional);
        assert_eq!(
            st.spells_cast_this_turn,
            if unsupported_additional { 1 } else { 2 }
        );
    }
}

/// A supported replay mana cost cannot hide an unsupported additional cast cost.
#[test]
fn flashback_rejects_unsupported_additional_costs() {
    for unsupported_additional in [false, true] {
        let mut cards = deck(&[row("Replay", "{9}", "Sorcery", "Gain three life.")]);
        cards.cards[0].spell_data.own_flashback = Some(parse_cost("{0}"));
        cards.cards[0].spell_data.unsupported_additional_cost = unsupported_additional;
        let mut st = state(vec![], vec![]);
        st.graveyard.push(CardIdx(0));

        cast_graveyard_spells(
            &cards,
            &mut st,
            &mut ManaPool::default(),
            1,
            &mut [0.0],
            &mut Vec::new(),
        );

        assert_eq!(st.life_gained, if unsupported_additional { 0 } else { 3 });
        assert_eq!(st.graveyard.contains(&CardIdx(0)), unsupported_additional);
        assert_eq!(st.exile.contains(&CardIdx(0)), !unsupported_additional);
    }
}

/// Sacrifice searches use their parsed offset, range, and entry-counter rule.
#[test]
fn sacrifice_search_obeys_its_cost_linked_metadata() {
    for (offset, allow_lower, counter, target_cost, target_type, accepted) in [
        (1, false, true, "{3}", "Creature", true),
        (1, false, true, "{2}", "Creature", false),
        (1, false, true, "{4}", "Creature", false),
        (2, true, false, "{4}", "Creature", true),
        (2, true, false, "{3}", "Creature", true),
        (2, true, false, "{5}", "Creature", false),
        (2, true, false, "{2}", "Artifact", false),
    ] {
        let cards = deck(&[row("Search target", target_cost, target_type, "")]);
        let mut st = state(vec![], vec![0]);
        let mut entries = Vec::new();

        entry::resolve_sacrifice_search(
            &cards,
            &mut st,
            1,
            Some(2),
            Some(SacrificeSearch {
                shuffle: false,
                mana_value_offset: offset,
                allow_lower_mana_value: allow_lower,
                enters_with_plus1_counter: counter,
            }),
            &mut entries,
        );

        assert_eq!(st.battlefield.len(), usize::from(accepted), "{target_cost}");
        assert_eq!(entries.len(), usize::from(accepted));
        if accepted {
            assert_eq!(st.battlefield[0].counters.plus1, u32::from(counter));
        }
    }
}

/// A permanent does not need its later enters-trigger target to be cast.
#[test]
fn targeted_entry_trigger_does_not_block_its_permanent_spell() {
    let cards = deck(&[row(
        "Entry return",
        "{1}",
        "Creature",
        "When this creature enters, return target creature card from your graveyard to the battlefield.",
    )]);
    let mut st = state(vec![0], vec![]);
    let mut pool = ManaPool {
        colorless: 1,
        ..ManaPool::default()
    };

    assert_eq!(cast(&cards, &mut st, &mut pool), [1.0]);

    assert!(st.hand.is_empty());
    assert_eq!(st.battlefield.len(), 1);
    assert!(st.graveyard.is_empty());
    assert_eq!(pool.total(), 0);
}
