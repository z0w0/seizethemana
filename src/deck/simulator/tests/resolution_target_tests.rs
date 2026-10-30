use super::*;
use crate::deck::simulator::cast_pass::cast_graveyard_spells;
use crate::deck::simulator::game::{fire_triggers, graveyard_entry_matches, new_perm_with};
use crate::deck::simulator::game_effects::{
    CardZone, apply_prepared_spell_effect_at, mill_library_card, move_to_graveyard,
    prepare_effect_targets, prepare_spell_targets,
};
use crate::deck::simulator::model::{SimAbilityCondition, SimTrigger};
use crate::deck::simulator::turn_loop_tests::{deck, row, state};
use rand::RngCore;

/// A target cannot follow the same physical card through a new graveyard entry.
#[test]
fn prepared_target_rejects_a_new_graveyard_generation() {
    let cards = deck(&[
        row(
            "Return",
            "{1}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield.",
        ),
        row("Creature", "{9}", "Creature", ""),
    ]);
    let mut st = state(Vec::new(), Vec::new());
    st.graveyard.push(CardIdx(1));
    let mut targets =
        prepare_spell_targets(&cards, &st, &cards.cards[0]).expect("seeded target exists");
    assert!(graveyard_entry_matches(&st, CardIdx(1), 0));
    assert!(targets.any_legal_target(&st));

    st.graveyard.clear();
    move_to_graveyard(&cards, &mut st, CardIdx(1), 1, CardZone::Hand);

    assert_eq!(graveyard_generation(&st, CardIdx(1)), 1);
    assert!(!targets.any_legal_target(&st));
    apply_prepared_spell_effect_at(
        &cards,
        &cards.cards[0].spell_data.effects[0],
        &mut st,
        1,
        false,
        Some(CardIdx(0)),
        &mut targets,
    );
    assert!(st.battlefield.is_empty());
    assert_eq!(st.graveyard, [CardIdx(1)]);
}

/// Spells without targets still resolve, and one surviving target is sufficient.
#[test]
fn target_legality_distinguishes_no_targets_partial_targets_and_all_illegal() {
    let cards = deck(&[
        row("First creature", "{9}", "Creature", ""),
        row("Second creature", "{9}", "Creature", ""),
    ]);
    let mut st = state(Vec::new(), Vec::new());
    assert!(
        prepare_effect_targets(&cards, &st, &[SimEffect::Draw(1)])
            .expect("draw has no targets")
            .any_legal_target(&st)
    );
    st.graveyard.extend([CardIdx(0), CardIdx(1)]);
    let effect = SimEffect::ReturnFromGraveyard {
        to_hand: false,
        count: 2,
    };
    let mut targets = prepare_effect_targets(&cards, &st, std::slice::from_ref(&effect))
        .expect("both targets exist");
    st.graveyard.remove(0);
    st.exile.push(CardIdx(0));
    assert!(targets.any_legal_target(&st));

    apply_prepared_spell_effect_at(&cards, &effect, &mut st, 1, false, None, &mut targets);

    assert_eq!(st.battlefield[0].card, CardRef::Deck(CardIdx(1)));
    assert!(st.graveyard.is_empty());
    assert_eq!(st.exile, [CardIdx(0)]);
    let remaining = prepare_effect_targets(
        &cards,
        &st,
        &[SimEffect::ReturnFromGraveyard {
            to_hand: false,
            count: 1,
        }],
    );
    assert!(remaining.is_none());
}

