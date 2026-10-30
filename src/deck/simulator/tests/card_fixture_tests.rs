//! Callable card evidence, with source presence and whole-card claims kept separate.

use super::card_ast_tests as ast;
use super::card_runtime_tests as runtime;
use super::card_test_support::{metadata_row, named_row, source_cards};
use crate::db::CardRow;
use crate::deck::simulator::oracle_lower::parse_sim_card;
use crate::deck::simulator::support::AbilitySupportStatus;
use serde_json::{Value, json};
use std::collections::HashSet;

/// Source-bound mechanic claims with executable control and report assertions.
#[path = "card_evidence_tests.rs"]
pub(super) mod evidence;

/// Published mechanic claims and compiler-label dispositions extend beyond the initial candidates.
#[path = "card_claim_inventory_tests.rs"]
mod claim_inventory;

/// The exact boundary reached by an assertion, not a whole-card support level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    /// Typed runtime data, without an executed action.
    Lowering,
    /// Complete pilot action path for the stated scenario.
    ProductionAction,
    /// Executed turn through the production log, aggregate, and report.
    Report,
}

/// One source card's named assertion scope and the rules it does not establish.
pub(super) struct CardFixture {
    pub(super) name: &'static str,
    pub(super) group: &'static str,
    pub(super) stage: Stage,
    pub(super) disposition: &'static str,
    pub(super) approximation: &'static str,
    pub(super) gap: &'static str,
    assertion: fn(&CardRow),
}

/// Bind an assertion function at compile time rather than recording an unchecked test name.
fn fixture(
    name: &'static str,
    group: &'static str,
    stage: Stage,
    disposition: &'static str,
    approximation: &'static str,
    gap: &'static str,
    assertion: fn(&CardRow),
) -> CardFixture {
    CardFixture {
        name,
        group,
        stage,
        disposition,
        approximation,
        gap,
        assertion,
    }
}

/// Register the initial 25 candidates without calling them fully supported cards.
pub(super) fn initial_fixtures() -> Vec<CardFixture> {
    let mut fixtures = mana_fixtures();
    fixtures.extend(search_fixtures());
    fixtures.extend(engine_fixtures());
    fixtures.extend(cost_and_zone_fixtures());
    fixtures
}

/// Separate native mana input checks from the executed ritual scenario.
fn mana_fixtures() -> Vec<CardFixture> {
    use Stage::*;
    vec![
        fixture(
            "Sol Ring",
            "Tap payment, colorless output, source reuse, actual cast, and report",
            Report,
            "verified_scenario",
            "none",
            "Other timing windows and interactions remain outside this scenario.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Arcane Signet",
            "Native stored-identity payment, partner union, empty-identity controls, and report",
            Report,
            "verified_scenario",
            "none",
            "The source starts in play. Its initial cast and non-native activation routes are outside this source-bound branch.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Fellwar Stone",
            "Declared colors, active seats, native payment, and report",
            Report,
            "verified_scenario",
            "declared opponent model",
            "Only the declared opponent-color model is verified; hidden opponent land information is not modeled.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Llanowar Elves",
            "Green payment, sickness, source reuse, and report",
            Report,
            "verified_scenario",
            "none",
            "Combat interactions and other timing windows are not covered.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Birds of Paradise",
            "Colored payment, sickness, source reuse, and report",
            Report,
            "verified_scenario",
            "none",
            "Flying and combat interactions are not certified by this mana scenario.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Delighted Halfling",
            "Only the printed colorless mana mode, its near misses, and report",
            Report,
            "verified_scenario",
            "none",
            "The second mana mode stays inert because its legendary-only and uncounterability wording cannot lower completely.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Dark Ritual",
            "Printed mana cost and black mana reward",
            Report,
            "verified_scenario",
            "none",
            "Only the declared main-phase pilot and its mana/report effects are covered.",
            evidence::assert_registered_fixture,
        ),
    ]
}

