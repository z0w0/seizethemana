// Per-format simulation rules. Format semantics (mulligan policy, default
// turn count) live in one table keyed by format name; the simulator reads
// rules from the deck's `SimDeck.rules` and never branches on a format
// enum. Commander features (cast loop, engine tier) gate on the deck
// having commanders, not on the format.

use super::model::Format;

/// The library shape a format plays with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulliganPolicy {
    /// Commander family (commander, brawl, oathbreaker): one free redraw
    /// when the opener's land count falls outside the band.
    FreeRedraw {
        /// Inclusive land-count band for keeping the opener.
        land_band: (u8, u8),
    },
    /// 60-card London mulligan: 7-card hands with 0, 1, 6, or 7 lands
    /// redraw once; only the redrawn hand bottoms one card toward 3
    /// lands (Karsten 2022). The sim cannot evaluate keep choices;
    /// documented in the output assumptions.
    London,
}

/// Rules the simulator needs per format.
#[derive(Debug, Clone, Copy)]
pub struct FormatRules {
    /// Format key (matches Scryfall legality names).
    pub key: &'static str,
    /// Library shape: command-zone singleton vs plain 60-card deck.
    pub shape: Format,
    /// Simulated turns when `--turns` is not given.
    pub default_turns: u32,
    /// Opening-hand redraw policy.
    pub mulligan: MulliganPolicy,
}

/// Rule table, one row per format `deck legal` knows. Unknown keys fall
/// back to [`fallback_rules`].
const TABLE: &[FormatRules] = &[
    FormatRules {
        key: "commander",
        shape: Format::Commander,
        default_turns: 10,
        // Karsten's commander model redraws 0-2 and 6-7 on the free
        // mulligan; the 2-6 band is close and changing it would churn
        // commander baselines for little gain.
        mulligan: MulliganPolicy::FreeRedraw { land_band: (2, 6) },
    },
    FormatRules {
        key: "brawl",
        shape: Format::Commander,
        default_turns: 10,
        mulligan: MulliganPolicy::FreeRedraw { land_band: (2, 6) },
    },
    FormatRules {
        key: "oathbreaker",
        shape: Format::Commander,
        default_turns: 10,
        mulligan: MulliganPolicy::FreeRedraw { land_band: (2, 6) },
    },
    FormatRules {
        key: "standard",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London,
    },
    FormatRules {
        key: "pioneer",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London,
    },
    FormatRules {
        key: "modern",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London,
    },
    FormatRules {
        key: "legacy",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London,
    },
    FormatRules {
        key: "vintage",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London,
    },
    FormatRules {
        key: "pauper",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London,
    },
];

/// Rules for a format key; unknown keys play as generic constructed.
pub fn rules_for(key: &str) -> FormatRules {
    TABLE
        .iter()
        .find(|r| r.key.eq_ignore_ascii_case(key))
        .copied()
        .unwrap_or(FormatRules {
            key: "constructed",
            shape: Format::Constructed,
            default_turns: 8,
            mulligan: MulliganPolicy::London,
        })
}

/// Rules inferred from deck shape: a COMMANDER section means the
/// commander-family rules, otherwise generic constructed.
pub fn rules_inferred(has_commander_section: bool) -> FormatRules {
    if has_commander_section {
        rules_for("commander")
    } else {
        rules_for("constructed")
    }
}
