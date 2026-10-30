use super::*;
use crate::deck::simulator::model::ManaYield;
use crate::deck::simulator::turn_loop_tests::{deck, row, state};

/// Removing a mana creature cannot invalidate the next source's position.
#[test]
fn sacrificing_a_mana_creature_preserves_the_source_iteration() {
    let cards = deck(&[
        row(
            "Sacrifice source",
            "{B}",
            "Creature",
            "Sacrifice this creature: Add {B}.",
        ),
        row("Green source", "{G}", "Creature", "{T}: Add {G}."),
        row("Spell", "{3}", "Sorcery", ""),
    ]);
    let mut st = state(vec![2], vec![]);
    st.next_uid = 2;
    for index in 0..2 {
        let mut source = new_perm_with(index + 1, &cards, CardIdx(index), 0, false);
        source.summoning_sick = false;
        st.battlefield.push(source);
    }
    let mut pool = ManaPool::default();
    tap_dorks_for_mana(&cards, &mut st, &mut pool, 1);
    assert_eq!(st.graveyard, [CardIdx(0)]);
    assert_eq!(st.battlefield.len(), 1);
    assert!(st.battlefield[0].tapped);
    assert_eq!(pool.fixed[2], 1);
    assert_eq!(pool.fixed[4], 1);
}

/// Phyrexian mana costs and explicit life costs share one legal life budget.
#[test]
fn native_mana_modes_pay_their_phyrexian_life_cost() {
    let cards = deck(&[row(
        "Life source",
        "{1}",
        "Artifact",
        "{B/P}, {T}: Add {G}.",
    )]);
    for (life, succeeds) in [(1, false), (3, true)] {
        let mut st = state(vec![], vec![]);
        st.life = life;
        let source = new_perm_with(1, &cards, CardIdx(0), 0, false);
        st.battlefield.push(source.clone());
        let mut pool = ManaPool::default();
        assert_eq!(
            mana::activate_mana_mode(&cards, &mut st, &source, &mut pool, 1),
            succeeds
        );
        assert_eq!(st.life, life - if succeeds { 2 } else { 0 });
        assert_eq!(st.life_paid, if succeeds { 2 } else { 0 });
        assert_eq!(pool.fixed[4], u32::from(succeeds));
        assert_eq!(st.battlefield[0].tapped, succeeds);
    }
}

/// A face-down card supplies neither its printed legend status nor its colors.
#[test]
fn facedown_legend_cannot_enable_legendary_color_mana() {
    let mut legend = row("Red legend", "{R}", "Legendary Creature", "Morph {1}");
    legend.colors = "[\"R\"]".into();
    let cards = deck(&[
        row(
            "Amber source",
            "{0}",
            "Artifact",
            "{T}: Add one mana of any color among legendary creatures and planeswalkers you control.",
        ),
        legend,
    ]);
    for face_down in [false, true] {
        let mut st = state(vec![], vec![]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
        let mut legend = new_perm_with(2, &cards, CardIdx(1), 0, false);
        legend.face_down = face_down;
        st.battlefield.push(legend);
        let pool = build_pool(&cards, &mut st, 1);
        assert_eq!(pool.total(), u32::from(!face_down));
    }
}

/// Color-count production reads the characteristics of the current objects.
#[test]
fn colors_present_excludes_facedown_printed_colors() {
    let mut creature = row("Colored card", "{R}", "Creature", "");
    creature.colors = "[\"R\"]".into();
    let cards = deck(&[row("Source", "{1}", "Artifact", ""), creature]);
    let mut st = state(vec![], vec![]);
    let source = new_perm_with(1, &cards, CardIdx(0), 0, false);
    let mut creature = new_perm_with(2, &cards, CardIdx(1), 0, false);
    creature.face_down = true;
    st.battlefield.push(creature);
    let yield_ = ManaYield {
        scaling: Some(Scale::ColorsPresent),
        ..Default::default()
    };
    assert_eq!(
        resolved_source_yield(&cards, &st, &source, &yield_, false).fixed,
        [0; 5]
    );
    st.battlefield[0].face_down = false;
    assert_eq!(
        resolved_source_yield(&cards, &st, &source, &yield_, false).fixed,
        [0, 0, 0, 1, 0]
    );
}