/// Register search evidence without treating blocked instructions as successful searches.
fn search_fixtures() -> Vec<CardFixture> {
    use Stage::*;
    vec![
        fixture(
            "Nature's Lore",
            "Forest selector and battlefield destination",
            Report,
            "verified_scenario",
            "pilot selects the sole legal target",
            "Choice among multiple matching targets remains a deterministic pilot decision.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Three Visits",
            "Forest selector and battlefield destination",
            Report,
            "verified_scenario",
            "pilot selects the sole legal target",
            "Choice among multiple matching targets remains a deterministic pilot decision.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Farseek",
            "Non-Forest subtype selector and tapped destination",
            Report,
            "verified_scenario",
            "pilot selects the sole legal target",
            "Dual-type lands and choice among multiple matching targets need further cases.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Cultivate",
            "Printed split search, rejected name restriction, preserved resources, and report",
            Report,
            "unsupported_safety",
            "none",
            "Printed split searches use bounded selection and destination capacities. Mutations that require different names stay inert; their rejection is not a whole-card support claim.",
            evidence::safety::assert_registered,
        ),
        fixture(
            "Kodama's Reach",
            "Printed split search, rejected name restriction, preserved resources, and report",
            Report,
            "unsupported_safety",
            "none",
            "Printed split searches use bounded selection and destination capacities. Mutations that require different names stay inert; their rejection is not a whole-card support claim.",
            evidence::safety::assert_registered,
        ),
        fixture(
            "Enlightened Tutor",
            "Library-top destination and shuffle",
            Report,
            "verified_scenario",
            "bounded search selection",
            "The action scenario uses one artifact target. Enchantment targets and multiple-choice ordering need further cases.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Vampiric Tutor",
            "Library-top destination, shuffle, and two-life drawback",
            Report,
            "verified_scenario",
            "bounded search selection",
            "The scenario fixes one legal choice; other pilot selections and life-loss interactions are not certified.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Trophy Mage",
            "Exact artifact mana value three",
            Report,
            "verified_scenario",
            "bounded search selection",
            "The optional-search choice uses the deterministic pilot; other entry interactions are not certified.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Sisay, Weatherlight Captain",
            "Rejected legendary and source-power search, preserved resources, changed-text control, and report",
            Report,
            "unsupported_safety",
            "none",
            "Source-power restriction and static power bonus remain unexecuted.",
            evidence::safety::assert_registered,
        ),
    ]
}

/// Keep parser-only opponent engines distinct from the executed draw report.
fn engine_fixtures() -> Vec<CardFixture> {
    use Stage::*;
    vec![
        fixture(
            "Harmonize",
            "Exact three-card draw, mana payment, and report observations",
            Report,
            "verified_scenario",
            "controlled initial zones",
            "Other turn horizons and empty-library boundaries are separate scenarios.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Beast Whisperer",
            "Actual creature-cast filter, physical draw, full turn, and report",
            Report,
            "verified_scenario",
            "none",
            "Other cast replacements and combined trigger chains are outside the scenario.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Rhystic Study",
            "Declared opponent cast, finite tax ledger, reward, and report",
            Report,
            "verified_scenario",
            "synthetic opponent events",
            "The opponent schedule and optional choices are declared model inputs, not multiplayer predictions.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Esper Sentinel",
            "Exact first-event filter, unsupported printed tax wording, preserved resources, and report",
            Report,
            "unsupported_safety",
            "synthetic opponent events",
            "The pinned 'this creature's power' tax wording is unrecognized. No draw executes. Other accepted tax wording does not certify this source.",
            evidence::safety::assert_registered,
        ),
        fixture(
            "Smothering Tithe",
            "Declared opponent draw, finite reserved sources, Treasure, and report",
            Report,
            "verified_scenario",
            "synthetic opponent events; Treasure mana-credit bank",
            "The opponent schedule and optional payments are declared inputs. Physical Treasure artifact interactions are not certified.",
            evidence::assert_registered_fixture,
        ),
    ]
}

