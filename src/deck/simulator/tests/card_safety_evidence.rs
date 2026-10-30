//! Callable unsupported-safety evidence stays separate from supported mechanic counts.

use crate::db::CardRow;
use crate::deck::simulator::card_boundary_cost_tests as costs;
use crate::deck::simulator::card_boundary_search_tests as search;
use crate::deck::simulator::card_test_support::{named_row, pinned_revision};
use crate::deck::simulator::compiler_safety_action_tests as compiler;
use serde_json::{Value, json};
use std::collections::HashSet;

use super::{SourceAssertion, SourceCallbackBinding};

/// One rejected rule with actual production actions and relevant executed-report checks.
struct SafetyEvidence {
    name: &'static str,
    prefixes: &'static [&'static str],
    obligation: &'static str,
    report_scope: &'static str,
    assertion_path: &'static str,
    assertion: SourceAssertion,
}

/// Source-preserving safety packets never count their changed-text controls as new supported identities.
const SAFETY: &[SafetyEvidence] = &[
    SafetyEvidence {
        name: "Cultivate",
        prefixes: &["Search your library"],
        obligation: "The printed split search moves distinct results to both destinations. Name-restricted mutations cannot move cards, shuffle, or grant linked rewards. Missing mana preserves the attempt; renamed wording stays equal and changed single-search wording executes.",
        report_scope: "Reports keep actual search placements separate from paid, rejected name-restricted mutations. Exact source records, physical library identities, RNG, and changed single-search controls remain checked.",
        assertion_path: "card_boundary_search_tests::{assert_split_search_actions,assert_split_search_riders,assert_split_search_report,assert_split_search_control}",
        assertion: assert_split_search_safety,
    },
    SafetyEvidence {
        name: "Kodama's Reach",
        prefixes: &["Search your library"],
        obligation: "The printed split search moves distinct results to both destinations. Name-restricted mutations cannot move cards, shuffle, or grant linked rewards. Missing mana preserves the attempt; renamed wording stays equal and changed single-search wording executes.",
        report_scope: "Reports keep actual search placements separate from paid, rejected name-restricted mutations. Exact source records, physical library identities, RNG, and changed single-search controls remain checked.",
        assertion_path: "card_boundary_search_tests::{assert_split_search_actions,assert_split_search_riders,assert_split_search_report,assert_split_search_control}",
        assertion: assert_split_search_safety,
    },
    SafetyEvidence {
        name: "Sisay, Weatherlight Captain",
        prefixes: &["{W}{U}{B}{R}{G}:"],
        obligation: "Even five available colors cannot enable the unsupported legendary and source-power search. Rejection preserves all physical zones, pool, and RNG; renamed and isolated selector controls stay inert. Changed exact artifact-search wording executes.",
        report_scope: "A full turn reports five unused mana, no search access, unchanged library identities, and exact unsupported wording, span, hash, and reason. The changed-text control reports its actual search result.",
        assertion_path: "card_boundary_search_tests::{assert_sisay_actions,assert_sisay_report,assert_sisay_control}",
        assertion: assert_sisay_safety,
    },
    SafetyEvidence {
        name: "Ox of Agonas",
        prefixes: &[
            "When this creature enters",
            "Escape",
            "This creature escapes",
        ],
        obligation: "Three, seven, and eight graveyard cards cannot enable the unsupported Escape route. The separate normal hand cast pays five mana including two red and grants no entry draw or escaped counter.",
        report_scope: "Normal-cast spend, unchanged library, no replay, no entry draw, exact source hash, and the unsupported entry reason. Escape's keyword payment has no separate ability report row.",
        assertion_path: "card_boundary_cost_tests::{assert_ox_cost_boundary,assert_ox_cost_boundary_report}",
        assertion: assert_ox_safety,
    },
    SafetyEvidence {
        name: "Esper Sentinel",
        prefixes: &["Whenever an opponent casts"],
        obligation: "The exact first-noncreature event retains its required unrecognized source-power tax. No free draw or player resource change occurs; renamed and fixed-tax text controls are separate.",
        report_scope: "The unsupported source wording and reason remain visible with zero player draws. The changed fixed-tax control reports its actual draw under the synthetic opponent model.",
        assertion_path: "card_engine_tests::assert_esper_inert + card_report_tests::assert_esper_report",
        assertion: assert_esper_safety,
    },
    SafetyEvidence {
        name: "Force of Will",
        prefixes: &["You may pay 1 life"],
        obligation: "A matching blue hand card cannot enable an unsupported alternate life payment. Rejection preserves physical zones, life, pool, and RNG. The printed five-mana route remains separate.",
        report_scope: "A full turn reports no free cast or life payment and preserves the exact unsupported wording, hash, span, and reason. Counterspell resolution is not certified.",
        assertion_path: "compiler_safety_action_tests::assert_force_of_will_payment",
        assertion: compiler::assert_force_of_will_payment,
    },
    SafetyEvidence {
        name: "Force of Negation",
        prefixes: &["If it's not your turn"],
        obligation: "A matching blue hand card cannot bypass the unsupported off-turn alternate requirement. Rejection preserves resources; the printed three-mana route is separate.",
        report_scope: "A full turn reports no free alternate cast and retains exact wording, hash, span, and reason. Counterspell targets and exile replacement are not certified.",
        assertion_path: "compiler_safety_action_tests::assert_force_of_negation_payment",
        assertion: compiler::assert_force_of_negation_payment,
    },
    SafetyEvidence {
        name: "Dwynen, Gilt-Leaf Daen",
        prefixes: &["Whenever Dwynen attacks"],
        obligation: "The paid source cannot attack while sick, then actually attacks beside other Elves without gaining invented scaled life. A changed fixed-life result proves that the same attack path can grant life.",
        report_scope: "Two executed turns retain cast spend, attackers, combat potential, life gain, physical identities, exact unsupported wording, source hash, span, and reason.",
        assertion_path: "compiler_safety_action_tests::{assert_dwynen_attack_safety,assert_dwynen_fixed_attack_control}",
        assertion: assert_dwynen_safety,
    },
];

