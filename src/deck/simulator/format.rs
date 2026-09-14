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
    /// 60-card London mulligan: one free mulligan, then ship-and-bottom.
    /// Outside the ship threshold the hand is redrawn and the same number
    /// of cards is bottomed at random (the sim cannot evaluate keep
    /// choices; documented in the output assumptions).
    London {
        /// Land count that ships the opener (fewer than this redraws).
        ship_lands: u8,
    },
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
        mulligan: MulliganPolicy::London { ship_lands: 1 },
    },
    FormatRules {
        key: "pioneer",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London { ship_lands: 1 },
    },
    FormatRules {
        key: "modern",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London { ship_lands: 1 },
    },
    FormatRules {
        key: "legacy",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London { ship_lands: 1 },
    },
    FormatRules {
        key: "vintage",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London { ship_lands: 1 },
    },
    FormatRules {
        key: "pauper",
        shape: Format::Constructed,
        default_turns: 8,
        mulligan: MulliganPolicy::London { ship_lands: 1 },
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
            mulligan: MulliganPolicy::London { ship_lands: 1 },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_covers_every_known_format() {
        for key in crate::deck::legal::KNOWN_FORMATS {
            assert_eq!(rules_for(key).key, *key, "missing rules for {key}");
        }
    }

    #[test]
    fn unknown_key_falls_back_to_constructed() {
        let rules = rules_for("kitchen-table");
        assert_eq!(rules.key, "constructed");
        assert_eq!(rules.shape, Format::Constructed);
        assert_eq!(rules.default_turns, 8);
    }

    #[test]
    fn commander_family_keeps_free_redraw_band() {
        for key in ["commander", "brawl", "oathbreaker"] {
            let rules = rules_for(key);
            assert_eq!(rules.shape, Format::Commander);
            assert_eq!(rules.default_turns, 10);
            assert!(
                matches!(
                    rules.mulligan,
                    MulliganPolicy::FreeRedraw { land_band: (2, 6) }
                ),
                "{key} lost its redraw band"
            );
        }
    }

    #[test]
    fn sixty_card_formats_use_london() {
        for key in [
            "standard", "pioneer", "modern", "legacy", "vintage", "pauper",
        ] {
            let rules = rules_for(key);
            assert_eq!(rules.shape, Format::Constructed);
            assert_eq!(rules.default_turns, 8);
            assert!(
                matches!(rules.mulligan, MulliganPolicy::London { ship_lands: 1 }),
                "{key} lost its London policy"
            );
        }
    }

    #[test]
    fn inference_follows_commander_section() {
        assert_eq!(rules_inferred(true).key, "commander");
        assert_eq!(rules_inferred(false).key, "constructed");
    }
}
