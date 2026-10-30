use super::*;
use crate::deck::simulator::commander_test_support::named_row;
use crate::deck::simulator::game::{CardRef, new_perm_with};
use crate::deck::simulator::game_mana::adjust_cast_cost;
use crate::deck::simulator::library_core_report_tests::put_permanents;
use crate::deck::simulator::model::{CardIdx, SpellCostFilter, SpellCostReduction};
use crate::deck::simulator::oracle_parser::cost::parse_cost;
use crate::deck::simulator::scenario_test_support::{deck, row, state};

/// Captured source cards affect matching captured spells and reject other spells.
#[test]
fn captured_reducers_match_types_and_colors_at_runtime() {
    for (source, matching, nonmatching) in [
        ("Goblin Electromancer", "Counterspell", "Arcane Signet"),
        ("Foundry Inspector", "Arcane Signet", "Counterspell"),
        ("Etherium Sculptor", "Arcane Signet", "Counterspell"),
        ("Goblin Anarchomancer", "Cultivate", "Counterspell"),
        ("Pearl Medallion", "Swords to Plowshares", "Counterspell"),
        ("Sapphire Medallion", "Counterspell", "Dark Ritual"),
        ("Jet Medallion", "Dark Ritual", "Cultivate"),
        ("Ruby Medallion", "Lightning Bolt", "Counterspell"),
    ] {
        let cards = deck(&[
            named_row(source),
            named_row(matching),
            named_row(nonmatching),
        ]);
        let battlefield = [new_perm_with(1, &cards, CardIdx(0), 0, false)];
        assert_eq!(
            spell_cost_reduction(&cards, &cards.cards[1], &battlefield),
            1,
            "{source}: {matching}"
        );
        assert_eq!(
            spell_cost_reduction(&cards, &cards.cards[2], &battlefield),
            0,
            "{source}: {nonmatching}"
        );
        assert_eq!(spell_cost_reduction(&cards, &cards.cards[1], &[]), 0);
    }
}

/// Tapped sources still apply, while removed or face-down sources stop applying.
#[test]
fn source_discounts_stack_and_follow_live_battlefield_state() {
    let cards = deck(&[
        named_row("Goblin Electromancer"),
        named_row("Goblin Electromancer"),
        named_row("Goblin Electromancer"),
        named_row("Counterspell"),
    ]);
    let mut game = state(vec![], vec![]);
    put_permanents(&cards, &mut game, &[0, 1, 2]);
    let spell = &cards.cards[3];
    assert_eq!(spell_cost_reduction(&cards, spell, &game.battlefield), 3);
    game.battlefield[0].tapped = true;
    assert_eq!(spell_cost_reduction(&cards, spell, &game.battlefield), 3);
    game.battlefield[1].face_down = true;
    assert_eq!(spell_cost_reduction(&cards, spell, &game.battlefield), 2);
    game.battlefield.remove(0);
    assert_eq!(spell_cost_reduction(&cards, spell, &game.battlefield), 1);
    game.battlefield[0].face_down = false;
    assert_eq!(spell_cost_reduction(&cards, spell, &game.battlefield), 2);
    game.battlefield.clear();
    assert_eq!(spell_cost_reduction(&cards, spell, &game.battlefield), 0);
}

/// Original and explicitly renamed controls use the same command-zone card lookup.
/// This checks engine references, not whether Electromancer is a legal commander.
#[test]
fn command_zone_sources_do_not_apply_until_they_are_permanents() {
    for renamed in [false, true] {
        let mut source = named_row("Goblin Electromancer");
        if renamed {
            source.name = "Renamed command-zone reducer control".into();
        }
        let mut cards = deck(&[named_row("Counterspell")]);
        cards.commanders = deck(&[source]).cards;
        let spell = &cards.cards[0];
        assert_eq!(spell_cost_reduction(&cards, spell, &[]), 0);
        let mut permanent =
            crate::deck::simulator::game::new_commander_perm(1, 0, 0, 0, true, false);
        assert_eq!(permanent.card, CardRef::Commander { slot: 0 });
        assert_eq!(spell_cost_reduction(&cards, spell, &[permanent.clone()]), 1);
        permanent.face_down = true;
        assert_eq!(spell_cost_reduction(&cards, spell, &[permanent]), 0);
    }
}

/// A spell-side reducer does not work when its MDFC is played as a land.
#[test]
fn hidden_mdfc_source_and_merged_target_properties_do_not_grant_discounts() {
    let cards = deck(&[
        row(
            "Split reducer",
            "{2} // ",
            "Artifact // Land",
            "Spells you cast cost {3} less to cast.\n//\n{T}: Add {C}.",
        ),
        row(
            "Split target",
            "{2}{U} // ",
            "Instant // Artifact Land",
            "Gain 1 life.\n//\n{T}: Add {R}.",
        ),
    ]);
    let mut permanent = new_perm_with(1, &cards, CardIdx(0), 0, false);
    let spell = &cards.cards[1];
    assert_eq!(spell_cost_reduction(&cards, spell, &[permanent.clone()]), 3);
    permanent.land_face = true;
    assert_eq!(spell_cost_reduction(&cards, spell, &[permanent]), 0);
    let artifact = SpellCostFilter::Types(vec![
        crate::deck::simulator::model::SpellCostCardType::Artifact,
    ]);
    assert!(spell.is_artifact);
    assert!(!artifact.matches(spell));
    assert!(spell.search_colors.is_none());
    assert!(
        !SpellCostFilter::Colors(vec![crate::deck::simulator::model::ManaColor::Red])
            .matches(spell)
    );
}