/// Keep each split-search source attached to its complete paid, rejected, and changed-text packet.
fn assert_split_search_safety(source: &CardRow) {
    search::assert_split_search_actions(source);
    search::assert_split_search_riders(source);
    search::assert_split_search_report(source);
    search::assert_split_search_control(source);
}

/// Source-power rejection and its supported wording control share the owner's row-based packet.
fn assert_sisay_safety(source: &CardRow) {
    search::assert_sisay_actions(source);
    search::assert_sisay_report(source);
    search::assert_sisay_control(source);
}

/// Join the completed cost action and report packet without copying its scenario.
fn assert_ox_safety(source: &CardRow) {
    costs::assert_ox_cost_boundary(source);
    costs::assert_ox_cost_boundary_report(source);
}

/// Join unsupported payment actions with their actual report disposition.
fn assert_esper_safety(source: &CardRow) {
    crate::deck::simulator::card_runtime_tests::engines::assert_esper_inert(source);
    crate::deck::simulator::card_report_tests::assert_esper_report(source);
}

/// The fixed-result control does not certify Dwynen's original scaled reward.
fn assert_dwynen_safety(source: &CardRow) {
    compiler::assert_dwynen_attack_safety(source);
    compiler::assert_dwynen_fixed_attack_control(source);
}

/// Run the exact registered source through its completed safety packet.
pub(in crate::deck::simulator) fn assert_registered(source: &CardRow) {
    let evidence = SAFETY
        .iter()
        .find(|evidence| evidence.name == source.name)
        .expect("registered safety packet");
    (evidence.assertion)(source);
}

/// Attach safety evidence only to exact original source lines, never as supported execution.
pub(in crate::deck::simulator) fn source_branches(source: &CardRow, wording: &str) -> Vec<Value> {
    let Some(evidence) = SAFETY.iter().find(|evidence| evidence.name == source.name) else {
        return vec![];
    };
    source.oracle_text.lines().filter(|line| evidence.prefixes.iter().any(|prefix| line.starts_with(prefix)) && wording.contains(line))
        .map(|line| json!({"source_text": line, "obligation": evidence.obligation, "report_scope": evidence.report_scope, "assertion": evidence.assertion_path, "disposition": "unsupported_safety_not_supported_execution"}))
        .collect()
}

/// A registered safety packet can close only its stated rejected-rule obligation.
pub(in crate::deck::simulator) fn has_source(name: &str) -> bool {
    SAFETY.iter().any(|evidence| evidence.name == name)
}

/// Execute each registered safety binding and return its actual callback for distinct-function counts.
pub(in crate::deck::simulator) fn execute_callbacks() -> Vec<SourceCallbackBinding> {
    SAFETY
        .iter()
        .map(|evidence| {
            (evidence.assertion)(&named_row(evidence.name));
            (evidence.name, evidence.assertion)
        })
        .collect()
}

/// Count actual callable safety sources and their immutable source links separately.
#[test]
fn registered_safety_packets_execute_their_source_bound_actions_and_reports() {
    let reviews = crate::deck::simulator::card_semantic_tests::reviewed_cards();
    let mut identities = HashSet::new();
    let mut links = 0;
    for evidence in SAFETY {
        let source = named_row(evidence.name);
        let revision = pinned_revision(&source);
        (evidence.assertion)(&source);
        assert!(identities.insert(source.oracle_id.clone()));
        for prefix in evidence.prefixes {
            let matching: Vec<_> = source
                .oracle_text
                .lines()
                .filter(|line| line.starts_with(prefix))
                .collect();
            let [wording] = matching.as_slice() else {
                panic!("one source line for {prefix}");
            };
            let branch = source_branches(&source, wording);
            assert_eq!(branch.len(), 1);
            let review = super::source_review_link(&source, wording, &reviews);
            assert!(
                !revision["flattened_input_sha256"]
                    .as_str()
                    .expect("source hash")
                    .is_empty()
            );
            assert!(review.is_object());
            links += 1;
        }
    }
    assert_eq!(identities.len(), SAFETY.len());
    println!(
        "unsupported-safety evidence: {} genuine source identities, {links} exact source-line links; zero supported identity or whole-card claims",
        identities.len()
    );
}
