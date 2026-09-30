//! Review tests for parsed Oracle syntax and semantics.
use super::*;

/// Damage and direct life loss remain distinct in parsed syntax.
#[test]
fn damage_and_life_loss_have_distinct_oracle_effects() {
    let damage = oracle_parser::parse_oracle_text("Bolt deals 3 damage to target player.", &[]);
    let [oracle_ast::OracleAbility::Spell(damage)] = damage.abilities.as_slice() else {
        panic!("expected damage spell");
    };
    assert!(matches!(
        damage.effects.as_slice(),
        [oracle_ast::OracleEffect::Damage {
            amount: 3,
            target: oracle_ast::DamageTarget::Player,
            ..
        }]
    ));

    let life_loss = oracle_parser::parse_oracle_text("Target player loses 3 life.", &[]);
    let [oracle_ast::OracleAbility::Spell(life_loss)] = life_loss.abilities.as_slice() else {
        panic!("expected life-loss spell");
    };
    assert!(matches!(
        life_loss.effects.as_slice(),
        [oracle_ast::OracleEffect::LoseLife { amount: 3, .. }]
    ));

    // CR 120.3a makes damage to a player cause life loss in ordinary cases,
    // but damage (CR 120) and losing life (CR 119.3) remain distinct events.
    let creature_damage =
        oracle_parser::parse_oracle_text("Bolt deals 3 damage to target creature.", &[]);
    let [oracle_ast::OracleAbility::Spell(creature_damage)] = creature_damage.abilities.as_slice()
    else {
        panic!("expected creature-damage spell");
    };
    assert!(matches!(
        creature_damage.effects.as_slice(),
        [oracle_ast::OracleEffect::Damage {
            amount: 3,
            target: oracle_ast::DamageTarget::Permanent,
            ..
        }]
    ));
}

/// Record typed mana activation costs with and without source sacrifice.
#[test]
fn mana_activations_keep_source_sacrifice_in_the_activation_cost() {
    let sacrifices =
        oracle_parser::parse_oracle_text("{T}, Sacrifice this artifact: Add {C}{C}.", &[]);
    let [oracle_ast::OracleAbility::Activated(sacrificing)] = sacrifices.abilities.as_slice()
    else {
        panic!("expected one mana activation");
    };
    assert!(
        sacrificing
            .costs
            .iter()
            .any(|cost| matches!(cost, oracle_ast::ActivationCost::Sacrifice { .. }))
    );
    assert!(matches!(
        sacrificing.effects.as_slice(),
        [oracle_ast::OracleEffect::Mana(yield_)] if yield_.colorless == 2
    ));

    let keeps_source = oracle_parser::parse_oracle_text("{T}: Add {G}.", &[]);
    let [oracle_ast::OracleAbility::Activated(non_sacrificing)] = keeps_source.abilities.as_slice()
    else {
        panic!("expected one mana activation");
    };
    assert!(
        !non_sacrificing
            .costs
            .iter()
            .any(|cost| matches!(cost, oracle_ast::ActivationCost::Sacrifice { .. }))
    );
    assert!(matches!(
        non_sacrificing.effects.as_slice(),
        [oracle_ast::OracleEffect::Mana(yield_)] if yield_.fixed[4] == 1
    ));

    assert!(sacrificing.is_mana_ability);
    assert!(non_sacrificing.is_mana_ability);

    let targeted = oracle_parser::parse_oracle_text(
        "{T}: Add {G} to target creature's controller's mana pool.",
        &[],
    );
    let [oracle_ast::OracleAbility::Activated(targeted)] = targeted.abilities.as_slice() else {
        panic!("expected a parsed activation");
    };
    assert!(!targeted.is_mana_ability);

    let timed = oracle_parser::parse_oracle_text("{T}: Add {G}. Activate only as a sorcery.", &[]);
    let [oracle_ast::OracleAbility::Activated(timed)] = timed.abilities.as_slice() else {
        panic!("expected a timed activation");
    };
    assert!(
        timed.is_mana_ability,
        "timing restrictions do not change CR 605 classification"
    );
    assert_eq!(
        timed.to_runtime_all()[0].kind,
        model::SimAbilityKind::ManaActivated
    );

    let mentions_mana = oracle_parser::parse_oracle_text("{T}: Draw a card. Add {G}.", &[]);
    let [oracle_ast::OracleAbility::Activated(mentions_mana)] = mentions_mana.abilities.as_slice()
    else {
        panic!("expected a multi-effect activation");
    };
    assert!(!mentions_mana.is_mana_ability);
}

