use super::oracle_ast::{
    AbilityRestriction, ActivationCost, ActivationTarget, CostObject, KeywordArgument, KeywordName,
    ObjectSubject, OracleAbility, OracleEffect, PlayerScope, SagaChapter, StaticEffect,
    StaticTarget, TriggerEvent, TurnStep, UnsupportedAbilityKind,
};
use super::oracle_parser::parse_oracle_text;

/// Parse one activation effect from synthetic Oracle text.
fn activation_effect(text: &str) -> OracleEffect {
    let card = parse_oracle_text(&format!("{{T}}: {text}"), &[]);
    let [OracleAbility::Activated(ability)] = card.abilities.as_slice() else {
        panic!("expected one activated ability for {text:?}");
    };
    ability.effects.first().expect("activated effect").clone()
}

/// Parse the event node from a triggered Oracle ability.
fn trigger_event(text: &str) -> TriggerEvent {
    let card = parse_oracle_text(text, &[]);
    let [OracleAbility::Triggered(ability)] = card.abilities.as_slice() else {
        panic!("expected one triggered ability for {text:?}");
    };
    ability.event.clone()
}

/// Parse the static-effect node from synthetic Oracle text.
fn static_effect(text: &str) -> StaticEffect {
    let card = parse_oracle_text(text, &[]);
    let [OracleAbility::Static(ability)] = card.abilities.as_slice() else {
        panic!(
            "expected one static ability for {text:?}, got {:#?}",
            card.abilities
        );
    };
    ability.effects.first().expect("static effect").clone()
}

/// Parse the search specification from an activated ability.
fn search_spec(text: &str) -> super::model::SearchSpec {
    let OracleEffect::Search(spec) = activation_effect(text) else {
        panic!("expected search effect for {text:?}");
    };
    spec
}

/// Check that activation costs and supported effects become typed nodes.
#[test]
fn parses_activated_costs_and_effects_as_typed_nodes() {
    let card = parse_oracle_text(
        "{2}, {T}, Sacrifice a creature, Pay 2 life: Draw two cards.",
        &[],
    );
    let [OracleAbility::Activated(ability)] = card.abilities.as_slice() else {
        panic!("expected one activated ability");
    };

    assert!(
        ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Mana(mana) if mana.total() == 2))
    );
    assert!(
        ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Tap(_)))
    );
    assert!(
        ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Sacrifice { count: 1, .. }))
    );
    assert!(
        ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::PayLife(2)))
    );
    assert!(matches!(
        ability.effects.as_slice(),
        [OracleEffect::Draw(2)]
    ));
}

