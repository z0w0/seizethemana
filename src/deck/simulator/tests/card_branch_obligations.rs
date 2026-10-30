//! Exact branch obligations backed by callable action and executed-report assertions.

/// Limits and observations for one verified branch, not every rule on its source card.
#[derive(Clone, Copy, serde::Serialize)]
pub(super) struct CoveredBranch {
    payment: &'static str,
    eligibility: &'static str,
    effect: &'static str,
    negative_controls: &'static str,
    report_metrics: &'static [&'static str],
    approximation: &'static str,
    action_assertion: &'static str,
    report_assertion: &'static str,
}

impl CoveredBranch {
    /// Keep the branch's actual model limit beside every source-line claim.
    pub(super) fn scope_limit(&self) -> &'static str {
        self.approximation
    }
}

/// Resolve every registered source to reviewed obligations instead of generic cost/effect labels.
pub(super) fn for_source(name: &str) -> CoveredBranch {
    MANA.iter()
        .chain(SEARCH)
        .chain(ENGINES)
        .chain(COSTS)
        .find(|(source, _)| *source == name)
        .map(|(_, branch)| *branch)
        .expect("each callable mechanic claim has explicit branch obligations")
}

/// Native payments distinguish output types, tap eligibility, and repeated source use.
const MANA: &[(&str, CoveredBranch)] = &[
    (
        "Arcane Signet",
        CoveredBranch {
            payment: "Tap the visible source once and use one identity-constrained colored unit for an actual cast.",
            eligibility: "Stored UR or partner U/R identity permits blue, red, and generic costs even though the commander has a colorless printed cost.",
            effect: "Produce one eligible native mana unit, or none when identity is empty or no commander exists.",
            negative_controls: "White, black, green, and true colorless targets fail. Empty identity cannot fund generic costs. Renamed sources retain the limit; changed fixed-green wording removes it.",
            report_metrics: &[
                "mana.avg_available_by_turn",
                "mana.avg_cast_spent_by_turn",
                "mana.avg_unused_by_turn",
                "card_support.source_hash",
            ],
            approximation: "The source is already in play. Native payment and complete-turn reports are verified; other production routes are tested separately in identity_mana_tests.",
            action_assertion: "card_identity_tests::assert_identity_actions",
            report_assertion: "card_identity_tests::assert_identity_report",
        },
    ),
    (
        "Command Tower",
        CoveredBranch {
            payment: "Tap the land source once and use one identity-constrained colored unit for an actual cast.",
            eligibility: "Stored UR or partner U/R identity permits blue, red, and generic costs even though the commander has a colorless printed cost.",
            effect: "Produce one eligible native mana unit, or none when identity is empty or no commander exists.",
            negative_controls: "White, black, green, and true colorless targets fail. Empty identity cannot fund generic costs. Renamed sources retain the limit; changed fixed-green wording removes it.",
            report_metrics: &[
                "mana.avg_available_by_turn",
                "mana.avg_cast_spent_by_turn",
                "mana.avg_unused_by_turn",
                "card_support.source_hash",
            ],
            approximation: "The land starts in play. This verifies native identity payments, not its land-drop timing or every activation route.",
            action_assertion: "card_identity_tests::assert_identity_actions",
            report_assertion: "card_identity_tests::assert_identity_report",
        },
    ),
    (
        "Sol Ring",
        CoveredBranch {
            payment: "Tap the ready source once; two colorless mana fund an actual {2} cast.",
            eligibility: "The physical source must be visible and untapped.",
            effect: "Produce exactly two colorless mana; the committed cast consumes both.",
            negative_controls: "A green pip, a tapped or face-down source, and a second tap fail. Changed wording produces red instead.",
            report_metrics: &["mana.avg_cast_spent_by_turn", "mana.avg_unused_by_turn"],
            approximation: "Only this main-phase payment branch is verified.",
            action_assertion: "card_mana_tests::assert_sol_ring",
            report_assertion: "card_report_tests::assert_sol_ring_report",
        },
    ),
    (
        "Llanowar Elves",
        CoveredBranch {
            payment: "Tap one mature creature source to fund an actual {G} cast.",
            eligibility: "A tapped, summoning-sick, or face-down source cannot pay its tap cost.",
            effect: "Produce and spend exactly one green mana.",
            negative_controls: "White payment, sickness, hidden source, and repeat use fail. Changed wording produces red.",
            report_metrics: &["mana.avg_cast_spent_by_turn", "mana.avg_unused_by_turn"],
            approximation: "Other creature abilities and combat are not covered.",
            action_assertion: "card_mana_tests::assert_llanowar",
            report_assertion: "card_report_tests::assert_llanowar_report",
        },
    ),
    (
        "Birds of Paradise",
        CoveredBranch {
            payment: "Tap one mature creature source to fund an actual {U} cast.",
            eligibility: "Tap eligibility and source reuse match the native creature payment path.",
            effect: "One colored choice can pay blue but not a true colorless pip.",
            negative_controls: "Explicit {C}, tapped, sick, hidden, and repeat-use controls fail. Changed wording produces red.",
            report_metrics: &["mana.avg_cast_spent_by_turn", "mana.avg_unused_by_turn"],
            approximation: "The scenario tests blue and colorless boundaries, not every color or flying.",
            action_assertion: "card_mana_tests::assert_birds",
            report_assertion: "card_report_tests::assert_birds_report",
        },
    ),
    (
        "Delighted Halfling",
        CoveredBranch {
            payment: "The complete colorless mode retains exactly one source-tap cost.",
            eligibility: "Both printed modes remain in the AST. Only the complete colorless mode lowers.",
            effect: "One colorless mana funds {C}; the unrecognized second-mode requirements emit no colored output.",
            negative_controls: "Even a legendary {G} spell cannot use the second mode. Tapped, sick, hidden, and repeat-use controls fail.",
            report_metrics: &[
                "mana.avg_cast_spent_by_turn",
                "mana.avg_unused_by_turn",
                "card_support.abilities.status",
                "card_support.abilities.reason",
            ],
            approximation: "The legendary and uncounterability mode is unsupported, not a second supported branch.",
            action_assertion: "card_mana_tests::assert_halfling_colorless",
            report_assertion: "card_report_tests::assert_halfling_report",
        },
    ),
    (
        "Fellwar Stone",
        CoveredBranch {
            payment: "Tap the source once; one declared blue capability funds a {U} cast.",
            eligibility: "A declared active seat must control a land with an eligible color.",
            effect: "Use declared blue capability without granting green or colorless payment.",
            negative_controls: "An empty declared color set and an inactive seat emit no mana. Changed wording becomes fixed red production.",
            report_metrics: &["mana.avg_cast_spent_by_turn", "mana.avg_unused_by_turn"],
            approximation: "Opponent land colors and activity are declared inputs; unknown-color credit is a separate branch.",
            action_assertion: "card_mana_tests::assert_fellwar",
            report_assertion: "card_report_tests::assert_fellwar_report",
        },
    ),
    (
        "Dark Ritual",
        CoveredBranch {
            payment: "Pay one actual black mana before the spell resolves.",
            eligibility: "A green-only pool cannot cast the pinned source.",
            effect: "Add exactly three black mana, put the spell in the graveyard, and fund a second cast.",
            negative_controls: "Wrong-color rejection preserves the card and pool. Renamed wording behaves equally; changed text produces two red.",
            report_metrics: &["mana.avg_cast_spent_by_turn", "graveyard.avg_size_by_turn"],
            approximation: "Only the declared main-phase cast sequence is covered.",
            action_assertion: "card_runtime_tests::assert_ritual",
            report_assertion: "card_report_tests::assert_ritual_report",
        },
    ),
];

