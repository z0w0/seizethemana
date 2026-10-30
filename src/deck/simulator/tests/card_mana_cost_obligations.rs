//! Published mana and payment branches with exact candidate review locations.

use super::{Binding, Obligation};

/// Mana production and cast-payment obligations are not new card-support claims.
pub(super) const OBLIGATIONS: &[Obligation] = &[
    Obligation {
        id: "cost.flat_reducers",
        requirement: "Live matching sources reduce one complete generic casting cost after increases, including optional costs and commander tax. Colored and colorless pips and nonmana costs remain required. Hidden, removed, or nonmatching sources grant no discount.",
        binding: Binding::Pending,
        candidates: &[
            (
                "game_mana/tests/cost_reduction_tests.rs",
                "source_discounts_stack_and_follow_live_battlefield_state",
            ),
            (
                "game_mana/tests/cost_reduction_cast_tests.rs",
                "captured_instant_reducer_changes_a_complete_cast_payment",
            ),
            (
                "game_mana/tests/cost_reduction_cast_tests.rs",
                "commander_recasts_reduce_the_tax_but_keep_the_colored_pip",
            ),
        ],
    },
    Obligation {
        id: "mana.colorless",
        requirement: "One visible source produces two colorless units, funds an actual generic cast, rejects a colored pip, and cannot tap twice.",
        binding: Binding::Supported("Sol Ring"),
        candidates: &[
            ("tests/card_mana_tests.rs", "assert_sol_ring"),
            ("tests/card_report_tests.rs", "assert_sol_ring_report"),
        ],
    },
    Obligation {
        id: "mana.fixed_color",
        requirement: "A mature source produces green, rejects white, respects sickness and hidden state, and reports actual spend.",
        binding: Binding::Supported("Llanowar Elves"),
        candidates: &[
            ("tests/card_mana_tests.rs", "assert_llanowar"),
            ("tests/card_report_tests.rs", "assert_llanowar_report"),
        ],
    },
    Obligation {
        id: "mana.any_color",
        requirement: "One any-color unit pays a colored pip but not true colorless; source state and source reuse remain enforced through reports.",
        binding: Binding::Supported("Birds of Paradise"),
        candidates: &[
            ("tests/card_mana_tests.rs", "assert_birds"),
            ("tests/card_report_tests.rs", "assert_birds_report"),
        ],
    },
    Obligation {
        id: "mana.identity",
        requirement: "Stored and partner identities, not printed commander colors, constrain native payments; missing and empty identities grant no generic or colorless mana.",
        binding: Binding::Supported("Arcane Signet"),
        candidates: &[
            ("tests/card_identity_tests.rs", "assert_identity_actions"),
            ("tests/card_identity_tests.rs", "assert_identity_report"),
        ],
    },
    Obligation {
        id: "mana.alternative_types",
        requirement: "A fixed alternative chooses one allowed type, cannot split one source across types, and preserves its choice under mana bonuses and reports.",
        binding: Binding::Packets(&["mana_alternatives_boundary"], &[]),
        candidates: &[(
            "tests/game_run_runtime_tests.rs",
            "mana_bonus_cannot_split_a_source_choice_into_different_colors",
        )],
    },
    Obligation {
        id: "mana.paid_modes",
        requirement: "Each mode pays its own generic, colored, tap, sacrifice, or life cost before producing its own output; no sibling mode supplies a free reward.",
        binding: Binding::Packets(
            &[
                "signet",
                "talisman",
                "mind_stone",
                "mana_paid_modes_boundary",
            ],
            &[],
        ),
        candidates: &[
            ("tests/card_cost_claim_tests.rs", "assert_signet_cost"),
            ("tests/card_cost_claim_tests.rs", "assert_talisman_modes"),
            ("tests/card_cost_claim_tests.rs", "assert_mind_stone_modes"),
        ],
    },
    Obligation {
        id: "mana.spend_restrictions",
        requirement: "Creature, artifact, legendary, instant/sorcery, and pip-only restrictions allow only eligible costs, including generic X and artifact-creature intersections.",
        binding: Binding::Packets(&["restrictions", "mana_restrictions_boundary"], &[]),
        candidates: &[
            (
                "tests/card_payment_claim_tests.rs",
                "assert_synthetic_restrictions",
            ),
            (
                "tests/turn_loop_mana_tests.rs",
                "mana_mode_keeps_its_own_spend_restriction",
            ),
        ],
    },
    Obligation {
        id: "mana.shared_tap",
        requirement: "Distinct legal modes share one source tap and cannot count that same permanent twice for production or another tap payment.",
        binding: Binding::Packets(&["signet", "talisman", "mind_stone", "shared_tap"], &[]),
        candidates: &[
            ("tests/card_cost_claim_tests.rs", "assert_mind_stone_modes"),
            (
                "tests/turn_loop_mana_tests.rs",
                "reusable_mana_mode_does_not_pay_a_separate_sacrifice_mode",
            ),
        ],
    },
    Obligation {
        id: "mana.untap_reuse",
        requirement: "A paid untap enables one later legal activation; sickness, missing costs, and insufficient output cannot create a free reusable source.",
        binding: Binding::Packets(
            &[
                "paid_untap",
                "untap_sickness_boundary",
                "granted_positive_loop_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/card_payment_claim_tests.rs",
                "assert_synthetic_paid_untap",
            ),
            (
                "tests/oracle_runtime_tests.rs",
                "proven_basalt_loop_funds_kinnans_colored_search_activation",
            ),
        ],
    },
    Obligation {
        id: "mana.unknown_cost_safety",
        requirement: "An unknown required activation payment blocks the entire mana mode while preserving the pool, source, physical zones, and report limitation.",
        binding: Binding::Packets(&["unknown_mana_cost_boundary"], &[]),
        candidates: &[
            (
                "tests/oracle_runtime_tests.rs",
                "compound_activation_requires_and_pays_all_costs",
            ),
            (
                "tests/turn_loop_mana_tests.rs",
                "mana_mode_must_pay_its_own_mana_cost_before_tapping",
            ),
        ],
    },
    Obligation {
        id: "mana.incomplete_output_safety",
        requirement: "Halfling retains both exact printed modes, but only the complete colorless mode lowers; even a legendary colored spell cannot use the unsupported mode and its report retains the blocking reason.",
        binding: Binding::Supported("Delighted Halfling"),
        candidates: &[
            ("tests/card_mana_tests.rs", "assert_halfling_colorless"),
            ("tests/card_report_tests.rs", "assert_halfling_report"),
        ],
    },
    Obligation {
        id: "mana.declared_opponents",
        requirement: "Declared colors require an active opponent with an actual land; empty colors and inactive seats cannot produce mana or fund a cast.",
        binding: Binding::Packets(&["declared_opponents_boundary"], &[]),
        candidates: &[
            ("tests/card_mana_tests.rs", "assert_fellwar"),
            ("tests/card_report_tests.rs", "assert_fellwar_report"),
        ],
    },
    Obligation {
        id: "mana.unknown_opponents",
        requirement: "Unknown-color production and its bonus give generic-only credit from turn two, preserve active-seat rules, and never pay colored or true colorless costs.",
        binding: Binding::Packets(&["unknown_opponents_boundary"], &[]),
        candidates: &[
            (
                "tests/plot_mana_boundary_tests.rs",
                "opponent_mana_native_and_leftover_bonuses_keep_color_and_turn_limits",
            ),
            (
                "tests/turn_loop_mana_tests.rs",
                "opponent_dependent_mana_is_generic_only_from_turn_two",
            ),
        ],
    },
    Obligation {
        id: "cost.hybrid",
        requirement: "Both printed hybrid choices can pay; a noncomponent color cannot, and the actual full-turn spend reaches the report.",
        binding: Binding::Packets(&["hybrid"], &[]),
        candidates: &[
            ("tests/card_payment_claim_tests.rs", "assert_hybrid_cast"),
            (
                "tests/card_payment_claim_tests.rs",
                "assert_hybrid_cast_report",
            ),
        ],
    },
    Obligation {
        id: "cost.twobrid",
        requirement: "One matching colored unit or two generic units pay the same twobrid symbol, without double spending or inventing a third payment option.",
        binding: Binding::Packets(&["twobrid"], &[]),
        candidates: &[
            (
                "tests/card_payment_claim_tests.rs",
                "assert_synthetic_twobrid",
            ),
            (
                "tests/payment_tests.rs",
                "twobrid_accepts_one_matching_color_or_two_generic",
            ),
        ],
    },
    Obligation {
        id: "cost.phyrexian",
        requirement: "Each Phyrexian pip pays the declared two life; wrong resources and the last-life boundary do not grant an unpaid reward, and reports separate life and mana.",
        binding: Binding::Packets(&["phyrexian", "phyrexian_last_life_boundary"], &[]),
        candidates: &[
            ("tests/card_payment_claim_tests.rs", "assert_phyrexian_cast"),
            (
                "tests/card_payment_claim_tests.rs",
                "assert_phyrexian_cast_report",
            ),
        ],
    },
    Obligation {
        id: "cost.creature_sacrifice",
        requirement: "A real creature or artifact creature pays once, including when tapped; artifact-only payment remains rejected and grants no draw or Treasure reward.",
        binding: Binding::Supported("Deadly Dispute"),
        candidates: &[
            (
                "tests/card_boundary_cost_tests.rs",
                "assert_dispute_cost_boundary",
            ),
            (
                "tests/card_boundary_cost_tests.rs",
                "assert_dispute_cost_boundary_report",
            ),
        ],
    },
    Obligation {
        id: "cost.discard_life",
        requirement: "Additional discards and life costs consume the actual resource before rewards; missing or combined insufficient resources preserve the attempted cast and pool.",
        binding: Binding::Packets(
            &[
                "discard_cost",
                "life_cycling",
                "combined_spell_cost_boundary",
            ],
            &[],
        ),
        candidates: &[
            ("tests/card_cost_claim_tests.rs", "assert_discard_cost"),
            (
                "tests/cast_zone_tests.rs",
                "missing_additional_cost_resources_do_not_spend_mana_or_cards",
            ),
            (
                "tests/game_fix_payment_tests.rs",
                "combined_life_costs_gate_on_the_total",
            ),
        ],
    },
    Obligation {
        id: "cost.convoke",
        requirement: "Each eligible creature supplies at most one generic or matching colored symbol, including hybrid choices, without creating true colorless mana.",
        binding: Binding::Packets(&["convoke"], &[]),
        candidates: &[
            (
                "tests/card_payment_claim_tests.rs",
                "assert_synthetic_convoke",
            ),
            (
                "tests/cast_payment_regression_tests.rs",
                "convoke_matches_hybrid_colors_without_creating_colorless_mana",
            ),
        ],
    },
    Obligation {
        id: "cost.improvise_affinity",
        requirement: "Improvise taps only untapped artifacts for generic costs; affinity counts matching artifacts without tapping; a shared artifact creature and banked Treasure cannot pay twice.",
        binding: Binding::Packets(
            &["improvise", "shared_tap", "affinity", "dispute_bank"],
            &[],
        ),
        candidates: &[
            (
                "tests/card_payment_claim_tests.rs",
                "assert_synthetic_improvise",
            ),
            (
                "tests/card_payment_claim_tests.rs",
                "assert_synthetic_shared_tap",
            ),
            ("tests/card_cost_claim_tests.rs", "assert_affinity_cost"),
        ],
    },
    Obligation {
        id: "cost.delve",
        requirement: "Delve exiles distinct physical graveyard cards for generic cost only, preserves the colored payment, and cannot reuse the exiled cards.",
        binding: Binding::Packets(&["delve"], &[]),
        candidates: &[
            (
                "tests/card_payment_claim_tests.rs",
                "assert_synthetic_delve",
            ),
            (
                "tests/cast_zone_tests.rs",
                "delve_pays_generic_cost_with_graveyard_cards_and_resolves",
            ),
        ],
    },
    Obligation {
        id: "cost.unsupported_symbols_reducers",
        requirement: "Snow, unknown mana symbols, and unmodeled static reducer scopes cannot grant a payable discount or waive a required cost; normal printed-cost routes stay distinct.",
        binding: Binding::Packets(&["unpaid_symbols", "reducer_safety"], &[]),
        candidates: &[
            (
                "tests/card_payment_claim_tests.rs",
                "assert_synthetic_unpaid_symbols",
            ),
            (
                "tests/card_cost_claim_tests.rs",
                "assert_unsupported_reducer",
            ),
        ],
    },
    Obligation {
        id: "cost.x_spells",
        requirement: "Choose one payable X before payment, charge repeated X symbols equally, permit zero, retain unrelated restricted mana, and use X zero on free casts.",
        binding: Binding::Packets(
            &[
                "paid_x",
                "cascade",
                "repeated_x_spell_boundary",
                "free_x_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/game_fix_board_tests.rs",
                "x_cost_preserves_unrelated_restricted_mana",
            ),
            (
                "tests/library_effect_tests.rs",
                "cascade_free_cast_sets_x_to_zero_and_keeps_the_pool",
            ),
        ],
    },
    Obligation {
        id: "cost.x_activations",
        requirement: "Charge-counter activations choose X before costs, support repeated symbols and zero, preserve unrelated mana, and leave unknown counter kinds inert.",
        binding: Binding::Packets(&["x_activation_boundary"], &[]),
        candidates: &[
            (
                "game_effects/tests/activation_tests.rs",
                "x_counter_activations_pay_one_chosen_value_for_every_x_symbol",
            ),
            (
                "game_effects/tests/activation_tests.rs",
                "unknown_x_counter_kinds_stay_inert",
            ),
        ],
    },
    Obligation {
        id: "cost.activation_sacrifices",
        requirement: "Source exclusion follows the printed word another; tapped creatures and tokens can pay, distinct source-plus-creature costs cannot reuse one identity, and failure preserves pending work and resources.",
        binding: Binding::Packets(
            &[
                "creature_return",
                "activation_targets",
                "activation_sacrifices_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "game_effects/tests/sacrifice_tests.rs",
                "missing_other_creature_preserves_every_payment_resource_and_rng",
            ),
            (
                "game_effects/tests/sacrifice_tests.rs",
                "source_sacrifice_requires_a_distinct_creature_for_both_source_flags",
            ),
        ],
    },
    Obligation {
        id: "cost.alternate_safety",
        requirement: "Unsupported alternate life and turn requirements cannot become free pitch spells; printed-cost casts remain separate and reports retain the exact blocked wording.",
        binding: Binding::SafetyPair("Force of Will", "Force of Negation"),
        candidates: &[
            (
                "tests/compiler_safety_action_tests.rs",
                "assert_force_of_will_payment",
            ),
            (
                "tests/compiler_safety_action_tests.rs",
                "assert_force_of_negation_payment",
            ),
        ],
    },
];