/// Keep unsupported trigger effects in the AST without lowering them.
#[test]
fn parses_distinct_trigger_events_and_keeps_unsupported_effects_inert() {
    let card = parse_oracle_text(
        "Whenever this creature attacks, draw a card.\nWhenever this creature attacks, venture into the dungeon.",
        &[],
    );
    let triggers = card
        .abilities
        .iter()
        .filter_map(|ability| match ability {
            OracleAbility::Triggered(trigger) => Some(trigger),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(triggers.len(), 2);
    assert!(matches!(triggers[0].event, TriggerEvent::Attacks(_)));
    assert!(matches!(
        triggers[0].effects.as_slice(),
        [OracleEffect::Draw(1)]
    ));
    assert!(matches!(
        triggers[1].effects.as_slice(),
        [OracleEffect::Unsupported(_)]
    ));
    assert!(triggers[1].to_runtime().is_none());
}

/// Parse keyword names and typed parameters from card data and Oracle text.
#[test]
fn parses_keyword_names_and_parameters_from_both_sources() {
    let card = parse_oracle_text(
        "Flying\nDredge 5\nWard {2}\nPrototype—{2}{U}, 3/3",
        &[
            "Flying".to_string(),
            "Dredge".to_string(),
            "Ward".to_string(),
            "Prototype".to_string(),
        ],
    );

    assert!(card.has_keyword(&KeywordName::Flying));
    assert_eq!(card.keyword_number(&KeywordName::Dredge), Some(5));
    let ward = card
        .keywords
        .iter()
        .find(|keyword| keyword.name == KeywordName::Ward)
        .expect("ward keyword");
    assert!(matches!(
        ward.arguments.as_slice(),
        [KeywordArgument::Number(2)]
    ));
    // The Prototype tail (`—{2}{U}, 3/3`) stays out: a mixed-symbol cost
    // does not decompose into a single numeral, and the size is not a
    // numeric parameter.
    let prototype = card
        .keywords
        .iter()
        .find(|keyword| matches!(&keyword.name, KeywordName::Other(name) if name == "Prototype"))
        .expect("prototype keyword");
    assert!(matches!(prototype.arguments.as_slice(), []));
}

/// Keep static rules and spell effects in distinct AST variants.
#[test]
fn parses_static_and_spell_effect_nodes_separately() {
    let card = parse_oracle_text("Creatures you control get +2/+2.\nDraw two cards.", &[]);

    assert!(card.abilities.iter().any(|ability| matches!(
        ability,
        OracleAbility::Static(static_ability)
            if matches!(static_ability.effects.as_slice(), [StaticEffect::CreatureBuff { power: 2, toughness: 2 }])
    )));
    assert!(card.abilities.iter().any(|ability| matches!(
        ability,
        OracleAbility::Spell(spell)
            if matches!(spell.effects.as_slice(), [OracleEffect::Draw(2)])
    )));
}

/// Cover every keyword name and each supported keyword parameter type.
#[test]
fn parses_every_keyword_name_and_parameter_type() {
    let names = [
        "Flying",
        "Haste",
        "Double strike",
        "Prowess",
        "Trample",
        "Menace",
        "Flash",
        "Undying",
        "Cascade",
        "Crew",
        "Cycling",
        "Basic landcycling",
        "Landcycling",
        "Dredge",
        "Kicker",
        "Flashback",
        "Escape",
        "Transform",
        "Ward",
        "First strike",
        "Deathtouch",
        "Lifelink",
        "Vigilance",
        "Reach",
        "Defender",
        "Indestructible",
        "Hexproof",
        "Protection",
        "Affinity",
        "Improvise",
        "Prototype",
    ];
    let card = parse_oracle_text(
        "Crew 3\nDredge 5\nWard {2}\nKicker {1}{R}\nPrototype—{2}{U}, 3/3",
        &names[..names.len() - 1]
            .iter()
            .map(|name| (*name).to_string())
            .chain(std::iter::once("Prototype".to_string()))
            .collect::<Vec<_>>(),
    );

    for name in [
        KeywordName::Flying,
        KeywordName::Haste,
        KeywordName::DoubleStrike,
        KeywordName::Prowess,
        KeywordName::Trample,
        KeywordName::Menace,
        KeywordName::Flash,
        KeywordName::Undying,
        KeywordName::Cascade,
        KeywordName::Crew,
        KeywordName::Cycling,
        KeywordName::BasicLandcycling,
        KeywordName::Landcycling,
        KeywordName::Dredge,
        KeywordName::Kicker,
        KeywordName::Flashback,
        KeywordName::Escape,
        KeywordName::Transform,
        KeywordName::Ward,
        KeywordName::FirstStrike,
        KeywordName::Deathtouch,
        KeywordName::Lifelink,
        KeywordName::Vigilance,
        KeywordName::Reach,
        KeywordName::Defender,
        KeywordName::Indestructible,
        KeywordName::Hexproof,
        KeywordName::Protection,
        KeywordName::Affinity,
        KeywordName::Improvise,
    ] {
        assert!(card.has_keyword(&name), "missing keyword {name:?}");
    }
    assert_eq!(card.keyword_number(&KeywordName::Crew), Some(3));
    assert_eq!(card.keyword_number(&KeywordName::Dredge), Some(5));
    let ward = card
        .keywords
        .iter()
        .find(|keyword| keyword.name == KeywordName::Ward)
        .expect("ward");
    assert!(matches!(
        ward.arguments.as_slice(),
        [KeywordArgument::Number(2)]
    ));
    let kicker = card
        .keywords
        .iter()
        .find(|keyword| keyword.name == KeywordName::Kicker)
        .expect("kicker");
    assert!(
        matches!(kicker.arguments.as_slice(), []),
        "a multi-symbol kicker cost does not decompose to a single numeral"
    );
    let prototype = card
        .keywords
        .iter()
        .find(|keyword| matches!(&keyword.name, KeywordName::Other(name) if name == "Prototype"))
        .expect("prototype");
    assert!(
        matches!(prototype.arguments.as_slice(), []),
        "the mixed-symbol prototype cost and the 3/3 size stay out"
    );

    let oracle_keyword = parse_oracle_text("Vigilance", &[]);
    assert!(matches!(
        oracle_keyword.keywords[0].arguments.as_slice(),
        []
    ));
}

/// Ability words never parse as keyword names: their labels strip off
/// (CR 702.200+), the trigger behind the label parses, and no
/// `KeywordName` variant carries an ability word.
#[test]
fn ability_words_strip_to_their_trigger_and_stay_out_of_keyword_names() {
    let landfall = parse_oracle_text(
        "Landfall — Whenever a land you control enters, you gain 1 life.",
        &[],
    );
    assert!(
        !landfall.keywords.iter().any(|keyword| matches!(
            &keyword.name,
            KeywordName::Other(name) if name.eq_ignore_ascii_case("landfall")
        )),
        "landfall is an ability word, not a keyword name"
    );
    assert!(
        !matches!(landfall.abilities.as_slice(), []),
        "the trigger behind the landfall label still parses"
    );

    let spellcraft = parse_oracle_text(
        "Spellcraft — Whenever you cast your second spell each turn, draw a card.",
        &[],
    );
    assert!(
        matches!(spellcraft.keywords.as_slice(), []),
        "no ability word becomes a keyword"
    );

    let unlabeled = parse_oracle_text(
        "Constellation — Whenever an enchantment enters, draw a card.",
        &[],
    );
    assert!(matches!(unlabeled.keywords.as_slice(), []));
}

/// The keyword table carries its category tag and stays consistent:
/// every entry maps its own spelling, Transform is a keyword action
/// (CR 701.27), and no entry carries an ability word.
#[test]
fn keyword_table_categories_stay_consistent() {
    use crate::deck::simulator::oracle_parser::keywords::{KEYWORDS, KeywordCategory};

    for entry in KEYWORDS {
        // Every spelling round-trips: the name lookup finds the same entry.
        let parsed = parse_oracle_text(entry.text, &[]);
        let known = parsed
            .keywords
            .first()
            .expect("table entry parses to itself");
        assert!(
            entry.name == known.name,
            "entry {entry:?} does not round-trip to its own variant"
        );
    }
    let transform = KEYWORDS
        .iter()
        .find(|entry| entry.text == "transform")
        .expect("transform in the table");
    assert_eq!(transform.category, KeywordCategory::Action);
    assert!(
        KEYWORDS
            .iter()
            .all(|entry| !entry.text.eq_ignore_ascii_case("landfall")),
        "ability words never appear as KeywordName variants"
    );
    // The prefix collision keeps the longer spelling first: a lookup of
    // "basic landcycling" must not match "landcycling"'s shorter head.
    let basic = KEYWORDS
        .iter()
        .position(|entry| entry.text == "basic landcycling")
        .expect("basic landcycling entry");
    let plain = KEYWORDS
        .iter()
        .position(|entry| entry.text == "landcycling")
        .expect("landcycling entry");
    assert!(basic < plain, "longer spelling must precede its prefix");
}

/// Parse every activation cost form and preserve activation restrictions.
#[test]
fn parses_all_activation_cost_shapes_and_limits() {
    let card = parse_oracle_text(
        "{2}{G}, {T}, Untap another creature you control, Sacrifice two creatures, Pay 3 life, Discard two cards, Remove two +1/+1 counters from this creature: Draw a card. Activate only as a sorcery. Activate only if you control three artifacts. This ability triggers only once each turn.",
        &[],
    );
    let [OracleAbility::Activated(ability)] = card.abilities.as_slice() else {
        panic!("expected activation");
    };

    assert!(ability.costs.iter().any(|cost| matches!(
        cost,
        ActivationCost::Mana(mana) if mana.total() == 3 && mana.pips[4] == 1
    )));
    assert!(
        ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Tap(ActivationTarget::AnotherObject)))
    );
    assert!(
        ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Untap(ActivationTarget::AnotherObject)))
    );
    assert!(ability.costs.iter().any(|cost| matches!(
        cost,
        ActivationCost::Sacrifice {
            count: 2,
            object: CostObject::Creature
        }
    )));
    assert!(
        ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::PayLife(3)))
    );
    assert!(
        ability
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Discard(2)))
    );
    assert!(ability.costs.iter().any(|cost| matches!(
        cost,
        ActivationCost::RemoveCounter { kind: Some(kind), count: 2 }
            if kind == "+1/+1"
    )));
    assert!(
        ability
            .restrictions
            .contains(&AbilityRestriction::SorcerySpeed)
    );
    assert!(
        ability
            .restrictions
            .contains(&AbilityRestriction::OncePerTurn)
    );
    assert!(ability.restrictions.iter().any(|restriction| matches!(
        restriction,
        AbilityRestriction::Condition(condition)
            if condition.contains("three artifacts")
    )));
    assert!(
        ability.to_runtime().is_none(),
        "unknown conditions stay inert"
    );
    assert!(matches!(
        ability.effects.as_slice(),
        [OracleEffect::Draw(1), ..]
    ));

    let untap = parse_oracle_text("{Q}: Draw a card.", &[]);
    let [OracleAbility::Activated(untap)] = untap.abilities.as_slice() else {
        panic!("expected untap activation");
    };
    assert!(
        untap
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Untap(ActivationTarget::Source)))
    );

    let loyalty = parse_oracle_text("−3: Draw two cards.", &[]);
    let [OracleAbility::Activated(loyalty)] = loyalty.abilities.as_slice() else {
        panic!("expected loyalty activation");
    };
    assert!(
        loyalty
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Loyalty(-3)))
    );
    let loyalty_gain = parse_oracle_text("+1: Draw a card.", &[]);
    let [OracleAbility::Activated(loyalty_gain)] = loyalty_gain.abilities.as_slice() else {
        panic!("expected loyalty gain activation");
    };
    assert!(
        loyalty_gain
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Loyalty(1)))
    );

    let other = parse_oracle_text("Exile a card: Draw a card.", &[]);
    let [OracleAbility::Activated(other)] = other.abilities.as_slice() else {
        panic!("expected preserved nonmana cost");
    };
    assert!(
        other
            .costs
            .iter()
            .any(|cost| matches!(cost, ActivationCost::Other(text) if text == "Exile a card"))
    );
    let any_sacrifice = parse_oracle_text("Sacrifice an artifact: Draw a card.", &[]);
    let [OracleAbility::Activated(any_sacrifice)] = any_sacrifice.abilities.as_slice() else {
        panic!("expected artifact sacrifice activation");
    };
    assert!(any_sacrifice.costs.iter().any(|cost| matches!(
        cost,
        ActivationCost::Sacrifice {
            count: 1,
            object: CostObject::Any
        }
    )));
}