/// Searches preserve only the filters, physical movements, and choices actually asserted.
const SEARCH: &[(&str, CoveredBranch)] = &[
    (
        "Nature's Lore",
        CoveredBranch {
            payment: "Pay the complete two-mana cast cost.",
            eligibility: "A Forest subtype matches; a Mountain does not.",
            effect: "Move the sole matching physical land to the battlefield without a land play.",
            negative_controls: "A subtype miss stays in the library. Changed artifact-to-hand wording changes the selector and destination.",
            report_metrics: &[
                "mana.avg_cast_spent_by_turn",
                "graveyard.avg_size_by_turn",
                "GameLog.land_drops",
                "GameLog.library_size",
            ],
            approximation: "The pilot has one matching target; arbitrary choices are not certified.",
            action_assertion: "card_runtime_tests::assert_land_search",
            report_assertion: "card_report_tests::assert_land_search_report",
        },
    ),
    (
        "Three Visits",
        CoveredBranch {
            payment: "Pay the complete two-mana cast cost.",
            eligibility: "A Forest subtype matches; a Mountain does not.",
            effect: "Move the sole matching physical land to the battlefield without a land play.",
            negative_controls: "A subtype miss stays in the library. Changed artifact-to-hand wording changes the selector and destination.",
            report_metrics: &[
                "mana.avg_cast_spent_by_turn",
                "graveyard.avg_size_by_turn",
                "GameLog.land_drops",
                "GameLog.library_size",
            ],
            approximation: "The pilot has one matching target; arbitrary choices are not certified.",
            action_assertion: "card_runtime_tests::assert_land_search",
            report_assertion: "card_report_tests::assert_land_search_report",
        },
    ),
    (
        "Farseek",
        CoveredBranch {
            payment: "Pay the complete two-mana cast cost.",
            eligibility: "A Mountain matches; a Forest-only land does not.",
            effect: "Move the matching physical land onto the battlefield tapped without a land play.",
            negative_controls: "A Forest-only miss stays in the library. Changed artifact-to-hand wording changes the selector and destination.",
            report_metrics: &[
                "mana.avg_cast_spent_by_turn",
                "graveyard.avg_size_by_turn",
                "GameLog.land_drops",
                "GameLog.library_size",
            ],
            approximation: "Other listed subtypes and dual-type lands need distinct branch evidence.",
            action_assertion: "card_runtime_tests::assert_land_search",
            report_assertion: "card_report_tests::assert_land_search_report",
        },
    ),
    (
        "Enlightened Tutor",
        CoveredBranch {
            payment: "Pay one white mana; selection is not hand access.",
            eligibility: "Select the sole artifact below four physical sorcery misses.",
            effect: "Shuffle the four remaining cards, then place that artifact on top; preserve every zone identity and RNG continuation.",
            negative_controls: "Five nonmatching cards still shuffle. Renamed text is equal; changed hand-destination wording changes movement.",
            report_metrics: &[
                "velocity.avg_cards_seen",
                "draw.avg_life_paid",
                "mana.avg_cast_spent_by_turn",
                "GameLog.library_size",
            ],
            approximation: "This proves the artifact branch, not enchantment selection or every legal pilot choice.",
            action_assertion: "card_search_tests::assert_top_tutor",
            report_assertion: "card_report_tests::assert_top_tutor_report",
        },
    ),
    (
        "Vampiric Tutor",
        CoveredBranch {
            payment: "Pay one black mana. The two-life drawback is life loss, not life payment.",
            eligibility: "The deterministic top-first pilot chooses one of five physical legal cards.",
            effect: "Shuffle the other four cards, place the chosen identity on top, and lose exactly two life.",
            negative_controls: "An empty library still loses two life without advancing the shuffle stream. Renamed and changed-destination controls are separate.",
            report_metrics: &[
                "velocity.avg_cards_seen",
                "draw.avg_life_paid",
                "draw.avg_life_gained",
                "mana.avg_cast_spent_by_turn",
            ],
            approximation: "Other legal choices and life-loss interactions are not certified.",
            action_assertion: "card_search_tests::assert_top_tutor",
            report_assertion: "card_report_tests::assert_top_tutor_report",
        },
    ),
    (
        "Trophy Mage",
        CoveredBranch {
            payment: "Pay the actual three-mana creature cast before its entry trigger.",
            eligibility: "Only the tested artifact with mana value exactly three matches.",
            effect: "Move the matching physical artifact from library to hand after entry.",
            negative_controls: "A two-value artifact and a three-value creature miss. Changed text accepts a two-value artifact.",
            report_metrics: &["velocity.avg_cards_seen", "mana.avg_cast_spent_by_turn"],
            approximation: "The deterministic pilot accepts the optional search; other entry interactions are not covered.",
            action_assertion: "card_search_tests::assert_trophy_mage",
            report_assertion: "card_report_tests::assert_trophy_report",
        },
    ),
    (
        "Wood Elves",
        CoveredBranch {
            payment: "Pay three mana including green for the physical creature cast.",
            eligibility: "A nonbasic Forest matches; a Swamp misses.",
            effect: "The actual entry trigger puts the Forest onto the battlefield without forcing tapped entry or a land play.",
            negative_controls: "A non-Forest stays in the library. Renamed wording is equal; changed hand wording does not put the result into play.",
            report_metrics: &[
                "mana.avg_cast_spent_by_turn",
                "velocity.avg_cards_seen",
                "GameLog.land_drops",
                "GameLog.library_size",
            ],
            approximation: "Only a sole legal Forest and its miss are covered.",
            action_assertion: "card_search_tests::assert_entry_land_search",
            report_assertion: "card_report_tests::assert_entry_land_report",
        },
    ),
    (
        "Farhaven Elf",
        CoveredBranch {
            payment: "Pay three mana including green for the physical creature cast.",
            eligibility: "A basic Swamp matches; a nonbasic Forest misses.",
            effect: "The accepted optional entry search puts the basic land onto the battlefield tapped without a land play.",
            negative_controls: "A nonbasic result stays in the library. Renamed wording is equal; changed hand wording changes the destination.",
            report_metrics: &[
                "mana.avg_cast_spent_by_turn",
                "velocity.avg_cards_seen",
                "GameLog.land_drops",
                "GameLog.library_size",
            ],
            approximation: "The pilot accepts the optional search; declining it and multiple-choice policy are not certified.",
            action_assertion: "card_search_tests::assert_entry_land_search",
            report_assertion: "card_report_tests::assert_entry_land_report",
        },
    ),
    (
        "Gravebreaker Lamia",
        CoveredBranch {
            payment: "Pay five mana for the physical creature cast before its entry trigger.",
            eligibility: "One available library card can move; an empty library supplies no target.",
            effect: "Move the chosen identity to the graveyard without a draw, hand access, or failed draw.",
            negative_controls: "The empty-library control has no reward or loss. Renamed wording is equal; changed hand wording changes the destination.",
            report_metrics: &[
                "graveyard.avg_size_by_turn",
                "velocity.avg_cards_seen",
                "mana.avg_cast_spent_by_turn",
            ],
            approximation: "The graveyard cost reducer, lifelink, and arbitrary target choices are not certified by this entry branch.",
            action_assertion: "card_runtime_tests::assert_graveyard_search",
            report_assertion: "card_report_tests::assert_lamia_report",
        },
    ),
];

