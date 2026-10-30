//! Exact source grammar checks do not certify execution or whole-card support.

use crate::db::CardRow;
use crate::deck::simulator::model::{SearchDestination, SimEffect};
use crate::deck::simulator::oracle_ast::{
    ActivationCost, ActivationTarget, KeywordArgument, OracleAbility, OracleEffect,
};
use crate::deck::simulator::oracle_lower::parse_sim_card;
use crate::deck::simulator::oracle_parser::parse_oracle_card;

/// Require exact source spans and a production input hash for every genuine source.
pub(super) fn assert_source_compiles(row: &CardRow) {
    let oracle = parse_oracle_card(row);
    let lowered = parse_sim_card(row);
    assert_eq!(
        lowered.source_hash,
        crate::deck::simulator::support::card_source_hash(row)
    );
    assert_eq!(lowered.name, row.name);
    assert_eq!(oracle.abilities.len(), oracle.ability_sources.len());
    for source in &oracle.ability_sources {
        if let Some(span) = source.span {
            assert_eq!(source.text, row.oracle_text[span.start..span.end]);
        }
    }
}

/// Preserve printed Escape count eight while declining the incompatible runtime route.
pub(super) fn assert_ox_escape_inert(row: &CardRow) {
    let oracle = parse_oracle_card(row);
    assert!(
        oracle
            .keywords
            .iter()
            .flat_map(|keyword| &keyword.arguments)
            .any(|argument| matches!(argument, KeywordArgument::EscapeCards(8)))
    );
    assert!(parse_sim_card(row).spell_data.own_escape.is_none());
    assert_eq!(parse_sim_card(row).min_cost.total(), 5);
}

/// A source-power and legendary restriction cannot become an unrestricted search.
pub(super) fn assert_sisay_inert(row: &CardRow) {
    let oracle = parse_oracle_card(row);
    let activation = oracle
        .abilities
        .iter()
        .find_map(|ability| match ability {
            OracleAbility::Activated(ability) => Some(ability),
            _ => None,
        })
        .expect("printed activation parses");
    assert!(
        activation
            .effects
            .iter()
            .any(|effect| matches!(effect, OracleEffect::Unsupported(_)))
    );
    assert!(activation.to_runtime_all().is_empty());
}

/// Exact mana-value equality survives parsing and lowering without becoming a maximum.
pub(super) fn assert_trophy_mage(row: &CardRow) {
    let oracle = parse_oracle_card(row);
    let trigger = oracle
        .abilities
        .iter()
        .find_map(|ability| match ability {
            OracleAbility::Triggered(ability) => Some(ability),
            _ => None,
        })
        .expect("entry trigger parses");
    let [OracleEffect::Search(spec)] = trigger.effects.as_slice() else {
        panic!(
            "expected complete exact-value search: {:?}",
            trigger.effects
        );
    };
    assert_eq!(spec.mana_value, Some(3));
    assert_eq!(spec.max_mana_value, None);
    assert_eq!(spec.min_mana_value, None);
    assert_eq!(spec.destination, SearchDestination::Hand);
    assert!(
        matches!(trigger.to_runtime_all()[0].effect_sequence(), [SimEffect::Search(spec)] if spec.mana_value == Some(3))
    );
}

/// Pinned tutor wording keeps its top-of-library destination and linked drawback.
pub(super) fn assert_top_tutor(row: &CardRow) {
    let lowered = parse_sim_card(row);
    assert!(
        matches!(lowered.spell_data.effects.first(), Some(SimEffect::Search(spec)) if spec.destination == SearchDestination::LibraryTop && spec.shuffle)
    );
    let vampiric = row.name == "Vampiric Tutor";
    assert_eq!(
        lowered.spell_data.life_loss_on_resolve,
        if vampiric { 2 } else { 0 }
    );
}

/// Preserve a mana source's printed payment, without claiming runtime colors or eligibility.
pub(super) fn assert_mana_source(row: &CardRow) {
    let oracle = parse_oracle_card(row);
    assert!(oracle.abilities.iter().any(
        |ability| matches!(ability, OracleAbility::Activated(ability)
            if matches!(ability.costs.as_slice(), [ActivationCost::Tap(ActivationTarget::Source)]))
    ));
    assert!(parse_sim_card(row).tap.is_some());
}

/// Commander-identity production cannot lower to unrestricted any-color mana.
pub(super) fn assert_identity_mana_source(row: &CardRow) {
    assert_mana_source(row);
    let lowered = parse_sim_card(row);
    let output = lowered.tap.expect("complete source-tap mana mode");
    assert_eq!(output.commander_pips, 1);
    assert_eq!(output.any_pips, 0);
    assert_eq!(output.colorless, 0);
}

