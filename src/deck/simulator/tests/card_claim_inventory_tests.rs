//! Audit published mechanic claims and all selected compiler labels without treating either as execution proof.

use super::evidence::covered_source_branches;
use crate::deck::simulator::card_test_support::{corpus, metadata_row, source_cards};
use crate::deck::simulator::oracle_lower::parse_sim_card;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Finite branch obligations and callable packet bindings for each published clause.
#[path = "card_mechanic_obligations.rs"]
mod mechanic_obligations;

/// Maintained public guidance and the runtime's own reader-facing support labels.
const CLAIM_SOURCES: &[(&str, &str)] = &[
    ("README.md", include_str!("../../../../README.md")),
    ("CHANGELOG.md", include_str!("../../../../CHANGELOG.md")),
    (
        "CONTRIBUTING.md",
        include_str!("../../../../CONTRIBUTING.md"),
    ),
    (
        "docs/architecture.md",
        include_str!("../../../../docs/architecture.md"),
    ),
    ("docs/design.md", include_str!("../../../../docs/design.md")),
    (
        "docs/simulator.md",
        include_str!("../../../../docs/simulator.md"),
    ),
    (
        ".agents/skills/seizethemana/SKILL.md",
        include_str!("../../../../.agents/skills/seizethemana/SKILL.md"),
    ),
    (
        "src/deck/simulator/support.rs",
        include_str!("../support.rs"),
    ),
    (
        "src/deck/simulator/report/inputs.rs",
        include_str!("../report/inputs.rs"),
    ),
    (
        "src/deck/simulator/report/assumptions.rs",
        include_str!("../report/assumptions.rs"),
    ),
];

/// Existing regression locations are review inputs, not callable full-path certifications.
const REGRESSION_LINKS: &[(&str, &str, &str)] = &[
    (
        "Mana",
        "oracle_runtime_tests.rs",
        "compound_activation_requires_and_pays_all_costs",
    ),
    (
        "Costs",
        "game_fix_payment_tests.rs",
        "combined_life_costs_gate_on_the_total",
    ),
    (
        "Lands",
        "game_mechanic_tests.rs",
        "landfall_engine_draws_on_land_drops",
    ),
    (
        "Draw and library",
        "library_effect_tests.rs",
        "dredge_replaces_a_draw_only_when_the_full_mill_amount_is_available",
    ),
    (
        "Graveyard",
        "game_fix_payment_tests.rs",
        "own_escape_pays_the_printed_cost_and_exiles_fodder",
    ),
    (
        "Tokens",
        "mechanic_tests.rs",
        "token_effect_yields_count_bodies",
    ),
    (
        "Permanents",
        "oracle_runtime_tests.rs",
        "attack_and_combat_damage_triggers_fire_only_for_an_attacker",
    ),
    (
        "Damage",
        "oracle_runtime_tests.rs",
        "direct_damage_and_life_loss_use_separate_runtime_counters",
    ),
];

/// Keep the full simulator guidance sections and every report assumption in the audit.
fn retained_guidance_statements() -> Vec<Value> {
    let mut statements = Vec::new();
    for (path, text) in CLAIM_SOURCES {
        if *path == "src/deck/simulator/report/assumptions.rs" {
            for (line, wording) in text
                .lines()
                .enumerate()
                .filter(|(_, line)| line.trim_start().starts_with('"'))
            {
                statements.push(json!({"source": path, "line": line + 1, "wording": wording.trim(), "evidence_kind": "model_assumption_not_execution_proof"}));
            }
            continue;
        }
        if !path.ends_with(".md") {
            continue;
        }
        let section = guidance_section(path, text);
        for paragraph in section
            .split("\n\n")
            .filter(|paragraph| !paragraph.trim().is_empty())
        {
            statements.push(json!({"source": path, "wording": paragraph.trim(), "evidence_kind": "published_guidance_not_execution_proof"}));
        }
    }
    statements
}