/// A mill trigger cannot create targets for another trigger in its pending batch.
#[test]
fn trigger_batch_prepares_return_targets_before_any_member_resolves() {
    let cards = deck(&[
        row(
            "Mill engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, mill a card.",
        ),
        row(
            "Return engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, return target creature card from your graveyard to the battlefield. Gain 3 life. Draw a card.",
        ),
        row("Milled creature", "{9}", "Creature", ""),
        row("Undrawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![3, 2]);
    for index in 0..2 {
        st.battlefield
            .push(new_perm_with(index + 1, &cards, CardIdx(index), 0, false));
    }
    st.next_uid = 2;

    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);

    assert_eq!(st.graveyard, [CardIdx(2)]);
    assert_eq!(st.battlefield.len(), 2);
    assert_eq!(st.life, 20);
    assert!(st.hand.is_empty());
    assert_eq!(st.library, [CardIdx(3)]);
}

/// Targets become available when the owning action ends, not at event capture.
#[test]
fn trigger_target_choice_waits_until_the_owning_action_ends() {
    let cards = deck(&[
        row(
            "Return engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, return target creature card from your graveyard to the battlefield.",
        ),
        row("Creature", "{9}", "Creature", ""),
    ]);
    let mut st = state(Vec::new(), vec![1]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.next_uid = 1;

    begin_resolution(&mut st);
    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);
    mill_library_card(&cards, &mut st, 1, false);
    end_resolution(&cards, &mut st, None);

    assert_eq!(st.battlefield.len(), 2);
    assert_eq!(st.battlefield[1].card, CardRef::Deck(CardIdx(1)));
    assert!(st.graveyard.is_empty());
}

/// An older waiting trigger cannot target a later stay of its original card.
#[test]
fn waiting_return_trigger_rejects_a_target_that_left_and_reentered() {
    let mut cards = deck(&[
        row(
            "Move engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, return target card from your graveyard to your hand.",
        ),
        row(
            "Return engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, return target creature card from your graveyard to the battlefield. Gain 3 life.",
        ),
        row("Creature", "{9}", "Creature", ""),
    ]);
    cards.cards[0].striations[0].abilities[0].effects = vec![
        SimEffect::ReturnFromGraveyard {
            to_hand: true,
            count: 1,
        },
        SimEffect::DiscardHandThenDrawSeven,
    ];
    let mut st = state(Vec::new(), Vec::new());
    st.graveyard.push(CardIdx(2));
    for index in 0..2 {
        st.battlefield
            .push(new_perm_with(index + 1, &cards, CardIdx(index), 0, false));
    }
    st.next_uid = 2;

    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);

    assert_eq!(st.graveyard, [CardIdx(2)]);
    assert_eq!(graveyard_generation(&st, CardIdx(2)), 1);
    assert_eq!(st.battlefield.len(), 2);
    assert!(st.hand.is_empty());
    assert_eq!(st.life, 20);
}

/// New ETB triggers prepare as one batch before older waiting triggers resume.
#[test]
fn nested_trigger_batch_cannot_retarget_after_its_first_member_mills() {
    let cards = deck(&[
        row(
            "First engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, return target creature card from your graveyard to the battlefield.",
        ),
        row(
            "Waiting engine",
            "{1}",
            "Artifact",
            "Whenever you cast a spell, return target creature card from your graveyard to the battlefield. Gain 3 life.",
        ),
        row(
            "Returned creature",
            "{9}",
            "Creature",
            "When this creature enters, mill a card.\nWhen this creature enters, return target creature card from your graveyard to the battlefield. Gain 3 life.",
        ),
        row("Milled creature", "{9}", "Creature", ""),
    ]);
    let mut st = state(Vec::new(), vec![3]);
    st.graveyard.push(CardIdx(2));
    for index in 0..2 {
        st.battlefield
            .push(new_perm_with(index + 1, &cards, CardIdx(index), 0, false));
    }
    st.next_uid = 2;

    fire_triggers(&cards, &mut st, SimTrigger::SpellCast, 1);

    assert_eq!(st.battlefield.len(), 3);
    assert_eq!(st.battlefield[2].card, CardRef::Deck(CardIdx(2)));
    assert_eq!(st.graveyard, [CardIdx(3)]);
    assert_eq!(st.life, 20);
}

/// Undying only returns the graveyard object created by its own death.
#[test]
fn queued_undying_rejects_a_new_graveyard_generation() {
    let cards = deck(&[row("Creature", "{9}", "Creature", "Undying")]);
    let mut st = state(Vec::new(), Vec::new());
    st.next_uid = 7;
    let victim = new_perm_with(7, &cards, CardIdx(0), 0, false);

    begin_resolution(&mut st);
    move_to_graveyard(&cards, &mut st, CardIdx(0), 1, CardZone::Battlefield);
    queue_undying(&cards, &mut st, &victim, 1);
    st.graveyard.clear();
    move_to_graveyard(&cards, &mut st, CardIdx(0), 1, CardZone::Hand);
    end_resolution(&cards, &mut st, None);

    assert_eq!(graveyard_generation(&st, CardIdx(0)), 2);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert!(st.battlefield.is_empty());
    assert_eq!(st.next_uid, 7);
}

/// A later hand-to-graveyard move cannot receive an earlier library trigger.
#[test]
fn queued_library_triggers_reject_a_new_graveyard_generation() {
    for trigger in [
        LibraryGraveyardTrigger::ReturnToBattlefield,
        LibraryGraveyardTrigger::DrainAndGain(3),
    ] {
        let mut cards = deck(&[row("Library card", "{9}", "Creature", "")]);
        cards.cards[0].library_graveyard_trigger = Some(trigger);
        let mut st = state(Vec::new(), vec![0]);

        begin_resolution(&mut st);
        mill_library_card(&cards, &mut st, 1, false);
        st.graveyard.clear();
        move_to_graveyard(&cards, &mut st, CardIdx(0), 1, CardZone::Hand);
        end_resolution(&cards, &mut st, None);

        assert_eq!(graveyard_generation(&st, CardIdx(0)), 2);
        assert_eq!(st.graveyard, [CardIdx(0)]);
        assert!(st.battlefield.is_empty());
        assert!(st.exile.is_empty());
        assert_eq!(st.life, 20);
        assert_eq!(st.opponent_life_lost, 0);
    }
}

/// Escape can exile its prepared target without panicking or resolving rewards.
#[test]
fn escape_preserves_target_choice_before_exiling_its_fodder() {
    let cards = deck(&[
        row(
            "Zombify",
            "{3}{B}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield. Gain 5 life. Draw a card.",
        ),
        row("Creature", "{9}", "Creature", ""),
        row("First land", "", "Land", ""),
        row("Second land", "", "Land", ""),
        row(
            "Escape grant",
            "{1}",
            "Enchantment",
            "Nonland cards in your graveyard have escape. The escape cost is equal to the card's mana cost plus exile three other cards from your graveyard.",
        ),
        row("Undrawn", "{9}", "Sorcery", ""),
    ]);
    let mut st = state(Vec::new(), vec![5]);
    st.graveyard.extend([0, 1, 2, 3].map(CardIdx));
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(4), 0, false));
    st.next_uid = 1;
    let mut pool = ManaPool {
        colorless: 3,
        fixed: [0, 0, 1, 0, 0],
        ..Default::default()
    };
    let before_random = st.library_rng.clone().next_u64();
    let mut spent = [0.0];

    cast_graveyard_spells(&cards, &mut st, &mut pool, 1, &mut spent, &mut Vec::new());

    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.exile, [CardIdx(1), CardIdx(2), CardIdx(3)]);
    assert_eq!(st.battlefield.len(), 1);
    assert!(st.hand.is_empty());
    assert_eq!(st.library, [CardIdx(5)]);
    assert_eq!(st.life, 20);
    assert_eq!(st.life_gained, 0);
    assert_eq!(st.seen, 0);
    assert_eq!(st.replay_casts, 1);
    assert_eq!(st.spells_cast_this_turn, 1);
    assert_eq!(pool.total(), 0);
    assert_eq!(spent, [4.0]);
    assert_eq!(st.library_rng.clone().next_u64(), before_random);
}