/// Classify triggered mana abilities only when their event and effect fit CR 605.1b.
#[test]
fn triggered_mana_ability_classification_uses_event_and_effect() {
    let mana = oracle_parser::parse_oracle_text(
        "Whenever you tap a nonland permanent for mana, add one mana of any type that permanent produced.",
        &[],
    );
    let [oracle_ast::OracleAbility::Triggered(mana)] = mana.abilities.as_slice() else {
        panic!("expected a triggered ability");
    };
    assert!(matches!(
        mana.event,
        oracle_ast::OracleTriggerEvent::TappedForMana
    ));
    assert!(mana.is_mana_ability);

    let nonmana = oracle_parser::parse_oracle_text(
        "Whenever you tap a nonland permanent for mana, draw a card.",
        &[],
    );
    let [oracle_ast::OracleAbility::Triggered(nonmana)] = nonmana.abilities.as_slice() else {
        panic!("expected a triggered ability");
    };
    assert!(!nonmana.is_mana_ability);

    let extra_effect = oracle_parser::parse_oracle_text(
        "Whenever you tap a nonland permanent for mana, add {G}. Gain 1 life.",
        &[],
    );
    let [oracle_ast::OracleAbility::Triggered(extra_effect)] = extra_effect.abilities.as_slice()
    else {
        panic!("expected a triggered ability");
    };
    assert!(extra_effect.is_mana_ability);

    let moves_library = oracle_parser::parse_oracle_text(
        "Whenever you tap a nonland permanent for mana, draw a card and add {G}.",
        &[],
    );
    let [oracle_ast::OracleAbility::Triggered(moves_library)] = moves_library.abilities.as_slice()
    else {
        panic!("expected a triggered ability");
    };
    assert!(!moves_library.is_mana_ability);

    let targets = oracle_parser::parse_oracle_text(
        "Whenever you tap a nonland permanent for mana, add one mana of any type that target permanent produced.",
        &[],
    );
    let [oracle_ast::OracleAbility::Triggered(targets)] = targets.abilities.as_slice() else {
        panic!("expected a triggered ability");
    };
    assert!(!targets.is_mana_ability);
}

/// Keep a threshold rule separate from event-triggered abilities.
#[test]
fn static_win_threshold_lowers_as_a_static_ability() {
    let oracle = oracle_parser::parse_oracle_text(
        "If there are 20 or more tower counters on this artifact, you win the game.",
        &[],
    );
    let [oracle_ast::OracleAbility::Static(ability)] = oracle.abilities.as_slice() else {
        panic!("expected a static threshold ability");
    };
    let runtime = ability.to_runtime().expect("supported threshold ability");
    assert_eq!(runtime.kind, model::SimAbilityKind::Static);
    assert!(matches!(
        runtime.effect,
        model::SimEffect::WinThreshold { counters: 20 }
    ));
}

/// Keep each mana mode's cost and restrictions on its own AST activation.
#[test]
fn mana_modes_keep_separate_costs_and_spend_limits() {
    let modes = oracle_parser::parse_oracle_text(
        "{T}: Add {W}.\n{2}: Add {C}.\n{T}: Add {G}. Spend this mana only to cast creature spells.",
        &[],
    );
    let [
        oracle_ast::OracleAbility::Activated(white),
        oracle_ast::OracleAbility::Activated(colorless),
        oracle_ast::OracleAbility::Activated(creature_only),
    ] = modes.abilities.as_slice()
    else {
        panic!("expected three separate mana modes");
    };
    assert!(white.is_mana_ability);
    assert!(colorless.is_mana_ability);
    assert!(creature_only.is_mana_ability);
    assert!(matches!(
        white.costs.as_slice(),
        [oracle_ast::ActivationCost::Tap(_)]
    ));
    assert!(
        matches!(colorless.costs.as_slice(), [oracle_ast::ActivationCost::Mana(cost)] if cost.total() == 2)
    );
    assert!(matches!(
        creature_only.effects.first(),
        Some(oracle_ast::OracleEffect::Mana(yield_))
            if yield_.restriction == Some(model::SpendRestriction::Creature)
    ));
}

/// Apply CR 605.1a's no-library-movement condition to every effect.
#[test]
fn mana_ability_classification_allows_other_effects_but_not_library_moves() {
    let life = oracle_parser::parse_oracle_text("{T}: Add {G}. Gain 1 life.", &[]);
    let [oracle_ast::OracleAbility::Activated(life)] = life.abilities.as_slice() else {
        panic!("expected one activation");
    };
    assert!(life.is_mana_ability);

    let draw = oracle_parser::parse_oracle_text("{T}: Draw a card. Add {G}.", &[]);
    let [oracle_ast::OracleAbility::Activated(draw)] = draw.abilities.as_slice() else {
        panic!("expected one activation");
    };
    assert!(!draw.is_mana_ability);

    let search = oracle_parser::parse_oracle_text(
        "{T}: Search your library for a basic land card, then add {G}.",
        &[],
    );
    let [oracle_ast::OracleAbility::Activated(search)] = search.abilities.as_slice() else {
        panic!("expected one activation");
    };
    assert!(!search.is_mana_ability);
}