/// Retain complete simulator sections instead of selecting only already-tested sentences.
fn guidance_section<'a>(path: &str, text: &'a str) -> &'a str {
    let (heading, next) = match path {
        "README.md" => ("### Deck simulation", "\n### "),
        ".agents/skills/seizethemana/SKILL.md" => ("### Simulation (goldfish)", "\n### "),
        "docs/architecture.md" => ("### Simulator runtime", "\n### "),
        "CONTRIBUTING.md" => ("## Add simulator rules and fixtures", "\n## "),
        _ => return text,
    };
    text.split(heading)
        .nth(1)
        .expect("maintained simulator section")
        .split(next)
        .next()
        .expect("section body")
}

/// Preserve each published table clause and its evidence gap rather than replacing it with a smaller claim.
fn documented_runtime_claims() -> Vec<Value> {
    let reference = CLAIM_SOURCES
        .iter()
        .find(|(path, _)| *path == "docs/simulator.md")
        .expect("simulator reference")
        .1;
    let mut claims = Vec::new();
    for (line, text) in reference.lines().enumerate() {
        let columns: Vec<_> = text.split('|').map(str::trim).collect();
        let Some((area, file, test)) = REGRESSION_LINKS
            .iter()
            .find(|(area, _, _)| columns.get(1) == Some(area))
        else {
            continue;
        };
        for clause in columns[2].split(';').flat_map(|clause| clause.split(". ")) {
            if clause.trim().is_empty() {
                continue;
            }
            let progress = mechanic_obligations::progress(clause_obligations(area, clause.trim()));
            claims.push(json!({
                "source": "docs/simulator.md", "line": line + 1, "area": area,
                "published_claim": clause.trim(),
                "area_review_start_not_clause_evidence": {"file": file, "test": test},
                "evidence_kind": "callable_packets_and_explicit_unbound_branches",
                "evidence_scope": clause_evidence_scope(area, clause.trim()),
                "obligations": progress["obligations"],
                "branch_verified": progress["branch_verified"]
            }));
        }
    }
    claims.extend(additional_runtime_claims());
    claims
}

