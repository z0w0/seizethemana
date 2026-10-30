use super::*;
use crate::deck::simulator::game::new_perm_with;
use crate::deck::simulator::model::CardIdx;
use crate::deck::simulator::turn_loop_tests::{deck, row, state};

/// Commander tax is added before affinity reduces the total generic cost.
#[test]
fn affinity_reduces_commander_tax_before_the_zero_floor() {
    for (artifacts, generic, accepted) in [(8, 0, true), (7, 0, false), (7, 1, true)] {
        let mut cards = deck(&[row("Artifact", "{9}", "Artifact", "")]);
        cards.commanders = deck(&[row(
            "Affinity commander",
            "{6}{R}{R}{R}",
            "Legendary Creature",
            "Affinity for artifacts",
        )])
        .cards;
        let profile = CommanderProfile::new(&cards);
        let mut st = state(vec![], vec![]);
        st.commander_casts = vec![1];
        for uid in 1..=artifacts {
            st.battlefield
                .push(new_perm_with(uid, &cards, CardIdx(0), 0, false));
        }
        st.next_uid = artifacts;
        let mut census = TurnCensus::new(1, 1);
        let mut pool = ManaPool {
            colorless: generic,
            fixed: [0, 0, 0, 3, 0],
            ..ManaPool::default()
        };

        commander_phase(
            &cards,
            &mut st,
            &mut pool,
            &mut census,
            &profile,
            1,
            &mut Vec::new(),
            false,
        );

        assert_eq!(st.commander_casts, [if accepted { 2 } else { 1 }]);
        assert_eq!(pool.total(), if accepted { 0 } else { 3 + generic });
        assert_eq!(
            census.mana_spent,
            [if accepted {
                f64::from(3 + generic)
            } else {
                0.0
            }]
        );
    }
}

/// Unsupported mandatory extra costs block command-zone casts before payment.
#[test]
fn commander_rejects_unsupported_additional_costs() {
    for unsupported in [false, true] {
        let mut cards = deck(&[]);
        cards.commanders = deck(&[row("Commander", "{1}", "Legendary Creature", "")]).cards;
        cards.commanders[0].spell_data.unsupported_additional_cost = unsupported;
        let profile = CommanderProfile::new(&cards);
        let mut st = state(vec![], vec![]);
        let mut pool = ManaPool {
            colorless: 1,
            ..ManaPool::default()
        };

        commander_phase(
            &cards,
            &mut st,
            &mut pool,
            &mut TurnCensus::new(1, 0),
            &profile,
            1,
            &mut Vec::new(),
            false,
        );

        assert_eq!(pool.total(), u32::from(unsupported));
        assert_eq!(st.battlefield.is_empty(), unsupported);
        assert_eq!(st.commander_casts, [u32::from(!unsupported)]);
    }
}

/// Plot waives unsupported printed mana, not an unsupported mandatory extra cost.
#[test]
fn plotted_cast_checks_the_separate_additional_cost_flag() {
    for unsupported in [false, true] {
        let mut cards = deck(&[row("Plotted", "{9}", "Sorcery", "Gain three life.")]);
        cards.cards[0].cost.unsupported = true;
        cards.cards[0].min_cost.unsupported = true;
        cards.cards[0].spell_data.unsupported_additional_cost = unsupported;
        let mut st = state(vec![], vec![]);
        st.plotted.push((CardIdx(0), 1));

        super::super::cast_plotted_cards(
            &cards,
            &mut st,
            &mut ManaPool::default(),
            2,
            &mut [0.0; 2],
            &mut Vec::new(),
        );

        assert_eq!(st.life_gained, if unsupported { 0 } else { 3 });
        assert_eq!(st.plotted.is_empty(), !unsupported);
        assert_eq!(st.graveyard.is_empty(), unsupported);
    }
}

