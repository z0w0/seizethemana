//! Parser and lowering regressions for complete ability requirements.

use super::*;
use crate::deck::simulator::deck_test_support::card;
use crate::deck::simulator::model::{Grant, LifeLossScope, XClass};
use crate::deck::simulator::oracle_parser::{parse_oracle_card, parse_oracle_text};
use crate::deck::simulator::support::AbilitySupportStatus;

/// Source-local activation costs, mana requirements, and charge-counter grammar.
#[path = "activation_cost_tests.rs"]
mod activation_cost_tests;

/// Parse one trigger without mixing parser checks with game execution.
fn trigger(text: &str) -> OracleTriggeredAbility {
    let oracle = parse_oracle_text(text, &[]);
    let [OracleAbility::Triggered(trigger)] = oracle.abilities.as_slice() else {
        panic!("expected one trigger: {:#?}", oracle.abilities);
    };
    trigger.clone()
}

/// A paid tribal draw retains both its event filter and mana payment.
#[test]
fn cast_filters_and_optional_controller_payments_survive_lowering() {
    let parsed =
        trigger("Whenever you cast an Elf spell, you may pay {G}. If you do, draw a card.");
    assert_eq!(parsed.event_filter.subtypes, ["elf"]);
    let lowered = parsed.to_runtime_all();
    let [ability] = lowered.as_slice() else {
        panic!("expected paid draw");
    };
    assert_eq!(ability.event_filter.subtypes, ["elf"]);
    assert!(
        matches!(ability.resolution_payment, Some(SimResolutionPayment::PayMana(cost)) if cost.pips[4] == 1)
    );
    assert!(matches!(ability.effect_sequence(), [SimEffect::Draw(1)]));

    let creature = trigger("Whenever you cast a creature spell, draw a card.");
    assert_eq!(
        creature.to_runtime_all()[0].event_filter.card_types,
        ["creature"]
    );
}

/// Unsupported requirements cannot leave a supported reward behind.
#[test]
fn unsupported_payments_and_compound_effects_do_not_grant_free_rewards() {
    for text in [
        "Whenever you cast an Elf spell, you may pay 2 life. If you do, draw a card.",
        "Whenever this creature attacks, draw a card and destroy target artifact.",
        "Whenever this creature attacks, draw a card and sacrifice a creature.",
        "Whenever this creature attacks, draw a card and venture into the dungeon.",
        "Whenever this creature attacks, draw two cards, then discard a card unless you pay 1 life.",
        "Whenever this creature attacks, draw two cards, then discard a card. Sacrifice a creature.",
        "Whenever this creature attacks, draw a card and discard a card. Sacrifice a creature.",
        "Whenever this creature attacks, draw a card and put a -1/-1 counter on target creature.",
    ] {
        assert!(trigger(text).to_runtime_all().is_empty(), "{text}");
    }
}

/// Unequal fixed counts retain the source event without adding a payment or free draw.
#[test]
fn fixed_unequal_draw_discard_triggers_keep_event_and_counts() {
    let hand_built = OracleTriggeredAbility {
        event: OracleTriggerEvent::Attacks(ObjectSubject::ThisPermanent),
        effects: vec![OracleEffect::DrawDiscard {
            draw: 2,
            discard: 1,
        }],
        is_mana_ability: false,
        once_per_turn: false,
        condition: None,
        event_filter: Default::default(),
        resolution_payment: None,
    };
    let parsed = trigger("Whenever this creature attacks, draw two cards, then discard a card.");
    assert_eq!(parsed.event, hand_built.event);
    assert!(matches!(
        parsed.effects.as_slice(),
        [OracleEffect::DrawDiscard {
            draw: 2,
            discard: 1
        }]
    ));
    for ability in [hand_built, parsed] {
        let lowered = ability.to_runtime_all();
        let [lowered] = lowered.as_slice() else {
            panic!("one complete draw-discard trigger");
        };
        assert_eq!(lowered.kind, SimAbilityKind::Triggered);
        assert_eq!(lowered.trigger, SimTrigger::Attacks);
        assert_eq!(lowered.event_subject, SimEventSubject::This);
        assert!(lowered.activation.is_none());
        assert!(lowered.resolution_payment.is_none());
        assert!(matches!(
            lowered.effect_sequence(),
            [SimEffect::DrawDiscard {
                draw: 2,
                discard: 1
            }]
        ));
    }
}

