//! A finite published-rule inventory separates completed packets from exact remaining branch reviews.

use super::super::evidence;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Candidate functions are review locations, not unchecked execution certificates.
type TestReference = (&'static str, &'static str);

/// A clause closes only through a registered callable packet, not through a compiler label.
#[derive(Clone, Copy)]
enum Binding {
    Supported(&'static str),
    Safety(&'static str),
    SafetyPair(&'static str, &'static str),
    Packets(&'static [&'static str], &'static [&'static str]),
    Pending,
}

/// One concrete branch and the existing functions that can help complete its review.
struct Obligation {
    id: &'static str,
    requirement: &'static str,
    binding: Binding,
    candidates: &'static [TestReference],
}

/// Mana and cast-cost branches keep the finite catalog below the file-size limit.
#[path = "card_mana_cost_obligations.rs"]
mod mana_cost;

/// Library movement and replay branches have their own co-located catalog section.
#[path = "card_zone_obligations.rs"]
mod zones;

/// Every entry belongs to a retained published clause; no source card is certified by this list.
const OBLIGATIONS: &[Obligation] = &[
    Obligation {
        id: "mana.static_grants",
        requirement: "The pilot selects an independent one-any-color tap ability. The native ability's spend restrictions, extra costs, and conditions do not apply to it. Physical tap state and creature tap eligibility still apply; positive and negative production actions and executed reports enforce these boundaries.",
        binding: Binding::Packets(
            &[
                "static_grant_boundary",
                "static_grant_restrictions_boundary",
                "empty_identity_grant_leftover_boundary",
                "granted_main_work_boundary",
                "granted_positive_loop_boundary",
            ],
            &[],
        ),
        candidates: &[(
            "tests/game_fix_payment_tests.rs",
            "static_grant_converts_every_land_tap",
        )],
    },
    Obligation {
        id: "permanents.blink_subset",
        requirement: "The declared blink-shaped subset re-fires the host's entry trigger once on the next turn without claiming an exile-and-return zone change; ordinary entry searches do not re-arm and reports show only the actual reward.",
        binding: Binding::Packets(&["blink_boundary"], &[]),
        candidates: &[(
            "tests/game_fix_board_tests.rs",
            "blink_refires_once_land_search_does_not",
        )],
    },
    Obligation {
        id: "hand.cleanup_policy",
        requirement: "End-step cleanup discards the highest-mana-value physical cards down to seven unless a live supported source removes the limit; discards reach graveyard access reports without becoming draws or library-only triggers.",
        binding: Binding::Packets(&["cleanup_boundary"], &[]),
        candidates: &[
            (
                "tests/game_fix_board_tests.rs",
                "end_step_discard_records_discard_turn",
            ),
            (
                "tests/parse_mechanic_tests.rs",
                "no_max_hand_size_and_x_counters_lower_from_static_data",
            ),
        ],
    },
    Obligation {
        id: "cast.companion",
        requirement: "The declared first-bench companion stays outside the opening hand and library, moves to hand only after one actual three-mana payment, cannot be fetched twice, and reports only its executed access; its deck condition remains an assumption.",
        binding: Binding::Packets(&["companion_boundary"], &[]),
        candidates: &[
            (
                "tests/counter_keyword_tests.rs",
                "companion_stays_out_of_the_shuffled_library",
            ),
            (
                "tests/counter_keyword_tests.rs",
                "companion_fetches_once_when_three_is_payable",
            ),
        ],
    },
    Obligation {
        id: "tokens.payment_bank",
        requirement: "Source-created Treasure bank units persist across turns, are not floated unused mana or battlefield artifacts, preserve failed payments, pay once, and cannot supply artifact totals, artifact costs, or artifact events.",
        binding: Binding::Supported("Deadly Dispute"),
        candidates: &[
            (
                "tests/card_boundary_cost_tests.rs",
                "assert_treasure_bank_boundary",
            ),
            (
                "tests/card_boundary_cost_tests.rs",
                "assert_treasure_bank_report",
            ),
        ],
    },
    Obligation {
        id: "tokens.creature_model",
        requirement: "Declared generic bodies and specialized paths preserve their stated counts; token work beyond the cap reports incomplete work, and exact printed colors, subtypes, sizes, or abilities are not silently certified.",
        binding: Binding::Packets(&["token_cap", "miirym"], &[]),
        candidates: &[
            (
                "tests/game_run_runtime_tests.rs",
                "token_effect_cap_requires_uncreated_objects",
            ),
            (
                "tests/mechanic_tests.rs",
                "token_effect_yields_count_bodies",
            ),
        ],
    },
    Obligation {
        id: "events.self_entry",
        requirement: "An actual paid source entry fires its own trigger once, selects only the exact eligible artifact, and reports the resulting physical hand access.",
        binding: Binding::Supported("Trophy Mage"),
        candidates: &[
            ("tests/card_search_tests.rs", "assert_trophy_mage"),
            ("tests/card_report_tests.rs", "assert_trophy_report"),
        ],
    },
    Obligation {
        id: "events.entry_filters",
        requirement: "Other-permanent, controlled-type, and nontoken entry filters accept only matching physical entries, including generic token batches, while preserving captured listener identity.",
        binding: Binding::Packets(
            &["miirym", "impact", "bank_events", "entry_capture_boundary"],
            &[],
        ),
        candidates: &[
            (
                "tests/oracle_runtime_tests.rs",
                "compound_entry_trigger_resolves_all_effects_once",
            ),
            (
                "tests/resolution_tests.rs",
                "multi_card_mill_finishes_before_library_return_etb_draws",
            ),
        ],
    },
    Obligation {
        id: "events.upkeep_main_end",
        requirement: "Upkeep, first-main, and end-step rewards run only at their own boundaries; cumulative upkeep consumes real sources, failure sacrifices the source, and Monarch draws at the end step.",
        binding: Binding::Packets(
            &[
                "ring",
                "remora",
                "phase_rewards_boundary",
                "upkeep_condition_boundary",
                "upkeep_target_generation_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/oracle_runtime_tests.rs",
                "first_main_and_end_step_triggers_fire_at_their_turn_boundaries",
            ),
            (
                "tests/opponent_runtime_tests.rs",
                "cumulative_upkeep_increases_and_does_not_float_main_phase_mana",
            ),
            (
                "tests/game_fix_board_tests.rs",
                "monarch_draws_extra_card_per_turn",
            ),
        ],
    },
    Obligation {
        id: "events.creature_cast",
        requirement: "An actual creature cast, not a sorcery cast, causes one physical draw and an executed report records exactly that reward.",
        binding: Binding::Supported("Beast Whisperer"),
        candidates: &[
            ("tests/card_engine_tests.rs", "assert_beast_whisperer"),
            ("tests/card_report_tests.rs", "assert_beast_report"),
        ],
    },
    Obligation {
        id: "events.attack_combat_death",
        requirement: "Only eligible actual attackers cause attack and combat-damage events; supported death filters use last-known token/card characteristics and captured sources after simultaneous deaths.",
        binding: Binding::Packets(
            &[
                "combat_events",
                "solemn_death",
                "plunderer_death",
                "reclamation_death",
                "cauldron_loss",
                "death_capture_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/oracle_runtime_tests.rs",
                "attack_and_combat_damage_triggers_fire_only_for_an_attacker",
            ),
            (
                "game_effects/tests/activation_tests.rs",
                "simultaneous_deaths_keep_last_known_listener_sources",
            ),
        ],
    },
    Obligation {
        id: "events.opponent_cast_tax",
        requirement: "One scheduled paid cast shares a finite opponent tax ledger; paid tax suppresses the reward, unpaid tax draws one physical player card, and reported activity is not multiplayer prediction.",
        binding: Binding::Supported("Rhystic Study"),
        candidates: &[
            ("tests/card_engine_tests.rs", "assert_rhystic"),
            (
                "tests/card_report_tests.rs",
                "assert_opponent_engine_report",
            ),
        ],
    },
    Obligation {
        id: "events.opponent_draw_tax",
        requirement: "One scheduled draw shares finite reserved sources with the next opponent spell; only unpaid tax produces the declared Treasure bank reward and actual follow-up spend.",
        binding: Binding::Supported("Smothering Tithe"),
        candidates: &[
            ("tests/card_engine_tests.rs", "assert_tithe"),
            (
                "tests/card_report_tests.rs",
                "assert_opponent_engine_report",
            ),
        ],
    },
    Obligation {
        id: "events.conditions_limits",
        requirement: "Supported true/false conditions and once-per-turn limits use the correct event and source identity; unknown conditions leave the full ability inert and do not consume unrelated usage flags.",
        binding: Binding::Packets(
            &[
                "elite_safety",
                "ring",
                "event_usage_boundary",
                "upkeep_condition_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/oracle_runtime_tests.rs",
                "intervening_if_condition_gates_the_trigger_resolution",
            ),
            (
                "tests/game_fix_zone_trigger_tests.rs",
                "trigger_and_activation_once_flags_do_not_cross",
            ),
        ],
    },
    Obligation {
        id: "events.scaled_attack_safety",
        requirement: "Dwynen's real paid attack with several Elves grants no invented scaled life; changed fixed-life wording proves that the attack timing and report can observe a supported reward.",
        binding: Binding::Safety("Dwynen, Gilt-Leaf Daen"),
        candidates: &[
            (
                "tests/compiler_safety_action_tests.rs",
                "assert_dwynen_attack_safety",
            ),
            (
                "tests/compiler_safety_action_tests.rs",
                "assert_dwynen_fixed_attack_control",
            ),
        ],
    },
    Obligation {
        id: "events.esper_tax_safety",
        requirement: "Esper's exact first-noncreature event keeps its unsupported printed tax wording; no free draw, resource change, or hidden source-power alias is introduced.",
        binding: Binding::Safety("Esper Sentinel"),
        candidates: &[
            ("tests/card_engine_tests.rs", "assert_esper_inert"),
            ("tests/card_report_tests.rs", "assert_esper_report"),
        ],
    },
    Obligation {
        id: "counters.power_equipment",
        requirement: "Supported positive and negative counters, static bonuses, and paid equipment use the same physical host for combat, crew, and station; face-down characteristics and unknown power do not leak printed values.",
        binding: Binding::Packets(&["counter_consumers", "power_consumers_boundary"], &[]),
        candidates: &[
            (
                "tests/mechanics_tests.rs",
                "combat_power_includes_positive_and_negative_counters",
            ),
            (
                "tests/game_fix_board_tests.rs",
                "equipment_buff_survives_board_shift",
            ),
        ],
    },
    Obligation {
        id: "permanents.crew_station",
        requirement: "Crew and station pay with distinct eligible physical bodies and actual power; sick bodies can crew or station, but a newly animated vehicle cannot evade attack sickness.",
        binding: Binding::Packets(
            &[
                "vehicle",
                "station",
                "counter_consumers",
                "sick_crew_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/game_fix_zone_trigger_tests.rs",
                "summoning_sick_bodies_crew_and_station",
            ),
            (
                "tests/game_fix_zone_trigger_tests.rs",
                "entry_turn_crewed_vehicle_waits_one_turn_to_attack",
            ),
        ],
    },
    Obligation {
        id: "permanents.saga",
        requirement: "Scheduled lore and read-ahead execute only the modeled chapters once, retain compound resolution boundaries, and report actual chapter rewards and limits.",
        binding: Binding::Packets(
            &[
                "saga",
                "saga_resolution_boundary",
                "saga_entry_cleanup_boundary",
                "saga_chapter_boundaries",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/mechanic_saga_tests.rs",
                "saga_iv_chapter_fires_and_saga_leaves_board",
            ),
            (
                "game_run/tests/census_boundary_tests.rs",
                "saga_chapter_uses_one_complete_resolution_boundary",
            ),
        ],
    },
    Obligation {
        id: "permanents.loyalty",
        requirement: "Stored loyalty pays one modeled activation per turn; insufficient loyalty blocks it and ultimate affordability remains a diagnostic rather than an executed win.",
        binding: Binding::Packets(&["loyalty", "loyalty_payment_boundary"], &[]),
        candidates: &[
            (
                "tests/game_mechanic_tests.rs",
                "planeswalker_fires_loyalty_and_gains",
            ),
            (
                "tests/game_mechanic_tests.rs",
                "loyalty_token_plus1_counts_as_engine",
            ),
        ],
    },
    Obligation {
        id: "damage.scopes",
        requirement: "Player-directed damage and direct life loss retain their target scopes and separate counters; inactive or eliminated seats and opponent-round timing remain correct through reports.",
        binding: Binding::Packets(
            &[
                "impact",
                "cauldron_loss",
                "damage_scopes",
                "paid_x",
                "damage_boundaries",
                "opponent_damage_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/oracle_runtime_tests.rs",
                "direct_damage_and_life_loss_use_separate_runtime_counters",
            ),
            (
                "tests/opponent_runtime_tests.rs",
                "opponent_round_damage_is_added_after_the_turn_census",
            ),
        ],
    },
    Obligation {
        id: "damage.unsupported_targets",
        requirement: "Unsupported creature damage, ambiguous targets, prevention, and replacement wording cannot leave a partial executable reward or claim resolved interaction in a report.",
        binding: Binding::Packets(&["damage_safety", "bolt_safety", "zacama_safety"], &[]),
        candidates: &[
            (
                "tests/oracle_runtime_tests.rs",
                "unsupported_spell_text_stays_inert_after_a_legal_cast",
            ),
            (
                "tests/compiler_safety_action_tests.rs",
                "assert_force_of_negation_payment",
            ),
        ],
    },
    Obligation {
        id: "combat.potential",
        requirement: "Combat is unblocked potential, not applied seat damage or wins; partners remain separate, no live defender suppresses attack side effects, and report thresholds retain their actual denominators.",
        binding: Binding::Packets(
            &[
                "combat_potential",
                "combat_events",
                "partner_potential_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/combat_contract_tests.rs",
                "no_live_defender_has_no_attack_or_damage_side_effects",
            ),
            (
                "tests/combat_contract_tests.rs",
                "partner_threshold_rates_stay_separate_through_reporting",
            ),
        ],
    },
    Obligation {
        id: "library.scry_surveil",
        requirement: "Scry and surveil choose only from exposed cards using visible land needs and the declared cost estimate. Scry can order kept cards and bottom rejected cards; surveil moves only rejected cards and preserves kept order. Paid and unpaid actions conserve physical cards, bound awareness to available cards, and keep selection, mill, and draw reports distinct.",
        binding: Binding::Packets(&["scry_surveil_boundary"], &[]),
        candidates: &[
            (
                "tests/zone_boundary_library_tests.rs",
                "assert_scry_surveil_report",
            ),
            (
                "game_effects/tests/selection_tests.rs",
                "surveil_rejects_only_unwanted_cards_and_preserves_kept_order",
            ),
        ],
    },
    Obligation {
        id: "library.life_reveal",
        requirement: "The mandatory first reveal occurs when a card exists, may be lethal, and optional repeats use visible life and known-list risk rather than the next hidden card.",
        binding: Binding::Packets(&["life_reveal_boundary"], &[]),
        candidates: &[(
            "tests/library_effect_tests.rs",
            "reveal_life_rule_uses_visible_risk_for_optional_repeats",
        )],
    },
    Obligation {
        id: "cast.face_choices",
        requirement: "Split, MDFC, plot, morph, and face-up paths enforce their declared cost and timing policies, do not grant unsupported Warp or manifest routes, and do not duplicate entry effects.",
        binding: Binding::Packets(
            &[
                "three_life_mdfc",
                "tapped_mdfc",
                "any_return",
                "face_routes_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/mechanic_tests.rs",
                "mdfc_plays_as_land_when_no_land_in_hand",
            ),
            (
                "tests/plot_mana_boundary_tests.rs",
                "nonterminal_plotted_spells_keep_all_legal_casts",
            ),
            ("tests/counter_keyword_tests.rs", "morph_cost_parses"),
        ],
    },
    Obligation {
        id: "cast.legend_rule",
        requirement: "Two same-name legendary physical permanents keep the earlier copy under the declared policy; distinct names and nonlegendary controls remain unaffected in actual state and reports.",
        binding: Binding::Packets(&["legend_policy_boundary"], &[]),
        candidates: &[
            (
                "tests/game_fix_payment_tests.rs",
                "legend_rule_keeps_one_copy_per_name",
            ),
            (
                "tests/game_fix_payment_tests.rs",
                "legend_rule_allows_distinct_names",
            ),
        ],
    },
    Obligation {
        id: "cast.kicker",
        requirement: "The single optional kicker payment consumes only usable spare mana and life; unavailable or restricted resources do not pay it and no full kicker resolution is inferred.",
        binding: Binding::Packets(&["kicker_boundary"], &[]),
        candidates: &[
            (
                "tests/game_mechanic_tests.rs",
                "kicker_pays_from_spare_mana",
            ),
            (
                "tests/game_mechanic_tests.rs",
                "kicker_ignores_restricted_bucket_mana",
            ),
        ],
    },
    Obligation {
        id: "turn.extra_slots",
        requirement: "Extra turns consume configured player slots, replay supported player actions, and delay rather than generate another opponent round.",
        binding: Binding::Packets(&["extra_slots_boundary"], &[]),
        candidates: &[
            (
                "tests/game_run_runtime_tests.rs",
                "extra_turns_do_not_generate_opponent_activity",
            ),
            (
                "tests/game_mechanic_tests.rs",
                "extra_turn_replays_land_drop_and_engines",
            ),
        ],
    },
    Obligation {
        id: "commander.costs",
        requirement: "Command-zone casts pay colored pips and cumulative two-mana tax before entry; unpayable, unsupported, and last-life routes preserve the correct timing and report observation.",
        binding: Binding::Packets(&["commander_payment_boundary"], &[]),
        candidates: &[
            (
                "tests/game_run_runtime_tests.rs",
                "commander_recasts_pay_increasing_tax",
            ),
            (
                "tests/preloss_event_tests.rs",
                "last_life_commander_payment_keeps_payability_and_report_timing",
            ),
        ],
    },
    Obligation {
        id: "events.settlement_caps",
        requirement: "Supported triggers retain captured source identity and characteristics, settle after the causing action, and report unresolved work at caps without marking an exactly finished queue incomplete.",
        binding: Binding::Packets(
            &[
                "settlement_boundary",
                "granted_main_work_boundary",
                "entry_capture_boundary",
                "death_capture_boundary",
                "upkeep_condition_boundary",
                "upkeep_target_generation_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/resolution_tests.rs",
                "multi_card_mill_finishes_before_library_return_etb_draws",
            ),
            (
                "tests/cast_limit_tests.rs",
                "hand_budget_checks_face_down_and_alternative_payment_after_the_last_cast",
            ),
        ],
    },
    Obligation {
        id: "rules.loss_boundaries",
        requirement: "Failed draws and zero life stop later actions at the supported boundary while linked effects finish as required; mill or search failure alone is not loss, and reports preserve reached observations.",
        binding: Binding::Packets(&["damage_boundaries", "cascade", "loss_boundary"], &[]),
        candidates: &[
            (
                "tests/cascade_boundary_tests.rs",
                "cascade_failed_draw_finishes_linked_child_effects_only",
            ),
            (
                "tests/preloss_event_tests.rs",
                "terminal_draw_and_mill_keep_land_access_without_an_end_turn",
            ),
        ],
    },
    Obligation {
        id: "keywords.declared_limits",
        requirement: "Supported mobilize, amass, offspring, explore, and connive paths retain their declared generic-body, quantity, target, and counter limits; recognition alone never certifies printed subtype or ability behavior.",
        binding: Binding::Packets(&["keyword_limits_boundary"], &[]),
        candidates: &[
            (
                "tests/mechanics_tests.rs",
                "mobilize_tokens_are_sacrificed_and_fire_death_triggers",
            ),
            (
                "tests/mechanics_tests.rs",
                "amass_grows_a_single_army_creature",
            ),
            ("tests/mechanics_tests.rs", "connive_parses_and_loots"),
        ],
    },
    Obligation {
        id: "interaction.readiness",
        requirement: "Only an instant-speed answer still held and payable after main actions counts as ready; report spare mana and readiness never imply that removal or a counter actually resolved.",
        binding: Binding::Packets(&["bolt_safety", "readiness_boundary"], &[]),
        candidates: &[
            (
                "tests/game_fix_payment_tests.rs",
                "interaction_readiness_spends_only_instant_legal_mana",
            ),
            (
                "tests/game_run_runtime_tests.rs",
                "interaction_readiness_handles_mana_that_cannot_pay_generic_costs",
            ),
        ],
    },
];

