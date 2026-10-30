//! Payment packet registrations retain field-only and synthetic evidence boundaries.

use super::{Callback, Packet};
use crate::deck::simulator::{card_cost_claim_tests as costs, card_payment_claim_tests as payment};
use crate::deck::simulator::{card_report_tests as report, card_runtime_tests as runtime};

/// Source casts and payment policies certify only these executed subsets.
pub(super) const PACKETS: &[Packet] = &[
    Packet {
        id: "dispute_bank",
        source: Some("Deadly Dispute"),
        clauses: &["As an additional cost", "Draw two cards"],
        disposition: "scoped_approximation",
        scope: "The existing physical-sacrifice and Treasure-bank packet proves one committed payment, failed-payment preservation, and exclusion from artifact totals, affinity, improvise, and artifact-only costs.",
        approximation: "Treasure payment bank",
        limit: "This reuses the existing callbacks. It adds no distinct function, source identity, or physical Treasure artifact.",
        callbacks: row_callbacks![
            runtime::assert_dispute_actions,
            report::assert_dispute_report
        ],
    },
    Packet {
        id: "signet",
        source: Some("Azorius Signet"),
        clauses: &["{1}, {T}:"],
        disposition: "scoped_supported",
        scope: "Pay one external mana before producing white and blue; reject missing payment and a second tap.",
        approximation: "none",
        limit: "Main-phase native mana collection only.",
        callbacks: row_callbacks![costs::assert_signet_cost, costs::assert_signet_cost_report],
    },
    Packet {
        id: "talisman",
        source: Some("Talisman of Dominance"),
        clauses: &["{T}: Add {C}", "{T}: Add {U} or {B}"],
        disposition: "scoped_supported",
        scope: "Printed colorless and blue/black modes share one tap; the colored mode deals self-damage, not a life payment.",
        approximation: "native mode-selection policy",
        limit: "The ordinary pilot chooses the colored mode; it does not optimize a true-colorless target.",
        callbacks: row_callbacks![
            costs::assert_talisman_modes,
            costs::assert_talisman_modes_report
        ],
    },
    Packet {
        id: "mind_stone",
        source: Some("Mind Stone"),
        clauses: &["{T}: Add {C}", "Sacrifice this artifact"],
        disposition: "scoped_supported",
        scope: "A funded untapped draw action sacrifices the real source. Ordinary-turn reports prove shared-tap exclusion after mana collection.",
        approximation: "native mana-first policy",
        limit: "The positive draw is an action assertion. The report does not prove that the pilot reserves the tap for drawing.",
        callbacks: row_callbacks![
            costs::assert_mind_stone_modes,
            costs::assert_mind_stone_modes_report
        ],
    },
    Packet {
        id: "discard_cost",
        source: Some("Thrill of Possibility"),
        clauses: &["As an additional cost", "Draw two cards"],
        disposition: "scoped_supported",
        scope: "Discard an existing physical hand card before two draws; missing fodder preserves the cast and pool.",
        approximation: "oldest-card payment policy",
        limit: "No general additional-life-cost or combined-cost certificate.",
        callbacks: row_callbacks![
            costs::assert_discard_cost,
            costs::assert_discard_cost_report
        ],
    },
    Packet {
        id: "affinity",
        source: Some("Emry, Lurker of the Loch"),
        clauses: &["This spell costs"],
        disposition: "scoped_supported",
        scope: "Real artifacts reduce generic cast cost without tapping; blue remains required.",
        approximation: "none",
        limit: "W source. This does not certify Emry's artifact replay ability.",
        callbacks: row_callbacks![
            costs::assert_affinity_cost,
            costs::assert_affinity_cost_report
        ],
    },
    Packet {
        id: "reducer_safety",
        source: Some("The Great Henge"),
        clauses: &["This spell costs"],
        disposition: "unsupported_safety",
        scope: "A large creature cannot grant the unsupported power discount; the printed nine-mana route remains separate.",
        approximation: "none",
        limit: "Independent mana and life-gain effects do not certify the reducer.",
        callbacks: row_callbacks![
            costs::assert_unsupported_reducer,
            costs::assert_unsupported_reducer_report
        ],
    },
    Packet {
        id: "hybrid",
        source: Some("Deathrite Shaman"),
        clauses: &[],
        disposition: "scoped_supported",
        scope: "Printed hybrid cost accepts black or green and rejects blue through actions and full turns.",
        approximation: "none",
        limit: "Printed-field evidence only. All three graveyard abilities remain unsupported.",
        callbacks: row_callbacks![
            payment::assert_hybrid_cast,
            payment::assert_hybrid_cast_report
        ],
    },
    Packet {
        id: "phyrexian",
        source: Some("Phyrexian Metamorph"),
        clauses: &[],
        disposition: "scoped_approximation",
        scope: "Pay three mana and two life even when blue is available; insufficient mana or life blocks casting.",
        approximation: "life-only Phyrexian payment policy",
        limit: "Printed-field evidence only. Copy effects and the exact last-life boundary are not certified here.",
        callbacks: row_callbacks![
            payment::assert_phyrexian_cast,
            payment::assert_phyrexian_cast_report
        ],
    },
    Packet {
        id: "twobrid",
        source: None,
        clauses: &[],
        disposition: "synthetic_supported",
        scope: "One matching color or two generic units pay; one unrelated unit cannot.",
        approximation: "none",
        limit: "No selected or wider genuine source supplies this clause.",
        callbacks: scenario_callbacks![payment::assert_synthetic_twobrid],
    },
    Packet {
        id: "convoke",
        source: None,
        clauses: &[],
        disposition: "synthetic_supported",
        scope: "One untapped matching creature pays once for generic, colored, or hybrid cost, not true colorless.",
        approximation: "deterministic payment selection",
        limit: "Synthetic payment bodies do not certify a genuine card.",
        callbacks: scenario_callbacks![payment::assert_synthetic_convoke],
    },
    Packet {
        id: "improvise",
        source: None,
        clauses: &[],
        disposition: "synthetic_supported",
        scope: "One real untapped artifact pays generic cost once, not a colored or colorless pip.",
        approximation: "deterministic payment selection",
        limit: "Synthetic core evidence only.",
        callbacks: scenario_callbacks![payment::assert_synthetic_improvise],
    },
    Packet {
        id: "shared_tap",
        source: None,
        clauses: &[],
        disposition: "synthetic_supported",
        scope: "Convoke and improvise cannot spend the same artifact creature twice.",
        approximation: "deterministic payment selection",
        limit: "Synthetic core evidence only.",
        callbacks: scenario_callbacks![payment::assert_synthetic_shared_tap],
    },
    Packet {
        id: "delve",
        source: None,
        clauses: &[],
        disposition: "synthetic_supported",
        scope: "Exile only the distinct graveyard instances needed for generic cost; colored cost and missing fodder remain enforced.",
        approximation: "deterministic payment selection",
        limit: "No genuine source or broader replay permission is certified.",
        callbacks: scenario_callbacks![payment::assert_synthetic_delve],
    },
    Packet {
        id: "unpaid_symbols",
        source: None,
        clauses: &[],
        disposition: "synthetic_safety",
        scope: "Snow and unknown mana symbols cannot produce a free cast or reward.",
        approximation: "none",
        limit: "Synthetic unsupported-cost safety only.",
        callbacks: scenario_callbacks![payment::assert_synthetic_unpaid_symbols],
    },
    Packet {
        id: "restrictions",
        source: None,
        clauses: &[],
        disposition: "synthetic_supported",
        scope: "Creature and artifact restrictions admit their own class and artifact creatures, not another class or nonspell costs.",
        approximation: "none",
        limit: "Legendary, instant/sorcery, pip-only, and generic-X report controls remain separate obligations.",
        callbacks: scenario_callbacks![payment::assert_synthetic_restrictions],
    },
    Packet {
        id: "paid_untap",
        source: None,
        clauses: &[],
        disposition: "synthetic_supported",
        scope: "Pay three mana before reusing one two-mana artifact; an unfunded untap preserves state and cannot loop.",
        approximation: "native mana-first policy",
        limit: "This artifact scenario does not prove creature sickness rules for an untap-symbol cost.",
        callbacks: scenario_callbacks![payment::assert_synthetic_paid_untap],
    },
];