/// Keep every published table clause attached to its exact finite branch requirements.
fn clause_obligations(area: &str, clause: &str) -> &'static [&'static str] {
    match (area, clause.trim_end_matches('.')) {
        ("Mana", "Choice, fixed, flexible, and colorless production") => &[
            "mana.colorless",
            "mana.fixed_color",
            "mana.any_color",
            "mana.identity",
            "mana.alternative_types",
            "mana.declared_opponents",
            "mana.unknown_opponents",
        ],
        ("Mana", "separate paid modes and supported spend restrictions") => {
            &["mana.paid_modes", "mana.spend_restrictions"]
        }
        ("Mana", "Modes share tap state") => &["mana.shared_tap"],
        ("Mana", "an explicit untap can allow another activation") => &["mana.untap_reuse"],
        ("Mana", "Unknown mandatory costs disable that mode") => &["mana.unknown_cost_safety"],
        (
            "Costs",
            "Colored and colorless pips, hybrid and Phyrexian payment, supported additional costs, convoke, improvise, delve, affinity, and flat spell-cost reducers",
        ) => &[
            "mana.colorless",
            "mana.fixed_color",
            "cost.hybrid",
            "cost.phyrexian",
            "cost.creature_sacrifice",
            "cost.discard_life",
            "cost.convoke",
            "cost.improvise_affinity",
            "cost.delve",
            "cost.flat_reducers",
        ],
        ("Costs", "Twobrid uses one matching colored mana or two generic mana") => {
            &["cost.twobrid"]
        }
        (
            "Costs",
            "Snow payments and variable or conditional static cost reducers stay unsupported",
        ) => &["cost.unsupported_symbols_reducers"],
        (
            "Lands",
            "Supported entry conditions, life payment, fetch targets, landfall, and additional land plays",
        ) => &[
            "lands.entry_life",
            "lands.fetch",
            "lands.landfall",
            "lands.extra_plays",
        ],
        (
            "Lands",
            "Stated extra-land counts stack across live sources and effects for the current turn",
        ) => &["lands.extra_plays"],
        ("Lands", "Landfall mana depends on a live resolution pool") => &["lands.landfall_pool"],
        (
            "Draw and library",
            "Shared draw operations, dredge replacement, mill, filtered search, cycling, cascade, loot, and wheels",
        ) => &[
            "draw.fixed",
            "draw.dredge",
            "library.mill",
            "search.types_relations",
            "search.subtype",
            "search.basic",
            "search.colors",
            "search.shuffle_top",
            "search.graveyard_destination",
            "search.split_safety",
            "search.source_power_safety",
            "cast.cycling_landcycling",
            "cast.cascade",
            "draw.loot_wheel",
            "draw.unequal_loot",
        ],
        ("Draw and library", "Board-scaled draw controls use synthetic runtime data") => {
            &["draw.scaled"]
        }
        (
            "Draw and library",
            "they do not establish support for the corresponding printed wording",
        ) => &["draw.scaled"],
        ("Draw and library", "Selection remains a fixed policy") => &[
            "selection.library_first",
            "selection.graveyard_first",
            "draw.unequal_loot",
        ],
        ("Draw and library", "Cascades do not chain") => &["cast.cascade"],
        ("Draw and library", "wheels draw seven") => &["draw.loot_wheel"],
        (
            "Graveyard",
            "Narrow targeted returns, flashback, and the three-card Escape subset move cards and spend resources",
        ) => &[
            "graveyard.return_any",
            "graveyard.return_creature",
            "graveyard.flashback_escape",
        ],
        ("Graveyard", "Other printed Escape counts stay inert") => {
            &["graveyard.other_escape_safety"]
        }
        ("Graveyard", "Filtered and forced-tapped return wording remains limited") => {
            &["graveyard.filtered_return_safety"]
        }
        ("Graveyard", "Access telemetry records first visits rather than full zone histories") => {
            &["access.first_visits"]
        }
        ("Tokens", "Treasures use a payment bank, not battlefield artifacts") => {
            &["tokens.payment_bank"]
        }
        (
            "Tokens",
            "They do not count for artifact totals, costs that require a battlefield artifact, or token entry and sacrifice events",
        ) => &["tokens.payment_bank"],
        ("Tokens", "A committed mana payment consumes them") => &["tokens.payment_bank"],
        ("Tokens", "Most creature tokens use generic bodies, commonly 2/2, with capped counts") => {
            &["tokens.creature_model"]
        }
        (
            "Tokens",
            "Exact token colors, subtypes, stats, and abilities are not general runtime guarantees",
        ) => &["tokens.creature_model"],
        (
            "Permanents",
            "Supported entry, upkeep, cast, landfall, attack, combat-damage, death, and opponent events",
        ) => &[
            "events.self_entry",
            "events.entry_filters",
            "events.upkeep_main_end",
            "events.creature_cast",
            "events.attack_combat_death",
            "lands.landfall",
            "events.opponent_cast_tax",
            "events.opponent_draw_tax",
        ],
        ("Permanents", "typed counters") => &["counters.power_equipment", "cost.x_activations"],
        ("Permanents", "station and crew") => &["permanents.crew_station"],
        ("Permanents", "Saga chapters") => &["permanents.saga"],
        ("Permanents", "loyalty activations") => &["permanents.loyalty"],
        ("Permanents", "Unsupported conditions stay inert") => &["events.conditions_limits"],
        ("Permanents", "Ultimates report affordability rather than resolving") => {
            &["permanents.loyalty"]
        }
        ("Damage", "Player-directed damage and life loss retain supported target scopes") => {
            &["damage.scopes"]
        }
        (
            "Damage",
            "Creature damage, ambiguous targets, prevention, and replacement effects remain limited or inert",
        ) => &["damage.unsupported_targets"],
        ("Damage", "Combat has no blockers") => &["combat.potential"],
        _ => panic!("published mechanic clause needs explicit review: {area}: {clause}"),
    }
}