/// Iterate the complete finite catalog without duplicating its thematic sections.
fn obligations() -> impl Iterator<Item = &'static Obligation> {
    mana_cost::OBLIGATIONS
        .iter()
        .chain(zones::OBLIGATIONS)
        .chain(OBLIGATIONS)
}

/// Only reviewed callable evidence with no missing stage can close an obligation.
fn is_bound(binding: Binding) -> bool {
    match binding {
        Binding::Supported(name) => evidence::mechanic_evidence()
            .iter()
            .any(|entry| entry.name == name),
        Binding::Safety(name) => evidence::safety::has_source(name),
        Binding::SafetyPair(first, second) => {
            evidence::safety::has_source(first) && evidence::safety::has_source(second)
        }
        Binding::Packets(ids, missing) => {
            assert!(!ids.is_empty());
            for id in ids {
                assert!(
                    evidence::packets::has_packet(id),
                    "missing callable packet: {id}"
                );
            }
            missing.is_empty()
        }
        Binding::Pending => false,
    }
}

/// Preserve the exact branch requirement and candidate functions beside its current closure state.
fn record(obligation: &Obligation) -> Value {
    let candidates: Vec<_> = obligation
        .candidates
        .iter()
        .map(|(file, function)| json!({"file": file, "function": function}))
        .collect();
    let (packets, missing) = match obligation.binding {
        Binding::Packets(ids, missing) => (evidence::packets::records(ids), missing),
        Binding::Pending => (
            vec![],
            &[
                "source_bound_packet_review",
                "positive_and_negative_controls",
                "complete_production_action",
                "relevant_executed_report",
            ][..],
        ),
        _ => (vec![], &[][..]),
    };
    json!({"id": obligation.id, "required_branch": obligation.requirement,
        "branch_verified": is_bound(obligation.binding), "existing_test_candidates": candidates,
        "executed_packets": packets, "unbound_required_stages": missing,
        "candidate_scope": "Candidates need stage review and explicit linkage. Their existence alone is not credited as positive, negative, complete-action, or report proof."})
}