/// Lower simple static mana grants only for the exact selected permanent class.
#[test]
fn simple_land_and_creature_mana_grants_lower_to_runtime_flags() {
    for (type_line, text, expected) in [
        (
            "Artifact",
            "Lands you control have \"{T}: Add one mana of any color.\"",
            Grant::Lands,
        ),
        (
            "Enchantment",
            "Creatures you control have \"{T}: Add one mana of any color.\"",
            Grant::Creatures,
        ),
    ] {
        let lowered = parse_sim_card(&card("Mana Grant", "{2}", type_line, text));
        assert_eq!(lowered.flags.grant, Some(expected), "{text}");
    }
    let fixed_mana = parse_sim_card(&card(
        "Fixed Grant",
        "{2}",
        "Artifact",
        "Lands you control have \"{T}: Add {G}.\"",
    ));
    assert!(fixed_mana.flags.grant.is_none());
}

/// References in the resolution cannot change the triggering object.
#[test]
fn entry_filters_do_not_take_the_source_from_the_resolution() {
    let trigger = trigger(
        "Whenever another nontoken Elf creature you control enters, put a charge counter on this artifact.",
    );
    assert_eq!(
        trigger.event,
        OracleTriggerEvent::Enters(ObjectSubject::AnotherPermanent)
    );
    let lowered = trigger.to_runtime_all();
    let [ability] = lowered.as_slice() else {
        panic!("expected filtered entry trigger");
    };
    assert_eq!(ability.event_subject, SimEventSubject::Another);
    assert!(ability.event_filter.nontoken);
    assert_eq!(ability.event_filter.card_types, ["creature"]);
    assert_eq!(ability.event_filter.subtypes, ["elf"]);
}

/// Unmodeled event qualifiers must not become unconditional triggers.
#[test]
fn unknown_event_qualifiers_gate_the_complete_trigger() {
    for text in [
        "Whenever you cast a creature spell from exile, draw a card.",
        "Whenever another creature with power 4 or greater enters under your control, draw a card.",
        "Whenever this creature attacks alone, draw a card.",
        "Whenever this creature or another creature enters under your control, draw a card.",
        "Whenever a creature enters, draw a card.",
    ] {
        assert!(trigger(text).to_runtime_all().is_empty(), "{text}");
    }
}

/// Source normalization keeps the original wording available for diagnostics.
#[test]
fn named_and_abbreviated_source_references_preserve_original_spans() {
    for name in ["Tara, Grove Keeper", "Sera, Grove Keeper"] {
        let short = name.split_once(',').expect("legendary name").0;
        let text = format!(
            "Whenever {short} attacks, draw a card.\nWhenever {name} deals combat damage to a player, gain 2 life."
        );
        let row = card(name, "{2}{G}", "Legendary Creature — Elf Druid", &text);
        let oracle = parse_oracle_card(&row);
        assert_eq!(oracle.abilities.len(), 2);
        for (ability, source) in oracle.abilities.iter().zip(&oracle.ability_sources) {
            let OracleAbility::Triggered(ability) = ability else {
                panic!("expected source trigger");
            };
            assert_eq!(ability.event.event_subject(), SimEventSubject::This);
            let span = source.span.expect("original byte span");
            assert_eq!(source.text, text[span.start..span.end]);
            assert!(source.text.contains(short));
        }
    }
}

/// Unknown counters and amounts cannot become convenient fixed effects.
#[test]
fn unknown_counter_kinds_and_variable_token_counts_stay_inert() {
    for resolution in [
        "put a stun counter on this artifact",
        "put a +1/+1 counter on target creature",
        "create that many 1/1 green Elf Warrior creature tokens",
    ] {
        let trigger = trigger(&format!("Whenever this creature attacks, {resolution}."));
        assert!(matches!(
            trigger.effects.as_slice(),
            [OracleEffect::Unsupported(_)]
        ));
        assert!(trigger.to_runtime_all().is_empty());
    }
    assert!(matches!(
        trigger("Whenever this creature attacks, put two charge counters on this artifact.")
            .to_runtime_all()[0]
            .effect,
        SimEffect::Counters(2)
    ));
    let tokens =
        trigger("Whenever this creature attacks, create three 1/1 green Elf creature tokens.");
    assert!(matches!(
        tokens.effects.as_slice(),
        [OracleEffect::Tribal(
            crate::deck::simulator::oracle_ast::tribal::OracleTribalEffect::CreateCreatureTokens {
                quantity: crate::deck::simulator::oracle_ast::tribal::OracleQuantity::Fixed(3),
                ..
            }
        )]
    ));
    assert!(matches!(
        tokens.to_runtime_all().as_slice(),
        [ability] if matches!(ability.effect, SimEffect::Tokens(3))
    ));
}