/// A surviving escape target keeps the legal parts and other spell effects.
#[test]
fn escape_with_one_surviving_target_resolves_the_legal_parts() {
    let mut cards = deck(&[
        row(
            "Return spell",
            "{3}{B}",
            "Sorcery",
            "Return target creature card from your graveyard to the battlefield.",
        ),
        row("Exiled creature", "{9}", "Creature", ""),
        row("Remaining creature", "{9}", "Creature", ""),
        row("First land", "", "Land", ""),
        row("Second land", "", "Land", ""),
        row(
            "Escape grant",
            "{1}",
            "Enchantment",
            "Nonland cards in your graveyard have escape. The escape cost is equal to the card's mana cost plus exile three other cards from your graveyard.",
        ),
    ]);
    cards.cards[0].spell_data.effects = vec![
        SimEffect::GainLife(3),
        SimEffect::ReturnFromGraveyard {
            to_hand: false,
            count: 2,
        },
    ];
    let mut st = state(Vec::new(), Vec::new());
    st.graveyard.extend([0, 1, 3, 4, 2].map(CardIdx));
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(5), 0, false));
    st.next_uid = 1;
    let mut pool = ManaPool {
        colorless: 3,
        fixed: [0, 0, 1, 0, 0],
        ..Default::default()
    };

    cast_graveyard_spells(&cards, &mut st, &mut pool, 1, &mut [0.0], &mut Vec::new());

    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.exile, [CardIdx(1), CardIdx(3), CardIdx(4)]);
    assert_eq!(st.battlefield.len(), 2);
    assert_eq!(st.battlefield[1].card, CardRef::Deck(CardIdx(2)));
    assert_eq!(st.life, 23);
    assert_eq!(st.life_gained, 3);
    assert_eq!(st.replay_casts, 1);
    assert_eq!(pool.total(), 0);
}