/// Halfling retains both printed modes, but only its complete colorless mode lowers.
pub(super) fn assert_halfling_modes(row: &CardRow) {
    use crate::deck::simulator::model::{SimAbilityKind, SimActivationCost};
    use crate::deck::simulator::support::AbilitySupportStatus;
    let oracle = parse_oracle_card(row);
    let [
        OracleAbility::Activated(colorless),
        OracleAbility::Activated(legendary),
    ] = oracle.abilities.as_slice()
    else {
        panic!("both printed mana modes remain separate");
    };
    for ability in [colorless, legendary] {
        assert!(matches!(
            ability.costs.as_slice(),
            [ActivationCost::Tap(ActivationTarget::Source)]
        ));
        assert!(ability.restrictions.is_empty());
        assert!(ability.is_mana_ability);
        assert!(!ability.mana_per_spell_cast);
    }
    assert!(
        matches!(colorless.effects.as_slice(), [OracleEffect::Mana(output)] if output.colorless == 1 && output.any_pips == 0 && output.restriction.is_none())
    );
    assert!(
        matches!(legendary.effects.as_slice(), [OracleEffect::Mana(output), OracleEffect::Unsupported(text)] if output.any_pips == 1 && text == "Spend this mana only to cast a legendary spell, and that spell can't be countered")
    );
    for (ability, colorless, any) in [(colorless, 1, 0), (legendary, 0, 1)] {
        let OracleEffect::Mana(output) = &ability.effects[0] else {
            panic!("typed printed mana production");
        };
        assert_halfling_output(output, colorless, any);
    }
    assert!(legendary.to_runtime_all().is_empty());
    let lowered = parse_sim_card(row);
    let abilities: Vec<_> = lowered.unlocked_abilities(0).collect();
    assert_eq!(abilities.len(), 1);
    assert_eq!(abilities[0].kind, SimAbilityKind::ManaActivated);
    assert_eq!(
        abilities[0].activation.as_ref().expect("tap cost").costs,
        [SimActivationCost::TapSource]
    );
    assert!(
        abilities[0]
            .activation
            .as_ref()
            .expect("tap cost")
            .restrictions
            .is_empty()
    );
    assert!(
        matches!(abilities[0].effect_sequence(), [SimEffect::Mana(output)] if output.colorless == 1 && output.any_pips == 0 && output.restriction.is_none())
    );
    assert!(matches!(
        lowered.support[0].status,
        AbilitySupportStatus::Supported
    ));
    assert!(
        matches!(&lowered.support[1].status, AbilitySupportStatus::Unsupported { reason } if !reason.is_empty())
    );
}

/// Exact output fields cannot invent extra mana or an unrelated production restriction.
fn assert_halfling_output(
    output: &crate::deck::simulator::model::ManaYield,
    colorless: u32,
    any: u32,
) {
    assert_eq!(output.colorless, colorless);
    assert_eq!(output.any_pips, any);
    assert_eq!(output.commander_pips, 0);
    assert_eq!(output.fixed, [0; 5]);
    assert_eq!(output.choice, [false; 5]);
    assert!(!output.choice_colorless && !output.legendary_colors && !output.opponent_any);
    assert!(!output.one_color && !output.alternatives && !output.cannot_pay_generic);
    assert!(output.scaling.is_none() && output.restriction.is_none());
    assert_eq!(output.total(), 1);
}

/// Esper keeps its exact first-event filter and unrecognized required tax wording.
pub(super) fn assert_esper_payment_inert(row: &CardRow) {
    use crate::deck::simulator::oracle_ast::{OracleResolutionPayment, OracleTriggerEvent};
    let oracle = parse_oracle_card(row);
    let [OracleAbility::Triggered(ability)] = oracle.abilities.as_slice() else {
        panic!("one first-noncreature trigger");
    };
    assert_eq!(
        ability.event,
        OracleTriggerEvent::OpponentSpellCast {
            noncreature: true,
            first: true
        }
    );
    assert!(ability.event_filter.noncreature && ability.event_filter.first_each_turn);
    assert!(matches!(
        ability.effects.as_slice(),
        [OracleEffect::Draw(1)]
    ));
    assert!(
        matches!(&ability.resolution_payment, Some(OracleResolutionPayment::Unsupported(text)) if text == "that player pays {x}, where x is this creature's power")
    );
    assert!(ability.to_runtime_all().is_empty());
    let lowered = parse_sim_card(row);
    assert!(lowered.unlocked_abilities(0).next().is_none());
    assert!(
        matches!(&lowered.support[0].status, crate::deck::simulator::support::AbilitySupportStatus::Unsupported { reason } if !reason.is_empty())
    );
}

/// An event engine must retain its printed event text in source evidence.
pub(super) fn assert_event_source(row: &CardRow) {
    let oracle = parse_oracle_card(row);
    assert!(
        oracle
            .abilities
            .iter()
            .any(|ability| matches!(ability, OracleAbility::Triggered(_)))
    );
    assert!(
        oracle
            .ability_sources
            .iter()
            .any(|source| source.text.starts_with("Whenever "))
    );
}

/// Targeted recursion remains a compiler observation until action targets are checked.
pub(super) fn assert_recursion_source(row: &CardRow) {
    let oracle = parse_oracle_card(row);
    assert!(
        oracle
            .ability_sources
            .iter()
            .any(|source| source.text.contains("target card from your graveyard"))
    );
}