/// Tax amounts and their triggering events stay distinct during lowering.
#[test]
fn opponent_draw_and_cast_taxes_keep_typed_amounts_and_event_kinds() {
    for (text, expected_trigger, expected_tax) in [
        (
            "Whenever an opponent casts a spell, you may draw a card unless that player pays {1}.",
            SimTrigger::OpponentSpellCast,
            OpponentTax::Fixed(1),
        ),
        (
            "Whenever an opponent casts a noncreature spell, you may draw a card unless that player pays {4}.",
            SimTrigger::OpponentNoncreatureCast,
            OpponentTax::Fixed(4),
        ),
        (
            "Whenever an opponent draws a card, that player may pay {2}. If the player doesn't, you create a Treasure token.",
            SimTrigger::OpponentDraw,
            OpponentTax::Fixed(2),
        ),
    ] {
        let abilities = trigger(text).to_runtime_all();
        let [ability] = abilities.as_slice() else {
            panic!("expected opponent tax: {text}");
        };
        assert_eq!(ability.trigger, expected_trigger);
        assert_eq!(ability.opponent_tax, Some(expected_tax));
    }
    let row = card(
        "Renamed Sentinel",
        "{W}",
        "Artifact Creature — Human Soldier",
        "Whenever an opponent casts their first noncreature spell each turn, draw a card unless that player pays {X}, where X is Renamed Sentinel's power.",
    );
    let oracle = parse_oracle_card(&row);
    let abilities = oracle.runtime_abilities();
    assert_eq!(
        abilities[0].trigger,
        SimTrigger::OpponentFirstNoncreatureCast
    );
    assert_eq!(abilities[0].opponent_tax, Some(OpponentTax::SourcePower));
}

/// Unsupported upkeep payments disable dependent opponent rewards.
#[test]
fn generic_cumulative_upkeep_is_required_for_dependent_rewards() {
    for (cost, expected) in [
        ("{1}", true),
        ("{U}", false),
        ("{W/U}", false),
        ("{S}", false),
        ("Pay 1 life", false),
    ] {
        let text = format!(
            "Cumulative upkeep {cost}\nWhenever an opponent casts a noncreature spell, you may draw a card unless that player pays {{4}}."
        );
        let row = card("Renamed Remora", "{U}", "Enchantment", &text);
        let oracle = parse_oracle_card(&row);
        assert_eq!(
            oracle.static_data.cumulative_upkeep.is_some(),
            expected,
            "{cost}"
        );
        assert_eq!(!oracle.runtime_abilities().is_empty(), expected, "{cost}");
    }
}

/// Support records describe emitted behavior and preserve source provenance.
#[test]
fn support_records_use_lowering_evidence_and_original_source() {
    let text = "Whenever this creature attacks, draw a card.\nWhenever this creature attacks, draw a card and destroy target artifact.\nWhenever this creature attacks, create two 1/1 green Elf creature tokens.";
    let row = card("Support Example", "{G}", "Creature — Elf", text);
    let lowered = parse_sim_card(&row);
    assert!(matches!(
        lowered.support[0].status,
        AbilitySupportStatus::Supported
    ));
    assert!(matches!(
        lowered.support[1].status,
        AbilitySupportStatus::Unsupported { .. }
    ));
    assert!(
        matches!(
            lowered.support[2].status,
            AbilitySupportStatus::Approximated {
                rule_id: "generic_creature_tokens"
            }
        ),
        "{:?}",
        lowered.support[2]
    );
    for support in &lowered.support {
        let span = support.span.expect("printed ability span");
        assert_eq!(support.source, text[span.start..span.end]);
    }
    assert_eq!(lowered.striations[0].abilities.len(), 2);
}

