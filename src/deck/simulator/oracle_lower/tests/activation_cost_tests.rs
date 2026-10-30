//! Activation requirements remain attached to their source-local effects.

use super::*;

/// Metalcraft survives lowering only as the exact supported activation gate.
#[test]
fn metalcraft_mana_keeps_its_condition_and_rejects_extra_requirements() {
    for requirement in [
        "you control three or more artifacts",
        "you control three or more artifacts and a creature",
    ] {
        let row = card(
            "Mox Opal",
            "{0}",
            "Legendary Artifact",
            &format!(
                "Metalcraft — {{T}}: Add one mana of any color. Activate only if {requirement}."
            ),
        );
        let lowered = parse_sim_card(&row);
        let abilities: Vec<_> = lowered.unlocked_abilities(0).collect();
        if requirement.contains(" and ") {
            assert!(abilities.is_empty());
            assert!(lowered.tap.is_none());
            assert!(matches!(
                lowered.support[0].status,
                AbilitySupportStatus::Unsupported { .. }
            ));
        } else {
            assert!(lowered.flags.requires_metalcraft);
            assert!(lowered.tap.is_some());
            assert_eq!(abilities.len(), 1);
            assert_eq!(
                abilities[0].condition,
                Some(SimAbilityCondition::Metalcraft)
            );
            assert!(
                matches!(abilities[0].effect_sequence(), [SimEffect::Mana(yield_)] if yield_.any_pips == 1)
            );
        }
    }
}

/// An optional counter target does not prevent Yawgmoth's draw or its full costs.
#[test]
fn optional_target_minus_counter_draw_keeps_the_life_and_creature_costs() {
    let row = card(
        "Yawgmoth, Thran Physician",
        "{2}{B}{B}",
        "Legendary Creature — Human Cleric",
        "Pay 1 life, Sacrifice another creature: Put a -1/-1 counter on up to one target creature and draw a card.",
    );
    let lowered = parse_sim_card(&row);
    let abilities: Vec<_> = lowered.unlocked_abilities(0).collect();
    assert_eq!(abilities.len(), 1);
    assert!(matches!(
        abilities[0].effect_sequence(),
        [SimEffect::DrawAndMinusCounter]
    ));
    let costs = abilities[0].activation.as_ref().expect("activation costs");
    assert_eq!(costs.life_payment(), 1);
    assert_eq!(costs.creature_sacrifices(), 1);
}

/// A color-choice annotation needs a typed one-color, per-counter mana yield.
#[test]
fn color_choice_annotation_does_not_hide_an_unmodeled_choice() {
    let unsupported = parse_sim_card(&card(
        "Unmodeled Choice",
        "{1}",
        "Artifact",
        "{T}: Choose a color. Draw a card.",
    ));
    assert!(unsupported.unlocked_abilities(0).next().is_none());
}

/// Charge counters retain their source, count, and owning chosen-X payment.
#[test]
fn source_charge_counters_require_complete_supported_wording() {
    for (cost, effect) in [
        ("{1}", "Put X charge counters on this artifact."),
        ("{X}", "Put X tower counters on this artifact."),
        ("{X}", "Put X age counters on this artifact."),
        ("{X}", "Put X +1/+1 counters on this artifact."),
        ("{X}", "Put X charge counters on target artifact."),
        ("{X}", "Put twice X charge counters on this artifact."),
        (
            "{1}",
            "Put one charge counter on this artifact for each card in your hand.",
        ),
        (
            "{1}",
            "Put a charge counter on this artifact or target creature.",
        ),
        ("{1}", "Put 0 charge counters on this artifact."),
    ] {
        let lowered = parse_sim_card(&card(
            "Charge source",
            "{1}",
            "Artifact",
            &format!("{cost}, {{T}}: {effect}"),
        ));
        assert!(
            lowered.unlocked_abilities(0).next().is_none(),
            "{cost}: {effect}"
        );
    }
    for (cost, effect, expected) in [
        ("{1}", "Put one charge counter on this artifact.", 1),
        ("{X}", "Put one charge counter on this artifact.", 1),
        ("{X}{X}", "Put X charge counters on this artifact.", 0),
        ("{1}", "Put two charge counters on this permanent.", 2),
    ] {
        let lowered = parse_sim_card(&card(
            "Charge source",
            "{1}",
            "Artifact",
            &format!("{cost}, {{T}}: {effect}"),
        ));
        let ability = lowered
            .unlocked_abilities(0)
            .next()
            .expect("complete activation");
        assert!(
            matches!(ability.effect, SimEffect::Counters(count) if count == expected),
            "{cost}: {effect}"
        );
    }
}

/// An ability word and its reminder cannot add a missing activation condition.
#[test]
fn metalcraft_label_does_not_invent_a_restriction() {
    let lowered = parse_sim_card(&card(
        "Ungated source",
        "{0}",
        "Artifact",
        "Metalcraft — {T}: Add one mana of any color. (You have metalcraft if you control three or more artifacts.)",
    ));
    let ability = lowered
        .unlocked_abilities(0)
        .next()
        .expect("unconditional mana ability");
    assert!(ability.condition.is_none());
    assert!(!lowered.flags.requires_metalcraft);
}
