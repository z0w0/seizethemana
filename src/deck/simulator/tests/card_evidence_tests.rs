//! Per-ability claims bind exact source revisions to real Rust assertions.

use crate::db::CardRow;
use crate::deck::simulator::card_report_tests as report;
use crate::deck::simulator::card_runtime_tests as runtime;
use crate::deck::simulator::card_test_support::{corpus, json_hash, named_row, pinned_revision};
use serde_json::{Value, json};
use std::collections::HashSet;

/// Run one evidence assertion against its pinned source row.
pub(in crate::deck::simulator) type SourceAssertion = fn(&CardRow);

/// Pair a registered source name with its actual assertion function.
pub(in crate::deck::simulator) type SourceCallbackBinding = (&'static str, SourceAssertion);

/// Exact covered payments, eligibility rules, rewards, controls, and report fields.
#[path = "card_branch_obligations.rs"]
mod branch_obligations;

/// Row-bound identity production and native payment reports.
#[path = "card_identity_tests.rs"]
mod identity;

/// Rejected source rules and their executed report evidence do not count as supported mechanics.
#[path = "card_safety_evidence.rs"]
pub(super) mod safety;

/// Reviewed source and synthetic packets keep their callable bindings and scope limits separate.
#[path = "card_packet_evidence.rs"]
pub(super) mod packets;

/// A scoped mechanic claim needs both production controls and an executed report path.
pub(in crate::deck::simulator) struct MechanicEvidence {
    pub(in crate::deck::simulator) name: &'static str,
    pub(in crate::deck::simulator) scope: &'static str,
    prefixes: &'static [&'static str],
    actions: SourceAssertion,
    report: SourceAssertion,
}

/// Register source abilities without deriving support from parser or compiler labels.
pub(in crate::deck::simulator) fn mechanic_evidence() -> Vec<MechanicEvidence> {
    let mut evidence = mana_evidence();
    evidence.extend(search_evidence());
    evidence.extend(engine_evidence());
    evidence.extend(cost_and_zone_evidence());
    evidence.extend(wider_corpus_evidence());
    evidence
}

/// Native sources and a ritual each need real payments rather than helper-only mana totals.
fn mana_evidence() -> Vec<MechanicEvidence> {
    vec![
        MechanicEvidence {
            name: "Sol Ring",
            scope: "Tap once for two colorless mana and fund an actual generic cast",
            prefixes: &["{T}:"],
            actions: runtime::mana::assert_sol_ring,
            report: report::assert_sol_ring_report,
        },
        MechanicEvidence {
            name: "Arcane Signet",
            scope: "Stored commander identity constrains native mana payments, including partner and empty identities",
            prefixes: &["{T}:"],
            actions: identity::assert_identity_actions,
            report: identity::assert_identity_report,
        },
        MechanicEvidence {
            name: "Command Tower",
            scope: "A land source uses stored commander identity rather than printed commander mana colors",
            prefixes: &["{T}:"],
            actions: identity::assert_identity_actions,
            report: identity::assert_identity_report,
        },
        MechanicEvidence {
            name: "Llanowar Elves",
            scope: "Mature creature taps for green; tapped, sick, and hidden sources are rejected",
            prefixes: &["{T}:"],
            actions: runtime::mana::assert_llanowar,
            report: report::assert_llanowar_report,
        },
        MechanicEvidence {
            name: "Birds of Paradise",
            scope: "Mature creature produces colored, not explicit colorless, mana",
            prefixes: &["{T}:"],
            actions: runtime::mana::assert_birds,
            report: report::assert_birds_report,
        },
        MechanicEvidence {
            name: "Delighted Halfling",
            scope: "Only the printed colorless mode executes; the unsupported legendary and uncounterability mode stays inert",
            prefixes: &["{T}: Add {C}."],
            actions: runtime::mana::assert_halfling_colorless,
            report: report::assert_halfling_report,
        },
        MechanicEvidence {
            name: "Fellwar Stone",
            scope: "Declared active opponent land colors constrain actual payment",
            prefixes: &["{T}:"],
            actions: runtime::mana::assert_fellwar,
            report: report::assert_fellwar_report,
        },
        MechanicEvidence {
            name: "Dark Ritual",
            scope: "Black payment resolves three black mana and funds another cast",
            prefixes: &["Add "],
            actions: runtime::assert_ritual,
            report: report::assert_ritual_report,
        },
    ]
}