/// Ordered spell effects must lower as one complete resolution.
#[test]
fn ordered_spell_effects_lower_together_or_not_at_all() {
    let row = card(
        "Ordered Example",
        "{G}",
        "Sorcery",
        "Draw a card, then gain 2 life.",
    );
    let lowered = parse_sim_card(&row);
    assert!(matches!(
        lowered.spell_data.effects.as_slice(),
        [SimEffect::Draw(1), SimEffect::GainLife(2)]
    ));
    let row = card(
        "Unsupported Example",
        "{G}",
        "Sorcery",
        "Draw a card, then destroy target artifact.",
    );
    let lowered = parse_sim_card(&row);
    assert!(lowered.spell_data.effects.is_empty());
    assert_eq!(lowered.spell_data.draws_on_cast, 0);
}

/// A recognized mana restriction is an annotation, not a dropped drawback.
#[test]
fn mana_spending_annotations_require_a_complete_recognized_restriction() {
    let known = parse_oracle_text(
        "{T}: Add {G}. Spend this mana only to cast creature spells.",
        &[],
    );
    assert_eq!(known.runtime_abilities().len(), 1);
    let unknown = parse_oracle_text(
        "{T}: Add {G}. Spend this mana only to cast creature or planeswalker spells.",
        &[],
    );
    assert!(unknown.runtime_abilities().is_empty());
}

/// Filter matching distinguishes card types, creature subtypes, and tokens.
#[test]
fn event_filters_accept_only_matching_card_properties() {
    let parsed = trigger("Whenever another nontoken Elf creature you control enters, draw a card.");
    let filter = &parsed.to_runtime_all()[0].event_filter;
    let elf = parse_sim_card(&card("Elf", "{G}", "Creature — Elf Druid", ""));
    let human = parse_sim_card(&card("Human", "{G}", "Creature — Human Druid", ""));
    let artifact = parse_sim_card(&card("Artifact", "{1}", "Artifact", ""));
    assert!(filter.matches_card(&elf, false));
    assert!(!filter.matches_card(&elf, true));
    assert!(!filter.matches_card(&human, false));
    assert!(!filter.matches_card(&artifact, false));
    let instant = parse_sim_card(&card("Instant", "{U}", "Instant", ""));
    let sorcery = parse_sim_card(&card("Sorcery", "{U}", "Sorcery", ""));
    let instant_trigger = trigger("Whenever you cast an instant spell, draw a card.");
    let instant_filter = &instant_trigger.to_runtime_all()[0].event_filter;
    assert!(instant_filter.matches_card(&instant, false));
    assert!(!instant_filter.matches_card(&sorcery, false));
}

/// Compiled spell effects are not runtime support evidence by themselves.
#[test]
fn ordinary_spell_support_stays_compiler_only_until_execution_is_verified() {
    let spell = parse_sim_card(&card("Draw Spell", "{U}", "Sorcery", "Draw a card."));
    assert!(matches!(
        spell.support[0].status,
        AbilitySupportStatus::CompilerOnly { .. }
    ));
}

/// Full-card lowering shares exact mana-spending grammar with the parser.
#[test]
fn singular_and_plural_spend_limits_lower_without_losing_restrictions() {
    use crate::deck::simulator::model::SpendRestriction;
    for class in ["an artifact spell", "artifact spells"] {
        let row = card(
            "Restricted Stone",
            "{1}",
            "Artifact",
            &format!("{{T}}: Add {{G}}. Spend this mana only to cast {class}."),
        );
        let lowered = parse_sim_card(&row);
        assert_eq!(
            lowered.tap.as_ref().and_then(|yield_| yield_.restriction),
            Some(SpendRestriction::Artifact)
        );
        let abilities = &lowered.striations[0].abilities;
        assert_eq!(abilities.len(), 1);
        assert!(
            matches!(abilities[0].effect_sequence(), [SimEffect::Mana(yield_)] if yield_.restriction == Some(SpendRestriction::Artifact))
        );
    }
    for class in [
        "creature spells of the chosen type",
        "creature or planeswalker spells",
    ] {
        let row = card(
            "Unmodeled Stone",
            "{1}",
            "Artifact",
            &format!("{{T}}: Add {{G}}. Spend this mana only to cast {class}."),
        );
        let lowered = parse_sim_card(&row);
        assert!(lowered.tap.is_none());
        assert!(
            lowered
                .striations
                .iter()
                .all(|striation| striation.abilities.is_empty())
        );
    }
}