/// Face-down artifacts cannot satisfy Metalcraft or use their printed power.
#[test]
fn face_down_characteristics_apply_to_supported_conditions() {
    let mut giant = row("Artifact creature", "{1}", "Artifact Creature", "");
    giant.power = Some("12".into());
    let cards = deck(&[giant]);
    let mut st = state(Vec::new(), Vec::new());
    for uid in 1..=3 {
        st.battlefield
            .push(new_perm_with(uid, &cards, CardIdx(0), 0, false));
    }
    assert!(condition_met(&cards, &st, &SimAbilityCondition::Metalcraft));
    assert!(condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
    for permanent in &mut st.battlefield {
        permanent.face_down = true;
    }
    assert!(!condition_met(
        &cards,
        &st,
        &SimAbilityCondition::Metalcraft
    ));
    assert!(!condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
    st.battlefield[0].counters.plus1 = 2;
    assert!(condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
    st.battlefield[0].counters.minus1 = 1;
    assert!(!condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
}

/// A played land face cannot provide its hidden creature front's power to an entry condition.
#[test]
fn ferocious_uses_the_physical_mdfc_face_before_uprising_draws() {
    let source = crate::deck::simulator::card_test_support::named_row("Garruk's Uprising");
    crate::deck::simulator::card_test_support::pinned_revision(&source);
    let mut mdfc = row(
        "Synthetic giant // Synthetic coast",
        "{5}{G}{G}",
        "Creature - Giant // Land",
        "Reach\n//\nThis land enters tapped.\n{T}: Add {G}.",
    );
    mdfc.power = Some("7".into());
    mdfc.toughness = Some("6".into());
    let cards = deck(&[mdfc, source, row("Drawn card", "{99}", "Sorcery", "")]);
    assert!(cards[CardIdx(0)].is_mdfc_spell);
    for land_face in [false, true] {
        let mut st = state(Vec::new(), vec![2]);
        let mut permanent = new_perm_with(1, &cards, CardIdx(0), 0, false);
        permanent.land_face = land_face;
        st.battlefield.push(permanent);
        st.battlefield
            .push(new_perm_with(2, &cards, CardIdx(1), 1, false));
        st.next_uid = 2;
        assert_eq!(
            condition_met(&cards, &st, &SimAbilityCondition::Ferocious),
            !land_face
        );
        crate::deck::simulator::game::fire_on_enter(&cards, &mut st, 1, 1, false);
        assert_eq!(st.hand, if land_face { vec![] } else { vec![CardIdx(2)] });
        assert_eq!(st.seen, u32::from(!land_face));
        assert_eq!(st.battlefield.len(), 2);
        assert_eq!(st.battlefield[0].uid, 1);
        assert_eq!(st.battlefield[0].land_face, land_face);
    }
}

/// Ferocious uses supported current power and excludes printed power on noncreatures.
#[test]
fn ferocious_requires_a_creature_with_current_power_at_least_four() {
    let mut body = row("Small creature", "{1}", "Creature", "");
    body.power = Some("2".into());
    let mut object = row("Not a creature", "{1}", "Artifact", "");
    object.power = Some("9".into());
    let cards = deck(&[
        body,
        object,
        row(
            "Power grant",
            "{1}",
            "Enchantment",
            "Creatures you control get +2/+2.",
        ),
    ]);
    let mut st = state(Vec::new(), Vec::new());
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(1), 0, false));
    assert!(!condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(0), 0, false));
    assert!(!condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
    st.battlefield
        .push(new_perm_with(3, &cards, CardIdx(2), 0, false));
    assert!(condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
    st.battlefield[1].counters.minus1 = 1;
    assert!(!condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
}

/// An entry condition includes supported prowess power without adding it twice in combat.
#[test]
fn uprising_counts_prowess_from_its_own_noncreature_cast() {
    let source = crate::deck::simulator::card_test_support::named_row("Garruk's Uprising");
    crate::deck::simulator::card_test_support::pinned_revision(&source);
    for prowess in [false, true] {
        let mut body = row(
            "Synthetic prowess body",
            "{R}",
            "Creature",
            if prowess { "Prowess" } else { "" },
        );
        body.power = Some("1".into());
        body.toughness = Some("2".into());
        let cards = deck(&[
            body,
            row("First noncreature", "{U}", "Sorcery", "Gain 1 life."),
            row("Second noncreature", "{U}", "Sorcery", "Gain 1 life."),
            source.clone(),
            row("Drawn card", "{99}", "Sorcery", ""),
        ]);
        let mut st = state(vec![1, 2], vec![4]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
        st.battlefield[0].summoning_sick = false;
        st.next_uid = 1;
        let mut pool = ManaPool {
            fixed: [0, 2, 0, 0, 0],
            ..Default::default()
        };
        crate::deck::simulator::turn_loop_tests::cast(&cards, &mut st, &mut pool);
        assert_eq!(st.prowess_casts, 2);
        assert_eq!(pool.total(), 0);
        assert!(!condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
        st.hand.push(CardIdx(3));
        pool.fixed[4] = 1;
        pool.colorless = 2;
        crate::deck::simulator::turn_loop_tests::cast(&cards, &mut st, &mut pool);
        assert_eq!(st.prowess_casts, 3);
        assert_eq!(pool.total(), 0);
        assert_eq!(st.hand, if prowess { vec![CardIdx(4)] } else { vec![] });
        assert_eq!(st.seen, u32::from(prowess));
        assert_eq!(
            condition_met(&cards, &st, &SimAbilityCondition::Ferocious),
            prowess
        );
        let combat = crate::deck::simulator::game_combat::combat_phase(&cards, &mut st, 1, &[0]);
        assert_eq!(combat.power, if prowess { 4 } else { 1 });
        assert!(st.completion.is_none());
    }
}

/// Prowess follows the live permanent at each cast and expires before the next turn's actions.
#[test]
fn prowess_does_not_apply_retroactively_and_resets_next_turn() {
    let mut body = row("Synthetic prowess body", "{0}", "Creature", "Prowess");
    body.power = Some("1".into());
    let cards = deck(&[
        body.clone(),
        body,
        row("First cast", "{0}", "Sorcery", "Gain 1 life."),
        row("Second cast", "{0}", "Sorcery", "Gain 1 life."),
        row("Next draw", "{99}", "Sorcery", ""),
    ]);
    let mut st = state(vec![2], vec![4]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.next_uid = 1;
    let mut pool = ManaPool::default();
    crate::deck::simulator::turn_loop_tests::cast(&cards, &mut st, &mut pool);
    assert_eq!(st.battlefield[0].prowess_bonus, 1);
    st.hand.push(CardIdx(1));
    crate::deck::simulator::turn_loop_tests::cast(&cards, &mut st, &mut pool);
    assert_eq!(st.battlefield[0].prowess_bonus, 1);
    assert_eq!(st.battlefield[1].prowess_bonus, 0);
    st.hand.push(CardIdx(3));
    crate::deck::simulator::turn_loop_tests::cast(&cards, &mut st, &mut pool);
    assert_eq!(st.battlefield[0].prowess_bonus, 2);
    assert_eq!(st.battlefield[1].prowess_bonus, 1);
    assert!(!condition_met(&cards, &st, &SimAbilityCondition::Ferocious));
    let mut census = crate::deck::simulator::game_run::TurnCensus::new(2, cards.cards.len());
    crate::deck::simulator::game_run::ending_phase(&cards, &mut st, &mut census, 1, false);
    assert!(
        st.battlefield
            .iter()
            .all(|permanent| permanent.prowess_bonus == 0)
    );
    let commander = crate::deck::simulator::game_commander::CommanderProfile::new(&cards);
    crate::deck::simulator::game_run::run_turn(
        &cards,
        &mut st,
        &mut census,
        &commander,
        &mut Vec::new(),
        2,
        false,
    );
    assert!(
        st.battlefield
            .iter()
            .all(|permanent| permanent.prowess_bonus == 0)
    );
    assert_eq!(st.prowess_casts, 0);
    assert_eq!(census.attack_power[1], 2);
    assert!(st.completion.is_none());
}

/// Cast conditions are captured before prowess resolves, unlike the spell's later entry event.
#[test]
fn prowess_cannot_enable_a_condition_at_the_same_cast_event() {
    let mut body = row("Synthetic prowess body", "{0}", "Creature", "Prowess");
    body.power = Some("1".into());
    let cards = deck(&[
        body,
        row(
            "Conditional cast observer",
            "{99}",
            "Enchantment",
            "Whenever you cast a noncreature spell, if you control a creature with power 4 or greater, draw a card.",
        ),
        row("First cast", "{0}", "Sorcery", "Gain 1 life."),
        row("Second cast", "{0}", "Sorcery", "Gain 1 life."),
        row("Third cast", "{0}", "Sorcery", "Gain 1 life."),
        row("Fourth cast", "{0}", "Sorcery", "Gain 1 life."),
        row("First draw", "{99}", "Sorcery", ""),
        row("Second draw", "{99}", "Sorcery", ""),
    ]);
    let mut st = state(vec![2, 3], vec![6, 7]);
    st.battlefield
        .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
    st.battlefield
        .push(new_perm_with(2, &cards, CardIdx(1), 0, false));
    st.next_uid = 2;
    let mut pool = ManaPool::default();
    crate::deck::simulator::turn_loop_tests::cast(&cards, &mut st, &mut pool);
    assert_eq!(st.battlefield[0].prowess_bonus, 2);
    assert_eq!(st.seen, 0);
    st.hand.push(CardIdx(4));
    crate::deck::simulator::turn_loop_tests::cast(&cards, &mut st, &mut pool);
    assert_eq!(st.battlefield[0].prowess_bonus, 3);
    assert_eq!(st.seen, 0);
    assert_eq!(st.library, [CardIdx(6), CardIdx(7)]);
    st.hand.push(CardIdx(5));
    crate::deck::simulator::turn_loop_tests::cast(&cards, &mut st, &mut pool);
    assert_eq!(st.battlefield[0].prowess_bonus, 4);
    assert_eq!(st.seen, 1);
    assert_eq!(st.hand, [CardIdx(7)]);
    assert!(st.completion.is_none());
}