/// Distinguish synthetic executor checks from safety checks for printed draw wording.
fn clause_evidence_scope(area: &str, clause: &str) -> &'static str {
    match (area, clause.trim_end_matches('.')) {
        ("Draw and library", "Board-scaled draw controls use synthetic runtime data") => {
            "synthetic_executor_only_not_printed_wording_support"
        }
        (
            "Draw and library",
            "they do not establish support for the corresponding printed wording",
        ) => "unsupported_printed_wording_safety_not_supported_execution",
        _ => "scoped_branch_evidence_not_whole_card_support",
    }
}

/// Published draw captions retain both limits without changing the finite branch requirement.
#[test]
fn board_scaled_draw_claims_keep_executor_and_printed_safety_scopes() {
    let claims = documented_runtime_claims();
    for (wording, scope) in [
        (
            "Board-scaled draw controls use synthetic runtime data",
            "synthetic_executor_only_not_printed_wording_support",
        ),
        (
            "they do not establish support for the corresponding printed wording",
            "unsupported_printed_wording_safety_not_supported_execution",
        ),
    ] {
        let claim = claims
            .iter()
            .find(|claim| claim["published_claim"] == wording)
            .expect("the exact published draw limit is inventoried");
        assert_eq!(claim["evidence_scope"], scope);
        let obligations = claim["obligations"].as_array().expect("clause obligations");
        assert_eq!(obligations.len(), 1);
        assert_eq!(obligations[0]["id"], "draw.scaled");
        assert_eq!(
            obligations[0]["required_branch"],
            "Board-scaled draws use the matching count, including zero and counts above eight; unknown scaling must not become a fixed free reward."
        );
    }
}

/// Public mechanic promises outside the table remain part of the same finite inventory.
const ADDITIONAL_CLAUSES: &[(&str, &[&str])] = &[
    (
        "When a supported any-color grant applies",
        &["mana.static_grants"],
    ),
    (
        "Blink-shaped entry effects can re-fire",
        &["permanents.blink_subset"],
    ),
    ("The end-step policy discards", &["hand.cleanup_policy"]),
    ("A companion is the first bench card", &["cast.companion"]),
    ("Supported X-cost spells choose", &["cost.x_spells"]),
    (
        "Supported charge-counter X activations choose",
        &["cost.x_activations"],
    ),
    (
        "Supported sacrifice costs can consume",
        &["cost.activation_sacrifices"],
    ),
    (
        "Unsupported required costs, conditions, targets, and effects",
        &[
            "cost.alternate_safety",
            "mana.incomplete_output_safety",
            "events.scaled_attack_safety",
            "events.esper_tax_safety",
        ],
    ),
    (
        "Scry and surveil choose only from exposed cards",
        &["library.scry_surveil"],
    ),
    (
        "The life-loss reveal policy makes",
        &["library.life_reveal"],
    ),
    (
        "Split cards use the modeled cheaper face",
        &["cast.face_choices"],
    ),
    ("Supported face-down casts pay", &["cast.face_choices"]),
    ("The legend-rule policy keeps", &["cast.legend_rule"]),
    ("One optional kicker payment is attempted", &["cast.kicker"]),
    (
        "Extra turns consume the next configured player turn slots",
        &["turn.extra_slots"],
    ),
    (
        "The commander starts in the command zone",
        &["commander.costs"],
    ),
    (
        "Supported triggers save source identity",
        &["events.settlement_caps"],
    ),
    (
        "A failed draw or zero life ends",
        &["rules.loss_boundaries"],
    ),
    (
        "Only supported counter kinds, keyword grants, and keyword actions",
        &["keywords.declared_limits"],
    ),
    (
        "Readiness requires an instant-speed answer",
        &["interaction.readiness"],
    ),
];

/// Retain actual assumption wording rather than substituting a shorter future-work claim.
fn additional_runtime_claims() -> Vec<Value> {
    let path = "src/deck/simulator/report/assumptions.rs";
    let source = CLAIM_SOURCES
        .iter()
        .find(|(name, _)| *name == path)
        .expect("public assumptions")
        .1;
    ADDITIONAL_CLAUSES.iter().map(|(anchor, ids)| {
        let (line, wording) = source.lines().enumerate().find(|(_, line)| line.contains(anchor)).expect("published mechanic promise remains inventoried");
        let progress = mechanic_obligations::progress(ids);
        json!({"source": path, "line": line + 1, "area": "Additional published mechanics", "published_claim": wording.trim(), "obligations": progress["obligations"], "branch_verified": progress["branch_verified"]})
    }).collect()
}