/// Search claims retain exact filters and physical destinations through the report boundary.
fn search_evidence() -> Vec<MechanicEvidence> {
    vec![
        MechanicEvidence {
            name: "Nature's Lore",
            scope: "Forest selector moves one physical result onto the battlefield",
            prefixes: &["Search your library"],
            actions: runtime::assert_land_search,
            report: report::assert_land_search_report,
        },
        MechanicEvidence {
            name: "Three Visits",
            scope: "Forest selector moves one physical result onto the battlefield",
            prefixes: &["Search your library"],
            actions: runtime::assert_land_search,
            report: report::assert_land_search_report,
        },
        MechanicEvidence {
            name: "Farseek",
            scope: "Non-Forest subtype selector moves one physical result onto the battlefield tapped",
            prefixes: &["Search your library"],
            actions: runtime::assert_land_search,
            report: report::assert_land_search_report,
        },
        MechanicEvidence {
            name: "Enlightened Tutor",
            scope: "Artifact selection stays on library top rather than becoming hand access",
            prefixes: &["Search your library"],
            actions: runtime::search::assert_top_tutor,
            report: report::assert_top_tutor_report,
        },
        MechanicEvidence {
            name: "Vampiric Tutor",
            scope: "Library-top placement retains two-life loss even when the search fails",
            prefixes: &["Search your library"],
            actions: runtime::search::assert_top_tutor,
            report: report::assert_top_tutor_report,
        },
        MechanicEvidence {
            name: "Trophy Mage",
            scope: "Entry search accepts exactly mana-value-three artifacts",
            prefixes: &["When this creature enters"],
            actions: runtime::search::assert_trophy_mage,
            report: report::assert_trophy_report,
        },
    ]
}

/// Own-cast and finite opponent engines need their actual recipient and payment ledger.
fn engine_evidence() -> Vec<MechanicEvidence> {
    vec![
        MechanicEvidence {
            name: "Harmonize",
            scope: "Pay four mana and draw three exact physical copies",
            prefixes: &["Draw "],
            actions: runtime::assert_draw_three,
            report: report::assert_harmonize_report,
        },
        MechanicEvidence {
            name: "Beast Whisperer",
            scope: "An actual creature cast, not a noncreature cast, draws one card",
            prefixes: &["Whenever you cast"],
            actions: runtime::engines::assert_beast_whisperer,
            report: report::assert_beast_report,
        },
        MechanicEvidence {
            name: "Rhystic Study",
            scope: "One scheduled opponent cast shares a finite tax-payment ledger",
            prefixes: &["Whenever an opponent casts"],
            actions: runtime::engines::assert_rhystic,
            report: report::assert_opponent_engine_report,
        },
        MechanicEvidence {
            name: "Smothering Tithe",
            scope: "One scheduled opponent draw shares finite sources with its next spell",
            prefixes: &["Whenever an opponent draws"],
            actions: runtime::engines::assert_tithe,
            report: report::assert_opponent_engine_report,
        },
    ]
}

/// Cost and recursion claims remain scoped to the physical route that was executed.
fn cost_and_zone_evidence() -> Vec<MechanicEvidence> {
    vec![
        MechanicEvidence {
            name: "Village Rites",
            scope: "A real creature sacrifice precedes two physical draws",
            prefixes: &["As an additional cost", "Draw "],
            actions: runtime::assert_sacrifice_draw,
            report: report::assert_sacrifice_report,
        },
        MechanicEvidence {
            name: "Deadly Dispute",
            scope: "The creature payment route produces two draws and one Treasure mana credit that funds a further cast",
            prefixes: &["As an additional cost", "Draw "],
            actions: runtime::assert_dispute_actions,
            report: report::assert_dispute_report,
        },
        MechanicEvidence {
            name: "Eternal Witness",
            scope: "An actual entry returns the sole chosen graveyard card to hand",
            prefixes: &["When this creature enters"],
            actions: runtime::search::assert_eternal_witness,
            report: report::assert_witness_report,
        },
    ]
}