/// Command-zone casts with numeric extra costs wait until that payment path exists.
#[test]
fn commander_numeric_additional_costs_never_receive_a_free_payment() {
    for (creatures, discards, life) in [(0, 0, 0), (1, 0, 0), (0, 1, 0), (0, 0, 3)] {
        let mut cards = deck(&[row("Available resource", "{9}", "Creature", "")]);
        cards.commanders = deck(&[row("Commander", "{1}", "Legendary Creature", "")]).cards;
        let costs = &mut cards.commanders[0].spell_data;
        costs.additional_cost_creatures = creatures;
        costs.additional_cost_discards = discards;
        costs.additional_cost_life = life;
        let blocked = creatures + discards + life > 0;
        let profile = CommanderProfile::new(&cards);
        let mut st = state(vec![0], vec![]);
        st.battlefield
            .push(new_perm_with(1, &cards, CardIdx(0), 0, false));
        st.next_uid = 1;
        let mut pool = ManaPool {
            colorless: 1,
            ..ManaPool::default()
        };
        let mut census = TurnCensus::new(1, 1);

        commander_phase(
            &cards,
            &mut st,
            &mut pool,
            &mut census,
            &profile,
            1,
            &mut Vec::new(),
            false,
        );

        assert_eq!(pool.total(), u32::from(blocked));
        assert_eq!(st.commander_casts, [u32::from(!blocked)]);
        assert_eq!(st.battlefield.len(), if blocked { 1 } else { 2 });
        assert_eq!(st.hand, [CardIdx(0)]);
        assert_eq!(st.life, 20);
        assert_eq!(st.life_paid, 0);
        assert!(st.graveyard.is_empty());
    }
}

/// Physical-card readiness uses the same tap and delve resources as spell payment.
#[test]
fn readiness_counts_only_available_convoke_improvise_and_delve_resources() {
    for mechanic in ["Convoke", "Improvise", "Delve"] {
        for available in [false, true] {
            let cost = if mechanic == "Convoke" { "{W}" } else { "{1}" };
            let mut cards = deck(&[
                row("Spell", cost, "Sorcery", mechanic),
                row("Resource", "{9}", "Artifact Creature", ""),
            ]);
            cards.cards[1].colors[0] = true;
            let profile = CommanderProfile::new(&cards);
            let mut st = state(vec![], vec![]);
            if mechanic == "Delve" {
                if available {
                    st.graveyard.push(CardIdx(1));
                }
            } else {
                st.battlefield
                    .push(new_perm_with(1, &cards, CardIdx(1), 0, !available));
                st.next_uid = 1;
            }
            let mut census = TurnCensus::new(1, 2);

            commander_phase(
                &cards,
                &mut st,
                &mut ManaPool::default(),
                &mut census,
                &profile,
                1,
                &mut Vec::new(),
                true,
            );

            assert_eq!(census.mana_ready[0], available.then_some(1), "{mechanic}");
            assert_eq!(
                st.battlefield.first().map(|permanent| permanent.tapped),
                if mechanic == "Delve" {
                    None
                } else {
                    Some(!available)
                }
            );
            assert!(
                st.exile.is_empty(),
                "readiness does not consume delve cards"
            );
        }
    }
}

/// Unknown, absent, and unsupported mandatory costs cannot mark a card ready.
#[test]
fn readiness_rejects_missing_and_unsupported_cost_data() {
    for (cost, unsupported_additional) in
        [("", false), ("{Q}", false), ("{S}", false), ("{0}", true)]
    {
        let mut cards = deck(&[row("Unavailable spell", cost, "Sorcery", "")]);
        cards.cards[0].spell_data.unsupported_additional_cost = unsupported_additional;
        let profile = CommanderProfile::new(&cards);
        let mut st = state(vec![], vec![]);
        let mut census = TurnCensus::new(1, 1);

        commander_phase(
            &cards,
            &mut st,
            &mut ManaPool {
                colorless: 99,
                ..ManaPool::default()
            },
            &mut census,
            &profile,
            1,
            &mut Vec::new(),
            true,
        );

        assert_eq!(census.mana_ready[0], None, "{cost}");
    }
}