/// Draw and opponent engines retain exact recipients and their finite model boundaries.
const ENGINES: &[(&str, CoveredBranch)] = &[
    (
        "Harmonize",
        CoveredBranch {
            payment: "Pay four mana including two green pips.",
            eligibility: "One green pip plus generic mana is insufficient.",
            effect: "Draw the three top physical cards, leaving the fourth in the library.",
            negative_controls: "Wrong-pip rejection preserves zones and mana. Renamed wording is equal; changed text draws only one.",
            report_metrics: &[
                "velocity.avg_cards_seen",
                "mana.avg_cast_spent_by_turn",
                "graveyard.avg_size_by_turn",
                "GameLog.library_size",
            ],
            approximation: "This is the nonempty-library main-phase branch, not every draw replacement or terminal boundary.",
            action_assertion: "card_runtime_tests::assert_draw_three",
            report_assertion: "card_report_tests::assert_harmonize_report",
        },
    ),
    (
        "Beast Whisperer",
        CoveredBranch {
            payment: "Cast a real zero-mana test creature while the source is on the battlefield.",
            eligibility: "A creature cast matches; a sorcery cast does not.",
            effect: "The matching cast draws exactly one physical library card.",
            negative_controls: "A noncreature cast does not draw. Renamed wording is equal; an artifact-only text change rejects the creature.",
            report_metrics: &["velocity.avg_cards_seen"],
            approximation: "Other cast replacements and combined trigger chains are not certified.",
            action_assertion: "card_engine_tests::assert_beast_whisperer",
            report_assertion: "card_report_tests::assert_beast_report",
        },
    ),
    (
        "Rhystic Study",
        CoveredBranch {
            payment: "The scheduled opponent pays for its cast before an optional one-mana tax.",
            eligibility: "One active finite seat supplies one cast and one remaining-source ledger.",
            effect: "Only an unpaid tax draws one physical player card.",
            negative_controls: "A paid tax gives no draw. Renamed wording is equal; changed unconditional two-card wording removes tax payments.",
            report_metrics: &[
                "opponent_model.observed_activity.totals",
                "velocity.avg_cards_seen",
            ],
            approximation: "The finite schedule and optional choices are declared synthetic inputs, not multiplayer predictions.",
            action_assertion: "card_engine_tests::assert_rhystic",
            report_assertion: "card_report_tests::assert_opponent_engine_report",
        },
    ),
    (
        "Smothering Tithe",
        CoveredBranch {
            payment: "The scheduled opponent shares sources between a two-mana draw tax and its reserved next spell.",
            eligibility: "One active finite seat supplies one draw event.",
            effect: "An unpaid tax banks one Treasure credit; that credit funds an actual one-mana follow-up cast.",
            negative_controls: "A paid tax gives no Treasure or follow-up cast. Renamed wording is equal; changed life-gain wording grants no Treasure.",
            report_metrics: &[
                "opponent_model.observed_activity.totals",
                "mana.avg_cast_spent_by_turn",
                "velocity.avg_cards_seen",
            ],
            approximation: "The opponent schedule is synthetic. Treasure credits do not certify physical artifact token interactions.",
            action_assertion: "card_engine_tests::assert_tithe",
            report_assertion: "card_report_tests::assert_opponent_engine_report",
        },
    ),
];