/// Parse every supported effect variant and retain unknown effect text.
#[test]
fn parses_every_effect_variant_and_preserves_unsupported_clauses() {
    assert!(matches!(
        activation_effect("Draw two cards."),
        OracleEffect::Draw(2)
    ));
    assert!(matches!(
        activation_effect("Draw two cards, then discard a card."),
        OracleEffect::Loot(2)
    ));
    assert!(matches!(
        activation_effect("Draw a card and put a -1/-1 counter on target creature."),
        OracleEffect::DrawAndMinusCounter
    ));
    assert!(matches!(
        activation_effect("Gain three life."),
        OracleEffect::GainLife(3)
    ));
    assert!(matches!(
        activation_effect("Search your library for a creature card with mana value 3 or less, put it onto the battlefield tapped, then shuffle."),
        OracleEffect::Search(spec)
            if spec.card_type == Some(super::model::SearchCardType::Creature)
                && spec.max_mana_value == Some(3)
                && spec.destination == super::model::SearchDestination::BattlefieldTapped
    ));
    assert!(
        matches!(activation_effect("Add {R}{G}."), OracleEffect::Mana(yield_)
        if yield_.fixed[3] == 1 && yield_.fixed[4] == 1)
    );
    assert!(matches!(
        activation_effect("Untap this creature."),
        OracleEffect::UntapSelf
    ));

    let banked = parse_oracle_text(
        "At the beginning of your first main phase, remove all charge counters from this artifact. Add one mana of any color for each charge counter removed this way.",
        &[],
    );
    let [OracleAbility::Triggered(banked)] = banked.abilities.as_slice() else {
        panic!("expected banked-mana trigger");
    };
    assert!(matches!(
        banked.effects.as_slice(),
        [OracleEffect::ManaPerCounter(yield_)] if yield_.any_pips == 1
    ));

    assert!(matches!(
        activation_effect("Create three Treasure tokens."),
        OracleEffect::Tokens(3)
    ));
    assert!(matches!(
        activation_effect("Put two charge counters on this artifact."),
        OracleEffect::Counters(2)
    ));
    assert!(matches!(
        activation_effect("Put X charge counters on this artifact."),
        OracleEffect::Counters(0)
    ));
    assert!(matches!(
        activation_effect("You may play an additional land this turn."),
        OracleEffect::ExtraLand
    ));
    assert!(matches!(
        activation_effect(
            "Exile this creature, then return it to the battlefield under its owner's control."
        ),
        OracleEffect::Blink
    ));
    assert!(matches!(
        activation_effect("You become the monarch."),
        OracleEffect::Monarch
    ));
    assert!(matches!(
        activation_effect("Mill three cards."),
        OracleEffect::Mill(3)
    ));
    assert!(matches!(
        activation_effect("Return target creature card from your graveyard to your hand."),
        OracleEffect::ReturnFromGraveyard {
            to_hand: true,
            count: 1
        }
    ));
    assert!(matches!(
        activation_effect("Each player discards their hand, then draws seven cards."),
        OracleEffect::Wheel
    ));
    assert!(matches!(
        activation_effect("Take an extra turn after this one."),
        OracleEffect::ExtraTurn
    ));
    assert!(matches!(
        activation_effect("Scry 2."),
        OracleEffect::Look {
            count: 2,
            surveil: false
        }
    ));
    assert!(matches!(
        activation_effect("Surveil 3."),
        OracleEffect::Look {
            count: 3,
            surveil: true
        }
    ));
    assert!(matches!(
        activation_effect("Target opponent loses two life."),
        OracleEffect::Drain(2)
    ));
    assert!(matches!(
        activation_effect("This spell deals 3 damage to target player."),
        OracleEffect::Drain(3)
    ));

    let threshold = parse_oracle_text(
        "At the beginning of your upkeep, if there are 20 or more tower counters on this artifact, you win the game.",
        &[],
    );
    let [OracleAbility::Triggered(threshold)] = threshold.abilities.as_slice() else {
        panic!("expected threshold trigger");
    };
    assert!(matches!(
        threshold.effects.as_slice(),
        [OracleEffect::WinThreshold(20)]
    ));

    assert!(matches!(
        activation_effect("Venture into the dungeon."),
        OracleEffect::Unsupported(text) if text.to_ascii_lowercase().contains("venture")
    ));
    let compound = parse_oracle_text("Draw two cards. Gain three life.", &[]);
    let [OracleAbility::Spell(compound)] = compound.abilities.as_slice() else {
        panic!("expected a spell with multiple effects");
    };
    assert!(matches!(
        compound.effects.as_slice(),
        [OracleEffect::Draw(2), OracleEffect::GainLife(3)]
    ));
    let destroy = parse_oracle_text("Destroy target creature.", &[]);
    let [OracleAbility::Spell(destroy)] = destroy.abilities.as_slice() else {
        panic!("expected an unsupported spell effect node");
    };
    assert!(matches!(
        destroy.effects.as_slice(),
        [OracleEffect::Unsupported(_)]
    ));
}