/// Wider sources vary entry destinations, basic restrictions, and unequal draw counts.
fn wider_corpus_evidence() -> Vec<MechanicEvidence> {
    vec![
        MechanicEvidence {
            name: "Wood Elves",
            scope: "A paid creature entry finds a nonbasic Forest and rejects a Swamp",
            prefixes: &["When this creature enters"],
            actions: runtime::search::assert_entry_land_search,
            report: report::assert_entry_land_report,
        },
        MechanicEvidence {
            name: "Farhaven Elf",
            scope: "A paid optional entry finds a basic Swamp tapped and rejects a nonbasic Forest",
            prefixes: &["When this creature enters"],
            actions: runtime::search::assert_entry_land_search,
            report: report::assert_entry_land_report,
        },
        MechanicEvidence {
            name: "Sift",
            scope: "Four mana draws three physical cards before the visible-card discard choice",
            prefixes: &["Draw "],
            actions: runtime::assert_sift,
            report: report::assert_sift_report,
        },
        MechanicEvidence {
            name: "Gravebreaker Lamia",
            scope: "A paid entry moves one physical library card to the graveyard without a draw",
            prefixes: &["When this creature enters"],
            actions: runtime::assert_graveyard_search,
            report: report::assert_lamia_report,
        },
    ]
}

/// Execute both registered boundaries for a candidate; names select fixtures, not game rules.
pub(in crate::deck::simulator) fn assert_registered_fixture(source: &CardRow) {
    let registry = mechanic_evidence();
    let evidence = registry
        .iter()
        .find(|evidence| evidence.name == source.name)
        .expect("the registered candidate has real mechanic assertions");
    (evidence.actions)(source);
    (evidence.report)(source);
}

/// Execute the supported registry and retain its real function pointers for honest callback counts.
pub(in crate::deck::simulator) fn execute_callbacks() -> Vec<SourceCallbackBinding> {
    let mut callbacks = Vec::new();
    for evidence in mechanic_evidence() {
        let source = named_row(evidence.name);
        (evidence.actions)(&source);
        (evidence.report)(&source);
        callbacks.extend([
            (evidence.name, evidence.actions),
            (evidence.name, evidence.report),
        ]);
    }
    callbacks
}

/// Attach only branch obligations whose exact source line belongs to this compiler record.
pub(in crate::deck::simulator) fn covered_source_branches(
    source: &CardRow,
    wording: &str,
) -> Vec<Value> {
    let registry = mechanic_evidence();
    let Some(evidence) = registry
        .iter()
        .find(|evidence| evidence.name == source.name)
    else {
        return vec![];
    };
    source.oracle_text.lines()
        .filter(|line| evidence.prefixes.iter().any(|prefix| line.starts_with(prefix)))
        .filter(|line| wording.contains(line))
        .map(|line| json!({"source_text": line, "covered_branch": branch_obligations::for_source(evidence.name)}))
        .collect()
}

/// Bind reviewed source text and obligations to executable assertions without serializing function names.
pub(in crate::deck::simulator) fn ability_links(
    evidence: &MechanicEvidence,
    source: &CardRow,
    reviews: &[Value],
) -> Vec<Value> {
    let revision = pinned_revision(source);
    let mut links = Vec::new();
    let branch = branch_obligations::for_source(evidence.name);
    let treasure_model_limit = ["Deadly Dispute", "Smothering Tithe"].contains(&evidence.name)
        .then_some("Treasure rewards use the mana-credit bank. Physical artifact token interactions are not certified.");
    for prefix in evidence.prefixes {
        let matching: Vec<_> = source
            .oracle_text
            .lines()
            .enumerate()
            .filter(|(_, text)| text.starts_with(prefix))
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "{}: exact source ability prefix {prefix}",
            source.name
        );
        let (line, text) = matching[0];
        let review_link = source_review_link(source, text, reviews);
        links.push(json!({
            "oracle_id": source.oracle_id, "metadata_sha256": revision["metadata_sha256"],
            "oracle_text_sha256": revision["oracle_text_sha256"], "flattened_input_sha256": revision["flattened_input_sha256"],
            "face": source.name, "source_line": line + 1, "source_text": text,
            "source_semantic_review": review_link,
            "scope": evidence.scope,
            "assertion_stages": ["production_action_positive", "production_action_near_miss", "full_turn", "production_log_aggregate_report"],
            "covered_branch": branch,
            "timing_scope": "Player actions run in the declared main-phase model. Off-turn priority and other timing permissions are not certified.",
            "treasure_model_limit": treasure_model_limit,
            "remaining_gap": branch.scope_limit(),
            "whole_card_execution": "not_established", "whole_card_reporting": "not_established"
        }));
    }
    links
}