/// Audit all maintained claim sources and keep compiler labels separate from runtime claims.
#[test]
fn maintained_claim_inventory_preserves_published_branches_and_regression_links() {
    let execution = mechanic_obligations::execute_bound_packets();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/deck/simulator/tests");
    for (area, file, test) in REGRESSION_LINKS {
        let source =
            std::fs::read_to_string(root.join(file)).expect("linked regression file exists");
        assert!(
            source.contains(&format!("fn {test}(")),
            "{area}: exact regression function"
        );
    }
    let claims = documented_runtime_claims();
    let areas: BTreeSet<_> = claims
        .iter()
        .filter(|claim| claim["source"] == "docs/simulator.md")
        .map(|claim| claim["area"].as_str().expect("claim area"))
        .collect();
    assert_eq!(areas.len(), REGRESSION_LINKS.len());
    assert!(claims.len() > REGRESSION_LINKS.len());
    for claim in &claims {
        assert!(
            claim["published_claim"]
                .as_str()
                .expect("published wording")
                .len()
                > 10
        );
        let obligations = claim["obligations"]
            .as_array()
            .expect("exact clause obligations");
        assert!(!obligations.is_empty());
        assert_eq!(
            claim["branch_verified"],
            obligations
                .iter()
                .all(|obligation| obligation["branch_verified"] == true)
        );
    }
    let linked: BTreeSet<_> = claims
        .iter()
        .flat_map(|claim| claim["obligations"].as_array().expect("clause obligations"))
        .map(|obligation| obligation["id"].as_str().expect("obligation identity"))
        .collect();
    assert_eq!(linked, mechanic_obligations::ids());
    let remaining = mechanic_obligations::remaining();
    let closed = claims
        .iter()
        .filter(|claim| claim["branch_verified"] == true)
        .count();
    let sources: Vec<_> = CLAIM_SOURCES.iter().map(|(path, text)| {
        assert!(!text.is_empty(), "{path}");
        json!({"source": path, "sha256": crate::deck::simulator::card_test_support::byte_hash(text.as_bytes())})
    }).collect();
    let statements = retained_guidance_statements();
    for (path, _) in CLAIM_SOURCES
        .iter()
        .filter(|(path, _)| path.ends_with(".md") || path.ends_with("assumptions.rs"))
    {
        assert!(
            statements
                .iter()
                .any(|statement| statement["source"] == *path),
            "{path}: guidance is not omitted"
        );
    }
    println!(
        "maintained claim audit: {} sources, {} retained guidance statements, {} published clauses across {} table areas; {closed} clauses closed, {} exact branch obligations remain",
        sources.len(),
        statements.len(),
        claims.len(),
        areas.len(),
        remaining.len()
    );
    if let Some(path) = std::env::var_os("STM_CLAIM_INVENTORY") {
        let inventory = json!({"execution_gate": "all registered callback bindings passed",
            "execution": execution,
            "published_claims": &claims, "remaining_branch_obligations": &remaining,
            "claim_sources": &sources, "whole_card_claims": 0});
        std::fs::write(
            path,
            serde_json::to_string_pretty(&inventory).expect("claim inventory serializes") + "\n",
        )
        .expect("write the explicitly requested validated branch-evidence inventory");
    }
    for obligation in remaining {
        println!(
            "remaining branch {}: {} | missing stages: {} | candidates: {}",
            obligation["id"],
            obligation["required_branch"],
            obligation["unbound_required_stages"],
            obligation["existing_test_candidates"]
        );
    }
}