/// Parse search card types, filters, destinations, and optional choices.
#[test]
fn parses_search_types_filters_destinations_and_optional_choices() {
    use super::model::{SearchCardType as CardType, SearchDestination as Destination};

    let types = [
        ("basic land", CardType::BasicLand),
        ("artifact or enchantment", CardType::ArtifactOrEnchantment),
        ("instant or sorcery", CardType::InstantSorcery),
        ("creature", CardType::Creature),
        ("planeswalker", CardType::Planeswalker),
        ("enchantment", CardType::Enchantment),
        ("artifact", CardType::Artifact),
        ("permanent", CardType::Permanent),
        ("land", CardType::Land),
    ];
    for (card_type_text, expected) in types {
        let spec = search_spec(&format!(
            "Search your library for a {card_type_text} card, put it into your hand, then shuffle."
        ));
        assert_eq!(
            spec.card_type,
            Some(expected),
            "type text {card_type_text:?}"
        );
    }

    let green = search_spec(
        "You may search your library for a green creature card with mana value exactly 3, then shuffle.",
    );
    assert_eq!(green.color, Some('G'));
    assert_eq!(green.mana_value, Some(3));
    assert!(green.optional);
    assert_eq!(green.destination, Destination::Hand);

    let colorless = search_spec(
        "Search your library for a colorless artifact or enchantment card with mana value 4 or more, exile that card, then shuffle.",
    );
    assert!(colorless.colorless);
    assert_eq!(colorless.min_mana_value, Some(4));
    assert_eq!(colorless.destination, Destination::Exile);

    let lower_bound = search_spec(
        "Search your library for a basic land card with mana value 2 or less, put it on top of your library, then shuffle.",
    );
    assert_eq!(lower_bound.max_mana_value, Some(2));
    assert_eq!(lower_bound.destination, Destination::LibraryTop);

    let tapped = search_spec(
        "Search your library for a land card, put it onto the battlefield tapped, then shuffle.",
    );
    assert_eq!(tapped.destination, Destination::BattlefieldTapped);
    let battlefield = search_spec(
        "Search your library for a creature card, put it onto the battlefield, then shuffle.",
    );
    assert_eq!(battlefield.destination, Destination::Battlefield);

    let restricted = search_spec(
        "Look at the top five cards of your library. You may put a non-Human creature card from among them onto the battlefield. Put the rest on the bottom of your library.",
    );
    assert_eq!(restricted.top_count, Some(5));
    assert!(restricted.non_human);
    assert!(restricted.optional);
    assert_eq!(restricted.destination, Destination::Battlefield);
}