/// Link selected abilities to their independently reviewed face group without closing its gaps.
fn source_review_link(source: &CardRow, text: &str, reviews: &[Value]) -> Value {
    let links = source_review_links(source, text, reviews);
    let [link] = links.as_slice() else {
        panic!(
            "{}: one reviewed group for claimed source line",
            source.name
        );
    };
    link.clone()
}

/// Preserve repeated printed clauses as separate semantic groups without certifying both through one callback.
fn source_review_links(source: &CardRow, text: &str, reviews: &[Value]) -> Vec<Value> {
    let selected = corpus()["cards"]
        .as_array()
        .expect("corpus has cards")
        .iter()
        .any(|card| {
            card["metadata"]["oracle_id"] == source.oracle_id
                && card["selection_tags"]
                    .as_array()
                    .expect("tags are a list")
                    .iter()
                    .any(|tag| tag == "edhrec_top250")
        });
    let review = reviews
        .iter()
        .find(|review| review["oracle_id"] == source.oracle_id);
    if !selected {
        assert!(review.is_none());
        return vec![json!({"disposition": "outside_the_dated_edhrec_selection"})];
    }
    let review = review.expect("every selected mechanic claim has a source review");
    let groups: Vec<_> = review["abilities"]
        .as_array()
        .expect("review has groups")
        .iter()
        .enumerate()
        .filter(|(_, group)| {
            source.name.split(" // ").any(|face| group["face"] == face)
                && group["source_text"]
                    .as_str()
                    .expect("group has source text")
                    .contains(text)
        })
        .collect();
    assert!(
        !groups.is_empty(),
        "{}: printed source has reviewed groups",
        source.name
    );
    groups
        .into_iter()
        .map(|(index, group)| {
            json!({"face": group["face"], "semantic_group_index": index + 1,
        "source_review_sha256": json_hash(review), "disposition": group["disposition"],
        "remaining_source_rules_gap": group["remaining_gap"]})
        })
        .collect()
}

/// Count only source identities whose actual action controls and report assertions pass.
#[test]
fn registered_mechanic_claims_execute_controls_full_turns_and_reports() {
    let registry = mechanic_evidence();
    let reviews = crate::deck::simulator::card_semantic_tests::reviewed_cards();
    let mut executed = HashSet::new();
    let mut ability_count = 0;
    let mut selected_sources = HashSet::new();
    let mut reviewed_groups = HashSet::new();
    for evidence in &registry {
        let source = named_row(evidence.name);
        (evidence.actions)(&source);
        (evidence.report)(&source);
        let links = ability_links(evidence, &source, &reviews);
        ability_count += links.len();
        for link in links {
            for field in [
                "payment",
                "eligibility",
                "effect",
                "negative_controls",
                "action_assertion",
                "report_assertion",
                "approximation",
            ] {
                assert!(
                    link["covered_branch"][field]
                        .as_str()
                        .expect("concrete covered obligation")
                        .len()
                        > 10,
                    "{}: {field}",
                    source.name
                );
            }
            assert!(
                !link["covered_branch"]["report_metrics"]
                    .as_array()
                    .expect("relevant report assertions")
                    .is_empty()
            );
            if let Some(group) = link["source_semantic_review"]["semantic_group_index"].as_u64() {
                selected_sources.insert(source.oracle_id.clone());
                reviewed_groups.insert((source.oracle_id.clone(), group));
            }
        }
        assert!(executed.insert(source.oracle_id));
    }
    assert_eq!(executed.len(), registry.len());
    println!(
        "executed mechanic evidence: {} source identities ({} selected EDHREC identities), {ability_count} source-line links, {} reviewed semantic-group links; zero whole-card claims",
        executed.len(),
        selected_sources.len(),
        reviewed_groups.len()
    );
}