/// A multicolor spell matches one color-union ability only once.
#[test]
fn a_color_union_does_not_multiply_one_sources_reduction() {
    let mut target = row("Two-color spell", "{3}{R}{G}", "Sorcery", "Gain 1 life.");
    target.colors = "[\"R\",\"G\"]".into();
    let cards = deck(&[named_row("Goblin Anarchomancer"), target]);
    let battlefield = [new_perm_with(1, &cards, CardIdx(0), 0, false)];
    assert_eq!(
        spell_cost_reduction(&cards, &cards.cards[1], &battlefield),
        1
    );
}

/// A red flashback cost adds color identity but does not make the blue front spell red.
#[test]
fn captured_color_reducer_requires_front_colors_not_color_identity() {
    let mut target = row("Blue flashback control", "{U}", "Instant", "Flashback {R}");
    target.colors = "[\"U\"]".into();
    target.color_identity = "[\"U\",\"R\"]".into();
    let mut cards = deck(&[named_row("Goblin Anarchomancer"), target]);
    let battlefield = [new_perm_with(1, &cards, CardIdx(0), 0, false)];
    assert!(cards.cards[1].color_identity[3]);
    assert_eq!(
        cards.cards[1].search_colors,
        Some([false, true, false, false, false])
    );
    assert_eq!(
        spell_cost_reduction(&cards, &cards.cards[1], &battlefield),
        0
    );
    cards.cards[1].search_colors = None;
    cards.cards[1].colors[3] = true;
    assert_eq!(
        spell_cost_reduction(&cards, &cards.cards[1], &battlefield),
        0
    );
}

/// Flat discounts change only generic mana, including after a commander-tax increase.
#[test]
fn total_cost_reductions_preserve_every_non_generic_symbol() {
    let cards = deck(&[
        row(
            "Generic reducer",
            "{9}",
            "Artifact",
            "Spells you cast cost {9} less to cast.",
        ),
        row("Symbol test", "{8}{W}{U/B}{2/R}{G/P}{C}{S}", "Sorcery", ""),
    ]);
    let battlefield = [new_perm_with(1, &cards, CardIdx(0), 0, false)];
    let spell = &cards.cards[1];
    let expected = crate::deck::simulator::model::Cost {
        generic: 1,
        ..spell.min_cost
    };
    assert_eq!(
        adjust_cast_cost(&cards, spell, &battlefield, spell.min_cost, 2),
        expected
    );
    let alternative = parse_cost("{4}{U}{C}");
    assert_eq!(
        adjust_cast_cost(&cards, spell, &battlefield, alternative, 0),
        crate::deck::simulator::model::Cost {
            generic: 0,
            ..alternative
        }
    );
    assert_eq!(
        adjust_cast_cost(&cards, spell, &battlefield, parse_cost("{U}"), 2),
        parse_cost("{U}")
    );
}

/// Affinity and live flat reductions both subtract from the final generic demand.
#[test]
fn live_spell_reductions_stack_with_affinity() {
    let cards = deck(&[
        named_row("Foundry Inspector"),
        row("Artifact body", "{9}", "Artifact", ""),
        row(
            "Affinity spell",
            "{8}{U}",
            "Artifact",
            "Affinity for artifacts",
        ),
    ]);
    let mut game = state(vec![], vec![]);
    put_permanents(&cards, &mut game, &[0, 1]);
    let spell = &cards.cards[2];
    assert!(spell.keyword_abilities.affinity);
    assert_eq!(spell_cost_reduction(&cards, spell, &game.battlefield), 1);
    assert_eq!(
        adjust_cast_cost(&cards, spell, &game.battlefield, spell.min_cost, 0),
        parse_cost("{5}{U}")
    );
}

/// A large legal typed amount cannot overflow when several sources contribute.
#[test]
fn summed_discounts_do_not_wrap_to_a_smaller_amount() {
    let mut cards = deck(&[row("Source", "{1}", "Artifact", "")]);
    cards.cards[0].cost_reductions = vec![
        SpellCostReduction {
            amount: u32::MAX,
            filter: SpellCostFilter::Any,
        },
        SpellCostReduction {
            amount: 1,
            filter: SpellCostFilter::Any,
        },
    ];
    let battlefield = [new_perm_with(1, &cards, CardIdx(0), 0, false)];
    assert_eq!(
        spell_cost_reduction(&cards, &cards.cards[0], &battlefield),
        u32::MAX
    );
}