/// A published clause closes only after every named branch has callable evidence.
pub(super) fn progress(ids: &[&str]) -> Value {
    let obligations: Vec<_> = ids
        .iter()
        .map(|id| {
            record(
                obligations()
                    .find(|obligation| obligation.id == *id)
                    .expect("published branch is inventoried"),
            )
        })
        .collect();
    let closed = obligations
        .iter()
        .all(|obligation| obligation["branch_verified"] == true);
    json!({"branch_verified": closed, "obligations": obligations})
}

/// Return an exact finite work list, without replacing published claims with future-work labels.
pub(super) fn remaining() -> Vec<Value> {
    obligations()
        .filter(|obligation| !is_bound(obligation.binding))
        .map(record)
        .collect()
}

/// Every catalog branch must be accounted for by the retained published clauses.
pub(super) fn ids() -> BTreeSet<&'static str> {
    obligations().map(|obligation| obligation.id).collect()
}

/// Execute each completed packet once before the clause inventory can report it as closed.
pub(super) fn execute_bound_packets() -> Value {
    let mut callbacks = evidence::execute_callbacks();
    callbacks.extend(evidence::safety::execute_callbacks());
    let previous: Vec<_> = callbacks
        .iter()
        .map(|(source, function)| {
            (
                Some(*source),
                evidence::packets::Callback::Row("registered source callback", *function),
            )
        })
        .collect();
    let (packet_bindings, packet_callbacks) = evidence::packets::execute_new_bindings(&previous);
    let source_bindings = callbacks.len();
    let mut unique = Vec::new();
    for callback in callbacks
        .into_iter()
        .map(|(_, function)| {
            evidence::packets::Callback::Row("registered source callback", function)
        })
        .chain(packet_callbacks)
    {
        if !unique
            .iter()
            .any(|previous: &evidence::packets::Callback| previous.same_function(callback))
        {
            unique.push(callback);
        }
    }
    println!(
        "executed catalog: {source_bindings} source/function bindings; {packet_bindings} packet source/function bindings; {} distinct registered callbacks total; transitive helper calls are not counted",
        unique.len()
    );
    json!({"source_function_bindings_executed": source_bindings,
        "packet_source_function_bindings_executed": packet_bindings,
        "total_source_function_bindings_executed": source_bindings + packet_bindings,
        "distinct_registered_callbacks_executed": unique.len(),
        "transitive_helper_calls_counted": false,
        "packet_scope_inventory": evidence::packets::source_inventory()})
}

/// Adding a callable packet cannot erase a declared missing stage.
#[test]
fn callable_packets_do_not_close_pending_or_partial_obligations() {
    assert!(!is_bound(Binding::Pending));
    assert!(!is_bound(Binding::Packets(
        &["miirym"],
        &["missing complete-action and report control"]
    )));
    assert!(is_bound(Binding::Packets(&["miirym"], &[])));
}

/// Candidate links remain exact review locations even before their missing stage evidence is added.
#[test]
fn finite_obligation_candidates_name_existing_functions() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/deck/simulator");
    let mut unique = BTreeSet::new();
    let mut packets = BTreeSet::new();
    for obligation in obligations() {
        assert!(unique.insert(obligation.id));
        assert!(!obligation.requirement.is_empty() && !obligation.candidates.is_empty());
        if let Binding::Packets(ids, _) = obligation.binding {
            packets.extend(ids.iter().copied());
        }
        for (file, function) in obligation.candidates {
            let source =
                std::fs::read_to_string(root.join(file)).expect("candidate test file exists");
            assert!(
                source.contains(&format!("fn {function}(")),
                "{}: {file}::{function}",
                obligation.id
            );
        }
    }
    assert_eq!(packets, evidence::packets::ids());
}