/// Source hashes track exact relevant metadata without print-specific details.
#[test]
fn source_hash_is_stable_and_changes_with_simulation_inputs() {
    let row = card(
        "Hash Example",
        "{U}",
        "Creature — Merfolk",
        "When this creature enters, draw a card.",
    );
    let original = parse_sim_card(&row).source_hash;
    assert_eq!(original.len(), 64);
    assert_eq!(parse_sim_card(&row).source_hash, original);
    let mut changed = row.clone();
    changed.oracle_text.push('\n');
    assert_ne!(parse_sim_card(&changed).source_hash, original);
    let mut changed = row.clone();
    changed.color_identity = "[\"U\"]".to_string();
    assert_ne!(parse_sim_card(&changed).source_hash, original);
    let mut changed = row.clone();
    changed.oracle_id = "different-oracle-id".to_string();
    assert_ne!(parse_sim_card(&changed).source_hash, original);
    let mut changed = row;
    changed.set_code = "different-print".to_string();
    assert_eq!(parse_sim_card(&changed).source_hash, original);
}

/// Unsupported dispatchers cannot give rewards that require controller payment.
#[test]
fn noncast_controller_payments_stay_inert_and_report_unsupported() {
    for event in [
        "When this creature enters",
        "Whenever this creature attacks",
        "At the beginning of your upkeep",
        "Whenever an opponent draws a card",
    ] {
        let row = card(
            "Paid Source",
            "{G}",
            "Creature — Elf",
            &format!("{event}, you may pay {{G}}. If you do, draw a card."),
        );
        let lowered = parse_sim_card(&row);
        assert!(
            lowered
                .striations
                .iter()
                .all(|striation| striation.abilities.is_empty()),
            "{event}"
        );
        assert!(
            matches!(
                lowered.support[0].status,
                AbilitySupportStatus::Unsupported { .. }
            ),
            "{event}"
        );
    }
}

/// Search consequences remain ordered and unknown linked payments gate the spell.
#[test]
fn search_retains_life_loss_and_unknown_sacrifice_requirements() {
    let row = card(
        "Linked Search",
        "{B}",
        "Sorcery",
        "Search your library for a card, put it into your hand, then shuffle. Sacrifice a creature.",
    );
    let lowered = parse_sim_card(&row);
    assert!(lowered.spell_data.effects.is_empty());
    assert!(matches!(
        lowered.support[0].status,
        AbilitySupportStatus::Unsupported { .. }
    ));
    let row = card(
        "Linked Search",
        "{B}",
        "Sorcery",
        "Search your library for a card, put it into your hand, then sacrifice a creature.",
    );
    assert!(parse_sim_card(&row).spell_data.effects.is_empty());
}

/// A known cast cost is not a resolution effect; an unknown cost blocks casting.
#[test]
fn additional_cast_requirements_are_complete_before_rewards_lower() {
    let known = card(
        "Village Rites",
        "{B}",
        "Instant",
        "As an additional cost to cast this spell, sacrifice a creature.\nDraw two cards.",
    );
    let lowered = parse_sim_card(&known);
    assert_eq!(lowered.spell_data.additional_cost_creatures, 1);
    assert!(matches!(
        lowered.spell_data.effects.as_slice(),
        [SimEffect::Draw(2)]
    ));
    let unknown = card(
        "Unmodeled Draw",
        "{B}",
        "Instant",
        "As an additional cost to cast this spell, sacrifice a creature and exile a card from your graveyard.\nDraw two cards.",
    );
    let lowered = parse_sim_card(&unknown);
    assert!(lowered.cost.unsupported);
    assert!(lowered.spell_data.unsupported_additional_cost);
    assert!(lowered.spell_data.effects.is_empty());
}