/// Preserve a spell effect after a separate reminder-text statement.
#[test]
fn spell_effect_after_keyword_reminder_stays_on_the_card() {
    let oracle = oracle_parser::parse_oracle_text(
        "Suspend 4—{G} (Rather than cast this card from your hand, pay {G} and exile it with four time counters on it. At the beginning of your upkeep, remove a time counter. When the last is removed, you may cast it without paying its mana cost.)\nCreate two 4/4 green Rhino creature tokens with trample.",
        &["Suspend".to_string()],
    );
    let effects: Vec<_> = oracle
        .abilities
        .iter()
        .filter_map(|ability| match ability {
            oracle_ast::OracleAbility::Spell(spell) => Some(spell.effects.as_slice()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        matches!(
            effects.as_slice(),
            [oracle_ast::OracleEffect::CreateTokens(2)]
        ),
        "parsed abilities: {:#?}",
        oracle.abilities
    );
}

/// Lower one activation once and retain its effects in resolution order.
#[test]
fn multi_effect_activation_keeps_one_cost_and_ordered_effects() {
    let card = oracle_parser::parse_oracle_text("{1}, {T}: Draw a card and gain 1 life.", &[]);
    let [oracle_ast::OracleAbility::Activated(activation)] = card.abilities.as_slice() else {
        panic!("expected one parsed activation");
    };
    assert_eq!(activation.effects.len(), 2);

    let runtime = activation.to_runtime_all();
    let [runtime] = runtime.as_slice() else {
        panic!("one parsed activation must lower to one runtime ability");
    };
    assert_eq!(runtime.kind, model::SimAbilityKind::Activated);
    let costs = runtime.activation.as_ref().expect("activation cost bundle");
    assert_eq!(costs.mana_cost().total(), 1);
    assert!(costs.taps_source());
    assert!(matches!(
        runtime.effect_sequence(),
        [model::SimEffect::Draw(1), model::SimEffect::GainLife(1)]
    ));
}

/// Record modeled and unsupported intervening-if condition lowering.
#[test]
fn trigger_condition_inventory_distinguishes_known_and_unknown_shapes() {
    let descend = oracle_parser::parse_oracle_text(
        "At the beginning of your upkeep, if there are four or more permanent cards in your graveyard, draw a card.",
        &[],
    );
    let [oracle_ast::OracleAbility::Triggered(descend)] = descend.abilities.as_slice() else {
        panic!("expected a triggered ability");
    };
    let known_runtime = descend.to_runtime_all();
    let [known] = known_runtime.as_slice() else {
        panic!("expected one runtime ability");
    };
    assert_eq!(known.trigger, model::SimTrigger::Upkeep);
    assert_eq!(
        known.condition,
        Some(model::SimAbilityCondition::Descend(4))
    );

    let unknown = oracle_parser::parse_oracle_text(
        "At the beginning of your upkeep, if you control another creature, draw a card.",
        &[],
    );
    let [oracle_ast::OracleAbility::Triggered(unknown)] = unknown.abilities.as_slice() else {
        panic!("expected a triggered ability");
    };
    assert!(unknown.to_runtime_all().is_empty());

    let compound = oracle_parser::parse_oracle_text(
        "At the beginning of your upkeep, if you have metalcraft and you control another creature, draw a card.",
        &[],
    );
    let [oracle_ast::OracleAbility::Triggered(compound)] = compound.abilities.as_slice() else {
        panic!("expected a trigger with a compound condition");
    };
    assert!(compound.to_runtime_all().is_empty());
}

/// Unsupported activation payments and non-self trigger subjects stay inert.
#[test]
fn unsupported_activation_costs_and_trigger_subjects_stay_inert() {
    let discarded = oracle_parser::parse_oracle_text("{1}, Discard a card: Draw a card.", &[]);
    let [oracle_ast::OracleAbility::Activated(discarded)] = discarded.abilities.as_slice() else {
        panic!("expected an activation");
    };
    assert!(discarded.to_runtime_all().is_empty());
    assert!(
        discarded
            .costs
            .iter()
            .any(|cost| matches!(cost, oracle_ast::ActivationCost::Other(_)))
    );

    let other_attack =
        oracle_parser::parse_oracle_text("Whenever another creature attacks, draw a card.", &[]);
    let [oracle_ast::OracleAbility::Triggered(other_attack)] = other_attack.abilities.as_slice()
    else {
        panic!("expected a trigger");
    };
    let runtime_abilities = other_attack.to_runtime_all();
    let [runtime] = runtime_abilities.as_slice() else {
        panic!("the supported event keeps its subject");
    };
    assert_eq!(runtime.event_subject, model::SimEventSubject::Another);
}
