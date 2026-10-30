//! Published land, library, search, draw, and replay branches with exact review candidates.

use super::{Binding, Obligation};

/// Physical zone movement remains separate from source presence and compiler recognition.
pub(super) const OBLIGATIONS: &[Obligation] = &[
    Obligation {
        id: "lands.entry_life",
        requirement: "Supported entry conditions and life payments affect the physical entering land once; insufficient life and unknown conditions do not grant a free untapped source.",
        binding: Binding::Packets(
            &[
                "three_life_mdfc",
                "tapped_mdfc",
                "shock_entry",
                "fast_entry",
                "slow_entry",
                "multiplayer_entry",
                "land_entry_safety_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/cast_zone_tests.rs",
                "shock_land_pays_life_to_enter_untapped",
            ),
            (
                "tests/turn_loop_mana_tests.rs",
                "duplicate_land_permanents_satisfy_each_others_gate",
            ),
        ],
    },
    Obligation {
        id: "lands.fetch",
        requirement: "A fetch pays life and source sacrifice once, accepts only its actual target types, keeps misses in the library, and preserves shuffle and entry rules through reports.",
        binding: Binding::Packets(&["fetch", "fabled_safety"], &[]),
        candidates: &[
            (
                "tests/cast_zone_tests.rs",
                "fetch_sacrifices_itself_and_searches_only_a_matching_land_type",
            ),
            (
                "tests/game_fix_board_tests.rs",
                "fetch_targets_come_from_oracle_text",
            ),
        ],
    },
    Obligation {
        id: "lands.extra_plays",
        requirement: "Exact extra-land counts stack across live sources and current-turn effects, including an allowance granted after the first land phase, without spending one allowance twice.",
        binding: Binding::Packets(&["azusa", "exploration", "explore", "stacked_lands"], &[]),
        candidates: &[
            (
                "tests/game_run_runtime_tests.rs",
                "additional_land_rules_keep_counts_instead_of_booleans",
            ),
            (
                "tests/turn_loop_land_tests.rs",
                "additional_land_granted_after_land_phase_is_available_later",
            ),
        ],
    },
    Obligation {
        id: "lands.landfall",
        requirement: "Only a real land entry triggers landfall, not a nonland entry or a library move that stays out of play; actual rewards and their denominator reach the report.",
        binding: Binding::Packets(&["landfall_draw", "landfall_library_move_boundary"], &[]),
        candidates: &[
            (
                "tests/oracle_runtime_tests.rs",
                "landfall_draws_only_when_a_land_enters",
            ),
            (
                "tests/game_mechanic_tests.rs",
                "landfall_engine_draws_on_land_drops",
            ),
        ],
    },
    Obligation {
        id: "lands.landfall_pool",
        requirement: "Landfall mana is usable only with a live resolution pool; it neither vanishes from a valid paid sequence nor appears after an unpooled event.",
        binding: Binding::Packets(&["cobra"], &[]),
        candidates: &[
            (
                "tests/game_mechanic_tests.rs",
                "per_cast_mana_engine_feeds_pool",
            ),
            (
                "tests/turn_loop_mana_tests.rs",
                "mana_ability_resolves_its_other_nonlibrary_effects",
            ),
        ],
    },
    Obligation {
        id: "draw.fixed",
        requirement: "A paid source draws three exact physical cards, rejects insufficient colored pips, preserves the remaining library, and reports three seen cards.",
        binding: Binding::Supported("Harmonize"),
        candidates: &[
            ("tests/card_runtime_tests.rs", "assert_draw_three"),
            ("tests/card_report_tests.rs", "assert_harmonize_report"),
        ],
    },
    Obligation {
        id: "draw.dredge",
        requirement: "Each actual draw chooses an available largest payable dredge amount, mills the full amount, returns the dredger instead of drawing, and rejects insufficient library size.",
        binding: Binding::Packets(&["dredge", "competing_dredgers_boundary"], &[]),
        candidates: &[
            (
                "tests/card_library_claim_tests.rs",
                "assert_loam_dredge_subset_report",
            ),
            (
                "tests/library_effect_tests.rs",
                "dredge_replaces_a_draw_only_when_the_full_mill_amount_is_available",
            ),
        ],
    },
    Obligation {
        id: "draw.scaled",
        requirement: "Board-scaled draws use the matching count, including zero and counts above eight; unknown scaling must not become a fixed free reward.",
        binding: Binding::Packets(&["scaled_draw", "scaled_draw_safety_boundary"], &[]),
        candidates: &[
            (
                "tests/game_run_runtime_tests.rs",
                "scaled_draws_above_eight_are_not_silently_truncated",
            ),
            (
                "tests/mechanic_tests.rs",
                "scaling_draw_engine_draws_by_board",
            ),
        ],
    },
    Obligation {
        id: "library.mill",
        requirement: "Move the exact top physical cards to the correct graveyard, fire only library-to-graveyard listeners, and keep player and opponent zones and access reports distinct.",
        binding: Binding::Packets(&["entry_mill", "dredge", "mill_origin_boundary"], &[]),
        candidates: &[
            (
                "tests/card_library_claim_tests.rs",
                "assert_emry_entry_mill_subset_report",
            ),
            (
                "tests/library_graveyard_tests.rs",
                "discarding_a_library_trigger_card_does_not_fire_its_mill_ability",
            ),
        ],
    },
    Obligation {
        id: "search.types_relations",
        requirement: "Each supported card-type union and exact/minimum/maximum mana-value relation selects only eligible front-face cards; unknown unions and repeated relations emit no partial search.",
        binding: Binding::Packets(&["search_types_boundary"], &[]),
        candidates: &[
            (
                "tests/search_filter_boundary_tests.rs",
                "paid_searches_match_only_the_named_front_face_spell_type",
            ),
            (
                "tests/search_filter_boundary_tests.rs",
                "paid_search_rejects_repeated_and_compound_mana_value_relations",
            ),
        ],
    },
    Obligation {
        id: "search.subtype",
        requirement: "A paid entry search accepts a nonbasic Forest subtype and rejects a Swamp, without spending a land play; changed destination wording changes movement.",
        binding: Binding::Supported("Wood Elves"),
        candidates: &[
            ("tests/card_search_tests.rs", "assert_entry_land_search"),
            ("tests/card_report_tests.rs", "assert_entry_land_report"),
        ],
    },
    Obligation {
        id: "search.basic",
        requirement: "A paid optional entry search accepts a basic Swamp tapped and rejects a nonbasic Forest, without counting the result as a land play.",
        binding: Binding::Supported("Farhaven Elf"),
        candidates: &[
            ("tests/card_search_tests.rs", "assert_entry_land_search"),
            ("tests/card_report_tests.rs", "assert_entry_land_report"),
        ],
    },
    Obligation {
        id: "search.colors",
        requirement: "Color and colorless filters use raw stored single-face colors, never merged colors or mana-cost inference; unknown multiface colors remain in the library and reports show only real hits.",
        binding: Binding::Packets(&["search_colors_boundary"], &[]),
        candidates: &[
            (
                "tests/search_filter_boundary_tests.rs",
                "color_search_uses_known_stored_colors_and_rejects_unknown_colorless_faces",
            ),
            (
                "tests/search_filter_boundary_tests.rs",
                "green_search_full_turn_reports_only_known_front_color_hits",
            ),
        ],
    },
    Obligation {
        id: "search.shuffle_top",
        requirement: "Select a non-top artifact, shuffle only the remaining identities, then place the selected identity on top; misses still obey shuffle and exact RNG continuation.",
        binding: Binding::Supported("Enlightened Tutor"),
        candidates: &[
            ("tests/card_search_tests.rs", "assert_top_tutor"),
            ("tests/card_report_tests.rs", "assert_top_tutor_report"),
        ],
    },
    Obligation {
        id: "search.graveyard_destination",
        requirement: "A paid creature entry moves one exact library card to the graveyard without a draw or hand access; an empty library grants no result or failed-draw loss.",
        binding: Binding::Supported("Gravebreaker Lamia"),
        candidates: &[
            ("tests/card_runtime_tests.rs", "assert_graveyard_search"),
            ("tests/card_report_tests.rs", "assert_lamia_report"),
        ],
    },
    Obligation {
        id: "search.split_safety",
        requirement: "Cultivate and Reach execute their complete multi-result plans. Unsupported name-restricted mutations cannot move cards, shuffle, grant linked rewards, or report phantom ramp through a paid ordinary turn.",
        binding: Binding::SafetyPair("Cultivate", "Kodama's Reach"),
        candidates: &[
            (
                "tests/card_boundary_search_tests.rs",
                "split_search_turns_report_spend_without_phantom_ramp",
            ),
            (
                "tests/card_boundary_search_tests.rs",
                "split_search_riders_cannot_pay_out_after_unsupported_search",
            ),
        ],
    },
    Obligation {
        id: "search.source_power_safety",
        requirement: "Sisay's legendary and source-power restrictions cannot become a broader search, including with all five colors and enough mana; changed supported wording proves the search path.",
        binding: Binding::Safety("Sisay, Weatherlight Captain"),
        candidates: &[
            (
                "tests/card_boundary_search_tests.rs",
                "sisay_full_turn_reports_keep_unsupported_library_and_mana",
            ),
            (
                "tests/card_boundary_search_tests.rs",
                "sisay_changed_text_runs_a_paid_supported_search",
            ),
        ],
    },
    Obligation {
        id: "cast.cycling_landcycling",
        requirement: "Cycling pays and discards the physical card before a draw; landcycling finds only the named land type, with resource-preserving failures and relevant access reports.",
        binding: Binding::Packets(&["cycling", "landcycling", "life_cycling"], &[]),
        candidates: &[(
            "tests/library_effect_tests.rs",
            "cycling_discards_and_draws_while_landcycling_finds_the_named_land_type",
        )],
    },
    Obligation {
        id: "cast.cascade",
        requirement: "Cascade follows library order, free-casts the first lower-value nonland hit, keeps miss order at the bottom, uses X zero, never chains, and stops parent resolution at terminal or incomplete child boundaries.",
        binding: Binding::Packets(
            &["cascade", "free_x_boundary", "cascade_child_boundary"],
            &[],
        ),
        candidates: &[
            (
                "tests/card_library_claim_tests.rs",
                "assert_wanderer_one_cascade_subset_report",
            ),
            (
                "tests/cascade_boundary_tests.rs",
                "cascade_failed_draw_finishes_linked_child_effects_only",
            ),
        ],
    },
    Obligation {
        id: "draw.loot_wheel",
        requirement: "Equal-count loot and fixed-seven wheels follow the declared selection/order policy, discard the current physical hand once, and report draw and graveyard changes without duplicating the cast card.",
        binding: Binding::Packets(&["looting", "relic", "windfall"], &[]),
        candidates: &[
            (
                "tests/card_library_claim_tests.rs",
                "assert_faithless_looting_hand_report",
            ),
            (
                "tests/card_library_claim_tests.rs",
                "assert_windfall_fixed_seven_report",
            ),
        ],
    },
    Obligation {
        id: "draw.unequal_loot",
        requirement: "Draw three exact cards before discarding the oldest hand card; wrong-color attempts preserve resources and changed two-draw wording changes the result and report.",
        binding: Binding::Supported("Sift"),
        candidates: &[
            ("tests/card_runtime_tests.rs", "assert_sift"),
            ("tests/card_report_tests.rs", "assert_sift_report"),
        ],
    },
    Obligation {
        id: "selection.library_first",
        requirement: "The declared top-first pilot chooses the known first eligible physical card among several candidates and reports library-top placement rather than hand arrival.",
        binding: Binding::Supported("Vampiric Tutor"),
        candidates: &[
            ("tests/card_search_tests.rs", "assert_top_tutor"),
            ("tests/card_report_tests.rs", "assert_top_tutor_report"),
        ],
    },
    Obligation {
        id: "selection.graveyard_first",
        requirement: "With several legal graveyard candidates, the declared pilot chooses the stated first eligible identity, not a manufactured target, and the actual return reaches reports.",
        binding: Binding::Packets(
            &[
                "creature_return",
                "any_return",
                "witness_land",
                "graveyard_selection_boundary",
            ],
            &[],
        ),
        candidates: &[("tests/card_search_tests.rs", "assert_eternal_witness")],
    },
    Obligation {
        id: "graveyard.return_any",
        requirement: "A paid entry returns the actual sole eligible artifact from the player's graveyard to hand; an empty graveyard grants no card and changed draw wording leaves the target untouched.",
        binding: Binding::Supported("Eternal Witness"),
        candidates: &[
            ("tests/card_search_tests.rs", "assert_eternal_witness"),
            ("tests/card_report_tests.rs", "assert_witness_report"),
        ],
    },
    Obligation {
        id: "graveyard.return_creature",
        requirement: "A targeted creature return selects the actual eligible graveyard identity, applies normal printed entry state, rejects illegal targets, and records battlefield access without an extra cast.",
        binding: Binding::Packets(
            &[
                "creature_return",
                "activation_targets",
                "upkeep_target_generation_boundary",
                "activation_generation_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/resolution_tests.rs",
                "graveyard_return_preserves_printed_tapped_entry",
            ),
            (
                "oracle_lower/tests/oracle_lower_tests.rs",
                "graveyard_returns_reject_unmodeled_filters_and_entry_rules",
            ),
        ],
    },
    Obligation {
        id: "graveyard.filtered_return_safety",
        requirement: "Unmodeled return filters, counts, and forced-tapped instructions leave the entire reward inert through production attempts and an explicit unsupported report record.",
        binding: Binding::Packets(&["filtered_return_boundary"], &[]),
        candidates: &[(
            "oracle_lower/tests/oracle_lower_tests.rs",
            "graveyard_returns_reject_unmodeled_filters_and_entry_rules",
        )],
    },
    Obligation {
        id: "graveyard.flashback_escape",
        requirement: "Paid flashback and the supported three-card Escape subset use distinct destination rules, consume exact physical fodder and additional resources, and cannot replay the same resource twice.",
        binding: Binding::Packets(
            &[
                "own_flashback",
                "granted_flashback",
                "granted_escape",
                "own_escape_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/game_fix_payment_tests.rs",
                "own_flashback_casts_from_the_graveyard",
            ),
            (
                "tests/escape_payment_boundary_tests.rs",
                "modeled_escape_pays_its_complete_mana_and_three_card_cost",
            ),
            (
                "tests/cast_settlement_tests.rs",
                "escaped_instant_returns_to_the_graveyard",
            ),
        ],
    },
    Obligation {
        id: "graveyard.other_escape_safety",
        requirement: "Ox's printed eight-card route stays inert even with enough fodder and mana; its separate normal cast pays the printed cost without escaped counters or unsupported entry rewards.",
        binding: Binding::Safety("Ox of Agonas"),
        candidates: &[
            (
                "tests/card_boundary_cost_tests.rs",
                "assert_ox_cost_boundary",
            ),
            (
                "tests/card_boundary_cost_tests.rs",
                "assert_ox_cost_boundary_report",
            ),
        ],
    },
    Obligation {
        id: "access.first_visits",
        requirement: "Executed first hand, battlefield, and graveyard visits keep the earliest physical-card observation, including terminal phases, rather than inventing full zone histories or simultaneous combo presence.",
        binding: Binding::Packets(
            &[
                "creature_return",
                "any_return",
                "witness_land",
                "looting",
                "entry_mill",
                "terminal_access_boundary",
            ],
            &[],
        ),
        candidates: &[
            (
                "tests/game_run_runtime_tests.rs",
                "role_sightings_keep_the_earliest_recorded_event_turn",
            ),
            (
                "tests/preloss_event_tests.rs",
                "terminal_draw_and_mill_keep_land_access_without_an_end_turn",
            ),
        ],
    },
];