/// Land and player-attack dispatchers reject qualifiers they cannot evaluate.
#[test]
fn qualified_land_and_attack_events_do_not_become_unconditional() {
    for text in [
        "Whenever a basic land you control enters, draw a card.",
        "Whenever a land enters under your control during your turn, draw a card.",
        "Whenever you attack with three or more creatures, draw a card.",
    ] {
        assert!(trigger(text).to_runtime_all().is_empty(), "{text}");
    }
    assert_eq!(
        trigger("Whenever a land you control enters, draw a card.")
            .to_runtime_all()
            .len(),
        1
    );
    assert_eq!(
        trigger("Whenever you attack, draw a card.")
            .to_runtime_all()
            .len(),
        1
    );
}

/// Noncreature printed subtypes are available to the same event predicate.
#[test]
fn subtype_filters_match_aura_equipment_vehicle_and_saga() {
    for (type_line, subtype) in [
        ("Enchantment — Aura", "aura"),
        ("Artifact — Equipment", "equipment"),
        ("Artifact — Vehicle", "vehicle"),
        ("Enchantment — Saga", "saga"),
    ] {
        let card = parse_sim_card(&card("Subtype", "{1}", type_line, ""));
        let filter = SimEventFilter {
            subtypes: vec![subtype.to_string()],
            ..Default::default()
        };
        assert!(filter.matches_card(&card, false), "{type_line}");
    }
}

/// Only supported spell X classes can create variable runtime amounts.
#[test]
fn x_markers_require_a_supported_owning_spell() {
    for (text, class) in [
        ("Draw X cards.", XClass::Draw),
        ("Mill X cards.", XClass::Mill),
        ("Create X 1/1 green Elf creature tokens.", XClass::Tokens),
        ("Create X Treasure tokens.", XClass::Treasures),
        ("Target player loses X life.", XClass::Drain),
    ] {
        let variable = parse_sim_card(&card("Variable Spell", "{X}{B}", "Sorcery", text));
        assert!(
            matches!(
                (class, variable.spell_data.effects.as_slice()),
                (XClass::Draw, [SimEffect::Draw(0)])
                    | (XClass::Mill, [SimEffect::Mill(0)])
                    | (XClass::Tokens, [SimEffect::Tokens(0)])
                    | (XClass::Treasures, [SimEffect::Treasures(0)])
                    | (
                        XClass::Drain,
                        [SimEffect::LoseLife {
                            amount: 0,
                            scope: LifeLossScope::TargetPlayer
                        }]
                    )
            ),
            "{text}"
        );
        let unbound = parse_sim_card(&card("Unbound Spell", "{B}", "Sorcery", text));
        assert!(unbound.spell_data.effects.is_empty(), "{text}");
        assert!(
            matches!(
                unbound.support[0].status,
                AbilitySupportStatus::Unsupported { .. }
            ),
            "{text}"
        );
    }
    let unknown = parse_sim_card(&card(
        "Unknown Amount",
        "{X}{B}",
        "Sorcery",
        "Target player loses that much life.",
    ));
    assert!(unknown.spell_data.effects.is_empty());
    assert!(
        trigger("Whenever this creature attacks, target player loses X life.")
            .to_runtime_all()
            .is_empty()
    );
}

/// One combined source event emits one complete effect sequence per timing.
#[test]
fn combined_source_events_keep_both_timings_without_repeating_effects() {
    for (name, text, second, amount) in [
        (
            "Stitcher's Supplier",
            "When Stitcher's Supplier enters or dies, mill three cards.",
            SimTrigger::Dies,
            3,
        ),
        (
            "Overlord of the Balemurk",
            "Whenever Overlord of the Balemurk enters or attacks, mill four cards.",
            SimTrigger::Attacks,
            4,
        ),
    ] {
        let lowered = parse_sim_card(&card(name, "{B}", "Creature — Zombie", text));
        let abilities: Vec<_> = lowered.unlocked_abilities(0).collect();
        assert_eq!(abilities.len(), 2, "{name}");
        assert_eq!(abilities[0].trigger, SimTrigger::Enters);
        assert_eq!(abilities[1].trigger, second);
        for ability in abilities {
            assert_eq!(ability.event_subject, SimEventSubject::This);
            assert!(
                matches!(ability.effect_sequence(), [SimEffect::Mill(count)] if *count == amount)
            );
        }
        assert_eq!(lowered.support.len(), 1);
        assert_eq!(lowered.support[0].source, text);
    }
    for event in [
        "this creature enters or attacks alone",
        "another creature enters or dies",
        "this creature or another creature enters or dies",
    ] {
        assert!(
            trigger(&format!("Whenever {event}, draw a card."))
                .to_runtime_all()
                .is_empty(),
            "{event}"
        );
    }
}