/// Parse trigger events, scopes, and object subjects.
#[test]
fn parses_trigger_events_scopes_conditions_and_subjects() {
    let cases = [
        (
            "When this creature enters, draw a card.",
            TriggerEvent::Enters(ObjectSubject::ThisPermanent),
        ),
        (
            "At the beginning of your upkeep, draw a card.",
            TriggerEvent::BeginningOfStep {
                step: TurnStep::Upkeep,
                player: PlayerScope::You,
            },
        ),
        (
            "At the beginning of your end step, draw a card.",
            TriggerEvent::BeginningOfStep {
                step: TurnStep::EndStep,
                player: PlayerScope::You,
            },
        ),
        (
            "At the beginning of your first main phase, draw a card.",
            TriggerEvent::BeginningOfStep {
                step: TurnStep::FirstMainPhase,
                player: PlayerScope::You,
            },
        ),
        (
            "At the beginning of each opponent's upkeep, draw a card.",
            TriggerEvent::BeginningOfStep {
                step: TurnStep::Other,
                player: PlayerScope::Opponent,
            },
        ),
        (
            "At the beginning of each player's upkeep, draw a card.",
            TriggerEvent::BeginningOfStep {
                step: TurnStep::Other,
                player: PlayerScope::Any,
            },
        ),
        (
            "Whenever a creature enters, draw a card.",
            TriggerEvent::Enters(ObjectSubject::Any),
        ),
        (
            "Whenever another creature attacks, draw a card.",
            TriggerEvent::Attacks(ObjectSubject::AnotherPermanent),
        ),
        (
            "Whenever a land you control attacks, draw a card.",
            TriggerEvent::Attacks(ObjectSubject::ControlledLand),
        ),
        (
            "Whenever this creature deals combat damage to a player, draw a card.",
            TriggerEvent::CombatDamageToPlayer(ObjectSubject::ThisPermanent),
        ),
        (
            "Whenever you cast a spell, draw a card.",
            TriggerEvent::CastsSpell { this_spell: false },
        ),
        (
            "When you cast this spell, draw a card.",
            TriggerEvent::CastsSpell { this_spell: true },
        ),
        (
            "Whenever this creature dies, draw a card.",
            TriggerEvent::Dies(ObjectSubject::ThisPermanent),
        ),
        (
            "Whenever a land you control enters, draw a card.",
            TriggerEvent::LandEnters(PlayerScope::You),
        ),
        (
            "Whenever you tap a nonland permanent for mana, draw a card.",
            TriggerEvent::TappedForMana,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(trigger_event(source), expected, "wrong event for {source}");
    }
    assert!(matches!(
        trigger_event("Whenever this creature becomes tapped, draw a card."),
        TriggerEvent::Other(_)
    ));
    assert!(matches!(
        trigger_event("Whenever an opponent's creature dies, draw a card."),
        TriggerEvent::Other(_)
    ));
    assert!(matches!(
        trigger_event("Whenever a creature attacks you, draw a card."),
        TriggerEvent::Other(_)
    ));

    let once = parse_oracle_text(
        "Whenever this creature enters, if you control another creature, draw a card. This ability triggers only once each turn.",
        &[],
    );
    let [OracleAbility::Triggered(once)] = once.abilities.as_slice() else {
        panic!("expected once-per-turn trigger");
    };
    assert!(once.once_per_turn);
    assert_eq!(
        once.condition.as_deref(),
        Some("you control another creature, draw a card")
    );

    let plain = parse_oracle_text("At the beginning of your upkeep, mill three cards.", &[]);
    let [OracleAbility::Triggered(plain)] = plain.abilities.as_slice() else {
        panic!("expected plain trigger");
    };
    assert!(plain.condition.is_none());
}

/// Parse static targets, mana grants, buffs, cost reductions, and limits.
#[test]
fn parses_static_targets_buffs_costs_and_unknown_rules() {
    assert!(matches!(
        static_effect("Creatures you control get +2/-1."),
        StaticEffect::CreatureBuff {
            power: 2,
            toughness: -1
        }
    ));
    assert!(matches!(
        static_effect("Creatures you control have flying."),
        StaticEffect::KeywordGrant { target: StaticTarget::CreaturesYouControl, keyword }
            if keyword.name == KeywordName::Flying
    ));
    assert!(matches!(
        static_effect("Lands you control have vigilance."),
        StaticEffect::KeywordGrant { target: StaticTarget::LandsYouControl, keyword }
            if keyword.name == KeywordName::Vigilance
    ));
    assert!(matches!(
        static_effect("Spells you cast have lifelink."),
        StaticEffect::KeywordGrant { target: StaticTarget::SpellsYouCast, keyword }
            if keyword.name == KeywordName::Lifelink
    ));
    assert!(matches!(
        static_effect("Permanents you control have hexproof."),
        StaticEffect::KeywordGrant { target: StaticTarget::PermanentsYouControl, keyword }
            if keyword.name == KeywordName::Hexproof
    ));
    assert!(matches!(
        static_effect("Creatures you control have \"{T}: Add one mana of any color.\""),
        StaticEffect::ManaGrant { target: StaticTarget::CreaturesYouControl, yield_ }
            if yield_.any_pips == 1
    ));
    assert!(matches!(
        static_effect("Lands you control have \"{T}: Add one mana of any color.\""),
        StaticEffect::ManaGrant { target: StaticTarget::LandsYouControl, yield_ }
            if yield_.any_pips == 1
    ));
    assert!(matches!(
        static_effect("Permanents you control have \"{T}: Add {C}.\""),
        StaticEffect::ManaGrant { target: StaticTarget::PermanentsYouControl, yield_ }
            if yield_.colorless == 1
    ));
    assert!(matches!(
        static_effect("Artifact spells you cast cost {1} less to cast."),
        StaticEffect::CostReduction { amount: 1, spell_class }
            if spell_class == "artifact spells you cast"
    ));
    assert!(matches!(
        static_effect("You may play an additional land on each of your turns."),
        StaticEffect::AdditionalLandDrop
    ));
    assert!(matches!(
        static_effect("If there are 20 or more tower counters on this artifact, you win the game."),
        StaticEffect::WinThreshold(20)
    ));
    let unsupported =
        static_effect("As long as you control a creature, this permanent has flying.");
    assert!(
        matches!(
            unsupported,
            StaticEffect::Unsupported(ref text) if text.to_ascii_lowercase().contains("as long as")
        ),
        "unexpected static node: {unsupported:?}"
    );
}

/// Preserve incomplete or unknown ability statements without guessing effects.
#[test]
fn preserves_unsupported_ability_shapes_without_inventing_effects() {
    let activated = parse_oracle_text("{T} Draw a card", &[]);
    let [OracleAbility::Unsupported(activated)] = activated.abilities.as_slice() else {
        panic!("expected incomplete activation to remain unsupported");
    };
    assert_eq!(activated.kind, UnsupportedAbilityKind::Activated);
    assert_eq!(activated.source, "{T} Draw a card");

    let triggered = parse_oracle_text("Whenever a permanent does something", &[]);
    let [OracleAbility::Unsupported(triggered)] = triggered.abilities.as_slice() else {
        panic!("expected incomplete trigger to remain unsupported");
    };
    assert_eq!(triggered.kind, UnsupportedAbilityKind::Triggered);

    let unknown = parse_oracle_text("Venture into the dungeon.", &[]);
    let [OracleAbility::Unsupported(unknown)] = unknown.abilities.as_slice() else {
        panic!("expected unknown statement to remain unsupported");
    };
    assert_eq!(unknown.kind, UnsupportedAbilityKind::Unknown);
}

/// Preserve combined Saga chapters and unsupported chapter text.
#[test]
fn parses_combined_saga_chapters_in_order_and_keeps_unsupported_text() {
    let card = parse_oracle_text("I, II — Draw a card.\nIII — Venture into the dungeon.", &[]);
    let chapters = card.saga_chapters().collect::<Vec<&SagaChapter>>();
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].chapters, [1, 2]);
    assert!(matches!(
        chapters[0].effects.as_slice(),
        [OracleEffect::Draw(1)]
    ));
    assert_eq!(chapters[1].chapters, [3]);
    assert!(matches!(
        chapters[1].effects.as_slice(),
        [OracleEffect::Unsupported(text)] if text.to_ascii_lowercase().contains("venture")
    ));
}