/// Keep supported creature payments separate from unverified alternative zone routes.
fn cost_and_zone_fixtures() -> Vec<CardFixture> {
    use Stage::*;
    vec![
        fixture(
            "Village Rites",
            "Creature sacrifice before two draws",
            Report,
            "verified_scenario",
            "sole legal sacrifice",
            "Death-listener chains and off-turn instant timing are separate cases.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Deadly Dispute",
            "Creature sacrifice, two draws, and Treasure",
            Report,
            "verified_scenario",
            "creature route only; Treasure mana-credit bank",
            "Artifact-only sacrifice payment is not implemented. Physical Treasure artifact interactions are not certified.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Eternal Witness",
            "Actual entry, physical graveyard return, near miss, and report",
            Report,
            "verified_scenario",
            "pilot card selection",
            "The optional return uses one unambiguous target and the deterministic pilot.",
            evidence::assert_registered_fixture,
        ),
        fixture(
            "Ox of Agonas",
            "Rejected eight-card Escape route, separate printed hand cost, preserved resources, and report",
            Report,
            "unsupported_safety",
            "none",
            "Escape with eight cards, discard-then-draw entry, and escaped counters remain unsupported.",
            evidence::safety::assert_registered,
        ),
    ]
}

/// Keep every candidate ability visible even when the card scenario covers only a subset.
fn ability_dispositions(source: &CardRow, fixture: &CardFixture) -> Vec<Value> {
    let compiled = parse_sim_card(source);
    compiled.support.iter().map(|ability| {
        let branches = evidence::covered_source_branches(source, &ability.source);
        let safety = evidence::safety::source_branches(source, &ability.source);
        let verified = !branches.is_empty();
        let safety_verified = !safety.is_empty();
        let disposition = match ability.status {
            AbilitySupportStatus::Unsupported { .. } => "unsupported",
            AbilitySupportStatus::Approximated { .. } => "compiler_approximation_only",
            AbilitySupportStatus::Supported | AbilitySupportStatus::CompilerOnly { .. } => "compiler_only",
        };
        json!({
            "oracle_id": source.oracle_id, "flattened_input_sha256": compiled.source_hash,
            "face": source.name, "semantic_ability_index": ability.ability_index,
            "source_text": ability.source, "source_span": ability.span,
            "disposition": disposition, "compiler_evidence": ability.status,
            "card_scenario_scope": fixture.group, "card_scenario_stage": format!("{:?}", fixture.stage),
            "remaining_gap": fixture.gap, "covered_branches": branches, "unsupported_safety_branches": safety,
            "ability_execution": if verified { "scoped_branches_verified" } else if safety_verified { "unsupported_safety_verified" } else { "not_certified" },
            "ability_reporting": if verified { "scoped_branches_verified" } else if safety_verified { "unsupported_safety_verified" } else { "not_certified" },
            "whole_card_execution": "not_established", "whole_card_reporting": "not_established"
        })
    }).collect()
}