/// Tapped entry applies to artifacts as well as lands, without tapping controls.
#[test]
fn nonland_permanents_keep_their_printed_tapped_entry() {
    for (name, entry, tapped) in [
        ("Timeless Lotus", "Timeless Lotus enters tapped.\n", true),
        ("Untapped Artifact", "", false),
    ] {
        let row = card(
            name,
            "{5}",
            "Legendary Artifact",
            &format!("{entry}{{T}}: Add {{W}}{{U}}{{B}}{{R}}{{G}}."),
        );
        let lowered = parse_sim_card(&row);
        assert_eq!(lowered.enters_tapped, tapped);
        assert!(!lowered.is_land);
        assert_eq!(lowered.tap.expect("fixed five-color output").fixed, [1; 5]);
    }
}

/// Static land allowances keep their Oracle counts and reject unknown qualifiers.
#[test]
fn additional_land_allowances_keep_exact_counts() {
    for (name, text, expected) in [
        (
            "Azusa, Lost but Seeking",
            "You may play two additional lands on each of your turns.",
            2,
        ),
        (
            "Exploration",
            "You may play an additional land on each of your turns.",
            1,
        ),
        (
            "Qualified Allowance",
            "You may play two additional lands on each of your turns if you control a creature.",
            0,
        ),
        (
            "Variable Allowance",
            "You may play X additional lands on each of your turns.",
            0,
        ),
    ] {
        let row = card(name, "{G}", "Enchantment", text);
        assert_eq!(
            parse_sim_card(&row).flags.extra_land_drops,
            expected,
            "{text}"
        );
    }
}

/// Linked searches retain the mana-value rule rather than emitting a generic search.
#[test]
fn sacrifice_search_lowering_uses_only_the_typed_resolution_route() {
    for (search, offset, lower, counter) in [
        (
            "Search your library for a creature card with mana value equal to 1 plus the sacrificed creature's mana value, put that card onto the battlefield with an additional +1/+1 counter on it, then shuffle.",
            1,
            false,
            true,
        ),
        (
            "Search your library for a creature card with mana value X or less, where X is 2 plus the sacrificed creature's mana value, put that card onto the battlefield, then shuffle.",
            2,
            true,
            false,
        ),
    ] {
        let row = card(
            "Linked Search",
            "{G}{U}",
            "Sorcery",
            &format!("As an additional cost to cast this spell, sacrifice a creature.\n{search}"),
        );
        let oracle = parse_oracle_card(&row);
        let lowered = parse_sim_card(&row);
        let rule = lowered
            .spell_data
            .search_after_sacrifice
            .expect("linked search metadata");
        assert_eq!(rule.mana_value_offset, offset);
        assert_eq!(rule.allow_lower_mana_value, lower);
        assert_eq!(rule.enters_with_plus1_counter, counter);
        assert!(oracle.abilities.iter().any(|ability| matches!(ability, OracleAbility::Spell(spell)
            if spell.effects.iter().any(|effect| matches!(effect, OracleEffect::SacrificeSearch(parsed) if *parsed == rule)))));
        assert!(lowered.spell_data.effects.is_empty());
        assert!(
            lowered
                .support
                .iter()
                .all(|record| matches!(record.status, AbilitySupportStatus::CompilerOnly { .. }))
        );
    }
    let unknown = card(
        "Unknown Linked Search",
        "{G}",
        "Sorcery",
        "As an additional cost to cast this spell, sacrifice a creature.\nSearch your library for a creature card with mana value greater than the sacrificed creature's mana value, put it onto the battlefield, then shuffle.",
    );
    let lowered = parse_sim_card(&unknown);
    assert!(lowered.spell_data.search_after_sacrifice.is_none());
    assert!(lowered.spell_data.effects.is_empty());
    assert!(
        lowered
            .support
            .iter()
            .all(|record| matches!(record.status, AbilitySupportStatus::Unsupported { .. }))
    );
}