/// Cost and recursion cases distinguish the actual physical route from untested alternatives.
const COSTS: &[(&str, CoveredBranch)] = &[
    (
        "Village Rites",
        CoveredBranch {
            payment: "Pay one black mana and sacrifice the physical creature before drawing.",
            eligibility: "No eligible creature prevents the cast.",
            effect: "Put the victim and spell in the graveyard and draw two exact physical cards.",
            negative_controls: "Missing-creature rejection preserves the hand, library, and pool. Renamed wording is equal; changed text draws one.",
            report_metrics: &[
                "velocity.avg_cards_seen",
                "graveyard.avg_size_by_turn",
                "mana.avg_cast_spent_by_turn",
            ],
            approximation: "The sole legal creature is selected; off-turn timing and death-trigger chains are not certified.",
            action_assertion: "card_runtime_tests::assert_sacrifice_draw",
            report_assertion: "card_report_tests::assert_sacrifice_report",
        },
    ),
    (
        "Deadly Dispute",
        CoveredBranch {
            payment: "Pay two mana including black and use the creature sacrifice route.",
            eligibility: "No eligible creature prevents this implemented route.",
            effect: "Draw two physical cards and bank one Treasure credit; the credit funds a drawn one-mana spell.",
            negative_controls: "Artifact-only payment fails; an artifact creature is consumed once, tapped or untapped. Banked rewards survive failed mana payments, cannot pay artifact sacrifice, and cannot count for affinity or improvise. Changed text draws one.",
            report_metrics: &[
                "velocity.avg_cards_seen",
                "graveyard.avg_size_by_turn",
                "mana.avg_cast_spent_by_turn",
            ],
            approximation: "Artifact-only sacrifice is unsupported. The Treasure bank does not preserve artifact token interactions.",
            action_assertion: "card_runtime_tests::assert_dispute_actions",
            report_assertion: "card_report_tests::assert_dispute_report",
        },
    ),
    (
        "Eternal Witness",
        CoveredBranch {
            payment: "Pay three mana for the physical creature cast before its entry trigger.",
            eligibility: "The sole physical artifact in the player's graveyard is an eligible any-card target.",
            effect: "Return that identity to hand; the source remains on the battlefield.",
            negative_controls: "An empty graveyard grants no card. Renamed wording is equal; changed draw wording leaves the graveyard target untouched.",
            report_metrics: &["graveyard.avg_size_by_turn", "mana.avg_cast_spent_by_turn"],
            approximation: "The pilot accepts one unambiguous target. Other types, choices, and optional declines need distinct evidence.",
            action_assertion: "card_search_tests::assert_eternal_witness",
            report_assertion: "card_report_tests::assert_witness_report",
        },
    ),
    (
        "Sift",
        CoveredBranch {
            payment: "Pay four mana including one blue pip.",
            eligibility: "A green-only colored source cannot supply the blue requirement.",
            effect: "Draw three physical cards, then discard the old hand card; preserve the remaining library and RNG stream.",
            negative_controls: "Wrong-color rejection preserves zones and mana. Renamed wording is equal; changed two-draw wording draws only two.",
            report_metrics: &[
                "velocity.avg_cards_seen",
                "graveyard.avg_size_by_turn",
                "mana.avg_cast_spent_by_turn",
            ],
            approximation: "Discard ranks visible cards and keeps older tied cards; other choices and replacements are not certified.",
            action_assertion: "card_runtime_tests::assert_sift",
            report_assertion: "card_report_tests::assert_sift_report",
        },
    ),
];