/// Assert registered scope, source revisions, and executed function counts together.
#[test]
fn initial_25_source_fixtures_execute_their_registered_assertions() {
    let fixtures = initial_fixtures();
    assert_eq!(fixtures.len(), 25);
    let mut identities = HashSet::new();
    let mut executed = Vec::new();
    for fixture in &fixtures {
        let source = named_row(fixture.name);
        assert!(identities.insert(source.oracle_id.clone()));
        assert!(
            !fixture.group.is_empty()
                && !fixture.gap.is_empty()
                && !fixture.approximation.is_empty()
        );
        ast::assert_source_compiles(&source);
        if fixture.stage == Stage::Report && fixture.disposition == "unsupported_safety" {
            assert!(evidence::safety::has_source(fixture.name));
        }
        (fixture.assertion)(&source);
        let abilities = ability_dispositions(&source, fixture);
        assert!(
            !abilities.is_empty(),
            "{} has per-ability dispositions",
            source.name
        );
        for ability in abilities {
            assert_eq!(ability["oracle_id"], source.oracle_id);
            assert_eq!(ability["face"], source.name);
            let covered = !ability["covered_branches"]
                .as_array()
                .expect("branch obligations are a list")
                .is_empty();
            let expected = if covered {
                "scoped_branches_verified"
            } else if !ability["unsupported_safety_branches"]
                .as_array()
                .expect("safety obligations")
                .is_empty()
            {
                "unsupported_safety_verified"
            } else {
                "not_certified"
            };
            assert_eq!(ability["ability_execution"], expected);
            assert_eq!(ability["ability_reporting"], expected);
            assert_eq!(ability["whole_card_execution"], "not_established");
            if covered {
                assert_ne!(ability["disposition"], "unsupported");
            }
        }
        executed.push((fixture.stage, fixture.disposition));
    }
    assert_eq!(executed.len(), 25);
    assert_eq!(
        executed
            .iter()
            .filter(|(stage, _)| *stage == Stage::Lowering)
            .count(),
        0
    );
    assert_eq!(
        executed
            .iter()
            .filter(|(_, disposition)| *disposition == "unsupported_safety")
            .count(),
        5
    );
    assert_eq!(
        executed
            .iter()
            .filter(|(stage, _)| *stage == Stage::ProductionAction)
            .count(),
        0
    );
    assert_eq!(
        executed
            .iter()
            .filter(|(stage, disposition)| *stage == Stage::Report
                && *disposition == "verified_scenario")
            .count(),
        20
    );
    assert_eq!(
        executed
            .iter()
            .filter(|(stage, _)| *stage == Stage::Report)
            .count(),
        25
    );
    println!(
        "source-card registry: 25 genuine identities; 0 lowering cases; 20 scoped supported report cases; 5 unsupported report cases; zero whole-card claims"
    );
}

/// Compiler/source checks scale to all genuine identities, including the final new capture.
#[test]
fn combined_source_registry_executes_all_356_identity_checks() {
    let cards = source_cards();
    let mut executed = HashSet::new();
    let assertion: fn(&CardRow) = ast::assert_source_compiles;
    for source in &cards {
        let row = metadata_row(&source["metadata"]);
        assertion(&row);
        assert!(executed.insert(row.oracle_id));
    }
    assert_eq!(executed.len(), 356);
    assert_eq!(
        cards.last().expect("registry has a final entry")["name"],
        "Gravebreaker Lamia"
    );
}

/// Keep card-level action controls separate from a full-turn report assertion.
#[test]
fn harmonize_action_controls_use_the_shared_production_boundary() {
    runtime::assert_draw_three(&named_row("Harmonize"));
}

/// Expand beyond the historical selection with a genuine newly captured source.
#[test]
fn wider_corpus_sift_executes_fixed_unequal_draw_discard() {
    evidence::assert_registered_fixture(&named_row("Sift"));
}

/// A second new source exercises an entry search to the player's graveyard.
#[test]
fn wider_corpus_lamia_uses_a_real_graveyard_search_destination() {
    evidence::assert_registered_fixture(&named_row("Gravebreaker Lamia"));
}

/// Historical non-EDHREC entry ramps vary the search class and tapped destination.
#[test]
fn wider_corpus_entry_ramp_varies_basic_and_subtype_searches() {
    let historical = super::card_test_support::corpus();
    for name in ["Wood Elves", "Farhaven Elf"] {
        let source = historical["cards"]
            .as_array()
            .expect("historical corpus")
            .iter()
            .find(|card| card["name"] == name)
            .expect("entry source belongs to the original corpus");
        assert!(
            !source["selection_tags"]
                .as_array()
                .expect("selection tags")
                .iter()
                .any(|tag| tag == "edhrec_top250")
        );
        evidence::assert_registered_fixture(&named_row(name));
    }
}