/// Waiving printed mana does not waive an unsupported mandatory additional cost.
#[test]
fn mandatory_additional_cost_support_is_separate_from_printed_mana_support() {
    let required = parse_sim_card(&card(
        "Skaab Ruinator",
        "{1}{U}{U}",
        "Creature — Zombie Horror",
        "As an additional cost to cast this spell, exile three creature cards from your graveyard.\nFlying\nYou may cast this card from your graveyard.",
    ));
    assert!(required.spell_data.unsupported_additional_cost);
    let unsupported_mana =
        parse_sim_card(&card("Unknown Mana Symbol", "{Z}", "Creature — Elf", ""));
    assert!(unsupported_mana.cost.unsupported);
    assert!(!unsupported_mana.spell_data.unsupported_additional_cost);
}

/// Artifact affinity needs its exact Oracle scope, not only the keyword name.
#[test]
fn affinity_flags_require_the_artifact_scope_and_keep_other_scopes() {
    use crate::deck::simulator::oracle_ast::{KeywordArgument, OracleKeywordName};
    for (scope, supported) in [
        ("artifacts", true),
        ("Forests", false),
        ("artifact creatures", false),
        ("artifacts and creatures", false),
        ("artifacts in your graveyard", false),
    ] {
        let mut row = card(
            "Scoped Affinity",
            "{7}",
            "Artifact Creature — Golem",
            &format!(
                "Affinity for {scope} (This spell costs {{1}} less to cast for each matching permanent you control.)"
            ),
        );
        row.keywords = "[\"Affinity\"]".to_string();
        let oracle = parse_oracle_card(&row);
        let affinity = oracle
            .keywords
            .iter()
            .find(|keyword| keyword.name == OracleKeywordName::Affinity)
            .expect("affinity keyword");
        assert!(
            matches!(affinity.arguments.as_slice(), [KeywordArgument::AffinityFor(parsed)] if parsed == &scope.to_ascii_lowercase()),
            "{scope}"
        );
        let lowered = parse_sim_card(&row);
        assert_eq!(lowered.keyword_abilities.affinity, supported, "{scope}");
        assert_eq!(lowered.battlefield_discount, supported, "{scope}");
        assert!(!lowered.keyword_abilities.improvise);
        assert_eq!(lowered.min_cost.generic, 7);
    }
    let mut bare = card("Unscoped Affinity", "{7}", "Artifact Creature — Golem", "");
    bare.keywords = "[\"Affinity\"]".to_string();
    assert!(!parse_sim_card(&bare).keyword_abilities.affinity);
}

/// A return keeps its complete card class, destination, and entry requirements.
#[test]
fn graveyard_returns_reject_unmodeled_filters_and_entry_rules() {
    for text in [
        "Return target creature card from your graveyard to your hand.",
        "Return target land card from your graveyard to the battlefield.",
        "Return target creature card from your graveyard to the battlefield tapped.",
        "Return up to one target creature card from your graveyard to the battlefield.",
        "Return two creature cards from your graveyard to the battlefield.",
        "Return target creature card with mana value 3 or less from your graveyard to the battlefield.",
        "Return target artifact card from your graveyard to your hand.",
        "Return this card from your graveyard to your hand.",
    ] {
        let lowered = parse_sim_card(&card("Unmodeled Return", "{B}", "Sorcery", text));
        assert!(lowered.spell_data.effects.is_empty(), "{text}");
        assert!(
            matches!(
                lowered.support[0].status,
                AbilitySupportStatus::Unsupported { .. }
            ),
            "{text}"
        );
    }
    for (text, to_hand) in [
        ("Return target card from your graveyard to your hand.", true),
        (
            "You may return target card from your graveyard to your hand.",
            true,
        ),
        (
            "Return target creature card from your graveyard to the battlefield.",
            false,
        ),
    ] {
        let lowered = parse_sim_card(&card("Supported Return", "{B}", "Sorcery", text));
        assert!(
            matches!(lowered.spell_data.effects.as_slice(), [SimEffect::ReturnFromGraveyard { to_hand: actual, count: 1 }] if *actual == to_hand),
            "{text}"
        );
    }
}