/// Inventory every selected compiler ability, including cards outside the 25 initial fixtures.
#[test]
fn all_selected_compiler_labels_have_explicit_runtime_evidence_dispositions() {
    let selected: BTreeSet<_> = corpus()["cards"]
        .as_array()
        .expect("corpus cards")
        .iter()
        .filter(|card| {
            card["selection_tags"]
                .as_array()
                .expect("selection tags")
                .iter()
                .any(|tag| tag == "edhrec_top250")
        })
        .map(|card| {
            card["metadata"]["oracle_id"]
                .as_str()
                .expect("Oracle identity")
                .to_string()
        })
        .collect();
    let mut source_ids = BTreeSet::new();
    let mut compiler_counts = BTreeMap::<String, usize>::new();
    let mut covered_records = 0;
    let mut unlinked_complete = BTreeSet::new();
    let mut ability_inventory = Vec::new();
    for source in source_cards() {
        let row = metadata_row(&source["metadata"]);
        if !selected.contains(&row.oracle_id) {
            continue;
        }
        assert!(source_ids.insert(row.oracle_id.clone()));
        let card = parse_sim_card(&row);
        let syntax = crate::deck::simulator::oracle_parser::parse_oracle_card(&row);
        assert_eq!(
            card.source_hash,
            crate::deck::simulator::support::card_source_hash(&row)
        );
        for ability in &card.support {
            let status = serde_json::to_value(&ability.status).expect("compiler status");
            let label = status["status"].as_str().expect("compiler status label");
            *compiler_counts.entry(label.to_string()).or_default() += 1;
            let branches = covered_source_branches(&row, &ability.source);
            let safety = super::evidence::safety::source_branches(&row, &ability.source);
            let packets = super::evidence::packets::source_branches(&row, &ability.source);
            if branches.is_empty() {
                if label != "unsupported" {
                    unlinked_complete.insert(row.name.clone());
                }
            } else {
                assert_ne!(label, "unsupported", "{}: covered source line", row.name);
                covered_records += 1;
            }
            if label == "unsupported" || label == "compiler_only" {
                assert!(
                    !status["reason"]
                        .as_str()
                        .expect("compiler reason")
                        .is_empty()
                );
            }
            if label == "approximated" {
                assert!(
                    !status["rule_id"]
                        .as_str()
                        .expect("declared approximation")
                        .is_empty()
                );
            }
            let unbound_required_stages =
                if !branches.is_empty() || !safety.is_empty() || !packets.is_empty() {
                    vec![]
                } else if label == "unsupported" {
                    vec!["unsupported_safety", "report_gap_disposition"]
                } else {
                    vec![
                        "positive",
                        "negative",
                        "complete_production_action",
                        "relevant_executed_report",
                    ]
                };
            ability_inventory.push(json!({
                "oracle_id": row.oracle_id, "name": row.name, "source_hash": card.source_hash,
                "ability_index": ability.ability_index, "source_wording": ability.source,
                "typed_mechanic": format!("{:?}", syntax.abilities[ability.ability_index]),
                "compiler_evidence": status,
                "compiler_label_is_not_execution_claim": true,
                "unbound_required_stages": unbound_required_stages,
                "covered_branches": branches,
                "unsupported_safety_evidence": safety,
                "reviewed_packet_evidence": packets,
                "whole_card_execution": "not_established"
            }));
        }
    }
    assert_eq!(source_ids, selected);
    assert_eq!(source_ids.len(), 250);
    assert_eq!(
        ability_inventory.len(),
        compiler_counts.values().sum::<usize>()
    );
    for record in &ability_inventory {
        assert_eq!(
            record["source_hash"]
                .as_str()
                .expect("exact source hash")
                .len(),
            64
        );
        assert!(
            !record["typed_mechanic"]
                .as_str()
                .expect("actual typed mechanic")
                .is_empty()
        );
        assert_eq!(record["whole_card_execution"], "not_established");
    }
    println!(
        "selected compiler inventory: {compiler_counts:?}; {covered_records} exact source records have registered branch evidence"
    );
    println!(
        "compiler-labelled cards without source-bound branch registration (not whole-card claims): {unlinked_complete:?}"
    );
}
